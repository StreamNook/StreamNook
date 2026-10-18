import Foundation
import Observation
import os
import SNKit
import UIKit

let chatLog = Logger(subsystem: "app.streamnook", category: "chat")

/// One open chat room: the core's socket (or a fixture replay) feeding the pipeline, the room's
/// modes, and the reply draft. The list and the composer both read it.
@Observable @MainActor
final class ChatSession {
    enum Source {
        /// The core's chat socket for a channel, read by this room alone (the chat stage).
        case core
        /// One of the open rooms: `ChatRooms` joins it and feeds it from the shared socket.
        case room
        /// Recorded frames replayed at a rate (frames a second), for building without a sign-in.
        case fixture(ChatFixture, rate: Double)
    }

    /// The room's key: the bare lowercased login on Twitch, `kick:slug` on Kick.
    let channel: String
    /// `twitch` or `kick`.
    let provider: String
    let core: CoreBridge
    let source: Source
    @ObservationIgnored let pipeline: ChatPipeline

    private(set) var modes: ChatRoomModes?
    private(set) var style = ChatStyle()
    private(set) var connected = false
    private(set) var error: String?
    private(set) var rowCount = 0
    /// The message a reply is drafted to, by room.
    var replyTo: ChatMessage?
    private(set) var paused = false
    /// The room's numeric Twitch id, once known.
    var roomID: String?
    /// Moderator tools are on wherever the viewer moderates, unless turned off for the room.
    var moderatorTools = false
    /// The viewer moderates here (their moderator or broadcaster badge in this room).
    private(set) var isModerator = false
    /// The open rooms this one belongs to, for the chat menu's tab actions.
    @ObservationIgnored weak var rooms: ChatRooms?

    @ObservationIgnored private var reader: Task<Void, Never>?
    @ObservationIgnored private var settingsTask: Task<Void, Never>?

    init(channel: String, core: CoreBridge, source: Source, roomID: String? = nil, provider: String = "twitch") {
        self.channel = channel.lowercased()
        self.provider = provider
        self.core = core
        self.source = source
        let box = WeakBox<ChatSession>()
        pipeline = ChatPipeline(style: ChatStyle(), scale: UITraitCollection.current.displayScale, onModes: { modes in
            Task { @MainActor in box.value?.modes = modes }
        }, onRowCount: { count in
            Task { @MainActor in box.value?.rowCount = count }
        })
        box.value = self
        self.roomID = roomID.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// A Kick room: its rows come from Kick's adapter, and nothing Twitch-backed applies.
    var isKick: Bool { provider == "kick" }

    /// The channel the platform knows: the Twitch login, or the Kick slug.
    var login: String {
        guard provider != "twitch", let colon = channel.firstIndex(of: ":") else { return channel }
        return String(channel[channel.index(after: colon)...])
    }

    func start() {
        guard reader == nil else { return }
        loadSettings()
        loadRole()
        switch source {
        case .room:
            connected = true
            reader = Task { [weak self] in await self?.loadHistory() }
        case .fixture(let fixture, let rate):
            roomID = fixture.roomID
            let pipeline = pipeline
            connected = true
            reader = Task.detached(priority: .userInitiated) {
                for await frame in fixture.replay(rate: rate) {
                    pipeline.ingest(frame)
                }
            }
        case .core:
            let channel = channel
            let core = core
            let pipeline = pipeline
            reader = Task { [weak self] in
                do {
                    let port = try await core.invoke("start_chat", args: ["channel": channel, "reattach": false], as: Int.self)
                    guard !Task.isCancelled else { return }
                    self?.connected = true
                    chatLog.notice("chat socket \(channel, privacy: .public) port \(port)")
                    Task { await self?.loadHistory() }
                    if port == 0, let fake = core as? FakeCore {
                        // FakeCore has no socket: its ticker speaks the socket's format.
                        for await frame in fake.chatTicker(channel: channel) { pipeline.ingest(frame) }
                        return
                    }
                    try await Self.read(port: port, into: pipeline)
                } catch {
                    guard !Task.isCancelled else { return }
                    self?.error = "\(error)"
                    chatLog.error("chat failed: \(String(describing: error), privacy: .public)")
                }
            }
        }
    }

    func stop() {
        reader?.cancel()
        reader = nil
        settingsTask?.cancel()
        if case .core = source {
            let core = core
            Task.detached { _ = try? await core.invoke("stop_chat", "{}") }
        }
    }

    /// Takes the room's connection again after `stop()`.
    func restart() {
        start()
    }

    /// Reads the core's chat bridge again on `port`, keeping the rows: back from the background,
    /// the socket the room was read from may be gone and the bridge may have a new port. The
    /// room's claim in the core never lapsed, so no new `start_chat` is needed.
    func reconnect(port: Int) {
        guard case .core = source, reader != nil, port > 0 else { return }
        reader?.cancel()
        error = nil
        let pipeline = pipeline
        let channel = channel
        chatLog.notice("chat socket \(channel, privacy: .public) reconnecting on port \(port)")
        reader = Task { [weak self] in
            do {
                self?.connected = true
                try await Self.read(port: port, into: pipeline)
            } catch {
                guard !Task.isCancelled else { return }
                self?.error = "\(error)"
                chatLog.error("chat reconnect failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    func setPaused(_ paused: Bool) {
        self.paused = paused
    }

    /// The core's chat settings, read once when the room opens.
    private func loadSettings() {
        let core = core
        settingsTask = Task { [weak self] in
            guard let json = try? await core.invoke("get_settings", "{}") else { return }
            let style = ChatStyle(settingsJSON: Data(json.utf8))
            guard let self else { return }
            self.style = style
            self.pipeline.setStyle(style)
        }
    }

    /// Whether the viewer moderates this room, from the core's send budget (USERSTATE badges).
    private func loadRole() {
        // The send budget reads Twitch's USERSTATE; Kick rooms carry no Twitch role.
        guard !isKick else { return }
        let core = core
        let channel = channel
        Task { [weak self] in
            guard let json = try? await core.invoke("chat_send_budget", args: ["channel": channel]),
                  let budget = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any],
                  let role = budget["role"] as? String, let self else { return }
            self.isModerator = role == "moderator" || role == "broadcaster"
            // On by default; turned off for this room earlier this session, it stays off.
            self.moderatorTools = self.isModerator && !(self.rooms?.modToolsOff.contains(self.channel) ?? false)
        }
    }

    /// The composer menu's switch: remembered for this room until the app closes.
    func setModeratorTools(_ on: Bool) {
        moderatorTools = on && isModerator
        rooms?.noteModeratorTools(on, for: channel)
    }

    /// After the connection was down or rebuilt: the rows missed meanwhile (within the window the
    /// core names, else the recent history), in time order among the rows held.
    func backfillGap(afterMs: Int64? = nil, beforeMs: Int64? = nil) {
        if isKick {
            let core = core
            let login = login
            let pipeline = pipeline
            Task {
                guard let json = try? await core.invoke("kick_chat_history", args: ["channel": login]),
                      let rows = try? JSONDecoder().decode([ChatMessage].self, from: Data(json.utf8)) else { return }
                pipeline.backfill(rows)
            }
            return
        }
        let core = core
        let channel = channel
        let pipeline = pipeline
        Task {
            guard let json = try? await core.invoke("load_channel_history", args: [
                "channel": channel, "afterMs": afterMs, "beforeMs": beforeMs,
            ]),
                  let rows = try? JSONDecoder().decode([ChatMessage].self, from: Data(json.utf8)) else { return }
            pipeline.backfill(rows)
        }
    }

    /// The join backfill: recent history above the live rows, then the room's badge art.
    private func loadHistory() async {
        if isKick {
            // Kick's own scrollback, laid out by Rust like live rows; Kick badges ride the rows.
            if let json = try? await core.invoke("kick_chat_history", args: ["channel": login]),
               let rows = try? JSONDecoder().decode([ChatMessage].self, from: Data(json.utf8)) {
                pipeline.prepend(rows)
                chatLog.notice("kick history for \(self.login, privacy: .public): \(rows.count) rows")
            }
            return
        }
        // Cached when fresh, else fetched: moderator, VIP, partner and event badges come from here.
        _ = try? await core.invoke("fetch_global_badges", "{}")
        let asked = Date()
        if let json = try? await core.invoke("load_channel_history", args: ["channel": channel]),
           let rows = try? JSONDecoder().decode([ChatMessage].self, from: Data(json.utf8)) {
            chatLog.notice("history for \(self.channel, privacy: .public): \(rows.count) rows in \(Int(Date().timeIntervalSince(asked) * 1000)) ms")
            pipeline.prepend(rows)
            if roomID == nil { roomID = rows.first?.tags["room-id"] }
        }
        // Without history the first live row names the room.
        for _ in 0..<60 where roomID == nil {
            try? await Task.sleep(for: .milliseconds(500))
            roomID = pipeline.recentMessages(1).first?.tags["room-id"]
        }
        if let roomID {
            do {
                _ = try await core.invoke("fetch_channel_badges", args: ["channelId": roomID])
                chatLog.notice("channel badges for \(roomID, privacy: .public) loaded")
            } catch {
                chatLog.notice("channel badges failed: \(String(describing: error), privacy: .public)")
            }
        } else {
            chatLog.notice("channel badges skipped: room id unknown")
        }
    }

    private nonisolated static func read(port: Int, into pipeline: ChatPipeline) async throws {
        guard let url = URL(string: "ws://127.0.0.1:\(port)") else { return }
        let socket = URLSession.shared.webSocketTask(with: url)
        socket.maximumMessageSize = 4 * 1024 * 1024
        socket.resume()
        try await withTaskCancellationHandler {
            while !Task.isCancelled {
                switch try await socket.receive() {
                case .string(let text): pipeline.ingest(text)
                case .data(let data): pipeline.ingest(String(decoding: data, as: UTF8.self))
                @unknown default: break
                }
            }
        } onCancel: {
            socket.cancel(with: .goingAway, reason: nil)
        }
    }
}

/// A weak reference a Sendable closure can hold.
final class WeakBox<T: AnyObject>: @unchecked Sendable {
    weak var value: T?
}
