import Foundation

/// The rate the shared image clock asks for: the slower of 30 and 60 Hz that still shows every
/// frame of the fastest animation on screen. Frames are never shorter than 20 ms (`FrameTimeline`),
/// so 60 always suffices. A 30 fps file stores its frames as whole milliseconds (33 ms), a hair
/// under a 30 Hz tick, so a 30 Hz clock would skip one every few seconds: it gets 60.
public enum ClockRate {
    public static func hz(shortest: Double) -> Int { shortest > 1.0 / 30 + 0.001 ? 30 : 60 }
}

/// When the chat list's display link runs and how fast: only while it has work, following new
/// rows at 60 Hz, at the display's full rate only under a finger (or the Resume live glide).
public enum ChatLinkPolicy {
    public enum Rate: Equatable, Sendable { case paused, follow, touch }
    /// Idle frames kept after the last work, so a burst does not pause and wake the link between batches.
    public static let linger = 6

    public static func rate(idleTicks: Int, gliding: Bool, touching: Bool) -> Rate {
        if touching || gliding { return .touch }
        return idleTicks >= linger ? .paused : .follow
    }

    /// Frames per second; Low Power Mode caps everything at 60.
    public static func range(_ rate: Rate, lowPower: Bool) -> (minimum: Float, maximum: Float, preferred: Float) {
        if rate == .touch, !lowPower { return (60, 120, 120) }
        return (30, 60, 60)
    }
}

/// A live stream paused this long has had its relay parked by the core (Rust's window); playing
/// again jumps back to the live edge instead of resuming where it stopped.
public enum PausePark {
    public static let after: Duration = .seconds(30)

    public static func jumpsToLive(pausedFor: Duration?, live: Bool) -> Bool {
        guard live, let pausedFor else { return false }
        return pausedFor >= after
    }
}

/// The watch loop's pace: a quarter second until the first frame, then a second; the live-edge
/// distance is asked every second on screen or in Picture in Picture, every 5 s heard in the
/// background, never while paused.
public enum MeasureCadence {
    public static func tick(loading: Bool) -> Duration { loading ? .milliseconds(250) : .seconds(1) }

    public static func sampleEvery(paused: Bool, background: Bool, pip: Bool) -> Duration? {
        if paused { return nil }
        return background && !pip ? .seconds(5) : .seconds(1)
    }
}

/// Which latency samples earn a log line: the distance moving a quarter second or more, a new
/// player state or rate, or a heartbeat every 30 s.
public struct LatencyLogGate: Sendable {
    public static let step = 250
    public static let heartbeat = 30.0
    private var last: (behind: Int, state: Int, rate: Float, at: Double)?

    public init() {}

    public mutating func shouldLog(behind: Int, state: Int, rate: Float, at now: Double) -> Bool {
        if let last, abs(last.behind - behind) < Self.step, last.state == state, last.rate == rate,
           now - last.at < Self.heartbeat { return false }
        last = (behind, state, rate, now)
        return true
    }
}
