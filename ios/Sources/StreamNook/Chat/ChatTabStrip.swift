import SNKit
import SwiftUI

/// The open rooms over the chat, shown from two rooms up under the player so single-room chat
/// keeps every point of height, and always on the Chats tab, where its controls sit at the right.
/// A 45 point bar: the rooms in one glazed capsule, the room on screen a darker pill. The capsule
/// and whatever sits beside it hug their content and sit centered in the bar as one group. When
/// they outgrow the bar the row compresses in place and never scrolls, clips or wraps: rooms behind
/// the one on screen fold to their faces one at a time, the longest name first, until the row
/// fits; then the folded faces shrink and draw closer; only then does the name on screen truncate.
/// It reads only the tabs, never the rows, so arriving messages never rebuild it.
struct ChatTabStrip: View {
    let rooms: ChatRooms
    /// Controls after the rooms, outside their glass (the Chats tab's play and add buttons).
    var trailing: AnyView?
    /// The bar's height. The Chats header is taller, so its 40 pt capsules get air above the
    /// separator instead of sitting on it.
    var height: CGFloat = ChatTabStrip.height
    /// What sits in the left side, opposite the controls (the Chats tab's Shared Chat or Together
    /// group for the room on screen).
    var leading: AnyView?
    /// What sits at the row's very left edge, outside the centered group (the Chats tab's
    /// minimize arrow).
    var edge: AnyView?

    static let height: CGFloat = 45

    var body: some View {
        // The group (the room's group chip, the rooms, their controls) sits centered as one, so a
        // single room never reads lopsided; past the width the rooms give way first.
        HStack(spacing: 6) {
            if let edge { edge }
            Spacer(minLength: 0)
            if let leading { leading }
            ViewThatFits(in: .horizontal) {
                ForEach(0..<max(1, rooms.tabs.count), id: \.self) { folds in
                    row(folding: folds)
                }
                row(folding: max(0, rooms.tabs.count - 1), tight: true)
            }
            .padding(4)
            .snGlass(Capsule())
            .layoutPriority(1)
            if let trailing { trailing.fixedSize() }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .frame(height: height)
        .overlay(alignment: .bottom) {
            // Quiet: the capsule's own rim already sits just above it.
            SN.borderSubtle.opacity(0.6).frame(height: 1).accessibilityHidden(true)
        }
    }

    /// The row with `folds` rooms folded: those not on screen, longest name first. Tight, the
    /// folded faces are smaller and closer, so the name on screen keeps its letters.
    private func row(folding folds: Int, tight: Bool = false) -> some View {
        let active = rooms.state.activeChannel
        let order = rooms.tabs.filter { $0.channel != active }.sorted { $0.label.count > $1.label.count }
        let folded = Set(order.prefix(folds).map(\.channel))
        return TabRow(spacing: tight ? 2 : 4) {
            ForEach(rooms.tabs) { tab in
                TabPill(tab: tab, active: tab.channel == active, folded: folded.contains(tab.channel),
                        crowded: folds > 0, tight: tight,
                        mark: rooms.state.mixedPlatforms && tab.provider != "twitch",
                        select: { rooms.select(tab.channel) }, close: { rooms.close(tab.channel) })
            }
        }
    }
}

/// Tabs side by side at their natural widths while they fit. Past the width it is offered, the
/// widest tabs give up width first (each down to a shared cap), so every tab stays inside the row.
struct TabRow: Layout {
    var spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let widths = Self.widths(subviews, available: proposal.width, spacing: spacing)
        let height = subviews.map { $0.sizeThatFits(.unspecified).height }.max() ?? 0
        return CGSize(width: widths.reduce(0, +) + spacing * CGFloat(max(0, subviews.count - 1)), height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let widths = Self.widths(subviews, available: bounds.width, spacing: spacing)
        var x = bounds.minX
        for (subview, width) in zip(subviews, widths) {
            subview.place(at: CGPoint(x: x, y: bounds.midY), anchor: .leading,
                          proposal: ProposedViewSize(width: width, height: bounds.height))
            x += width + spacing
        }
    }

    /// Natural widths, or, when they overflow, each capped at the level where they just fit and
    /// then measured again, so a truncated tab keeps no empty room after its "…".
    static func widths(_ subviews: Subviews, available: CGFloat?, spacing: CGFloat) -> [CGFloat] {
        let natural = subviews.map { $0.sizeThatFits(.unspecified).width }
        guard let available else { return natural }
        let room = available - spacing * CGFloat(max(0, subviews.count - 1))
        let capped = capped(natural, toFit: room)
        guard capped != natural else { return natural }
        return zip(subviews, capped).map { subview, width in
            min(width, subview.sizeThatFits(ProposedViewSize(width: width, height: 32)).width)
        }
    }

    /// The widths after capping the widest at one shared level so the total fits `room`.
    static func capped(_ natural: [CGFloat], toFit room: CGFloat) -> [CGFloat] {
        guard natural.reduce(0, +) > room, !natural.isEmpty else { return natural }
        let sorted = natural.sorted()
        var used: CGFloat = 0
        for (i, width) in sorted.enumerated() {
            let left = CGFloat(sorted.count - i)
            let cap = (room - used) / left
            if width >= cap { return natural.map { min($0, max(0, cap)) } }
            used += width
        }
        return natural
    }
}

/// A name as wide as it needs, at most `limit`: offered less, it truncates and takes only the
/// width of what it shows, so no gap opens between a cut name and its close button.
private struct HuggingName: Layout {
    var limit: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let text = subviews.first else { return .zero }
        let width = min(proposal.width ?? .infinity, limit)
        return text.sizeThatFits(ProposedViewSize(width: width, height: proposal.height))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews.first?.place(at: CGPoint(x: bounds.minX, y: bounds.midY), anchor: .leading,
                              proposal: ProposedViewSize(width: bounds.width, height: bounds.height))
    }
}

/// One room: its face, a broadcast mark on the stream's own tab, its name, and a close button on
/// every room but the stream's. Folded (a crowded row), a room not on screen shows its face and
/// marks only. Offered less than its natural width, only the name gives way.
private struct TabPill: View {
    let tab: ChatTabInfo
    let active: Bool
    var folded = false
    /// Other rooms are folded: the room on screen may use the width they gave up.
    var crowded = false
    /// The row is at its tightest: a folded room is a smaller face with less air.
    var tight = false
    let mark: Bool
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            Button(action: select) {
                HStack(spacing: 6) {
                    TabFace(url: tab.avatar, label: tab.label, size: folded && tight ? 16 : 20)
                        // Live rooms wear the sidebar's dot; the stream's tab says it with its mark.
                        .overlay(alignment: .bottomTrailing) {
                            if tab.live == true && !tab.pinnedToStream {
                                Circle().fill(SN.live)
                                    .frame(width: 8, height: 8)
                                    .overlay(Circle().stroke(SN.background, lineWidth: 1.5))
                                    .offset(x: 2, y: 2)
                                    .accessibilityLabel("Live")
                            }
                        }
                    if tab.pinnedToStream {
                        Image(systemName: "dot.radiowaves.left.and.right")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(SN.accent)
                            .accessibilityHidden(true)
                    }
                    if mark {
                        PlatformMark(platform: tab.provider == "kick" ? .kick : .twitch, size: 11)
                    }
                    if !folded {
                        // At most 96 pt (the content's 136 pt), unless the others folded for it.
                        HuggingName(limit: crowded && active ? .infinity : 96) {
                            Text(tab.label)
                                .font(.sn(13, active ? 600 : 400))
                                .foregroundStyle(active ? SN.textPrimary : SN.textSecondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                        .layoutPriority(-1)
                    }
                    // Mentions of you here, until you read them.
                    if tab.mentions > 0 && !active {
                        Text(tab.mentions > 99 ? "99+" : "\(tab.mentions)")
                            .font(.sn(10.5, 600))
                            .monospacedDigit()
                            .foregroundStyle(.white)
                            .padding(.horizontal, 4)
                            .frame(minWidth: 16, minHeight: 16)
                            .background(Capsule().fill(SN.error))
                    }
                }
                .padding(.trailing, folded ? 0 : 4)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(tab.label)
            .accessibilityAddTraits(active ? .isSelected : [])
            if folded {
                EmptyView()
            } else if tab.pinnedToStream {
                Color.clear.frame(width: 2, height: 1)
            } else {
                Button(action: close) {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(SN.textMuted)
                        // The full pill height keeps the target about as large as a 24 pt square.
                        .frame(width: 20, height: 32)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close \(tab.label) chat")
            }
        }
        // A folded pill is a chip: the face 6 pt from each end (2 pt when tight).
        .padding(.leading, folded ? (tight ? 2 : 6) : 10)
        .padding(.trailing, folded ? (tight ? 2 : 6) : 4)
        .frame(height: 32)
        .background { if active { GlazeSelected() } }
        .animation(SNMotion.run(.easeOut(duration: 0.2)), value: folded)
    }
}

/// A room's face (20 points, 16 in the tightest row), or its first letter on a surface circle.
private struct TabFace: View {
    let url: String?
    let label: String
    var size: CGFloat = 20

    var body: some View {
        Group {
            if let url = url.flatMap(URL.init(string:)) {
                AsyncImage(url: url) { image in
                    image.resizable().scaledToFill()
                } placeholder: {
                    initial
                }
            } else {
                initial
            }
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .accessibilityHidden(true)
    }

    private var initial: some View {
        Circle().fill(SN.surface)
            .overlay {
                Text(label.prefix(1).uppercased())
                    .font(.sn(10, 600))
                    .foregroundStyle(SN.textMuted)
            }
    }
}
