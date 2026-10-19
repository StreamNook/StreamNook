import Foundation
import Testing
@testable import SNKit

@Suite struct LiveVotesTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    private static let prediction = #"""
    {
      "channel_id": "123", "prediction_id": "p1", "title": "Beats it first try?",
      "phase": "open",
      "deadline_ms": 1700000090000,
      "badge": null,
      "outcomes": [{ "id": "yes", "title": "Yes", "color": "blue", "percent": 36, "ratio": "1:2.78",
                     "points_label": "3.6K", "people_label": "412", "winner": false, "mine": false }],
      "can_predict": true,
      "picked": null,
      "status_line": null,
      "result": null,
      "totals_line": "1,100 people · 10K Nooks",
      "points_name": "Nooks",
      "points_icon": "https://..."
    }
    """#

    private static let poll = #"""
    { "channel_id": "123", "poll_id": "q1", "title": "Which boss next?", "phase": "open",
      "deadline_ms": 1700000042000, "badge": null,
      "choices": [{ "id": "a", "title": "Malenia", "percent": 58, "votes_label": "698", "mine": false, "winner": false }],
      "can_vote": true, "my_choice_id": null, "footer": "Tap a choice to vote", "voters_line": "1,204 voters",
      "result_line": null }
    """#

    @Test func decodesThePredictionView() throws {
        let view = try decode(PredictionView.self, Self.prediction)
        #expect(view.channelID == "123" && view.predictionID == "p1")
        #expect(view.isOpen && view.deadlineMS == 1_700_000_090_000 && view.badge == nil)
        #expect(view.canPredict && view.picked == nil && view.statusLine == nil && view.result == nil)
        #expect(view.totalsLine == "1,100 people · 10K Nooks")
        #expect(view.pointsName == "Nooks" && view.pointsIcon == "https://...")
        let yes = try #require(view.outcome("yes"))
        #expect(yes.title == "Yes" && yes.color == "blue" && yes.percent == 36 && yes.ratio == "1:2.78")
        #expect(yes.pointsLabel == "3.6K" && yes.peopleLabel == "412" && !yes.winner && !yes.mine)
    }

    @Test func decodesPushesAndAnEndedResult() throws {
        let push = try decode(PredictionPush.self, #"{ "channel_id": "123", "view": \#(Self.prediction) }"#)
        #expect(push.channelID == "123" && push.view?.title == "Beats it first try?")
        let gone = try decode(PredictionPush.self, #"{ "channel_id": "123", "view": null }"#)
        #expect(gone.view == nil)
        let ended = Self.prediction
            .replacingOccurrences(of: #""result": null"#, with: #""result": { "tone": "won", "line": "Yes won · you won 280 Nooks" }"#)
            .replacingOccurrences(of: #""phase": "open""#, with: #""phase": "ended""#)
        let view = try decode(PredictionView.self, ended)
        #expect(view.result == ResultView(tone: "won", line: "Yes won · you won 280 Nooks"))
        #expect(!view.isOpen)
        let pollPush = try decode(PollPush.self, #"{ "channel_id": "123", "view": null }"#)
        #expect(pollPush.view == nil)
    }

    @Test func decodesTheBetAndTheOutcome() throws {
        let bet = try decode(BetView.self, #"""
        { "amount": 100, "max": 12850, "presets": [{ "label": "10", "amount": 10 }, { "label": "100", "amount": 100 },
          { "label": "1K", "amount": 1000 }, { "label": "Max", "amount": 12850 }],
          "ok": true, "problem": null, "button": "Predict 100 on Yes", "win_line": "Win up to 272",
          "confirm_title": "Predict 100 Nooks on Yes?", "confirm_line": "Win up to 272 if Yes wins.",
          "confirm_note": "You can add more later, but you can't switch sides.", "balance_line": "12,850 Nooks" }
        """#)
        #expect(bet.amount == 100 && bet.max == 12_850 && bet.ok && bet.problem == nil)
        #expect(bet.presets.map(\.label) == ["10", "100", "1K", "Max"])
        #expect(bet.presets.last?.amount == 12_850)
        #expect(bet.button == "Predict 100 on Yes" && bet.winLine == "Win up to 272")
        #expect(bet.confirmTitle == "Predict 100 Nooks on Yes?" && bet.confirmLine == "Win up to 272 if Yes wins.")
        #expect(bet.confirmNote == "You can add more later, but you can't switch sides.")
        #expect(bet.balanceLine == "12,850 Nooks")
        let outcome = try decode(PredictOutcome.self, #"{ "success": true, "message": "Predicted 100 on Yes", "view": null }"#)
        #expect(outcome.success && outcome.message == "Predicted 100 on Yes" && outcome.view == nil)
        #expect(try decode(BetView?.self, "null") == nil)
    }

    @Test func decodesThePollView() throws {
        let view = try decode(PollView.self, Self.poll)
        #expect(view.channelID == "123" && view.pollID == "q1" && view.title == "Which boss next?")
        #expect(view.isOpen && view.deadlineMS == 1_700_000_042_000 && view.badge == nil)
        #expect(view.canVote && view.myChoiceID == nil && view.resultLine == nil)
        #expect(view.footer == "Tap a choice to vote" && view.votersLine == "1,204 voters")
        #expect(view.choices == [ChoiceView(id: "a", title: "Malenia", percent: 58, votesLabel: "698", mine: false, winner: false)])
        #expect(try decode(PollView?.self, "null") == nil)
    }

    @Test func theClockTicksMinutesAndSeconds() {
        #expect(LiveVoteClock.text(deadlineMS: 130_000, nowMS: 0) == "2:10")
        #expect(LiveVoteClock.text(deadlineMS: 42_000, nowMS: 0) == "0:42")
        #expect(LiveVoteClock.text(deadlineMS: 1_000, nowMS: 400) == "0:01")
        #expect(LiveVoteClock.text(deadlineMS: 0, nowMS: 5_000) == "0:00")
        #expect(LiveVoteClock.text(deadlineMS: 605_000, nowMS: 0) == "10:05")
    }

    @Test func settingsComeFromTheSettingsView() {
        #expect(LiveVoteSettings(values: [:]) == LiveVoteSettings())
        let read = LiveVoteSettings(values: ["show_polls": false, "show_predictions": true, "chat_overlay_order": "poll-first",
                                             "chat_design": ["polls_start_collapsed": true]])
        #expect(read == LiveVoteSettings(showPolls: false, showPredictions: true, pollFirst: true, pollsStartCollapsed: true))
    }

    // MARK: The stack (Rust decides it; these pin the shapes and the viewer's own state)

    @Test func decodesTheStackLayout() throws {
        let layout = try decode(StackLayout.self, #"""
        { "slots": [{ "kind": "poll", "open": true }, { "kind": "drop", "open": false }, { "kind": "banner", "open": false }],
          "max_height": 280.0 }
        """#)
        #expect(layout.slots.map(\.kind) == ["poll", "drop", "banner"])
        #expect(layout.slots.map(\.card) == [.poll, .drop, nil])
        #expect(layout.slots.map(\.open) == [true, false, false])
        #expect(layout.maxHeight == 280)
        #expect(try decode(StackLayout.self, #"{ "slots": [], "max_height": 0 }"#) == StackLayout(slots: [], maxHeight: 0))
    }

    /// The request as JSON, read back as a dictionary.
    private func encoded(_ request: StackRequest) throws -> [String: Any] {
        let args = try #require(try JSONSerialization.jsonObject(with: Data(request.argsJSON.utf8)) as? [String: Any])
        return try #require(args["request"] as? [String: Any])
    }

    @Test func encodesTheStackRequestAsRustReadsIt() throws {
        let viewer = StackViewer(opened: .drop, folded: [.poll, .prediction])
        let request = StackRequest(cards: [.poll, .drop], settings: LiveVoteSettings(pollFirst: true, pollsStartCollapsed: true),
                                   viewer: viewer, chatHeight: 380)
        let json = try encoded(request)
        #expect(json["cards"] as? [String] == ["poll", "drop"])
        #expect(json["order"] as? String == "poll-first")
        #expect(json["polls_start_collapsed"] as? Bool == true)
        #expect(json["opened"] as? String == "drop")
        // Folded in a fixed order, so the same state always asks the same question.
        #expect(json["folded"] as? [String] == ["prediction", "poll"])
        #expect((json["chat_height"] as? NSNumber)?.doubleValue == 380)

        let plain = try encoded(StackRequest(cards: [], settings: LiveVoteSettings(), viewer: StackViewer(), chatHeight: 0))
        #expect(plain["order"] as? String == "prediction-first")
        #expect(plain["polls_start_collapsed"] as? Bool == false)
        #expect(plain["opened"] == nil, "nothing opened is left out; Rust reads it as none")
        #expect(plain["folded"] as? [String] == [])
        #expect((plain["chat_height"] as? NSNumber)?.doubleValue == 0)
    }

    @Test func theViewerOpensOneCardAndFoldsStick() {
        var viewer = StackViewer()
        // Opening the drop strip beside an open poll folds the poll: one card is open at a time.
        viewer.tap(.drop, wasOpen: false, shown: [.poll, .drop])
        #expect(viewer.opened == .drop && viewer.folded == [.poll])
        // Folding the open card folds everything shown, so nothing opens by itself in its place.
        viewer.tap(.drop, wasOpen: true, shown: [.poll, .drop])
        #expect(viewer.opened == nil && viewer.folded == [.poll, .drop])
        viewer.tap(.poll, wasOpen: false, shown: [.poll, .drop])
        #expect(viewer.opened == .poll && viewer.folded == [.drop])
        // A new poll is a new card: the viewer's choice for the old one goes.
        viewer.forget(.poll)
        #expect(viewer.opened == nil && viewer.folded == [.drop])
    }

    @Test func aTapShowsItsLayoutAtOnce() {
        let layout = StackLayout(slots: [StackSlot(kind: "prediction", open: true), StackSlot(kind: "poll", open: false),
                                         StackSlot(kind: "drop", open: false)], maxHeight: 280)
        #expect(layout.showing(open: .drop).slots.map(\.open) == [false, false, true])
        #expect(layout.showing(open: nil).slots.map(\.open) == [false, false, false])
        #expect(layout.showing(open: nil).maxHeight == 280)
        #expect(StackLayout.strips([.poll, .drop]) == StackLayout(slots: [StackSlot(kind: "poll", open: false),
                                                                           StackSlot(kind: "drop", open: false)], maxHeight: 0))
    }

    /// The fake's layout for a request, as the slots' kinds, the open ones and the cap.
    private func fakeStack(_ cards: [String], _ height: Double, _ extra: [String: Any] = [:]) throws -> (kinds: [String], open: [String], max: Double) {
        var request: [String: Any] = ["cards": cards, "chat_height": height]
        request.merge(extra) { $1 }
        let layout = try decode(StackLayout.self, FakeCore.json(FakeLiveVotes.stackLayout(request)))
        return (layout.slots.map(\.kind), layout.slots.filter(\.open).map(\.kind), layout.maxHeight)
    }

    /// FakeCore stacks the cards the way chat_top_stack.rs does (its tests, case for case), so
    /// the fake chat stage shows what the real core would.
    @Test func theFakeStacksLikeRust() throws {
        var l = try fakeStack(["drop", "poll"], 700)
        #expect(l.kinds == ["poll", "drop"] && l.open == ["poll"] && l.max == 280)
        l = try fakeStack(["drop", "poll", "prediction"], 900)
        #expect(l.kinds == ["prediction", "poll", "drop"] && l.open == ["prediction"])
        #expect(try fakeStack(["drop", "poll", "prediction"], 900, ["order": "poll-first"]).open == ["poll"])
        l = try fakeStack(["drop", "poll"], 380)
        #expect(l.open.isEmpty && l.max == 152)
        #expect(try fakeStack(["drop", "poll"], 120).max == 76)
        #expect(try fakeStack(["drop", "poll"], 700, ["folded": ["poll"]]).open.isEmpty)
        #expect(try fakeStack(["drop", "poll"], 700, ["folded": ["poll"], "opened": "drop"]).open == ["drop"])
        #expect(try fakeStack(["drop", "poll"], 380, ["opened": "poll"]).open == ["poll"])
        #expect(try fakeStack(["poll"], 700, ["polls_start_collapsed": true]).open.isEmpty)
        #expect(try fakeStack(["poll", "prediction"], 700, ["order": "poll-first", "polls_start_collapsed": true]).open == ["prediction"])
        #expect(try fakeStack(["drop"], 700).open == ["drop"])
        #expect(try fakeStack(["drop"], 700, ["opened": "poll"]).open == ["drop"])
        l = try fakeStack(["banner", "poll"], 0)
        #expect(l.kinds == ["poll"] && l.open == ["poll"] && l.max == 0)
    }

    @Test func theFakeDropDecodes() throws {
        let ready = try decode(WatchDrop.self, FakeRewards.watchDropJSON(ready: true))
        #expect(ready.complete && ready.trailing.isEmpty && ready.claimLabel == "Claim" && ready.percent == 100)
        let earning = try decode(WatchDrop.self, FakeRewards.watchDropJSON(ready: false))
        #expect(!earning.complete && earning.trailing == "1h 48m to watch" && earning.percent == 40)
    }

    private struct NoAnswer: Error {}

    /// The fake's answer to a command it must know.
    private func ask(_ fake: FakeLiveVotes, _ cmd: String, _ args: [String: Any]) throws -> (json: String, push: String?) {
        guard let reply = try fake.answer(cmd, args) else { throw NoAnswer() }
        return (reply.json, reply.push?.payload)
    }

    private func view<T: Decodable>(_ type: T.Type, _ json: String?) throws -> T {
        guard let json else { throw NoAnswer() }
        return try decode(T.self, json)
    }

    @Test func theFakeAnswersEveryStateAndOnlyRecords() throws {
        let fake = FakeLiveVotes()
        #expect(try decode(PredictionPush.self, ask(fake, "get_channel_prediction", ["channelId": "4242", "channelLogin": "nook"]).json).view == nil)
        #expect(try decode(PollView?.self, ask(fake, "watch_channel_poll_view", ["channelId": "4242"]).json) == nil)

        let open = try decode(PollPush.self, fake.pollPush(channel: "4242", .open))
        let poll = try #require(open.view)
        #expect(poll.title == "Which boss next?" && poll.canVote && poll.deadlineMS != nil)
        #expect(poll.choices.map(\.title) == ["Malenia", "Radahn", "Mohg"])
        #expect(poll.choices.map(\.percent) == [58, 31, 11])
        #expect(poll.votersLine == "1,204 voters")
        let vote = try ask(fake, "vote_on_poll", ["pollId": poll.pollID, "choiceId": "b", "channelId": "4242"])
        #expect(fake.votes == ["fake-poll/b"])
        let votedPush = try view(PollPush.self, vote.push)
        let voted = try #require(votedPush.view)
        #expect(voted.myChoiceID == "b" && !voted.canVote && voted.footer == "You voted Radahn")
        #expect(throws: CoreFailure.self) { _ = try fake.answer("vote_on_poll", ["pollId": poll.pollID, "choiceId": "a"]) }
        let endedPush = try decode(PollPush.self, fake.pollPush(channel: "4242", .ended))
        let ended = try #require(endedPush.view)
        #expect(ended.phase == "ended" && ended.badge == "Final" && ended.resultLine == "Malenia won · 58%")
        #expect(ended.choices.first?.winner == true)
        #expect(try decode(PollPush.self, fake.pollPush(channel: "4242", .off)).view == nil)

        let openPush = try decode(PredictionPush.self, fake.predictionPush(channel: "4242", .open))
        let prediction = try #require(openPush.view)
        #expect(prediction.title == "Beats it first try?" && prediction.canPredict && prediction.picked == nil)
        #expect(prediction.outcomes.map(\.color) == ["blue", "pink"])
        #expect(prediction.outcomes.map(\.percent) == [36, 64])
        #expect(prediction.pointsName == "Nooks")
        let bet = try decode(BetView.self, ask(fake, "prediction_bet", ["channelId": "4242", "outcomeId": "yes", "amount": "100"]).json)
        #expect(bet.ok && bet.amount == 100 && bet.button == "Predict 100 on Yes")
        #expect(bet.presets.map(\.label) == ["10", "100", "1K", "Max"])
        #expect(bet.balanceLine == "12,850 Nooks")
        let clamped = try decode(BetView.self, ask(fake, "prediction_bet", ["channelId": "4242", "outcomeId": "no", "amount": "99999999"]).json)
        #expect(clamped.amount == 12_850)
        let predict = try ask(fake, "predict", ["channelId": "4242", "outcomeId": "yes", "amount": 100])
        let predicted = try decode(PredictOutcome.self, predict.json)
        #expect(predicted.success && predicted.message == "Predicted 100 on Yes")
        #expect(predicted.view?.picked == "yes" && predicted.view?.statusLine == "100 on Yes")
        #expect(try view(PredictionPush.self, predict.push).view?.picked == "yes")
        #expect(fake.predicts == ["yes:100"])
        let otherSide = try decode(BetView.self, ask(fake, "prediction_bet", ["channelId": "4242", "outcomeId": "no"]).json)
        #expect(!otherSide.ok && otherSide.problem == "You already picked Yes")
        let topUp = try decode(BetView.self, ask(fake, "prediction_bet", ["channelId": "4242", "outcomeId": "yes"]).json)
        #expect(topUp.button == "Add 10 to Yes" && topUp.confirmNote == nil)

        let lockedPush = try decode(PredictionPush.self, fake.predictionPush(channel: "4242", .locked))
        let locked = try #require(lockedPush.view)
        #expect(locked.phase == "locked" && locked.badge == "Closed" && !locked.canPredict && locked.deadlineMS == nil)
        #expect(locked.statusLine == "100 on Yes · waiting for the result")
        let wonPush = try decode(PredictionPush.self, fake.predictionPush(channel: "4242", .won))
        let won = try #require(wonPush.view)
        #expect(won.result?.tone == "won" && won.result?.line.hasPrefix("Yes won · you won") == true)
        #expect(won.outcomes.first?.winner == true)
        let refundedPush = try decode(PredictionPush.self, fake.predictionPush(channel: "4242", .refunded))
        let refunded = try #require(refundedPush.view)
        #expect(refunded.result == ResultView(tone: "refunded", line: "Canceled · points refunded"))
        #expect(try decode(PredictionPush.self, fake.predictionPush(channel: "4242", .off)).view == nil)
        let closed = try decode(PredictOutcome.self, ask(fake, "predict", ["channelId": "4242", "outcomeId": "yes", "amount": 10]).json)
        #expect(!closed.success && closed.message == "Predictions are closed")
    }

    @Test func fakeCoreAnswersTheCommandsWithoutFixtures() async throws {
        let core = FakeCore()
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "t",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        _ = core.liveVotes.predictionPush(channel: "4242", .open)
        let push = try await core.invoke("get_channel_prediction", args: ["channelId": "4242", "channelLogin": "nook"],
                                         as: PredictionPush.self)
        #expect(push.view?.predictionID == "fake-prediction")
        let poll = try await core.invoke("watch_channel_poll_view", args: ["channelId": "4242"], as: PollView?.self)
        #expect(poll == nil)
        let request = StackRequest(cards: [.drop, .prediction], settings: LiveVoteSettings(), viewer: StackViewer(), chatHeight: 700)
        let json = try await core.invoke("chat_top_stack", request.argsJSON)
        let layout = try JSONDecoder().decode(StackLayout.self, from: Data(json.utf8))
        #expect(layout == StackLayout(slots: [StackSlot(kind: "prediction", open: true), StackSlot(kind: "drop", open: false)],
                                      maxHeight: 280))
    }
}
