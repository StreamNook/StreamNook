import SNKit
import SwiftUI

/// The top of chat, under the pinned banner: the room's poll and prediction and the watched
/// stream's drop, as Rust lays them out (`chat_top_stack`). At most one card is open; the rest
/// are one-line strips, and a tap on any card's head opens or folds it. The whole stack keeps
/// to Rust's height cap, and an open card's body scrolls inside it when it does not fit.
struct LiveVotesStack: View {
    let model: LiveVotesModel
    /// The watched stream's drop, nil where there is none to show (the chat stage, other rooms).
    let drop: WatchDropModel?

    var body: some View {
        let slots = model.slots
        VStack(spacing: 6) {
            ForEach(slots) { slot in
                card(slot)
                    .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .frame(maxHeight: model.maxHeight > 0 ? model.maxHeight : nil, alignment: .top)
        .animation(SNMotion.run(LiveVoteLook.fold), value: slots)
    }

    @ViewBuilder
    private func card(_ slot: StackSlot) -> some View {
        switch slot.card {
        case .poll:
            if let poll = model.poll { PollCard(model: model, poll: poll, open: slot.open) }
        case .prediction:
            if let prediction = model.prediction { PredictionCard(model: model, prediction: prediction, open: slot.open) }
        case .drop:
            if let drop, let shown = drop.shown { DropCard(model: model, drops: drop, drop: shown, open: slot.open) }
        case nil:
            EmptyView()
        }
    }
}

/// What the cards share: the fold motion and the prediction side colors.
enum LiveVoteLook {
    /// Folding and opening, the pinned banner's curve.
    static let fold = Animation.timingCurve(0.16, 1, 0.3, 1, duration: 0.22)
    /// Twitch's two prediction sides.
    static let blue = RGBA(hex: 0x3B82F6).color
    static let pink = RGBA(hex: 0xF43F8E).color

    /// A side's color as Twitch names it: blue and pink, any CSS color Rust passes on, else the
    /// theme accent as it reads on a card.
    @MainActor
    static func color(_ name: String) -> Color {
        switch name.lowercased() {
        case "blue": blue
        case "pink": pink
        default: RGBA(css: name)?.color ?? SN.accentOnSurface
        }
    }
}

/// The small mark of a card's kind: the accent square for a poll, the two sides split corner to
/// corner for a prediction, the gift for a drop.
struct VoteKindMark: View {
    let card: TopCard

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 3.5, style: .continuous)
        Group {
            switch card {
            case .poll:
                shape.fill(SN.accentOnSurface)
            case .prediction:
                shape.fill(LinearGradient(stops: [
                    .init(color: LiveVoteLook.blue, location: 0), .init(color: LiveVoteLook.blue, location: 0.5),
                    .init(color: LiveVoteLook.pink, location: 0.5), .init(color: LiveVoteLook.pink, location: 1),
                ], startPoint: .topLeading, endPoint: .bottomTrailing))
            case .drop:
                Image(systemName: Symbol.gift)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(SN.accentOnSurface)
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
                    .font(.sn(12, 600))
                    .monospacedDigit()
                    .foregroundStyle(SN.textSecondary)
            }
        } else if let badge {
            Text(badge)
                .font(.sn(12, 500))
                .foregroundStyle(SN.textMuted)
                .lineLimit(1)
        }
    }
}

/// A card's head, the one row every card always has, strip or open: its mark, its kind's label,
/// the one line it stands for (a strip's; an open vote card shows its question below instead),
/// the status and the chevron. The whole row, its padding included, folds or opens the card, so
/// a strip is one 34 point target and two strips fit Rust's smallest cap.
struct CardHead<Mark: View, Status: View>: View {
    let open: Bool
    /// "Poll", "Prediction", "Drop": spoken always, shown uppercase when `showsLabel`.
    let label: String
    var showsLabel = true
    let line: String
    /// The line shows while open too (the drop's name, which the open card does not repeat).
    var lineWhenOpen = false
    let action: () -> Void
    @ViewBuilder let mark: Mark
    @ViewBuilder let status: Status

    var body: some View {
        Button {
            withAnimation(SNMotion.run(LiveVoteLook.fold)) { action() }
        } label: {
            HStack(spacing: 6) {
                mark
                if showsLabel {
                    Text(label)
                        .font(.sn(11, 600))
                        .textCase(.uppercase)
                        .tracking(0.6)
                        .foregroundStyle(SN.textSecondary)
                        .fixedSize()
                }
                if !open || lineWhenOpen {
                    Text(line)
                        .font(.sn(13, 600))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .padding(.leading, showsLabel ? 2 : 0)
                        .transition(.opacity)
                }
                Spacer(minLength: 8)
                status
                    .fixedSize()
                Image(systemName: "chevron.down")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(SN.textMuted)
                    .rotationEffect(.degrees(open ? 180 : 0))
                    .accessibilityHidden(true)
            }
            .frame(height: 18)
            .padding(.vertical, 8)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityLabel("\(label): \(line)")
        .accessibilityHint(open ? "Folds the \(label.lowercased())" : "Opens the \(label.lowercased())")
    }
}

/// An open card's body under its head. It stays laid out while the card is a strip (at no
/// height), so opening grows it from its measured height in one motion, and inside Rust's cap
/// it scrolls, fading at the edge that hides more.
struct CardBody<Content: View>: View {
    let open: Bool
    @ViewBuilder let content: Content
    @State private var height: CGFloat = 0
    @State private var edges = HiddenEdges()

    var body: some View {
        ScrollView(.vertical) {
            content
                .padding(.bottom, 12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
        }
        .scrollBounceBehavior(.basedOnSize)
        .onScrollGeometryChange(for: HiddenEdges.self) { geo in
            HiddenEdges(top: geo.contentOffset.y > 1,
                        bottom: geo.contentOffset.y + geo.containerSize.height < geo.contentSize.height - 1)
        } action: { _, next in
            edges = next
        }
        .mask {
            VStack(spacing: 0) {
                LinearGradient(colors: [edges.top ? .clear : .black, .black], startPoint: .top, endPoint: .bottom)
                    .frame(height: 12)
                Color.black
                LinearGradient(colors: [.black, edges.bottom ? .clear : .black], startPoint: .top, endPoint: .bottom)
                    .frame(height: 16)
            }
        }
        .frame(minHeight: 0, maxHeight: open ? height : 0, alignment: .top)
        .opacity(open ? 1 : 0)
        .allowsHitTesting(open)
        .accessibilityHidden(!open)
        // A card that opens before its body was measured grows into place instead of snapping.
        .animation(SNMotion.run(LiveVoteLook.fold), value: height)
    }

    struct HiddenEdges: Equatable {
        var top = false
        var bottom = false
    }
}

/// The surface every card and strip sits on. The head pads itself (it is the tap target), so
/// the surface pads only the sides.
struct VoteCardSurface: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .clipped()
            .modifier(StaticPopover(tint: 0.96))
            .background(SN.background, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
    }
}

/// A poll's or prediction's question on its open card: its own line under the head, two lines at
/// most, Twitch's stray line breaks and doubled spaces read as single spaces.
struct VoteQuestion: View {
    let text: String

    var body: some View {
        Text(text.split(whereSeparator: \.isWhitespace).joined(separator: " "))
            .font(.sn(15, 600))
            .foregroundStyle(SN.textPrimary)
            .lineSpacing(2)
            .lineLimit(2)
            .truncationMode(.tail)
            .multilineTextAlignment(.leading)
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .leading)
            .accessibilityAddTraits(.isHeader)
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
