import Foundation

/// One open chat room as the core keeps the phone's tabs (Rust `services::chat_tabs::ChatTab`).
public struct ChatTabInfo: Codable, Sendable, Equatable, Hashable, Identifiable {
    /// The room's key: the bare lowercased login on Twitch, `provider:slug` otherwise.
    public var channel: String
    public var provider: String
    /// The name the platform addresses the room by.
    public var login: String
    /// The numeric Twitch id; nil on other platforms.
    public var channelID: String?
    public var label: String
    public var avatar: String?
    /// Follows the watched stream: first, and not closable.
    public var pinnedToStream: Bool

    public var id: String { channel }

    enum CodingKeys: String, CodingKey {
        case channel, provider, login, channelID = "channel_id", label, avatar, pinnedToStream = "pinned_to_stream"
    }

    public init(channel: String, provider: String = "twitch", login: String, channelID: String? = nil, label: String,
                avatar: String? = nil, pinnedToStream: Bool = false) {
        self.channel = channel
        self.provider = provider
        self.login = login
        self.channelID = channelID
        self.label = label
        self.avatar = avatar
        self.pinnedToStream = pinnedToStream
    }
}

/// The tabs as the core answers them (Rust `ChatTabsView`), plus, for a change, the rooms the
/// shell must join and leave (Rust `ChatTabsChange`).
public struct ChatTabsState: Codable, Sendable, Equatable {
    public var tabs: [ChatTabInfo]
    public var activeChannel: String?
    /// The open rooms span platforms, so non-Twitch tabs wear a provider mark.
    public var mixedPlatforms: Bool
    /// Grows with every change; an answer older than the one shown is dropped.
    public var revision: UInt64
    public var acquire: [ChatTabInfo]
    public var release: [ChatTabInfo]

    enum CodingKeys: String, CodingKey {
        case tabs, activeChannel = "active_channel", mixedPlatforms = "mixed_platforms", revision, acquire, release
    }

    public init(tabs: [ChatTabInfo] = [], activeChannel: String? = nil, mixedPlatforms: Bool = false, revision: UInt64 = 0,
                acquire: [ChatTabInfo] = [], release: [ChatTabInfo] = []) {
        self.tabs = tabs
        self.activeChannel = activeChannel
        self.mixedPlatforms = mixedPlatforms
        self.revision = revision
        self.acquire = acquire
        self.release = release
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        tabs = try c.decodeIfPresent([ChatTabInfo].self, forKey: .tabs) ?? []
        activeChannel = try c.decodeIfPresent(String.self, forKey: .activeChannel)
        mixedPlatforms = try c.decodeIfPresent(Bool.self, forKey: .mixedPlatforms) ?? false
        revision = try c.decodeIfPresent(UInt64.self, forKey: .revision) ?? 0
        acquire = try c.decodeIfPresent([ChatTabInfo].self, forKey: .acquire) ?? []
        release = try c.decodeIfPresent([ChatTabInfo].self, forKey: .release) ?? []
    }

    /// The tab on screen.
    public var active: ChatTabInfo? { tabs.first { $0.channel == activeChannel } }
}

/// One channel the "Add a chat" sheet lists (Rust `ChatRoomRow`).
public struct ChatRoomRow: Codable, Sendable, Equatable, Hashable, Identifiable {
    public var provider: String
    /// The key the room's tab would get.
    public var key: String
    public var userLogin: String
    public var userID: String
    public var userName: String
    public var profileImageURL: String?
    public var isLive: Bool
    public var viewerCount: Int
    public var gameName: String
    /// From a pasted link, not a search.
    public var pasted: Bool

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case provider, key, userLogin = "user_login", userID = "user_id", userName = "user_name"
        case profileImageURL = "profile_image_url", isLive = "is_live", viewerCount = "viewer_count"
        case gameName = "game_name", pasted
    }

    public init(provider: String = "twitch", key: String, userLogin: String, userID: String = "", userName: String,
                profileImageURL: String? = nil, isLive: Bool, viewerCount: Int = 0, gameName: String = "",
                pasted: Bool = false) {
        self.provider = provider
        self.key = key
        self.userLogin = userLogin
        self.userID = userID
        self.userName = userName
        self.profileImageURL = profileImageURL
        self.isLive = isLive
        self.viewerCount = viewerCount
        self.gameName = gameName
        self.pasted = pasted
    }

    /// A followed live stream as a row (the sheet's "Live now" list). Rust stamps `is_live` on
    /// these; a follow without it counts as live, as the phone always read them.
    public init(followed s: TwitchStream) {
        self.init(key: s.userLogin.lowercased(), userLogin: s.userLogin.lowercased(), userID: s.userID ?? "",
                  userName: s.userName.isEmpty ? s.userLogin : s.userName, profileImageURL: s.profileImageURL,
                  isLive: s.isLive ?? true, viewerCount: s.viewerCount, gameName: s.gameName)
    }

    /// The line under the name: the game (or "Live") while live, "Offline" when not.
    public var subline: String {
        if pasted { return "Open this Kick chat" }
        if isLive { return gameName.isEmpty ? "Live" : gameName }
        return "Offline"
    }
}

/// How a pinned message's sender is drawn, worked out by the core (`sender_view`).
public struct PinnedSenderView: Codable, Sendable, Equatable {
    public var nameColor: String
    /// Every Twitch badge, in chat order.
    public var badges: [ChatViewBadge]
    /// The one badge a collapsed banner shows: broadcaster, staff, moderator or VIP.
    public var roleBadge: ChatViewBadge?
    public var paint: ChatPaint?

    enum CodingKeys: String, CodingKey {
        case nameColor = "name_color", badges, roleBadge = "role_badge", paint
    }

    public init(nameColor: String, badges: [ChatViewBadge] = [], roleBadge: ChatViewBadge? = nil, paint: ChatPaint? = nil) {
        self.nameColor = nameColor
        self.badges = badges
        self.roleBadge = roleBadge
        self.paint = paint
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        nameColor = try c.decodeIfPresent(String.self, forKey: .nameColor) ?? "#9147FF"
        badges = (try? c.decodeIfPresent([ChatViewBadge].self, forKey: .badges)) ?? []
        roleBadge = try? c.decodeIfPresent(ChatViewBadge.self, forKey: .roleBadge)
        paint = try? c.decodeIfPresent(ChatPaint.self, forKey: .paint)
    }
}

/// One pinned chat message (the core's pinned-chat read).
public struct PinnedChat: Codable, Sendable, Equatable, Identifiable {
    public var id: String
    public var messageID: String
    public var text: String
    public var senderID: String
    public var senderName: String
    public var senderShownName: String
    public var senderColor: String
    public var senderView: PinnedSenderView?

    enum CodingKeys: String, CodingKey {
        case id, messageID = "message_id", text = "message_text", senderID = "sender_id", senderName = "sender_name"
        case senderShownName = "sender_shown_name", senderColor = "sender_color", senderView = "sender_view"
    }

    public init(id: String, messageID: String = "", text: String, senderID: String = "", senderName: String,
                senderShownName: String = "", senderColor: String = "#9147FF", senderView: PinnedSenderView? = nil) {
        self.id = id
        self.messageID = messageID
        self.text = text
        self.senderID = senderID
        self.senderName = senderName
        self.senderShownName = senderShownName
        self.senderColor = senderColor
        self.senderView = senderView
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(String.self, forKey: .id) ?? UUID().uuidString
        messageID = try c.decodeIfPresent(String.self, forKey: .messageID) ?? ""
        text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
        senderID = try c.decodeIfPresent(String.self, forKey: .senderID) ?? ""
        senderName = try c.decodeIfPresent(String.self, forKey: .senderName) ?? ""
        senderShownName = try c.decodeIfPresent(String.self, forKey: .senderShownName) ?? ""
        senderColor = try c.decodeIfPresent(String.self, forKey: .senderColor) ?? "#9147FF"
        senderView = try? c.decodeIfPresent(PinnedSenderView.self, forKey: .senderView)
    }

    /// The name the sender's chat rows show.
    public var shownName: String { senderShownName.isEmpty ? senderName : senderShownName }
}

/// A `channel-state` push carrying a room's pins (Rust `ChannelUpdate::Pinned`).
public struct PinnedUpdate: Decodable, Sendable, Equatable {
    public var login: String
    public var pinned: [PinnedChat]

    public static func parse(_ json: String) -> PinnedUpdate? {
        guard let data = json.data(using: .utf8),
              let head = try? JSONDecoder().decode(Head.self, from: data), head.section == "pinned" else { return nil }
        return try? JSONDecoder().decode(PinnedUpdate.self, from: data)
    }

    private struct Head: Decodable { var section: String }
}

/// A watch-streak milestone the viewer can share (Twitch's watch-streak read).
public struct WatchStreakMilestone: Codable, Sendable, Equatable {
    public var milestoneID: String
    public var streakCount: Int?

    enum CodingKeys: String, CodingKey { case milestoneID = "milestone_id", streakCount = "streak_count" }

    public init(milestoneID: String, streakCount: Int? = nil) {
        self.milestoneID = milestoneID
        self.streakCount = streakCount
    }

    /// "<n> stream watch streak", or "Watch streak milestone" without a count.
    public var title: String {
        if let n = streakCount, n > 0 { return "\(n) stream watch streak" }
        return "Watch streak milestone"
    }
}

/// A pending resub the viewer can share (Twitch's resub-token read).
public struct ResubNotice: Codable, Sendable, Equatable {
    public var id: String
    public var cumulativeMonths: Int
    public var streakMonths: Int
    public var isGift: Bool
    public var gifter: String?

    enum CodingKeys: String, CodingKey {
        case id, cumulativeMonths = "cumulative_tenure_months", streakMonths = "streak_tenure_months"
        case isGift = "is_gift_subscription", gifter = "gifter_display_name"
    }

    public init(id: String, cumulativeMonths: Int, streakMonths: Int = 0, isGift: Bool = false, gifter: String? = nil) {
        self.id = id
        self.cumulativeMonths = cumulativeMonths
        self.streakMonths = streakMonths
        self.isGift = isGift
        self.gifter = gifter
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        cumulativeMonths = try c.decodeIfPresent(Int.self, forKey: .cumulativeMonths) ?? 0
        streakMonths = try c.decodeIfPresent(Int.self, forKey: .streakMonths) ?? 0
        isGift = try c.decodeIfPresent(Bool.self, forKey: .isGift) ?? false
        gifter = try c.decodeIfPresent(String.self, forKey: .gifter)
    }

    /// "Gift sub from <gifter>" for a gift with a known gifter, else the months subscribed.
    public var title: String {
        if isGift, let gifter, !gifter.isEmpty { return "Gift sub from \(gifter)" }
        return "\(cumulativeMonths) month\(cumulativeMonths == 1 ? "" : "s") subscribed"
    }
}

/// The rooms an outage left a gap in and the window to read back (Rust `chat_outage::Gap`).
public struct ChatGap: Codable, Sendable, Equatable {
    public var channels: [String]
    public var afterMs: Int64
    public var beforeMs: Int64

    enum CodingKeys: String, CodingKey { case channels, afterMs = "after_ms", beforeMs = "before_ms" }

    public init(channels: [String], afterMs: Int64, beforeMs: Int64) {
        self.channels = channels
        self.afterMs = afterMs
        self.beforeMs = beforeMs
    }
}

extension ChatSocketFrame {
    /// The room a frame belongs to, as a tab key (lowercased, no `#`); nil for frames every room
    /// shares (status words, heartbeats).
    public var room: String? {
        let raw: String?
        switch self {
        case .row(let message): raw = message.channel
        case .roomState(let channel, _), .clearMessage(let channel, _), .clearChat(let channel, _, _, _),
             .cosmetics(let channel, _, _, _), .patch(let channel, _), .notice(let channel, _, _):
            raw = channel
        case .status, .other, .member: raw = nil
        }
        guard let raw else { return nil }
        let key = raw.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "#")).lowercased()
        return key.isEmpty ? nil : key
    }
}
