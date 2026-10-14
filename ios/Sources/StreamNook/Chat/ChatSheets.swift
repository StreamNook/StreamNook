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
                    Button {
                        dismiss()
                        model.commandsOpen = true
                    } label: {
                        Label("Chat commands", systemImage: "terminal")
                    }
                    Button {
                        dismiss()
                        ChatReload.reload(session)
                    } label: {
                        Label("Reload this chat", systemImage: "arrow.clockwise")
                    }
                }
            }
            .tint(Color(ChatTheme.textPrimary))
            .navigationTitle(session.channel)
            .navigationBarTitleDisplayMode(.inline)
        }
    }
}

/// Drops and retakes the room's connection.
@MainActor
enum ChatReload {
    static func reload(_ session: ChatSession) {
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
