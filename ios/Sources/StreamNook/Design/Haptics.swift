import UIKit

/// Haptics mark changes of state, never motion: no buzz on tab switches, sheets, pull to refresh
/// or navigation.
@MainActor
enum Haptics {
    /// The heaviest single tick: a long press arms (the chat fan-out opens).
    static func arm() { UIImpactFeedbackGenerator(style: .medium).impactOccurred() }
    /// A light tick: a selection moves to another bucket, a handle is tapped or snaps.
    static func tick() { UISelectionFeedbackGenerator().selectionChanged() }
    /// The lightest step: a dial crosses a tier or a tenth, a handle is grabbed.
    static func step() { UIImpactFeedbackGenerator(style: .light).impactOccurred(intensity: 0.6) }
    /// A committed action, or a message that mentions you.
    static func commit() { UIImpactFeedbackGenerator(style: .rigid).impactOccurred() }
    /// A destructive action committed (ban, timeout, delete), or a sign-out armed.
    static func destructive() { UINotificationFeedbackGenerator().notificationOccurred(.warning) }
}
