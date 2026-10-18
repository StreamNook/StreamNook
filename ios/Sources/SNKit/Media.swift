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
    /// Seek-preview frames (finished VODs only).
    public var storyboard: Storyboard?

    enum CodingKeys: String, CodingKey {
        case status, title, chapters, storyboard
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
        storyboard = (try? c.decodeIfPresent(Storyboard.self, forKey: .storyboard)) ?? nil
    }

    public var isRecording: Bool { status == "recording" }
}

/// A VOD's seek-preview sprite sheets: each image is a grid of frames, one every `intervalSecs`.
public struct Storyboard: Decodable, Sendable, Equatable {
    public struct Variant: Decodable, Sendable, Equatable {
        public var width: Double
        public var height: Double
        public var rows: Int
        public var cols: Int
        public var count: Int
        public var intervalSecs: Double
        public var images: [String]

        enum CodingKeys: String, CodingKey {
            case width, height, rows, cols, count, images
            case intervalSecs = "interval_secs"
        }
    }

    /// One frame: the sheet to load and where the frame sits in it, in the sheet's pixels.
    public struct Frame: Equatable, Sendable {
        public var url: URL
        public var x: Double
        public var y: Double
        public var width: Double
        public var height: Double
        public var sheetWidth: Double
        public var sheetHeight: Double
    }

    public var baseURL: String
    /// Smallest frames first.
    public var variants: [Variant]

    enum CodingKeys: String, CodingKey {
        case variants
        case baseURL = "base_url"
    }

    /// The frame for `seconds`, from the largest variant no wider than `maxWidth` pixels.
    public func frame(at seconds: Double, maxWidth: Double = 320) -> Frame? {
        guard let variant = variants.last(where: { $0.width <= maxWidth }) ?? variants.first,
              variant.intervalSecs > 0, variant.rows > 0, variant.cols > 0, variant.count > 0 else { return nil }
        let index = min(variant.count - 1, max(0, Int(seconds / variant.intervalSecs)))
        let perSheet = variant.rows * variant.cols
        let sheet = index / perSheet
        guard variant.images.indices.contains(sheet),
              let url = URL(string: baseURL + variant.images[sheet]) else { return nil }
        let cell = index % perSheet
        return Frame(url: url, x: Double(cell % variant.cols) * variant.width, y: Double(cell / variant.cols) * variant.height,
                     width: variant.width, height: variant.height,
                     sheetWidth: Double(variant.cols) * variant.width, sheetHeight: Double(variant.rows) * variant.height)
    }
}
