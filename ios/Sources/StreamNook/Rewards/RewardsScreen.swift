import SNKit
import SwiftUI

/// Rewards: Twitch Drops and the badge wall, two segments under one title. Each visit opens on
/// Drops with no search and every game closed.
struct RewardsScreen: View {
    let model: AppModel

    private var rewards: RewardsStore { model.rewards }

    var body: some View {
        @Bindable var rewards = rewards
        ScrollViewReader { scroller in
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if model.signedIn {
                        GlazeSegments([Segment(value: RewardsStore.Part.drops, label: "Drops"),
                                       Segment(value: RewardsStore.Part.badges, label: "Badges")],
                                      selection: Binding(get: { rewards.segment }, set: { if let next = $0 { rewards.segment = next } }))
                            .padding(.top, 4)
                        switch rewards.segment {
                        case .drops: DropsSegment(model: model, scroller: scroller)
                        case .badges: badges
                        }
                    } else {
                        SignedOutRewards(model: model)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 24)
            }
        }
        .scrollIndicators(.hidden)
        .scrollEdgeEffectStyle(.soft, for: .bottom)
        .scrollDismissesKeyboard(.immediately)
        .background(SN.background)
        .navigationTitle("Rewards")
        .toolbarTitleDisplayMode(.inlineLarge)
        .refreshable { await refresh() }
        .onAppear {
            rewards.visited()
            if model.signedIn { Task { await rewards.load(refresh: true) } }
        }
        .onChange(of: model.signedIn) { _, signedIn in
            if signedIn { Task { await rewards.load(refresh: true) } }
        }
        .task { rewards.listen(toasts: model.toasts) }
        .onChange(of: model.host.sheet == nil) { _, closed in if closed { rewards.sheetClosed() } }
    }

    private var badges: some View {
        EmptyNote(text: "Your badge wall shows here once it is built.", hint: nil)
    }

    private func refresh() async {
        switch rewards.segment {
        case .drops: await rewards.load(refresh: true)
        case .badges: break
        }
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
