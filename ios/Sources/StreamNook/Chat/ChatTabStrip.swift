import SNKit
import SwiftUI

/// The open rooms over the chat, shown from two rooms up so single-room chat keeps every point
/// of height. A 45 point bar: the rooms in one glazed capsule that scrolls inside its fixed rim,
/// the room on screen a darker pill. It reads only the tabs, never the rows, so arriving
/// messages never rebuild it.
struct ChatTabStrip: View {
    let rooms: ChatRooms

    static let height: CGFloat = 45

    var body: some View {
        HStack(spacing: 0) {
            // The capsule hugs its rooms; once they outgrow the bar they scroll inside its rim.
            ViewThatFits(in: .horizontal) {
                pills
                ScrollView(.horizontal, showsIndicators: false) { pills }
            }
            .clipShape(Capsule())
            .snGlass(Capsule())
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 8)
        .frame(height: Self.height)
        .overlay(alignment: .bottom) {
            SN.borderSubtle.frame(height: 1).accessibilityHidden(true)
        }
    }

    private var pills: some View {
        HStack(spacing: 4) {
            ForEach(rooms.tabs) { tab in
                TabPill(tab: tab, active: tab.channel == rooms.state.activeChannel,
                        mark: rooms.state.mixedPlatforms && tab.provider != "twitch",
                        select: { rooms.select(tab.channel) }, close: { rooms.close(tab.channel) })
            }
        }
        .padding(4)
    }
}

/// One room: its face, a broadcast mark on the stream's own tab, its name, and a close button on
/// every room but the stream's.
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
                            .font(.system(size: 11, weight: .bold))
                            .foregroundStyle(SN.accent)
                            .accessibilityHidden(true)
                    }
                    if mark {
                        PlatformMark(platform: tab.provider == "kick" ? .kick : .twitch, size: 11)
                    }
                    Text(tab.label)
                        .font(.sn(13, active ? 600 : 400))
                        .foregroundStyle(active ? SN.textPrimary : SN.textSecondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
                .padding(.trailing, 4)
                .frame(maxWidth: 136, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(tab.label)
            .accessibilityAddTraits(active ? .isSelected : [])
            if tab.pinnedToStream {
                Color.clear.frame(width: 6, height: 1)
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
