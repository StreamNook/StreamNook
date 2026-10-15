import SNKit
import SwiftUI

/// Picks another channel's chat to open in a tab. Live follows come first (that is where
/// moderating usually happens, and they need no network call); two characters or more search
/// every channel, offline ones included, since chat is joinable either way. The core answers the
/// search and its order (live rooms first, by audience).
struct AddChatSheet: View {
    let rooms: ChatRooms
    @Environment(\.dismiss) private var dismiss
    @State private var query = ""
    @State private var results: [ChatRoomRow] = []
    @State private var searching = false

    private var searchingNow: Bool { query.trimmingCharacters(in: .whitespaces).count >= 2 }

    private var list: [ChatRoomRow] {
        searchingNow ? results : rooms.followedLive().map(ChatRoomRow.init(followed:))
    }

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: "Add a chat")
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 16))
                    .foregroundStyle(SN.textMuted)
                    .accessibilityHidden(true)
                TextField("Search channels", text: $query)
                    .font(.sn(15))
                    .foregroundStyle(SN.textPrimary)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.search)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .snGlass(RoundedRectangle(cornerRadius: 12, style: .continuous))
            .padding(.horizontal, 16)
            .padding(.bottom, 8)

            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    if !searchingNow {
                        Text("LIVE NOW")
                            .font(.sn(12, 600))
                            .tracking(0.6)
                            .foregroundStyle(SN.textMuted)
                            .padding(.horizontal, 4)
                            .padding(.bottom, 6)
                            .accessibilityAddTraits(.isHeader)
                    }
                    if searching && list.isEmpty {
                        note("Searching…")
                    } else if list.isEmpty {
                        note(searchingNow ? "No channels found." : "Nobody you follow is live. Search above to open any channel.")
                    } else {
                        ForEach(list) { row in
                            let open = rooms.tabs.contains { $0.channel == row.key }
                            Button {
                                pick(row)
                            } label: {
                                RoomRow(row: row, open: open)
                            }
                            .buttonStyle(PressFade())
                            .disabled(open)
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 16)
            }
        }
        .presentationDetents([.fraction(0.7)])
        .presentationDragIndicator(.visible)
        .onAppear {
            if !rooms.addQuery.isEmpty {
                query = rooms.addQuery
                rooms.addQuery = ""
            }
        }
        .task(id: query) { await search() }
    }

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.sn(14))
            .foregroundStyle(SN.textMuted)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 24)
    }

    /// Two characters or more, 0.32 s after the last keystroke. The iPhone opens Twitch rooms only.
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
        results = found.filter { $0.provider == "twitch" }
        searching = false
    }

    private func pick(_ row: ChatRoomRow) {
        rooms.add(row)
        query = ""
        dismiss()
    }
}

/// One channel: its face with a live dot, the name, what it is doing, and its audience or "Open".
private struct RoomRow: View {
    let row: ChatRoomRow
    let open: Bool

    var body: some View {
        HStack(spacing: 12) {
            face
                .overlay(alignment: .bottomTrailing) {
                    if row.isLive {
                        Circle().fill(SN.live)
                            .frame(width: 12, height: 12)
                            .overlay { Circle().stroke(SN.background, lineWidth: 2) }
                            .offset(x: 2, y: 2)
                            .accessibilityHidden(true)
                    }
                }
            VStack(alignment: .leading, spacing: 1) {
                Text(row.userName.isEmpty ? row.userLogin : row.userName)
                    .font(.sn(15))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                Text(row.subline)
                    .font(.sn(12.5))
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
            if open {
                Text("Open")
                    .font(.sn(12))
                    .foregroundStyle(SN.textMuted)
            } else if row.isLive, row.viewerCount > 0 {
                HStack(spacing: 4) {
                    Image(systemName: "person.2.fill").font(.system(size: 13)).accessibilityHidden(true)
                    Text(row.viewerCount.formatted()).monospacedDigit()
                }
                .font(.sn(12))
                .foregroundStyle(SN.textMuted)
            }
        }
        .padding(.horizontal, 4)
        .padding(.vertical, 8)
        .contentShape(Rectangle())
        .opacity(open ? 0.45 : 1)
        .accessibilityElement(children: .combine)
    }

    private var face: some View {
        Group {
            if let url = row.profileImageURL.flatMap(URL.init(string:)) {
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
