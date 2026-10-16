import SNKit
import SwiftUI

/// Every reward in one campaign, in the order they are earned, with where it can be earned and
/// a way to go and watch it.
struct CampaignSheet: View {
    let model: AppModel
    let card: CampaignCard
    let close: () -> Void
    @State private var channelsOpen = false

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 8) {
                        if !card.gameName.isEmpty {
                            Text(card.gameName).lineLimit(1)
                        }
                        Spacer(minLength: 8)
                        Text(card.metaLabel).monospacedDigit()
                    }
                    .font(.sn(12.5))
                    .foregroundStyle(SN.textMuted)

                    if card.needsLink { linkRow }
                    if !card.channels.isEmpty { channelsRow }
                    if let category = card.findStream {
                        Button { findStream(category) } label: { Text("Find a stream") }
                            .buttonStyle(.snSecondary)
                            .padding(.vertical, 2)
                    }
                    VStack(spacing: 6) {
                        ForEach(card.drops) { DropRewardRow(drop: $0) }
                    }
                    .padding(.top, 4)
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 24)
            }
            .scrollIndicators(.hidden)
            .navigationTitle(card.name)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(action: close) { Image(systemName: Symbol.close) }
                        .accessibilityLabel("Close")
                }
            }
        }
        .presentationDetents([.fraction(0.8), .large])
        .presentationDragIndicator(.visible)
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
            HStack(alignment: .top, spacing: 8) {
                Image(systemName: Symbol.warning)
                    .font(.system(size: 14))
                    .foregroundStyle(SN.warning)
                    .padding(.top, 1)
                Text("Your game account is not linked, so this campaign will not earn yet.")
                    .font(.sn(12.5))
                    .foregroundStyle(SN.textSecondary)
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 4)
                if card.linkURL != nil {
                    Image(systemName: Symbol.openOutside)
                        .font(.system(size: 13))
                        .foregroundStyle(SN.textMuted)
                        .padding(.top, 1)
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .snPanel()
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
