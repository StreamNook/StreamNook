import SNKit
import SwiftUI

/// The open rooms over the chat, shown from two rooms up so single-room chat keeps every point
/// of height. A 45 point bar: the rooms in one glazed capsule, the room on screen a darker pill.
/// The capsule hugs its rooms; when they outgrow the bar it fills the bar (the same inset at both
/// ends) and the widest names truncate first, so the row compresses in place and never scrolls,
/// clips or wraps. It reads only the tabs, never the rows, so arriving messages never rebuild it.
struct ChatTabStrip: View {
    let rooms: ChatRooms

    static let height: CGFloat = 45

    var body: some View {
        HStack(spacing: 0) {
            TabRow(spacing: 4) {
                ForEach(rooms.tabs) { tab in
                    TabPill(tab: tab, active: tab.channel == rooms.state.activeChannel,
                            mark: rooms.state.mixedPlatforms && tab.provider != "twitch",
                            select: { rooms.select(tab.channel) }, close: { rooms.close(tab.channel) })
                }
            }
            .padding(4)
            .snGlass(Capsule())
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .frame(height: Self.height)
        .overlay(alignment: .bottom) {
            SN.borderSubtle.frame(height: 1).accessibilityHidden(true)
        }
    }
}

/// Tabs side by side at their natural widths while they fit. Past the width it is offered, the
/// widest tabs give up width first (each down to a shared cap), so short names keep their room and
/// every tab stays inside the row.
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

    /// Natural widths, or, when they overflow, each capped at the level where they just fit.
    static func widths(_ subviews: Subviews, available: CGFloat?, spacing: CGFloat) -> [CGFloat] {
        let natural = subviews.map { $0.sizeThatFits(.unspecified).width }
        guard let available else { return natural }
        let room = available - spacing * CGFloat(max(0, subviews.count - 1))
        return capped(natural, toFit: room)
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

/// One room: its face, a broadcast mark on the stream's own tab, its name, and a close button on
/// every room but the stream's. Offered less than its natural width, only the name gives way.
private struct TabPill: View {
    let tab: ChatTabInfo
    let active: Bool
    let mark: Bool
    let select: () -> Void
    let close: () -> Void

    var body: some View {
        HStack(spacing: 0) {
            Button(action: select) {
                HStack(spacing: 6) {
                    TabFace(url: tab.avatar, label: tab.label)
                    if tab.pinnedToStream {
                        Image(systemName: "dot.radiowaves.left.and.right")
                            .font(.system(size: 11, weight: .medium))
                            .foregroundStyle(SN.accent)
                            .accessibilityHidden(true)
                    }
                    if mark {
                        PlatformMark(platform: tab.provider == "kick" ? .kick : .twitch, size: 11)
                    }
                    // At most 96 pt, so the content stays within the 136 pt a tab may take.
                    Text(tab.label)
                        .font(.sn(13, active ? 600 : 400))
                        .foregroundStyle(active ? SN.textPrimary : SN.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        .frame(maxWidth: 96, alignment: .leading)
                        .fixedSize(horizontal: false, vertical: true)
                        .layoutPriority(-1)
                }
                .padding(.trailing, 4)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(tab.label)
            .accessibilityAddTraits(active ? .isSelected : [])
            if tab.pinnedToStream {
                Color.clear.frame(width: 2, height: 1)
            } else {
                Button(action: close) {
                    Image(systemName: "xmark")
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(SN.textMuted)
                        .frame(width: 24, height: 24)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Close \(tab.label) chat")
            }
        }
        .padding(.leading, 10)
        .padding(.trailing, 4)
        .frame(height: 32)
        .background { if active { GlazeSelected() } }
    }
}

/// A room's 20 point face, or its first letter on a surface circle.
private struct TabFace: View {
    let url: String?
    let label: String

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
        .frame(width: 20, height: 20)
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
