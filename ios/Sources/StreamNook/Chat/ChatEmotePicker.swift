import SNKit
import SwiftUI
import UIKit

/// What the picker shows: the selected tab and the core's answer for it (`emote_picker`: tabs
/// with counts, sections, each cell's flags). Typing searches every source at once: the core's
/// ranked search (`match_emote_tokens`, picker mode) with the emoji after it, an All tab, and
/// the other tabs narrowing the results to theirs. Lives in the composer's model, so the tab,
/// the search and the answers survive closing and opening the picker.
@Observable @MainActor
final class EmotePickerModel {
    /// The 7TV size emotes ask for: the smallest tier that covers a 28 pt emote at the chat's
    /// emote scale on this screen, the rule the desktop and Android phone use (`inlineEmoteTier`).
    static func tier(emoteScale: Double) -> String {
        let needed = 28 * emoteScale * Double(UITraitCollection.current.displayScale)
        let tiers: [(name: String, px: Double)] = [("1x", 32), ("2x", 64), ("3x", 96), ("4x", 128)]
        return tiers.first { $0.px >= needed }?.name ?? "4x"
    }

    let session: ChatSession
    private(set) var tab = PickerTabs.defaultTab(provider: "twitch")
    var query = "" {
        didSet {
            guard query != oldValue else { return }
            // Images has no search field.
            if section == .images { return }
            // In GIFs the field searches GIPHY (trending with nothing typed), as on Android.
            if section == .gifs {
                searchGifs(debounce: true)
                return
            }
            if Self.trim(oldValue).isEmpty, searching { scope = PickerTabs.searchScope(from: tab) }
            if !searching {
                search = nil
                emojiMatches = []
            }
            reload(debounce: true)
        }
    }
    private(set) var answer: PickerAnswer?
    /// While searching: the results' tab (`all`, `favorites`, `emoji` or a source).
    private(set) var scope = "all"
    /// The core's ranked search. The last answer stays on screen until the next one lands, so
    /// narrowing a search never flickers.
    private(set) var search: PickerSearch?
    /// Every emoji the search matches.
    private(set) var emojiMatches: [String] = []
    private(set) var loading = false
    private(set) var emoji: EmojiCatalog?
    /// Bumped when the grid should scroll back to the top (a new tab or search).
    private(set) var resetToken = 0
    private var open = false
    private var loadTask: Task<Void, Never>?

    /// The bottom bar's sections: emotes, Twitch chat GIFs, or a picture to upload.
    enum Section { case emotes, gifs, images }
    private(set) var section = Section.emotes
    /// The Images section: the host, the upload in flight and the uploads by host.
    let images: ImageUploadModel
    /// Twitch's answer about GIFs in this channel; nil (no GIFs section) until it answers, and
    /// when the question failed (signed out, offline), as on Android.
    private(set) var gifStatus: GifPickerStatus?
    /// The results for `gifsKey`; nil while they load.
    private(set) var gifs: [GifItem]?
    private var gifsKey = ""
    /// Why a send did not go through, under the search.
    private(set) var gifNotice = ""
    /// The search for `gifsKey` failed: the pane offers to try again.
    private(set) var gifLoadFailed = false
    /// The GIF being sent; the others dim until it answers.
    private(set) var sendingGif: String?
    private var gifTask: Task<Void, Never>?

    init(session: ChatSession) {
        self.session = session
        images = ImageUploadModel(core: session.core) { [weak session] in session?.rooms?.settings }
        tab = PickerTabs.defaultTab(provider: session.provider)
    }

    /// Images is offered where the setting has it on, except TikTok, whose chat drops links.
    var showsImages: Bool { session.style.showImageUploads && session.provider != "tiktok" }

    /// The section the body shows: Images falls back to the emotes once it is turned off.
    var shownSection: Section { section == .images && !showsImages ? .emotes : section }

    /// The tabs in order: Favorites, Emoji, then the room's emote tabs.
    var tabs: [String] {
        PickerTabs.order(coreTabs: answer?.tabs.map(\.id) ?? ["favorites", "twitch", "bttv", "7tv", "ffz"], gif: false)
    }

    var trimmedQuery: String { Self.trim(query) }
    var searching: Bool { !trimmedQuery.isEmpty }

    /// The tab row: All leads it while searching.
    var visibleTabs: [String] { searching ? PickerTabs.searchOrder(tabs) : tabs }

    /// The highlighted tab: the results' scope while searching, else the selected tab.
    var activeTab: String { searching ? scope : tab }

    /// The emotes the search shows in its scope, best first.
    var searchCells: [PickerCell] { search?.cells(in: scope) ?? [] }

    /// The emoji the search shows: a first few after the emotes under All, every one under Emoji.
    var searchEmoji: [String] {
        switch scope {
        case "all": return Array(emojiMatches.prefix(PickerSearch.emojiInAll))
        case "emoji": return emojiMatches
        default: return []
        }
    }

    /// The selected tab's sections, once the core has answered for it.
    var sections: [PickerSection] {
        guard let answer, answer.tab == tab else { return [] }
        return answer.sections
    }

    var waiting: Bool {
        tab != "emoji" && (answer == nil || answer?.tab != tab || answer?.ready == false) && loading
    }

    /// A tab's number: its hits while searching, else what it holds (Emoji shows none).
    func count(_ id: String) -> Int? {
        if searching {
            if id == "emoji" { return emojiMatches.count }
            return search?.count(id, emojiHits: emojiMatches.count)
        }
        return id == "emoji" ? nil : answer?.count(id)
    }

    func setOpen(_ on: Bool) {
        guard on != open else { return }
        open = on
        // The emote disk cache works in bursts while the picker shows.
        let core = session.core
        Task { _ = try? await core.invoke("asset_cache_set_burst", args: ["active": on]) }
        if on {
            // Images is a detour, as on Android: every open starts back on the emotes.
            if section == .images { section = .emotes }
            reload(debounce: false)
            loadGifStatus()
        }
    }

    // MARK: GIFs (the core's commands/gifs.rs, the same calls as Android's picker)

    /// Asks once per open whether GIFs can be posted here: Twitch rooms with a known room.
    private func loadGifStatus() {
        guard session.provider == "twitch", let room = session.roomID else { return }
        let core = session.core
        Task { [weak self] in
            let status = try? await core.invoke("get_gif_picker_status", args: ["channelId": room], as: GifPickerStatus.self)
            self?.gifStatus = status
        }
    }

    func pick(section next: Section) {
        guard next != section else { return }
        section = next
        resetToken += 1
        switch next {
        case .gifs: searchGifs(debounce: false)
        case .emotes: reload(debounce: false)
        // The pane reads its own panel when it shows.
        case .images: break
        }
    }

    /// Trending with nothing typed, else a search 250 ms after the last key.
    private func searchGifs(debounce: Bool) {
        guard gifStatus?.canUse == true, let room = session.roomID else { return }
        let typed = query
        let key = "\(room)|\(typed)"
        gifTask?.cancel()
        let core = session.core
        gifTask = Task { [weak self] in
            if debounce, !typed.isEmpty { try? await Task.sleep(for: .milliseconds(250)) }
            guard !Task.isCancelled else { return }
            do {
                let items = try await core.invoke("search_gifs", args: [
                    "channelId": room, "query": typed.isEmpty ? nil : typed, "offset": 0,
                ], as: [GifItem].self)
                guard let self, !Task.isCancelled else { return }
                self.gifs = items
                self.gifsKey = key
                self.gifLoadFailed = false
            } catch {
                guard let self, !Task.isCancelled else { return }
                self.gifs = []
                self.gifsKey = key
                self.gifLoadFailed = true
            }
        }
    }

    /// Try again after a failed GIF load.
    func retryGifs() {
        gifLoadFailed = false
        gifsKey = ""
        searchGifs(debounce: false)
    }

    /// The results for what is typed now; nil while they load.
    var shownGifs: [GifItem]? {
        guard let room = session.roomID, gifsKey == "\(room)|\(query)" else { return nil }
        return gifs
    }

    /// Picking a GIF sends it, as Twitch's own picker does (a GIF has no text form to insert).
    /// True when it went out.
    func send(_ gif: GifItem) async -> Bool {
        guard sendingGif == nil, let room = session.roomID else { return false }
        sendingGif = gif.id
        gifNotice = ""
        defer { sendingGif = nil }
        do {
            let outcome = try await session.core.invoke("send_gif_message", args: [
                "channelId": room, "gifId": gif.id, "gifUrl": gif.url, "searchTerm": query.isEmpty ? nil : query,
            ], as: SendGifOutcome.self)
            if outcome.sent { return true }
            gifNotice = outcome.notSentLine
        } catch {
            gifNotice = "That GIF could not be sent."
        }
        return false
    }

    func select(_ id: String) {
        guard id != tab else { return }
        tab = id
        resetToken += 1
        reload(debounce: false)
    }

    /// A tab press: narrows the results while searching, else selects the tab. Clearing the
    /// search comes back to the tab selected before it.
    func pick(_ id: String) {
        guard searching else { return select(id) }
        guard id != scope else { return }
        scope = id
        resetToken += 1
    }

    func reload(debounce: Bool) {
        loadTask?.cancel()
        resetToken += debounce ? 1 : 0
        loadTask = Task { [weak self] in
            if debounce { try? await Task.sleep(for: .milliseconds(150)) }
            guard !Task.isCancelled else { return }
            await self?.load()
        }
    }

    private static func trim(_ text: String) -> String {
        text.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func load() async {
        if tab == "emoji" || searching, emoji == nil { emoji = EmojiCatalog.bundled() }
        if searching, answer != nil { return await loadSearch() }
        let core = session.core
        let args: [String: Any?] = [
            "provider": session.provider, "channel": session.login, "channelId": session.roomID,
            "tab": tab == "emoji" ? nil : tab, "query": "", "tier": Self.tier(emoteScale: session.style.emoteScale),
        ]
        loading = true
        defer { loading = false }
        guard let first = try? await core.invoke("emote_picker", args: args, as: PickerAnswer.self),
              !Task.isCancelled else { return }
        answer = first
        // Opened with a search kept from before: the tab row is known now, so search.
        if searching { return await loadSearch() }
        // A Kick room's set is read by its slug inside the same answer.
        if !first.ready, !session.isKick {
            // The room's emotes are not in the core yet; ask it to fetch them, then ask again.
            _ = try? await core.invoke("fetch_channel_emotes", args: [
                "channelName": session.channel, "channelId": session.roomID, "provider": "twitch", "tier": Self.tier(emoteScale: session.style.emoteScale),
            ])
            if let again = try? await core.invoke("emote_picker", args: args, as: PickerAnswer.self), !Task.isCancelled {
                answer = again
            }
        }
    }

    /// The search across every source: the core ranks the emotes as the desktop menu's search
    /// does, and the emoji come from the same catalog as the desktop's.
    private func loadSearch() async {
        let typed = trimmedQuery
        emojiMatches = emoji?.matches(typed) ?? []
        let core = session.core
        let args: [String: Any?] = [
            "provider": session.provider, "channel": session.login, "channelId": session.roomID,
            "query": typed, "profile": "search", "limit": PickerSearch.limit, "picker": true, "tier": Self.tier(emoteScale: session.style.emoteScale),
        ]
        loading = true
        defer { loading = false }
        guard var found = try? await core.invoke("match_emote_tokens", args: args, as: PickerSearch.self),
              !Task.isCancelled else { return }
        if !found.ready, session.provider == "twitch" {
            // The room's emotes are not in the core yet; ask it to fetch them, then ask again.
            _ = try? await core.invoke("fetch_channel_emotes", args: [
                "channelName": session.channel, "channelId": session.roomID, "provider": "twitch", "tier": Self.tier(emoteScale: session.style.emoteScale),
            ])
            guard let again = try? await core.invoke("match_emote_tokens", args: args, as: PickerSearch.self),
                  !Task.isCancelled else { return }
            found = again
        }
        search = found
    }

    /// Adds or removes a favorite and says so.
    func toggleFavorite(_ cell: PickerCell) async -> String {
        let core = session.core
        do {
            if cell.favorite {
                _ = try await core.invoke("remove_favorite_emote_cache", args: ["emoteId": cell.emoteID])
                reload(debounce: false)
                return "Removed \(cell.name) from favorites"
            }
            let saved: [String: Any] = [
                "id": cell.emoteID, "name": cell.name, "url": cell.url, "provider": cell.provider,
                "isZeroWidth": cell.isZeroWidth, "addedAt": Int(Date().timeIntervalSince1970 * 1000),
            ]
            let data = try JSONSerialization.data(withJSONObject: saved)
            _ = try await core.invoke("add_favorite_emote_cache", args: ["emoteData": String(decoding: data, as: UTF8.self)])
            reload(debounce: false)
            return "Added \(cell.name) to favorites"
        } catch {
            return "Failed to update favorites"
        }
    }
}

/// The emote picker over the composer: a search field, the tabs, and the selected tab's grid.
/// Picking an emote puts it in the draft and keeps the picker open.
struct ChatEmotePicker: View {
    let model: ChatComposerModel
    /// A swipe down on the panel's top (search and tabs) puts it away.
    var onSwipeDown: () -> Void = {}
    private var picker: EmotePickerModel { model.picker }

    var body: some View {
        VStack(spacing: 6) {
            // Images has no search and no tabs, as on Android.
            if picker.shownSection != .images {
                searchField
            }
            if picker.shownSection == .emotes {
                tabRow
                    .simultaneousGesture(swipeDown)
            }
            switch picker.shownSection {
            case .gifs: gifGrid
            case .images: ChatImageUploadPane(model: model)
            case .emotes: grid
            }
            // Emotes, GIFs and Images, as Android's bar: GIFs only where the channel has them on.
            if picker.gifStatus?.showsSection == true || picker.showsImages { sectionBar }
        }
        .padding(8)
        // In the keyboard's place: the composer sizes it; flat, with the hairline a keyboard has.
        .frame(maxHeight: .infinity, alignment: .top)
        .background(SN.background)
        .overlay(alignment: .top) { Rectangle().fill(SN.borderSubtle).frame(height: 1) }
    }

    private var searchField: some View {
        @Bindable var picker = picker
        return HStack(spacing: 6) {
            Image(systemName: "magnifyingglass")
                .font(.system(size: 13, weight: .medium))
                .foregroundStyle(ComposerInk.quiet)
            TextField("", text: $picker.query,
                      prompt: Text(picker.section == .gifs ? "Search GIFs..." : "Search emotes...").foregroundStyle(ComposerInk.quiet))
                .font(.sn(13))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(SN.surface, in: .rect(cornerRadius: 10))
        .simultaneousGesture(swipeDown)
    }

    private var swipeDown: some Gesture {
        DragGesture(minimumDistance: 12).onEnded { value in
            if value.translation.height > 56, value.translation.height > abs(value.translation.width) { onSwipeDown() }
        }
    }

    private var tabRow: some View {
        HStack(spacing: 4) {
            ForEach(picker.visibleTabs, id: \.self) { id in
                let active = picker.activeTab == id
                Button {
                    picker.pick(id)
                } label: {
                    VStack(spacing: 2) {
                        tabLabel(id, active: active)
                            .frame(height: 16)
                        Text(picker.count(id).map(PickerTabs.countLabel) ?? " ")
                            .font(.sn(10).monospacedDigit())
                            .foregroundStyle(active ? Color.white : ComposerInk.quiet)
                    }
                    .foregroundStyle(active ? Self.color(id) : ComposerInk.quiet)
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 6)
                    .background(active ? Color.white.opacity(0.08) : .clear, in: .rect(cornerRadius: 8))
                    .contentShape(.rect)
                }
                .buttonStyle(.plain)
                .accessibilityLabel(PickerTabs.title(id))
                .accessibilityAddTraits(active ? .isSelected : [])
            }
        }
    }

    @ViewBuilder private func tabLabel(_ id: String, active: Bool) -> some View {
        switch id {
        case "all":
            Text("All")
                .font(.sn(12, 600))
                .foregroundStyle(active ? Color.white : ComposerInk.quiet)
        case "favorites":
            // A pointed shape needs a point more to look the size of the logos beside it.
            Image(systemName: "star.fill")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(active ? Color.yellow : ComposerInk.quiet)
        case "emoji":
            Image(systemName: "face.smiling")
                .font(.system(size: 15, weight: .regular))
                .foregroundStyle(active ? SN.accent : ComposerInk.quiet)
        default:
            // The provider's own logo, in its brand color while selected.
            ProviderLogo(provider: id, size: 14, tinted: active)
        }
    }

    /// The active tab wears its own brand color; Favorites and Emoji keep the accent.
    static func color(_ id: String) -> Color {
        switch id {
        case "twitch": return Color(red: 0.57, green: 0.28, blue: 1)
        case "bttv": return Color(red: 0.84, green: 0, blue: 0.08)
        case "7tv": return Color(red: 0.16, green: 0.71, blue: 0.96)
        case "ffz": return .white
        case "kick": return Color(red: 0.33, green: 0.98, blue: 0.09)
        default: return SN.accent
        }
    }

    /// The segmented bar at the bottom: one button per section, icon and label (Android's
    /// MenuSwitcher).
    private var sectionBar: some View {
        HStack(spacing: 4) {
            sectionButton(.emotes, label: "Emotes") {
                Image(systemName: "face.smiling").font(.system(size: 13, weight: .medium))
            }
            if picker.gifStatus?.showsSection == true {
                sectionButton(.gifs, label: "GIFs") {
                    Text("GIF")
                        .font(.system(size: 8.5, weight: .heavy))
                        .tracking(0.4)
                        .padding(.horizontal, 3)
                        .overlay(RoundedRectangle(cornerRadius: 3).stroke(lineWidth: 1))
                }
            }
            if picker.showsImages {
                sectionButton(.images, label: "Images") {
                    Image(systemName: "photo").font(.system(size: 13, weight: .medium))
                }
            }
        }
        .padding(.top, 6)
        .overlay(alignment: .top) { Rectangle().fill(SN.borderSubtle).frame(height: 1) }
    }

    private func sectionButton<Icon: View>(_ section: EmotePickerModel.Section, label: String,
                                           @ViewBuilder icon: () -> Icon) -> some View {
        let active = picker.shownSection == section
        return Button {
            picker.pick(section: section)
        } label: {
            HStack(spacing: 6) {
                icon()
                Text(label).font(.sn(12, 600)).lineLimit(1)
            }
            .foregroundStyle(active ? SN.accent : SN.textSecondary)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 6)
            .background(active ? Color.white.opacity(0.08) : .clear, in: .rect(cornerRadius: 8))
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(active ? .isSelected : [])
    }

    /// GIFs: the Tier 2/3 line where they are out of reach, or two columns of GIPHY results
    /// that send on a tap.
    @ViewBuilder private var gifGrid: some View {
        if let line = picker.gifStatus?.tierLine {
            VStack(spacing: 6) {
                Text(line).font(.sn(12)).foregroundStyle(SN.textSecondary)
                Text("GIFs other people post still show in your chat.").font(.sn(11)).foregroundStyle(ComposerInk.quiet)
            }
            .multilineTextAlignment(.center)
            .padding(.horizontal, 24)
            .frame(maxWidth: .infinity, minHeight: 160)
            .frame(maxHeight: .infinity, alignment: .top)
        } else if picker.gifLoadFailed, picker.shownGifs != nil {
            VStack(spacing: 8) {
                Text("Couldn't load GIFs.").font(.sn(13)).foregroundStyle(SN.textMuted)
                Button("Try again") { picker.retryGifs() }
                    .font(.sn(13, 600))
            }
            .frame(maxWidth: .infinity, minHeight: 128)
            .frame(maxHeight: .infinity, alignment: .top)
        } else if let gifs = picker.shownGifs {
            ScrollViewReader { reader in
                ScrollView {
                    VStack(spacing: 8) {
                        if !picker.gifNotice.isEmpty {
                            Text(picker.gifNotice).font(.sn(11)).foregroundStyle(ComposerInk.warning)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        if gifs.isEmpty {
                            centered("No GIFs found")
                        } else {
                            // Two columns at a fixed height: GIPHY art is landscape, and a steady
                            // row height keeps the scroll still while previews stream in.
                            LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                                ForEach(gifs) { gif in gifCell(gif) }
                            }
                        }
                        Text("Powered by GIPHY. Picking a GIF posts it straight to chat.")
                            .font(.sn(10))
                            .foregroundStyle(ComposerInk.quiet)
                            .padding(.top, 4)
                    }
                    .padding(.horizontal, 4)
                    .id("top")
                }
                .mask(Self.edgeFade)
                .onChange(of: picker.resetToken) { _, _ in reader.scrollTo("top", anchor: .top) }
            }
        } else {
            centered("Loading GIFs...")
        }
    }

    private func gifCell(_ gif: GifItem) -> some View {
        let sending = picker.sendingGif == gif.id
        return Button {
            Task {
                // Sent: the panel goes, as Android's does.
                if await picker.send(gif) { onSwipeDown() }
            }
        } label: {
            CosmeticImage(gif.previewURL, animate: model.session.style.animatesEmotes, fill: true, pointHeight: 96)
                .frame(maxWidth: .infinity)
                .frame(height: 96)
                .clipped()
                .background(Color.black.opacity(0.2))
                .clipShape(.rect(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(SN.borderSubtle))
                .overlay {
                    if sending {
                        Text("Sending...").font(.sn(11, 600)).foregroundStyle(.white)
                            .frame(maxWidth: .infinity, maxHeight: .infinity)
                            .background(Color.black.opacity(0.6), in: .rect(cornerRadius: 6))
                    }
                }
                .opacity(picker.sendingGif != nil && !sending ? 0.4 : 1)
        }
        .buttonStyle(.plain)
        .disabled(picker.sendingGif != nil)
        .accessibilityLabel(gif.title.isEmpty ? "GIF" : gif.title)
    }

    @ViewBuilder private var grid: some View {
        if picker.searching {
            searchResults
        } else if picker.tab == "emoji" {
            emojiGrid
        } else if picker.sections.isEmpty {
            if picker.waiting || picker.answer == nil {
                centered("Loading emotes...")
            } else if picker.tab == "favorites", picker.query.isEmpty {
                emptyFavorites
            } else {
                centered("No emotes found")
            }
        } else {
            ScrollViewReader { reader in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8, pinnedViews: [.sectionHeaders]) {
                        ForEach(picker.sections) { section in
                            Section {
                                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: max(1, section.columns)),
                                          spacing: 8) {
                                    ForEach(section.emotes) { cell in
                                        EmoteCell(cell: cell, wide: section.columns < 7,
                                                  animate: model.session.style.animatesEmotes) {
                                            model.insert(cell.name)
                                        } favorite: {
                                            Task { model.show(await picker.toggleFavorite(cell)) }
                                        }
                                    }
                                }
                                .padding(.horizontal, 4)
                            } header: {
                                header(section.title, count: picker.sections.count > 1 ? section.emotes.count : nil,
                                       key: section.key)
                            }
                        }
                    }
                    .id("top")
                }
                .mask(Self.edgeFade)
                .onChange(of: picker.resetToken) { _, _ in reader.scrollTo("top", anchor: .top) }
            }
        }
    }

    /// A 16 pt eased fade at the bottom of a scrolling grid.
    static var edgeFade: some View {
        VStack(spacing: 0) {
            Color.black
            LinearGradient(colors: [.black, .black.opacity(0.6), .clear], startPoint: .top, endPoint: .bottom)
                .frame(height: 16)
        }
    }

    private var emojiGrid: some View {
        let categories = picker.emoji?.filtered(picker.query) ?? []
        return Group {
            if categories.isEmpty {
                centered(picker.emoji == nil ? "Loading emotes..." : "No emojis found")
            } else {
                ScrollViewReader { reader in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 8, pinnedViews: [.sectionHeaders]) {
                            ForEach(categories) { category in
                                Section {
                                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 4), count: 8), spacing: 4) {
                                        ForEach(category.emoji, id: \.self) { emoji in
                                            emojiButton(emoji)
                                        }
                                    }
                                    .padding(.horizontal, 4)
                                } header: {
                                    header(category.name, count: nil)
                                }
                            }
                        }
                        .id("top")
                    }
                    .mask(Self.edgeFade)
                    .onChange(of: picker.resetToken) { _, _ in reader.scrollTo("top", anchor: .top) }
                }
            }
        }
    }

    private func emojiButton(_ emoji: String) -> some View {
        Button {
            model.insert(emoji)
        } label: {
            Text(emoji)
                .font(.system(size: 24))
                .padding(6)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(picker.emoji?.keywords[emoji]?.first ?? emoji)
    }

    /// The search across every source: one ranked grid, best match first, wide emotes taking
    /// two or three cells, then the emoji. The tabs narrow it to one source.
    @ViewBuilder private var searchResults: some View {
        let cells = picker.searchCells
        let emoji = picker.searchEmoji
        if picker.search == nil, picker.scope != "emoji" {
            centered("Searching...")
        } else if cells.isEmpty, emoji.isEmpty {
            centered(picker.scope == "emoji" ? "No emoji found" : "No emotes found for \u{201C}\(picker.trimmedQuery)\u{201D}")
        } else {
            GeometryReader { geo in
                // Seven cells and six 8 pt gaps across, inside the grid's 4 pt sides.
                let unit = max(0, (geo.size.width - 8 - 8 * CGFloat(SearchGrid.columns - 1)) / CGFloat(SearchGrid.columns))
                ScrollViewReader { reader in
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 8, pinnedViews: [.sectionHeaders]) {
                            if !cells.isEmpty {
                                ForEach(Array(SearchGrid.lines(cells).enumerated()), id: \.offset) { _, line in
                                    HStack(spacing: 8) {
                                        ForEach(line, id: \.key) { cell in
                                            EmoteCell(cell: cell, wide: cell.span > 1,
                                                      animate: model.session.style.animatesEmotes) {
                                                model.insert(cell.name)
                                            } favorite: {
                                                Task { model.show(await picker.toggleFavorite(cell)) }
                                            }
                                            .frame(width: unit * CGFloat(cell.span) + 8 * CGFloat(cell.span - 1))
                                        }
                                    }
                                    .padding(.horizontal, 4)
                                }
                                if let total = picker.search?.cappedTotal(picker.scope) {
                                    Text("Showing the best \(cells.count) of \(total). Keep typing to narrow it down.")
                                        .font(.sn(10))
                                        .foregroundStyle(ComposerInk.quiet)
                                        .multilineTextAlignment(.center)
                                        .frame(maxWidth: .infinity)
                                        .padding(.top, 2)
                                }
                            }
                            if !emoji.isEmpty {
                                Section {
                                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 4), count: 8), spacing: 4) {
                                        ForEach(emoji, id: \.self) { emojiButton($0) }
                                    }
                                    .padding(.horizontal, 4)
                                    if picker.scope == "all", picker.emojiMatches.count > emoji.count {
                                        Button("Show all \(picker.emojiMatches.count) emoji") { picker.pick("emoji") }
                                            .font(.sn(11))
                                            .foregroundStyle(SN.textSecondary)
                                            .buttonStyle(.plain)
                                            .frame(maxWidth: .infinity)
                                            .padding(.vertical, 4)
                                    }
                                } header: {
                                    // Under All the emoji follow the emotes as their own block.
                                    if picker.scope == "all" {
                                        header("Emoji", count: picker.emojiMatches.count, mark: "emoji")
                                    }
                                }
                            }
                        }
                        .padding(.top, 2)
                        .id("top")
                    }
                    .mask(Self.edgeFade)
                    .onChange(of: picker.resetToken) { _, _ in reader.scrollTo("top", anchor: .top) }
                }
            }
        }
    }

    /// What leads a section header: the provider's logo on a provider tab, a clock on Recently
    /// used, a star on the rest of Favorites.
    @ViewBuilder private func headerMark(_ key: String, tab: String) -> some View {
        switch tab {
        case "favorites":
            Image(systemName: key == "recent" ? "clock.fill" : "star.fill")
                .font(.system(size: 9, weight: .bold))
                .foregroundStyle(key == "recent" ? SN.textSecondary : .yellow)
        case "emoji":
            EmptyView()
        default:
            ProviderLogo(provider: tab, size: 11, tinted: true)
        }
    }

    /// `mark` is the tab whose mark leads the title (the selected tab when not given).
    private func header(_ title: String, count: Int?, key: String = "", mark: String? = nil) -> some View {
        HStack(spacing: 5) {
            headerMark(key, tab: mark ?? picker.tab)
            Text(title.uppercased()).foregroundStyle(SN.textPrimary)
            if let count { Text("(\(count))").opacity(0.5) }
            Spacer()
        }
        .font(.system(size: 10, weight: .bold))
        .tracking(1)
        .foregroundStyle(SN.textSecondary)
        .padding(.vertical, 6)
        .background(SN.background)
        .overlay(alignment: .bottom) { Rectangle().fill(SN.borderSubtle).frame(height: 1) }
    }

    private var emptyFavorites: some View {
        GeometryReader { geo in
            VStack(spacing: 8) {
                Image(systemName: "star")
                    .font(.system(size: 20, weight: .regular))
                Text("No favorites yet. Long-press an emote to add it.")
                    .font(.sn(12))
                    .multilineTextAlignment(.center)
            }
            .foregroundStyle(ComposerInk.quiet)
            .padding(.horizontal, 24)
            .frame(maxWidth: .infinity)
            .position(x: geo.size.width / 2, y: geo.size.height * 0.4)
        }
    }

    private func centered(_ text: String) -> some View {
        // Fills the grid's space from the top, so the search and tabs never move with it.
        Text(text)
            .font(.sn(12))
            .foregroundStyle(ComposerInk.quiet)
            .multilineTextAlignment(.center)
            .padding(.horizontal, 24)
            .frame(maxWidth: .infinity, minHeight: 128)
            .frame(maxHeight: .infinity, alignment: .top)
    }
}

/// One emote in the picker: the image at most 32 pt tall, a purple ring on modifiers and a
/// yellow one on zero-width emotes, dimmed with a lock when locked, dimmed when your personal
/// emote shows instead, and a yellow star when it is a favorite. A long press favorites it
/// and says what the emote is.
private struct EmoteCell: View {
    let cell: PickerCell
    let wide: Bool
    /// The chat's "Animate emotes" setting, as the desktop picker follows it.
    let animate: Bool
    let pick: () -> Void
    let favorite: () -> Void

    var body: some View {
        Button(action: pick) {
            EmoteThumb(url: cell.url, height: 32, wide: wide, animate: animate)
                .padding(wide ? 4 : 6)
                .frame(maxWidth: .infinity, minHeight: 32)
                .overlay(RoundedRectangle(cornerRadius: 8).stroke(ring.opacity(0.45), lineWidth: ring == .clear ? 0 : 1))
                .opacity(cell.locked ? 0.4 : cell.replacedBy != nil ? 0.5 : 1)
                .overlay(alignment: .bottomTrailing) {
                    if cell.locked {
                        Image(systemName: "lock.fill").font(.system(size: 10)).foregroundStyle(.white.opacity(0.6)).padding(2)
                    }
                }
                .overlay(alignment: .topTrailing) {
                    if cell.favorite {
                        Image(systemName: "star.fill").font(.system(size: 9)).foregroundStyle(.yellow).padding(1)
                    }
                }
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .disabled(cell.locked)
        .contextMenu {
            Button(cell.favorite ? "Remove from favorites" : "Add to favorites",
                   systemImage: cell.favorite ? "star.slash" : "star", action: favorite)
            if !cell.locked {
                Button("Insert", systemImage: "text.insert", action: pick)
            }
        } preview: {
            EmoteCard(cell: cell)
        }
        .accessibilityLabel(cell.name)
        .accessibilityHint(cell.detailLines.joined(separator: ", "))
        .accessibilityAction(named: cell.favorite ? "Remove from favorites" : "Add to favorites", favorite)
    }

    private var ring: Color {
        if cell.isModifier { return .purple }
        if cell.isZeroWidth { return .yellow }
        return .clear
    }
}

/// The long-press card: the emote large, its name, where it comes from and what kind of emote
/// it is (the desktop's hover card).
private struct EmoteCard: View {
    let cell: PickerCell

    var body: some View {
        VStack(spacing: 10) {
            EmoteThumb(url: cell.url, height: 96, wide: true)
                .frame(maxWidth: 220)
            VStack(spacing: 3) {
                Text(cell.name)
                    .font(.sn(16, 700))
                    .foregroundStyle(SN.textPrimary)
                // Where it comes from: the provider's logo, then who made it.
                HStack(spacing: 6) {
                    ProviderLogo(provider: cell.provider, size: 12, tinted: true)
                    if let owner = cell.ownerName, !owner.isEmpty {
                        Text("by \(owner)")
                            .font(.sn(12))
                            .foregroundStyle(SN.textSecondary)
                    }
                }
                ForEach(cell.detailLines.filter { !$0.hasPrefix("by ") }, id: \.self) { line in
                    Text(line.uppercased())
                        .font(.system(size: 9.5, weight: .bold))
                        .tracking(0.8)
                        .foregroundStyle(Self.tint(line))
                }
            }
            .multilineTextAlignment(.center)
        }
        .padding(18)
        .frame(minWidth: 200)
        .background(SN.background)
    }

    static func tint(_ line: String) -> Color {
        if line.hasPrefix("Modifier") { return .purple }
        if line == "Zero-Width" { return .yellow }
        return SN.textMuted
    }
}

/// An emote image at a fixed height and its own width (wide emotes stay wide), through the
/// shared image pipeline. An animated one moves on the shared clock, every frame of it decoded
/// once for every copy on screen, and only while it is on screen.
struct EmoteThumb: View {
    let url: String
    var height: CGFloat = 32
    var wide = false
    /// False draws the first frame (the chat's "Animate emotes" setting, in the picker).
    var animate = true

    var body: some View {
        CosmeticImage(ChatImageURL.thumbnail(url, points: height), animate: animate, pointHeight: height)
            .frame(maxWidth: wide ? .infinity : height, maxHeight: height)
            .frame(height: height)
    }
}
