import SNKit
import SwiftUI

/// A live poll. Open: the question on its own line, then each choice as its title and share over
/// a thin bar, then who voted. One tap votes (it spends nothing, so there is no confirm); the
/// mark shows at once and comes back off if Twitch refuses. As a strip it is one line: the
/// question, or the result once ended.
struct PollCard: View {
    let model: LiveVotesModel
    let poll: PollView
    let open: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            CardHead(open: open, label: "Poll", line: poll.resultLine ?? poll.title,
                     action: { model.tap(.poll) }) {
                VoteKindMark(card: .poll)
            } status: {
                VoteTimeSlot(deadlineMS: poll.deadlineMS, badge: poll.badge)
            }
            CardBody(open: open) {
                VStack(alignment: .leading, spacing: 12) {
                    VoteQuestion(text: poll.title)
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(poll.choices) { choice in
                            choiceRow(choice)
                        }
                    }
                    Text([poll.votersLine, poll.footer].filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(.sn(12))
                        .foregroundStyle(SN.textMuted)
                        .lineLimit(1)
                }
            }
        }
        .modifier(VoteCardSurface())
    }

    /// The title line (title and marks left, share right), then the bar under it: text never
    /// sits on the fill, and the fill is the floored accent at full strength over the neutral
    /// track, so it reads in every theme.
    private func choiceRow(_ choice: ChoiceView) -> some View {
        let mine = model.isMine(choice)
        let strong = mine || choice.winner
        return Button {
            Haptics.tick()
            model.vote(choice)
        } label: {
            VStack(alignment: .leading, spacing: 5) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text(choice.title)
                        .font(.sn(14, strong ? 600 : 400))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    if mine {
                        Image(systemName: Symbol.check)
                            .font(.system(size: 12, weight: .semibold))
                            .foregroundStyle(SN.accentOnSurface)
                            .accessibilityHidden(true)
                    }
                    if choice.winner {
                        Image(systemName: "crown.fill")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(SN.accentOnSurface)
                            .accessibilityHidden(true)
                    }
                    Spacer(minLength: 8)
                    Text("\(choice.percent)%")
                        .font(.sn(13, strong ? 600 : 500))
                        .monospacedDigit()
                        .foregroundStyle(strong ? SN.textPrimary : SN.textSecondary)
                        .contentTransition(.numericText())
                }
                ProgressTrack(percent: Double(choice.percent), height: 4, color: SN.accentOnSurface)
            }
            // A taller target than the drawn row, without moving the rows apart.
            .padding(.vertical, 4)
            .contentShape(Rectangle())
            .animation(SNMotion.run(.easeOut(duration: 0.15)), value: mine)
        }
        .buttonStyle(.plain)
        // Not `.disabled`: that grays every row once you have voted, and your mark is the row
        // that must read clearly then. `vote` refuses on its own when voting is not offered.
        .allowsHitTesting(model.canVote)
        .accessibilityLabel("\(choice.title), \(choice.percent) percent, \(choice.votesLabel) votes")
        .accessibilityAddTraits(mine ? .isSelected : [])
    }
}
