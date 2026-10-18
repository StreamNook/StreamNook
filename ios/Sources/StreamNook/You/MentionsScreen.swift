import SNKit
import SwiftUI
import UIKit

/// Mentions: every message that mentioned you, replied to you or matched a highlight word, newest
/// first, from the core's inbox. Unread rows carry a dot and read rows sit a step dimmer. A tap
/// opens the chat at the message; swipe right to mark one read, left to remove it or mute its
/// chat; a long press copies it or opens its chat. Reached from You and from the chat menu.
struct MentionsScreen: View {
    let model: AppModel
    /// Shown in the chat menu's sheet: opening a row closes the sheet first.
    var close: (() -> Void)?

    private var store: MentionsStore { model.mentions }
    private var settings: SettingsStore { model.settingsStore }

    var body: some View {
        @Bindable var store = model.mentions
        let list = store.list
        content(list)
            .background(SN.background)
            .navigationTitle("Mentions")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar(.hidden, for: .tabBar)
            .toolbar {
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button("Mark all read") { Task { await store.markRead(nil) } }
                        .font(.sn(14, 500))
                        .disabled(list.unreadTotal == 0)
                    Menu {
                        Button("Clear all", systemImage: "trash", role: .destructive) { store.confirmingClear = true }
                            .disabled(list.entries.isEmpty)
                    } label: {
                        Image(systemName: "ellipsis")
                    }
                    .accessibilityLabel("More")
                }
                if let close {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Done", action: close).font(.sn(14, 500))
                    }
                }
            }
            .confirmationDialog("Clear all mentions?", isPresented: $store.confirmingClear, titleVisibility: .visible) {
                Button("Clear all", role: .destructive) { Task { await store.clearAll() } }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("Every mention is removed from this iPhone. Chats are not changed.")
            }
            .onAppear {
                store.appeared()
                if !settings.loaded { Task { await settings.refresh() } }
            }
            .onDisappear { store.disappeared() }
    }

    @ViewBuilder
    private func content(_ list: MentionList) -> some View {
        if !list.loaded && !store.failed {
            LoadingMark()
        } else if list.entries.isEmpty {
            ScrollView {
                EmptyNote(text: store.failed ? "Mentions did not load." : "No mentions yet.",
                          hint: store.failed ? "Pull down to try again."
                                             : "When someone mentions or replies to you in a chat, it shows up here.")
            }
            .refreshable { await store.refresh() }
        } else {
            rows(list)
        }
    }

    private func rows(_ list: MentionList) -> some View {
        let muted = Set(settings.strings("mentions.muted_channels"))
        let mixed = list.mixedPlatforms
        return List {
            ForEach(list.entries) { entry in
                Button { open(entry) } label: {
                    MentionRow(entry: entry, mixed: mixed)
                }
                .buttonStyle(RowPress())
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 12, bottom: 0, trailing: 12))
                .listRowSeparatorTint(SN.borderSubtle)
                // The hairline starts under the text, past the dot column.
                .alignmentGuide(.listRowSeparatorLeading) { _ in 20 }
                .swipeActions(edge: .leading, allowsFullSwipe: true) {
                    if !entry.read {
                        Button { Task { await store.markRead([entry.id]) } } label: {
                            Label("Mark read", systemImage: "envelope.open")
                        }
                        .tint(SN.accent)
                    }
                }
                .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                    Button(role: .destructive) { Task { await store.remove(entry.id) } } label: {
                        Label("Remove", systemImage: "trash")
                    }
                    muteButton(entry, muted: muted.contains(entry.room))
                        .tint(Color(white: 0.42))
                }
                .contextMenu {
                    Button("Open chat", systemImage: "bubble.left.and.text.bubble.right") { open(entry) }
                    Button("Copy message", systemImage: "doc.on.doc") { UIPasteboard.general.string = entry.text }
                        .disabled(entry.removed)
                    if !entry.read {
                        Button("Mark read", systemImage: "envelope.open") { Task { await store.markRead([entry.id]) } }
                    }
                    muteButton(entry, muted: muted.contains(entry.room))
                    Button("Remove", systemImage: "trash", role: .destructive) { Task { await store.remove(entry.id) } }
                }
                .onAppear {
                    if entry.id == list.entries.last?.id { Task { await store.loadMore() } }
                }
            }
            if store.loadingMore {
                ProgressView()
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 12)
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .refreshable { await store.refresh() }
    }

    private func muteButton(_ entry: MentionEntry, muted: Bool) -> some View {
        Button {
            Task {
                if muted {
                    await store.unmute(entry.room, store: settings)
                } else {
                    await store.mute(entry.room, store: settings)
                }
            }
        } label: {
            Label(muted ? "Unmute chat" : "Mute chat", systemImage: muted ? "bell" : "bell.slash")
        }
    }

    private func open(_ entry: MentionEntry) {
        close?()
        Task { await model.openMention(entry.id) }
    }
}

/// One mention: the unread dot, the sender in their color and how they reached you, the time,
/// the message, and the chat (with its platform's logo only where platforms mix).
struct MentionRow: View {
    let entry: MentionEntry
    var mixed = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Circle()
                .fill(SN.accent)
                .frame(width: 8, height: 8)
                .opacity(entry.read ? 0 : 1)
                // Centered on the first line's capitals, not its baseline.
                .alignmentGuide(.firstTextBaseline) { $0[.bottom] + 1.2 }
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 5) {
                    Text(entry.shownSender)
                        .font(.sn(14.5, 600))
                        .foregroundStyle(MentionColors.name(entry.senderColor))
                        .lineLimit(1)
                        .layoutPriority(1)
                    Text(entry.kindPhrase)
                        .font(.sn(13))
                        .foregroundStyle(SN.textSecondary)
                        .lineLimit(1)
                    Spacer(minLength: 8)
                    TimelineView(.everyMinute) { context in
                        Text(MentionText.time(entry.atMs, nowMs: Int64(context.date.timeIntervalSince1970 * 1000)))
                            .font(.sn(12))
                            .monospacedDigit()
                            .foregroundStyle(SN.textMuted)
                    }
                }
                Text(entry.shownText)
                    .font(.sn(14))
                    .italic(entry.removed)
                    .lineSpacing(2)
                    .foregroundStyle(entry.removed ? SN.textMuted : SN.textPrimary)
                    .lineLimit(4)
                    .multilineTextAlignment(.leading)
                    .frame(maxWidth: .infinity, alignment: .leading)
                HStack(spacing: 4) {
                    if mixed { ProviderLogo(provider: entry.platform, size: 11, tinted: true) }
                    Text(entry.channelLogin)
                        .font(.sn(12, 500))
                        .foregroundStyle(SN.textMuted)
                        .lineLimit(1)
                }
                .padding(.top, 1)
            }
            .opacity(entry.read ? 0.6 : 1)
        }
        .padding(.leading, 2)
        .padding(.vertical, 12)
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(entry.read ? "" : "Unread. ")\(entry.shownSender) \(entry.kindPhrase) in \(entry.channelLogin): \(entry.shownText)")
        .accessibilityHint("Opens the chat at this message")
    }
}

/// The Mentions screen in a sheet, opened from the chat menu over the watch layer.
struct MentionsSheet: View {
    let model: AppModel
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            MentionsScreen(model: model, close: { dismiss() })
        }
        .presentationDragIndicator(.visible)
        .presentationBackground {
            if !Look.shared.frosted { SN.background }
        }
    }
}

/// The You tab's way in: Mentions with its unread count.
struct MentionsEntryRow: View {
    let model: AppModel

    var body: some View {
        NavigationLink(value: Route.mentions) {
            HStack(spacing: 12) {
                Image(systemName: "at")
                    .font(.system(size: 18))
                    .foregroundStyle(SN.textSecondary)
                    .frame(width: 24)
                VStack(alignment: .leading, spacing: 2) {
                    Text("Mentions").font(.snRowTitle).foregroundStyle(SN.textPrimary)
                    Text("Messages that mention or reply to you").font(.snRowDetail).foregroundStyle(SN.textMuted).lineLimit(1)
                }
                Spacer(minLength: 8)
                UnreadCount(count: model.mentions.unreadTotal)
                Image(systemName: Symbol.disclosure)
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(SN.textMuted)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowPress())
        .accessibilityValue(model.mentions.unreadTotal > 0 ? "\(model.mentions.unreadTotal) unread" : "")
    }
}

/// An unread count in the accent's capsule; nothing at zero. Past 99 it reads "99+".
struct UnreadCount: View {
    let count: Int

    var body: some View {
        if count > 0 {
            Text(count > 99 ? "99+" : "\(count)")
                .font(.sn(12, 600))
                .monospacedDigit()
                .foregroundStyle(Look.shared.palette.accent.luminance > 0.5 ? Color.black.opacity(0.88) : .white)
                .padding(.horizontal, 7)
                .frame(minWidth: 22, minHeight: 22)
                .background(Capsule().fill(SN.accent))
                .accessibilityHidden(true)
        }
    }
}
