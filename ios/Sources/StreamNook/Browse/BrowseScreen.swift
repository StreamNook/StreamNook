import Observation
import SNKit
import SwiftUI
import UIKit

/// Browse: what Twitch recommends live, or the categories, with search covering both; or the
/// same on Kick. Needs no sign-in.
struct BrowseScreen: View {
    let model: AppModel
    @AppStorage(StreamView.key) private var view = StreamView.cards
    /// `twitch` or `kick`; anything else reads as Twitch.
    @AppStorage("sn-browse-platform") private var platform = "twitch"
    @AppStorage(PlatformChooserStyle.key) private var chooser = PlatformChooserStyle.row
    @State private var liveLatch = SettleLatch()
    @State private var categoryLatch = SettleLatch()

    private var home: HomeStore { model.home }
    private var browse: BrowseModel { model.browseState }
    private var onKick: Bool { platform == "kick" }

    var body: some View {
        @Bindable var browse = model.browseState
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                if chooser == .search {
                    // Option B: this field stands where the system search bar stood.
                    PlatformSearchField(options: [.twitch, .kick], platform: platformBinding, query: $browse.query,
                                        prompt: searchPrompt)
                        .padding(.horizontal, 16)
                }
                // The modes and the platform share one row, so nothing below moves.
                HStack(spacing: 8) {
                    GlazeSegments([Segment(value: BrowseModel.Mode.live, label: "Live"),
                                   Segment(value: BrowseModel.Mode.categories, label: "Categories")],
                                  selection: modeBinding,
                                  accessory: chooser == .capsule
                                      ? AnyView(PlatformMenu(options: [.twitch, .kick], selection: platformBinding, size: 16))
                                      : nil)
                    Spacer(minLength: 0)
                    if chooser == .row { PlatformSwitch(platform: platformBinding) }
                }
                .padding(.horizontal, 16)
                .padding(.top, 4)
                content
            }
        }
        .scrollIndicators(.hidden)
        .scrollEdgeEffectStyle(.soft, for: .bottom)
        .scrollDismissesKeyboard(.immediately)
        .background(SN.background)
        .navigationTitle("Browse")
        .toolbarTitleDisplayMode(.inlineLarge)
        .modifier(SystemSearch(enabled: chooser != .search, query: $browse.query, prompt: searchPrompt))
        .toolbar {
            if chooser == .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    PlatformMenu(options: [.twitch, .kick], selection: platformBinding)
                }
                if browse.mode == .live { ToolbarSpacer(.fixed, placement: .topBarTrailing) }
            }
            if browse.mode == .live {
                ToolbarItem(placement: .topBarTrailing) { StreamViewButtons(view: $view) }
            }
        }
        .refreshable { await refresh() }
        .task { if !home.recommendedLoaded { await home.loadRecommended() } }
        // Twitch ids only: hype trains and Shared Viewership never look at Kick rows.
        .reportsChannels(onKick ? [] : browse.streamResults ?? home.recommended, to: model.core)
        // The core answers these signed out too (the public GQL), so Browse never waits on a sign-in.
        .task(id: "\(platform)|\(browse.mode)|\(browse.query)|\(model.signedIn)") {
            if onKick {
                if browse.mode == .live { await browse.kick.search(browse.query, core: model.core) }
            } else {
                await browse.search(model.core)
            }
        }
        .task(id: "\(platform)|\(browse.mode)|\(model.signedIn)|\(browse.kick.category?.id ?? "")") {
            if onKick {
                if browse.mode == .categories {
                    await browse.kick.loadCategories(model.core)
                } else {
                    await browse.kick.loadDirectory(model.core)
                }
            } else if browse.mode == .categories {
                await browse.loadCategories(model.core)
            }
        }
    }

    private var searchPrompt: String {
        browse.mode == .categories ? "Search categories" : onKick ? "Search Kick channels" : "Search channels"
    }

    /// Switching platform clears the search and any chosen Kick category.
    private var platformBinding: Binding<String> {
        Binding(get: { onKick ? "kick" : "twitch" }, set: { next in
            guard next != (onKick ? "kick" : "twitch") else { return }
            platform = next
            browse.query = ""
            browse.kick.reset()
        })
    }

    private var modeBinding: Binding<BrowseModel.Mode?> {
        Binding(get: { browse.mode }, set: { if let mode = $0 { browse.mode = mode } })
    }

    @ViewBuilder
    private var content: some View {
        if onKick {
            KickBrowseBody(model: model, kick: browse.kick, mode: browse.mode, query: browse.query, view: view,
                           liveLatch: liveLatch, categoryLatch: categoryLatch) { category in
                // A Kick category filters the directory in place instead of opening a screen.
                browse.query = ""
                browse.kick.category = category
                browse.mode = .live
            }
        } else {
            results
        }
    }

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
                // A search also finds channels that are not streaming (Rust settles `is_live` from
                // Twitch's streams answer); they get their own group under the live cards.
                let live = shown.filter { !$0.isOffline }
                let offline = shown.filter(\.isOffline)
                if !live.isEmpty {
                    StreamGrid(model: model, streams: live, view: view,
                               settleKey: "\(view):\(browse.streamResults != nil ? "q:\(browse.trimmed)" : "recommended")",
                               latch: liveLatch)
                }
                if !offline.isEmpty {
                    OfflineChannels(channels: offline, open: { model.startWatching($0) })
                }
            }
        case .categories:
            let shown = browse.categoryResults ?? browse.categories
            if (browse.categoriesLoading || browse.searching) && shown.isEmpty {
                SkeletonTiles()
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
        if onKick {
            home.renewPreviews()
            if browse.mode == .categories {
                await browse.kick.loadCategories(model.core, force: true)
            } else if !browse.trimmed.isEmpty {
                await browse.kick.search(browse.query, core: model.core, immediate: true)
            } else {
                await browse.kick.loadDirectory(model.core)
            }
            return
        }
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
    /// Browse switched to Kick.
    let kick = KickBrowseModel()
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
        kick.reset()
    }

    /// Searches 350 ms after the last keystroke, in the mode showing; a canceled search writes
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
    @Environment(\.displayScale) private var scale

    var body: some View {
        let playing = latch.playing(settleKey)
        let count = WindowShape(width: width, height: 874).categoryColumns
        let art = CategoryTile.artSize(gridWidth: width, columns: count, scale: scale)
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: CategoryTile.gap, alignment: .top), count: count),
                  spacing: CategoryTile.rowGap) {
            ForEach(Array(categories.enumerated()), id: \.element.id) { index, category in
                Button { open(category) } label: { CategoryTile(category: category, art: art) }
                    .buttonStyle(CardPress())
                    .settleIn(index: index, active: playing)
                    .onAppear {
                        if let more, index >= categories.count - 3 * count { Task { await more() } }
                    }
            }
        }
        .padding(.horizontal, CategoryTile.inset)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .padding(.top, CategoryTile.top)
        .padding(.bottom, 24)
        .settles(latch, key: settleKey, ready: !categories.isEmpty)
    }
}

/// One category: the box art is the tile, with nothing around it and nothing over it, and a chin
/// under it: the name on one line, then Twitch's live total for the category. Every chin is one
/// height (a long name ends in an ellipsis, and VoiceOver reads it whole), so no row grows.
struct CategoryTile: View {
    let category: TwitchCategory
    /// The box art's size in pixels, the size the tile is drawn at.
    let art: (width: Int, height: Int)
    @Environment(\.displayScale) private var scale

    static let gap: CGFloat = 10
    static let rowGap: CGFloat = 14
    /// The page's 16 pt margin, so the art lines up with the search field and segments above it.
    static let inset: CGFloat = 16
    /// The space between the header and the first row of tiles.
    static let top: CGFloat = 4

    /// The art's pixel size for a column, rounded up a step so small width changes ask for the
    /// same picture.
    static func artSize(gridWidth: CGFloat, columns: Int, scale: CGFloat) -> (width: Int, height: Int) {
        let column = (gridWidth - 2 * inset - CGFloat(columns - 1) * gap) / CGFloat(columns)
        let width = Int((max(column, 60) * scale / 30).rounded(.up)) * 30
        return (width, width * 4 / 3)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            artwork
            CategoryChin.block
                .overlay(alignment: .topLeading) {
                    VStack(alignment: .leading, spacing: 0) {
                        Text(category.name)
                            .font(CategoryChin.nameFont)
                            .foregroundStyle(SN.textPrimary)
                            .lineLimit(1)
                            .allowsTightening(true)
                        if let viewers {
                            Text(viewers)
                                .font(CategoryChin.viewersFont)
                                // Nobody live is the least useful fact, so it is a step quieter (still AA).
                                .foregroundStyle(category.viewerCount == 0 ? SN.textSecondary.opacity(0.7) : SN.textSecondary)
                                .lineLimit(1)
                        }
                    }
                }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel([category.name, viewers].compactMap { $0 }.joined(separator: ", "))
    }

    private var artwork: some View {
        let shape = RoundedRectangle(cornerRadius: SNRadius.tile, style: .continuous)
        return Color.clear
            .aspectRatio(3 / 4, contentMode: .fit)
            .background { BoxArt(url: TwitchImage.boxArt(category.boxArtURL, width: art.width, height: art.height)) }
            .clipShape(shape)
            // A pixel-thin edge so art as dark as the page keeps its shape; on bright art it vanishes.
            .overlay { shape.strokeBorder(SN.textPrimary.opacity(0.10), lineWidth: 1 / scale) }
    }

    /// Absent only when the core could not count (its fallback search carries no viewers).
    private var viewers: String? {
        guard let count = category.viewerCount else { return nil }
        switch count {
        case 0: return "No live streams"
        case 1: return "1 viewer"
        default: return "\(count.formatted(.number.notation(.compactName))) viewers"
        }
    }
}

/// A category's box art. Twitch's CDN answers a category with no art by redirecting to its shared
/// `404_boxart` picture (a gray controller box); that answer shows our own quiet placeholder instead.
private struct BoxArt: View {
    let url: URL?
    /// The picture for `url`, or nil once Twitch said it has none.
    @State private var loaded: Loaded?

    private struct Loaded {
        let url: URL
        let image: UIImage?
    }

    var body: some View {
        ZStack {
            if url == nil || (loaded?.url == url && loaded?.image == nil) {
                SN.surface
                Image(systemName: "gamecontroller")
                    .font(.system(size: 26, weight: .regular))
                    .foregroundStyle(SN.textMuted)
            } else if let loaded, loaded.url == url, let image = loaded.image {
                Image(uiImage: image).resizable().scaledToFill()
            } else {
                SN.surface
            }
        }
        .task(id: url) {
            guard let url, loaded?.url != url else { return }
            guard let (data, response) = try? await URLSession.shared.data(from: url), !Task.isCancelled else { return }
            if response.url?.absoluteString.contains("404_boxart") == true {
                loaded = Loaded(url: url, image: nil)
            } else if let image = await UIImage(data: data)?.byPreparingForDisplay(), !Task.isCancelled {
                loaded = Loaded(url: url, image: image)
            }
        }
    }
}

/// The category chin's type and its fixed height, shared by the tile and its skeleton.
@MainActor
enum CategoryChin {
    static let nameFont = Font.sn(12.5, 500)
    static let viewersFont = Font.snCaption

    /// An invisible block the chin's height: a name line, then the viewers line, so a tile the core
    /// sent no viewers for still lines up with its row.
    static var block: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(verbatim: " ").font(nameFont)
            Text(verbatim: " ").font(viewersFont)
        }
        .hidden()
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The system search bar under the title, unless the screen draws its own field instead.
private struct SystemSearch: ViewModifier {
    let enabled: Bool
    @Binding var query: String
    let prompt: String

    func body(content: Content) -> some View {
        if enabled {
            content
                .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: prompt)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .submitLabel(.search)
        } else {
            content
        }
    }
}
