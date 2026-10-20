import Foundation
import Synchronization

/// FakeCore's answers for the app's screens: the settings the look reads, the account, the home
/// snapshot and search. `signedIn` decides whether a Twitch account is there.
public final class FakeFixtures: Sendable {
    private struct State {
        var signedIn: Bool
        var settings: [String: String]
        /// The paths a write changed, which the settings pages' reset arrows follow.
        var changed: Set<String> = []
    }

    private let state: Mutex<State>
    private let cosmetics = FakeCosmetics()
    /// Drops, the badge wall and channel points: canned, and every write only recorded.
    public let rewards = FakeRewards()
    /// The mentions inbox: canned rows, every change announced.
    public let mentions = FakeMentions()
    /// The whisper inbox: canned conversations, sends recorded and never sent anywhere.
    public let whispers: FakeWhispers
    /// The stream manager on the fixture account's own channel: writes change only the fake.
    public let streamManager = FakeStreamManager()

    public init(signedIn: Bool, whispers: FakeWhispers = FakeWhispers()) {
        self.whispers = whispers
        state = Mutex(State(signedIn: signedIn, settings: [
            "theme": #""winters-glass""#,
            "glass_transparency": "100",
            "glass_blur": "true",
            "setup_complete": "true",
        ]))
    }

    public var signedIn: Bool { state.withLock { $0.signedIn } }

    /// Stores or forgets the fixture account, as a sign-in or sign-out in the core would.
    public func setSignedIn(_ signedIn: Bool) {
        state.withLock { $0.signedIn = signedIn }
    }

    static let user = TwitchUser(id: "424242", login: "nookviewer", displayName: "NookViewer")

    /// The paths written since launch and not reset: what `get_settings_view` lists as changed.
    public var changedPaths: [String] { state.withLock { $0.changed }.sorted() }

    /// The saved settings as one object.
    private func settingsRoot() -> [String: Any] {
        state.withLock { $0.settings }.reduce(into: [String: Any]()) { root, pair in
            root[pair.key] = try? JSONSerialization.jsonObject(with: Data(pair.value.utf8), options: [.fragmentsAllowed])
        }
    }

    /// Saves the top-level keys `paths` touched from `root`, dropping a key that became empty.
    private func store(_ root: [String: Any], touching paths: [String]) {
        let tops = Set(paths.compactMap { $0.split(separator: ".").first.map(String.init) })
        state.withLock { state in
            for top in tops {
                if let value = root[top], let data = try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]) {
                    state.settings[top] = String(decoding: data, as: UTF8.self)
                } else {
                    state.settings[top] = nil
                }
            }
        }
    }

    /// Sets (or with nil removes) the value at a dotted path, making the groups above it.
    static func set(_ root: inout [String: Any], _ path: String, _ value: Any?) {
        var parts = path.split(separator: ".").map(String.init)
        guard !parts.isEmpty else { return }
        let first = parts.removeFirst()
        if parts.isEmpty {
            root[first] = value
            return
        }
        var group = root[first] as? [String: Any] ?? [:]
        set(&group, parts.joined(separator: "."), value)
        root[first] = group
    }

    /// Whether two list items are the same value, compared as the JSON they save as.
    private static func same(_ a: Any, _ b: Any) -> Bool {
        let encode = { (value: Any) in try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed, .sortedKeys]) }
        return encode(a).map { $0 == encode(b) } ?? false
    }

    /// One `edit_settings_list` edit on a list, as the core applies it.
    static func edited(_ list: [Any], _ edit: [String: Any]) -> [Any] {
        let id = edit["id"] as? String
        let index = edit["index"] as? Int
        func targets(_ offset: Int, _ item: Any) -> Bool {
            if let index { return offset == index }
            if let id { return (item as? [String: Any])?["id"] as? String == id }
            return false
        }
        switch edit["op"] as? String {
        case "add":
            return list + [edit["item"] ?? ["id": UUID().uuidString]]
        case "remove":
            if let value = edit["value"] { return list.filter { !same($0, value) } }
            return list.enumerated().filter { !targets($0.offset, $0.element) }.map(\.element)
        case "update":
            let patch = edit["patch"] as? [String: Any] ?? [:]
            return list.enumerated().map { offset, item in
                guard targets(offset, item), var row = item as? [String: Any] else { return item }
                for (key, value) in patch { row[key] = value is NSNull ? nil : value }
                return row
            }
        case "toggle":
            guard let value = edit["value"] else { return list }
            let had = list.contains { same($0, value) }
            return had ? list.filter { !same($0, value) } : list + [value]
        case "move":
            guard let from = list.enumerated().first(where: { targets($0.offset, $0.element) })?.offset,
                  let to = edit["to"] as? Int else { return list }
            var moved = list
            let item = moved.remove(at: from)
            moved.insert(item, at: max(0, min(to, moved.count)))
            return moved
        default:
            return list
        }
    }

    /// The core's refusal of a command called without a key it requires, in Tauri's words.
    static func missing(_ key: String, _ cmd: String) -> CoreFailure {
        .command(json: "\"invalid args `\(key)` for command `\(cmd)`: command \(cmd) missing required key \(key)\"")
    }

    /// Whether an edit has the shape the core's `ListEdit` reads: a known op with the keys it needs.
    static func readsAsListEdit(_ edit: [String: Any]) -> Bool {
        switch edit["op"] as? String {
        case "add", "remove": true
        case "update": edit["patch"] is [String: Any]
        case "move": edit["to"] is Int
        case "toggle": edit["value"] != nil
        default: false
        }
    }

    /// The sections `refresh_home_section` knows (`home_snapshot::refresh`).
    static let homeSections: Set<String> = [
        "followed_live", "offline", "recommended", "hype_trains", "collaborations", "shared_chats",
        "watch_streaks", "drops", "continue_watching", "discover",
    ]
    /// The answer to `cmd`, or nil when the fixtures do not know it.
    func answer(_ cmd: String, _ args: [String: Any]) throws -> String? {
        switch cmd {
        case "load_settings":
            let pairs = state.withLock { $0.settings }.map { "\"\($0.key)\":\($0.value)" }.sorted()
            return "{" + pairs.joined(separator: ",") + "}"
        case "patch_settings":
            guard let patch = args["patch"] as? [String: Any] else { throw Self.missing("patch", cmd) }
            let encoded = patch.compactMapValues { value -> String? in
                guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]) else { return nil }
                return String(decoding: data, as: UTF8.self)
            }
            state.withLock { $0.settings.merge(encoded) { $1 } }
            return "null"
        case "set_settings_fields":
            guard let changes = args["changes"] as? [String: Any] else { throw Self.missing("changes", cmd) }
            var root = settingsRoot()
            for (path, value) in changes { Self.set(&root, path, value is NSNull ? nil : value) }
            store(root, touching: Array(changes.keys))
            state.withLock { $0.changed.formUnion(changes.keys) }
            return "null"
        case "reset_settings":
            guard let paths = args["paths"] as? [String] else { throw Self.missing("paths", cmd) }
            var root = settingsRoot()
            for path in paths { Self.set(&root, path, nil) }
            store(root, touching: paths)
            state.withLock { state in
                state.changed = state.changed.filter { c in !paths.contains { c == $0 || c.hasPrefix($0 + ".") || $0.hasPrefix(c + ".") } }
            }
            return "null"
        case "edit_settings_list":
            guard let path = args["path"] as? String else { throw Self.missing("path", cmd) }
            guard let edit = args["edit"] as? [String: Any], Self.readsAsListEdit(edit) else { throw Self.missing("edit", cmd) }
            var root = settingsRoot()
            let list = Self.edited(SettingsPath.value(in: root, path) as? [Any] ?? [], edit)
            Self.set(&root, path, list)
            store(root, touching: [path])
            state.withLock { _ = $0.changed.insert(path) }
            return try FakeCore.json(list)
        case "get_boot_state":
            let (setupDone, signedIn) = state.withLock { ($0.settings["setup_complete"] == "true", $0.signedIn) }
            return try FakeCore.json(["needs_setup": !setupDone, "signed_in": signedIn,
                                      "sign_in_reason": NSNull(), "sign_in_text": NSNull()])
        case "complete_setup":
            let first = state.withLock { state -> Bool in
                defer { state.settings["setup_complete"] = "true" }
                return state.settings["setup_complete"] != "true"
            }
            return try FakeCore.json(["baseline_announcements": first])
        case "has_stored_credentials":
            return signedIn ? "true" : "false"
        case "get_user_info":
            guard signedIn else { throw CoreFailure.command(json: #""Not authenticated""#) }
            return try FakeCore.json(FakeCore.object(UserRow(Self.user)))
        case "sign_out_active_twitch_account":
            state.withLock { $0.signedIn = false }
            return "null"
        case "is_drops_authenticated":
            return rewards.connected ? "true" : "false"
        case "refresh_home_section":
            guard let section = args["section"] as? String else { throw Self.missing("section", cmd) }
            guard Self.homeSections.contains(section) else {
                throw CoreFailure.command(json: "\"unknown home section: \(section)\"")
            }
            return "null"
        case "set_home_extra_channels":
            return "null"
        case "get_home_snapshot":
            let streams = FakeCore.streams.map(\.stream)
            let followed: [Any] = signedIn ? streams.prefix(6).map(FakeCore.object) : []
            let snapshot: [String: Any] = [
                "followed_live": followed,
                "recommended": streams.dropFirst(6).prefix(14).map(FakeCore.object),
                "hype_trains": [["channel_id": streams[1].userID ?? "", "is_active": true, "level": 3, "is_golden_kappa": false]],
                "watch_streaks": [streams[0].userID ?? "": 12],
            ]
            return try FakeCore.json(snapshot)
        case "search_channels":
            let query = (args["query"] as? String ?? "").lowercased()
            let found = FakeCore.streams.map(\.stream).filter { $0.userLogin.contains(query) || $0.title.lowercased().contains(query) }
            return try FakeCore.json(found.map(FakeCore.object))
        case "search_categories":
            let query = (args["query"] as? String ?? "").lowercased()
            let found = FakeCore.categories.filter { $0.name.lowercased().contains(query) }
            return try FakeCore.json(found.map(FakeCore.object))
        default:
            if let answer = try rewards.answer(cmd, args) { return answer }
            if let answer = try mentions.answer(cmd, args) { return answer }
            if let answer = try whispers.answer(cmd, args) { return answer }
            if let answer = try streamManager.answer(cmd, args) { return answer }
            return try cosmetics.answer(cmd, args, userID: Self.user.id)
        }
    }

    /// `get_user_info`'s shape, which TwitchUser only decodes.
    private struct UserRow: Encodable {
        let id: String
        let login: String
        let display_name: String
        let profile_image_url: String?

        init(_ user: TwitchUser) {
            id = user.id
            login = user.login
            display_name = user.displayName
            profile_image_url = user.profileImageURL
        }
    }
}
