import Foundation

/// The sleep timer as the core runs it (`sleep_timer_state`, pushed as `sleep-timer://state`):
/// whether one runs, when it stops playback, and the menu's words.
public struct SleepTimerState: Decodable, Sendable, Equatable {
    public struct Preset: Decodable, Sendable, Equatable, Hashable {
        public var minutes: Int
        public var label: String

        public init(minutes: Int, label: String) {
            self.minutes = minutes
            self.label = label
        }
    }

    /// "off", "running", "fading" or "fired".
    public var phase: String
    public var endsAtMs: Int64?
    public var remainingSecs: Int?
    /// "Stops in 42 min" while one runs.
    public var rowLabel: String?
    public var fadeSecs: Int
    public var presets: [Preset]

    public var running: Bool { phase == "running" || phase == "fading" }

    public static func parse(_ json: String) -> SleepTimerState? {
        try? JSONDecoder().decode(SleepTimerState.self, from: Data(json.utf8))
    }

    /// Minutes for a custom pick of hours and minutes; nil when it comes to nothing.
    public static func customMinutes(hours: Int, minutes: Int) -> Int? {
        let total = max(0, hours) * 60 + max(0, minutes)
        return total > 0 ? min(total, 24 * 60) : nil
    }
}
