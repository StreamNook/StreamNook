import SNKit
import SwiftUI

extension Symbol {
    static let giftOutline = "gift"
    static let calendar = "calendar"
    static let broadcast = "dot.radiowaves.left.and.right"
    static let dice = "dice"
}

/// The Drops segment: the connect card, or the search and the three lists the core builds.
struct DropsSegment: View {
    let model: AppModel
    let scroller: ScrollViewProxy
    @State private var latch = SettleLatch()

    private var rewards: RewardsStore { model.rewards }

    var body: some View {
        @Bindable var rewards = rewards
        Group {
            switch rewards.drops {
            case nil:
                Color.clear.frame(height: 1)
            case let drops? where !drops.connected:
                ConnectDropsCard(connecting: rewards.connecting, error: rewards.connectError) {
                    Task { await rewards.connect() }
                }
            case let drops?:
                connected(drops)
            }
        }
        .sheet(item: $rewards.campaignSheet) { card in
            CampaignSheet(model: model, card: card) { rewards.campaignSheet = nil }
        }
        .task(id: rewards.query) {
            guard rewards.drops?.connected == true else { return }
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            await rewards.load(refresh: false)
        }
        .task(id: "\(rewards.focusCampaign ?? "")|\(rewards.drops?.inProgress.count ?? -1)") {
            guard let id = rewards.takeFocus() else { return }
            try? await Task.sleep(for: .milliseconds(320))
            withAnimation(SNMotion.run(SNMotion.settle)) { scroller.scrollTo(id, anchor: .center) }
        }
    }

    @ViewBuilder
    private func connected(_ drops: RewardsDrops) -> some View {
        let key = "drops:\(drops.query)"
        let settling = latch.playing(key)
        let ready = !(drops.inProgress.isEmpty && drops.available.isEmpty)
        VStack(alignment: .leading, spacing: 0) {
            DropsSearchField(text: Binding(get: { rewards.query }, set: { rewards.query = $0 }))
                .padding(.vertical, 4)

            SectionLabel(text: "In progress").padding(.top, 8).padding(.bottom, 6)
            if drops.inProgress.isEmpty {
                Text(drops.inProgressEmpty)
                    .font(.sn(13))
                    .foregroundStyle(SN.textMuted)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(16)
                    .snPanel()
            } else {
                ForEach(Array(drops.inProgress.enumerated()), id: \.element.key) { index, group in
                    GameGroupSection(group: group, open: rewards.isOpen(group.key)) {
                        withAnimation(SNMotion.run(SNMotion.settle)) { rewards.toggle(group.key) }
                    } content: {
                        ForEach(group.campaigns) { card in
                            Button { rewards.campaignSheet = card } label: {
                                ProgressCard(card: card, flashed: rewards.flashed == card.id)
                            }
                            .buttonStyle(CardPress())
                            .id(card.id)
                        }
                    }
                    .settleIn(index: index, active: settling)
                }
            }

            if !drops.available.isEmpty {
                SectionLabel(text: "Available now").padding(.top, 16).padding(.bottom, 6)
                ForEach(Array(drops.available.enumerated()), id: \.element.key) { index, group in
                    GameGroupSection(group: group, open: rewards.isOpen(group.key)) {
                        withAnimation(SNMotion.run(SNMotion.settle)) { rewards.toggle(group.key) }
                    } content: {
                        ForEach(group.campaigns) { card in
                            Button { rewards.campaignSheet = card } label: { AvailableCard(card: card) }
                                .buttonStyle(CardPress())
                                .id(card.id)
                        }
                    }
                    .settleIn(index: drops.inProgress.count + index, active: settling)
                }
            }

            if !drops.earned.isEmpty {
                SectionLabel(text: "Recently earned").padding(.top, 16).padding(.bottom, 6)
                VStack(spacing: 8) {
                    ForEach(drops.earned) { EarnedRow(drop: $0) }
                }
            }
        }
        .settles(latch, key: key, ready: ready)
    }
}

/// A section's small uppercase label.
struct SectionLabel: View {
    let text: String

    var body: some View {
        Text(text)
            .font(.snSection)
            .textCase(.uppercase)
            .tracking(0.6)
            .foregroundStyle(SN.textMuted)
            .accessibilityAddTraits(.isHeader)
    }
}

/// The search over game and campaign names, inline above the lists.
private struct DropsSearchField: View {
    @Binding var text: String
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: Symbol.search)
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(SN.textMuted)
            TextField("", text: $text, prompt: Text("Search games or campaigns").foregroundStyle(SN.textMuted))
                .font(.sn(14))
                .foregroundStyle(SN.textPrimary)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.search)
                .focused($focused)
            if !text.isEmpty {
                Button { text = "" } label: {
                    Image(systemName: Symbol.clear)
                        .font(.system(size: 15))
                        .foregroundStyle(SN.textMuted)
                        .frame(width: 32, height: 32)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.leading, 12)
        .padding(.trailing, text.isEmpty ? 12 : 4)
        .frame(height: 40)
        .snPanel(radius: 10)
        .contentShape(Rectangle())
        .onTapGesture { focused = true }
    }
}

/// One game, collapsed to its heading until opened.
private struct GameGroupSection<Content: View>: View {
    let group: DropsGroup
    let open: Bool
    let toggle: () -> Void
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button(action: toggle) {
                HStack(spacing: 10) {
                    RewardArt(url: group.art, width: 40, height: 53, radius: 4, placeholder: nil)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(group.game)
                            .font(.sn(14, 600))
                            .foregroundStyle(SN.textPrimary)
                            .lineLimit(1)
                        Text(group.countLabel)
                            .font(.snCaption)
                            .foregroundStyle(SN.textMuted)
                    }
                    Spacer(minLength: 8)
                    Image(systemName: Symbol.disclosure)
                        .font(.system(size: 13, weight: .bold))
                        .foregroundStyle(SN.textMuted)
                        .rotationEffect(.degrees(open ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(RowPress())
            .padding(.top, 12)
            .padding(.bottom, 6)
            .accessibilityElement(children: .combine)
            .accessibilityValue(open ? "Expanded" : "Collapsed")
            .accessibilityHint(open ? "Collapses the game" : "Shows the game's campaigns")

            if open {
                VStack(spacing: 8) { content() }
                    .padding(.bottom, 4)
                    .transition(.opacity.combined(with: .offset(y: -6)))
            }
        }
        .clipped()
    }
}

/// An in-progress campaign: its name, claimed out of total, and the progress bar.
private struct ProgressCard: View {
    let card: CampaignCard
    let flashed: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 8) {
                Text(card.name)
                    .font(.sn(13.5, 500))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                Spacer(minLength: 4)
                Text(card.claimedLabel)
                    .font(.snCaption)
                    .monospacedDigit()
                    .foregroundStyle(SN.textMuted)
            }
            ProgressTrack(percent: card.percent, height: 6, color: SN.accent)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .snPanel()
        .overlay {
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .strokeBorder(SN.accent, lineWidth: 2)
                .opacity(flashed ? 1 : 0)
                .animation(SNMotion.run(.easeOut(duration: 0.5)), value: flashed)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(card.name), \(card.claimedLabel) claimed")
    }
}

/// A running campaign not started yet: its reward, name, and rewards with the time left.
private struct AvailableCard: View {
    let card: CampaignCard

    var body: some View {
        HStack(spacing: 10) {
            RewardArt(url: card.art, width: 52, height: 52, radius: 6, ring: true)
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(card.name)
                        .font(.sn(13.5, 500))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    if card.needsLink {
                        Image(systemName: Symbol.warning)
                            .font(.system(size: 12))
                            .foregroundStyle(SN.warning)
                            .accessibilityLabel("Game account not linked")
                    }
                }
                Text(card.detail)
                    .font(.sn(11.5))
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .snPanel()
        .accessibilityElement(children: .combine)
    }
}

/// A drop earned lately.
private struct EarnedRow: View {
    let drop: EarnedDrop

    var body: some View {
        HStack(spacing: 10) {
            RewardArt(url: drop.image, width: 36, height: 36, radius: 6)
            VStack(alignment: .leading, spacing: 1) {
                Text(drop.name)
                    .font(.sn(13.5))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                if let game = drop.game {
                    Text(game).font(.snCaption).foregroundStyle(SN.textMuted).lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            Image(systemName: Symbol.checkCircle)
                .font(.system(size: 16))
                .foregroundStyle(SN.success)
                .accessibilityLabel("Earned")
        }
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .snPanel()
        .accessibilityElement(children: .combine)
    }
}

/// A rounded progress track.
struct ProgressTrack: View {
    let percent: Double
    var height: CGFloat = 4
    var color: Color = SN.accent

    var body: some View {
        GeometryReader { geo in
            ZStack(alignment: .leading) {
                Capsule().fill(SN.surface)
                Capsule().fill(color)
                    .frame(width: geo.size.width * min(max(percent, 0), 100) / 100)
            }
        }
        .frame(height: height)
        .animation(SNMotion.run(.easeOut(duration: 0.7)), value: percent)
        .accessibilityHidden(true)
    }
}

/// Drops not connected: what connecting gets you, and the button.
private struct ConnectDropsCard: View {
    let connecting: Bool
    let error: String?
    let connect: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                PlatformMark(platform: .twitch, size: 15)
                Text("Twitch Drops").font(.snRowStrong).foregroundStyle(SN.textPrimary)
            }
            Text("Connect drops to track campaign progress and earn while you watch.")
                .font(.sn(13))
                .foregroundStyle(SN.textSecondary)
                .lineSpacing(3)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 6)
                .padding(.bottom, 16)
            Button(action: connect) {
                HStack(spacing: 6) {
                    Text(connecting ? "Approve on Twitch…" : "Connect drops")
                    if !connecting {
                        Image(systemName: Symbol.openOutside).font(.system(size: 14, weight: .semibold))
                    }
                }
            }
            .buttonStyle(.snPrimary)
            .disabled(connecting)
            if let error {
                Text(error)
                    .font(.snCaption)
                    .foregroundStyle(SN.error)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 8)
            }
        }
        .padding(16)
        .snPanel()
        .padding(.top, 8)
    }
}
