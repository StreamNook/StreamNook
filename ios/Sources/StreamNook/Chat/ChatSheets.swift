import SNKit
import SwiftUI

/// The chat menu behind the composer's round button: the room's modes, then the actions for
/// this room in the phone's order (follow, subscribe, add a chat, reload, commands, moderator
/// tools, close). It stays open while following or unfollowing, so the row flips in place.
struct ChatMenuSheet: View {
    let model: ChatComposerModel
    let session: ChatSession
    @Environment(\.dismiss) private var dismiss
    /// Asked for only when the core holds nothing for this room (it was never watched).
    @State private var fetched: ChannelRelationship?
    @State private var followBusy = false

    /// The core reads it when the room opens and pushes every change, so the menu opens with it.
    private var relationship: ChannelRelationship? {
        session.rooms?.relationship(for: session.login) ?? fetched
    }

    /// The room's name: its tab's label (a Kick room's key is `kick:slug`), else its login.
    private var label: String {
        session.rooms?.tabs.first(where: { $0.channel == session.channel })?.label ?? session.login
    }

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: label)
            List {
                if let modes = session.modes, !modes.modes.isEmpty {
                    Section {
                        // Status, not actions: one quiet block with no card, its lines closer together
                        // than the rows below (a row per mode would keep the list's minimum row height).
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(modes.modes, id: \.kind) { mode in
                                HStack(spacing: 12) {
                                    Image(systemName: RoomModeSymbol.name(mode.kind))
                                        .font(.system(size: 13))
                                        .foregroundStyle(SN.textMuted)
                                        .frame(width: 20)
                                        .accessibilityHidden(true)
                                    Text(mode.compact)
                                        .font(.sn(13.5))
                                        .foregroundStyle(SN.textSecondary)
                                }
                                .accessibilityElement(children: .combine)
                                .accessibilityHint(mode.detail)
                            }
                        }
                        .padding(.vertical, 4)
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                        .listRowInsets(EdgeInsets(top: 0, leading: 20, bottom: 0, trailing: 20))
                    } header: {
                        ListHeader(text: "This chat is in")
                    } footer: {
                        if let standing = modes.standing {
                            ListFooter(text: standing)
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
                    if session.roomID != nil || session.isKick {
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
                                if relationship?.subscribed == true || kickSubscribed {
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
                    // Commands are Twitch's; a Kick room sends text only.
                    if !session.isKick {
                        Button {
                            dismiss()
                            model.commandsOpen = true
                        } label: {
                            Label("Chat commands", systemImage: "terminal")
                        }
                    }
                    // The stream manager, for the broadcaster and the channel's moderators: Rust
                    // offers each role only what Twitch lets it do.
                    if !session.isKick, session.roomID != nil, session.isModerator {
                        Button {
                            dismiss()
                            model.manageOpen = true
                        } label: {
                            HStack {
                                Label("Manage channel", systemImage: "slider.horizontal.3")
                                Spacer()
                                Image(systemName: "chevron.right")
                                    .font(.system(size: 13, weight: .semibold))
                                    .foregroundStyle(SN.textMuted)
                            }
                        }
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
                                    .foregroundStyle(SN.textSecondary)
                            }
                        }
                    }
                    // The stream's own chat closes with the player unless you keep it.
                    if let rooms = session.rooms, let tab = rooms.tabs.first(where: { $0.channel == session.channel }),
                       tab.pinnedToStream {
                        Button {
                            rooms.keep(tab.channel, !tab.kept)
                        } label: {
                            HStack {
                                Label {
                                    Text("Keep this chat")
                                } icon: {
                                    Image(systemName: tab.kept ? "bookmark.fill" : "bookmark")
                                        .foregroundStyle(tab.kept ? SN.accent : SN.textSecondary)
                                }
                                Spacer()
                                Text(tab.kept ? "On" : "Off")
                                    .font(.sn(12.5))
                                    .foregroundStyle(SN.textSecondary)
                            }
                        }
                    }
                    if let tab = session.rooms?.tabs.first(where: { $0.channel == session.channel }), !tab.pinnedToStream {
                        Button(role: .destructive) {
                            dismiss()
                            session.rooms?.close(tab.channel)
                        } label: {
                            Label("Close this chat", systemImage: "xmark")
                                .foregroundStyle(SN.error)
                        }
                    }
                } footer: {
                    if session.isModerator, session.moderatorTools {
                        ListFooter(text: "Long-press any message for delete, timeout and ban.")
                    }
                }
            }
            .snGroupedList()
        }
        .presentationDragIndicator(.visible)
        // Nothing to read here: the role comes from the room, the relationship from the core's
        // channel state. Only a room the core never watched asks.
        .task { await loadUnheld() }
    }

    /// Your Kick follow list says you subscribe here.
    private var kickSubscribed: Bool { session.isKick && session.rooms?.kick?.subscribed(to: session.login) == true }

    private var subscribeTitle: String {
        if session.isKick { return kickSubscribed ? "Gift Subs" : "Subscribe to \(label)" }
        guard let relationship else { return "Subscribe to \(label)" }
        if relationship.subscribed { return "Gift Subs" }
        return relationship.hasSubHistory ? "Resubscribe to \(label)" : "Subscribe to \(label)"
    }

    private func loadUnheld() async {
        guard !session.isKick, relationship == nil, let roomID = session.roomID else { return }
        fetched = try? await session.core.invoke("channel_relationship", args: [
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
            session.rooms?.noteFollowing(!following, for: session.login)
            fetched?.following = !following
        } catch {
            model.show("\(following ? "Unfollow" : "Follow") failed. Try logging out and back in via Settings to re-authenticate.")
        }
    }

    /// The platform's own checkout in the web sheet, as the signed-in account.
    private func subscribe() {
        let core = session.core
        let login = session.login
        let provider = session.provider
        let title = "Subscribe to \(label)"
        Task {
            _ = try? await core.invoke("open_subscribe_window", args: [
                "channelLogin": login, "title": title, "provider": provider,
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
                                            .foregroundStyle(SN.textPrimary)
                                        Text(row.description)
                                            .font(.sn(12.5))
                                            .foregroundStyle(SN.textSecondary)
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
            .tint(SN.textPrimary)
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
