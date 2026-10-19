import Foundation

/// Go-live alerts on the iPhone arrive by push: notify.streamnook.app sends an APNs alert when a
/// channel the core registered for this device goes live. The core decides which channels those
/// are and keeps the server in step (`services/push_alerts.rs`); the app only hands over this
/// device's token, presents the alert and opens the stream from a tap. These are the parts of
/// that with no UIKit in them.
public enum LivePush {
    /// A go-live push, as the server sends it under the `sn` key (`type` is `live`).
    public struct Alert: Equatable, Sendable {
        public let channelID: String
        public let login: String
        public let startedAt: String?

        public init(channelID: String, login: String, startedAt: String? = nil) {
            self.channelID = channelID
            self.login = login
            self.startedAt = startedAt
        }
    }

    /// The go-live alert a notification carries, or nil for any other notification.
    public static func alert(in userInfo: [AnyHashable: Any]) -> Alert? {
        guard let sn = userInfo["sn"] as? [String: Any], sn["type"] as? String == "live",
              let login = sn["login"] as? String, !login.isEmpty else { return nil }
        return Alert(channelID: sn["channel_id"] as? String ?? "", login: login, startedAt: sn["started_at"] as? String)
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
