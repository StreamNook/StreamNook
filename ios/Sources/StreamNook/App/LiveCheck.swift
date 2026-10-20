import BackgroundTasks
import Foundation
import SNKit
import UIKit
import UserNotifications

/// The fallback for go-live alerts the push did not bring, like Android's 15-minute poll and its
/// check on resume. iOS runs a background app refresh when it chooses (no sooner than
/// `LivePush.refreshInterval` after it is asked for, often later, sometimes not at all), and the
/// app checks on every return to it, at most every five minutes. Each run starts the core if it
/// has to and
/// calls `live_poll_check`, which runs Android's live check with the same rules (followed, not
/// muted, notifications and Live channels on) and answers with the go-lives to post. They post
/// under the push's own identifier and thread, `live-<channel_id>`, so a push and a polled alert
/// for one broadcast are one banner.
///
/// A push shows with no app code running, so the core cannot see it by itself. The app hands it
/// what it can see: the go-lives still on screen at each run, and every go-live it presents,
/// opens or finds on return to the app (`live_poll_seen`). The core skips those broadcasts.
@MainActor
enum LiveCheck {
    /// The app's model, set when it is made. A refresh that launches the app finds it here.
    static weak var model: AppModel?

    /// Registers the refresh's handler. iOS requires it before launch ends (the app delegate).
    static func register() {
        // On the main queue, so the handler runs on the main actor.
        let registered = BGTaskScheduler.shared.register(forTaskWithIdentifier: LivePush.refreshTask, using: .main) { @Sendable task in
            // Handed over on the main queue and used only there.
            nonisolated(unsafe) let task = task
            MainActor.assumeIsolated {
                if let refresh = task as? BGAppRefreshTask {
                    handle(refresh)
                } else {
                    task.setTaskCompleted(success: false)
                }
            }
        }
        if !registered { log.error("live check: the refresh was not registered") }
    }

    /// On every move to the background: asks for the next refresh while go-live alerts can arrive
    /// (the same test the push token waits for), and drops a pending one when they cannot. Before
    /// boot has read the alert types it leaves things as they are.
    static func backgrounded(_ model: AppModel) async {
        guard model.started, !model.alertTypes.isEmpty else { return }
        let store = model.settingsStore
        let allowed = MentionAlerts.allowed(await MentionAlerts.status())
        schedule(LivePush.wantsToken(types: model.alertTypes, allowed: allowed, isOn: { store.bool($0, true) }))
    }

    /// Asks iOS for the next refresh, or drops the one pending.
    static func schedule(_ wanted: Bool) {
        let scheduler = BGTaskScheduler.shared
        guard wanted else {
            scheduler.cancel(taskRequestWithIdentifier: LivePush.refreshTask)
            return
        }
        let request = BGAppRefreshTaskRequest(identifier: LivePush.refreshTask)
        request.earliestBeginDate = Date(timeIntervalSinceNow: LivePush.refreshInterval)
        do {
            try scheduler.submit(request)
        } catch {
            log.error("live check not scheduled: \(String(describing: error), privacy: .public)")
        }
    }

    private static func handle(_ task: BGAppRefreshTask) {
        // The next one first, so a run that runs out of time still leaves one asked for.
        schedule(true)
        let finish = Finish(task)
        let work = Task { @MainActor in
            let active = await run()
            if active == false { schedule(false) }
            finish.done(active != nil)
        }
        // iOS calls this on a queue of its own; it must not assume the main actor.
        task.expirationHandler = { @Sendable in
            work.cancel()
            finish.done(false)
        }
    }

    /// One live check: starts the core when this refresh launched the app, hands it the go-lives
    /// on screen, and posts what it found. True while go-live alerts are on, false when they are
    /// off (or notifications are not allowed), nil when the check could not run.
    @discardableResult
    static func run() async -> Bool? {
        guard let model else {
            log.error("live check: no app model to run on")
            return nil
        }
        let wasRunning = model.started
        guard await model.startCore() else { return nil }
        lastRun = Date()
        // Started for this check, with no screen: the core is told it runs in the background.
        if !wasRunning, UIApplication.shared.applicationState == .background { model.core.lifecycle("background") }
        guard MentionAlerts.allowed(await MentionAlerts.status()) else { return false }
        let (args, onScreen) = await deliveredArgs(key: "delivered")
        do {
            let json = try await model.core.invoke("live_poll_check", args)
            let check = try JSONDecoder().decode(LivePush.Check.self, from: Data(json.utf8))
            for found in check.alerts { await post(found) }
            log.notice("live check: \(onScreen) go-lives on screen, \(check.alerts.count) posted, active \(check.active)")
            return check.active
        } catch {
            log.error("live check failed: \(String(describing: error), privacy: .public)")
            return nil
        }
    }

    /// When the last check ran in this process.
    private static var lastRun: Date?
    /// Returns to the app check at most this often, like Android's resume check.
    private static let foregroundGap: TimeInterval = 5 * 60

    /// On every return to the app: records the go-lives that arrived meanwhile, then runs the
    /// check, at most once per five minutes (a burst of returns runs it once), as Android does on
    /// resume.
    static func foregrounded() async {
        await recordDelivered()
        if let lastRun, Date().timeIntervalSince(lastRun) < foregroundGap { return }
        await run()
    }

    /// Records the go-lives on screen now (pushes that arrived while the app was away).
    static func recordDelivered() async {
        guard let model, model.started else { return }
        let (args, onScreen) = await deliveredArgs(key: "seen")
        guard onScreen > 0 else { return }
        _ = try? await model.core.invoke("live_poll_seen", args)
    }

    /// Records one go-live the app presented, opened or saw dismissed. Returns whether it was new
    /// to the record (false: the live check already showed this broadcast), nil when the core
    /// could not be asked.
    static func recordSeen(_ sighting: String) async -> Bool? {
        guard let model, await model.startCore() else { return nil }
        guard let json = try? await model.core.invoke("live_poll_seen", "{\"seen\":[\(sighting)]}"),
              let fresh = try? JSONDecoder().decode([String].self, from: Data(json.utf8)) else { return nil }
        return !fresh.isEmpty
    }

    private static func post(_ found: LivePush.Found) async {
        let content = UNMutableNotificationContent()
        content.title = found.title
        content.body = found.body
        content.sound = .default
        content.threadIdentifier = LivePush.notificationID(channelID: found.channelID)
        content.userInfo = LivePush.userInfo(found)
        let request = UNNotificationRequest(identifier: LivePush.notificationID(channelID: found.channelID),
                                            content: content, trigger: nil)
        do {
            try await UNUserNotificationCenter.current().add(request)
        } catch {
            log.error("go-live notification failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The go-live notifications on screen as a command's arguments, `{key: [sighting]}`, and how
    /// many there are.
    nonisolated private static func deliveredArgs(key: String) async -> (String, Int) {
        let delivered = await UNUserNotificationCenter.current().deliveredNotifications()
        let sightings = delivered.compactMap {
            LivePush.sighting(identifier: $0.request.identifier, userInfo: $0.request.content.userInfo, date: $0.date)
        }
        let data = (try? JSONSerialization.data(withJSONObject: [key: sightings])) ?? Data("{\"\(key)\":[]}".utf8)
        return (String(decoding: data, as: UTF8.self), sightings.count)
    }

    /// One notification as a sighting's JSON, for `recordSeen`.
    nonisolated static func sightingJSON(_ notification: UNNotification) -> String? {
        guard let sighting = LivePush.sighting(identifier: notification.request.identifier,
                                               userInfo: notification.request.content.userInfo,
                                               date: notification.date),
              let data = try? JSONSerialization.data(withJSONObject: sighting) else { return nil }
        return String(decoding: data, as: UTF8.self)
    }

    /// Completes the refresh once, whichever of the run and the expiration is first.
    private final class Finish: @unchecked Sendable {
        private let lock = NSLock()
        private var task: BGAppRefreshTask?

        init(_ task: BGAppRefreshTask) {
            self.task = task
        }

        func done(_ success: Bool) {
            let task = lock.withLock {
                defer { self.task = nil }
                return self.task
            }
            task?.setTaskCompleted(success: success)
        }
    }
}
