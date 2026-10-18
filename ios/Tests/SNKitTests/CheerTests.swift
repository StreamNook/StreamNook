import Foundation
import Testing
@testable import SNKit

@Suite struct CheerFlowTests {
    private func check(_ text: String, total: Int = 100, problems: [String] = []) -> CheerCheck {
        CheerCheck(channel: "nook", channelID: "1", text: text, total: total, balance: 300, problems: problems,
                   needsBits: !problems.isEmpty, plainText: "hi")
    }

    @Test func aHoldTheBarAlreadyShowedGoesOutAtOnce() {
        var flow = CheerFlow()
        flow.draft = check("hi Cheer100")
        #expect(flow.held(check("hi Cheer100 ")) == .send(check("hi Cheer100 ")))
        #expect(flow.status == .sending(check("hi Cheer100 ")))
    }

    @Test func anUnseenHoldAsksFirst() {
        var flow = CheerFlow()
        #expect(flow.held(check("hi Cheer100")) == .ask)
        #expect(flow.shown?.total == 100)
        flow.dismiss()
        #expect(flow.status == nil)
    }

    @Test func aHoldRustRefusesSaysWhyAndKeepsTheText() {
        var flow = CheerFlow()
        let short = check("Cheer500", total: 500, problems: ["You have 300 bits. This cheer needs 500."])
        #expect(flow.held(short) == .restore("Cheer500"))
        guard case .refused(_, let problem, let canContinue, let needsBits)? = flow.status else {
            Issue.record("not refused")
            return
        }
        #expect(problem.hasPrefix("You have 300 bits"))
        #expect(!canContinue && needsBits)
    }

    @Test func answersClearOrRefuse() {
        var flow = CheerFlow()
        let c = check("hi Cheer100")
        flow.draft = c
        flow.sending(c)
        #expect(flow.answered(CheerOutcome(sent: true, balance: 200), for: c) == nil)
        #expect(flow.status == nil && flow.draft == nil)
        flow.sending(c)
        #expect(flow.answered(CheerOutcome(sent: false, balance: 50, canContinue: true), for: c) == "hi Cheer100")
        guard case .refused(let shown, let problem, true, false)? = flow.status else {
            Issue.record("not refused")
            return
        }
        #expect(shown.balance == 50)
        #expect(problem == "Your cheer of 100 bits didn't go out.")
    }

    @Test func theFakeHoldsACheeringMessageAndNeverSpends() throws {
        let held = try #require(FakeRewards.heldCheer("gg Nook100 Cheer50", channel: "Nook"))
        let result = try JSONDecoder().decode(SendResult.self, from: Data(held.utf8))
        #expect(result.cheer?.total == 150)
        #expect(result.cheer?.plainText == "gg")
        #expect(result.dropLine == nil)
        #expect(FakeRewards.heldCheer("gg 100 times", channel: "nook") == nil)
        let rewards = FakeRewards()
        _ = try rewards.answer("send_cheer", ["text": "gg Nook100"])
        #expect(rewards.cheers == ["gg Nook100"])
        let bits = try JSONDecoder().decode(BitsState.self, from: Data(try #require(try rewards.answer("get_bits_state", [:])).utf8))
        #expect(bits.cheermotes.first?.tier(for: 250)?.bits == 100)
        #expect(bits.quickAmounts == [1, 100, 500, 1000])
    }
}
