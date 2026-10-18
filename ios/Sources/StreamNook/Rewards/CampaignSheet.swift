import SNKit
import SwiftUI

/// Every reward in one campaign, in the order they are earned, with where it can be earned and
/// a way to go and watch it. Sized to its content; "Find a stream" stays at the bottom.
struct CampaignSheet: View {
    let model: AppModel
    let card: CampaignCard
    let close: () -> Void
    @State private var channelsOpen = false

    /// The pinned button's 50 points and its insets, which the fitted height must include.
    private var pinned: CGFloat { card.findStream == nil ? 0 : 66 }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            VStack(spacing: 4) {
                SheetTitle(text: card.name)
                    .padding(.bottom, -8)
                Text([card.gameName, card.metaLabel].filter { !$0.isEmpty }.joined(separator: " · "))
                    .font(.sn(12.5))
                    .monospacedDigit()
                    .foregroundStyle(SN.textMuted)
            }
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .padding(.bottom, 8)

            if card.needsLink { linkRow }
            if !card.channels.isEmpty { channelsRow }
            VStack(spacing: 6) {
                ForEach(card.drops) { DropRewardRow(drop: $0) }
            }
            .padding(.top, 4)
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 16 + pinned)
        .fittedSheet(maxFraction: 0.8)
        .safeAreaInset(edge: .bottom) {
            if let category = card.findStream {
                Button { findStream(category) } label: { Text("Find a stream") }
                    .buttonStyle(.snSecondary)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
            }
        }
    }

    /// Watching earns nothing until the game account is linked; the row opens the link page.
    private var linkRow: some View {
        Button {
            guard let text = card.linkURL, let url = URL(string: text) else { return }
            Task {
                if !(await UIApplication.shared.open(url)) {
                    model.toasts.show("Could not open the account link page.")
                }
            }
        } label: {
            WarningRow(text: "Your game account is not linked, so this campaign will not earn yet.",
                       trailing: card.linkURL == nil ? nil : Symbol.openOutside)
        }
        .buttonStyle(CardPress())
        .disabled(card.linkURL == nil)
        .opacity(card.linkURL == nil ? 0.7 : 1)
    }

    /// The channels a restricted campaign credits on, collapsed; each one starts its stream.
    private var channelsRow: some View {
        VStack(spacing: 4) {
            Button {
                withAnimation(SNMotion.run(SNMotion.settle)) { channelsOpen.toggle() }
            } label: {
                HStack(spacing: 8) {
                    Text(card.channelsLabel)
                        .font(.sn(12.5))
                        .foregroundStyle(SN.textSecondary)
                    Spacer(minLength: 8)
                    Image(systemName: Symbol.disclosure)
                        .font(.system(size: 12, weight: .bold))
                        .foregroundStyle(SN.textMuted)
                        .rotationEffect(.degrees(channelsOpen ? 90 : 0))
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .frame(maxWidth: .infinity, alignment: .leading)
                .snPanel()
            }
            .buttonStyle(CardPress())
            .accessibilityValue(channelsOpen ? "Expanded" : "Collapsed")

            if channelsOpen {
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(Array(card.channels.enumerated()), id: \.element.id) { index, channel in
                            Button { watch(channel) } label: {
                                HStack {
                                    Text(channel.name).font(.sn(13)).foregroundStyle(SN.textPrimary)
                                    Spacer()
                                    Text("Watch").font(.snCaption).foregroundStyle(SN.textMuted)
                                }
                                .padding(.horizontal, 12)
                                .frame(height: 40)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(RowPress())
                            if index < card.channels.count - 1 { Divider().overlay(SN.borderSubtle) }
                        }
                    }
                }
                // About 40 percent of the screen, so eighty channels never push the rewards away.
                .frame(maxHeight: 320)
                .fixedSize(horizontal: false, vertical: true)
                .snPanel()
                .transition(.opacity)
            }
        }
    }

    private func findStream(_ category: TwitchCategory) {
        close()
        model.nav.tab = .browse
        model.nav.push(.category(category))
    }

    /// Starts the channel's stream with its live details when it is live, bare otherwise. The
    /// Twitch user id rides along so drops follow this channel even when it is not followed.
    private func watch(_ channel: CampaignChannel) {
        close()
        Task {
            let live = try? await model.core.invoke("check_stream_online", args: ["userLogin": channel.login],
                                                    as: TwitchStream?.self)
            var stream = (live ?? nil) ?? TwitchStream(id: "channel-\(channel.login)", userLogin: channel.login,
                                                      userName: channel.name, title: "", viewerCount: 0, gameName: "")
            if stream.userID?.isEmpty ?? true { stream.userID = channel.id }
            model.startWatching(stream)
        }
    }
}

/// A caution inside a sheet: the warning glyph and a sentence, with an optional trailing symbol
/// when the row opens something.
struct WarningRow: View {
    let text: String
    var trailing: String?

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: Symbol.warning)
                .font(.system(size: 14))
                .foregroundStyle(SN.warning)
                .padding(.top, 1)
            Text(text)
                .font(.sn(12.5))
                .foregroundStyle(SN.textSecondary)
                .multilineTextAlignment(.leading)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 4)
            if let trailing {
                Image(systemName: trailing)
                    .font(.system(size: 13))
                    .foregroundStyle(SN.textMuted)
                    .padding(.top, 1)
            }
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .snPanel()
        .accessibilityElement(children: .combine)
    }
}

/// One reward: its picture, name, where it stands, Twitch's progress line and a bar.
private struct DropRewardRow: View {
    let drop: DropRow

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            RewardArt(url: drop.image, width: 40, height: 40, radius: 6, ring: true)
                .opacity(drop.claimed || drop.image.isEmpty ? 1 : 0.9)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(drop.name)
                        .font(.sn(13))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    Spacer(minLength: 4)
                    if !drop.status.isEmpty {
                        Text(drop.status)
                            .font(.sn(11, drop.done && !drop.claimed ? 600 : 400))
                            .monospacedDigit()
                            .foregroundStyle(drop.done ? SN.success : SN.textMuted)
                    }
                }
                if let detail = drop.detail {
                    Text(detail)
                        .font(.sn(11))
                        .foregroundStyle(SN.textMuted)
                        .lineLimit(1)
                }
                if let bar = drop.bar {
                    ProgressTrack(percent: bar, height: 4, color: drop.done ? SN.success : SN.accent)
                        .padding(.top, 3)
                }
            }
            .frame(minHeight: 40)
        }
        .padding(.vertical, 4)
        .accessibilityElement(children: .combine)
    }
}
