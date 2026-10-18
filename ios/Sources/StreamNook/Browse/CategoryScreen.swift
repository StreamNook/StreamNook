import SNKit
import SwiftUI

/// One category's live streams, pushed from Browse over the tab bar, paging as the end nears.
struct CategoryScreen: View {
    let model: AppModel
    let category: TwitchCategory
    @State private var streams: [TwitchStream] = []
    @State private var cursor: String?
    @State private var loaded = false
    @State private var loadingMore = false
    @State private var latch = SettleLatch()

    /// The list stops growing here rather than trimming, so nothing moves under the finger.
    static let cap = 320

    var body: some View {
        ScrollView {
            if !loaded {
                SkeletonCards()
            } else if streams.isEmpty {
                EmptyNote(text: "No live streams in this category.", hint: nil)
            } else {
                StreamGrid(model: model, streams: streams, view: .cards, settleKey: "category", latch: latch,
                           showsCategory: false)
                if cursor != nil, streams.count < Self.cap {
                    Text(loadingMore ? "Loading more streams" : " ")
                        .font(.snCaption)
                        .foregroundStyle(SN.textMuted)
                        .padding(.bottom, 16)
                        .onAppear { Task { await loadMore() } }
                }
            }
        }
        .scrollIndicators(.hidden)
        .snShellBackground()
        .navigationTitle(category.name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .refreshable {
            model.home.renewPreviews()
            await load()
        }
        .task { if !loaded { await load() } }
        .reportsChannels(streams, to: model.core)
    }

    private func load() async {
        do {
            let page = try await model.core.invoke("get_streams_by_game", args: ["gameId": category.id, "cursor": nil, "limit": 40],
                                                   as: Page<TwitchStream>.self)
            streams = page.items
            cursor = page.cursor
        } catch {
            log.error("category streams failed: \(String(describing: error), privacy: .public)")
        }
        loaded = true
    }

    private func loadMore() async {
        guard !loadingMore, let next = cursor else { return }
        loadingMore = true
        defer { loadingMore = false }
        guard let page = try? await model.core.invoke("get_streams_by_game", args: ["gameId": category.id, "cursor": next, "limit": 40],
                                                      as: Page<TwitchStream>.self) else {
            cursor = nil
            return
        }
        let seen = Set(streams.map(\.id))
        streams += page.items.filter { !seen.contains($0.id) }
        cursor = page.items.isEmpty ? nil : page.cursor
    }
}
