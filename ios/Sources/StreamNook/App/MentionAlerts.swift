import Foundation
import SNKit
import UIKit
import UserNotifications

/// The iPhone's notifications while StreamNook runs. The core decides when a mention is worth
/// announcing (the setting, streamer mode, a per-chat cooldown, replay age, the chat not being on
/// screen) and says so with `mention-notify`; this posts it. It never asks for permission on its
/// own: that happens when someone turns an alert on, in Settings or on the setup wizard's page.
@MainActor
enum MentionAlerts {
    /// Posts the notification for an entry the core announced, under the category the core's
    /// alert list names. Without a grant nothing is posted and nothing is asked.
    static func post(_ entry: MentionEntry, category: String? = nil) async {
        let center = UNUserNotificationCenter.current()
        guard allowed(await center.notificationSettings().authorizationStatus) else { return }
        NotificationRouter.install()
        let content = UNMutableNotificationContent()
        content.title = MentionText.notificationTitle(entry)
        content.body = entry.text
        if let category { content.categoryIdentifier = category }
        // One stack per chat; the message id as the identifier, so a second delivery of the same
        // message replaces the first instead of showing twice.
        content.threadIdentifier = entry.room
        content.sound = .default
        content.userInfo = [NotificationRouter.mentionKey: entry.id]
        do {
            try await center.add(UNNotificationRequest(identifier: entry.id, content: content, trigger: nil))
        } catch {
            log.error("mention notification failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// Asks for permission the first time a notification setting is turned on; a grant already
    /// given (or refused) is kept as it is.
    static func askIfUndecided() async {
        let center = UNUserNotificationCenter.current()
        guard await center.notificationSettings().authorizationStatus == .notDetermined else { return }
        _ = try? await center.requestAuthorization(options: [.alert, .sound, .badge])
    }

    static func allowed(_ status: UNAuthorizationStatus) -> Bool {
        status == .authorized || status == .provisional || status == .ephemeral
    }

    /// Where notifications stand for StreamNook now.
    static func status() async -> UNAuthorizationStatus {
        await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
    }

    /// StreamNook's notification settings in the iPhone's Settings app.
    static func openSettings() {
        if let url = URL(string: UIApplication.openNotificationSettingsURLString) { UIApplication.shared.open(url) }
    }
}

/// Shows notifications as banners while StreamNook is in front, and routes a tapped mention to
/// its chat. Installed at launch (installing asks nothing), so a tap that opens the app is caught.
final class NotificationRouter: NSObject, UNUserNotificationCenterDelegate {
    static let mentionKey = "mention_id"
    @MainActor private static let shared = NotificationRouter()
    /// Opens a mention by its id; set once the app can open chats.
    @MainActor private static var handler: ((String) -> Void)?
    /// A mention tapped before the app could open it.
    @MainActor private static var pending: String?

    @MainActor static func install() {
        let center = UNUserNotificationCenter.current()
        if !(center.delegate is NotificationRouter) { center.delegate = shared }
    }

    /// The app is ready to open mentions; one tapped while it booted opens now.
    @MainActor static func handle(_ open: @escaping (String) -> Void) {
        handler = open
        if let id = pending {
            pending = nil
            open(id)
        }
    }

    @MainActor private static func route(_ id: String) {
        if let handler { handler(id) } else { pending = id }
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification) async
        -> UNNotificationPresentationOptions {
        [.banner, .sound, .list]
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse) async {
        guard response.actionIdentifier == UNNotificationDefaultActionIdentifier,
              let id = response.notification.request.content.userInfo[Self.mentionKey] as? String else { return }
        await MainActor.run { Self.route(id) }
    }
}
