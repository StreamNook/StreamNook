import Foundation
import Testing
@testable import SNKit

@Suite struct ModSuiteTests {
    private func core() throws -> FakeCore {
        let core = FakeCore()
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "test",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        return core
    }

    @Test func theSwipeSliderDecodesWithRustsRuler() async throws {
        let slider = try await core().swipeSlider(width: 390)
        #expect(slider.enabled)
        #expect(slider.steps.map(\.label) == ["Delete", "1m", "10m", "1h", "Ban"])
        #expect(slider.starts == [72, 106, 140, 174, 242])
        #expect(slider.steps[2].chip == "Time out Grumpy_Gus 10m")
        #expect(slider.steps[0].tone == nil)
    }

    /// The same pulls as Rust's `the_cancel_zone_the_gap_before_ban_and_ban`.
    @Test func stepsUnderThePullMatchRust() async throws {
        let slider = try await core().swipeSlider(width: 390)
        #expect(slider.step(at: 10) == nil)
        #expect(slider.step(at: 76) == 0)
        #expect(slider.step(at: 176) == 3)
        #expect(slider.step(at: 216) == 3, "the gap before Ban still shows 1h")
        #expect(slider.step(at: 246) == 4)
        #expect(slider.clamp(400) == 292.5, "the pull stops at the end of Ban's zone")
        #expect(slider.clamp(-20) == 0)
    }

    /// The same edges as Rust's `hysteresis_holds_a_step_near_its_edge`.
    @Test func hysteresisHoldsAStepNearItsEdge() async throws {
        let slider = try await core().swipeSlider(width: 390)
        #expect(slider.step(at: 107, keeping: 0) == 0)
        #expect(slider.step(at: 111, keeping: 0) == 1)
        #expect(slider.step(at: 104, keeping: 1) == 1)
        #expect(slider.step(at: 101, keeping: 1) == 0)
        #expect(slider.step(at: 20, keeping: 1) == nil)
    }

    @Test func eachStepNamesItsAction() async throws {
        let slider = try await core().swipeSlider(width: 390)
        #expect(slider.steps[0].action(messageID: "m1") as? [String: String] == ["kind": "delete", "messageId": "m1"])
        #expect(slider.steps[2].action(messageID: "m1") as? [String: AnyHashable] == ["kind": "timeout", "seconds": 600])
        #expect(slider.steps[4].action(messageID: "m1") as? [String: String] == ["kind": "ban"])
    }

    @Test func theRoomLogAndHeldQueueDecode() async throws {
        let core = try core()
        let log = try await core.roomModLog(channel: "zackrawrr")
        #expect(log.map(\.action) == ["timeout", "delete", "slow"])
        #expect(log[0].liftable)
        #expect(log[0].targetID == "42")
        #expect(log[2].target == nil)
        let held = try await core.heldQueue(channel: "zackrawrr")
        #expect(held.first?.label == "Held by AutoMod \u{00B7} Swearing, level 2")
    }

    @Test func typedLengthsAndStepEditsGoThroughTheCore() async throws {
        let core = try core()
        let typed = try await core.parseTimeout("1h30m")
        #expect(typed.seconds == 5_400)
        #expect(typed.label == "1h 30m")
        await #expect(throws: CoreFailure.self) { _ = try await core.parseTimeout("soon") }
        let added = try await core.editSliderTimeouts("add", seconds: 2_700)
        #expect(added.contains(2_700))
        await #expect(throws: CoreFailure.self) { _ = try await core.editSliderTimeouts("add", seconds: 0) }
    }
}

@Suite struct MessageReachTests {
    @Test func aModeratorMayDeleteOnlyTheirOwnMessage() {
        let own = MessageReach.of(viewerModerates: true, viewerIsBroadcaster: false, viewerID: "7", targetID: "7", badges: ["moderator/1"])
        #expect(own == .deleteOnly)
        #expect(own.canDelete && !own.canTimeout)
    }

    @Test func nobodyActsOnTheBroadcastersMessages() {
        #expect(MessageReach.of(viewerModerates: true, viewerIsBroadcaster: true, viewerID: "1", targetID: "1", badges: []) == .nothing)
        #expect(MessageReach.of(viewerModerates: true, viewerIsBroadcaster: false, viewerID: "7", targetID: "1", badges: ["broadcaster"]) == .nothing)
    }

    @Test func moderatorsAndEachOther() {
        #expect(MessageReach.of(viewerModerates: true, viewerIsBroadcaster: false, viewerID: "7", targetID: "8", badges: ["moderator"]) == .nothing)
        #expect(MessageReach.of(viewerModerates: true, viewerIsBroadcaster: true, viewerID: "1", targetID: "8", badges: ["moderator/1"]) == .noDelete)
        #expect(MessageReach.of(viewerModerates: true, viewerIsBroadcaster: false, viewerID: "7", targetID: "9", badges: ["subscriber/12"]) == .all)
    }
}
