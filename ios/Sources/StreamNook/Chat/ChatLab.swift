#if DEBUG
import os
import SNKit
import SwiftUI
import UIKit

/// A debug stage for chat: the watch page's portrait column (a 16:9 video area over the chat
/// panel) or the landscape overlay, fed by the bundled fixture or the core. Drive steps open it
/// in a window of its own over the app, so it needs nothing from the screens around it.
@MainActor
enum ChatLab {
    private static var window: UIWindow?
    private(set) static var session: ChatSession?
    static let state = ChatLabState()

    static func open(core: CoreBridge, mode: String) {
        close()
        let session: ChatSession
        if mode.hasPrefix("core:") {
            session = ChatSession(channel: String(mode.dropFirst(5)), core: core, source: .core)
        } else {
            guard let fixture = ChatFixture.bundled() else {
                log.error("chatlab: no bundled fixture")
                return
            }
            let rate: Double = switch mode {
            case "busy": 300
            case "", "calm": 4
            default: Double(mode) ?? 4
            }
            let fake = FakeCore()
            try? fake.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "lab",
                            vaultKey: Data(repeating: 1, count: 32), codecs: [])
            fake.chatFixture = fixture
            session = ChatSession(channel: fixture.channel, core: fake, source: .fixture(fixture, rate: rate))
        }
        self.session = session
        guard let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first else { return }
        let window = UIWindow(windowScene: scene)
        window.windowLevel = .normal + 1
        window.rootViewController = UIHostingController(rootView: ChatLabView(session: session, state: state))
        window.makeKeyAndVisible()
        self.window = window
        log.notice("chatlab open \(mode, privacy: .public)")
    }

    static func close() {
        session?.stop()
        session = nil
        window?.isHidden = true
        window = nil
    }

    static var list: ChatListView? {
        session.flatMap { ChatRegistry.shared.list(for: $0) }
    }
}

@Observable @MainActor
final class ChatLabState {
    var overlay = false
}

struct ChatLabView: View {
    let session: ChatSession
    let state: ChatLabState

    var body: some View {
        ZStack {
            Color.black.ignoresSafeArea()
            if state.overlay {
                // Immersive landscape: chat floats over the video on one side.
                HStack(spacing: 0) {
                    Spacer()
                    ChatPanel(session: session, overlay: true)
                        .frame(width: 340)
                        .background(.black.opacity(0.45))
                }
                .ignoresSafeArea()
                .background(videoStandIn)
            } else {
                VStack(spacing: 0) {
                    videoStandIn
                        .aspectRatio(16 / 9, contentMode: .fit)
                    ChatPanel(session: session)
                        .background(Color(ChatTheme.background))
                }
            }
        }
        .statusBarHidden(state.overlay)
    }

    private var videoStandIn: some View {
        LinearGradient(colors: [Color(red: 0.12, green: 0.1, blue: 0.2), Color(red: 0.05, green: 0.05, blue: 0.08)],
                       startPoint: .topLeading, endPoint: .bottomTrailing)
            .overlay(alignment: .topLeading) {
                Text("\(session.channel) · \(session.rowCount) rows")
                    .font(.caption.monospacedDigit())
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .glassEffect()
                    .padding(10)
            }
    }
}

/// Chat drive steps (see Drive.swift):
///
/// - `chatlab[:calm|busy|RATE|core:CHANNEL]` opens the chat stage with the fixture (4 frames a
///   second, 300, or RATE) or the core's socket for CHANNEL
/// - `chatoverlay[:on|off]` switches the stage to the landscape overlay
/// - `chatpause` scrolls the list up as a reader would, which pauses it
/// - `chatresume` taps Resume live
/// - `chatperf:SECONDS` logs frame pacing and memory every five seconds, then a summary
/// - `chattype:TEXT` puts TEXT in the composer, `chatsend` sends it
/// - `chatfocus` focuses the composer (raises the keyboard), `chatblur` lowers it
/// - `chatpicker`, `chatmenu`, `chatcommands` open the emote picker, the chat menu, the command list
/// - `chatreply` drafts a reply to the newest row
/// - `chatmod[:off]` turns the room's moderator tools on (or off) for the fan
/// - `chatclose` closes the stage
///
/// The watch page's rooms:
///
/// - `chatadd:LOGIN` opens LOGIN's chat beside the stream (found through the core's search);
///   `chatadd:INDEX` the Browse list's stream at INDEX
/// - `chattab:INDEX` puts the tab at INDEX on screen; `chattabclose:INDEX` closes it
/// - `addchat[:QUERY]` opens the "Add a chat" sheet, searching QUERY; `addchat:close` closes it
/// - `chatpins[:close]` unrolls (or rolls up) the pinned banner
/// - `chatreload` reloads the room on screen (the core rebuilds the connection)
/// - `chatrooms` logs the tabs, the room on screen, its pins and streaks
/// - `findpins` opens, beside the stream, the first Browse channel with pinned messages
/// - `findstreak` opens the first followed live channel with a watch streak to share
@MainActor
enum ChatDrive {
    private static weak var model: AppModel?
    /// The chat stage when it is open, otherwise the watch layer's room.
    static var currentSession: ChatSession? { ChatLab.session ?? model?.chat.activeSession }
    static var currentList: ChatListView? { currentSession.flatMap { ChatRegistry.shared.list(for: $0) } }

    static func run(_ name: String, _ value: String, model: AppModel) async -> Bool {
        self.model = model
        // Steps that write (send, moderator tools) only ever run against a fake core, never a
        // signed-in account. The fan steps only open and point it; nothing is ever released on a
        // bucket, so they run anywhere.
        if ["chatsend", "chatmod"].contains(name), !(currentSession?.core is FakeCore) {
            log.error("drive: \(name, privacy: .public) refused on a real core")
            return true
        }
        switch name {
        case "chatlab":
            ChatLab.open(core: model.core, mode: value)
        case "chatoverlay":
            let on = value != "off"
            ChatLab.state.overlay = on
            // Immersive landscape, as the watch page turns when the phone does.
            if let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first {
                scene.requestGeometryUpdate(.iOS(interfaceOrientations: on ? .landscapeRight : .portrait)) { error in
                    log.error("chatoverlay: \(error.localizedDescription, privacy: .public)")
                }
            }
        case "chatpause":
            guard let list = currentList else { return true }
            list.pauseForJump()
            list.scroll.setContentOffset(CGPoint(x: 0, y: max(-list.scroll.contentInset.top, list.scroll.contentOffset.y - 420)),
                                         animated: true)
        case "chatresume":
            currentList?.resume(force: true, glide: true)
        case "chatperf":
            await perf(seconds: Double(value) ?? 60)
        case "chattype":
            composer?.draft = value
        case "chatsend":
            await composer?.send()
        case "chatfocus":
            composer?.requestFocus(true)
        case "chatblur":
            composer?.requestFocus(false)
        case "chatpicker":
            composer?.panel = .emotes
        case "chatmenu":
            composer?.menuOpen = true
        case "chatcommands":
            composer?.commandsOpen = true
        case "chatreply":
            if let session = currentSession, let last = session.pipeline.recentMessages(1).last {
                session.replyTo = last
            }
        case "chatfan":
            let parts = value.split(separator: ",")
            let bucket = FanBucket.allCases.first { $0.title.lowercased() == parts.first.map(String.init) }
            currentList?.debugFan(select: bucket, reach: parts.count > 1 ? CGFloat(Double(parts[1]) ?? 0) : 0)
        case "chatfanend":
            currentList?.debugEndFan()
        case "chatmod":
            currentSession?.moderatorTools = value != "off"
        case "chatclose":
            ChatLab.close()
        case "chatadd":
            // A number picks the Browse list's stream at that index.
            if let i = Int(value) {
                guard model.streams.indices.contains(i) else { return true }
                await add(login: model.streams[i].userLogin, rooms: model.chat)
            } else {
                await add(login: value, rooms: model.chat)
            }
        case "chattab":
            let tabs = model.chat.tabs
            if let i = Int(value), tabs.indices.contains(i) { model.chat.select(tabs[i].channel) }
        case "chattabclose":
            let tabs = model.chat.tabs
            if let i = Int(value), tabs.indices.contains(i) { model.chat.close(tabs[i].channel) }
        case "addchat":
            if value == "close" {
                model.chat.addOpen = false
            } else {
                model.chat.addQuery = value
                model.chat.addOpen = true
            }
        case "chatpins":
            withAnimation(.timingCurve(0.16, 1, 0.3, 1, duration: 0.2)) { model.chat.pinsOpen = value != "close" }
        case "chatreload":
            if let session = model.chat.activeSession { ChatReload.reload(session) }
        case "chatrooms":
            let rooms = model.chat
            let tabs = rooms.tabs.map {
                "\($0.channel)\($0.pinnedToStream ? "*" : "")=\(rooms.sessions[$0.channel]?.rowCount ?? -1)"
            }.joined(separator: ",")
            let streaks = rooms.activeStreaks
            log.notice("drive rooms tabs \(tabs, privacy: .public) active \(rooms.state.activeChannel ?? "-", privacy: .public) rev \(rooms.state.revision) pins \(rooms.activePins.count) watch \(streaks.watch?.title ?? "-", privacy: .public) resub \(streaks.resub?.title ?? "-", privacy: .public) mod \(rooms.activeSession?.isModerator ?? false) rows \(rooms.activeSession?.rowCount ?? 0)")
        case "findpins":
            await findPins(model)
        case "findstreak":
            await findStreak(model)
        default:
            return false
        }
        return true
    }

    /// Opens LOGIN's room beside the stream, as picking it in the sheet would.
    private static func add(login: String, rooms: ChatRooms) async {
        let found = (try? await rooms.core.invoke("search_chat_rooms", args: ["query": login], as: [ChatRoomRow].self)) ?? []
        guard let row = found.first(where: { $0.provider == "twitch" && $0.userLogin == login.lowercased() }) else {
            log.error("drive chatadd: \(login, privacy: .public) not found in \(found.count) results")
            return
        }
        log.notice("drive chatadd \(row.userLogin, privacy: .public) live \(row.isLive)")
        rooms.add(row)
    }

    /// The first Browse channel whose chat has pinned messages, opened beside the stream.
    private static func findPins(_ model: AppModel) async {
        for stream in model.streams.prefix(15) {
            guard let id = stream.userID, !id.isEmpty else { continue }
            log.notice("drive findpins probing \(stream.userLogin, privacy: .public)")
            guard let json = try? await model.core.invoke("get_pinned_chat_messages", args: ["channelId": id]),
                  let pins = try? JSONDecoder().decode([PinnedChat].self, from: Data(json.utf8)), !pins.isEmpty else { continue }
            log.notice("drive findpins \(stream.userLogin, privacy: .public) has \(pins.count) pin(s)")
            await add(login: stream.userLogin, rooms: model.chat)
            return
        }
        log.error("drive findpins: none of \(model.streams.prefix(15).count) channels has a pin")
    }

    /// The first followed live channel with a watch streak to share, opened to watch.
    private static func findStreak(_ model: AppModel) async {
        for stream in model.home.lists.followedLive {
            guard let id = stream.userID, !id.isEmpty,
                  let milestone = try? await model.core.invoke("get_watch_streak", args: ["channelId": id],
                                                              as: WatchStreakMilestone?.self) else { continue }
            log.notice("drive findstreak \(stream.userLogin, privacy: .public): \(milestone.title, privacy: .public)")
            model.startWatching(stream)
            return
        }
        log.error("drive findstreak: no followed live channel has a streak to share")
    }

    private static var composer: ChatComposerModel? {
        currentSession.flatMap { ComposerRegistry.shared.models[ObjectIdentifier($0)] }
    }

    private static func perf(seconds: Double) async {
        guard let list = currentList, let session = currentSession else {
            log.error("chatperf: no chat stage")
            return
        }
        list.pacing.reset()
        let start = ContinuousClock.now
        let before = session.pipeline.stats()
        var lastFrames = 0
        var lastDropped = 0
        var peak = 0
        while ContinuousClock.now - start < .seconds(seconds) {
            try? await Task.sleep(for: .seconds(5))
            let p = list.pacing
            let stats = session.pipeline.stats()
            let memory = Memory.footprintMB()
            peak = max(peak, memory)
            let pct = p.percentiles()
            log.notice("chatperf t=\(Int((ContinuousClock.now - start) / .seconds(1)))s frames \(p.frames - lastFrames) dropped \(p.dropped - lastDropped) p50 \(String(format: "%.2f", pct.p50), privacy: .public) p99 \(String(format: "%.2f", pct.p99), privacy: .public) rows \(stats.rows - before.rows) frames-in \(stats.frames - before.frames) mem \(memory)MB images \(ImagePipeline.shared.memoryCost / 1_048_576)MB/\(ImagePipeline.shared.memoryCount) animating \(AnimationClock.shared.registeredCount)")
            lastFrames = p.frames
            lastDropped = p.dropped
        }
        let p = list.pacing
        let pct = p.percentiles()
        let stats = session.pipeline.stats()
        let rows = stats.rows - before.rows
        let avg = rows > 0 ? (stats.layoutMS - before.layoutMS) / Double(rows) : 0
        log.notice("chatperf summary \(Int(seconds))s: frames \(p.frames) dropped \(p.dropped) expected \(String(format: "%.2f", p.expected * 1000), privacy: .public)ms p50 \(String(format: "%.2f", pct.p50), privacy: .public)ms p99 \(String(format: "%.2f", pct.p99), privacy: .public)ms worst \(String(format: "%.2f", pct.worst), privacy: .public)ms rows-laid-out \(rows) avg-layout \(String(format: "%.3f", avg), privacy: .public)ms peak-mem \(peak)MB")
    }
}

enum Memory {
    /// The app's physical footprint, what iOS counts against its memory limit.
    static func footprintMB() -> Int {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        guard result == KERN_SUCCESS else { return -1 }
        return Int(info.phys_footprint / 1_048_576)
    }
}
#endif
