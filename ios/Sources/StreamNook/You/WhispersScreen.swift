import SNKit
import SwiftUI

/// Whispers: your Twitch private conversations, newest first, from the core's inbox. Unread
/// conversations carry their count; a tap opens the conversation in the chat menu's native sheet
/// (`AppModel.openWhisper`, which the chat inbox and a tapped whisper notification use too).
/// Reached from You for now (`AppModel.openWhispers`).
struct WhispersScreen: View {
    let model: AppModel

    private var store: WhispersStore { model.whispers }

    var body: some View {
        content
            .background(SN.background)
            .navigationTitle(WhisperText.title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar(.hidden, for: .tabBar)
            .task { await store.refresh() }
    }

    @ViewBuilder
    private var content: some View {
        if !store.loaded && !store.failed {
            LoadingMark()
        } else if store.list.conversations.isEmpty {
            ScrollView {
                EmptyNote(text: store.failed ? WhisperText.failed : WhisperText.empty,
                          hint: store.failed ? WhisperText.failedHint : WhisperText.emptyHint)
            }
            .refreshable { await store.refresh() }
        } else {
            List {
                ForEach(store.list.conversations) { row in
                    Button {
                        model.openWhisper(row.key)
                    } label: {
                        WhisperListRow(row: row)
                    }
                    .buttonStyle(RowPress())
                    .listRowBackground(Color.clear)
                    .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
                    .listRowSeparatorTint(SN.borderSubtle)
                    // The hairline starts under the name, past the picture.
                    .alignmentGuide(.listRowSeparatorLeading) { _ in 56 }
                }
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .refreshable { await store.refresh() }
        }
    }
}

/// One conversation in the list: the picture, the name, the time, the newest line, the unread
/// count.
struct WhisperListRow: View {
    let row: WhisperRow

    private var unread: Bool { row.unreadCount > 0 }

    var body: some View {
        HStack(spacing: 12) {
            WhisperPicture(url: row.profileImageURL, name: row.shownName, size: 44)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    // One weight, so a name never changes width when it is read.
                    Text(row.shownName)
                        .font(.sn(15, 600))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    TimelineView(.everyMinute) { context in
                        Text(MentionText.time(row.lastMessageTimestamp, nowMs: Int64(context.date.timeIntervalSince1970 * 1000)))
                            .font(.sn(12))
                            .monospacedDigit()
                            .foregroundStyle(SN.textMuted)
                    }
                }
                HStack(spacing: 8) {
                    Text(WhisperText.preview(row))
                        .font(.sn(13.5))
                        .foregroundStyle(SN.textSecondary)
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    UnreadCount(count: row.unreadCount)
                }
            }
            // Read conversations are dimmed rather than unread ones decorated, as in Mentions.
            .opacity(unread ? 1 : 0.6)
        }
        .padding(.vertical, 11)
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(unread ? "Unread. " : "")\(row.shownName): \(WhisperText.preview(row))")
        .accessibilityHint("Opens the conversation")
    }
}

/// A round profile picture, or the first letter on the theme's surface while there is none.
struct WhisperPicture: View {
    let url: String?
    let name: String
    var size: CGFloat = 44

    var body: some View {
        if url != nil {
            Avatar(url: url, size: size)
        } else {
            Circle().fill(SN.surface)
                .frame(width: size, height: size)
                .overlay {
                    Text(name.prefix(1).uppercased())
                        .font(.sn(size * 0.4, 600))
                        .foregroundStyle(SN.textSecondary)
                }
                .accessibilityHidden(true)
        }
    }
}

/// The conversation the whisper sheet shows.
struct WhisperSheetKey: Identifiable, Hashable {
    let key: String
    var id: String { key }
}

/// A conversation in the native sheet the chat menu uses: the system grabber, medium and large,
/// and a slide down or a tap above it closes it.
struct WhisperSheet: View {
    let model: AppModel
    let key: String

    var body: some View {
        NavigationStack {
            WhisperConversationScreen(model: model, key: key)
        }
        .snSheetLight()
        .presentationDetents([.medium, .large])
        // At medium a drag scrolls the thread rather than growing the sheet.
        .presentationContentInteraction(.scrolls)
        .presentationDragIndicator(.visible)
    }
}

/// One conversation: the thread, oldest at the top, and the composer. Open, it counts as read
/// (the core neither counts nor notifies a whisper landing in it while the app is in front).
struct WhisperConversationScreen: View {
    let model: AppModel
    let key: String

    @State private var stored: WhisperThread?
    @State private var loaded = false
    @State private var draft = ""
    @State private var shown = Self.page
    @FocusState private var focused: Bool

    /// Messages drawn at first in a long thread; "Show earlier messages" adds this many.
    static let page = 200

    private var store: WhispersStore { model.whispers }

    /// The newest change shows at once; the full re-read follows it.
    private var thread: WhisperThread? {
        guard let stored, let update = store.lastUpdate else { return stored }
        return update.applied(to: stored, key: key)
    }

    /// Who this is with: the list's row, else the thread's own record.
    private var row: WhisperRow? {
        if let row = store.row(key) { return row }
        guard let thread else { return nil }
        let last = thread.messages.last
        return WhisperRow(key: key, userID: thread.userID, userLogin: thread.userLogin, userName: thread.userName,
                          profileImageURL: thread.profileImageURL, lastMessageTimestamp: thread.lastMessageTimestamp,
                          unreadCount: thread.unreadCount, lastText: last?.message ?? "", lastIsSent: last?.isSent ?? false)
    }

    var body: some View {
        let messages = thread?.messages ?? []
        let hidden = max(0, messages.count - shown)
        let pending = store.pending.filter { $0.key == key }
        TimelineView(.everyMinute) { context in
            let nowMs = Int64(context.date.timeIntervalSince1970 * 1000)
            let lines = WhisperThreadItem.lines(Array(messages.dropFirst(hidden)), pending: pending, nowMs: nowMs)
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        if hidden > 0 {
                            Button("Show earlier messages") { shown += Self.page }
                                .font(.sn(13))
                                .foregroundStyle(SN.textMuted)
                                .padding(.vertical, 10)
                        }
                        if loaded && messages.isEmpty && pending.isEmpty {
                            EmptyNote(text: WhisperText.threadEmpty, hint: nil)
                        }
                        ForEach(lines) { line in
                            lineView(line).id(line.id)
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .padding(.horizontal, 12)
                    .padding(.top, 6)
                }
                .scrollDismissesKeyboard(.interactively)
                .defaultScrollAnchor(.bottom)
                // Messages fade out under the bar instead of being cut by it.
                .scrollEdgeEffectStyle(.soft, for: .top)
                .onChange(of: lines.count) {
                    withAnimation(.easeOut(duration: 0.2)) { proxy.scrollTo("bottom", anchor: .bottom) }
                }
            }
        }
        .safeAreaInset(edge: .bottom) { composer }
        .background(SN.background)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .toolbar {
            ToolbarItem(placement: .principal) { header }
        }
        .onAppear { store.setOpen(key) }
        .onDisappear { store.setOpen(nil) }
        .task(id: store.revision) {
            do {
                stored = try await store.thread(key)
            } catch {
                log.error("whisper_thread failed: \(String(describing: error), privacy: .public)")
            }
            loaded = true
        }
    }

    private var header: some View {
        let name = row?.shownName ?? ""
        let login = row?.userLogin ?? ""
        return HStack(spacing: 8) {
            WhisperPicture(url: row?.profileImageURL, name: name, size: 28)
            VStack(alignment: .leading, spacing: 0) {
                Text(name).font(.sn(15, 700)).foregroundStyle(SN.textPrimary).lineLimit(1)
                if !login.isEmpty, login.lowercased() != name.lowercased() {
                    Text("@\(login)").font(.sn(11.5)).foregroundStyle(SN.textMuted).lineLimit(1)
                }
            }
        }
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private func lineView(_ line: WhisperThreadItem) -> some View {
        switch line {
        case .day(_, let label):
            Text(label)
                .font(.sn(12, 500))
                .foregroundStyle(SN.textMuted)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 12)
        case .message(let message, let last, let time):
            VStack(alignment: message.isSent ? .trailing : .leading, spacing: 4) {
                WhisperBubble(text: message.message, mine: message.isSent)
                if let time {
                    Text(time)
                        .font(.sn(11.5))
                        .monospacedDigit()
                        .foregroundStyle(SN.textMuted)
                        .padding(.horizontal, 4)
                }
            }
            .frame(maxWidth: .infinity, alignment: message.isSent ? .trailing : .leading)
            .padding(.bottom, last ? 8 : 3)
        case .pending(let pending):
            pendingView(pending)
                .padding(.top, 3)
                .padding(.bottom, 8)
        }
    }

    private func pendingView(_ pending: PendingWhisper) -> some View {
        VStack(alignment: .trailing, spacing: 4) {
            switch pending.status {
            case .sending:
                WhisperBubble(text: pending.text, mine: true, sending: true)
                Text(WhisperText.sending).font(.sn(11.5)).foregroundStyle(SN.textMuted).padding(.horizontal, 4)
            case .failed(let reason):
                WhisperBubble(text: pending.text, mine: true, failed: true)
                // Under its bubble, right edges on the time line's: why, then what to do.
                VStack(alignment: .trailing, spacing: 2) {
                    // The mark rides the first line, right beside the words it marks.
                    Text("\(Text(Image(systemName: Symbol.error)).foregroundStyle(SN.error)) \(reason)")
                        .font(.sn(12.5))
                        .foregroundStyle(SN.textSecondary)
                        .multilineTextAlignment(.trailing)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: 12) {
                        if let row {
                            Button(WhisperText.retry) { store.retry(pending.id, to: row) }
                                .font(.sn(12.5, 600))
                                .foregroundStyle(SN.accentOnSurface)
                        }
                        Button(WhisperText.discard) { store.discard(pending.id) }
                            .font(.sn(12.5))
                            .foregroundStyle(SN.textMuted)
                    }
                }
                .padding(.leading, 64)
                .padding(.trailing, 4)
                .accessibilityElement(children: .contain)
            }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
    }

    private var canSend: Bool { row != nil && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 6) {
            TextField("", text: $draft, prompt: Text(row.map { WhisperText.placeholder($0.shownName) } ?? "")
                .foregroundStyle(SN.textMuted))
                .font(.sn(15))
                .foregroundStyle(SN.textPrimary)
                .padding(.vertical, 10)
                .padding(.horizontal, 12)
                .focused($focused)
                .submitLabel(.send)
                .onSubmit(send)
                .frame(minHeight: 40)
                .glassEffect(.regular, in: .rect(cornerRadius: 12))
                .disabled(row == nil)
            Button(action: send) {
                Image(systemName: "paperplane.fill")
                    .font(.system(size: 18, weight: .bold))
                    .foregroundStyle(canSend ? Color.white : SN.textMuted)
                    .frame(width: 44, height: 44)
            }
            .glassEffect(.regular.interactive(), in: .circle)
            .disabled(!canSend)
            .accessibilityLabel("Send")
        }
        .padding(.horizontal, 10)
        .padding(.top, 8)
        .padding(.bottom, 8)
    }

    private func send() {
        guard let row, canSend else { return }
        store.send(draft, to: row)
        draft = ""
    }
}

/// One message: yours on the right in the accent's tint (strong enough in dark to part from the
/// surface on a grey accent, lighter on light themes so the text keeps its contrast), theirs on
/// the left on the surface. One still on its way takes a fainter fill with its text at full
/// strength; the "Sending" line carries the state.
struct WhisperBubble: View {
    let text: String
    let mine: Bool
    var sending = false
    var failed = false

    private var mineFill: Color {
        let light = Look.shared.palette.background.luminance > 0.5
        return SN.accent.opacity(sending ? (light ? 0.1 : 0.2) : (light ? 0.2 : 0.35))
    }

    var body: some View {
        // The far side keeps a margin, so a long message never runs edge to edge.
        HStack(spacing: 0) {
            if mine { Spacer(minLength: 64) }
            Text(text)
                .font(.sn(15))
                .foregroundStyle(SN.textPrimary)
                .multilineTextAlignment(.leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 7)
                .background(mine ? mineFill : SN.surface, in: .rect(cornerRadius: 18, style: .continuous))
                .overlay {
                    if failed {
                        RoundedRectangle(cornerRadius: 18, style: .continuous).strokeBorder(SN.error.opacity(0.6), lineWidth: 1)
                    }
                }
                .textSelection(.enabled)
            if !mine { Spacer(minLength: 64) }
        }
    }
}

/// The You tab's way in: Whispers with its unread count. Here until the chat inbox opens
/// them.
struct WhispersEntryRow: View {
    let model: AppModel

    var body: some View {
        NavigationLink(value: Route.whispers) {
            HStack(spacing: 12) {
                Image(systemName: "bubble.left")
                    .font(.system(size: 18))
                    .foregroundStyle(SN.textSecondary)
                    .frame(width: 24)
                VStack(alignment: .leading, spacing: 2) {
                    Text(WhisperText.title).font(.snRowTitle).foregroundStyle(SN.textPrimary)
                    Text(WhisperText.entryDetail).font(.snRowDetail).foregroundStyle(SN.textMuted).lineLimit(1)
                }
                Spacer(minLength: 8)
                UnreadCount(count: model.whispers.unreadTotal)
                Image(systemName: Symbol.disclosure)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(SN.textMuted)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowPress())
        .accessibilityValue(model.whispers.unreadTotal > 0 ? "\(model.whispers.unreadTotal) unread" : "")
    }
}
