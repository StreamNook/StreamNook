import Foundation

/// Go-live alerts on the iPhone arrive by push: notify.streamnook.app sends an APNs alert when a
/// channel the core registered for this device goes live. The core decides which channels those
/// are and keeps the server in step (`services/push_alerts.rs`); the app only hands over this
/// device's token, presents the alert and opens the stream from a tap. A background refresh is
/// the fallback: the core runs Android's live check (`services/live_poll.rs`) and the app posts
/// what it finds under the push's own identifier, so the two replace each other. These are the
/// parts of that with no UIKit in them.
public enum LivePush {
    /// The background refresh that runs the live check. Info.plist permits it.
    public static let refreshTask = "app.streamnook.live-check"
    /// The soonest a refresh may start after it is scheduled; iOS picks the real time.
    public static let refreshInterval: TimeInterval = 15 * 60

    /// A go-live push, as the server sends it under the `sn` key (`type` is `live`). `polled` marks
    /// one the app posted from the live check rather than one the server pushed.
    public struct Alert: Equatable, Sendable {
        public let channelID: String
        public let login: String
        public let startedAt: String?
        public let polled: Bool

        public init(channelID: String, login: String, startedAt: String? = nil, polled: Bool = false) {
            self.channelID = channelID
            self.login = login
            self.startedAt = startedAt
            self.polled = polled
        }
    }

    /// The go-live alert a notification carries, or nil for any other notification.
    public static func alert(in userInfo: [AnyHashable: Any]) -> Alert? {
        guard let sn = userInfo["sn"] as? [String: Any], sn["type"] as? String == "live",
              let login = sn["login"] as? String, !login.isEmpty else { return nil }
        return Alert(channelID: sn["channel_id"] as? String ?? "", login: login, startedAt: sn["started_at"] as? String,
                     polled: sn["source"] as? String == "poll")
    }

    /// A channel's go-live notification identifier and thread: the push's collapse id, so a push
    /// and a polled alert for the same channel replace each other instead of stacking.
    public static func notificationID(channelID: String) -> String {
        "live-\(channelID)"
    }

    /// A go-live notification as the core's live check reads it (`live_poll_check` and
    /// `live_poll_seen`): its identifier, the broadcast's start time, when it was delivered, and
    /// whether the app posted it. Nil for anything that is not a go-live.
    public static func sighting(identifier: String, userInfo: [AnyHashable: Any], date: Date?) -> [String: Any]? {
        let alert = alert(in: userInfo)
        let id: String
        if identifier.hasPrefix("live-") {
            id = identifier
        } else if let channelID = alert?.channelID, !channelID.isEmpty {
            id = notificationID(channelID: channelID)
        } else {
            return nil
        }
        var out: [String: Any] = ["id": id, "polled": alert?.polled ?? false]
        if let startedAt = alert?.startedAt, !startedAt.isEmpty { out["started_at"] = startedAt }
        if let date { out["date"] = Int(date.timeIntervalSince1970) }
        return out
    }

    /// One go-live the live check found, worded as the push words it.
    public struct Found: Decodable, Equatable, Sendable {
        public let channelID: String
        public let login: String
        public let startedAt: String
        public let title: String
        public let body: String

        public init(channelID: String, login: String, startedAt: String, title: String, body: String) {
            self.channelID = channelID
            self.login = login
            self.startedAt = startedAt
            self.title = title
            self.body = body
        }

        enum CodingKeys: String, CodingKey {
            case channelID = "channel_id", login, startedAt = "started_at", title, body
        }
    }

    /// What one live check answered: the go-lives to post (already recorded as shown), and whether
    /// go-live alerts are on at all; while they are off no further refresh is asked for.
    public struct Check: Decodable, Equatable, Sendable {
        public let active: Bool
        public let alerts: [Found]
    }

    /// The `sn` payload a polled alert carries: the push's, marked as the app's own.
    public static func userInfo(_ found: Found) -> [String: Any] {
        ["sn": ["type": "live", "channel_id": found.channelID, "login": found.login, "started_at": found.startedAt,
                "source": "poll"]]
    }

    /// Whether the alert is about the Twitch channel already playing, by id when both have one and
    /// by login otherwise. That banner is left out: the viewer is watching it.
    public static func isPlaying(_ alert: Alert, login: String?, channelID: String?) -> Bool {
        if let channelID, !channelID.isEmpty, !alert.channelID.isEmpty { return channelID == alert.channelID }
        guard let login else { return false }
        return login.caseInsensitiveCompare(alert.login) == .orderedSame
    }

    /// Whether the iPhone should ask for a push token: notifications allowed, the master switch on,
    /// and a go-live type the core lists for this iPhone switched on. `isOn` reads a saved switch.
    public static func wantsToken(types: [AlertType], allowed: Bool, isOn: (String) -> Bool) -> Bool {
        guard allowed, isOn("live_notifications.enabled"),
              let live = types.first(where: { $0.id == "live" }) else { return false }
        return isOn(live.setting)
    }

    /// The device token as the server takes it: lowercase hex.
    public static func hex(_ token: Data) -> String {
        token.map { String(format: "%02x", $0) }.joined()
    }

    /// The APNs environment a token belongs to, from the provisioning profile the build was signed
    /// with (`embedded.mobileprovision`). A development-signed build (xtool's dev profile, its
    /// entitlement `aps-environment` = `development`) gets sandbox tokens; a distribution-signed
    /// one gets production tokens, and an App Store or TestFlight install carries no profile at
    /// all. The entitlement is what decides it, so this reads the entitlement rather than guessing
    /// from the build configuration.
    public static func environment(profile: Data?) -> String {
        guard let profile,
              let start = profile.range(of: Data("<?xml".utf8)),
              let end = profile.range(of: Data("</plist>".utf8), in: start.lowerBound..<profile.endIndex),
              let plist = try? PropertyListSerialization.propertyList(from: profile[start.lowerBound..<end.upperBound], format: nil)
                as? [String: Any],
              let entitlements = plist["Entitlements"] as? [String: Any],
              entitlements["aps-environment"] as? String == "development" else { return "production" }
        return "sandbox"
    }
}
