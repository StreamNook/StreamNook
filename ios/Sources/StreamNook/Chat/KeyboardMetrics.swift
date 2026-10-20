import UIKit

/// The keyboard's last height, so the emote panel can take the keyboard's place at the keyboard's
/// size (the system emoji keyboard's pattern): swapping one for the other leaves the composer where
/// it was. Kept across launches; before any keyboard has opened, a common keyboard's height.
@MainActor
enum KeyboardMetrics {
    private static let key = "sn-last-kb"
    /// A common portrait keyboard with its suggestion bar, home indicator included.
    static let defaultHeight: CGFloat = 336
    /// Below this a frame is an accessory bar or a hardware keyboard's strip, not a keyboard.
    private static let minimumHeight: CGFloat = 200

    private(set) static var lastHeight: CGFloat = {
        let saved = CGFloat(UserDefaults.standard.double(forKey: key))
        return saved >= minimumHeight ? saved : defaultHeight
    }()

    private static var watching: NSObjectProtocol?

    /// Starts following the keyboard (once).
    static func start() {
        guard watching == nil else { return }
        watching = NotificationCenter.default.addObserver(
            forName: UIResponder.keyboardWillChangeFrameNotification, object: nil, queue: .main
        ) { note in
            guard let frame = (note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue else { return }
            MainActor.assumeIsolated { record(frame) }
        }
    }

    private static func record(_ frame: CGRect) {
        let screen = UIScreen.main.bounds
        // Portrait only: the panel sizes itself for the phone held upright.
        guard screen.height > screen.width, frame.minY < screen.height, frame.height >= minimumHeight else { return }
        lastHeight = frame.height
        UserDefaults.standard.set(Double(frame.height), forKey: key)
    }

    /// The emote panel's height under the input row: the keyboard's, less what the chat already
    /// keeps below the composer with the keyboard down (the home-indicator strip the watch layer
    /// and Chats leave, less the 24 pt they reach into it) and the 8 pt gap above the panel, so the
    /// input row stays exactly where the keyboard put it.
    static var panelHeight: CGFloat {
        let inset = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow)?.safeAreaInsets.bottom ?? 0
        return max(200, lastHeight - max(0, inset - 24) - 8)
    }
}
