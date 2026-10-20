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
    /// The channel's own page on its platform (its live page, or the video), for a chat this app
    /// cannot show. Nil on entries filed before it.
    public var watchURL: String?
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
    /// The message as the chat drew it, words and emotes in order. Empty on older entries and on a
    /// message with no emote: show `text`.
    public var parts: [MentionPart] = []

    enum CodingKeys: String, CodingKey {
        case id, platform, room, text, kind, read, removed, parts
        case openKey = "open_key", watchURL = "watch_url"
        case channelLogin = "channel_login", channelID = "channel_id"
        case senderID = "sender_id", senderLogin = "sender_login", senderName = "sender_name"
        case senderColor = "sender_color", replyParentID = "reply_parent_id", atMs = "at_ms"
    }

    public init(id: String, platform: String = "twitch", room: String, openKey: String? = nil, watchURL: String? = nil,
                channelLogin: String, channelID: String? = nil,
                senderID: String = "", senderLogin: String, senderName: String, senderColor: String? = nil,
                text: String, kind: Kind = .mention, replyParentID: String? = nil, atMs: Int64, read: Bool = false,
                removed: Bool = false) {
        self.id = id
        self.platform = platform
        self.room = room
        self.openKey = openKey
        self.watchURL = watchURL
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
        watchURL = try c.decodeIfPresent(String.self, forKey: .watchURL)
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
        parts = (try? c.decodeIfPresent([MentionPart].self, forKey: .parts)) ?? []
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

/// One piece of a filed message: words, or an emote drawn where it was said.
public enum MentionPart: Codable, Sendable, Equatable, Hashable {
    case text(String)
    case emote(name: String, url: String)

    enum CodingKeys: String, CodingKey { case type, text, name, url }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        switch try c.decodeIfPresent(String.self, forKey: .type) {
        case "emote":
            self = .emote(name: try c.decodeIfPresent(String.self, forKey: .name) ?? "",
                          url: try c.decodeIfPresent(String.self, forKey: .url) ?? "")
        default:
            self = .text(try c.decodeIfPresent(String.self, forKey: .text) ?? "")
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .text(let text):
            try c.encode("text", forKey: .type)
            try c.encode(text, forKey: .text)
        case .emote(let name, let url):
            try c.encode("emote", forKey: .type)
            try c.encode(name, forKey: .name)
            try c.encode(url, forKey: .url)
        }
    }
}

/// The phones' inbox chips. `whispers` arrives once the whisper store joins the core's view; a
/// chip the app does not know is still listed by its id.
public enum InboxFilterID: String, Sendable, CaseIterable {
    case all, mentions, highlights, whispers
}

/// One chip of `get_inbox_view`, with how many unread rows it holds.
public struct InboxChip: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var unread: Int

    public init(id: String, unread: Int) {
        self.id = id
        self.unread = unread
    }
}

/// A message folded under its row: the same words from the same sender, right after it.
public struct InboxFolded: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    public var atMs: Int64
    public var read: Bool

    enum CodingKeys: String, CodingKey { case id, read, atMs = "at_ms" }

    public init(id: String, atMs: Int64, read: Bool) {
        self.id = id
        self.atMs = atMs
        self.read = read
    }
}

/// One row of `get_inbox_view`: the newest of a run of identical messages, the rest folded under
/// it, and the sender's picture once the core has it.
public struct InboxRowView: Decodable, Sendable, Equatable, Identifiable {
    public var entry: MentionEntry
    public var folded: [InboxFolded]
    public var unread: Bool
    public var avatar: String?

    public var id: String { entry.id }
    /// Every message the row stands for, newest first.
    public var ids: [String] { [entry.id] + folded.map(\.id) }

    public init(entry: MentionEntry, folded: [InboxFolded] = [], unread: Bool, avatar: String? = nil) {
        self.entry = entry
        self.folded = folded
        self.unread = unread
        self.avatar = avatar
    }
}

/// `get_inbox_view`: one page of the timeline for a chip, newest first, built by the core
/// (filtered, identical messages folded, pictures attached).
public struct InboxView: Decodable, Sendable, Equatable {
    public var rows: [InboxRowView]
    public var hasMore: Bool
    /// What `beforeMs` asks for to get the next page.
    public var beforeMs: Int64?
    public var unreadTotal: Int
    public var chips: [InboxChip]

    enum CodingKeys: String, CodingKey {
        case rows, chips, hasMore = "has_more", beforeMs = "before_ms", unreadTotal = "unread_total"
    }

    public init(rows: [InboxRowView], hasMore: Bool, beforeMs: Int64?, unreadTotal: Int, chips: [InboxChip]) {
        self.rows = rows
        self.hasMore = hasMore
        self.beforeMs = beforeMs
        self.unreadTotal = unreadTotal
        self.chips = chips
    }

    /// A later page under the rows held (a row already held is not repeated).
    public mutating func append(_ page: InboxView) {
        let have = Set(rows.map(\.id))
        rows += page.rows.filter { !have.contains($0.id) }
        hasMore = page.hasMore
        beforeMs = page.beforeMs
        unreadTotal = page.unreadTotal
        chips = page.chips
    }

    /// More than one platform in the rows: they then wear their platform's logo.
    public var mixedPlatforms: Bool { Set(rows.map(\.entry.platform)).count > 1 }
}

/// `mention-arrived`: a live entry just filed unread (its chat is not on screen), for the card
/// over the message bar.
public struct MentionArrival: Decodable, Sendable, Equatable {
    public var entry: MentionEntry
    public var avatar: String?

    public init(entry: MentionEntry, avatar: String?) {
        self.entry = entry
        self.avatar = avatar
    }
}

/// The inbox's words, the same as Android's (mentions/inboxText.ts).
public enum InboxText {
    public static func chip(_ id: String) -> String {
        switch InboxFilterID(rawValue: id) {
        case .all: "All"
        case .mentions: "Mentions"
        case .highlights: "Highlights"
        case .whispers: "Whispers"
        case nil: id.capitalized
        }
    }

    /// The empty list's words for a chip.
    public static func empty(_ id: String) -> (text: String, hint: String) {
        switch InboxFilterID(rawValue: id) {
        case .highlights: ("No highlights yet.", "Messages with one of your highlight phrases show up here.")
        case .whispers: ("No whispers yet.", "Whispers sent to you show up here.")
        case .mentions: ("No mentions yet.", "When someone mentions or replies to you in a chat, it shows up here.")
        default: ("Nothing here yet.", "Mentions, replies and your highlight phrases show up here.")
        }
    }

    /// "6 identical mentions", the fold's line under its row (`count` includes the row itself).
    public static func folded(_ entry: MentionEntry, count: Int) -> String {
        let noun = switch entry.kind {
        case .reply: "replies"
        case .word: "messages"
        case .mention: "mentions"
        }
        return "\(count) identical \(noun)"
    }

    /// The chat a row was said in, by its channel name rather than the room key.
    public static func place(_ entry: MentionEntry) -> String {
        entry.channelLogin.isEmpty ? MentionText.channel(ofRoom: entry.room) : entry.channelLogin
    }

    /// The picture cache's key for a sender, as the core's `avatar_request` builds it: Twitch and
    /// YouTube ids, Kick slugs. Nil where the core looks none up.
    public static func avatarKey(_ entry: MentionEntry) -> String? {
        let id: String = switch entry.platform {
        case "kick": entry.senderLogin
        case "twitch", "youtube": entry.senderID
        default: ""
        }
        return id.isEmpty ? nil : "\(entry.platform):\(id.lowercased())"
    }
}
