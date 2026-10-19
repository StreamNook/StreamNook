import Foundation
import Observation
import SNKit

/// Your Kick account as the core knows it: whether it holds a Kick sign-in, who it is, and how
/// many Kick channels you follow. The core runs the sign-in itself (`kick_account_sync`: the
/// kick.com login, then Kick's Authorize page, both in the web sheet it opens through host
/// requests); this only starts it, reads the result and keeps the row's words.
@Observable @MainActor
final class KickAccount {
    private(set) var account: RosterAccount?
    /// The platforms this build can watch (roster `can_watch`), read with the Kick row since both
    /// come from the one roster. Settings rows about a platform show only for these.
    private(set) var watchable: [String] = []
    /// Kick channels on your follow list, imported or added by hand.
    private(set) var follows = 0
    /// Kick channels you are subscribed to, by slug.
    private(set) var subscribedSlugs: Set<String> = []
    /// A sign-in or sign-out is running.
    private(set) var busy = false
    /// A re-read of your follows is running.
    private(set) var syncing = false
    /// What the row says instead of its status for a while (a step, or an outcome).
    private(set) var step: String?

    @ObservationIgnored private var core: CoreBridge?
    @ObservationIgnored private weak var toasts: Toasts?
    @ObservationIgnored private var listening: Task<Void, Never>?

    var connected: Bool { account?.connected == true }
    var name: String? { connected ? account?.name : nil }
    var avatarURL: String? { connected ? account?.avatarURL : nil }

    /// The row's second line.
    var detail: String { step ?? KickWords.status(connected: connected, follows: follows) }

    func start(core: CoreBridge, toasts: Toasts) {
        self.core = core
        self.toasts = toasts
        guard listening == nil else { return }
        let events = core.events.all()
        listening = Task { [weak self] in
            for await event in events {
                guard let self else { return }
                switch event.name {
                case "account-roster-changed", "platform-account-changed":
                    await self.refresh()
                case "home-snapshot" where event.payloadJSON.contains("\"section\":\"following\""):
                    // The follow list changed under the Following tab (an import, a sign-out).
                    await self.refresh()
                case "platform-session-expired":
                    if event.payloadJSON.contains("kick") { self.expired() }
                default:
                    continue
                }
            }
        }
        Task { await refresh() }
    }

    /// Reads the roster and the follow list. A failed read keeps what is shown.
    func refresh() async {
        guard let core else { return }
        if let roster = try? await core.invoke("get_account_roster", args: [:], as: AccountRoster.self) {
            account = roster.account("kick")
            watchable = roster.watchable
        }
        if let list = try? await core.invoke("get_provider_follows", args: [:], as: [ProviderFollow].self) {
            let kick = list.filter { $0.provider == "kick" }
            follows = kick.count
            subscribedSlugs = Set(kick.filter(\.subscribed).map { $0.channel.lowercased() })
        }
    }

    /// "Sign in with Kick": one action, the kick.com login then Kick's Authorize page.
    func connect() async {
        guard let core, !busy else { return }
        busy = true
        step = KickWords.waiting
        do {
            let sync = try await core.invoke("kick_account_sync", args: ["interactive": true], as: KickSync.self)
            log.notice("kick sign-in imported \(sync.imported) follows")
            step = nil
        } catch {
            let reason = KickWords.outcome(Self.words(error))
            log.error("kick sign-in: \(reason, privacy: .public)")
            show(reason)
            toasts?.show(reason)
        }
        busy = false
        await refresh()
        // The follows can come in while Kick's Authorize step was left unfinished: the row would
        // read "Not signed in" with nothing said, so it says the sign-in did not complete.
        if !connected, step == nil {
            let reason = KickWords.outcome("")
            log.notice("kick sign-in returned without Kick's authorization")
            show(reason)
            toasts?.show(reason)
        }
    }

    /// Signs out of Kick: the core drops the token, the sign-in browser's kick.com cookies and
    /// the imported follows. An open Kick stream and its chat keep running read-only.
    func disconnect() async {
        guard let core, !busy else { return }
        busy = true
        do {
            _ = try await core.invoke("kick_disconnect", args: [:])
        } catch {
            log.error("kick_disconnect failed: \(String(describing: error), privacy: .public)")
        }
        follows = 0
        await refresh()
        busy = false
        step = nil
    }

    /// The refresh icon: a silent re-read while the kick.com session lives, else the sign-in.
    func resync() async {
        guard let core, !busy, !syncing else { return }
        syncing = true
        defer { syncing = false }
        let synced = (try? await core.invoke("kick_account_is_synced", args: [:], as: Bool.self)) ?? false
        do {
            if !synced { busy = true; step = KickWords.waiting }
            let sync = try await core.invoke("kick_account_sync", args: ["interactive": !synced], as: KickSync.self)
            busy = false
            step = nil
            toasts?.show(KickWords.found(sync.imported), .success)
        } catch {
            busy = false
            step = nil
            log.error("kick resync failed: \(String(describing: error), privacy: .public)")
            toasts?.show(KickWords.refreshFailed)
        }
        await refresh()
    }

    /// Whether you subscribe to this Kick channel.
    func subscribed(to slug: String) -> Bool { subscribedSlugs.contains(slug.lowercased()) }

    private func expired() {
        toasts?.show("Your Kick session expired.", .warning, action: "Reconnect") { [weak self] in
            Task { await self?.connect() }
        }
        Task { await refresh() }
    }

    /// An outcome stays on the row for six seconds unless something replaced it.
    private func show(_ message: String) {
        step = message
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(6))
            if self?.step == message { self?.step = nil }
        }
    }

    private static func words(_ error: Error) -> String {
        if case CoreFailure.command(let json) = error {
            if let text = try? JSONDecoder().decode(String.self, from: Data(json.utf8)) { return text }
            return json
        }
        return "\(error)"
    }
}
