import Foundation

/// One chat event kind a platform sends, as the settings page offers it: the key the core stamps
/// on such a row (`ChatRowView.event`) and the chip's words.
public struct ChatEventKind: Sendable, Equatable, Hashable {
    public let key: String
    public let label: String
}

/// A platform's line of event kinds on Chat > Chat Events > Platforms.
public struct ChatEventPlatform: Sendable, Equatable, Identifiable {
    public let id: String
    public let label: String
    public let kinds: [ChatEventKind]
}

/// The event kinds each platform sends, in the same keys and words as the desktop and Android
/// page (`PROVIDER_EVENT_CATEGORIES` in the web overlay config), so a kind turned off on one
/// device stays off on every other.
public enum ChatEventKinds {
    private static func line(_ id: String, _ label: String, _ kinds: [(String, String)]) -> ChatEventPlatform {
        ChatEventPlatform(id: id, label: label, kinds: kinds.map { ChatEventKind(key: "\(id):\($0.0)", label: $0.1) })
    }

    public static let all: [ChatEventPlatform] = [
        line("twitch", "Twitch", [("subscription", "Subscriptions"), ("gift", "Gifts"), ("cheer", "Bits"), ("raid", "Raids"),
                                  ("milestone", "Milestones"), ("redemption", "Channel points"),
                                  ("announcement", "Announcements")]),
        line("kick", "Kick", [("subscription", "Subscriptions"), ("gift", "Gifts"), ("follow", "Follows"), ("raid", "Raids")]),
        line("youtube", "YouTube", [("subscription", "Subscriptions"), ("gift", "Gifts"), ("cheer", "Super Chats")]),
        line("tiktok", "TikTok", [("gift", "Gifts"), ("cheer", "Coins"), ("follow", "Follows")]),
    ]

    /// The lines for the platforms this build can watch (the roster's `can_watch`), in order.
    public static func platforms(watchable: [String]) -> [ChatEventPlatform] {
        all.filter { watchable.contains($0.id) }
    }
}
