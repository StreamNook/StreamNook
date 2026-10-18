import Foundation
import Testing
@testable import SNKit

@Suite struct StreamManagerTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String?) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(try #require(json).utf8))
    }

    @Test func fakeViewDecodes() throws {
        let fake = FakeStreamManager()
        let view = try decode(StreamManagerView.self, try fake.answer("get_stream_manager", [:]))
        #expect(view.role == "broadcaster")
        #expect(view.streamRows.map(\.id) == ["info", "poll", "prediction", "raid", "ads", "rewards"])
        #expect(view.canAnnounce)
        #expect(view.info?.category?.boxArt?.absoluteString.hasSuffix("-52x72.jpg") == true)
        #expect(view.chat?.modes.first?.valueLabel == "10 min")
        #expect(view.limits.pollChoicesMax == 5)
        #expect(view.sections.first { $0.id == "rewards" }?.badge == "3 waiting")
    }

    @Test func queueWritesOnlyTouchTheFake() throws {
        let fake = FakeStreamManager()
        let queue = try decode(RewardQueue.self, try fake.answer("get_reward_queue", [:]))
        #expect(queue.groups.map(\.heading) == ["Song request · 2", "Hydrate! · 1"])
        let done = try decode(ManagerOutcome.self, try fake.answer("stream_manager_act", [
            "action": ManagerAction.redemptions(ids: ["r1"], fulfill: true).argument,
        ]))
        #expect(done.ok)
        #expect(done.queue?.count == 2)
        #expect(done.view?.queueCount == 2)
        let paused = try decode(ManagerOutcome.self, try fake.answer("stream_manager_act", [
            "action": ManagerAction.reward(rewardID: "song", paused: true, enabled: nil).argument,
        ]))
        #expect(paused.queue?.groups.first?.paused == true)
        #expect(fake.acts == ["redemptions", "reward"])
    }

    @Test func actionsMatchTheCoresShape() throws {
        let draft = StreamInfoDraft(title: "Hi", category: ManagerCategory(id: "1", name: "Chess", boxArtURL: ""), tags: ["Chill"], language: "en")
        let save = ManagerAction.saveInfo(draft).argument
        #expect(save["kind"] as? String == "save_info")
        let d = try #require(save["draft"] as? [String: Any])
        #expect((d["category"] as? [String: Any])?["id"] as? String == "1")
        let poll = ManagerAction.createPoll(title: "Q", choices: ["a", "b"], seconds: 60, pointsPerVote: nil).argument
        #expect(poll["points_per_vote"] is NSNull)
        #expect(ManagerAction.chatMode(mode: "slow", on: true, value: 30).argument["value"] as? UInt32 == 30)
        #expect(ManagerAction.lockPrediction.argument["kind"] as? String == "lock_prediction")
        // Every argument survives JSON, as the bridge sends it.
        for action in [ManagerAction.saveInfo(draft), .endPoll(showResults: false), .runAd(seconds: 30), .shield(on: true),
                       .announce(text: "x", color: "blue"), .reward(rewardID: "r", paused: nil, enabled: false)] {
            #expect(JSONSerialization.isValidJSONObject(action.argument))
        }
    }

    @Test func raidCountsDownAndClears() throws {
        let fake = FakeStreamManager()
        let target = try decode([RaidTarget].self, try fake.answer("search_raid_targets", ["query": "moss"]))
        #expect(target.map(\.live) == [true, true, false])
        let raid = try decode(ManagerOutcome.self, try fake.answer("stream_manager_act", ["action": ManagerAction.raid(target[0]).argument]))
        let pending = try #require(raid.view?.raid)
        #expect(pending.targetName == "mossyrock")
        #expect(managerCountdown(pending.deadlineMS).hasPrefix("1:"))
        let canceled = try decode(ManagerOutcome.self, try fake.answer("stream_manager_act", ["action": ManagerAction.cancelRaid.argument]))
        #expect(canceled.view?.raid == nil)
        #expect(managerCountdown(0) == "0:00")
    }

    @Test func infoCheckFlagsBadTags() throws {
        let fake = FakeStreamManager()
        let check = try decode(InfoCheck.self, try fake.answer("check_stream_info", [
            "draft": StreamInfoDraft(title: "Hi", tags: ["ok", "two words"]).argument,
        ]))
        #expect(!check.ok)
        #expect(check.problem("tag:1") != nil)
        #expect(check.tagProblem != nil)
        #expect(check.titleCount == "2 / 140")
    }
}
