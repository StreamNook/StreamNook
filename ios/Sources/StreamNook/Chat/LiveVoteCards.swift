import SNKit
import SwiftUI

/// The room's poll and prediction cards at the top of chat, under the pinned banner, in the
/// order the Top card setting picks. A folded card is a one-line strip; a tap on a card's head
/// folds or opens it.
struct LiveVotesStack: View {
    let model: LiveVotesModel

    var body: some View {
        let slots = model.slots
        VStack(spacing: 6) {
            ForEach(slots) { slot in
                Group {
                    switch slot.kind {
                    case .poll:
                        if let poll = model.poll { PollCard(model: model, poll: poll, slot: slot) }
                    case .prediction:
                        if let prediction = model.prediction { PredictionCard(model: model, prediction: prediction, slot: slot) }
                    }
                }
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .animation(SNMotion.run(LiveVoteLook.fold), value: slots)
    }
}

/// What the two cards share: the fold motion, the side colors, the head and the strip.
enum LiveVoteLook {
    /// Folding and opening, the pinned banner's curve.
    static let fold = Animation.timingCurve(0.16, 1, 0.3, 1, duration: 0.22)
    /// Twitch's two prediction sides.
    static let blue = RGBA(hex: 0x3B82F6).color
    static let pink = RGBA(hex: 0xF43F8E).color

    /// A side's color as Twitch names it: blue and pink, any CSS color Rust passes on, else the
    /// theme accent.
    @MainActor
    static func color(_ name: String) -> Color {
        switch name.lowercased() {
        case "blue": blue
        case "pink": pink
        default: RGBA(css: name)?.color ?? SN.accent
        }
    }
}

/// The small square that marks a card's kind: the accent for a poll, the two sides split
/// corner to corner for a prediction.
struct VoteKindMark: View {
    let kind: LiveVoteKind

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 3.5, style: .continuous)
        Group {
            switch kind {
            case .poll:
                shape.fill(SN.accent)
            case .prediction:
                shape.fill(LinearGradient(stops: [
                    .init(color: LiveVoteLook.blue, location: 0), .init(color: LiveVoteLook.blue, location: 0.5),
                    .init(color: LiveVoteLook.pink, location: 0.5), .init(color: LiveVoteLook.pink, location: 1),
                ], startPoint: .topLeading, endPoint: .bottomTrailing))
            }
        }
        .frame(width: 12, height: 12)
        .accessibilityHidden(true)
    }
}

/// The time slot: a countdown ticking to Rust's deadline while open, else Rust's badge.
struct VoteTimeSlot: View {
    let deadlineMS: Int64?
    let badge: String?

    var body: some View {
        if let deadlineMS {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                Text(LiveVoteClock.text(deadlineMS: deadlineMS, nowMS: LiveVoteClock.nowMS(context.date)))
                    .font(.sn(11.5, 600))
                    .monospacedDigit()
                    .foregroundStyle(SN.textSecondary)
            }
        } else if let badge {
            Text(badge)
                .font(.sn(11.5, 500))
                .foregroundStyle(SN.textMuted)
                .lineLimit(1)
        }
    }
}

/// A card's head: its kind, then (folded) the one line it stands for, the time slot and the
/// chevron. The whole row folds or opens the card.
struct VoteHead: View {
    let kind: LiveVoteKind
    let slot: LiveVoteSlot
    /// The line a folded card shows: the title, or the result once there is one.
    let line: String
    let deadlineMS: Int64?
    let badge: String?
    let model: LiveVotesModel

    private var label: String { kind == .poll ? "Poll" : "Prediction" }

    var body: some View {
        Button {
            withAnimation(SNMotion.run(LiveVoteLook.fold)) { model.toggle(slot) }
        } label: {
            HStack(spacing: 6) {
                VoteKindMark(kind: kind)
                Text(label)
                    .font(.sn(10.5, 700))
                    .textCase(.uppercase)
                    .tracking(0.5)
                    .foregroundStyle(SN.textSecondary)
                    .fixedSize()
                if slot.folded {
                    Text(line)
                        .font(.sn(12.5, 600))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                        .padding(.leading, 2)
                }
                Spacer(minLength: 6)
                VoteTimeSlot(deadlineMS: deadlineMS, badge: badge)
                    .fixedSize()
                Image(systemName: slot.folded ? "chevron.down" : "chevron.up")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(SN.textMuted)
                    .accessibilityHidden(true)
            }
            .frame(minHeight: 18)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(label): \(line)")
        .accessibilityHint(slot.folded ? "Opens the \(label.lowercased())" : "Folds the \(label.lowercased())")
    }
}

/// The surface every card and strip sits on.
struct VoteCardSurface: ViewModifier {
    let folded: Bool

    func body(content: Content) -> some View {
        content
            .padding(.horizontal, 12)
            .padding(.vertical, folded ? 9 : 11)
            .frame(maxWidth: .infinity, alignment: .leading)
            .clipped()
            .modifier(StaticPopover(tint: 0.96))
            .background(SN.background, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

/// A one-line line of Rust's words on a soft tile: the stake, the result.
struct VoteLinePill: View {
    let text: String
    var tint: Color?
    var icon: String?
    var showsIcon = false

    var body: some View {
        HStack(spacing: 8) {
            if showsIcon { PointsArt(iconURL: icon, size: 15) }
            Text(text)
                .font(.sn(12.5, tint == nil ? 500 : 600))
                .foregroundStyle(tint ?? SN.textPrimary.opacity(0.9))
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background {
            RoundedRectangle(cornerRadius: 8, style: .continuous).fill((tint ?? SN.textSecondary).opacity(tint == nil ? 0.1 : 0.14))
        }
    }
}
