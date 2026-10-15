import Foundation

/// The core's link governor moved the stream (`playback-adapt`): the rendition now playing, the
/// extra distance from the live edge it keeps after stalls on a poor link, and a URL when the relay
/// restarted on another rendition (the player reloads on it).
public struct PlaybackAdapt: Decodable, Sendable, Equatable {
    public var quality: String
    public var auto: Bool
    public var extraCushion: Double
    public var url: String?
    public var reason: String

    public init(quality: String, auto: Bool, extraCushion: Double, url: String? = nil, reason: String) {
        self.quality = quality
        self.auto = auto
        self.extraCushion = extraCushion
        self.url = url
        self.reason = reason
    }

    public static func parse(_ json: String) -> PlaybackAdapt? {
        try? JSONDecoder().decode(PlaybackAdapt.self, from: Data(json.utf8))
    }
}

/// One report for the core's `report_playback_health`.
public enum LinkReport: Equatable, Sendable {
    /// Whole segments downloaded from the network: `bytes` in `ms`.
    case transfer(bytes: Int64, ms: Double)
    /// Playback has been frozen `ms` so far.
    case stall(ms: Double)
    /// A stall ended after `ms`.
    case stallEnd(ms: Double)

    /// The command's arguments.
    public var args: [String: Any?] {
        switch self {
        case let .transfer(bytes, ms): ["event": "frag", "bytes": bytes, "ms": ms]
        case let .stall(ms): ["event": "stall", "bytes": nil, "ms": ms]
        case let .stallEnd(ms): ["event": "stall_end", "bytes": nil, "ms": ms]
        }
    }
}

/// What the player tells the core about the link, from what AVPlayer shows: stalls (waiting to
/// play after the first frame), once a second while they last and once when they end, and, on the
/// whole-segment route only, the bytes AVPlayer's access log says it downloaded and how long that
/// took. On the low-latency route AVPlayer reads parts from the relay over loopback, which says
/// nothing about the link; the core measures it there itself. Times are seconds on any clock.
public struct LinkReporter: Sendable {
    /// Below this a transfer is mostly time to first byte and says little about the rate.
    public static let minTransferBytes: Int64 = 64 * 1024

    private var stallSince: Double?
    private var lastStallReport: Double?
    /// The access log event being read and what of it was already reported.
    private var event = -1
    private var reportedBytes: Int64 = 0
    private var reportedDuration: Double = 0

    public init() {}

    /// Call on every look at the player. `waiting` is true while it waits to play after the first
    /// frame (a user's pause is not waiting).
    public mutating func waiting(_ waiting: Bool, at now: Double) -> LinkReport? {
        if waiting {
            guard let since = stallSince else {
                stallSince = now
                lastStallReport = now
                return nil
            }
            if let last = lastStallReport, now - last >= 1 {
                lastStallReport = now
                return .stall(ms: (now - since) * 1000)
            }
            return nil
        }
        guard let since = stallSince else { return nil }
        stallSince = nil
        lastStallReport = nil
        return .stallEnd(ms: (now - since) * 1000)
    }

    /// A pause, a reload or a route change: a running stall was not the link's doing.
    public mutating func reset() {
        stallSince = nil
        lastStallReport = nil
        event = -1
        reportedBytes = 0
        reportedDuration = 0
    }

    /// The access log's newest event: its index and its running byte and transfer-time totals.
    /// Returns the downloads since the last report once they are big enough to judge the link by.
    public mutating func accessLog(event index: Int, bytes: Int64, transferDuration: Double) -> LinkReport? {
        if index != event {
            event = index
            reportedBytes = 0
            reportedDuration = 0
        }
        let newBytes = bytes - reportedBytes
        let newDuration = transferDuration - reportedDuration
        guard newBytes >= Self.minTransferBytes, newDuration > 0 else { return nil }
        reportedBytes = bytes
        reportedDuration = transferDuration
        return .transfer(bytes: newBytes, ms: newDuration * 1000)
    }
}
