import Foundation
import Observation
import os
import SNKit
import Synchronization
import UIKit

/// Every open chat room: the tab that follows the watched stream and the rooms opened beside it.
/// The core keeps the tab rules (`chat_tabs_*`, the same ones the Android app follows) and says,
/// with every change, which rooms to join and leave; this joins them, reads the core's one chat
/// socket and hands each frame to its room, so a background room keeps every message and
/// switching tabs never reconnects. It also keeps what belongs to each room beside its rows: its
/// pinned messages and its watch-streak and resub banners.
@Observable @MainActor
final class ChatRooms {
    /// What a room offers to share above the composer.
    struct Streaks: Equatable {
        var watch: WatchStreakMilestone?
        var resub: ResubNotice?
    }

    let core: CoreBridge
    @ObservationIgnored weak var toasts: Toasts?
    /// Your Kick account, which a Kick room's composer needs to send.
    @ObservationIgnored weak var kick: KickAccount?

    private(set) var state = ChatTabsState()
    private(set) var sessions: [String: ChatSession] = [:]
    /// The "Add a chat" sheet is up.
    var addOpen = false
    /// What the sheet searches when it opens (drive steps).
    @ObservationIgnored var addQuery = ""
    /// Pinned messages by Twitch login, newest first, as the core's channel state reads them.
    private(set) var pins: [String: [PinnedChat]] = [:]
    /// Each Twitch room's Shared Chat or Together group, by login, as the core's channel state
    /// reads it (the first answer, then its pushes).
    private(set) var groups: [String: ChannelGroup] = [:]
    /// The viewer's relationship to each Twitch room's channel, as the core holds it (read when
    /// the room opened, pushed on every change), so the chat menu opens with it known.
    private(set) var relationships: [String: ChannelRelationship] = [:]
    /// The pinned banner is unrolled: the room's own tap, else Chat > Pinned Messages > Collapsed.
    var pinsOpen: Bool {
        get { pinsChoice ?? !(activeSession?.style.pinnedStartCollapsed ?? true) }
        set { pinsChoice = newValue }
    }
    /// A tap on the banner, held until the room on screen changes.
    private var pinsChoice: Bool?
    /// The room on screen has pins and draws them (Style: Hidden draws nothing while folded).
    var pinsShown: Bool {
        !activePins.isEmpty && (activeSession?.style ?? ChatStyle()).pinsShown(open: pinsOpen)
    }
    /// A pin is folded out of sight by Style: Hidden, so the chat menu offers to open it.
    var hiddenPinWaiting: Bool { !activePins.isEmpty && !pinsShown }
    @ObservationIgnored private var shownChannel: String?
    /// Watch streak and resub by room, read once when the room opens.
    private(set) var streaks: [String: Streaks] = [:]
    /// "room:watch" and "room:resub" waved away this session.
    private(set) var dismissed: Set<String> = []
    /// A share is running; both Share chips wait for it.
    private(set) var sharing = false
    /// Where a streak share stands, by "room:watch" / "room:resub": armed for the room's next
    /// send, sending, shared (said briefly before the banner goes) or failed (one line, the
    /// banner and the typed text kept for another try).
    enum ShareStep: Equatable { case armed, sending, shared, failed }
    private(set) var shareSteps: [String: ShareStep] = [:]
    /// The resub share includes the sub streak (on by default).
    var includeStreak = true
    /// Rooms (by login) whose moderator tools were turned off this session.
    @ObservationIgnored private(set) var modToolsOff: Set<String> = []
    /// A mention opened from Mentions or its notification that the room's rows no longer hold,
    /// by room: shown pinned over the chat ("Older than chat history") until dismissed.
    var mentionCards: [String: MentionEntry] = [:]
    /// The mentions inbox, for the chat menu's Mentions row and its count.
    @ObservationIgnored weak var mentions: MentionsStore?
    /// The Mentions sheet is up over the watch layer (opened from the chat menu).
    var mentionsOpen = false
    /// The latest mention being brought into view; an older search gives way to it.
    @ObservationIgnored var mentionFocus: Task<Void, Never>?
    /// The Twitch follows that are live now, for the "Add a chat" sheet (the home snapshot's).
    @ObservationIgnored var followedLive: @MainActor () -> [TwitchStream] = { [] }
    /// Moves with every home snapshot read, so the picker's offline follows appear once the core
    /// has them (they land after the live list at launch).
    @ObservationIgnored var homeSnapshots: @MainActor () -> Int = { 0 }
    /// A live Twitch channel's group (streaming together or sharing its chat), by channel id: the
    /// home snapshot's maps the stream cards read.
    @ObservationIgnored var groupFor: @MainActor (String) -> ChannelGroup? = { _ in nil }

    @ObservationIgnored private let routes = ChatRoutes()
    @ObservationIgnored private var reader: Task<Void, Never>?
    @ObservationIgnored private var port: Int?
    /// Joins and leaves run one after another, so the first room's `start_chat` finishes before
    /// the next room joins its connection.
    @ObservationIgnored private var queue: Task<Void, Never>?
    @ObservationIgnored private var listening = false
    @ObservationIgnored private var backgroundedAt: ContinuousClock.Instant?
    /// A chat list of these rooms can be seen (the app in front, chat on screen).
    @ObservationIgnored private var onScreen = true

    /// No chat of these rooms can be seen: every room keeps its rows as text and draws none
    /// until one shows again (`ChatPipeline.setSuspended`).
    func setOnScreen(_ onScreen: Bool) {
        self.onScreen = onScreen
        for session in sessions.values { session.pipeline.setSuspended(!onScreen) }
    }
    /// Twitch's connection is up (a Twitch room ran `start_chat`); Kick rooms ride the bridge alone.
    @ObservationIgnored private var ircUp = false
    /// Kick rooms closed within their release grace, by room key.
    @ObservationIgnored private var releasing: [String: Task<Void, Never>] = [:]

    /// The signed-in Twitch account's id (set by the app model from its sign-in).
    @ObservationIgnored var viewerID: @MainActor () -> String? = { nil }

    init(core: CoreBridge) {
        self.core = core
    }

    var tabs: [ChatTabInfo] { state.tabs }
    var active: ChatTabInfo? { state.active }
    var activeSession: ChatSession? { state.activeChannel.flatMap { sessions[$0] } }
    /// The strip shows from two rooms up; one room keeps every point of height for chat.
    var showsStrip: Bool { state.tabs.count >= 2 }
    /// The stream's own chat is on screen, so its hype train, drops, polls and predictions may show.
    /// Twitch only: a Kick id would name an unrelated Twitch channel.
    var viewingStream: Bool { active?.pinnedToStream == true && active?.provider == "twitch" }

    /// The active room's pins: a Twitch room's from the core's channel state, another platform's
    /// from its own PINNED frames.
    var activePins: [PinnedChat] {
        guard let tab = active else { return [] }
        return pins[Self.pinsKey(tab)] ?? []
    }

    /// Where a room's pins are kept: the Twitch login, or the room's key (`kick:slug`) elsewhere,
    /// so a Kick slug never reads a Twitch channel's pins.
    private static func pinsKey(_ tab: ChatTabInfo) -> String {
        tab.provider == "twitch" ? tab.login : tab.channel
    }

    // MARK: Tabs

    /// Starts following the core's tab, rebuild and channel-state pushes.
    func start() {
        guard !listening else { return }
        listening = true
        load()
        let tabsEvents = core.events.stream("chat-tabs")
        let rebuilt = core.events.stream("chat-rebuilt")
        let channelState = core.events.stream("channel-state")
        let gaps = core.events.stream("chat-gap")
        Task { [weak self] in
            for await event in gaps {
                guard let gap = try? JSONDecoder().decode(ChatGap.self, from: Data(event.payloadJSON.utf8)) else { continue }
                self?.backfill(gap)
            }
        }
        Task { [weak self] in
            for await event in tabsEvents {
                guard let data = event.payloadJSON.data(using: .utf8),
                      let view = try? JSONDecoder().decode(ChatTabsState.self, from: data) else { continue }
                self?.mirror(view)
            }
        }
        Task { [weak self] in
            for await event in rebuilt {
                let payload = (try? JSONSerialization.jsonObject(with: Data(event.payloadJSON.utf8))) as? [String: Any]
                guard let port = payload?["port"] as? Int else { continue }
                self?.rebuilt(port: port)
            }
        }
        Task { [weak self] in
            for await event in channelState {
                if let update = PinnedUpdate.parse(event.payloadJSON) {
                    self?.takePins(login: update.login.lowercased(), update.pinned)
                } else if let self, let room = self.groupRoom(event.payloadJSON) {
                    self.setGroup(room.group, login: room.login)
                } else if let update = RelationshipUpdate.parse(event.payloadJSON) {
                    self?.takeRelationship(login: update.login.lowercased(), update.relationship)
                }
            }
        }
    }

    /// Points the stream tab at the watched stream; nil when the player closed (rooms opened
    /// beside it stay).
    func syncStream(_ stream: TwitchStream?) {
        // A Kick room is `kick:slug` and carries no Twitch id.
        let room = stream.map {
            Self.room(provider: $0.platform, channel: $0.userLogin, id: $0.twitchID, label: $0.userName,
                      avatar: $0.profileImageURL)
        }
        change("chat_tabs_sync_stream", ["room": room])
    }

    /// The saved chats, as the core restored them for whoever is signed in (not joined yet).
    func load() {
        Task {
            guard let view = try? await core.invoke("chat_tabs_get", args: [:], as: ChatTabsState.self) else { return }
            mirror(view)
        }
    }

    /// Joins every chat not joined yet: the Chats tab's first look.
    func connect() {
        change("chat_tabs_connect", [:])
    }

    /// The rooms on screen right now, for the mentions hub (mentions there are read as they land).
    func setVisible(_ rooms: [String]) {
        Task { _ = try? await core.invoke("chat_tabs_visible", args: ["rooms": rooms]) }
    }

    /// Keeps the stream's chat after the player closes, or lets it close with it.
    func keep(_ channel: String, _ keep: Bool) {
        Task {
            guard let view = try? await core.invoke("chat_tabs_keep", args: ["channel": channel, "keep": keep],
                                                    as: ChatTabsState.self) else { return }
            mirror(view)
        }
    }

    /// Opens a room as one of your chats and puts it on screen.
    func add(_ row: ChatRoomRow) {
        change("chat_tabs_add", ["room": Self.room(provider: row.provider, channel: row.userLogin, id: row.userID,
                                                   label: row.userName.isEmpty ? row.userLogin : row.userName,
                                                   avatar: row.profileImageURL)])
    }

    /// A room as the core's `ChatRoomInput` reads it (nested values cannot be optionals).
    private static func room(provider: String, channel: String, id: String?, label: String, avatar: String?) -> [String: Any] {
        ["provider": provider, "channel": channel, "channel_id": id.flatMap { $0.isEmpty ? nil : $0 } ?? NSNull(),
         "label": label, "avatar": avatar ?? NSNull()]
    }

    /// Closes a room opened beside the stream.
    func close(_ channel: String) {
        change("chat_tabs_remove", ["channel": channel])
    }

    /// Puts an open room on screen; shown at once, the core's answer confirms it.
    func select(_ channel: String) {
        guard channel != state.activeChannel, state.tabs.contains(where: { $0.channel == channel }) else { return }
        state.activeChannel = channel
        showActive()
        Task {
            guard let view = try? await core.invoke("chat_tabs_set_active", args: ["channel": channel],
                                                    as: ChatTabsState.self) else { return }
            mirror(view)
        }
    }

    private func change(_ command: String, _ args: [String: Any?]) {
        // Encoded here: a dictionary of Any cannot cross into the task.
        let clean = args.mapValues { $0 ?? NSNull() }
        let payload = (try? JSONSerialization.data(withJSONObject: clean, options: [.sortedKeys]))
            .map { String(decoding: $0, as: UTF8.self) } ?? "{}"
        Task {
            do {
                let json = try await core.invoke(command, payload)
                let change = try JSONDecoder().decode(ChatTabsState.self, from: Data(json.utf8))
                apply(change)
            } catch {
                chatLog.error("\(command, privacy: .public) failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// Shows the core's list unless a newer one is already on screen.
    private func mirror(_ view: ChatTabsState) {
        guard view.revision >= state.revision else { return }
        var shown = view
        shown.acquire = []
        shown.release = []
        state = shown
        showActive()
    }

    /// Mirrors a change, then leaves and joins the rooms it names.
    private func apply(_ change: ChatTabsState) {
        mirror(change)
        // A stream switch leaves one room and joins another: the connection stays up between.
        let staying = Set(sessions.keys).subtracting(change.release.map(\.channel))
            .union(change.acquire.map(\.channel))
        for tab in change.release { leave(tab, last: staying.isEmpty) }
        for tab in change.acquire { join(tab) }
        showActive()
    }

    /// Only the room on screen lays its rows out.
    private func showActive() {
        for (key, session) in sessions { session.pipeline.setVisible(key == state.activeChannel) }
        if shownChannel != state.activeChannel {
            shownChannel = state.activeChannel
            pinsChoice = nil
        }
    }

    // MARK: Joining and leaving

    private func join(_ tab: ChatTabInfo) {
        // Back inside a Kick room's release grace: the adapter never let go.
        releasing[tab.channel]?.cancel()
        releasing[tab.channel] = nil
        guard sessions[tab.channel] == nil else { return }
        guard tab.provider == "twitch" || tab.provider == "kick" else {
            chatLog.notice("chat room \(tab.channel, privacy: .public) skipped: the iPhone plays Twitch and Kick")
            return
        }
        let kick = tab.provider == "kick"
        let session = ChatSession(channel: kick ? tab.channel : tab.login, core: core, source: .room,
                                  roomID: kick ? nil : tab.channelID, provider: tab.provider)
        session.rooms = self
        session.pipeline.setVisible(tab.channel == state.activeChannel)
        session.pipeline.setSuspended(!onScreen)
        sessions[tab.channel] = session
        // Frames name their room: the Twitch login, or `kick:slug`.
        routes.set(tab.channel, session.pipeline)
        if kick {
            joinKick(tab, session: session)
            return
        }
        let core = core
        let login = tab.login
        enqueue { [weak self] in
            guard let self else { return }
            do {
                if self.ircUp, let port = self.port {
                    _ = try await core.invoke("join_chat_channel", args: ["channel": login])
                    chatLog.notice("chat room \(login, privacy: .public) joined on port \(port)")
                } else {
                    // The first Twitch room starts Twitch's connection; a Kick room may already
                    // have the bridge up, on the same port.
                    let port = try await core.invoke("start_chat", args: ["channel": login, "reattach": false], as: Int.self)
                    self.ircUp = true
                    if self.port != port {
                        self.port = port
                        self.read(port: port)
                    }
                    chatLog.notice("chat socket for \(login, privacy: .public) on port \(port)")
                }
            } catch {
                chatLog.error("chat room \(login, privacy: .public) failed: \(String(describing: error), privacy: .public)")
            }
        }
        session.start()
        loadExtras(tab)
    }

    /// A Kick room: Kick's adapter (its own Pusher socket) publishes onto the core's one chat
    /// bridge, which this reads like any room; the adapter's answer is the bridge's port.
    private func joinKick(_ tab: ChatTabInfo, session: ChatSession) {
        let core = core
        let slug = tab.login
        enqueue { [weak self] in
            guard let self else { return }
            do {
                let port = try await core.invoke("provider_chat_connect", args: ["provider": "kick", "channel": slug],
                                                 as: Int.self)
                if self.port == nil || self.port != port {
                    self.port = port
                    self.read(port: port)
                }
                chatLog.notice("kick room \(slug, privacy: .public) joined on port \(port)")
            } catch {
                chatLog.error("kick room \(slug, privacy: .public) failed: \(String(describing: error), privacy: .public)")
            }
        }
        session.start()
    }

    private func leave(_ tab: ChatTabInfo, last: Bool) {
        guard let session = sessions.removeValue(forKey: tab.channel) else { return }
        session.stop()
        mentionCards[tab.channel.lowercased()] = nil
        routes.remove(tab.channel)
        if tab.provider == "kick" {
            pins[Self.pinsKey(tab)] = nil
            leaveKick(tab, last: last)
            return
        }
        pins[tab.login] = nil
        groups[tab.login] = nil
        relationships[tab.login.lowercased()] = nil
        let core = core
        let login = tab.login
        enqueue { [weak self] in
            _ = try? await core.invoke("unwatch_channel_state", args: ["login": login])
            if last {
                // The last room closed: the connection goes too.
                _ = try? await core.invoke("stop_chat", "{}")
                self?.ircUp = false
                self?.stopReading()
                chatLog.notice("chat closed with its last room \(login, privacy: .public)")
            } else {
                _ = try? await core.invoke("leave_chat_channel", args: ["channel": login])
                chatLog.notice("chat room \(login, privacy: .public) left")
            }
        }
    }

    /// A Kick room lets its adapter go after a 15 s grace, so a stream switch away and back (or a
    /// closed tab reopened) keeps the socket.
    private func leaveKick(_ tab: ChatTabInfo, last: Bool) {
        let core = core
        let slug = tab.login
        let key = tab.channel
        releasing[key] = Task { [weak self] in
            try? await Task.sleep(for: .seconds(15))
            guard !Task.isCancelled, let self else { return }
            self.releasing[key] = nil
            guard self.sessions[key] == nil else { return }
            _ = try? await core.invoke("provider_chat_disconnect", args: ["provider": "kick", "channel": slug])
            chatLog.notice("kick room \(slug, privacy: .public) released")
            if self.sessions.isEmpty { self.stopReading() }
        }
        _ = last
    }

    private func enqueue(_ work: @escaping @MainActor () async -> Void) {
        let previous = queue
        queue = Task {
            await previous?.value
            await work()
        }
    }

    // MARK: The socket

    private func read(port: Int) {
        reader?.cancel()
        let routes = routes
        let box = WeakBox<ChatRooms>()
        box.value = self
        reader = Task.detached(priority: .userInitiated) {
            do {
                try await Self.read(port: port, routes: routes) { room, pin in
                    Task { @MainActor in box.value?.takePlatformPin(room: room, pin) }
                }
            } catch {
                guard !Task.isCancelled else { return }
                chatLog.error("chat socket ended: \(String(describing: error), privacy: .public)")
            }
        }
    }

    private func stopReading() {
        reader?.cancel()
        reader = nil
        port = nil
    }

    /// Reads the bridge into the rooms. A platform's own pin (`PINNED`/`UNPINNED`) goes to
    /// `onPin` with its room, since the open rooms keep pins beside the rows.
    private nonisolated static func read(port: Int, routes: ChatRoutes,
                                         onPin: @escaping @Sendable (String, PinnedChat?) -> Void) async throws {
        guard let url = URL(string: "ws://127.0.0.1:\(port)") else { return }
        let socket = URLSession.shared.webSocketTask(with: url)
        socket.maximumMessageSize = 4 * 1024 * 1024
        socket.resume()
        try await withTaskCancellationHandler {
            while !Task.isCancelled {
                let text: String
                switch try await socket.receive() {
                case .string(let string): text = string
                case .data(let data): text = String(decoding: data, as: UTF8.self)
                @unknown default: continue
                }
                let frame = ChatSocketFrame.decode(text)
                if case .pinned(_, let pin) = frame {
                    if let room = frame.room { onPin(room, pin) }
                    continue
                }
                routes.deliver(frame)
            }
        } onCancel: {
            socket.cancel(with: .goingAway, reason: nil)
        }
    }

    // MARK: Recovery

    /// The app went to the background (PiP never counts as away).
    func backgrounded(inPiP: Bool) {
        backgroundedAt = inPiP ? nil : .now
    }

    /// Back from the background: the bridge may listen on a new port, and after 90 s away the
    /// connection is rebuilt (the core throttles automatic rebuilds and skips them with no room).
    func foregrounded(port: Int?) {
        // The socket read before the background may be gone even on the same port.
        if let port, port > 0, reader != nil {
            self.port = port
            read(port: port)
        }
        guard let since = backgroundedAt else { return }
        backgroundedAt = nil
        let away = Int((ContinuousClock.now - since) / .seconds(1))
        guard away >= 90, !sessions.isEmpty else { return }
        let core = core
        Task {
            _ = try? await core.invoke("rebuild_chat", args: ["force": false, "reason": "away for \(away)s"])
        }
    }

    /// "Reload this chat": the Twitch connection is rebuilt with every room, never throttled.
    func reload(_ session: ChatSession) {
        let core = core
        if session.isKick {
            // A Kick room drops its adapter and takes it again: a fresh Pusher socket.
            let slug = session.login
            Task {
                _ = try? await core.invoke("provider_chat_disconnect", args: ["provider": "kick", "channel": slug])
                do {
                    let port = try await core.invoke("provider_chat_connect", args: ["provider": "kick", "channel": slug],
                                                     as: Int.self)
                    if self.port != port {
                        self.port = port
                        self.read(port: port)
                    }
                    session.backfillGap()
                } catch {
                    chatLog.error("kick reload failed: \(String(describing: error), privacy: .public)")
                }
            }
            return
        }
        let reason = "manual reload of \(session.channel)"
        Task {
            do {
                _ = try await core.invoke("rebuild_chat", args: ["force": true, "reason": reason])
            } catch {
                chatLog.error("rebuild_chat failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// The core rebuilt the connection: read the bridge again and fill each room's gap.
    private func rebuilt(port: Int) {
        chatLog.notice("chat rebuilt on port \(port)")
        self.port = port
        read(port: port)
        for session in sessions.values { session.backfillGap() }
    }

    /// The connection is back after an outage: each room reads the rows it missed.
    private func backfill(_ gap: ChatGap) {
        chatLog.notice("chat back after an outage; backfilling \(gap.channels.count) room(s)")
        for session in sessions.values where gap.channels.contains(session.channel) {
            session.backfillGap(afterMs: gap.afterMs, beforeMs: gap.beforeMs)
        }
    }

    // MARK: Pins and streaks

    /// What a Twitch room shows over and under its rows: pins (the core polls them every 30 s
    /// while the room is watched, and at once after a pin) and its streak banners, read once.
    private func loadExtras(_ tab: ChatTabInfo) {
        guard tab.provider == "twitch" else { return }
        guard let id = tab.channelID, !id.isEmpty else {
            // Without the id there are no pins, streak or resub offers to read for this room.
            chatLog.notice("chat room \(tab.login, privacy: .public) has no channel id: no streak offers read")
            return
        }
        let core = core
        Task {
            guard let json = try? await core.invoke("watch_channel_state", args: ["login": tab.login, "channelId": id]) else { return }
            if let object = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any],
               let list = object["pinned"], let data = try? JSONSerialization.data(withJSONObject: list),
               let pinned = try? JSONDecoder().decode([PinnedChat].self, from: data), sessions[tab.channel] != nil {
                takePins(login: tab.login, pinned)
            }
            if let held = RelationshipUpdate.held(inState: json) {
                takeRelationship(login: tab.login.lowercased(), held)
            }
            // The group the channel is in now; pushes only follow changes.
            struct Held: Decodable {
                var collab: ChannelGroup?
                var sharedChat: ChannelGroup?
                enum CodingKeys: String, CodingKey { case collab, sharedChat = "shared_chat" }
            }
            if sessions[tab.channel] != nil, let held = try? JSONDecoder().decode(Held.self, from: Data(json.utf8)) {
                setGroup(held.collab ?? held.sharedChat, login: tab.login)
            }
        }
        Task {
            var found = Streaks()
            found.watch = try? await core.invoke("get_watch_streak", args: ["channelId": id], as: WatchStreakMilestone?.self)
            found.resub = try? await core.invoke("get_resub_notification", args: ["channelLogin": tab.login], as: ResubNotice?.self)
            // What the banners will show for this room (the core log says why, per offer).
            let watch = found.watch.map { "\($0.streakCount.map(String.init) ?? "?") streams" } ?? "none"
            chatLog.notice("chat room \(tab.login, privacy: .public) streak offers: watch \(watch, privacy: .public) resub \(found.resub == nil ? "none" : "yes", privacy: .public)")
            guard sessions[tab.channel] != nil else { return }
            streaks[tab.channel] = found
        }
    }

    /// The relationship the core holds for a room's channel; nil before its first read.
    func relationship(for login: String) -> ChannelRelationship? {
        relationships[login.lowercased()]
    }

    /// A follow or unfollow this app just made, shown before the core's push confirms it.
    func noteFollowing(_ following: Bool, for login: String) {
        relationships[login.lowercased()]?.following = following
    }

    private func takeRelationship(login: String, _ relationship: ChannelRelationship?) {
        guard sessions.values.contains(where: { $0.channel == login }) else { return }
        relationships[login] = relationship
    }

    /// The group a room's channel-state push carries, for an open Twitch room.
    private func groupRoom(_ json: String) -> (login: String, group: ChannelGroup?)? {
        for tab in state.tabs where tab.provider == "twitch" {
            if let group = ChannelGroup.collabPush(json, login: tab.login) { return (tab.login, group) }
        }
        return nil
    }

    private func setGroup(_ group: ChannelGroup?, login: String) {
        guard groups[login] != group else { return }
        groups[login] = group
        let kind = group.map { $0.together ? "together" : "shared chat" } ?? "none"
        chatLog.notice("chat room \(login, privacy: .public) group: \(kind, privacy: .public)")
    }

    /// The room on screen's group, when its channel streams together or shares its chat.
    var activeGroup: ChannelGroup? {
        guard let tab = active, tab.provider == "twitch" else { return nil }
        // The maps the Following cards read come first (by Twitch id), so the chip and the card
        // always agree; the channel's own pushed state covers a room the maps miss.
        if let id = tab.channelID, !id.isEmpty, let group = groupFor(id) { return group }
        return groups[tab.login]
    }

    private func takePins(login: String, _ pinned: [PinnedChat]) {
        guard sessions.values.contains(where: { $0.channel == login }) else { return }
        pins[login] = pinned
    }

    /// A platform's own pin for an open room (Kick's and YouTube's PINNED frames), or its removal:
    /// the room's one pin, shown in the same banner as Twitch's.
    private func takePlatformPin(room: String, _ pin: PinnedChat?) {
        guard let tab = state.tabs.first(where: { $0.channel.lowercased() == room }), sessions[tab.channel] != nil else { return }
        pins[Self.pinsKey(tab)] = pin.map { [$0] } ?? []
    }

    /// The streaks to show for the room on screen, without those waved away.
    var activeStreaks: Streaks {
        guard let key = state.activeChannel, var found = streaks[key] else { return Streaks() }
        if dismissed.contains("\(key):watch") { found.watch = nil }
        if dismissed.contains("\(key):resub") { found.resub = nil }
        return found
    }

    func noteModeratorTools(_ on: Bool, for login: String) {
        if on { modToolsOff.remove(login) } else { modToolsOff.insert(login) }
    }

    func dismiss(_ what: String) {
        guard let key = state.activeChannel else { return }
        dismissed.insert("\(key):\(what)")
        shareSteps["\(key):\(what)"] = nil
    }

    /// Where this share stands in the room on screen.
    func shareStep(_ what: String) -> ShareStep? {
        guard let key = state.activeChannel else { return nil }
        return shareSteps["\(key):\(what)"]
    }

    /// The share armed in a room, which its composer's next send carries.
    func armedShare(in channel: String) -> String? {
        ["resub", "watch"].first { shareSteps["\(channel):\($0)"] == .armed }
    }

    /// Share pressed: the room's next send carries this share. One at a time.
    func armShare(_ what: String) {
        guard let key = state.activeChannel else { return }
        for kind in ["watch", "resub"] where shareSteps["\(key):\(kind)"] != .shared { shareSteps["\(key):\(kind)"] = nil }
        shareSteps["\(key):\(what)"] = .armed
    }

    func cancelShare(_ what: String) {
        guard let key = state.activeChannel else { return }
        shareSteps["\(key):\(what)"] = nil
    }

    /// The composer's send while a share is armed: the share, with what is typed (or nothing).
    /// Answers whether Twitch took it; the caller clears the draft only then.
    func sendArmedShare(in channel: String, message: String) async -> Bool {
        guard channel == state.activeChannel, let what = armedShare(in: channel) else { return false }
        return what == "watch" ? await shareWatchStreak(message: message) : await shareResub(message: message)
    }

    /// Twitch took a share: said in its banner for a moment, then the banner goes (the token is
    /// spent). Not taken: one line in the banner, which stays for another try.
    private func settleShare(_ ok: Bool, room: String, what: String, clear: @escaping () -> Void) {
        let key = "\(room):\(what)"
        guard ok else {
            shareSteps[key] = .failed
            return
        }
        shareSteps[key] = .shared
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .milliseconds(2500))
            guard let self, self.shareSteps[key] == .shared else { return }
            // The banners fold it away on their own animation.
            clear()
            self.shareSteps[key] = nil
        }
    }

    /// Shares the watch streak with the composer's text; answers whether Twitch took it.
    func shareWatchStreak(message: String) async -> Bool {
        guard let tab = active, let id = tab.channelID, let milestone = streaks[tab.channel]?.watch, !sharing else { return false }
        let room = tab.channel
        sharing = true
        shareSteps["\(room):watch"] = .sending
        defer { sharing = false }
        let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
        let ok = (try? await core.invoke("share_watch_streak", args: [
            "channelId": id, "milestoneId": milestone.milestoneID, "message": text.isEmpty ? nil : text,
        ], as: Bool.self)) ?? false
        settleShare(ok, room: room, what: "watch") { [weak self] in self?.streaks[room]?.watch = nil }
        return ok
    }

    /// Shares the resub with the composer's text, with or without the sub streak.
    func shareResub(message: String) async -> Bool {
        guard let tab = active, let resub = streaks[tab.channel]?.resub, !sharing else { return false }
        let room = tab.channel
        sharing = true
        shareSteps["\(room):resub"] = .sending
        defer { sharing = false }
        let text = message.trimmingCharacters(in: .whitespacesAndNewlines)
        let ok = (try? await core.invoke("use_resub_token", args: [
            "channelLogin": tab.login, "message": text.isEmpty ? nil : text, "includeStreak": includeStreak,
            "tokenId": resub.id,
        ], as: Bool.self)) ?? false
        settleShare(ok, room: room, what: "resub") { [weak self] in self?.streaks[room]?.resub = nil }
        return ok
    }
}

/// Which room's pipeline takes the frames for each Twitch login, read by the socket's reader
/// off the main thread. A frame that names no room (a heartbeat, a status word) goes to all.
final class ChatRoutes: Sendable {
    private let table = Mutex<[String: ChatPipeline]>([:])

    func set(_ login: String, _ pipeline: ChatPipeline) {
        table.withLock { $0[login.lowercased()] = pipeline }
    }

    func remove(_ login: String) {
        table.withLock { _ = $0.removeValue(forKey: login.lowercased()) }
    }

    func deliver(_ frame: ChatSocketFrame) {
        let targets: [ChatPipeline] = table.withLock { table in
            if let room = frame.room { return table[room].map { [$0] } ?? [] }
            // An own color that names no room is the viewer's there only while one room is open.
            if case .ownColor = frame { return table.count == 1 ? Array(table.values) : [] }
            return Array(table.values)
        }
        for pipeline in targets { pipeline.ingest(frame: frame) }
    }
}
