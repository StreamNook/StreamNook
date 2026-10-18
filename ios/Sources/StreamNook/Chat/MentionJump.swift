import SNKit
import SwiftUI

// Opening a mention (a Mentions row or its notification, the same route on every app): the core
// marks it read and hands it back, its chat opens, and then the message is brought into view.
// In the rows: scrolled to the middle and flashed. Not there: one history fetch around it, then
// again. Still not there: the entry pinned over the chat as "Older than chat history".

extension AppModel {
    func openMention(_ id: String) async {
        guard let entry = await mentions.open(id) else {
            toasts.show("That mention is no longer in Mentions.")
            return
        }
        guard openChat(for: entry) else { return }
        chat.focus(entry)
    }

    /// Puts a mention's chat on screen the way the app opens any chat: its tab when it is open,
    /// a tab beside the stream that is playing, or the channel itself when nothing plays.
    @discardableResult
    func openChat(for entry: MentionEntry) -> Bool {
        guard entry.platform == "twitch" || entry.platform == "kick" else {
            let name = entry.platform == "youtube" ? "YouTube" : entry.platform.capitalized
            toasts.show("\(name) chats do not open on iPhone yet.")
            return false
        }
        // `room` is the lowercased match key; `chatKey` (the core's `open_key`) opens the chat in
        // the channel's own case.
        let key = entry.chatKey
        let channel = MentionText.channel(ofRoom: key)
        if let tab = chat.tabs.first(where: { $0.channel.lowercased() == entry.room }) {
            chat.select(tab.channel)
        } else if nav.playing != nil {
            chat.add(ChatRoomRow(provider: entry.platform, key: key, userLogin: channel,
                                 userID: entry.channelID ?? "", userName: entry.channelLogin, isLive: false))
        } else {
            let kick = entry.platform == "kick"
            let seed = TwitchStream(id: "", userLogin: channel, userName: entry.channelLogin, title: "",
                                    viewerCount: 0, gameName: "", userID: entry.channelID,
                                    provider: kick ? "kick" : nil, key: kick ? key : nil)
            startWatching(kick ? seed.asKick : seed)
        }
        if nav.playerMode != .full {
            withAnimation(SNMotion.run(SNMotion.settleBack)) { nav.playerMode = .full }
        }
        return true
    }
}

extension ChatRooms {
    /// Brings a mention into view once its room is on screen. A newer focus replaces this one.
    func focus(_ entry: MentionEntry) {
        mentionFocus?.cancel()
        mentionCards[entry.room] = nil
        mentionFocus = Task { [weak self] in await self?.bringIntoView(entry) }
    }

    func dismissMentionCard(_ room: String) {
        withAnimation(.snappy) { mentionCards[room] = nil }
    }

    private func bringIntoView(_ entry: MentionEntry) async {
        // The core's tab change, then the room's join: a few seconds at most.
        guard let session = await waitFor(seconds: 10, { [self] in
            // Sessions name their room in lowercase, whatever case the tab key has.
            guard let active = state.activeChannel, let session = sessions[active], session.channel == entry.room else { return nil }
            return session
        }) else { return }
        // The join's history is where an earlier mention usually is.
        _ = await waitFor(seconds: 10, { session.historyLoaded ? true : nil })
        if await jump(to: entry.id, in: session) { return }
        guard !Task.isCancelled else { return }
        await session.backfill(around: entry.atMs)
        if await jump(to: entry.id, in: session) { return }
        guard !Task.isCancelled, sessions.values.contains(where: { $0 === session }) else { return }
        withAnimation(.snappy) { mentionCards[entry.room] = entry }
    }

    /// Scrolls to and flashes a row the room holds; false when the room does not hold it.
    private func jump(to id: String, in session: ChatSession) async -> Bool {
        guard session.pipeline.message(id) != nil else { return false }
        // Held rows reach the list on its next frames.
        for _ in 0..<20 {
            guard !Task.isCancelled else { return false }
            if let list = ChatRegistry.shared.list(for: session), list.jump(to: id) { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        return false
    }

    private func waitFor<T>(seconds: Double, _ value: () -> T?) async -> T? {
        let deadline = ContinuousClock.now + .seconds(seconds)
        while ContinuousClock.now < deadline, !Task.isCancelled {
            if let found = value() { return found }
            try? await Task.sleep(for: .milliseconds(150))
        }
        return value()
    }
}

/// A mention the chat's rows no longer hold, pinned over the top of the chat until dismissed:
/// who said it and how, when, and the message.
struct MentionContextCard: View {
    let entry: MentionEntry
    let dismiss: () -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                Image(systemName: "clock.arrow.circlepath")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(SN.textMuted)
                    .accessibilityHidden(true)
                Text("Older than chat history")
                    .font(.sn(11.5, 600))
                    .foregroundStyle(SN.textMuted)
                Spacer(minLength: 8)
                Button(action: dismiss) {
                    Text("Dismiss")
                        .font(.sn(12, 500))
                        .foregroundStyle(SN.textSecondary)
                        .frame(minWidth: 44, minHeight: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                // A full touch target that leaves the header line its own height.
                .padding(.vertical, -14)
                .padding(.trailing, -6)
            }
            HStack(alignment: .firstTextBaseline, spacing: 5) {
                Text(entry.shownSender)
                    .font(.sn(13, 600))
                    .foregroundStyle(MentionColors.name(entry.senderColor))
                    .lineLimit(1)
                    .layoutPriority(1)
                Text(entry.kindPhrase)
                    .font(.sn(12.5))
                    .foregroundStyle(SN.textSecondary)
                    .lineLimit(1)
                Spacer(minLength: 6)
                Text(MentionText.time(entry.atMs, nowMs: Int64(Date().timeIntervalSince1970 * 1000)))
                    .font(.sn(11.5))
                    .monospacedDigit()
                    .foregroundStyle(SN.textMuted)
            }
            Text(entry.shownText)
                .font(.sn(13))
                .italic(entry.removed)
                .lineSpacing(2)
                .foregroundStyle(entry.removed ? SN.textMuted : SN.textPrimary)
                .lineLimit(4)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .padding(.bottom, 10)
        .background {
            shape.fill(SN.backgroundTertiary.opacity(0.94 + (1 - Look.shared.glassiness) * 0.06))
                .overlay { shape.stroke(SN.borderSubtle, lineWidth: 1) }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAction(named: "Dismiss", dismiss)
    }
}

/// A sender's name color as the inbox keeps it, or Twitch's default purple.
@MainActor
enum MentionColors {
    static func name(_ hex: String?) -> Color {
        Color(uiColor: ChatTheme.color(hex) ?? ChatTheme.fallbackName)
    }
}
