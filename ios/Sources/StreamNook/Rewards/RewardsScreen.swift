import SNKit
import SwiftUI

/// Rewards: Twitch Drops and the badge wall, two segments under one title. Drops opens first every
/// time the tab is visited.
struct RewardsScreen: View {
    let model: AppModel
    @State private var segment: Part = .drops
    @State private var dropsConnected: Bool?
    @State private var connecting = false

    enum Part: Hashable { case drops, badges }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                if model.signedIn {
                    GlazeSegments([Segment(value: RewardsScreen.Part.drops, label: "Drops"),
                                   Segment(value: RewardsScreen.Part.badges, label: "Badges")],
                                  selection: Binding(get: { segment }, set: { if let next = $0 { segment = next } }))
                        .padding(.top, 4)
                    switch segment {
                    case .drops: drops
                    case .badges: badges
                    }
                } else {
                    SignedOutRewards(model: model)
                }
            }
            .padding(.horizontal, 16)
            .padding(.bottom, 24)
        }
        .scrollIndicators(.hidden)
        .background(SN.background)
        .navigationTitle("Rewards")
        .toolbarTitleDisplayMode(.inlineLarge)
        .refreshable { await checkDrops() }
        .task(id: model.signedIn) { if model.signedIn { await checkDrops() } }
        .onAppear { segment = .drops }
        .task { await followDropsSignIn() }
        .onChange(of: model.host.sheet == nil) { _, closed in if closed { connecting = false } }
    }

    @ViewBuilder
    private var drops: some View {
        switch dropsConnected {
        case nil:
            EmptyView()
        case false?:
            ConnectDropsCard(connecting: connecting) { Task { await connectDrops() } }
        case true?:
            EmptyNote(text: "Your drop campaigns show here once the Drops screen is built.", hint: nil)
        }
    }

    private var badges: some View {
        EmptyNote(text: "Your badge wall shows here once it is built.", hint: nil)
    }

    private func checkDrops() async {
        dropsConnected = (try? await model.core.invoke("is_drops_authenticated", args: [:], as: Bool.self)) ?? false
    }

    /// The core opens Twitch's Drops authorize page in the web sheet; the grant comes back
    /// through the sheet and the core announces it.
    private func connectDrops() async {
        connecting = true
        do {
            _ = try await model.core.invoke("open_drops_sheet", "{}")
        } catch {
            connecting = false
            model.toasts.show("Could not open the drops sign-in. Try again in a moment.")
            log.error("open_drops_sheet failed: (String(describing: error), privacy: .public)")
        }
    }

    /// Drops sign-in outcomes, from the sheet or anywhere else, and the sheet closing unfinished.
    private func followDropsSignIn() async {
        for await event in model.core.events.all() {
            switch event.name {
            case "drops-login-complete":
                connecting = false
                await checkDrops()
            case "drops-login-error":
                connecting = false
                model.toasts.show("Drops sign-in failed. Try again.")
            default:
                continue
            }
        }
    }
}

/// Drops not connected: the gift, "Twitch Drops", what connecting gets you, and the button.
private struct ConnectDropsCard: View {
    let connecting: Bool
    let connect: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: Symbol.gift).font(.system(size: 15, weight: .semibold)).foregroundStyle(SN.accent)
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
                    Text(connecting ? "Connecting…" : "Connect drops")
                    Image(systemName: Symbol.openOutside).font(.system(size: 14, weight: .semibold))
                }
            }
            .buttonStyle(.snPrimary)
            .disabled(connecting)
        }
        .padding(16)
        .snPanel()
        .padding(.top, 8)
    }
}

/// Signed out of Twitch, Rewards says what it holds and offers the sign-in.
private struct SignedOutRewards: View {
    let model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Image(systemName: Symbol.gift).font(.system(size: 15, weight: .semibold)).foregroundStyle(SN.accent)
                Text("Drops and badges").font(.snRowStrong).foregroundStyle(SN.textPrimary)
            }
            Text("Sign in with Twitch to follow your drop campaigns and see which badges you can still earn.")
                .font(.sn(13))
                .foregroundStyle(SN.textSecondary)
                .lineSpacing(3)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 6)
                .padding(.bottom, 16)
            SignInButton(model: model, maxWidth: .infinity)
        }
        .padding(16)
        .snPanel()
        .padding(.top, 8)
    }
}
