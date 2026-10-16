import SNKit
import SwiftUI

/// The channels streaming together with the watched one (Shared Viewership), or sharing its chat:
/// who they are, the host, their viewers, and a tap to switch the player to any of them.
struct TogetherSheet: View {
    let group: ChannelGroup
    /// The login playing now; its row says "Watching" and is not a button.
    let playing: String
    let onWatch: (ChannelGroup.Member) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            VStack(spacing: 8) {
                HStack(spacing: -6) {
                    ForEach(group.members.prefix(5), id: \.userID) { member in
                        Face(url: member.avatarURL, size: 28)
                            .overlay(Circle().stroke(SN.background, lineWidth: 2))
                    }
                }
                Text(group.together ? "Streaming together" : "Sharing chat")
                    .font(.snSheetTitle)
                    .foregroundStyle(SN.textPrimary)
                    .accessibilityAddTraits(.isHeader)
                Text(group.summary)
                    .font(.sn(13))
                    .monospacedDigit()
                    .foregroundStyle(SN.textSecondary)
            }
            .padding(.top, 22)
            .padding(.bottom, 12)

            VStack(spacing: 0) {
                ForEach(group.members, id: \.userID) { member in
                    let here = member.login.lowercased() == playing.lowercased()
                    Button {
                        dismiss()
                        onWatch(member)
                    } label: {
                        row(member, here: here)
                    }
                    .buttonStyle(.plain)
                    .disabled(here)
                    .accessibilityLabel(here ? "\(member.displayName), watching" : "Watch \(member.displayName)")
                    if member.userID != group.members.last?.userID {
                        Divider().overlay(SN.borderSubtle).padding(.leading, 56)
                    }
                }
            }
            .snPanel()
            .padding(.horizontal, 16)
            .padding(.bottom, 16)
        }
        .fittedSheet()
    }

    private func row(_ member: ChannelGroup.Member, here: Bool) -> some View {
        HStack(spacing: 12) {
            Face(url: member.avatarURL, size: 32)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(member.displayName)
                        .font(.sn(15, 500))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    if member.isLeader == true {
                        Text("HOST")
                            .font(.sn(10, 700))
                            .tracking(0.4)
                            .foregroundStyle(SN.accent)
                    }
                }
                if here {
                    Text("Watching")
                        .font(.sn(12))
                        .foregroundStyle(SN.textMuted)
                }
            }
            Spacer(minLength: 8)
            if let viewers = member.viewerCount {
                Text(viewers.formatted())
                    .font(.sn(13, 500))
                    .monospacedDigit()
                    .foregroundStyle(SN.textSecondary)
            }
            if !here {
                Image(systemName: Symbol.play)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(SN.textSecondary)
            }
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 56)
        .contentShape(Rectangle())
    }
}

/// A round channel face, or a quiet circle while it loads.
private struct Face: View {
    let url: String?
    let size: CGFloat

    var body: some View {
        AsyncImage(url: url.flatMap(URL.init(string:))) { image in
            image.resizable().scaledToFill()
        } placeholder: {
            SN.surface
        }
        .frame(width: size, height: size)
        .clipShape(Circle())
        .accessibilityHidden(true)
    }
}
