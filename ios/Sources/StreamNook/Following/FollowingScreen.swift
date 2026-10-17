import SNKit
import SwiftUI

/// The channels you follow that are live, as cards or rows, on every platform the iPhone plays
/// (Twitch and Kick), ranked by viewers in Rust. With a Kick follow on the list, a platform filter
/// shows above it. Signed out, it asks you to sign in.
struct FollowingScreen: View {
    let model: AppModel
    @AppStorage(StreamView.key) private var view = StreamView.cards
    /// `twitch`, `kick`, or empty for everything.
    @AppStorage("sn-following-platform") private var platform = ""
    @State private var latch = SettleLatch()

    private var home: HomeStore { model.home }

    /// The filter only exists once you follow someone on a second platform; without one a
    /// saved filter is ignored.
    private var filter: String? {
        guard home.lists.following.followsKick, platform == "twitch" || platform == "kick" else { return nil }
        return platform
    }

    private var rows: [TwitchStream] {
        let all = home.lists.followingLive
        guard let filter else { return all }
        return all.filter { $0.platform == filter }
    }

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
            VStack(alignment: .leading, spacing: 10) {
                if home.lists.following.followsKick {
                    GlazeSegments([Segment(value: "twitch", label: "Twitch", accessibility: "Show Twitch channels", mark: .twitch),
                                   Segment(value: "kick", label: "Kick", accessibility: "Show Kick channels", mark: .kick)],
                                  selection: filterBinding, allowsNone: true)
                        .padding(.horizontal, 16)
                        .padding(.top, 4)
                }
                content
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

    @ViewBuilder
    private var content: some View {
        if !home.followedLoaded {
            SkeletonCards()
        } else if rows.isEmpty {
            EmptyNote(text: emptyText)
        } else {
            StreamGrid(model: model, streams: rows, view: view, settleKey: "following:\(view):\(filter ?? "all")",
                       latch: latch)
        }
    }

    private var emptyText: String {
        switch filter {
        case "twitch": return "No Twitch channels you follow are live right now."
        case "kick": return "No Kick channels you follow are live right now."
        default: return "No followed channels are live right now."
        }
    }

    /// Tap a platform to show only it; tap it again to show everything.
    private var filterBinding: Binding<String?> {
        Binding(get: { filter }, set: { platform = $0 ?? "" })
    }
}
