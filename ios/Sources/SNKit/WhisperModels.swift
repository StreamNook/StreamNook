import Foundation

/// One whisper as the core's `services::whisper_storage_service::StoredWhisper` sends it.
public struct WhisperMessage: Codable, Sendable, Equatable, Hashable, Identifiable {
    public var id: String
    public var fromUserID: String
    public var fromUserLogin: String
    public var fromUserName: String
    public var toUserID: String
    public var toUserLogin: String
    public var toUserName: String
    public var message: String
    /// Epoch milliseconds.
    public var timestamp: Int64
    /// You sent it.
    public var isSent: Bool

    enum CodingKeys: String, CodingKey {
        case id, message, timestamp
        case fromUserID = "from_user_id", fromUserLogin = "from_user_login", fromUserName = "from_user_name"
        case toUserID = "to_user_id", toUserLogin = "to_user_login", toUserName = "to_user_name"
        case isSent = "is_sent"
    }

    public init(id: String, fromUserID: String = "", fromUserLogin: String = "", fromUserName: String = "",
                toUserID: String = "", toUserLogin: String = "", toUserName: String = "",
                message: String, timestamp: Int64, isSent: Bool) {
        self.id = id
        self.fromUserID = fromUserID
        self.fromUserLogin = fromUserLogin
        self.fromUserName = fromUserName
        self.toUserID = toUserID
        self.toUserLogin = toUserLogin
        self.toUserName = toUserName
        self.message = message
        self.timestamp = timestamp
        self.isSent = isSent
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        fromUserID = try c.decodeIfPresent(String.self, forKey: .fromUserID) ?? ""
        fromUserLogin = try c.decodeIfPresent(String.self, forKey: .fromUserLogin) ?? ""
        fromUserName = try c.decodeIfPresent(String.self, forKey: .fromUserName) ?? ""
        toUserID = try c.decodeIfPresent(String.self, forKey: .toUserID) ?? ""
        toUserLogin = try c.decodeIfPresent(String.self, forKey: .toUserLogin) ?? ""
        toUserName = try c.decodeIfPresent(String.self, forKey: .toUserName) ?? ""
        message = try c.decodeIfPresent(String.self, forKey: .message) ?? ""
        timestamp = try c.decodeIfPresent(Int64.self, forKey: .timestamp) ?? 0
        isSent = try c.decodeIfPresent(Bool.self, forKey: .isSent) ?? false
    }
}

/// One row of the core's `whisper_list`: who, the newest line, the unread count.
public struct WhisperRow: Codable, Sendable, Equatable, Hashable, Identifiable {
    /// The conversation's key in the core's inbox (the other person's Twitch id).
    public var key: String
    public var userID: String
    public var userLogin: String
    public var userName: String
    public var profileImageURL: String?
    public var lastMessageTimestamp: Int64
    public var unreadCount: Int
    public var lastText: String
    public var lastIsSent: Bool

    public var id: String { key }

    enum CodingKeys: String, CodingKey {
        case key
        case userID = "user_id", userLogin = "user_login", userName = "user_name"
        case profileImageURL = "profile_image_url", lastMessageTimestamp = "last_message_timestamp"
        case unreadCount = "unread_count", lastText = "last_text", lastIsSent = "last_is_sent"
    }

    public init(key: String, userID: String, userLogin: String, userName: String, profileImageURL: String? = nil,
                lastMessageTimestamp: Int64, unreadCount: Int = 0, lastText: String = "", lastIsSent: Bool = false) {
        self.key = key
        self.userID = userID
        self.userLogin = userLogin
        self.userName = userName
        self.profileImageURL = profileImageURL
        self.lastMessageTimestamp = lastMessageTimestamp
        self.unreadCount = unreadCount
        self.lastText = lastText
        self.lastIsSent = lastIsSent
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        key = try c.decode(String.self, forKey: .key)
        userID = try c.decodeIfPresent(String.self, forKey: .userID) ?? key
        userLogin = try c.decodeIfPresent(String.self, forKey: .userLogin) ?? ""
        userName = try c.decodeIfPresent(String.self, forKey: .userName) ?? ""
        profileImageURL = try c.decodeIfPresent(String.self, forKey: .profileImageURL)
        lastMessageTimestamp = try c.decodeIfPresent(Int64.self, forKey: .lastMessageTimestamp) ?? 0
        unreadCount = try c.decodeIfPresent(Int.self, forKey: .unreadCount) ?? 0
        lastText = try c.decodeIfPresent(String.self, forKey: .lastText) ?? ""
        lastIsSent = try c.decodeIfPresent(Bool.self, forKey: .lastIsSent) ?? false
    }

    /// The person, never empty.
    public var shownName: String { WhisperText.name(userName, userLogin) }
}

/// The core's `whisper_list`: the signed-in account's conversations, newest first. `ownerID` is
/// empty when nobody is signed in; the writes (`whisper_send`, `whisper_mark_read`) take it back.
public struct WhisperList: Codable, Sendable, Equatable {
    public var ownerID: String
    public var conversations: [WhisperRow]
    public var unreadTotal: Int

    enum CodingKeys: String, CodingKey {
        case conversations
        case ownerID = "owner_id", unreadTotal = "unread_total"
    }

    public init(ownerID: String, conversations: [WhisperRow], unreadTotal: Int) {
        self.ownerID = ownerID
        self.conversations = conversations
        self.unreadTotal = unreadTotal
    }
}

/// One conversation with every message, oldest first (`whisper_thread`).
public struct WhisperThread: Codable, Sendable, Equatable {
    public var userID: String
    public var userLogin: String
    public var userName: String
    public var profileImageURL: String?
    public var messages: [WhisperMessage]
    public var lastMessageTimestamp: Int64
    public var unreadCount: Int

    enum CodingKeys: String, CodingKey {
        case messages
        case userID = "user_id", userLogin = "user_login", userName = "user_name"
        case profileImageURL = "profile_image_url", lastMessageTimestamp = "last_message_timestamp"
        case unreadCount = "unread_count"
    }

    public init(userID: String, userLogin: String, userName: String, profileImageURL: String? = nil,
                messages: [WhisperMessage], lastMessageTimestamp: Int64 = 0, unreadCount: Int = 0) {
        self.userID = userID
        self.userLogin = userLogin
        self.userName = userName
        self.profileImageURL = profileImageURL
        self.messages = messages
        self.lastMessageTimestamp = lastMessageTimestamp
        self.unreadCount = unreadCount
    }
}

/// One conversation's change (`whisper-conversation-updated`, `whisper-notify`, `whisper_send`'s
/// answer): move `replacedKey` to `key` if set, take `meta`, append `message` unless it is there.
public struct WhisperUpdate: Codable, Sendable, Equatable {
    public struct Meta: Codable, Sendable, Equatable {
        public var userID: String
        public var userLogin: String
        public var userName: String
        public var profileImageURL: String?
        public var lastMessageTimestamp: Int64
        public var unreadCount: Int

        enum CodingKeys: String, CodingKey {
            case userID = "user_id", userLogin = "user_login", userName = "user_name"
            case profileImageURL = "profile_image_url", lastMessageTimestamp = "last_message_timestamp"
            case unreadCount = "unread_count"
        }

        public init(userID: String, userLogin: String, userName: String, profileImageURL: String? = nil,
                    lastMessageTimestamp: Int64, unreadCount: Int) {
            self.userID = userID
            self.userLogin = userLogin
            self.userName = userName
            self.profileImageURL = profileImageURL
            self.lastMessageTimestamp = lastMessageTimestamp
            self.unreadCount = unreadCount
        }
    }

    public var ownerID: String
    public var key: String
    public var replacedKey: String?
    public var meta: Meta
    public var message: WhisperMessage?

    enum CodingKeys: String, CodingKey {
        case key, meta, message
        case ownerID = "owner_id", replacedKey = "replaced_key"
    }

    public init(ownerID: String, key: String, replacedKey: String? = nil, meta: Meta, message: WhisperMessage?) {
        self.ownerID = ownerID
        self.key = key
        self.replacedKey = replacedKey
        self.meta = meta
        self.message = message
    }

    public static func parse(_ json: String) -> WhisperUpdate? {
        try? JSONDecoder().decode(WhisperUpdate.self, from: Data(json.utf8))
    }

    /// The thread after this change, for the conversation `key`; another conversation's change
    /// leaves it as it was.
    public func applied(to thread: WhisperThread?, key open: String) -> WhisperThread? {
        guard key == open || replacedKey == open else { return thread }
        var next = thread ?? WhisperThread(userID: meta.userID, userLogin: meta.userLogin, userName: meta.userName,
                                           profileImageURL: meta.profileImageURL, messages: [])
        if let message, !next.messages.contains(where: { $0.id == message.id }) {
            next.messages.append(message)
            next.messages.sort { $0.timestamp < $1.timestamp }
        }
        next.userID = meta.userID
        next.userLogin = meta.userLogin
        next.userName = meta.userName
        next.profileImageURL = meta.profileImageURL ?? next.profileImageURL
        next.lastMessageTimestamp = meta.lastMessageTimestamp
        next.unreadCount = meta.unreadCount
        return next
    }
}

/// A message typed here that Twitch has not confirmed: on its way, or refused with the core's
/// reason. A sent one comes back from the core as a real message.
public struct PendingWhisper: Sendable, Equatable, Identifiable {
    public enum Status: Sendable, Equatable { case sending, failed(String) }

    public var id: String
    public var key: String
    public var text: String
    public var status: Status
    public var atMs: Int64

    public init(id: String, key: String, text: String, status: Status = .sending, atMs: Int64) {
        self.id = id
        self.key = key
        self.text = text
        self.status = status
        self.atMs = atMs
    }
}

/// The whisper words, the same on both phones (the Android shell's `whisperModel.ts`).
public enum WhisperText {
    public static let title = "Whispers"
    public static let entryDetail = "Private messages on Twitch"
    public static let empty = "No whispers yet."
    public static let emptyHint = "When someone whispers you on Twitch, the conversation shows up here."
    public static let failed = "Whispers did not load."
    public static let failedHint = "Pull down to try again."
    public static let threadEmpty = "No messages yet."
    public static let sending = "Sending"
    public static let retry = "Retry"
    public static let discard = "Discard"
    public static let fallbackReason = "Failed to send message. Please try again."

    public static func name(_ name: String, _ login: String) -> String {
        name.isEmpty ? (login.isEmpty ? "Someone" : login) : name
    }

    /// The list row's second line: the newest message, yours marked as yours.
    public static func preview(_ row: WhisperRow) -> String {
        guard !row.lastText.isEmpty else { return "" }
        return row.lastIsSent ? "You: \(row.lastText)" : row.lastText
    }

    public static func placeholder(_ name: String) -> String { "Whisper \(name)" }

    /// The notification title, as the desktop's: "Whisper from Bob".
    public static func notificationTitle(_ name: String) -> String { "Whisper from \(name)" }

    /// The notice when a notification reply did not go out, as the desktop's.
    public static func replyFailedTitle(_ name: String) -> String { "Your reply to \(name) was not sent" }

    /// The reason under a failed bubble: the core's sentence, or the plain fallback.
    public static func reason(_ error: Error) -> String {
        let text = (error as? CoreFailure).map(\.description) ?? String(describing: error)
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? fallbackReason : trimmed
    }

    /// The heading over a day of messages: "Today", "Yesterday", else the date.
    public static func day(_ atMs: Int64, nowMs: Int64, calendar: Calendar = .current) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(atMs) / 1000)
        let now = Date(timeIntervalSince1970: TimeInterval(nowMs) / 1000)
        let days = calendar.dateComponents([.day], from: calendar.startOfDay(for: date), to: calendar.startOfDay(for: now)).day ?? 0
        if days == 0 { return "Today" }
        if days == 1 { return "Yesterday" }
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = calendar.component(.year, from: date) == calendar.component(.year, from: now) ? "EEE, MMM d" : "MMM d, yyyy"
        return formatter.string(from: date)
    }

    /// A clock time under a run of messages: "3:05 PM".
    public static func clock(_ atMs: Int64, calendar: Calendar = .current) -> String {
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "h:mm a"
        return formatter.string(from: Date(timeIntervalSince1970: TimeInterval(atMs) / 1000))
    }
}

/// One line of the conversation view.
public enum WhisperThreadItem: Sendable, Equatable, Identifiable {
    case day(id: String, label: String)
    /// `last` ends a run (the run's time shows under it).
    case message(WhisperMessage, last: Bool, time: String?)
    case pending(PendingWhisper)

    public var id: String {
        switch self {
        case .day(let id, _): id
        case .message(let m, _, _): m.id
        case .pending(let p): p.id
        }
    }

    /// Messages close together from one side form a run.
    static let runGapMs: Int64 = 5 * 60_000

    /// The conversation view's lines, oldest first: a heading per day, each message marked as the
    /// end of its run or not, then what is still pending (under Today).
    public static func lines(_ messages: [WhisperMessage], pending: [PendingWhisper], nowMs: Int64,
                             calendar: Calendar = .current) -> [WhisperThreadItem] {
        var out: [WhisperThreadItem] = []
        var lastDay = ""
        for (i, m) in messages.enumerated() {
            let label = WhisperText.day(m.timestamp, nowMs: nowMs, calendar: calendar)
            if label != lastDay {
                out.append(.day(id: "day-\(m.timestamp)", label: label))
                lastDay = label
            }
            let next = i + 1 < messages.count ? messages[i + 1] : nil
            let last = next.map { n in
                n.isSent != m.isSent || n.timestamp - m.timestamp > runGapMs
                    || WhisperText.day(n.timestamp, nowMs: nowMs, calendar: calendar) != label
            } ?? true
            out.append(.message(m, last: last, time: last ? WhisperText.clock(m.timestamp, calendar: calendar) : nil))
        }
        if !pending.isEmpty, !messages.isEmpty, lastDay != "Today" {
            out.append(.day(id: "day-pending", label: "Today"))
        }
        out += pending.map(WhisperThreadItem.pending)
        return out
    }
}
