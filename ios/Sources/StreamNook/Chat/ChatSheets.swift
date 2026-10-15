import SNKit
import SwiftUI

/// The viewer's relationship to the room's channel, as the core's `channel_relationship`
/// answers it.
struct ChannelRelationship: Decodable, Equatable {
    /// Nil while unknown: the Follow row waits rather than guessing.
    var following: Bool?
    var subscribed: Bool
    var hasSubHistory: Bool
    var badgeURL: String?

    enum CodingKeys: String, CodingKey {
        case following, subscribed, hasSubHistory = "has_sub_history", badgeURL = "badge_url"
    }
}

/// The chat menu behind the composer's round button: the room's modes, then the actions for
/// this room in the phone's order (follow, subscribe, add a chat, reload, commands, moderator
/// tools, close). It stays open while following or unfollowing, so the row flips in place.
struct ChatMenuSheet: View {
    let model: ChatComposerModel
    let session: ChatSession
    @Environment(\.dismiss) private var dismiss
    @State private var relationship: ChannelRelationship?
    @State private var followBusy = false
    @State private var moderates = false

    private var label: String { session.channel }

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: label)
            List {
                if let modes = session.modes, !modes.modes.isEmpty {
                    Section {
                        ForEach(modes.modes, id: \.kind) { mode in
                            Label {
                                Text(mode.compact)
                            } icon: {
                                Image(systemName: RoomModeSymbol.name(mode.kind))
                                    .foregroundStyle(Color(ChatTheme.textSecondary))
                            }
                            .accessibilityHint(mode.detail)
                        }
                    } header: {
                        Text("This chat is in")
                            .foregroundStyle(Color(red: 0x8C / 255, green: 0xA0 / 255, blue: 0xA7 / 255))
                    } footer: {
                        if let standing = modes.standing {
                            Text(standing)
                                .font(.sn(12.5))
                                .foregroundStyle(SN.textSecondary)
                        }
                    }
                }
                Section {
                    if session.roomID != nil, let following = relationship?.following {
                        Button {
                            Task { await toggleFollow(following) }
                        } label: {
                            Label {
                                Text(following ? "Unfollow \(label)" : "Follow \(label)")
                            } icon: {
                                Image(systemName: following ? "heart.slash" : "heart")
                                    .foregroundStyle(following ? Color.red : Color.green)
                            }
                        }
                        .disabled(followBusy)
                    }
                    if session.roomID != nil {
                        Button {
                            dismiss()
                            subscribe()
                        } label: {
                            HStack {
                                Label {
                                    Text(subscribeTitle)
                                } icon: {
                                    if let badge = relationship?.badgeURL {
                                        EmoteThumb(url: badge, height: 19).accessibilityHidden(true)
                                    } else {
                                        Image(systemName: "star")
                                    }
                                }
                                Spacer()
                                if relationship?.subscribed == true {
                                    Text("Subscribed").font(.sn(12.5)).foregroundStyle(SN.textSecondary)
                                }
                            }
                        }
                    }
                    if let rooms = session.rooms {
                        Button {
                            dismiss()
                            rooms.addOpen = true
                        } label: {
                            Label("Add a chat", systemImage: "plus.bubble")
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
                    if moderates || session.isModerator {
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
                                    .foregroundStyle(SN.textSecondary)
                            }
                        }
                    }
                    if let tab = session.rooms?.tabs.first(where: { $0.login == session.channel }), !tab.pinnedToStream {
                        Button(role: .destructive) {
                            dismiss()
                            session.rooms?.close(tab.channel)
                        } label: {
                            Label("Close this chat", systemImage: "xmark")
                                .foregroundStyle(SN.error)
                        }
                    }
                } footer: {
                    if moderates || session.isModerator, session.moderatorTools {
                        Text("Long-press any message for delete, timeout and ban.")
                            .font(.sn(12.5))
                            .foregroundStyle(SN.textSecondary)
                    }
                }
            }
            .listSectionSpacing(16)
            .contentMargins(.top, 4, for: .scrollContent)
            .scrollContentBackground(.hidden)
            .tint(Color(ChatTheme.textPrimary))
        }
        .presentationDragIndicator(.visible)
        // Only while the sheet is open: the follow and subscription lookups and the role.
        .task { await load() }
    }

    private var subscribeTitle: String {
        guard let relationship else { return "Subscribe to \(label)" }
        if relationship.subscribed { return "Gift Subs" }
        return relationship.hasSubHistory ? "Resubscribe to \(label)" : "Subscribe to \(label)"
    }

    private func load() async {
        let core = session.core
        if let json = try? await core.invoke("chat_send_budget", args: ["channel": session.channel]),
           let budget = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any],
           let role = budget["role"] as? String {
            moderates = role == "moderator" || role == "broadcaster"
        }
        guard let roomID = session.roomID else { return }
        relationship = try? await core.invoke("channel_relationship", args: [
            "channelId": roomID, "channelLogin": session.channel,
        ], as: ChannelRelationship.self)
    }

    private func toggleFollow(_ following: Bool) async {
        guard let roomID = session.roomID else { return }
        followBusy = true
        defer { followBusy = false }
        do {
            // The desktop's heart-drop takes 600 ms before an unfollow.
            if following { try? await Task.sleep(for: .milliseconds(600)) }
            _ = try await session.core.invoke(following ? "unfollow_channel" : "follow_channel", args: ["targetUserId": roomID])
            relationship?.following = !following
        } catch {
            model.show("\(following ? "Unfollow" : "Follow") failed. Try logging out and back in via Settings to re-authenticate.")
        }
    }

    private func subscribe() {
        let core = session.core
        let login = session.channel
        Task {
            _ = try? await core.invoke("open_subscribe_window", args: [
                "channelLogin": login, "title": "Subscribe to \(login)", "provider": "twitch",
            ])
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

/// Every command the core lists for the room, searchable, one section per category; a tap
/// puts "/name " in the draft. Locked rows say what they need.
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

/// The symbol the composer and the chat menu show for a room mode.
enum RoomModeSymbol {
    static func name(_ kind: String) -> String {
        switch kind {
        case "subs_only": return "star"
        case "emote_only": return "face.smiling"
        case "followers_only": return "heart"
        case "slow": return "timer"
        default: return "touchid"
        }
    }
}
