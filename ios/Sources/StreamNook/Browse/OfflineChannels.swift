import SNKit
import SwiftUI

/// Channels a search found that are not streaming. They are not stream cards: there is no preview,
/// no viewers and nothing to play, and drawn as cards they read as live channels with 0 viewers that
/// opened onto nothing. A row opens the channel's offline room, where its chat is and where its
/// stream starts when it goes live.
struct OfflineChannels: View {
    let channels: [TwitchStream]
    let open: (TwitchStream) -> Void

    private static let avatar: CGFloat = 36

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionLabel(text: "Offline channels")
                .padding(.horizontal, 4)
            VStack(spacing: 0) {
                ForEach(Array(channels.enumerated()), id: \.element.userLogin) { index, channel in
                    if index > 0 {
                        Rectangle().fill(SN.borderSubtle).frame(height: 0.5)
                            .padding(.leading, 14 + Self.avatar + 12)
                    }
                    Button { open(channel) } label: { row(channel) }
                        .buttonStyle(CardPress())
                }
            }
            // The stream cards' own surface, so the group and the live cards above read as one list.
            .snMediaCard()
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 24)
    }

    private func row(_ channel: TwitchStream) -> some View {
        HStack(spacing: 12) {
            picture(channel)
            VStack(alignment: .leading, spacing: 2) {
                Text(channel.userName.isEmpty ? channel.userLogin : channel.userName)
                    .font(.snRowStrong)
                    .foregroundStyle(SN.textPrimary)
                Text(channel.gameName.isEmpty ? "Offline" : channel.gameName)
                    .font(.snRowDetail)
                    .foregroundStyle(SN.textSecondary)
            }
            .lineLimit(1)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    /// Search hands back the channel's picture as its thumbnail; with neither, its initial.
    @ViewBuilder
    private func picture(_ channel: TwitchStream) -> some View {
        let url = channel.profileImageURL ?? (channel.thumbnailURL.isEmpty ? nil : channel.thumbnailURL)
        if url != nil {
            Avatar(url: url, size: Self.avatar)
        } else {
            Text(String((channel.userName.isEmpty ? channel.userLogin : channel.userName).prefix(1)).uppercased())
                .font(.sn(14, 600))
                .foregroundStyle(SN.textSecondary)
                .frame(width: Self.avatar, height: Self.avatar)
                .background(Circle().fill(SN.surface))
                .accessibilityHidden(true)
        }
    }
}
