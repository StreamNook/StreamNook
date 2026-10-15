import SNKit
import SwiftUI
import UIKit

/// What the picker shows: the selected tab, the search, and the core's answer for them
/// (`emote_picker`: tabs with counts, sections, each cell's flags). Lives in the composer's
/// model, so the tab, the search and the answer survive closing and opening the picker.
@Observable @MainActor
final class EmotePickerModel {
    /// The 7TV size thumbnails ask for: the smallest that covers a 28 pt emote on this screen.
    static var tier: String { UITraitCollection.current.displayScale >= 3 ? "3x" : "2x" }

    let session: ChatSession
    private(set) var tab = PickerTabs.defaultTab(provider: "twitch")
    var query = "" {
        didSet { if query != oldValue { reload(debounce: true) } }
    }
    private(set) var answer: PickerAnswer?
    private(set) var loading = false
    private(set) var emoji: EmojiCatalog?
    /// Bumped when the grid should scroll back to the top (a new tab or search).
    private(set) var resetToken = 0
    private var open = false
    private var loadTask: Task<Void, Never>?

    init(session: ChatSession) {
        self.session = session
    }

    /// The tabs in order: Favorites, Emoji, then the room's emote tabs.
    var tabs: [String] {
        PickerTabs.order(coreTabs: answer?.tabs.map(\.id) ?? ["favorites", "twitch", "bttv", "7tv", "ffz"], gif: false)
    }

    /// The selected tab's sections, once the core has answered for it.
    var sections: [PickerSection] {
        guard let answer, answer.tab == tab else { return [] }
        return answer.sections
    }

    var waiting: Bool {
        tab != "emoji" && (answer == nil || answer?.tab != tab || answer?.ready == false) && loading
    }

    func count(_ id: String) -> Int? {
        id == "emoji" ? nil : answer?.count(id)
    }

    func setOpen(_ on: Bool) {
        guard on != open else { return }
        open = on
        // The emote disk cache works in bursts while the picker shows.
        let core = session.core
        Task { _ = try? await core.invoke("asset_cache_set_burst", args: ["active": on]) }
        if on { reload(debounce: false) }
    }

    func select(_ id: String) {
        guard id != tab else { return }
        tab = id
        resetToken += 1
        reload(debounce: false)
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

    private func load() async {
        if tab == "emoji", emoji == nil { emoji = EmojiCatalog.bundled() }
        let core = session.core
        let args: [String: Any?] = [
            "provider": "twitch", "channel": session.channel, "channelId": session.roomID,
            "tab": tab == "emoji" ? nil : tab, "query": query, "tier": Self.tier,
        ]
        loading = true
        defer { loading = false }
        guard let first = try? await core.invoke("emote_picker", args: args, as: PickerAnswer.self),
              !Task.isCancelled else { return }
        answer = first
        if !first.ready, query.isEmpty {
            // The room's emotes are not in the core yet; ask it to fetch them, then ask again.
            _ = try? await core.invoke("fetch_channel_emotes", args: [
                "channelName": session.channel, "channelId": session.roomID, "provider": "twitch", "tier": Self.tier,
            ])
            if let again = try? await core.invoke("emote_picker", args: args, as: PickerAnswer.self), !Task.isCancelled {
                answer = again
            }
        }
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
    private var picker: EmotePickerModel { model.picker }

    var body: some View {
        @Bindable var picker = picker
        VStack(spacing: 6) {
            TextField("", text: $picker.query, prompt: Text("Search emotes...").foregroundStyle(Color(ChatTheme.textMuted)))
                .font(.sn(13))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(.horizontal, 12)
                .padding(.vertical, 7)
                .background(Color(ChatTheme.surface), in: .rect(cornerRadius: 10))
            tabRow
            grid
        }
        .padding(8)
        .frame(height: min(420, screenHeight * 0.45))
        .background(Color(ChatTheme.background), in: .rect(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color(ChatTheme.borderSubtle)))
        .shadow(color: .black.opacity(0.5), radius: 20, y: 8)
    }

    private var screenHeight: CGFloat {
        (UIApplication.shared.connectedScenes.first as? UIWindowScene)?.screen.bounds.height ?? 844
    }

    private var tabRow: some View {
        HStack(spacing: 4) {
            ForEach(picker.tabs, id: \.self) { id in
                let active = picker.tab == id
                Button {
                    picker.select(id)
                } label: {
                    VStack(spacing: 1) {
                        tabLabel(id, active: active)
                        if let count = picker.count(id) {
                            Text("\(count)").font(.sn(10)).opacity(0.7)
                        }
                    }
                    .foregroundStyle(active ? Self.color(id) : Color(ChatTheme.textSecondary))
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
        case "favorites":
            Text("★").font(.system(size: 13, weight: .bold)).foregroundStyle(.yellow)
        case "emoji":
            Text("😀").font(.system(size: 15))
        default:
            Text(Self.shortTitle(id))
                .font(.system(size: 12, weight: active ? .heavy : .medium))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
    }

    static func shortTitle(_ id: String) -> String {
        switch id {
        case "bttv": return "BTTV"
        case "ffz": return "FFZ"
        default: return PickerTabs.title(id)
        }
    }

    /// The active tab wears its own brand colour; Favorites and Emoji keep the accent.
    static func color(_ id: String) -> Color {
        switch id {
        case "twitch": return Color(red: 0.57, green: 0.28, blue: 1)
        case "bttv": return Color(red: 0.84, green: 0, blue: 0.08)
        case "7tv": return Color(red: 0.16, green: 0.71, blue: 0.96)
        case "ffz": return .white
        case "kick": return Color(red: 0.33, green: 0.98, blue: 0.09)
        default: return Color(ChatTheme.accent)
        }
    }

    @ViewBuilder private var grid: some View {
        if picker.tab == "emoji" {
            emojiGrid
        } else if picker.sections.isEmpty {
            centered(picker.waiting || picker.answer == nil ? "Loading emotes..." : "No emotes found")
        } else {
            ScrollViewReader { reader in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8, pinnedViews: [.sectionHeaders]) {
                        ForEach(picker.sections) { section in
                            Section {
                                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: max(1, section.columns)),
                                          spacing: 8) {
                                    ForEach(section.emotes) { cell in
                                        EmoteCell(cell: cell, wide: section.columns < 7) {
                                            model.insert(cell.name)
                                        } favorite: {
                                            Task { model.show(await picker.toggleFavorite(cell)) }
                                        }
                                    }
                                }
                                .padding(.horizontal, 4)
                            } header: {
                                header(section.title, count: section.emotes.count)
                            }
                        }
                    }
                    .id("top")
                }
                .onChange(of: picker.resetToken) { _, _ in reader.scrollTo("top", anchor: .top) }
            }
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
                                    }
                                    .padding(.horizontal, 4)
                                } header: {
                                    header(category.name, count: nil)
                                }
                            }
                        }
                        .id("top")
                    }
                    .onChange(of: picker.resetToken) { _, _ in reader.scrollTo("top", anchor: .top) }
                }
            }
        }
    }

    private func header(_ title: String, count: Int?) -> some View {
        HStack(spacing: 4) {
            Text(title.uppercased()).foregroundStyle(Color(ChatTheme.textPrimary))
            if let count { Text("(\(count))").opacity(0.5) }
            Spacer()
        }
        .font(.system(size: 10, weight: .bold))
        .tracking(1)
        .foregroundStyle(Color(ChatTheme.textSecondary))
        .padding(.vertical, 6)
        .padding(.horizontal, 4)
        .background(Color(ChatTheme.background))
        .overlay(alignment: .bottom) { Rectangle().fill(Color(ChatTheme.borderSubtle)).frame(height: 1) }
    }

    private func centered(_ text: String) -> some View {
        Text(text)
            .font(.sn(12))
            .foregroundStyle(Color(ChatTheme.textMuted))
            .frame(maxWidth: .infinity, minHeight: 128)
    }
}

/// One emote in the picker: the image at most 32 pt tall, a purple ring on modifiers and a
/// yellow one on zero-width emotes, dimmed with a lock when locked, dimmed when your personal
/// emote shows instead, and a yellow star when it is a favorite. A long press favorites it
/// and says what the emote is.
private struct EmoteCell: View {
    let cell: PickerCell
    let wide: Bool
    let pick: () -> Void
    let favorite: () -> Void

    var body: some View {
        Button(action: pick) {
            EmoteThumb(url: cell.url, height: 32, wide: wide)
                .padding(wide ? 4 : 6)
                .frame(maxWidth: .infinity, minHeight: 32)
                .background(ring.opacity(0.10), in: .rect(cornerRadius: 6))
                .overlay(RoundedRectangle(cornerRadius: 6).stroke(ring.opacity(0.5), lineWidth: ring == .clear ? 0 : 1))
                .shadow(color: ring.opacity(0.6), radius: ring == .clear ? 0 : 3)
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
                    .foregroundStyle(Color(ChatTheme.textPrimary))
                Text(Self.source(cell))
                    .font(.sn(12))
                    .foregroundStyle(Color(ChatTheme.textSecondary))
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
        .background(Color(ChatTheme.background))
    }

    /// "7TV · by someone", or the provider alone.
    static func source(_ cell: PickerCell) -> String {
        let provider = PickerTabs.title(cell.provider)
        guard let owner = cell.ownerName, !owner.isEmpty else { return provider }
        return "\(provider) · by \(owner)"
    }

    static func tint(_ line: String) -> Color {
        if line.hasPrefix("Modifier") { return .purple }
        if line == "Zero-Width" { return .yellow }
        return Color(ChatTheme.textMuted)
    }
}

/// An emote image at a fixed height and its own width (wide emotes stay wide), through the
/// shared image pipeline.
struct EmoteThumb: View {
    let url: String
    var height: CGFloat = 32
    var wide = false
    @Environment(\.displayScale) private var scale
    @State private var image: CGImage?

    var body: some View {
        Group {
            if let image {
                Image(decorative: image, scale: scale)
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: .fit)
            } else {
                Color.clear
            }
        }
        .frame(maxWidth: wide ? .infinity : height, maxHeight: height)
        .frame(height: height)
        .task(id: url) {
            let pixels = Int(height * scale)
            let tiered = ChatImageURL.thumbnail(url, points: height)
            if let hit = ImagePipeline.shared.cached(tiered, pixelHeight: pixels, animated: false) {
                image = hit.first
                return
            }
            image = nil
            let decoded: DecodedImage? = await withCheckedContinuation { continuation in
                ImagePipeline.shared.load(tiered, pixelHeight: pixels, animated: false) { continuation.resume(returning: $0) }
            }
            image = decoded?.first
        }
    }
}
