import SNKit
import SwiftUI

/// The channels you follow that are live, as cards or rows. Signed out, it asks you to sign in.
struct FollowingScreen: View {
    let model: AppModel
    @AppStorage(StreamView.key) private var view = StreamView.cards
    @State private var latch = SettleLatch()

    private var home: HomeStore { model.home }

    var body: some View {
        Group {
            if model.signedIn {
                list
            } else {
                SignedOutLanding(model: model)
            }
        }
        .background(SN.background)
        .navigationTitle("Following")
        .toolbarTitleDisplayMode(.inlineLarge)
        .toolbar {
            if model.signedIn {
                ToolbarItem(placement: .topBarTrailing) { StreamViewButtons(view: $view) }
            }
        }
    }

    private var list: some View {
        ScrollView {
            if !home.followedLoaded {
                SkeletonCards()
            } else if home.lists.followedLive.isEmpty {
                EmptyNote(text: "No followed channels are live right now.")
            } else {
                StreamGrid(model: model, streams: home.lists.followedLive, view: view, settleKey: "following:\(view)", latch: latch)
            }
        }
        .scrollIndicators(.hidden)
        .refreshable { await home.loadFollowing(pulled: true) }
        .task(id: model.signedIn) {
            if model.signedIn && !home.followedLoaded { await home.loadFollowing() }
        }
        // Every visit asks for a fresh hype-train pass (the core floors it at 15 s); the first
        // load above already brings one.
        .onAppear { if home.followedLoaded { Task { await home.refreshHypeTrains() } } }
    }
}
