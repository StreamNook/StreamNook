import Observation
import SNKit
import SwiftUI

/// Browse: what Twitch recommends live, or the categories, with search covering both. Needs no
/// sign-in.
struct BrowseScreen: View {
    let model: AppModel
    @AppStorage(StreamView.key) private var view = StreamView.cards
    @State private var liveLatch = SettleLatch()
    @State private var categoryLatch = SettleLatch()

    private var home: HomeStore { model.home }
    private var browse: BrowseModel { model.browseState }

    var body: some View {
        @Bindable var browse = model.browseState
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                GlazeSegments([Segment(value: BrowseModel.Mode.live, label: "Live"),
                               Segment(value: BrowseModel.Mode.categories, label: "Categories")],
                              selection: modeBinding)
                    .padding(.horizontal, 16)
                    .padding(.top, 4)
                content
            }
        }
        .scrollIndicators(.hidden)
        .scrollDismissesKeyboard(.immediately)
        .background(SN.background)
        .navigationTitle("Browse")
        .toolbarTitleDisplayMode(.inlineLarge)
        .searchable(text: $browse.query, placement: .navigationBarDrawer(displayMode: .always),
                    prompt: browse.mode == .live ? "Search channels" : "Search categories")
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .submitLabel(.search)
        .toolbar {
            if browse.mode == .live {
                ToolbarItem(placement: .topBarTrailing) { StreamViewButtons(view: $view) }
            }
        }
        .refreshable { await refresh() }
        .task { if !home.recommendedLoaded { await home.loadRecommended() } }
        .reportsChannels(browse.streamResults ?? home.recommended, to: model.core)
        // The core answers these signed out too (the public GQL), so Browse never waits on a sign-in.
        .task(id: "\(browse.mode)|\(browse.query)|\(model.signedIn)") { await browse.search(model.core) }
        .task(id: "\(browse.mode)|\(model.signedIn)") {
            if browse.mode == .categories { await browse.loadCategories(model.core) }
        }
    }

    private var modeBinding: Binding<BrowseModel.Mode?> {
        Binding(get: { browse.mode }, set: { if let mode = $0 { browse.mode = mode } })
    }

    private var content: some View { results }

    @ViewBuilder
    private var results: some View {
        switch browse.mode {
        case .live:
            let shown = browse.streamResults ?? home.recommended
            if (!home.recommendedLoaded && shown.isEmpty) || (browse.searching && shown.isEmpty) {
                SkeletonCards()
            } else if shown.isEmpty {
                EmptyNote(text: browse.streamResults != nil ? "No channels found." : "Nothing to recommend yet.")
            } else {
                StreamGrid(model: model, streams: shown, view: view,
                           settleKey: "\(view):\(browse.streamResults != nil ? "q:\(browse.trimmed)" : "recommended")",
                           latch: liveLatch)
            }
        case .categories:
            let shown = browse.categoryResults ?? browse.categories
            if (browse.categoriesLoading || browse.searching) && shown.isEmpty {
                SkeletonCards()
            } else if shown.isEmpty {
                EmptyNote(text: browse.categoryResults != nil ? "No categories found." : "No categories yet, pull to refresh.", hint: nil)
            } else {
                CategoryGrid(categories: shown, latch: categoryLatch,
                             settleKey: browse.categoryResults != nil ? "q:\(browse.trimmed)" : "top",
                             open: { model.nav.push(.category($0)) },
                             more: browse.categoryResults == nil && browse.hasMore ? { await browse.loadMore(model.core) } : nil)
                if browse.categoryResults == nil && browse.loadingMore {
                    Text("Loading more categories")
                        .font(.snCaption)
                        .foregroundStyle(SN.textMuted)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 16)
                }
            }
        }
    }

    private func refresh() async {
        if !browse.trimmed.isEmpty {
            home.renewPreviews()
            await browse.search(model.core, immediate: true)
        } else if browse.mode == .live {
            await home.loadRecommended(pulled: true)
        } else {
            await browse.loadCategories(model.core, force: true)
        }
    }
}

/// Browse's own state: the mode, the search and its results, and the paged categories.
@Observable @MainActor
final class BrowseModel {
    enum Mode: Hashable { case live, categories }

    var mode = Mode.live
    var query = ""
    private(set) var streamResults: [TwitchStream]?
    private(set) var categoryResults: [TwitchCategory]?
    private(set) var searching = false
    private(set) var categories: [TwitchCategory] = []
    private(set) var categoriesLoading = false
    private(set) var hasMore = true
    private var cursor: String?
    private(set) var loadingMore = false

    var trimmed: String { query.trimmingCharacters(in: .whitespaces) }

    /// Leaving the Browse tab drops the search and the mode, as on Android, where the tab's
    /// screen unmounts; the loaded categories stay for the session.
    func leftTab() {
        mode = .live
        query = ""
        streamResults = nil
        categoryResults = nil
        searching = false
    }

    /// Searches 350 ms after the last keystroke, in the mode showing; a cancelled search writes
    /// nothing, and a failed one shows no results.
    func search(_ core: CoreBridge, immediate: Bool = false) async {
        let text = trimmed
        guard !text.isEmpty else {
            streamResults = nil
            categoryResults = nil
            searching = false
            return
        }
        searching = true
        if !immediate {
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled else { return }
        }
        switch mode {
        case .live:
            let found = try? await core.invoke("search_channels", args: ["query": text], as: [TwitchStream].self)
            guard !Task.isCancelled else { return }
            streamResults = found ?? []
        case .categories:
            let found = try? await core.invoke("search_categories", args: ["query": text, "limit": 40], as: [TwitchCategory].self)
            guard !Task.isCancelled else { return }
            categoryResults = found ?? []
        }
        searching = false
    }

    func loadCategories(_ core: CoreBridge, force: Bool = false) async {
        guard force || categories.isEmpty, !categoriesLoading else { return }
        categoriesLoading = true
        defer { categoriesLoading = false }
        do {
            let page = try await core.invoke("get_top_games_paginated", args: ["cursor": nil, "limit": 40],
                                             as: Page<TwitchCategory>.self)
            categories = page.items
            cursor = page.cursor
            hasMore = page.cursor != nil
        } catch {
            log.error("categories failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The next page, when the end of the list comes near. Twitch can repeat an entry across a
    /// page boundary, so repeats are dropped.
    func loadMore(_ core: CoreBridge) async {
        guard !loadingMore, hasMore, let cursor else { return }
        loadingMore = true
        defer { loadingMore = false }
        do {
            let page = try await core.invoke("get_top_games_paginated", args: ["cursor": cursor, "limit": 40],
                                             as: Page<TwitchCategory>.self)
            let seen = Set(categories.map(\.id))
            categories += page.items.filter { !seen.contains($0.id) }
            self.cursor = page.cursor
            hasMore = page.cursor != nil && !page.items.isEmpty
        } catch {
            hasMore = false
        }
    }
}

/// Categories as box-art tiles, three across on a phone and more on a wider window, loading the
/// next page as the end nears.
struct CategoryGrid: View {
    let categories: [TwitchCategory]
    let latch: SettleLatch
    let settleKey: String
    let open: (TwitchCategory) -> Void
    let more: (() async -> Void)?
    @State private var width: CGFloat = 402

    var body: some View {
        let playing = latch.playing(settleKey)
        let count = WindowShape(width: width, height: 874).categoryColumns
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12, alignment: .top), count: count), spacing: 12) {
            ForEach(Array(categories.enumerated()), id: \.element.id) { index, category in
                Button { open(category) } label: {
                    VStack(alignment: .leading, spacing: 6) {
                        Color.clear
                            .aspectRatio(3 / 4, contentMode: .fit)
                            .background {
                                AsyncImage(url: TwitchImage.boxArt(category.boxArtURL)) { phase in
                                    if let image = phase.image { image.resizable().scaledToFill() } else { SN.surface }
                                }
                            }
                            .clipShape(RoundedRectangle(cornerRadius: SNRadius.thumb, style: .continuous))
                        Text(category.name)
                            .font(.snCardTitle)
                            .foregroundStyle(SN.textPrimary)
                            .lineLimit(1)
                            .padding(.horizontal, 2)
                    }
                    .padding(8)
                    .snMediaCard()
                }
                .buttonStyle(CardPress())
                .settleIn(index: index, active: playing)
                .onAppear {
                    if let more, index >= categories.count - 3 * count { Task { await more() } }
                }
            }
        }
        .padding(.horizontal, 16)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .padding(.bottom, 24)
        .settles(latch, key: settleKey, ready: !categories.isEmpty)
    }
}
