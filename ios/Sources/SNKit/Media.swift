import Foundation

/// A clip `clip_create` just made: the link to share and the title it was saved under.
public struct MadeClip: Decodable, Sendable, Equatable {
    public var slug: String
    public var url: String
    public var title: String
    public var channelName: String
}

/// One of "Your clips" (`my_clips_list`), newest first.
public struct MyClip: Decodable, Sendable, Equatable, Identifiable {
    public var slug: String
    public var title: String
    public var channelLogin: String
    public var channelName: String
    /// Unix milliseconds.
    public var madeAt: Int64
    public var durationSeconds: Double
    /// "live" or "vod".
    public var source: String
    public var thumbnailUrl: String?
    public var url: String
    public var ageLabel: String
    public var lengthLabel: String

    public var id: String { slug }
}

/// One row of a channel's videos or clips (`channel_media`), every label worded by the core.
public struct MediaRow: Decodable, Sendable, Equatable, Identifiable {
    /// "video" or "clip".
    public var kind: String
    public var id: String
    public var title: String
    public var thumbnailUrl: String
    public var url: String
    public var lengthLabel: String
    public var ageLabel: String
    public var viewsLabel: String
    public var byline: String?
    public var progress: Double?
    public var watched: Bool
    public var recording: Bool
    public var vodId: String?
    public var vodOffset: Int?

    public var isClip: Bool { kind == "clip" }

    /// The row's second line: age and length for a video, who clipped it and when for a clip.
    public var metaLine: String {
        if isClip {
            return [byline, ageLabel].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
        }
        return [ageLabel, lengthLabel].filter { !$0.isEmpty }.joined(separator: " · ")
    }

    /// The third line: views and the category for a video, length and views for a clip.
    public var countLine: String {
        if isClip { return [lengthLabel, viewsLabel].joined(separator: " · ") }
        return [viewsLabel, byline].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

public struct MediaPage: Decodable, Sendable, Equatable {
    public var rows: [MediaRow]
    public var cursor: String?
}

/// What the Channel sheet lists: a channel's videos (by kind, newest or most viewed) or its
/// clips (by period). The raw values are the core's `channel_media` arguments.
public enum MediaShelf: Hashable, Sendable {
    case videos(VideoKind, VideoSort)
    case clips(ClipPeriod)

    public enum VideoKind: String, CaseIterable, Sendable {
        case archive, highlight, upload
        public var title: String {
            switch self {
            case .archive: "Past broadcasts"
            case .highlight: "Highlights"
            case .upload: "Uploads"
            }
        }
    }

    public enum VideoSort: String, CaseIterable, Sendable {
        case recent, popular
        public var title: String { self == .recent ? "Recent" : "Popular" }
    }

    public enum ClipPeriod: String, CaseIterable, Sendable {
        case day = "24h", week = "7d", month = "30d", all
        public var title: String {
            switch self {
            case .day: "24 hours"
            case .week: "7 days"
            case .month: "30 days"
            case .all: "All time"
            }
        }
    }

    /// The `channel_media` arguments for this shelf.
    public func args(channelID: String, cursor: String?) -> [String: Any?] {
        switch self {
        case .videos(let kind, let sort):
            ["channelId": channelID, "kind": "videos", "filter": kind.rawValue, "sort": sort.rawValue, "cursor": cursor]
        case .clips(let period):
            ["channelId": channelID, "kind": "clips", "filter": period.rawValue, "cursor": cursor]
        }
    }
}

/// The replay rows to add for the player's position (`vod_replay_tick`); a reset drops the
/// replay rows shown before them.
public struct ReplayStep: Decodable, Sendable, Equatable {
    public var reset: Bool
    public var rows: [ChatMessage]
    public var loading: Bool
    public var error: String?
}

/// What a VOD start tells the player (`start_stream`'s `vod`): how long it runs, where to begin
/// (a saved position), the stamp its position reports carry back, and the seek bar's marks.
public struct VodInfo: Decodable, Sendable, Equatable {
    public struct Range: Decodable, Sendable, Equatable {
        public var startSecs: Double
        public var endSecs: Double
        enum CodingKeys: String, CodingKey {
            case startSecs = "start_secs", endSecs = "end_secs"
        }
    }

    public struct Chapter: Decodable, Sendable, Equatable {
        public var startSecs: Double
        public var endSecs: Double
        public var title: String
        enum CodingKeys: String, CodingKey {
            case title
            case startSecs = "start_secs", endSecs = "end_secs"
        }
    }

    public var videoID: String
    /// "recording" while the broadcast is still live, "recorded" when finished.
    public var status: String
    public var lengthSeconds: Double?
    public var recordedAt: String?
    public var channelLogin: String?
    public var title: String?
    public var startPositionSecs: Double?
    public var playID: Int
    public var mutedSegments: [Range]
    public var chapters: [Chapter]

    enum CodingKeys: String, CodingKey {
        case status, title, chapters
        case videoID = "video_id", lengthSeconds = "length_seconds", recordedAt = "recorded_at"
        case channelLogin = "channel_login", startPositionSecs = "start_position_secs", playID = "play_id"
        case mutedSegments = "muted_segments"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        videoID = try c.decode(String.self, forKey: .videoID)
        status = try c.decodeIfPresent(String.self, forKey: .status) ?? "unknown"
        lengthSeconds = try c.decodeIfPresent(Double.self, forKey: .lengthSeconds)
        recordedAt = try c.decodeIfPresent(String.self, forKey: .recordedAt)
        channelLogin = try c.decodeIfPresent(String.self, forKey: .channelLogin)
        title = try c.decodeIfPresent(String.self, forKey: .title)
        startPositionSecs = try c.decodeIfPresent(Double.self, forKey: .startPositionSecs)
        playID = try c.decodeIfPresent(Int.self, forKey: .playID) ?? 0
        mutedSegments = try c.decodeIfPresent([Range].self, forKey: .mutedSegments) ?? []
        chapters = try c.decodeIfPresent([Chapter].self, forKey: .chapters) ?? []
    }

    public var isRecording: Bool { status == "recording" }
}
