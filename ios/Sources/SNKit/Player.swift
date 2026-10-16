import Foundation

/// A `channel-state` push for the watched channel's numbers (the core's `ChannelUpdate::Viewers`):
/// viewers, the broadcast's start, title and category, all from one Helix poll. Other sections
/// (points, pinned, collab) decode as nil and are skipped.
public struct ChannelViewers: Decodable, Sendable, Equatable {
    public var login: String
    public var viewerCount: Int?
    public var startedAt: String?
    public var title: String?
    public var gameName: String?

    enum CodingKeys: String, CodingKey {
        case login
        case viewerCount = "viewer_count"
        case startedAt = "started_at"
        case title
        case gameName = "game_name"
    }

    public init(login: String, viewerCount: Int? = nil, startedAt: String? = nil, title: String? = nil,
                gameName: String? = nil) {
        self.login = login
        self.viewerCount = viewerCount
        self.startedAt = startedAt
        self.title = title
        self.gameName = gameName
    }

    /// The push when it is the viewers section, nil for any other section.
    public static func parse(_ json: String) -> ChannelViewers? {
        guard let state = try? JSONDecoder().decode(Section.self, from: Data(json.utf8)), state.section == "viewers"
        else { return nil }
        return try? JSONDecoder().decode(ChannelViewers.self, from: Data(json.utf8))
    }

    private struct Section: Decodable { var section: String }
}

/// `watch-session://redirect`: a raid the core follows, with the target's seed row.
public struct RaidRedirect: Decodable, Sendable {
    public var reason: String
    public var target: TwitchStream
}

/// `watch_session_resolve_offline`'s answer: what to do about a stream that looked offline.
/// `action` is `nothing`, `still_live`, `offline_chat`, `notify`, `stop` or `switch_to`.
public struct OfflineDecision: Decodable, Sendable {
    public struct Notice: Decodable, Sendable, Equatable {
        public var message: String
        public var error: Bool
    }

    public var action: String
    public var why: String?
    public var notice: Notice?
    public var stream: TwitchStream?
}

extension ChannelViewers {
    /// `watch-session://stream-update` for the watched stream: a fresh title and category (Twitch
    /// channel.update), never blanking what it did not carry.
    public static func streamUpdate(_ json: String) -> ChannelViewers? {
        guard let object = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else { return nil }
        let title = object["title"] as? String
        let game = object["game_name"] as? String
        let viewers = object["viewer_count"] as? Int
        guard title != nil || game != nil || viewers != nil else { return nil }
        return ChannelViewers(login: "", viewerCount: viewers, title: title, gameName: game)
    }
}

/// `watch_start`'s answer: the relay to play and the row Rust completed (Following's row, or the
/// seed with its gaps filled). A superseded start (a newer start or a stop won) carries nothing.
public struct WatchStart: Decodable, Sendable, Equatable {
    public var superseded: Bool
    public var row: TwitchStream?
    public var stream: StreamStart?

    public init(superseded: Bool, row: TwitchStream? = nil, stream: StreamStart? = nil) {
        self.superseded = superseded
        self.row = row
        self.stream = stream
    }
}

extension TwitchStream {
    /// The row as `watch_start`'s seed: what the card already knows, in the core's field names.
    public var seed: [String: Any] {
        guard let data = try? JSONEncoder().encode(self),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return object
    }
}

/// The player's saved preferences, read from `load_settings`: the quality picked last (the
/// intent, not what was served) and what leaving the app does to a playing stream.
public struct PlayerPrefs: Decodable, Sendable, Equatable {
    public var quality: String?
    public var backgroundMode: String?

    enum CodingKeys: String, CodingKey {
        case quality
        case videoPlayer = "video_player"
    }

    enum VideoPlayerKeys: String, CodingKey {
        case backgroundMode = "background_mode"
    }

    public init(quality: String? = nil, backgroundMode: String? = nil) {
        self.quality = quality
        self.backgroundMode = backgroundMode
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        quality = try c.decodeIfPresent(String.self, forKey: .quality)
        let player = try? c.nestedContainer(keyedBy: VideoPlayerKeys.self, forKey: .videoPlayer)
        backgroundMode = try player?.decodeIfPresent(String.self, forKey: .backgroundMode)
    }

    /// Leaving the app floats the video in Picture in Picture unless the viewer chose audio only.
    public var floatsVideo: Bool { backgroundMode != "audio" }
}

/// The words the player's controls show.
public enum PlayerText {
    /// "2h 14m" or "14m" since the broadcast began; empty when the start is unknown or unreadable.
    public static func uptime(startedAt: String?, now: Date = Date()) -> String {
        guard let startedAt, let start = parseDate(startedAt) else { return "" }
        let seconds = max(0, Int(now.timeIntervalSince(start)))
        let hours = seconds / 3600
        let minutes = (seconds % 3600) / 60
        return hours > 0 ? "\(hours)h \(minutes)m" : "\(minutes)m"
    }

    /// The quality sheet's row name: `best` is "Auto", `audio_only` "Audio only", `worst`
    /// "Lowest"; a named tier keeps its spelling ("1080p60 (source)").
    public static func qualityLabel(_ name: String) -> String {
        switch name {
        case "best": return "Auto"
        case "audio_only": return "Audio only"
        case "worst": return "Lowest"
        default: return name.prefix(1).uppercased() + name.dropFirst()
        }
    }

    /// The sheet's rows: Auto first (the way back to the screen cap), then the core's tiers in
    /// its order.
    public static func qualityRows(_ available: [String]) -> [String] {
        available.filter { $0 == "best" } + available.filter { $0 != "best" }
    }

    /// The ticked quality row: this session's pick, else the saved one, else `best`.
    public static func tickedQuality(picked: String?, saved: String?) -> String {
        if let picked, !picked.isEmpty { return picked }
        if let saved, !saved.isEmpty { return saved }
        return "best"
    }

    private static func parseDate(_ text: String) -> Date? {
        let full = ISO8601DateFormatter()
        full.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = full.date(from: text) { return date }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: text)
    }
}

/// The tallest rendition worth asking for on this display: its short edge in real pixels snapped
/// DOWN to a rung Twitch publishes, so a panel a few pixels short of a rung still gets that rung's
/// lower neighbour rather than wobbling between them. 0 when the edge is unknown (no cap).
public enum ScreenCap {
    public static let rungs = [360, 480, 720, 1080, 1440, 2160]

    public static func height(shortEdgePixels px: Int) -> Int {
        guard px > 0 else { return 0 }
        return rungs.last { px >= $0 } ?? rungs[0]
    }
}
