#if DEBUG
import Foundation
import SNKit

/// Poll and prediction drive steps (see Drive.swift), on the chat stage or the watch layer's
/// room. The fake steps run only on FakeCore and emit its pushes for the room on screen, so no
/// real poll or prediction is ever touched:
///
/// - `pollfake:open|voted|ended|off` puts FakeCore's poll ("Which boss next?") in that state
/// - `predfake:open|picked|locked|won|refunded|off` puts FakeCore's prediction ("Beats it first
///   try?") in that state (`picked` and later states carry a 100 point stake on Yes)
/// - `predbet:SIDE:AMOUNT` picks a side (its id, title or index) and types AMOUNT, as the viewer
///   would; `predbet:SIDE` only picks the side, `predbet:SIDE:pINDEX` taps the preset at INDEX
/// - `predconfirm` opens the confirm sheet (FakeCore only); `predpress` presses its Predict
///   (FakeCore only, so points are never spent)
/// - `pollvote:INDEX` taps the poll's choice at INDEX (FakeCore only)
/// - `votesfold:poll|prediction` folds or opens that card as a tap on its head does
/// - `liveVotesLog` logs both cards, the folds, the bet and the settings they read
@MainActor
enum LiveVotesDrive {
    static func run(_ name: String, _ value: String, model app: AppModel) async -> Bool {
        let steps = ["pollfake", "predfake", "predbet", "predconfirm", "predpress", "pollvote", "votesfold", "liveVotesLog"]
        guard steps.contains(name) else { return false }
        guard let session = ChatDrive.currentSession else {
            log.error("drive \(name, privacy: .public): no chat on screen")
            return true
        }
        let model = LiveVotesRegistry.shared.model(for: session)
        let fake = session.core as? FakeCore
        // Anything that votes, spends or fakes a push runs only against FakeCore.
        if ["pollfake", "predfake", "predconfirm", "predpress", "pollvote"].contains(name), fake == nil {
            log.error("drive \(name, privacy: .public) refused on a real core")
            return true
        }
        let room = session.roomID ?? "4242"
        switch name {
        case "pollfake":
            guard let fake, let state = FakeLiveVotes.PollState(rawValue: value.isEmpty ? "open" : value) else { break }
            if session.roomID == nil { session.roomID = room }
            fake.events.emit(name: "poll-state", payloadJSON: fake.liveVotes.pollPush(channel: room, state), target: nil)
        case "predfake":
            guard let fake, let state = FakeLiveVotes.PredictionState(rawValue: value.isEmpty ? "open" : value) else { break }
            if session.roomID == nil { session.roomID = room }
            fake.events.emit(name: "prediction-state", payloadJSON: fake.liveVotes.predictionPush(channel: room, state),
                             target: nil)
        case "predbet":
            let parts = value.split(separator: ":", maxSplits: 1).map(String.init)
            guard let prediction = model.prediction, let side = parts.first else { break }
            let outcome = prediction.outcomes.first { $0.id == side || $0.title.lowercased() == side.lowercased() }
                ?? Int(side).flatMap { prediction.outcomes.indices.contains($0) ? prediction.outcomes[$0] : nil }
            if let outcome { model.pick(outcome.id) }
            if parts.count > 1 {
                let amount = parts[1]
                if amount.hasPrefix("p"), let index = Int(amount.dropFirst()) {
                    // A preset needs Rust's presets first.
                    try? await Task.sleep(for: .milliseconds(400))
                    model.choosePreset(index)
                } else {
                    model.type(amount)
                }
            }
            try? await Task.sleep(for: .milliseconds(500))
        case "predconfirm":
            try? await Task.sleep(for: .milliseconds(300))
            model.openConfirm()
        case "predpress":
            await model.confirmPredict()
        case "pollvote":
            guard let poll = model.poll, let index = Int(value), poll.choices.indices.contains(index) else { break }
            model.vote(poll.choices[index])
        case "votesfold":
            if let slot = model.slots.first(where: { $0.kind.rawValue == value }) { model.toggle(slot) }
        default:
            let slots = model.slots.map { "\($0.key)=\($0.folded ? "folded" : "open")" }.joined(separator: ",")
            let poll = model.poll.map { "\($0.pollID) \($0.phase) canVote \($0.canVote) mine \($0.myChoiceID ?? "-") pending \(model.pendingChoice ?? "-") footer \($0.footer)" } ?? "none"
            let prediction = model.prediction.map { "\($0.predictionID) \($0.phase) canPredict \($0.canPredict) picked \($0.picked ?? "-") status \($0.statusLine ?? "-") result \($0.result?.line ?? "-")" } ?? "none"
            let bet = model.bet.map { "amount \($0.amount) ok \($0.ok) button \($0.button) problem \($0.problem ?? "-") win \($0.winLine)" } ?? "none"
            let s = model.settings
            log.notice("drive livevotes room \(session.roomID ?? "-", privacy: .public) slots \(slots, privacy: .public) poll \(poll, privacy: .public) prediction \(prediction, privacy: .public) side \(model.side ?? "-", privacy: .public) typed \(model.typed, privacy: .public) preset \(model.chosenPreset ?? -1) bet \(bet, privacy: .public) confirm \(model.confirmOpen) settings polls \(s.showPolls) predictions \(s.showPredictions) pollFirst \(s.pollFirst) collapsed \(s.pollsStartCollapsed)")
            if let fake {
                log.notice("drive livevotes fake votes \(fake.liveVotes.votes.joined(separator: ","), privacy: .public) predicts \(fake.liveVotes.predicts.joined(separator: ","), privacy: .public)")
            }
        }
        return true
    }
}
#endif
