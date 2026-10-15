import SNKit
import SwiftUI

/// The chat menu behind the composer's round button: the room's modes, then the actions for
/// this room.
struct ChatMenuSheet: View {
    let model: ChatComposerModel
    let session: ChatSession
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                if let modes = session.modes, !modes.modes.isEmpty {
                    Section {
                        HStack(spacing: 12) {
                            Image(systemName: "lock.fill")
                                .font(.system(size: 17))
                                .foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: 2) {
                                Text("This chat is in")
                                    .font(.sn(13.5))
                                    .foregroundStyle(.secondary)
                                Text(modes.modes.map(\.label).joined(separator: " · "))
                                    .font(.sn(14))
                            }
                        }
                        .accessibilityElement(children: .combine)
                    } footer: {
                        if let standing = modes.standing { Text(standing) }
                    }
                }
                Section {
                    if let rooms = session.rooms {
                        Button {
                            dismiss()
                            rooms.addOpen = true
                        } label: {
                            Label("Add a chat", systemImage: "bubble.left.and.bubble.right")
                        }
                    }
                    Button {
                        dismiss()
                        ChatReload.reload(session)
                    } label: {
                        Label("Reload this chat", systemImage: "arrow.clockwise")
                    }
                    Button {
                        dismiss()
                        model.commandsOpen = true
                    } label: {
                        Label("Chat commands", systemImage: "terminal")
                    }
                    // Only where the actions would succeed: Helix refuses them without mod powers here.
                    if session.isModerator {
                        Button {
                            session.setModeratorTools(!session.moderatorTools)
                        } label: {
                            HStack {
                                Label {
                                    Text("Moderator tools")
                                } icon: {
                                    Image(systemName: session.moderatorTools ? "checkmark.shield.fill" : "checkmark.shield")
                                        .foregroundStyle(session.moderatorTools ? SN.accent : SN.textSecondary)
                                }
                                Spacer()
                                Text(session.moderatorTools ? "On" : "Off")
                                    .font(.sn(12.5))
                                    .foregroundStyle(SN.textMuted)
                            }
                        }
                    }
                    if let tab = session.rooms?.tabs.first(where: { $0.login == session.channel }), !tab.pinnedToStream {
                        Button(role: .destructive) {
                            dismiss()
                            session.rooms?.close(tab.channel)
                        } label: {
                            Label("Close this chat", systemImage: "xmark")
                        }
                    }
                } footer: {
                    if session.isModerator, session.moderatorTools {
                        Text("Long-press any message for delete, timeout and ban.")
                    }
                }
            }
            .tint(Color(ChatTheme.textPrimary))
            .navigationTitle(session.channel)
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

/// "Reload this chat": an open room rebuilds the whole Twitch connection with every room (a
/// reload is reached for because messages stopped, and a re-join has no connection to ride);
/// the chat stage drops and retakes its own.
@MainActor
enum ChatReload {
    static func reload(_ session: ChatSession) {
        if let rooms = session.rooms {
            rooms.reload(session)
            return
        }
        session.stop()
        session.restart()
    }
}

/// Every command the core lists for the room, searchable; a tap puts "/name " in the draft.
struct ChatCommandSheet: View {
    let session: ChatSession
    let pick: (String) -> Void
    @State private var query = ""
    @State private var rows: [CommandRow] = []

    var body: some View {
        NavigationStack {
            List {
                if rows.isEmpty {
                    Text("Nothing matches. Try a word from what you want to do, like ban, slow or reminder.")
                        .font(.sn(13))
                        .foregroundStyle(.secondary)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 32)
                        .listRowBackground(Color.clear)
                }
                ForEach(sections, id: \.0) { label, commands in
                    Section(label) {
                        ForEach(commands) { row in
                            Button {
                                pick(row.name)
                            } label: {
                                HStack(alignment: .top) {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text("/\(row.name)").font(.sn(14, 500))
                                            .foregroundStyle(Color(ChatTheme.textPrimary))
                                        Text(row.description)
                                            .font(.sn(12.5))
                                            .foregroundStyle(Color(ChatTheme.textSecondary))
                                        if let lock = row.lock {
                                            Text(lock).font(.sn(11.5)).foregroundStyle(.tertiary)
                                        }
                                    }
                                    Spacer(minLength: 0)
                                    if row.lock != nil {
                                        Image(systemName: "lock.fill").font(.system(size: 14)).foregroundStyle(.tertiary)
                                    }
                                }
                                .foregroundStyle(.primary)
                            }
                            .disabled(row.lock != nil)
                            .opacity(row.lock != nil ? 0.5 : 1)
                        }
                    }
                }
            }
            .tint(Color(ChatTheme.textPrimary))
            .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search commands")
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .navigationTitle("Chat commands")
            .navigationBarTitleDisplayMode(.inline)
            .task(id: query) { await load() }
        }
    }

    /// One section per category, in the order the table first names it.
    private var sections: [(String, [CommandRow])] {
        var order: [String] = []
        var groups: [String: [CommandRow]] = [:]
        for row in rows {
            if groups[row.label] == nil { order.append(row.label) }
            groups[row.label, default: []].append(row)
        }
        return order.map { ($0, groups[$0] ?? []) }
    }

    private func load() async {
        if !query.isEmpty { try? await Task.sleep(for: .milliseconds(120)) }
        guard !Task.isCancelled else { return }
        guard let json = try? await session.core.invoke("command_catalog", args: [
            "channel": session.channel, "platform": "phone", "query": query,
        ]), let list = try? JSONDecoder().decode([CommandRow].self, from: Data(json.utf8)) else {
            rows = []
            return
        }
        rows = list
    }
}
