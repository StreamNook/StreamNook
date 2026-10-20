import SNKit
import SwiftUI
import UIKit

/// The one rule for putting things away (`OutsideTapRule`): a tap outside the field being typed
/// in puts the keyboard away, and a tap outside an open pop-out closes it, and that tap presses
/// nothing else. One recognizer on the window watches every touch; views only say what they are:
///
/// - `.closesOnOutsideTap($isPresented)` on a pop-out the app draws itself (the emote picker,
///   the suggestion strip). Sheets, menus and context menus close on an outside tap already.
/// - `.outsideTapAnchor(id)` on the button that toggles that pop-out, so pressing it is the
///   button's own business.
/// - `.keepsKeyboard()` on a field's own controls (the composer's emote, send and command
///   buttons), where a press acts and the keyboard stays.
/// - `.tapsThroughKeyboard()` where a tap is plainly meant for what it lands on (search results,
///   a form's buttons and switches): the keyboard goes and the press lands too.
///
/// While something would close, other taps wait for this one to fail; scrolling never waits.
@MainActor
final class OutsideTap {
    static let shared = OutsideTap()

    fileprivate final class Weak {
        weak var view: OutsideTapMarker?
        init(_ view: OutsideTapMarker) { self.view = view }
    }

    private var markers: [Weak] = []
    private var installed = Set<ObjectIdentifier>()

    fileprivate func add(_ marker: OutsideTapMarker) {
        markers.removeAll { $0.view == nil || $0.view === marker }
        markers.append(Weak(marker))
    }

    fileprivate func remove(_ marker: OutsideTapMarker) {
        markers.removeAll { $0.view == nil || $0.view === marker }
    }

    /// Puts the recognizer on `window` once.
    fileprivate func install(on window: UIWindow) {
        guard installed.insert(ObjectIdentifier(window)).inserted else { return }
        window.addGestureRecognizer(OutsideTapRecognizer(target: nil, action: nil))
    }

    private var live: [OutsideTapMarker] { markers.compactMap(\.view) }

    /// The text view being typed in, when one is.
    static func editingView() -> UIView? {
        FirstResponder.current as? UIView
    }

    /// The view of the screen on top: the root, or the sheet presented last. What lies under
    /// a sheet is covered, so it neither closes nor keeps anything.
    static func topView(of window: UIWindow) -> UIView? {
        var top = window.rootViewController
        while let next = top?.presentedViewController, !next.isBeingDismissed { top = next }
        return top?.view
    }

    /// Alerts own their taps: their buttons and their own field.
    static func alertOnTop(of window: UIWindow) -> Bool {
        var top = window.rootViewController
        while let next = top?.presentedViewController, !next.isBeingDismissed { top = next }
        return top is UIAlertController
    }

    /// Something would close: the keyboard is up or a pop-out is open.
    func armed(in window: UIWindow) -> Bool {
        guard !Self.alertOnTop(of: window), let top = Self.topView(of: window) else { return false }
        if let field = Self.editingView(), field.isDescendant(of: top) { return true }
        return live.contains { $0.window === window && $0.role.isOpenPopout && $0.isDescendant(of: top) }
    }

    /// What a tap at `point` (window coordinates) would put away.
    func decide(at point: CGPoint, in window: UIWindow) -> OutsideTapRule.Decision {
        guard !Self.alertOnTop(of: window), let top = Self.topView(of: window) else { return .init() }
        let here = live.filter { $0.window === window && !$0.isHidden && $0.alpha > 0 && $0.isDescendant(of: top) }
        func frame(_ view: UIView) -> CGRect { view.convert(view.bounds, to: window) }
        var popouts: [String: [CGRect]] = [:]
        var order: [String] = []
        var keepers: [CGRect] = []
        var through: [CGRect] = []
        for marker in here {
            switch marker.role {
            case .popout(let id, true):
                if popouts[id] == nil { order.append(id) }
                popouts[id, default: []].append(frame(marker))
            case .keeper:
                keepers.append(frame(marker))
            case .through:
                through.append(frame(marker))
            case .anchor, .popout(_, false), .window:
                // An anchor counts once its pop-out is known open (below).
                break
            }
        }
        for marker in here {
            if case .anchor(let id) = marker.role, popouts[id] != nil { popouts[id]?.append(frame(marker)) }
        }
        let field = Self.editingView().flatMap { view -> CGRect? in
            guard view.isDescendant(of: top) else { return nil }
            // A search bar's Cancel and clear buttons belong to its field.
            var bar: UIView? = view
            while let current = bar, !(current is UISearchBar) { bar = current.superview }
            return frame(bar ?? view)
        }
        return OutsideTapRule.decide(at: point, field: field, keepers: keepers,
                                     popouts: order.map { OutsideTapRule.Popout(id: $0, regions: popouts[$0] ?? []) },
                                     through: through)
    }

    /// Puts away what `decision` says.
    func apply(_ decision: OutsideTapRule.Decision, in window: UIWindow) {
        if decision.keyboard { window.endEditing(true) }
        guard !decision.popouts.isEmpty else { return }
        for marker in live where marker.window === window {
            if case .popout(let id, true) = marker.role, decision.popouts.contains(id) { marker.close?() }
        }
    }

    /// A tap at `point` as the recognizer handles one, for drive steps (the device takes no
    /// synthetic touches): the same decision and the same closing. Returns what it put away.
    func tap(at point: CGPoint) -> OutsideTapRule.Decision? {
        guard let window = Self.keyWindow else { return nil }
        let decision = decide(at: point, in: window)
        apply(decision, in: window)
        return decision
    }

    static var keyWindow: UIWindow? {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first { $0.isKeyWindow }
    }

    /// The open pop-outs on screen, for the drive log.
    var openPopouts: [String] {
        var seen: [String] = []
        for marker in live {
            if case .popout(let id, true) = marker.role, marker.window != nil, !seen.contains(id) { seen.append(id) }
        }
        return seen
    }
}

/// Finds the first responder: the one the responder chain answers an untargeted action with.
@MainActor
private enum FirstResponder {
    private(set) static weak var found: UIResponder?

    static var current: UIResponder? {
        found = nil
        UIApplication.shared.sendAction(#selector(UIResponder.sn_noteFirstResponder), to: nil, from: nil, for: nil)
        // Only a text input brings a keyboard.
        return found is UITextInput ? found : nil
    }

    fileprivate static func note(_ responder: UIResponder) { found = responder }
}

extension UIResponder {
    @objc fileprivate func sn_noteFirstResponder() {
        MainActor.assumeIsolated { FirstResponder.note(self) }
    }
}

/// Watches every touch in its window. A tap that would put something away is used up: it
/// recognizes, which cancels the touch for the views under it, and the taps waiting on it fail.
/// A drag (more than 10 pt) or a hold (0.5 s) fails it, so scrolling works as ever, and hold menus
/// never wait on it. A touch outside the screen on top (a sheet's dimmed surround, a menu, an
/// alert) is the system's own: it closes that sheet or menu in one tap.
private final class OutsideTapRecognizer: UIGestureRecognizer, UIGestureRecognizerDelegate {
    private var decision: OutsideTapRule.Decision?
    private var start: CGPoint = .zero
    private var began: TimeInterval = 0
    private var hold: DispatchWorkItem?

    override init(target: Any?, action: Selector?) {
        super.init(target: target, action: action)
        cancelsTouchesInView = true
        delaysTouchesBegan = false
        delaysTouchesEnded = true
        delegate = self
    }

    private var window: UIWindow? { view as? UIWindow }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let window, touches.count == 1, (event.allTouches?.count ?? 0) <= 1, let touch = touches.first else {
            state = .failed
            return
        }
        start = touch.location(in: window)
        began = touch.timestamp
        let mine = MainActor.assumeIsolated { () -> Bool in
            guard let top = OutsideTap.topView(of: window), let hit = touch.view else { return false }
            return hit.isDescendant(of: top)
        }
        guard mine else {
            state = .failed
            return
        }
        let decision = MainActor.assumeIsolated { OutsideTap.shared.decide(at: start, in: window) }
        guard decision.closesAnything else {
            state = .failed
            return
        }
        self.decision = decision
        // A finger that stays down is a hold: fail before the hold menus wait on this too long.
        let item = DispatchWorkItem { [weak self] in
            if self?.state == .possible { self?.state = .failed }
        }
        hold = item
        DispatchQueue.main.asyncAfter(deadline: .now() + OutsideTapRule.longestTap, execute: item)
    }

    override func touchesMoved(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let window, let touch = touches.first else { return }
        let p = touch.location(in: window)
        if hypot(p.x - start.x, p.y - start.y) > OutsideTapRule.slop { state = .failed }
    }

    override func touchesEnded(_ touches: Set<UITouch>, with event: UIEvent) {
        guard let window, let touch = touches.first, let decision else {
            state = .failed
            return
        }
        let p = touch.location(in: window)
        guard OutsideTapRule.isTap(distance: hypot(p.x - start.x, p.y - start.y), duration: touch.timestamp - began) else {
            state = .failed
            return
        }
        if decision.usedUp {
            // Recognizing cancels the touch for what is under it.
            MainActor.assumeIsolated { OutsideTap.shared.apply(decision, in: window) }
            state = .ended
        } else {
            // Failing lets the press that waited on this land first, where it was pressed; then
            // the keyboard goes (a sheet riding the keyboard moves only after the press).
            state = .failed
            DispatchQueue.main.async { OutsideTap.shared.apply(decision, in: window) }
        }
    }

    override func touchesCancelled(_ touches: Set<UITouch>, with event: UIEvent) {
        state = .failed
    }

    override func reset() {
        super.reset()
        decision = nil
        hold?.cancel()
        hold = nil
    }

    /// While something would close, taps elsewhere wait for this one to fail, so the tap that
    /// closes presses nothing. Scrolling, pinching, hold menus and watchers that only observe
    /// touches (they cancel nothing) never wait; a hold that begins ends this one.
    override func shouldBeRequiredToFail(by other: UIGestureRecognizer) -> Bool {
        guard let window, other.view?.window === window, other.cancelsTouchesInView,
              !(other is UIPanGestureRecognizer), !(other is UIPinchGestureRecognizer) else { return false }
        if let hold = other as? UILongPressGestureRecognizer, hold.minimumPressDuration >= 0.3 { return false }
        return MainActor.assumeIsolated { OutsideTap.shared.armed(in: window) }
    }

    // A scroll that starts takes the touch; this one then fails on its own.
    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                           shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        false
    }
}

/// An invisible view behind a SwiftUI view that says what it is to `OutsideTap`. It never takes
/// a touch.
final class OutsideTapMarker: UIView {
    enum Role {
        case popout(String, Bool)
        case anchor(String)
        case keeper
        case through
        /// Only puts the recognizer on its window.
        case window

        var isOpenPopout: Bool {
            if case .popout(_, true) = self { return true }
            return false
        }
    }

    var role: Role = .keeper
    var close: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isUserInteractionEnabled = false
        backgroundColor = .clear
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        MainActor.assumeIsolated {
            if let window {
                OutsideTap.shared.install(on: window)
                OutsideTap.shared.add(self)
            } else {
                OutsideTap.shared.remove(self)
            }
        }
    }
}

private struct OutsideTapMarkerView: UIViewRepresentable {
    let role: OutsideTapMarker.Role
    var close: (() -> Void)?

    func makeUIView(context: Context) -> OutsideTapMarker {
        let view = OutsideTapMarker()
        view.role = role
        view.close = close
        return view
    }

    func updateUIView(_ view: OutsideTapMarker, context: Context) {
        view.role = role
        view.close = close
    }
}

/// Puts the outside-tap recognizer on the app's window from the root view, so a keyboard opened
/// on a screen with no pop-out still goes away on a tap outside it.
struct OutsideTapInstaller: UIViewRepresentable {
    func makeUIView(context: Context) -> OutsideTapMarker {
        let view = OutsideTapMarker()
        view.role = .window
        return view
    }

    func updateUIView(_ view: OutsideTapMarker, context: Context) {}
}

extension View {
    /// This view is a pop-out: while `isPresented`, a tap outside it closes it, pressing nothing
    /// else. Its toggle button carries `.outsideTapAnchor(id)` with the same `id`.
    func closesOnOutsideTap(_ isPresented: Binding<Bool>, id: String) -> some View {
        background(OutsideTapMarkerView(role: .popout(id, isPresented.wrappedValue)) { isPresented.wrappedValue = false })
    }

    /// The button that opens and closes pop-out `id`: pressing it while that pop-out is open is
    /// the button's to handle, not a tap outside.
    func outsideTapAnchor(_ id: String) -> some View {
        background(OutsideTapMarkerView(role: .anchor(id)))
    }

    /// A field's own controls: a press here acts and the keyboard stays.
    func keepsKeyboard() -> some View {
        background(OutsideTapMarkerView(role: .keeper))
    }

    /// A tap here is plainly meant for what it lands on (search results, a form's buttons and
    /// switches): it puts the keyboard away and presses that too.
    func tapsThroughKeyboard() -> some View {
        background(OutsideTapMarkerView(role: .through))
    }
}
