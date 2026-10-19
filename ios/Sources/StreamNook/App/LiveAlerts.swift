import SNKit
import SwiftUI
import UIKit

/// Go-live alerts on the iPhone. notify.streamnook.app pushes them through APNs, app open or
/// closed, for the channels the core registered for this device (followed, not muted, with
/// notifications and Live channels on). This asks the iPhone for the device token once alerts can
/// arrive, hands it to the core with the build's APNs environment and the app's bundle id as the
/// topic, and opens the stream when one is tapped. It never asks for permission itself.
@MainActor
enum LiveAlerts {
    private static var core: CoreBridge?

    /// Asks the iPhone for this device's push token when notifications are allowed and go-live
    /// alerts are on; the token arrives at `registered(token:)`. Cheap to repeat: the iPhone hands
    /// back the token it already has, and the core skips a device it registered a short while ago.
    static func sync(_ model: AppModel) async {
        guard model.started else { return }
        core = model.core
        let store = model.settingsStore
        let allowed = MentionAlerts.allowed(await MentionAlerts.status())
        guard LivePush.wantsToken(types: model.alertTypes, allowed: allowed, isOn: { store.bool($0, true) }) else { return }
        UIApplication.shared.registerForRemoteNotifications()
    }

    /// The token the iPhone issued, handed to the core with its environment and topic.
    static func registered(token: Data) {
        guard let core else { return }
        let hex = LivePush.hex(token)
        let env = environment
        let topic = Bundle.main.bundleIdentifier ?? ""
        Task {
            do {
                let outcome = try await core.invoke("push_set_apns_device", args: ["token": hex, "env": env, "topic": topic])
                log.notice("live alerts: \(outcome, privacy: .public) (\(env, privacy: .public))")
            } catch {
                log.error("live alerts registration failed: \(String(describing: error), privacy: .public)")
            }
        }
    }

    static func failed(_ error: Error) {
        log.error("no push token: \(String(describing: error), privacy: .public)")
    }

    /// `sandbox` for a development-signed build, `production` for TestFlight and the App Store,
    /// read from the profile this build was signed with (see `LivePush.environment`).
    private static let environment: String = LivePush.environment(
        profile: Bundle.main.url(forResource: "embedded", withExtension: "mobileprovision").flatMap { try? Data(contentsOf: $0) }
    )
}

extension AppModel {
    /// A tapped go-live alert: its stream plays full, the way a tapped mention opens its channel
    /// when nothing plays. The channel already playing only comes forward.
    func openLiveAlert(_ alert: LivePush.Alert) {
        if !isPlaying(alert) {
            let seed = TwitchStream(id: "", userLogin: alert.login.lowercased(), userName: alert.login, title: "",
                                    viewerCount: 0, gameName: "", userID: alert.channelID.isEmpty ? nil : alert.channelID,
                                    startedAt: alert.startedAt)
            startWatching(seed)
        }
        if nav.playerMode != .full {
            withAnimation(SNMotion.run(SNMotion.settleBack)) { nav.playerMode = .full }
        }
    }

    /// Whether the alert is about the Twitch channel playing now.
    func isPlaying(_ alert: LivePush.Alert) -> Bool {
        guard let playing = nav.playing, playing.isTwitch else { return false }
        return LivePush.isPlaying(alert, login: playing.userLogin, channelID: playing.userID)
    }
}
