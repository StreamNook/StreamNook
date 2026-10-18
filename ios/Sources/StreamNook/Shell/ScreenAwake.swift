import UIKit

/// Who wants the screen on right now: the playing stream, the Chats tab. The idle timer is one
/// app-wide switch, so each asks through `hold` and it stays off while anyone still holds it.
@MainActor
enum ScreenAwake {
    private static var holders: Set<String> = []

    static func hold(_ owner: String, _ on: Bool) {
        if on { holders.insert(owner) } else { holders.remove(owner) }
        UIApplication.shared.isIdleTimerDisabled = !holders.isEmpty
    }
}
