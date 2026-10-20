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
    /// Canned answers for the screens beyond the spike (settings, the home snapshot, the account),
    /// so their signed-in states can be drawn without a real account. Nil answers none of them.
    public let fixtures: FakeFixtures?
    /// Polls and predictions, answered with or without fixtures so the fixture chat stage can
    /// draw the cards too. Votes and predictions are only recorded.
    public let liveVotes = FakeLiveVotes()

    public init(fixtures: FakeFixtures? = nil) {
        self.fixtures = fixtures
        let events = events
        fixtures?.mentions.onChange.withLock { change in
            change = { total in
                events.emit(name: "mention-inbox-changed", payloadJSON: #"{"unread_total":\#(total)}"#, target: nil)
            }
        }
        fixtures?.whispers.onUpdate.withLock { change in
            change = { update in
                guard let data = try? JSONEncoder().encode(update) else { return }
                events.emit(name: "whisper-conversation-updated", payloadJSON: String(decoding: data, as: UTF8.self), target: nil)
            }
        }
    }

    /// Files an incoming whisper in the fixture inbox and asks for its notification, as the core
    /// does when one arrives outside the conversation on screen.
    public func simulateWhisper(from key: String, text: String) {
        guard let update = fixtures?.whispers.receive(from: key, text: text),
              let data = try? JSONEncoder().encode(update) else { return }
        guard update.meta.unreadCount > 0 else { return }
        events.emit(name: "whisper-notify", payloadJSON: String(decoding: data, as: UTF8.self), target: nil)
    }

    /// Files a mention in the fixture inbox and asks for its card over the message bar and its
    /// notification, as the core does when a live message mentions you in a chat that is not on
    /// screen.
    /// Files a mention quietly, as a backfill would: no arrival card.
    public func fileMention(_ entry: MentionEntry) {
        fixtures?.mentions.file(entry)
    }

    public func simulateMention(_ entry: MentionEntry) {
        fixtures?.mentions.file(entry)
        guard let data = try? JSONEncoder().encode(entry) else { return }
        let json = String(decoding: data, as: UTF8.self)
        events.emit(name: "mention-arrived", payloadJSON: #"{"entry":\#(json),"avatar":null}"#, target: nil)
        events.emit(name: "mention-notify", payloadJSON: json, target: nil)
    }

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

    /// The whole settings object, as the core answers it: the fake's saved settings with the
    /// recording's chat look over them, so a fixture room draws as it was recorded.
    private func settingsObject() throws -> [String: Any] {
        let saved = try fixtures?.answer("load_settings", [:])
        var root = saved.flatMap { try? JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any] } ?? [:]
        if let design = chatFixture.flatMap({ try? JSONSerialization.jsonObject(with: $0.chatDesignJSON) as? [String: Any] }) {
            // A setting written since keeps its value over the recording's.
            root["chat_design"] = design.merging(root["chat_design"] as? [String: Any] ?? [:]) { $1 }
        }
        return root
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
        case "watch_start":
            guard let login = args["login"] as? String else { throw CoreFailure.command(json: #""missing login""#) }
            var row = (args["seed"] as? [String: Any]) ?? [:]
            row["user_login"] = login.lowercased()
            // The real answer's shape: the completed row (ids, start, art) and the relay's start.
            for (key, value) in ["id": "", "user_id": "", "user_name": login, "title": "Watching \(login)",
                                 "viewer_count": 0, "game_id": "", "game_name": "", "thumbnail_url": "",
                                 "started_at": "2026-10-19T00:00:00Z"] as [String: Any] where row[key] == nil {
                row[key] = value
            }
            return try Self.json(["superseded": false, "row": row,
                                  "stream": ["url": Self.testStreamURL, "quality": "best", "entitled": false,
                                             "available": ["best", "720p60", "480p", "audio_only"]]] as [String: Any])
        case "get_stream_prefetch_present", "get_stream_low_latency":
            // The fixture stream is a plain normal-latency one.
            return "false"
        case "watch_open_room":
            let login = (args["login"] as? String ?? "").lowercased()
            return try Self.json(["superseded": false,
                                  "room": ["user_id": "", "login": login, "display_name": login, "shiny": false, "chatters": 3],
                                  "row": ["id": "", "user_login": login, "user_name": login, "title": "", "viewer_count": 0,
                                          "game_name": "", "thumbnail_url": "", "is_live": false]] as [String: Any])
        case "stop_stream", "stop_chat", "watch_stop":
            return "null"
        case "load_settings":
            return try Self.json(try settingsObject())
        case "get_settings_view":
            // What the settings pages draw from: the same object, and the paths a write changed.
            return try Self.json(["values": try settingsObject(), "changed": fixtures?.changedPaths ?? []] as [String: Any])
        case "load_channel_history":
            return "[]"
        case "get_alert_types":
            return AlertTypes.fakeJSON
        case "fetch_channel_emotes":
            guard let emotes = chatFixture?.emotesJSON else { return #"{"twitch":[],"bttv":[],"7tv":[],"ffz":[],"kick":[]}"# }
            return String(decoding: emotes, as: UTF8.self)
        case "command_catalog":
            let rows = chatFixture.flatMap { try? JSONSerialization.jsonObject(with: $0.commandsJSON) as? [[String: Any]] } ?? []
            let tokens = (args["query"] as? String ?? "").lowercased().split(separator: " ")
            let hits = rows.filter { row in
                let hay = ["name", "usage", "description"].compactMap { row[$0] as? String }.joined(separator: " ").lowercased()
                return tokens.allSatisfy { hay.contains($0) }
            }
            return try Self.json(hits)
        case "match_emote_tokens":
            return try Self.json(Self.emoteMatches(chatFixture?.emotesJSON, args: args))
        case "emote_picker":
            return try Self.json(Self.emotePicker(chatFixture?.emotesJSON, args: args))
        case "run_slash_command":
            return try Self.json(Self.slashOutcome(args["text"] as? String ?? ""))
        case "channel_relationship":
            // Never a real account: a fixed "not following, never subscribed" answer.
            return try Self.json(["following": false, "subscribed": false, "has_sub_history": false,
                                  "cumulative_months": 0, "badge_url": NSNull()] as [String: Any])
        case "asset_cache_set_burst":
            return "null"
        case "get_chatter_mod_history", "acknowledge_chat_warning", "resolve_automod_message", "get_chatter_card_tools",
             "get_touch_mod_slider", "get_room_mod_log", "get_automod_queue", "edit_mod_slider_timeouts", "parse_mod_timeout",
             "add_blocked_term", "remove_blocked_term", "nuke_active", "nuke_stop",
             "moderate_chatter", "set_user_note", "set_chat_user_override":
            // Moderation never reaches Twitch from the fake: canned answers only.
            return try FakeModTools.answer(cmd, args)
        case "send_chat_message":
            let text = args["message"] as? String ?? ""
            // Every real caller names the room and the sender; without the sender the core sends
            // over IRC and the row never gets Twitch's id. The fake refuses such a send.
            guard let sender = args["senderId"] as? String, !sender.isEmpty,
                  let room = args["broadcasterId"] as? String, !room.isEmpty else {
                throw CoreFailure.command(json: #""send_chat_message needs senderId and broadcasterId""#)
            }
            // Rust holds a message that cheers rather than sending it, as the real core does.
            if let held = FakeRewards.heldCheer(text, channel: args["targetChannel"] as? String ?? "") { return held }
            state.withLock { $0.sent.append(text) }
            // A send needs a Twitch sign-in, which the fake never has.
            throw CoreFailure.command(json: #""Not signed in to Twitch""#)
        default:
            if let reply = try liveVotes.answer(cmd, args) {
                // The core confirms a vote or a prediction with its push a moment later.
                if let push = reply.push {
                    let events = events
                    Task {
                        try? await Task.sleep(for: .milliseconds(250))
                        events.emit(name: push.name, payloadJSON: push.payload, target: nil)
                    }
                }
                return reply.json
            }
            if let fixtures, let answer = try fixtures.answer(cmd, args) {
                // The real core announces a drops sign-in once Twitch hands the grant back; the
                // fake grants it at once and says so a moment later, the same way.
                if cmd == "open_drops_sheet" {
                    let events = events
                    Task {
                        try? await Task.sleep(for: .milliseconds(400))
                        events.emit(name: "drops-login-complete", payloadJSON: "null", target: nil)
                    }
                }
                return answer
            }
            throw CoreFailure.command(json: #""unknown command \#(cmd)""#)
        }
    }

    public func setWebCookies(origin: String, cookiesJSON: String) throws {}

    /// Signs the fixture account in or out the way the core announces it: a sign-in emits
    /// `twitch-login-complete`, and either way the roster changes.
    public func simulateTwitchSignIn(_ signedIn: Bool) {
        fixtures?.setSignedIn(signedIn)
        if signedIn { events.emit(name: "twitch-login-complete", payloadJSON: "null", target: nil) }
        events.emit(name: "account-roster-changed", payloadJSON: "{}", target: nil)
    }

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
        var message: [String: Any] = [
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
            // The fake wallet channel's room, so a stage on FakeCore learns its room the way
            // Twitch rows tell it.
            "tags": ["room-id": "4242"],
        ]
        // Two StreamNook members, so a member's Atmosphere and badge show on fake rows.
        let member: (atmosphere: String, color: String)? = switch name {
        case "glassowl": ("aurora", "#2dd4bf")
        case "midnightmod": ("midnight", "#4ade80")
        default: nil
        }
        var view: [String: Any] = ["spans": [["type": "text", "text": lines[n % lines.count]]]]
        if let member {
            view["name_color"] = member.color
            view["badges"] = [["url": "streamnook-house:streamnook-default", "title": "StreamNook", "provider": "streamnook"]]
            view["atmosphere"] = member.atmosphere
        }
        message["metadata"] = ["view": view]
        return (try? json(message)) ?? "{}"
    }

    /// A stand-in for the core's emote matcher over the fixture's emote set: prefix matches for
    /// the suggestion strip, substring matches (everything, grouped, when nothing is typed) for
    /// the picker.
    static func emoteMatches(_ emotesJSON: Data?, args: [String: Any]) -> [String: Any] {
        let set = emotesJSON.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: [[String: Any]]] } ?? [:]
        let typed = args["query"] as? String ?? ""
        let search = args["profile"] as? String == "search"
        // As the core does: a colon trigger is set aside, and a one-character word typed in
        // passing matches only an emote of exactly that name.
        let bare = typed.hasPrefix(":") ? String(typed.dropFirst()) : typed
        let exactOnly = args["typing"] as? Bool == true && !bare.isEmpty && bare.count < 2
        let query = (search || !typed.hasPrefix(":") ? typed : bare).lowercased()
        var rows: [[String: Any]] = []
        for provider in ["7tv", "bttv", "ffz", "twitch"] {
            for emote in set[provider] ?? [] {
                guard let name = emote["name"] as? String, let url = emote["url"] as? String else { continue }
                let lower = name.lowercased()
                let hit = exactOnly ? name == bare
                    : query.isEmpty ? search : (search ? lower.contains(query) : lower.hasPrefix(query))
                guard hit else { continue }
                var row: [String: Any] = [
                    "id": emote["id"] as? String ?? name, "name": name, "url": url, "provider": provider,
                    "source_label": provider.uppercased(), "source_detail": "Channel",
                ]
                if query.isEmpty { row["group"] = provider == "twitch" ? "Global Emotes" : "Channel Emotes" }
                if let width = emote["width"] { row["width"] = width }
                rows.append(row)
            }
        }
        let limit = args["limit"] as? Int ?? (search ? 60 : 50)
        // Hits per source before the cap, as the core counts them (the fake has no favorites).
        let counts = Dictionary(grouping: rows) { $0["provider"] as? String ?? "" }.mapValues(\.count)
        return ["rows": Array(rows.prefix(limit)), "total": rows.count, "ready": true, "counts": counts]
    }

    /// A stand-in for the core's picker over the fixture's set: the room's tabs with counts, and
    /// the selected tab as one section ("Global Emotes" for Twitch, "Standard" for the rest).
    static func emotePicker(_ emotesJSON: Data?, args: [String: Any]) -> [String: Any] {
        let set = emotesJSON.flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: [[String: Any]]] } ?? [:]
        let ids = ["twitch", "bttv", "7tv", "ffz"]
        // The fake has no favorites: that tab answers with nothing in it.
        if args["tab"] as? String == "favorites" {
            var tabs: [[String: Any]] = [["id": "favorites", "count": 0]]
            tabs += ids.map { ["id": $0, "count": set[$0]?.count ?? 0] }
            return ["ready": emotesJSON != nil, "tab": "favorites", "tabs": tabs, "sections": [Any]()]
        }
        let tab = (args["tab"] as? String).flatMap { ids.contains($0) ? $0 : nil } ?? "twitch"
        let query = (args["query"] as? String ?? "").lowercased()
        let rows = (set[tab] ?? []).filter { emote in
            query.isEmpty || ((emote["name"] as? String)?.lowercased().contains(query) ?? false)
        }.map { emote -> [String: Any] in
            var cell = emote
            cell["provider"] = tab
            cell["favorite"] = false
            cell["locked"] = false
            return cell
        }
        let section: [String: Any] = [
            "key": tab == "twitch" ? "globals" : "standard",
            "title": tab == "twitch" ? "Global Emotes" : "Standard",
            "columns": 7, "emotes": rows,
        ]
        var tabs: [[String: Any]] = [["id": "favorites", "count": 0]]
        tabs += ids.map { ["id": $0, "count": set[$0]?.count ?? 0] }
        return ["ready": emotesJSON != nil, "tab": tab, "tabs": tabs, "sections": rows.isEmpty ? [] : [section]]
    }

    /// A stand-in for the core's slash dispatcher that never writes anywhere: a command names
    /// what it would do in a toast or a notice; `/me` and unknown commands go to chat as typed.
    static func slashOutcome(_ text: String) -> [String: Any] {
        let words = text.dropFirst().split(separator: " ").map(String.init)
        let command = words.first?.lowercased() ?? ""
        switch command {
        case "ban":
            guard words.count > 1 else {
                return ["handled": true, "clear_draft": true, "toasts": [["text": "Usage: /ban <username> [reason]", "kind": "info"]],
                        "notices": ["Usage: /ban <username> [reason]"]]
            }
            return ["handled": true, "clear_draft": true, "toasts": [["text": "Banned \(words[1])", "kind": "success"]], "notices": []]
        case "uptime":
            return ["handled": true, "clear_draft": true, "toasts": [], "notices": ["Live for 1 hour 2 minutes"]]
        case "shrug":
            return ["handled": true, "clear_draft": true, "send": (words.dropFirst().joined(separator: " ") + " ¯\\_(ツ)_/¯")
                        .trimmingCharacters(in: .whitespaces), "toasts": [], "notices": []]
        case "usercard", "user":
            return ["handled": true, "clear_draft": true, "toasts": [], "notices": [],
                    "ui": ["kind": "open_user_card", "login": words.count > 1 ? words[1] : ""]]
        default:
            return ["handled": false, "clear_draft": false, "toasts": [], "notices": []]
        }
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
            gameName: game.name, userID: "\(5_000 + i)", gameID: game.id,
            broadcasterType: i % 3 == 0 ? "partner" : nil
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
