import Foundation
import Observation

/// The phone's Low Power Mode, followed live. On: emotes, badges and paints hold still
/// (AnimationClock), decorative movement stops (`snStill`, `MotionGate`), the chat list and the
/// image clock stay at or under 60 Hz, and the core hears it (`set_low_power`).
@MainActor @Observable
final class PowerState {
    static let shared = PowerState()
    /// Posted on the main thread when `lowPower` changes.
    static let changed = Notification.Name("app.streamnook.power.changed")

    private(set) var lowPower = ProcessInfo.processInfo.isLowPowerModeEnabled
    /// Called with the new value on every change (the core report).
    @ObservationIgnored var onChange: ((Bool) -> Void)?

    private init() {
        NotificationCenter.default.addObserver(forName: .NSProcessInfoPowerStateDidChange, object: nil,
                                               queue: .main) { _ in
            MainActor.assumeIsolated { PowerState.shared.refresh() }
        }
    }

    /// Reads Low Power Mode again; a change is posted and handed to `onChange`.
    func refresh() {
        let now = ProcessInfo.processInfo.isLowPowerModeEnabled
        guard now != lowPower else { return }
        lowPower = now
        NotificationCenter.default.post(name: Self.changed, object: nil)
        onChange?(now)
    }
}
