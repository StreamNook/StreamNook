import Foundation
#if canImport(CoreGraphics)
// On Apple platforms CGRect's geometry lives here; Linux has it in Foundation.
import CoreGraphics
#endif

/// What one tap puts away. A tap outside the field being typed in puts the keyboard away, and a
/// tap outside an open pop-out closes it; that tap does nothing else, so what sits under it is
/// not pressed by the same touch (a chat row, the video). Where the tap is plainly meant for what
/// it lands on (a search result, a form's buttons and switches) the keyboard still goes and the
/// press lands too. The screens only measure; the rule lives here.
public enum OutsideTapRule {
    /// An open pop-out: its own frame and its toggles' frames (a tap on the button that opened
    /// it is that button's to handle).
    public struct Popout: Sendable {
        public var id: String
        public var regions: [CGRect]

        public init(id: String, regions: [CGRect]) {
            self.id = id
            self.regions = regions
        }

        func contains(_ point: CGPoint) -> Bool {
            regions.contains { $0.contains(point) }
        }
    }

    public struct Decision: Equatable, Sendable {
        /// Put the keyboard away.
        public var keyboard: Bool
        /// The pop-outs to close, by id.
        public var popouts: [String]
        /// The tap still presses what it lands on.
        public var passes: Bool

        public init(keyboard: Bool = false, popouts: [String] = [], passes: Bool = false) {
            self.keyboard = keyboard
            self.popouts = popouts
            self.passes = passes
        }

        public var closesAnything: Bool { keyboard || !popouts.isEmpty }

        /// The tap closes something and stops there: nothing under it is pressed.
        public var usedUp: Bool { closesAnything && !passes }
    }

    /// - Parameters:
    ///   - field: the field being typed in, nil while there is no keyboard.
    ///   - keepers: where a tap never puts the keyboard away and always acts: a field's own
    ///     controls (the composer's emote, send and command buttons). A pop-out it is outside of
    ///     still closes (the unrolled pins, when the field is tapped to type).
    ///   - popouts: the open pop-outs.
    ///   - through: where a tap that puts the keyboard away still presses what it lands on.
    public static func decide(at point: CGPoint, field: CGRect?, keepers: [CGRect], popouts: [Popout],
                              through: [CGRect] = []) -> Decision {
        let outside = popouts.filter { !$0.contains(point) }.map(\.id)
        // Inside an open pop-out is the field's company too: picking an emote while typing in
        // the picker's search keeps the keyboard.
        let insidePopout = popouts.contains { $0.contains(point) }
        let keyboard = field.map { field in
            !(field.contains(point) || keepers.contains { $0.contains(point) } || insidePopout)
        } ?? false
        let inKeeper = keepers.contains { $0.contains(point) }
        return Decision(keyboard: keyboard, popouts: outside,
                        passes: inKeeper || (outside.isEmpty && through.contains { $0.contains(point) }))
    }

    /// How far a finger may move and still be a tap (points), and how long it may stay down: a
    /// drag scrolls and a hold opens a hold menu, and neither closes anything. Hold menus never
    /// wait on a tap, so a deliberate slow tap still counts.
    public static let slop: CGFloat = 10
    public static let longestTap: TimeInterval = 0.5

    /// Whether a touch that moved `distance` points and lasted `duration` seconds is a tap.
    public static func isTap(distance: CGFloat, duration: TimeInterval) -> Bool {
        distance <= slop && duration <= longestTap
    }
}
