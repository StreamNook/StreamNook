import Foundation
import Testing
@testable import SNKit

@Suite struct RewardsDropsTests {
    /// The core's shape, as services/rewards_drops.rs serializes it.
    static let coreJSON = #"""
    {"connected":true,"query":"","in_progress":[{"key":"progress:Rust","game":"Rust","art":"a","count_label":"1 campaign",
    "campaigns":[{"id":"c1","name":"Round 41","game_id":"263490","game_name":"Rust","art":"","claimed_label":"1/4",
    "percent":42.5,"detail":"4 rewards · 3d left","needs_link":false,"link_url":null,"meta_label":"1/4 claimed",
    "channels":[{"id":"9","name":"Shroud"}],"channels_label":"Earn on 1 channel",
    "find_stream":{"id":"263490","name":"Rust","box_art_url":"x"},
    "drops":[{"id":"d1","name":"Door","image":"","claimed":false,"done":false,"status":"12m left","detail":null,"bar":80.0},
             {"id":"d2","name":"Badge","image":"","claimed":false,"done":false,"status":"","detail":"Subscribe or gift a sub","bar":null}]}]}],
    "available":[],"earned":[{"id":"e1","name":"Hat","image":"","game":null}],
    "in_progress_empty":"No drop campaigns in progress. Watch a drops-enabled stream to start earning."}
    """#

    @Test func decodesTheCoresSegment() throws {
        let drops = try JSONDecoder().decode(RewardsDrops.self, from: Data(Self.coreJSON.utf8))
        #expect(drops.connected)
        let card = try #require(drops.inProgress.first?.campaigns.first)
        #expect(card.started)
        #expect(card.channels.first?.login == "shroud")
        #expect(card.findStream?.name == "Rust")
        #expect(card.drops[0].bar == 80)
        #expect(card.drops[1].bar == nil)
        #expect(drops.earned.first?.game == nil)
        #expect(drops.group(holding: "c1")?.key == "progress:Rust")
        #expect(drops.group(holding: "nope") == nil)
    }

    @Test func fakeAnswersDecodeAndFilter() async throws {
        let rewards = FakeRewards()
        let all = try #require(try rewards.answer("get_rewards_drops", ["query": ""]))
        let drops = try JSONDecoder().decode(RewardsDrops.self, from: Data(all.utf8))
        #expect(drops.inProgress.count == 2)
        #expect(!drops.available.isEmpty)
        let some = try #require(try rewards.answer("get_rewards_drops", ["query": "apex"]))
        let found = try JSONDecoder().decode(RewardsDrops.self, from: Data(some.utf8))
        #expect(found.inProgress.isEmpty)
        #expect(found.available.first?.game == "Apex Legends")
        #expect(found.inProgressEmpty == "No drops in progress match that search.")
        rewards.connected = false
        let off = try #require(try rewards.answer("get_rewards_drops", [:]))
        #expect(try JSONDecoder().decode(RewardsDrops.self, from: Data(off.utf8)).connected == false)
        _ = try rewards.answer("claim_drop", ["dropId": "d2"])
        #expect(rewards.claims == ["d2"])
    }

    @Test func theWatchDropDecodesAndClearsAfterAClaim() throws {
        let rewards = FakeRewards()
        let first = try JSONDecoder().decode(WatchDrop?.self, from: Data(try #require(try rewards.answer("get_watch_drop", [:])).utf8))
        #expect(first?.complete == true)
        _ = try rewards.answer("claim_drop", ["dropId": first?.dropID ?? ""])
        let next = try JSONDecoder().decode(WatchDrop?.self, from: Data(try #require(try rewards.answer("get_watch_drop", [:])).utf8))
        #expect(next?.complete == false)
        rewards.connected = false
        #expect(try rewards.answer("get_watch_drop", [:]) == "null")
    }

    @Test func theFakeWallDecodes() throws {
        let json = try #require(try FakeRewards().answer("get_badge_wall", ["sort": "available"]))
        let wall = try JSONDecoder().decode(BadgeWall.self, from: Data(json.utf8))
        #expect(wall.tiles.first?.status == "available")
        #expect(wall.missing.state == "list")
        #expect(wall.tile(holding: "mouse/1")?.title == "Mouse Pad")
        #expect(wall.tiles.first { $0.owned }?.lit == true)
        #expect(wall.tiles.first { $0.status == "expired" }?.word == "ENDED")
    }
}
