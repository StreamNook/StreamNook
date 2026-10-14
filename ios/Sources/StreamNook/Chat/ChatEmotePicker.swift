import SNKit
import SwiftUI

/// The emote picker over the composer: provider tabs, a search field and the room's emotes in
/// the sections the core groups them into. Picking an emote puts it in the draft and keeps the
/// picker open.
struct ChatEmotePicker: View {
    let model: ChatComposerModel
    @State private var tab = "twitch"
    @State private var query = ""
    @State private var rows: [EmoteMatch] = []
    @State private var loading = true

    private static let tabs: [(id: String, title: String, color: Color)] = [
        ("twitch", "Twitch", Color(red: 0.57, green: 0.28, blue: 1)),
        ("bttv", "BetterTTV", Color(red: 0.84, green: 0, blue: 0.08)),
        ("7tv", "7TV", Color(red: 0.16, green: 0.71, blue: 0.96)),
        ("ffz", "FrankerFaceZ", .white),
    ]

    var body: some View {
        VStack(spacing: 6) {
            TextField("", text: $query, prompt: Text("Search emotes...").foregroundStyle(Color(ChatTheme.textMuted)))
                .font(.sn(13))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .padding(.horizontal, 12)
                .padding(.vertical, 7)
                .background(Color(ChatTheme.surface), in: .rect(cornerRadius: 10))
            HStack(spacing: 4) {
                ForEach(Self.tabs, id: \.id) { item in
                    let count = rows.filter { $0.provider == item.id }.count
                    Button {
                        tab = item.id
                    } label: {
                        VStack(spacing: 1) {
                            Text(item.title).font(.system(size: 12, weight: tab == item.id ? .heavy : .medium)).lineLimit(1)
                            Text("\(count)").font(.sn(10)).opacity(0.7)
                        }
                        .foregroundStyle(tab == item.id ? item.color : Color(ChatTheme.textSecondary))
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 6)
                        .background(tab == item.id ? Color.white.opacity(0.08) : .clear, in: .rect(cornerRadius: 8))
                    }
                    .buttonStyle(.plain)
                }
            }
            grid
        }
        .padding(8)
        .frame(height: min(420, screenHeight * 0.45))
        .background(Color(ChatTheme.background), in: .rect(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color(ChatTheme.borderSubtle)))
        .shadow(color: .black.opacity(0.5), radius: 20, y: 8)
        .task(id: query) { await load() }
    }

    private var screenHeight: CGFloat {
        (UIApplication.shared.connectedScenes.first as? UIWindowScene)?.screen.bounds.height ?? 844
    }

    private var sections: [(String, [EmoteMatch])] {
        var order: [String] = []
        var groups: [String: [EmoteMatch]] = [:]
        for row in rows where row.provider == tab {
            let title = row.group ?? row.sourceLabel ?? "Emotes"
            if groups[title] == nil { order.append(title) }
            groups[title, default: []].append(row)
        }
        return order.map { ($0, groups[$0] ?? []) }
    }

    @ViewBuilder private var grid: some View {
        let sections = sections
        if loading && rows.isEmpty {
            centered("Loading emotes...")
        } else if sections.isEmpty {
            centered("No emotes found")
        } else {
            ScrollViewReader { reader in
                ScrollView {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 7), spacing: 8,
                              pinnedViews: [.sectionHeaders]) {
                        ForEach(sections, id: \.0) { title, emotes in
                            Section {
                                ForEach(emotes) { emote in
                                    Button {
                                        model.insert(emote.text)
                                    } label: {
                                        ChatImageView(url: ChatImageURL.thumbnail(emote.url), size: 32)
                                            .padding(6)
                                            .contentShape(.rect)
                                    }
                                    .buttonStyle(.plain)
                                    .accessibilityLabel(emote.name)
                                }
                            } header: {
                                HStack(spacing: 4) {
                                    Text(title.uppercased()).foregroundStyle(Color(ChatTheme.textPrimary))
                                    Text("(\(emotes.count))").opacity(0.5)
                                    Spacer()
                                }
                                .font(.system(size: 10, weight: .bold))
                                .tracking(1)
                                .foregroundStyle(Color(ChatTheme.textSecondary))
                                .padding(.vertical, 6)
                                .padding(.horizontal, 4)
                                .background(Color(ChatTheme.background))
                            }
                        }
                    }
                    .padding(.horizontal, 4)
                    .id("top")
                }
                .onChange(of: tab) { _, _ in reader.scrollTo("top", anchor: .top) }
            }
        }
    }

    private func centered(_ text: String) -> some View {
        Text(text)
            .font(.sn(12))
            .foregroundStyle(Color(ChatTheme.textMuted))
            .frame(maxWidth: .infinity, minHeight: 128)
    }

    /// The room's emotes from the core: browsing groups them, a search ranks them.
    private func load() async {
        if !query.isEmpty { try? await Task.sleep(for: .milliseconds(150)) }
        guard !Task.isCancelled else { return }
        let session = model.session
        let args: [String: Any?] = [
            "provider": "twitch", "channel": session.channel, "channelId": session.roomID, "query": query,
            "profile": "search", "order": "default", "tier": "3x", "limit": 5000,
        ]
        loading = true
        defer { loading = false }
        if let json = try? await session.core.invoke("match_emote_tokens", args: args),
           let result = try? JSONDecoder().decode(EmoteMatchResult.self, from: Data(json.utf8)) {
            rows = result.rows
            if !result.ready, query.isEmpty {
                // The room's emotes are still loading in the core; ask it to fetch them.
                _ = try? await session.core.invoke("fetch_channel_emotes", args: [
                    "channelName": session.channel, "channelId": session.roomID, "provider": "twitch", "tier": "3x",
                ])
                if let again = try? await session.core.invoke("match_emote_tokens", args: args),
                   let retry = try? JSONDecoder().decode(EmoteMatchResult.self, from: Data(again.utf8)) {
                    rows = retry.rows
                }
            }
            if !rows.contains(where: { $0.provider == tab }), let first = rows.first?.provider {
                tab = first
            }
        }
    }
}
