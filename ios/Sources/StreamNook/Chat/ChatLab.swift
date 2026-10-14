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
@MainActor
enum ChatDrive {
    private static weak var model: AppModel?
    /// The chat stage when it is open, otherwise the watch layer's room.
    static var currentSession: ChatSession? { ChatLab.session ?? model?.watch.chat }
    static var currentList: ChatListView? { currentSession.flatMap { ChatRegistry.shared.list(for: $0) } }

    static func run(_ name: String, _ value: String, model: AppModel) async -> Bool {
        self.model = model
        // Steps that write (send, fan actions that can delete, time out or ban, moderator tools)
        // only ever run against a fake core, never a signed-in account.
        if ["chatsend", "chatfan", "chatfanend", "chatmod"].contains(name), !(currentSession?.core is FakeCore) {
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
        default:
            return false
        }
        return true
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
