import Foundation

/// A Twitch channel's offline room (the core's `OfflineRoom`): who, when they were last live and
/// with what, the latest broadcast, the next scheduled stream, and how many wait in chat.
public struct OfflineRoom: Decodable, Sendable, Equatable {
    public struct Vod: Decodable, Sendable, Equatable {
        public var id: String
        public var title: String
        public var createdAt: String
        public var lengthSeconds: Int
        public var category: String?

        enum CodingKeys: String, CodingKey {
            case id, title, category
            case createdAt = "created_at"
            case lengthSeconds = "length_seconds"
        }
    }

    public struct Segment: Decodable, Sendable, Equatable {
        public var startAt: String
        public var title: String?
        public var category: String?

        enum CodingKeys: String, CodingKey {
            case title, category
            case startAt = "start_at"
        }
    }

    public var userID: String
    public var login: String
    public var displayName: String
    public var avatarURL: String?
    public var lastLiveAt: String?
    public var lastTitle: String?
    public var lastCategory: String?
    public var latestVod: Vod?
    public var nextStream: Segment?
    public var sadEmoteURL: String?
    public var happyEmoteURL: String?
    public var shiny: Bool
    public var chatters: Int?

    enum CodingKeys: String, CodingKey {
        case login, shiny, chatters
        case userID = "user_id"
        case displayName = "display_name"
        case avatarURL = "avatar_url"
        case lastLiveAt = "last_live_at"
        case lastTitle = "last_title"
        case lastCategory = "last_category"
        case latestVod = "latest_vod"
        case nextStream = "next_stream"
        case sadEmoteURL = "sad_emote_url"
        case happyEmoteURL = "happy_emote_url"
    }

    public init(userID: String = "", login: String, displayName: String, avatarURL: String? = nil,
                lastLiveAt: String? = nil, lastTitle: String? = nil, lastCategory: String? = nil,
                latestVod: Vod? = nil, nextStream: Segment? = nil, sadEmoteURL: String? = nil,
                happyEmoteURL: String? = nil, shiny: Bool = false, chatters: Int? = nil) {
        self.userID = userID
        self.login = login
        self.displayName = displayName
        self.avatarURL = avatarURL
        self.lastLiveAt = lastLiveAt
        self.lastTitle = lastTitle
        self.lastCategory = lastCategory
        self.latestVod = latestVod
        self.nextStream = nextStream
        self.sadEmoteURL = sadEmoteURL
        self.happyEmoteURL = happyEmoteURL
        self.shiny = shiny
        self.chatters = chatters
    }
}

/// `watch_open_room`'s answer: the room, or the live row when the card that opened it was stale.
public struct RoomOpen: Decodable, Sendable {
    public var superseded: Bool
    public var room: OfflineRoom?
    public var live: TwitchStream?
    public var row: TwitchStream?
}

/// The room's words.
public enum RoomText {
    /// "Last live 3 hours ago", or nil when the end is unknown.
    public static func lastLive(_ iso: String?, now: Date = Date()) -> String? {
        guard let iso, let date = parse(iso) else { return nil }
        return "Last live " + ago(max(0, now.timeIntervalSince(date)))
    }

    /// "just now", "5 minutes ago", "3 hours ago", "2 days ago", "4 weeks ago", "1 year ago".
    public static func ago(_ seconds: TimeInterval) -> String {
        let minutes = Int(seconds / 60)
        let steps: [(Int, String)] = [(60 * 24 * 365, "year"), (60 * 24 * 7, "week"), (60 * 24, "day"), (60, "hour"), (1, "minute")]
        for (size, unit) in steps where minutes >= size {
            let n = minutes / size
            return "\(n) \(unit)\(n == 1 ? "" : "s") ago"
        }
        return "just now"
    }

    /// "Next stream Friday 7:00 PM" ("today" for today), or nil when nothing is scheduled.
    public static func next(_ segment: OfflineRoom.Segment?, now: Date = Date()) -> String? {
        guard let segment, let date = parse(segment.startAt) else { return nil }
        let weekday = DateFormatter()
        weekday.dateFormat = "EEEE"
        let time = DateFormatter()
        time.timeStyle = .short
        time.dateStyle = .none
        let day = Calendar.current.isDate(date, inSameDayAs: now) ? "today" : weekday.string(from: date)
        return "Next stream \(day) \(time.string(from: date))"
    }

    /// "12 waiting in chat"; nil when the count is unknown or nobody waits.
    public static func waiting(_ chatters: Int?) -> String? {
        guard let chatters, chatters > 0 else { return nil }
        return "\(chatters.formatted()) waiting in chat"
    }

    static func parse(_ text: String) -> Date? {
        let full = ISO8601DateFormatter()
        full.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = full.date(from: text) { return date }
        return ISO8601DateFormatter().date(from: text)
    }
}

/// `watch-session://went-live`: the room's channel is live and playable.
public struct WentLive: Decodable, Sendable {
    public var stream: TwitchStream
}

/// `watch-session://waiting`: how many wait in the open room's chat.
public struct RoomWaiting: Decodable, Sendable {
    public var login: String
    public var count: Int
}
