import SNKit
import SwiftUI

/// The motion StreamNook's surfaces use. Movement is dropped under Reduced or Off motion (the
/// setting) and under the phone's own Reduce Motion; fades stay unless motion is Off.
@MainActor
enum SNMotion {
    /// A drill-in sliding in or out: 0.22 s on (0.2, 0.8, 0.2, 1).
    static let drill = Animation.timingCurve(0.2, 0.8, 0.2, 1, duration: 0.22)
    /// The tab switch: 0.16 s on the same curve.
    static let tab = Animation.timingCurve(0.2, 0.8, 0.2, 1, duration: 0.16)
    /// The chosen pill gliding between segments (stiffness 350, damping 30).
    static let glide = Animation.spring(response: 0.34, dampingFraction: 0.8)
    /// The watch layer rising in (stiffness 420, damping 36, mass 0.7).
    static let watchIn = Animation.interpolatingSpring(mass: 0.7, stiffness: 420, damping: 36)
    /// The watch layer dropping away: 0.18 s ease-in.
    static let watchOut = Animation.easeIn(duration: 0.18)
    /// The mini box snapping to a side, or the layer springing back from a drag.
    static let settleBack = Animation.interpolatingSpring(mass: 0.7, stiffness: 420, damping: 36)
    /// A list's items settling in: 0.34 s on (0.16, 1, 0.3, 1).
    static let settle = Animation.timingCurve(0.16, 1, 0.3, 1, duration: 0.34)
    /// The stagger between settling items, capped.
    static func settleDelay(_ index: Int) -> Double { min(Double(index) * 0.045, 0.4) }
    /// Page content of the setup wizard: 0.4 s on (0.16, 1, 0.3, 1).
    static let page = Animation.timingCurve(0.16, 1, 0.3, 1, duration: 0.4)
    /// A toast coming in.
    static let toastIn = Animation.spring(response: 0.3, dampingFraction: 0.85)

    /// The animation to run, or none when motion is Off.
    static func run(_ animation: Animation) -> Animation? {
        Look.shared.motion == .off ? nil : animation
    }
}

extension EnvironmentValues {
    /// Whether movement is dropped here: the Motion setting below Full, or the phone's Reduce Motion.
    @Entry var snStill = false
}

extension View {
    /// Gives everything inside the motion state from the setting and the phone.
    func snMotionScope() -> some View { modifier(MotionScope()) }

    /// One item of a list settling in the first time the list shows real items: it fades and rises
    /// 14 points while growing from 97 percent, staggered by its index. `active` is false once the
    /// list has settled, so later items (a refresh, the next page) just appear.
    func settleIn(index: Int, active: Bool) -> some View {
        modifier(SettleIn(index: index, active: active))
    }
}

private struct MotionScope: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var system

    func body(content: Content) -> some View {
        content.environment(\.snStill, system || Look.shared.motion.dropsMovement)
    }
}

private struct SettleIn: ViewModifier {
    let index: Int
    let active: Bool
    @Environment(\.snStill) private var still
    @State private var shown = false

    func body(content: Content) -> some View {
        let hidden = active && !shown
        content
            .opacity(hidden ? 0 : 1)
            .offset(y: hidden && !still ? 14 : 0)
            .scaleEffect(hidden && !still ? 0.97 : 1)
            .onAppear {
                guard active, !shown else { return }
                withAnimation(SNMotion.run(SNMotion.settle.delay(SNMotion.settleDelay(index)))) { shown = true }
            }
    }
}

/// Plays a list's settle once per list identity: `key` names the list (the view mode, the search,
/// the filter), and a new key plays it again. Skeletons and empty states never use it up.
@MainActor @Observable
final class SettleLatch {
    private var settled: Set<String> = []

    /// Whether items of the list `key` settle in as they appear.
    func playing(_ key: String) -> Bool { !settled.contains(key) }

    /// The list `key` has shown its items; from here on new items just appear.
    func finish(_ key: String) async {
        guard !settled.contains(key) else { return }
        try? await Task.sleep(for: .seconds(0.8))
        settled.insert(key)
    }
}

extension View {
    /// Marks the list `key` settled once it has real items on screen.
    func settles(_ latch: SettleLatch, key: String, ready: Bool) -> some View {
        task(id: "\(key)|\(ready)") { if ready { await latch.finish(key) } }
    }
}
