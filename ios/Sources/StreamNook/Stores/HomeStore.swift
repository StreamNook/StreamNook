import Foundation
import Observation
import SNKit

/// The core's home snapshot as the tab screens read it: who you follow that is live, what Twitch
/// recommends, hype trains and watch streaks. Rust polls and pushes `home-snapshot`; this keeps
/// the latest copy and asks for a section only when a screen first needs it or is pulled.
@Observable @MainActor
final class HomeStore {
    private(set) var lists = HomeLists()
    /// The Browse list: the recommended section, or the busiest categories' streams without it.
    private(set) var recommended: [TwitchStream] = []
    private(set) var followedLoaded = false
    private(set) var recommendedLoaded = false
    /// Changes when a list is refreshed, so previews are fetched anew.
    private(set) var previewStamp = Int(Date().timeIntervalSince1970)

    private let core: CoreBridge
    private var listening: Task<Void, Never>?

    init(core: CoreBridge) {
        self.core = core
    }

    /// Follows the snapshot pushes for as long as the app runs.
    func start() {
        guard listening == nil else { return }
        let pushes = core.events.stream("home-snapshot")
        listening = Task { [weak self] in
            for await _ in pushes {
                guard let self else { return }
                await self.readSnapshot()
            }
        }
    }

    /// The followed-live list, asked of Twitch through the core. `pulled` renews the previews.
    func loadFollowing(pulled: Bool = false) async {
        if pulled { previewStamp += 1 }
        _ = try? await core.invoke("refresh_home_section", args: ["section": "followed_live"])
        _ = try? await core.invoke("refresh_home_section", args: ["section": "hype_trains"])
        await readSnapshot()
        followedLoaded = true
    }

    func loadRecommended(pulled: Bool = false) async {
        if pulled { previewStamp += 1 }
        do {
            recommended = try await Browse.load(core)
        } catch {
            log.error("browse failed: \(String(describing: error), privacy: .public)")
        }
        await readSnapshot()
        recommendedLoaded = true
        log.notice("browse listed \(self.recommended.count) streams")
    }

    func refreshHypeTrains() async {
        _ = try? await core.invoke("refresh_home_section", args: ["section": "hype_trains"])
    }

    /// New preview frames on the next paint, for a pulled list the core does not hold.
    func renewPreviews() {
        previewStamp += 1
    }

    /// The app came back to the front. The core reloads the followed list when nobody has asked
    /// for it for a minute (then hype trains and Shared Viewership), and says whether it did, so
    /// the previews are renewed with it. The lists land as `home-snapshot` pushes.
    func resumed() async {
        guard (try? await core.invoke("note_app_resumed", args: [:], as: Bool.self)) == true else { return }
        previewStamp += 1
        log.notice("resume refreshed the followed list")
    }

    /// Forgets the account's lists, for a sign-out.
    func signedOut() {
        lists.followedLive = []
        followedLoaded = false
    }

    private func readSnapshot() async {
        guard let snapshot = try? await core.invoke("get_home_snapshot", args: [:], as: HomeLists.self) else { return }
        lists = snapshot
        if !snapshot.recommended.isEmpty { recommended = snapshot.recommended }
    }
}
