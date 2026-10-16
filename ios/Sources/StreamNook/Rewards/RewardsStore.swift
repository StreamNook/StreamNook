import Foundation
import Observation
import SNKit

/// The Rewards tab's state: the Drops segment the core builds (`get_rewards_drops`), the
/// search, which game groups are open, the Drops sign-in, and a campaign to bring into view when
/// the player's drop bar sends someone here. Lives on the app model so the drop bar can reach it.
@Observable @MainActor
final class RewardsStore {
    enum Part: Hashable { case drops, badges }

    private let core: CoreBridge
    var segment = Part.drops
    /// Nil until the first answer: the segment draws nothing while it is pending.
    private(set) var drops: RewardsDrops?
    var query = ""
    /// Open game groups by their key (`progress:<game>`, `available:<game>`).
    var openGroups: Set<String> = []
    private(set) var connecting = false
    /// Why the last connect failed, under the button.
    private(set) var connectError: String?
    /// The campaign whose sheet is up.
    var campaignSheet: CampaignCard?
    /// A campaign the drop bar asked to show; taken once it is on screen.
    var focusCampaign: String?
    /// The campaign ringed after a jump, for 1.8 s.
    private(set) var flashed: String?
    private var listening: Task<Void, Never>?

    init(core: CoreBridge) {
        self.core = core
    }

    var connected: Bool? { drops?.connected }

    /// A search opens every group it matched.
    func isOpen(_ key: String) -> Bool {
        !query.trimmingCharacters(in: .whitespaces).isEmpty || openGroups.contains(key)
    }

    func toggle(_ key: String) {
        if openGroups.contains(key) { openGroups.remove(key) } else { openGroups.insert(key) }
    }

    /// Each visit starts on Drops with no search and every group closed, as the phone does.
    func visited() {
        segment = .drops
        query = ""
        openGroups = []
    }

    /// Reads the segment: `refresh` asks Twitch again (a visit, a pull); otherwise the core
    /// filters its last read by the search. A failed call reads as not connected.
    func load(refresh: Bool) async {
        do {
            let next = try await core.invoke("get_rewards_drops", args: ["query": query, "refresh": refresh],
                                             as: RewardsDrops.self)
            // A newer search already answered: keep it.
            guard next.query == query.trimmingCharacters(in: .whitespaces) || refresh else { return }
            drops = next
        } catch {
            log.error("get_rewards_drops failed: \(String(describing: error), privacy: .public)")
            drops = RewardsDrops(connected: false)
        }
    }

    /// Opens Twitch's Drops authorize page in the web sheet. The grant comes back through the
    /// sheet; the core stores it and announces the outcome.
    func connect() async {
        connecting = true
        connectError = nil
        do {
            _ = try await core.invoke("open_drops_sheet", "{}")
        } catch {
            connecting = false
            connectError = "\(error)"
            log.error("open_drops_sheet failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The web sheet went away. Still connecting means Twitch never handed the grant back.
    func sheetClosed() {
        guard connecting else { return }
        // The outcome event can land a moment after the sheet closes.
        Task {
            try? await Task.sleep(for: .milliseconds(600))
            guard connecting else { return }
            connecting = false
            connectError = "Sign-in was closed before Twitch approved drops."
        }
    }

    /// Follows the core's Drops sign-in outcomes for the app's lifetime.
    func listen(toasts: Toasts) {
        guard listening == nil else { return }
        let events = core.events.all()
        listening = Task { [weak self] in
            for await event in events {
                guard let self else { return }
                switch event.name {
                case "drops-login-complete":
                    self.connecting = false
                    self.connectError = nil
                    toasts.show("Drops connected!", .success)
                    await self.load(refresh: true)
                case "drops-login-error":
                    self.connecting = false
                    let reason = CoreFailure.command(json: event.payloadJSON).description
                    self.connectError = reason
                    toasts.show("Drops connection failed: \(reason)")
                default:
                    continue
                }
            }
        }
    }

    /// The campaign to bring into view, once its card exists: its group opens, and the card is
    /// ringed for 1.8 s. Returns the id to scroll to.
    func takeFocus() -> String? {
        guard let id = focusCampaign, let drops, let group = drops.group(holding: id) else { return nil }
        focusCampaign = nil
        openGroups.insert(group.key)
        flashed = id
        Task {
            try? await Task.sleep(for: .seconds(1.8))
            if flashed == id { flashed = nil }
        }
        return id
    }
}
