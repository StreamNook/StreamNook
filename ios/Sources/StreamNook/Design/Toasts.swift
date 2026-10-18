import Observation
import SNKit
import SwiftUI
import UIKit

/// The app's toasts. The phone shows only errors and toasts that carry an action (Undo, Log In);
/// everything else is dropped here, so callers can say what the desktop says.
@Observable @MainActor
final class Toasts {
    struct Toast: Identifiable, Equatable {
        let id = UUID()
        let text: String
        let kind: ToastKind
        var action: String?
        var seconds: Double?
        var perform: (@MainActor () -> Void)?

        static func == (a: Toast, b: Toast) -> Bool { a.id == b.id }
    }

    /// Newest last.
    private(set) var shown: [Toast] = []

    func show(_ text: String, _ kind: ToastKind = .error, action: String? = nil, live: Bool = false,
              sticky: Bool = false, perform: (@MainActor () -> Void)? = nil) {
        guard kind.shows(hasAction: action != nil) else { return }
        let toast = Toast(text: text, kind: kind, action: action, seconds: ToastKind.seconds(live: live, sticky: sticky),
                          perform: perform)
        withAnimation(SNMotion.run(SNMotion.toastIn)) { shown.append(toast) }
        AccessibilityNotification.Announcement(text).post()
        if let seconds = toast.seconds {
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(seconds))
                self?.dismiss(toast)
            }
        }
    }

    func dismiss(_ toast: Toast) {
        withAnimation(SNMotion.run(.easeIn(duration: 0.16))) { shown.removeAll { $0.id == toast.id } }
    }
}

/// The toasts, bottom center just above the tab bar, newest lowest, each sliding up 32 points
/// with a fade. Drawn in a window of their own (see `ToastWindow`) so they float over sheets,
/// popovers and menus too.
struct ToastHost: View {
    let toasts: Toasts
    let spot: ToastSpot

    var body: some View {
        VStack(spacing: 8) {
            ForEach(toasts.shown) { toast in
                ToastCard(toast: toast) { toasts.dismiss(toast) }
                    .transition(.asymmetric(insertion: .opacity.combined(with: .offset(y: 32)), removal: .opacity))
            }
        }
        .onGeometryChange(for: CGRect.self) { proxy in
            proxy.frame(in: .global)
        } action: { frame in
            spot.frame = frame.height > 0 ? frame : nil
        }
        .frame(maxWidth: 380)
        .padding(.horizontal, 12)
        // The tab bar's height and gap, above the bottom safe area.
        .padding(.bottom, 56 + 12)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
        .ignoresSafeArea(.keyboard)
        .snMotionScope()
    }
}

/// One toast: a glass card with the words, a close button, and an accent action under them.
private struct ToastCard: View {
    let toast: Toasts.Toast
    let dismiss: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 10) {
                if toast.kind == .error {
                    Image(systemName: Symbol.error).font(.system(size: 16)).foregroundStyle(SN.error)
                }
                Text(toast.text)
                    .font(.snLabel)
                    .foregroundStyle(SN.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                Button(action: dismiss) {
                    Image(systemName: Symbol.close)
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(SN.textMuted)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Dismiss")
            }
            if let action = toast.action {
                Button(action) {
                    toast.perform?()
                    dismiss()
                }
                .font(.snLabelStrong)
                .foregroundStyle(SN.accent)
                .buttonStyle(.plain)
            }
        }
        .padding(16)
        .snGlass(RoundedRectangle(cornerRadius: 18, style: .continuous))
        .floatShadow()
        .accessibilityElement(children: .contain)
    }
}

/// Where the toasts are in their window, so every other touch passes through to the app.
@MainActor
final class ToastSpot {
    var frame: CGRect?
}

/// The toasts' own window, just above the app's, so a toast shows over the system sheets,
/// popovers, menus and dialogs the app opens. It takes a touch only on the toasts, and the status
/// bar follows the app under it.
@MainActor
final class ToastWindow {
    private static var shared: ToastWindow?
    private let window: PassWindow

    private init(scene: UIWindowScene, below: UIWindow, toasts: Toasts) {
        let spot = ToastSpot()
        let host = FollowingController(rootView: AnyView(ToastHost(toasts: toasts, spot: spot).preferredColorScheme(.dark)))
        host.below = below
        host.view.backgroundColor = .clear
        window = PassWindow(windowScene: scene)
        window.spot = spot
        window.windowLevel = UIWindow.Level(rawValue: UIWindow.Level.normal.rawValue + 0.5)
        window.backgroundColor = .clear
        window.rootViewController = host
        window.isHidden = false
    }

    /// Puts the window up once, over the app's window `below`.
    static func install(over below: UIWindow, toasts: Toasts) {
        guard shared == nil, let scene = below.windowScene else { return }
        shared = ToastWindow(scene: scene, below: below, toasts: toasts)
    }
}

private final class PassWindow: UIWindow {
    var spot: ToastSpot?

    override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
        guard let frame = spot?.frame, frame.contains(point) else { return nil }
        return super.hitTest(point, with: event)
    }
}

/// Leaves the status bar and the supported orientations to whatever the app shows under it.
private final class FollowingController: UIHostingController<AnyView> {
    weak var below: UIWindow?

    private var front: UIViewController? {
        var top = below?.rootViewController
        while let next = top?.presentedViewController, !next.isBeingDismissed { top = next }
        return top
    }

    override var childForStatusBarStyle: UIViewController? { front }
    override var childForStatusBarHidden: UIViewController? { front }
    override var supportedInterfaceOrientations: UIInterfaceOrientationMask {
        below?.rootViewController?.supportedInterfaceOrientations ?? .portrait
    }
}

/// Puts the toast window up once the app is in a window.
struct ToastWindowProbe: UIViewRepresentable {
    let toasts: Toasts

    func makeUIView(context: Context) -> ProbeView {
        let view = ProbeView()
        view.toasts = toasts
        view.isUserInteractionEnabled = false
        return view
    }

    func updateUIView(_ view: ProbeView, context: Context) {}

    final class ProbeView: UIView {
        var toasts: Toasts?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            guard let window, let toasts else { return }
            ToastWindow.install(over: window, toasts: toasts)
        }
    }
}
