import SNKit
import SwiftUI

/// "Add a chat": the chat picker in a sheet, from the Chats strip's + and the chat menu.
struct AddChatSheet: View {
    let rooms: ChatRooms
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: "Add a chat")
            ChatPicker(rooms: rooms, onPicked: { dismiss() })
        }
        .presentationDetents([.fraction(0.85)])
        .presentationDragIndicator(.visible)
        // Nothing is see-through at 0 percent glassiness.
        .presentationBackground {
            if !Look.shared.frosted { SN.backgroundTertiary }
        }
    }
}

/// The "add a chat" picker: a search for any channel, then your follows that are live, then your
/// follows that are offline. Tap a row to open its chat; a chat already open is marked "Open"
/// where it sits, and tapping it switches to it. The lists are the core's (`chat_picker`, built
/// from the Following snapshot, so opening this makes no request); searching is
/// `search_chat_rooms`. Drawn inline as the Chats tab when no chat is open, and in the sheet.
struct ChatPicker: View {
    let rooms: ChatRooms
    var onPicked: () -> Void = {}
    /// Room under the lists (the inline picker clears the tab bar on its own).
    var bottomPadding: CGFloat = 16
    @State private var query = ""
    @State private var results: [ChatRoomRow] = []
    @State private var searching = false
    @State private var lists = ChatPickerLists()

    private var searchingNow: Bool { query.trimmingCharacters(in: .whitespaces).count >= 2 }

    var body: some View {
        VStack(spacing: 0) {
            // The magnifier and the text sit on the rows' face and name columns.
            HStack(spacing: 12) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 16))
                    .foregroundStyle(SN.textMuted)
                    .frame(width: 36)
                    .accessibilityHidden(true)
                TextField("Search any channel", text: $query)
                    .font(.sn(15))
                    .foregroundStyle(SN.textPrimary)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.search)
            }
            .padding(.leading, 4)
            .padding(.trailing, 12)
            .padding(.vertical, 10)
            .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .keepsKeyboard()
            .padding(.horizontal, 16)
            .padding(.bottom, 19)

            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    if searchingNow {
                        if searching && results.isEmpty {
                            note("Searching…")
                        } else if results.isEmpty {
                            note("No channels found.")
                        } else {
                            header("Results", count: nil)
                            let mixed = Set(results.map(\.provider)).count > 1
                            let followed = Set((lists.live + lists.offline).map(\.key))
                            ForEach(results) { row in
                                item(row, mixed: mixed, following: followed.contains(row.key))
                            }
                        }
                    } else {
                        header("Live", count: lists.live.count)
                        if lists.live.isEmpty {
                            note("Nobody you follow is live.")
                        } else {
                            ForEach(lists.live) { row in item(row, mixed: lists.mixedPlatforms) }
                        }
                        if !lists.offline.isEmpty {
                            header("Offline", count: lists.offline.count)
                            ForEach(lists.offline) { row in item(row, mixed: lists.mixedPlatforms) }
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 16)
                .padding(.bottom, bottomPadding)
            }
            .scrollDismissesKeyboard(.never)
            // A tap on a result puts the keyboard away and opens it.
            .tapsThroughKeyboard()
            // Rows fade out at both edges instead of being cut.
            .mask {
                VStack(spacing: 0) {
                    LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom).frame(height: 16)
                    Color.black
                    LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: 16)
                }
            }
            .padding(.top, -16)
        }
        .onAppear {
            if !rooms.addQuery.isEmpty {
                query = rooms.addQuery
                rooms.addQuery = ""
            }
        }
        .task(id: query) { await search() }
        // The tabs moved (an open mark), or the Following snapshot did: read the lists again.
        .task(id: "\(rooms.state.revision)|\(rooms.followedLive().count)") { await load() }
    }

    private func header(_ text: String, count: Int?) -> some View {
        HStack {
            Text(text.uppercased())
                .font(.sn(12, 600))
                .tracking(0.6)
            Spacer()
            if let count {
                Text("\(count)").font(.sn(12, 500)).monospacedDigit()
            }
        }
        .foregroundStyle(SN.textMuted)
        .padding(.horizontal, 4)
        .padding(.top, 10)
        .padding(.bottom, 2)
        .accessibilityAddTraits(.isHeader)
    }

    private func item(_ row: ChatRoomRow, mixed: Bool, following: Bool = false) -> some View {
        let open = rooms.tabs.contains { $0.channel == row.key }
        return Button {
            pick(row, open: open)
        } label: {
            RoomRow(row: row, open: open, mixed: mixed, following: following,
                    group: row.isLive && row.provider == "twitch" && !row.userID.isEmpty ? rooms.groupFor(row.userID) : nil)
        }
        .buttonStyle(PressFade())
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.sn(14))
            .foregroundStyle(SN.textMuted)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 24)
    }

    private func load() async {
        guard let found = try? await rooms.core.invoke("chat_picker", args: ["platforms": ["twitch", "kick"]],
                                                       as: ChatPickerLists.self) else { return }
        lists = found
    }

    /// Two characters or more, 0.32 s after the last keystroke: Twitch and Kick together, or the
    /// one Kick room a pasted kick.com link names.
    private func search() async {
        let q = query.trimmingCharacters(in: .whitespaces)
        guard q.count >= 2 else {
            results = []
            searching = false
            return
        }
        searching = true
        try? await Task.sleep(for: .milliseconds(320))
        guard !Task.isCancelled else { return }
        let found = (try? await rooms.core.invoke("search_chat_rooms", args: ["query": q], as: [ChatRoomRow].self)) ?? []
        guard !Task.isCancelled else { return }
        results = found.filter { $0.provider == "twitch" || $0.provider == "kick" }
        searching = false
    }

    private func pick(_ row: ChatRoomRow, open: Bool) {
        if open { rooms.select(row.key) } else { rooms.add(row) }
        query = ""
        onPicked()
    }
}

/// One channel: its face with a live dot, the name, what it is doing (or when it was last live),
/// and a + to open its chat, or "Open" when it already is.
private struct RoomRow: View {
    let row: ChatRoomRow
    let open: Bool
    /// The list spans platforms, so a Kick room wears Kick's mark.
    var mixed = false
    /// A search result you follow says so.
    var following = false
    /// A live room streaming together or sharing its chat, as the cards say it.
    var group: ChannelGroup?

    private var detail: String {
        guard row.isLive else { return row.subline }
        var parts = [row.gameName.isEmpty ? "Live" : row.gameName]
        if row.viewerCount > 0 { parts.append(row.viewerCount.formatted(.number.notation(.compactName))) }
        if following { parts.append("following") }
        return parts.joined(separator: " · ")
    }

    var body: some View {
        HStack(spacing: 12) {
            face
                .opacity(row.isLive ? 1 : 0.6)
                .overlay(alignment: .bottomTrailing) {
                    if row.isLive {
                        Circle().fill(SN.live)
                            .frame(width: 10, height: 10)
                            .overlay { Circle().stroke(SN.background, lineWidth: 2) }
                            .offset(x: 1, y: 1)
                            .accessibilityHidden(true)
                    }
                }
            VStack(alignment: .leading, spacing: 1) {
                HStack(spacing: 5) {
                    Text(row.userName.isEmpty ? row.userLogin : row.userName)
                        .font(.sn(15, 500))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    if mixed && row.provider == "kick" { PlatformMark(platform: .kick, size: 12) }
                }
                // One line: the group's faces and word lead it when there is one.
                HStack(spacing: 4) {
                    if let group {
                        HStack(spacing: -4) {
                            ForEach(group.others.prefix(3), id: \.userID) { member in
                                AsyncImage(url: member.avatarURL.flatMap(URL.init(string:))) { image in
                                    image.resizable().scaledToFill()
                                } placeholder: {
                                    SN.surface
                                }
                                .frame(width: 14, height: 14)
                                .clipShape(Circle())
                            }
                        }
                        Text(group.together ? "Together ·" : "Shared chat ·")
                            .fixedSize()
                    }
                    Text(detail).lineLimit(1)
                }
                .font(.sn(12.5))
                .foregroundStyle(SN.textMuted)
            }
            Spacer(minLength: 0)
            if open {
                HStack(spacing: 4) {
                    Image(systemName: "checkmark").font(.system(size: 10, weight: .bold))
                    Text("Open").font(.sn(11.5, 600))
                }
                .foregroundStyle(SN.textPrimary)
                .padding(.horizontal, 10)
                .frame(height: 22)
                .background(Capsule().fill(SN.accent.opacity(0.2)))
            } else {
                Image(systemName: "plus")
                    .font(.system(size: 12, weight: .bold))
                    .foregroundStyle(SN.textSecondary)
                    .frame(width: 28, height: 28)
                    .snGlass(Circle())
            }
        }
        .padding(.horizontal, 4)
        .padding(.vertical, 6)
        .background {
            if open { RoundedRectangle(cornerRadius: 12, style: .continuous).fill(Color.white.opacity(0.04)).padding(.horizontal, -4) }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityHint(open ? "Shows this chat" : "Opens this chat")
    }

    private var face: some View {
        Group {
            if row.profileImageURL == nil, row.provider == "kick" {
                BrandTile(platform: .kick, size: 36)
            } else if let url = row.profileImageURL.flatMap(URL.init(string:)) {
                AsyncImage(url: url) { image in
                    image.resizable().scaledToFill()
                } placeholder: {
                    initial
                }
            } else {
                initial
            }
        }
        .frame(width: 36, height: 36)
        .clipShape(Circle())
    }

    private var initial: some View {
        Circle().fill(SN.surface)
            .overlay {
                Text((row.userName.isEmpty ? row.userLogin : row.userName).prefix(1).uppercased())
                    .font(.sn(13, 600))
                    .foregroundStyle(SN.textMuted)
            }
    }
}

/// A row pressed reads at 70 percent.
private struct PressFade: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.opacity(configuration.isPressed ? 0.7 : 1)
    }
}
