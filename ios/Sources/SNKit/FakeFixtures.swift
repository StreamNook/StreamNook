import Foundation
import Synchronization

/// FakeCore's answers for the app's screens: the settings the look reads, the account, the home
/// snapshot and search. `signedIn` decides whether a Twitch account is there.
public final class FakeFixtures: Sendable {
    private struct State {
        var signedIn: Bool
        var settings: [String: String]
    }

    private let state: Mutex<State>
    private let cosmetics = FakeCosmetics()
    /// Drops, the badge wall and channel points: canned, and every write only recorded.
    public let rewards = FakeRewards()
    /// The mentions inbox: canned rows, every change announced.
    public let mentions = FakeMentions()
    /// The stream manager on the fixture account's own channel: writes change only the fake.
    public let streamManager = FakeStreamManager()

    public init(signedIn: Bool) {
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

    /// The answer to `cmd`, or nil when the fixtures do not know it.
    func answer(_ cmd: String, _ args: [String: Any]) throws -> String? {
        switch cmd {
        case "load_settings":
            let pairs = state.withLock { $0.settings }.map { "\"\($0.key)\":\($0.value)" }.sorted()
            return "{" + pairs.joined(separator: ",") + "}"
        case "patch_settings":
            guard let patch = args["patch"] as? [String: Any] else { return "null" }
            let encoded = patch.compactMapValues { value -> String? in
                guard let data = try? JSONSerialization.data(withJSONObject: value, options: [.fragmentsAllowed]) else { return nil }
                return String(decoding: data, as: UTF8.self)
            }
            state.withLock { $0.settings.merge(encoded) { $1 } }
            return "null"
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
            return "false"
        case "refresh_home_section", "set_home_extra_channels":
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
