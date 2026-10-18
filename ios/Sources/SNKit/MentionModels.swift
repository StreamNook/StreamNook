import Foundation

/// One entry of the mentions inbox, as the core's `services::mention_inbox::MentionEntry` sends
/// it. The core files, dedupes, prunes and marks them; the app only shows them.
public struct MentionEntry: Codable, Sendable, Equatable, Hashable, Identifiable {
    public enum Kind: String, Codable, Sendable {
        /// Your login or display name.
        case mention
        /// A reply to one of your messages.
        case reply
        /// One of your highlight phrases.
        case word
    }

    /// The platform's message id.
    public var id: String
    /// `twitch`, `kick`, `youtube`, ...
    public var platform: String
    /// The chat tab key: the bare Twitch login, `provider:slug` otherwise.
    public var room: String
    /// The key that opens the chat: the bare Twitch login, else `provider:channel` in the
    /// channel's own case (YouTube ids are case-sensitive). Nil on entries filed before it.
    public var openKey: String?
    /// The channel as chat addresses it (login or slug).
    public var channelLogin: String
    public var channelID: String?
    public var senderID: String
    public var senderLogin: String
    public var senderName: String
    public var senderColor: String?
    public var text: String
    public var kind: Kind
    public var replyParentID: String?
    public var atMs: Int64
    public var read: Bool
    /// A moderator deleted the message after it was filed.
    public var removed: Bool

    enum CodingKeys: String, CodingKey {
        case id, platform, room, text, kind, read, removed
        case openKey = "open_key"
        case channelLogin = "channel_login", channelID = "channel_id"
        case senderID = "sender_id", senderLogin = "sender_login", senderName = "sender_name"
        case senderColor = "sender_color", replyParentID = "reply_parent_id", atMs = "at_ms"
    }

    public init(id: String, platform: String = "twitch", room: String, openKey: String? = nil, channelLogin: String, channelID: String? = nil,
                senderID: String = "", senderLogin: String, senderName: String, senderColor: String? = nil,
                text: String, kind: Kind = .mention, replyParentID: String? = nil, atMs: Int64, read: Bool = false,
                removed: Bool = false) {
        self.id = id
        self.platform = platform
        self.room = room
        self.openKey = openKey
        self.channelLogin = channelLogin
        self.channelID = channelID
        self.senderID = senderID
        self.senderLogin = senderLogin
        self.senderName = senderName
        self.senderColor = senderColor
        self.text = text
        self.kind = kind
        self.replyParentID = replyParentID
        self.atMs = atMs
        self.read = read
        self.removed = removed
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        platform = try c.decodeIfPresent(String.self, forKey: .platform) ?? "twitch"
        room = try c.decode(String.self, forKey: .room)
        openKey = try c.decodeIfPresent(String.self, forKey: .openKey)
        channelLogin = try c.decodeIfPresent(String.self, forKey: .channelLogin) ?? room
        channelID = try c.decodeIfPresent(String.self, forKey: .channelID)
        senderID = try c.decodeIfPresent(String.self, forKey: .senderID) ?? ""
        senderLogin = try c.decodeIfPresent(String.self, forKey: .senderLogin) ?? ""
        senderName = try c.decodeIfPresent(String.self, forKey: .senderName) ?? senderLogin
        senderColor = try c.decodeIfPresent(String.self, forKey: .senderColor)
        text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
        kind = (try? c.decode(Kind.self, forKey: .kind)) ?? .mention
        replyParentID = try c.decodeIfPresent(String.self, forKey: .replyParentID)
        atMs = try c.decodeIfPresent(Int64.self, forKey: .atMs) ?? 0
        read = try c.decodeIfPresent(Bool.self, forKey: .read) ?? false
        removed = try c.decodeIfPresent(Bool.self, forKey: .removed) ?? false
    }

    /// What opens the chat: `open_key`, or the room key on an entry filed before it existed.
    public var chatKey: String { openKey.flatMap { $0.isEmpty ? nil : $0 } ?? room }

    /// The row's kind words: "mentioned you", "replied to you", "highlight".
    public var kindPhrase: String {
        switch kind {
        case .mention: "mentioned you"
        case .reply: "replied to you"
        case .word: "highlight"
        }
    }

    /// The name shown for the sender, never empty.
    public var shownSender: String { senderName.isEmpty ? (senderLogin.isEmpty ? "Someone" : senderLogin) : senderName }

    /// What the row quotes: the message, or why it is gone.
    public var shownText: String { removed ? "Message deleted by a moderator" : text }
}

/// One page of `get_mention_inbox`: newest first.
public struct MentionPage: Decodable, Sendable, Equatable {
    public var entries: [MentionEntry]
    public var hasMore: Bool
    public var unreadTotal: Int

    enum CodingKeys: String, CodingKey {
        case entries, hasMore = "has_more", unreadTotal = "unread_total"
    }

    public init(entries: [MentionEntry], hasMore: Bool, unreadTotal: Int) {
        self.entries = entries
        self.hasMore = hasMore
        self.unreadTotal = unreadTotal
    }
}

/// The `mention-inbox-changed` payload.
public struct MentionInboxChanged: Decodable, Sendable, Equatable {
    public var unreadTotal: Int
    enum CodingKeys: String, CodingKey { case unreadTotal = "unread_total" }

    public static func parse(_ json: String) -> MentionInboxChanged? {
        try? JSONDecoder().decode(MentionInboxChanged.self, from: Data(json.utf8))
    }
}

/// The pages of the inbox the Mentions screen holds. Pages come from the core; a fresh first page
/// (after a change) keeps the older pages already loaded below it, and local marks show at once
/// until the core's next page replaces them.
public struct MentionList: Sendable, Equatable {
    public private(set) var entries: [MentionEntry] = []
    public private(set) var hasMore = false
    public private(set) var unreadTotal = 0
    /// A first page arrived at least once.
    public private(set) var loaded = false

    public init() {}

    /// How many rows a page asks for.
    public static let pageSize = 50

    /// The `beforeMs` that asks for the page after the rows held, nil when there is none.
    public var nextBefore: Int64? { hasMore ? entries.last?.atMs : nil }

    /// More than one platform in the list: rows then wear their platform's logo.
    public var mixedPlatforms: Bool { Set(entries.map(\.platform)).count > 1 }

    /// A first page (newest first). Older rows loaded earlier stay under it while the core says
    /// there is more past this page.
    public mutating func takeFirst(_ page: MentionPage) {
        loaded = true
        unreadTotal = page.unreadTotal
        guard page.hasMore, let oldest = page.entries.last?.atMs else {
            entries = page.entries
            hasMore = page.hasMore
            return
        }
        let ids = Set(page.entries.map(\.id))
        let older = entries.filter { $0.atMs < oldest && !ids.contains($0.id) }
        entries = page.entries + older
        hasMore = older.isEmpty ? true : hasMore
    }

    /// A later page, under the rows held.
    public mutating func takeMore(_ page: MentionPage) {
        let ids = Set(entries.map(\.id))
        entries += page.entries.filter { !ids.contains($0.id) }
        hasMore = page.hasMore
        unreadTotal = page.unreadTotal
    }

    public mutating func setUnread(_ total: Int) { unreadTotal = total }

    /// These rows read (all when nil), shown before the core confirms.
    public mutating func markRead(_ ids: [String]?) {
        let wanted = ids.map { Set($0) }
        var changed = 0
        for i in entries.indices where !entries[i].read && (wanted?.contains(entries[i].id) ?? true) {
            entries[i].read = true
            changed += 1
        }
        unreadTotal = wanted == nil ? 0 : max(0, unreadTotal - changed)
    }

    public mutating func remove(_ id: String) {
        guard let i = entries.firstIndex(where: { $0.id == id }) else { return }
        if !entries[i].read { unreadTotal = max(0, unreadTotal - 1) }
        entries.remove(at: i)
    }

    public mutating func clear() {
        entries = []
        hasMore = false
        unreadTotal = 0
    }
}

/// The words a mention is shown in, outside its row.
public enum MentionText {
    /// The notification title, as the desktop's: "Viewer replied to you in somechannel",
    /// "Viewer mentioned you in somechannel", "Viewer in somechannel" for a highlight phrase.
    public static func notificationTitle(_ entry: MentionEntry) -> String {
        switch entry.kind {
        case .reply: "\(entry.shownSender) replied to you in \(entry.channelLogin)"
        case .mention: "\(entry.shownSender) mentioned you in \(entry.channelLogin)"
        case .word: "\(entry.shownSender) in \(entry.channelLogin)"
        }
    }

    /// A row's time: "now", "5m", "3h", "2d", then the date ("Oct 3", with the year when it is
    /// not this year's).
    public static func time(_ atMs: Int64, nowMs: Int64, calendar: Calendar = .current) -> String {
        let seconds = max(0, (nowMs - atMs) / 1000)
        if seconds < 60 { return "now" }
        if seconds < 3600 { return "\(seconds / 60)m" }
        if seconds < 86_400 { return "\(seconds / 3600)h" }
        if seconds < 7 * 86_400 { return "\(seconds / 86_400)d" }
        let date = Date(timeIntervalSince1970: TimeInterval(atMs) / 1000)
        let now = Date(timeIntervalSince1970: TimeInterval(nowMs) / 1000)
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = calendar.component(.year, from: date) == calendar.component(.year, from: now) ? "MMM d" : "MMM d, yyyy"
        return formatter.string(from: date)
    }

    /// The platform a room key names: `kick:slug` is Kick, a bare login Twitch.
    public static func platform(ofRoom room: String) -> String {
        guard let colon = room.firstIndex(of: ":") else { return "twitch" }
        return String(room[..<colon])
    }

    /// The channel a room key names, without its platform.
    public static func channel(ofRoom room: String) -> String {
        guard let colon = room.firstIndex(of: ":") else { return room }
        return String(room[room.index(after: colon)...])
    }
}

