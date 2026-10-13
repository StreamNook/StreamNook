import Foundation
import Synchronization

/// A stand-in for the Rust core with canned answers, so the app's whole flow (browse, play,
/// chat) runs before the real core is linked.
public final class FakeCore: CoreBridge {
    /// Apple's public HLS example, played in place of a relay URL.
    public static let testStreamURL = "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/master.m3u8"

    public let events = CoreEvents()
    public let kind = "FakeCore"

    private struct State {
        var started: CoreDirs?
        var calls: [String] = []
        var chatFixture: ChatFixture?
        var sent: [String] = []
    }

    private let state = Mutex(State())

    public init() {}

    /// The commands invoked so far, oldest first.
    public var calls: [String] { state.withLock { $0.calls } }
    public var isStarted: Bool { state.withLock { $0.started != nil } }
    /// Recorded chat the fake answers chat commands from (settings, emotes, history).
    public var chatFixture: ChatFixture? {
        get { state.withLock { $0.chatFixture } }
        set { state.withLock { $0.chatFixture = newValue } }
    }
    /// Messages handed to `send_chat_message`, oldest first.
    public var sentMessages: [String] { state.withLock { $0.sent } }

    public func start(dirs: CoreDirs, version: String, vaultKey: Data, codecs: [String]) throws {
        guard vaultKey.count == 32 else { throw CoreFailure.start(message: "vault key must be 32 bytes") }
        let first = state.withLock { state -> Bool in
            defer { state.started = dirs }
            return state.started == nil
        }
        if first {
            events.emit(name: "core-started", payloadJSON: #"{"version":"\#(version)","core":"fake"}"#, target: nil)
        }
    }

    public func invoke(_ cmd: String, _ argsJSON: String) async throws -> String {
        let started = state.withLock { state -> Bool in
            state.calls.append(cmd)
            return state.started != nil
        }
        guard started else { throw CoreFailure.command(json: #""core not started""#) }
        let args = (try? JSONSerialization.jsonObject(with: Data(argsJSON.utf8))) as? [String: Any] ?? [:]
        try await Task.sleep(for: .milliseconds(120))
        switch cmd {
        case "get_top_games_paginated":
            return try Self.json([Self.categories.map(Self.object), NSNull()] as [Any])
        case "get_streams_by_game":
            guard let gameID = args["gameId"] as? String else {
                throw CoreFailure.command(json: #""missing gameId""#)
            }
            let limit = args["limit"] as? Int ?? 20
            let streams = Self.streams.filter { $0.gameID == gameID }.prefix(limit).map(\.stream)
            return try Self.json([streams.map(Self.object), NSNull()] as [Any])
        case "start_stream":
            guard args["url"] is String else { throw CoreFailure.command(json: #""missing url""#) }
            return try Self.json(["url": Self.testStreamURL, "quality": "best"])
        case "start_chat":
            // No socket: the app reads `chatTicker` when the core is fake.
            return "0"
        case "stop_stream", "stop_chat":
            return "null"
        case "get_settings":
            let design = chatFixture.flatMap { try? JSONSerialization.jsonObject(with: $0.chatDesignJSON) } ?? [String: Any]()
            return try Self.json(["chat_design": design])
        case "load_channel_history":
            return "[]"
        case "fetch_channel_emotes":
            guard let emotes = chatFixture?.emotesJSON else { return #"{"twitch":[],"bttv":[],"7tv":[],"ffz":[],"kick":[]}"# }
            return String(decoding: emotes, as: UTF8.self)
        case "send_chat_message":
            let text = args["message"] as? String ?? ""
            state.withLock { $0.sent.append(text) }
            // A send needs a Twitch sign-in, which the fake never has.
            throw CoreFailure.command(json: #""Not signed in to Twitch""#)
        default:
            throw CoreFailure.command(json: #""unknown command \#(cmd)""#)
        }
    }

    public func setWebCookies(origin: String, cookiesJSON: String) throws {}

    public func lifecycle(_ state: String) {
        events.emit(name: "app-lifecycle", payloadJSON: #"{"state":"\#(state)"}"#, target: nil)
    }

    /// Chat frames for a channel in the socket's format, one every `interval`, until the reader
    /// stops reading.
    public func chatTicker(channel: String, interval: Duration = .milliseconds(700)) -> AsyncStream<String> {
        AsyncStream { continuation in
            let task = Task {
                continuation.yield("IRC_CONNECTED")
                continuation.yield(#"{"type":"ROOMSTATE","channel":"\#(channel)"}"#)
                var n = 0
                while !Task.isCancelled {
                    n += 1
                    if n % 15 == 0 {
                        continuation.yield("HEARTBEAT")
                    } else {
                        continuation.yield(Self.chatMessage(n, channel: channel))
                    }
                    try? await Task.sleep(for: interval)
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    static func chatMessage(_ n: Int, channel: String) -> String {
        let names = ["nookfan", "glassowl", "pixelpine", "latebloomer", "kappakid", "midnightmod"]
        let lines = ["hi chat", "that was clean", "LUL", "what game is next", "gg", "first time here",
                     "audio is perfect", "the edge readout is live", "Pog", "nice"]
        let name = names[n % names.count]
        let message: [String: Any] = [
            "id": "fake-\(n)",
            "user_id": "\(1000 + n % names.count)",
            "username": name,
            "display_name": name.capitalized,
            "color": NSNull(),
            "badges": [Any](),
            "timestamp": ISO8601DateFormatter().string(from: Date()),
            "content": lines[n % lines.count],
            "provider": "twitch",
            "channel": channel,
            "tags": [String: String](),
        ]
        return (try? json(message)) ?? "{}"
    }

    // MARK: Canned data

    static let categories = [
        TwitchCategory(id: "509658", name: "Just Chatting"),
        TwitchCategory(id: "21779", name: "League of Legends"),
    ]

    static let streams: [(gameID: String, stream: TwitchStream)] = (0..<26).map { i in
        let game = categories[i < 14 ? 0 : 1]
        let login = "fakestreamer\(i)"
        return (game.id, TwitchStream(
            id: "\(900_000 + i)", userLogin: login, userName: "FakeStreamer\(i)",
            title: "Spike stream \(i): testing the iPhone app", viewerCount: 40_000 - i * 1_350,
            gameName: game.name
        ))
    }

    static func object(_ value: some Encodable) -> Any {
        guard let data = try? JSONEncoder().encode(value),
              let object = try? JSONSerialization.jsonObject(with: data)
        else { return NSNull() }
        return object
    }

    static func json(_ value: Any) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }
}
