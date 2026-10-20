import SNKit
import SwiftUI

/// Rewards: Twitch Drops and the badge wall, chosen in the title row (Brandon: the parts beside
/// the title, so the row under it goes and the content moves up). Each visit opens on Drops with
/// no search and every game closed.
struct RewardsScreen: View {
    let model: AppModel

    private var rewards: RewardsStore { model.rewards }
    /// The marker under the last row; the bottom safe area already spans the tab bar.
    private static let end = "rewards-end"

    var body: some View {
        @Bindable var rewards = rewards
        ScrollViewReader { scroller in
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    if model.signedIn {
                        switch rewards.segment {
                        case .drops: DropsSegment(model: model, scroller: scroller)
                        case .badges: BadgesSegment(model: model)
                        }
                    } else {
                        SignedOutRewards(model: model)
                    }
                    Color.clear.frame(height: 0).id(RewardsScreen.end)
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 24)
            }
            .onChange(of: rewards.scrollToEnd) { _, asked in
                guard asked else { return }
                rewards.scrollToEnd = false
                withAnimation(SNMotion.run(SNMotion.settle)) {
                    scroller.scrollTo(RewardsScreen.end, anchor: .bottom)
                }
            }
        }
        .scrollEdgeEffectStyle(.soft, for: .bottom)
        .scrollDismissesKeyboard(.never)
        // A tap on a result puts the keyboard away and opens it.
        .tapsThroughKeyboard()
        .snShellBackground()
        .navigationTitle("Rewards")
        .toolbarTitleDisplayMode(.inlineLarge)
        .toolbar {
            if model.signedIn {
                ToolbarItem(placement: .topBarTrailing) {
                    TitleSegments(segments: [Segment(value: RewardsStore.Part.drops, label: "Drops"),
                                             Segment(value: RewardsStore.Part.badges, label: "Badges")],
                                  selection: $rewards.segment)
                }
            }
        }
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

    private func refresh() async {
        switch rewards.segment {
        case .drops: await rewards.load(refresh: true)
        case .badges: await rewards.loadWall(force: true)
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
