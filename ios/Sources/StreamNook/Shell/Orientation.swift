import SwiftUI
import UIKit

/// Portrait everywhere; the full watch layer alone may turn to landscape.
@MainActor
final class AppDelegate: NSObject, UIApplicationDelegate {
    static var orientations: UIInterfaceOrientationMask = .portrait

    /// The notification delegate goes in before launch ends, so a tapped mention or go-live that
    /// opened the app reaches it. Installing it asks for nothing. The background live check's
    /// handler must be registered before launch ends too.
    func application(_ application: UIApplication,
                     didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil) -> Bool {
        NotificationRouter.install()
        LiveCheck.register()
        return true
    }

    /// The push token go-live alerts are sent to (`LiveAlerts.sync` asks for it).
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        LiveAlerts.registered(token: deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        LiveAlerts.failed(error)
    }

    func application(_ application: UIApplication, supportedInterfaceOrientationsFor window: UIWindow?) -> UIInterfaceOrientationMask {
        Self.orientations
    }
}

@MainActor
enum Orientation {
    /// Lets the screen turn with the phone (watching full), or holds it in portrait, turning back
    /// at once if it is on its side.
    static func allowLandscape(_ allow: Bool) {
        let mask: UIInterfaceOrientationMask = allow ? .allButUpsideDown : .portrait
        guard AppDelegate.orientations != mask else { return }
        AppDelegate.orientations = mask
        for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
            for window in scene.windows { window.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations() }
            if !allow, scene.effectiveGeometry.interfaceOrientation.isLandscape {
                scene.requestGeometryUpdate(.iOS(interfaceOrientations: .portrait))
            }
        }
    }

    /// The player's rotate button: turns the screen to landscape or portrait and holds it there,
    /// whatever the phone's own rotation lock says. Leaving the full player calls
    /// `allowLandscape(false)` and the next one `allowLandscape(true)`, which hands the screen back
    /// to the sensor.
    static func hold(landscape: Bool) {
        AppDelegate.orientations = landscape ? .landscape : .portrait
        for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
            for window in scene.windows { window.rootViewController?.setNeedsUpdateOfSupportedInterfaceOrientations() }
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: landscape ? .landscapeRight : .portrait)) { error in
                log.error("rotate failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    #if DEBUG
    /// Turns the screen from code, for drive steps (the phone stays where it is).
    static func turn(landscape: Bool) {
        for scene in UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }) {
            scene.requestGeometryUpdate(.iOS(interfaceOrientations: landscape ? .landscapeRight : .portrait)) { error in
                log.error("turn failed: \(error.localizedDescription, privacy: .public)")
            }
        }
    }
    #endif
}
