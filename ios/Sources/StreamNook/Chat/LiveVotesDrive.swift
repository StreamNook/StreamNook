#if DEBUG
import Foundation
import SNKit
import SwiftUI

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
/// - `dropfake:on|ready|off` shows FakeCore's drop in the stack (40 percent of the way, or ready
///   to claim), or takes it away; on the chat stage, else over the watch layer's room. A claim
///   on it is only logged
/// - `votesfold:poll|prediction|drop` folds or opens that card as a tap on its head does
/// - `chatheight:POINTS` asks Rust's layout for a chat column that tall (the short-column rule
///   below 460, the 40 percent cap); `chatheight:off` goes back to the measured height
/// - `liveVotesLog` logs the cards, Rust's layout and what it was asked, the bet and the settings
@MainActor
enum LiveVotesDrive {
    static func run(_ name: String, _ value: String, model app: AppModel) async -> Bool {
        let steps = ["pollfake", "predfake", "dropfake", "predbet", "predconfirm", "predpress", "pollvote", "votesfold",
                     "chatheight", "liveVotesLog"]
        guard steps.contains(name) else { return false }
        guard let session = ChatDrive.currentSession else {
            log.error("drive \(name, privacy: .public): no chat on screen")
            return true
        }
        let model = LiveVotesRegistry.shared.model(for: session)
        let fake = session.core as? FakeCore
        // Anything that votes, spends or fakes a push runs only against FakeCore.
        if ["pollfake", "predfake", "dropfake", "predconfirm", "predpress", "pollvote"].contains(name), fake == nil {
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
        case "dropfake":
            let drops = ChatLab.session == nil ? WatchDropModel.watched : ChatLab.drop
            guard value != "off" else {
                drops.set(nil)
                break
            }
            let json = FakeRewards.watchDropJSON(ready: value == "ready")
            guard let drop = try? JSONDecoder().decode(WatchDrop.self, from: Data(json.utf8)) else { break }
            if drops === ChatLab.drop {
                // The stage has no account: a claim moves on to the next drop, as the fake core does.
                drops.claimer = { claimed in
                    log.notice("drive dropfake claim \(claimed.dropID, privacy: .public)")
                    try? await Task.sleep(for: .milliseconds(300))
                    drops.set(try? JSONDecoder().decode(WatchDrop.self,
                                                        from: Data(FakeRewards.watchDropJSON(ready: false).utf8)))
                }
                drops.opener = { shown in log.notice("drive dropfake open campaign \(shown.campaignID, privacy: .public)") }
            }
            drops.visible = true
            drops.set(drop)
        case "chatheight":
            model.forcedHeight = value == "off" ? nil : Double(value)
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
            if let card = TopCard(rawValue: value), model.slots.contains(where: { $0.card == card }) {
                withAnimation(SNMotion.run(LiveVoteLook.fold)) { model.tap(card) }
            }
        default:
            let slots = model.slots.map { "\($0.kind)=\($0.open ? "open" : "strip")" }.joined(separator: ",")
            let asked = model.stackRequest
            let stack = "slots \(slots) max \(Int(model.maxHeight)) asked cards \(asked.cards.map(\.rawValue).joined(separator: ",")) opened \(asked.opened?.rawValue ?? "-") folded \(asked.folded.map(\.rawValue).joined(separator: ",")) height \(Int(asked.chatHeight))\(model.forcedHeight == nil ? "" : " (forced)") drop \(model.dropID ?? "-")"
            log.notice("drive livevotes stack \(stack, privacy: .public)")
            let poll = model.poll.map { "\($0.pollID) \($0.phase) canVote \($0.canVote) mine \($0.myChoiceID ?? "-") pending \(model.pendingChoice ?? "-") footer \($0.footer)" } ?? "none"
            let prediction = model.prediction.map { "\($0.predictionID) \($0.phase) canPredict \($0.canPredict) picked \($0.picked ?? "-") status \($0.statusLine ?? "-") result \($0.result?.line ?? "-")" } ?? "none"
            let bet = model.bet.map { "amount \($0.amount) ok \($0.ok) button \($0.button) problem \($0.problem ?? "-") win \($0.winLine)" } ?? "none"
            let s = model.settings
            log.notice("drive livevotes room \(session.roomID ?? "-", privacy: .public) poll \(poll, privacy: .public) prediction \(prediction, privacy: .public) side \(model.side ?? "-", privacy: .public) typed \(model.typed, privacy: .public) preset \(model.chosenPreset ?? -1) bet \(bet, privacy: .public) confirm \(model.confirmOpen) settings polls \(s.showPolls) predictions \(s.showPredictions) pollFirst \(s.pollFirst) collapsed \(s.pollsStartCollapsed)")
            if let fake {
                log.notice("drive livevotes fake votes \(fake.liveVotes.votes.joined(separator: ","), privacy: .public) predicts \(fake.liveVotes.predicts.joined(separator: ","), privacy: .public)")
            }
        }
        return true
    }
}
#endif
