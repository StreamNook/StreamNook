import Observation
import SNKit
import SwiftUI

/// Browse switched to Kick: Kick's live directory (most watched first, one page of 100), an exact
/// channel jump for a search, and Kick's categories, where a tap filters the directory in place.
/// Every row is a Kick row, so no card wears a platform mark.
@Observable @MainActor
final class KickBrowseModel {
    private(set) var streams: [TwitchStream] = []
    /// The answer to the current search (nil without one).
    private(set) var results: [TwitchStream]?
    private(set) var categories: [ProviderCategory] = []
    /// The category the directory is filtered to.
    var category: ProviderCategory?
    private(set) var loading = false
    private(set) var loaded = false
    private(set) var categoriesLoading = false
    private(set) var searching = false

    /// The directory, or the chosen category's streams. A newer load wins: a canceled one writes
    /// nothing, and a failed one shows the empty state.
    func loadDirectory(_ core: CoreBridge) async {
        loading = true
        defer { loading = false }
        let page = try? await core.invoke("provider_directory", args: [
            "provider": "kick", "category": category?.id, "cursor": nil, "limit": 100,
        ], as: StreamPage.self)
        guard !Task.isCancelled else { return }
        streams = (page?.streams ?? []).map(\.asKick)
        loaded = true
    }

    /// Kick's search is an exact channel jump: at most one channel, 350 ms after the last key.
    func search(_ query: String, core: CoreBridge, immediate: Bool = false) async {
        let text = query.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else {
            results = nil
            searching = false
            return
        }
        searching = true
        if !immediate {
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled else { return }
        }
        let page = try? await core.invoke("provider_search", args: ["provider": "kick", "query": text], as: StreamPage.self)
        guard !Task.isCancelled else { return }
        results = (page?.streams ?? []).map(\.asKick)
        searching = false
    }

    /// Kick's most watched categories (the core keeps the first page ten minutes). A failed load
    /// keeps what was loaded.
    func loadCategories(_ core: CoreBridge, force: Bool = false) async {
        guard force || categories.isEmpty, !categoriesLoading else { return }
        categoriesLoading = true
        defer { categoriesLoading = false }
        if let page = try? await core.invoke("provider_categories", args: ["provider": "kick", "cursor": nil, "limit": 40],
                                              as: CategoryPage.self) {
            categories = page.categories
        }
    }

    /// Switching platform starts Kick fresh.
    func reset() {
        category = nil
        results = nil
        searching = false
    }
}

/// The Kick half of Browse, under the same header, search field and modes.
struct KickBrowseBody: View {
    let model: AppModel
    let kick: KickBrowseModel
    let mode: BrowseModel.Mode
    let query: String
    let view: StreamView
    let liveLatch: SettleLatch
    let categoryLatch: SettleLatch
    /// A category tile was tapped: Browse goes back to Live, filtered to it.
    let pick: (ProviderCategory) -> Void

    var body: some View {
        switch mode {
        case .live:
            live
        case .categories:
            categoriesBody
        }
    }

    private var trimmed: String { query.trimmingCharacters(in: .whitespaces) }

    @ViewBuilder
    private var live: some View {
        let shown = kick.results ?? kick.streams
        if let category = kick.category, trimmed.isEmpty {
            CategoryChip(name: category.name) {
                kick.category = nil
            }
            .padding(.horizontal, 16)
        }
        if (!kick.loaded && shown.isEmpty) || (kick.searching && shown.isEmpty) || (kick.loading && shown.isEmpty) {
            SkeletonCards()
        } else if shown.isEmpty {
            EmptyNote(text: kick.results != nil ? "No Kick channels found." : "Nothing live on Kick right now.")
        } else {
            // A search can find a channel that is not streaming: it joins the offline group, never
            // a live card with no viewers.
            let live = shown.filter { !$0.isOffline }
            let offline = shown.filter(\.isOffline)
            if !live.isEmpty {
                StreamGrid(model: model, streams: live, view: view,
                           settleKey: "kick:\(view):\(kick.results != nil ? "q:\(trimmed)" : "c:\(kick.category?.id ?? "all")")",
                           latch: liveLatch)
            }
            if !offline.isEmpty {
                OfflineChannels(channels: offline, open: { model.startWatching($0) })
            }
        }
    }

    @ViewBuilder
    private var categoriesBody: some View {
        let shown = ProviderCategory.filter(kick.categories, query)
        if kick.categoriesLoading && kick.categories.isEmpty {
            SkeletonTiles()
        } else if shown.isEmpty {
            EmptyNote(text: trimmed.isEmpty ? "No categories yet, pull to refresh." : "No categories found.", hint: nil)
        } else {
            CategoryGrid(categories: shown.map(\.tile), latch: categoryLatch, settleKey: "kick",
                         open: { tile in if let found = kick.categories.first(where: { $0.id == tile.id }) { pick(found) } },
                         more: nil)
        }
    }
}

/// The chosen Kick category above the list, with an X that shows all of Kick again.
private struct CategoryChip: View {
    let name: String
    let clear: () -> Void

    var body: some View {
        Button(action: clear) {
            HStack(spacing: 2) {
                Text(name)
                    .font(.sn(13, 500))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                Image(systemName: Symbol.close)
                    .font(.system(size: 11, weight: .bold))
                    .foregroundStyle(SN.textSecondary)
                    .frame(width: 22, height: 24)
            }
            .padding(.leading, 12)
            .padding(.trailing, 4)
            .padding(.vertical, 2)
            .snGlass(Capsule())
            // The whole chip clears, with a full-size target around it.
            .padding(.vertical, 10)
            .contentShape(Rectangle())
            .padding(.vertical, -10)
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Show all of Kick, not just \(name)")
    }
}
