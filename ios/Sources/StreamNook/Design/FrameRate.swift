import QuartzCore

/// The frame rates Core Animation decoration asks for: looping decoration never asks for 120 Hz
/// (touch scrolling still gets it).
enum FrameRate {
    /// Washes, glints, flashes and other movement people follow: up to 60.
    static var decor: CAFrameRateRange { CAFrameRateRange(minimum: 30, maximum: 60, preferred: 60) }
    /// Slow loops (a curtain drifting, a ping): 30 is enough.
    static var slowDecor: CAFrameRateRange { CAFrameRateRange(minimum: 20, maximum: 60, preferred: 30) }
}
