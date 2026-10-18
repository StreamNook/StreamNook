import SNKit
import SwiftUI

/// A live poll: the question, a row per choice with its share drawn as a fill, then who voted.
/// One tap votes (it spends nothing, so there is no confirm); the mark shows at once and comes
/// back off if Twitch refuses. Folded, it is one line: the question, or the result once ended.
struct PollCard: View {
    let model: LiveVotesModel
    let poll: PollView
    let slot: LiveVoteSlot

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VoteHead(kind: .poll, slot: slot, line: poll.resultLine ?? poll.title, deadlineMS: poll.deadlineMS,
                     badge: poll.badge, model: model)
            if !slot.folded {
                VStack(alignment: .leading, spacing: 8) {
                    Text(poll.title)
                        .font(.sn(15, 600))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                    VStack(spacing: 6) {
                        ForEach(poll.choices) { choice in
                            choiceRow(choice)
                        }
                    }
                    Text([poll.votersLine, poll.footer].filter { !$0.isEmpty }.joined(separator: " · "))
                        .font(.sn(11.5))
                        .foregroundStyle(SN.textMuted)
                        .lineLimit(1)
                }
                .padding(.top, 8)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .modifier(VoteCardSurface(folded: slot.folded))
    }

    private func choiceRow(_ choice: ChoiceView) -> some View {
        let mine = model.isMine(choice)
        let strong = mine || choice.winner
        let shape = RoundedRectangle(cornerRadius: 8, style: .continuous)
        return Button {
            Haptics.tick()
            model.vote(choice)
        } label: {
            HStack(spacing: 6) {
                Text(choice.title)
                    .font(.sn(13, strong ? 600 : 500))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                Spacer(minLength: 8)
                Text("\(choice.percent)%")
                    .font(.sn(12.5, 600))
                    .monospacedDigit()
                    .foregroundStyle(strong ? SN.textPrimary : SN.textSecondary)
                if mine || choice.winner {
                    Image(systemName: choice.winner && !mine ? "crown.fill" : Symbol.check)
                        .font(.system(size: 10, weight: .bold))
                        .foregroundStyle(SN.accent)
                        .accessibilityHidden(true)
                }
            }
            .padding(.horizontal, 12)
            .frame(height: 36)
            .background {
                shape.fill(SN.surface)
                    .overlay(alignment: .leading) {
                        GeometryReader { geo in
                            Rectangle()
                                .fill(SN.accent.opacity(strong ? 0.42 : 0.2))
                                .frame(width: geo.size.width * CGFloat(min(max(choice.percent, 0), 100)) / 100)
                        }
                    }
                    .clipShape(shape)
            }
            .overlay { if mine { shape.stroke(SN.accent, lineWidth: 1.5) } }
            .contentShape(shape)
            .animation(SNMotion.run(.easeOut(duration: 0.3)), value: choice.percent)
            .animation(SNMotion.run(.easeOut(duration: 0.15)), value: mine)
        }
        .buttonStyle(.plain)
        .disabled(!model.canVote)
        .accessibilityLabel("\(choice.title), \(choice.percent) percent, \(choice.votesLabel) votes")
        .accessibilityAddTraits(mine ? .isSelected : [])
    }
}
