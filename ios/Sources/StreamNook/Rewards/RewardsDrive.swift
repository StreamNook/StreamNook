#if DEBUG
import Foundation
import SNKit

/// Drive steps for the Rewards tab:
///
/// - `rewards:drops`, `rewards:badges` pick the segment
/// - `dsearch:TEXT` searches the Drops lists (`_` for spaces); `dsearch:` clears it
/// - `dopen:all` opens every game group; `dopen:KEY` one group by key (`progress:Rust`)
/// - `dsheet:INDEX` opens the campaign sheet of the INDEXth campaign (in progress, then available)
/// - `dfocus:ID` asks for a campaign the way the player's drop bar does
/// - `dstate` logs what the segment holds
/// - `bsort:NAME` sorts the badge wall (`newest`, `available`, `soon`, `usage`, `oldest`);
///   `bsheet:INDEX` opens a tile's sheet; `bmissing:all` shows every missing row; `bstate` logs
///   the wall
/// - `fakedrops:on`, `fakedrops:off` connect or disconnect FakeCore's drops
@MainActor
enum RewardsDrive {
    static func run(_ name: String, _ value: String, model: AppModel) async -> Bool {
        let rewards = model.rewards
        switch name {
        case "rewards":
            rewards.segment = value == "badges" ? .badges : .drops
        case "dsearch":
            rewards.query = value.replacingOccurrences(of: "_", with: " ")
            try? await Task.sleep(for: .milliseconds(500))
        case "dopen":
            guard let drops = await loaded(rewards) else { return true }
            let groups = drops.inProgress + drops.available
            rewards.openGroups.formUnion(value == "all" ? groups.map(\.key) : [value.replacingOccurrences(of: "_", with: " ")])
        case "dsheet":
            guard let drops = await loaded(rewards) else { return true }
            let cards = (drops.inProgress + drops.available).flatMap(\.campaigns)
            let index = Int(value) ?? 0
            guard cards.indices.contains(index) else {
                log.error("drive dsheet: no campaign \(index)")
                return true
            }
            rewards.campaignSheet = cards[index]
        case "dfocus":
            rewards.focusCampaign = value
            model.nav.tab = .rewards
        case "bsort":
            rewards.badgeSort = BadgeSort(rawValue: value) ?? .newest
        case "bsheet":
            for _ in 0..<50 where rewards.wall?.tiles.isEmpty ?? true { try? await Task.sleep(for: .milliseconds(200)) }
            let tiles = rewards.wall?.tiles ?? []
            let index = Int(value) ?? 0
            if tiles.indices.contains(index) { rewards.badgeSheet = tiles[index] }
        case "bmissing":
            rewards.missingExpanded = value == "all"
        case "bstate":
            let wall = rewards.wall
            log.notice("drive bstate tiles \(wall?.tiles.count ?? -1) \(wall?.countLabel ?? "", privacy: .public) missing \(wall?.missing.state ?? "-", privacy: .public) rows \(wall?.missing.rows.count ?? -1) sync \(wall?.syncLeft ?? -1) first \(wall?.tiles.first?.title ?? "", privacy: .public)")
        case "dstate":
            let drops = rewards.drops
            log.notice("drive dstate connected \(drops?.connected ?? false) progress \(drops?.inProgress.count ?? -1) available \(drops?.available.count ?? -1) earned \(drops?.earned.count ?? -1) query '\(drops?.query ?? "", privacy: .public)'")
        case "fakedrops":
            guard let fake = model.core as? FakeCore, let fixtures = fake.fixtures else {
                log.error("drive fakedrops: not FakeCore")
                return true
            }
            fixtures.rewards.connected = value != "off"
            await rewards.load(refresh: true)
        default:
            return false
        }
        return true
    }

    /// The segment once the core has answered, waiting up to 10 seconds.
    private static func loaded(_ rewards: RewardsStore) async -> RewardsDrops? {
        for _ in 0..<50 {
            if let drops = rewards.drops, drops.connected { return drops }
            try? await Task.sleep(for: .milliseconds(200))
        }
        log.error("drive: the drops segment never loaded")
        return nil
    }
}
#endif
