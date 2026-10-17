import Foundation
import Observation
import SNKit

/// The wallet sheet's rewards: Rust's judged list for the channel (its last list paints at
/// once, a fresh read replaces it), re-read when the balance moves past the one it was judged
/// against, when the soonest cooldown ends, and after a redeem. Redeeming goes through Rust's
/// one door, `redeem_reward`.
@Observable @MainActor
final class RewardsMenuModel {
    let login: String
    private let core: CoreBridge
    private(set) var snapshot: ChannelRewardsSnapshot?
    private(set) var loading = true
    private(set) var error: String?
    /// The reward being redeemed.
    private(set) var redeeming: String?
    /// A short message at the top of the sheet: a redeem's outcome.
    private(set) var toast: (text: String, failed: Bool)?
    /// The emote an unlock gave, for the reveal.
    var revealed: UnlockedEmote?
    private var request = 0
    private var listening: Task<Void, Never>?
    private var cooldownTimer: Task<Void, Never>?

    init(login: String, core: CoreBridge) {
        self.login = login.lowercased()
        self.core = core
    }

    var rewards: [ChannelReward] { snapshot?.rewards ?? [] }

    /// The reward as the latest list judges it; gone when it left the list.
    func live(_ reward: ChannelReward) -> ChannelReward {
        rewards.first { $0.id == reward.id } ?? reward.gone()
    }

    func start() async {
        if listening == nil {
            let pushes = core.events.stream("channel-rewards")
            listening = Task { [weak self] in
                for await event in pushes {
                    guard let self,
                          let next = try? JSONDecoder().decode(ChannelRewardsSnapshot.self, from: Data(event.payloadJSON.utf8)),
                          next.login.lowercased() == self.login else { continue }
                    self.take(next)
                }
            }
        }
        let seq = request
        if let json = try? await core.invoke("get_cached_channel_rewards", args: ["channelLogin": login]),
           let cached = try? JSONDecoder().decode(ChannelRewardsSnapshot?.self, from: Data(json.utf8)), let cached,
           request == seq, snapshot == nil {
            take(cached)
        }
        await reload(initial: true)
    }

    func stop() {
        listening?.cancel()
        cooldownTimer?.cancel()
    }

    /// A fresh read; only the newest applies, and a failed background read keeps the list.
    func reload(initial: Bool = false) async {
        request += 1
        let seq = request
        do {
            // The command's parameter is named channelId and takes the login.
            let next = try await core.invoke("get_channel_rewards", args: ["channelId": login], as: ChannelRewardsSnapshot.self)
            guard seq == request else { return }
            take(next)
            error = nil
        } catch {
            guard seq == request else { return }
            if initial && snapshot == nil {
                self.error = (error as? CoreFailure)?.description ?? "Failed to load rewards"
            }
        }
        loading = false
    }

    /// The balance moved since the verdicts were made: have them judged again.
    func balanceChanged(_ balance: Int64?) {
        guard let balance, let judged = snapshot?.balance, balance != judged else { return }
        Task { await reload() }
    }

    private func take(_ next: ChannelRewardsSnapshot) {
        snapshot = next
        loading = false
        scheduleCooldownEnd()
    }

    /// One timer to the soonest cooldown end, a beat past it, then a fresh verdict.
    private func scheduleCooldownEnd() {
        cooldownTimer?.cancel()
        let soonest = rewards.compactMap(\.availability.cooldownUntilMS).min()
        guard let soonest else { return }
        let delay = max(0, Double(soonest) / 1000 - Date().timeIntervalSince1970) + 0.25
        cooldownTimer = Task { [weak self] in
            try? await Task.sleep(for: .seconds(min(delay, 6 * 3600)))
            guard !Task.isCancelled else { return }
            await self?.reload()
        }
    }

    /// Redeems through Rust. Returns whether it went through.
    func redeem(_ reward: ChannelReward, channelID: String, text: String? = nil, emote: (id: String, name: String)? = nil) async -> Bool {
        guard redeeming == nil else { return false }
        redeeming = reward.id
        defer { redeeming = nil }
        do {
            let outcome = try await core.invoke("redeem_reward", args: [
                "channelLogin": login, "channelId": channelID, "rewardId": reward.id,
                "text": text, "emoteId": emote?.id, "emoteName": emote?.name,
            ], as: RedeemOutcome.self)
            show(outcome.message, failed: !outcome.success)
            if outcome.success {
                if let emote = outcome.unlockedEmote { revealed = emote }
                _ = try? await core.invoke("refresh_channel_state", args: ["login": login, "section": "points"])
                await reload()
            }
            return outcome.success
        } catch {
            show((error as? CoreFailure)?.description ?? "Redemption failed", failed: true)
            return false
        }
    }

    /// The channel's sub emotes and their modified versions, for an unlock. The command's
    /// parameter is named channelId and takes the login.
    func loadEmotes() async throws -> [ModifiableEmote] {
        try await core.invoke("get_modifiable_emotes", args: ["channelId": login], as: [ModifiableEmote].self)
    }

    func show(_ text: String, failed: Bool) {
        toast = (text, failed)
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            if self?.toast?.text == text { self?.toast = nil }
        }
    }
}
