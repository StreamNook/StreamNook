import Foundation

/// One kind of alert this iPhone sends, as the core lists them (`get_alert_types`,
/// services/alert_types.rs): the row's words, the setting its switch writes, the notification
/// category it posts under, and whether the setup wizard offers it. Android draws its
/// Notifications section and setup step from the same list, so both phones say the same thing.
/// A type this iPhone cannot send is not in the list.
public struct AlertType: Decodable, Sendable, Equatable, Identifiable {
    /// `live`, `mentions`, `drops`, `badges`.
    public let id: String
    public let title: String
    public let detail: String
    /// The dotted settings path the switch writes (`live_notifications.show_mention_notifications`).
    public let setting: String
    /// The notification category it posts under.
    public let channel: String
    /// Offered in the setup wizard's Notifications step.
    public let setup: Bool

    public init(id: String, title: String, detail: String, setting: String, channel: String, setup: Bool) {
        self.id = id
        self.title = title
        self.detail = detail
        self.setting = setting
        self.channel = channel
        self.setup = setup
    }
}

public enum AlertTypes {
    /// The list for this build, in the order the phones draw it; empty when the core cannot say.
    public static func load(core: CoreBridge) async -> [AlertType] {
        (try? await core.invoke("get_alert_types", args: [:], as: [AlertType].self)) ?? []
    }

    /// What the setup step writes when it is left forward: each offered type as the step showed
    /// it (on only when chosen), and the master switch on when any is chosen.
    public static func setupChoices(_ types: [AlertType], chosen: Set<String>) -> [String: Any?] {
        var changes: [String: Any?] = [:]
        for type in types where type.setup {
            changes[type.setting] = chosen.contains(type.id)
        }
        if types.contains(where: { $0.setup && chosen.contains($0.id) }) {
            changes["live_notifications.enabled"] = true
        }
        return changes
    }

    /// The list the fake core answers: the iPhone's today, go-lives (pushed) and mentions.
    static let fakeJSON = #"""
    [{"id":"live","title":"Live channels","detail":"When a channel you follow goes live","setting":"live_notifications.show_live_notifications","channel":"live-channels","setup":true},
     {"id":"mentions","title":"Mentions and replies","detail":"When someone mentions or replies to you","setting":"live_notifications.show_mention_notifications","channel":"mentions","setup":true}]
    """#
}
