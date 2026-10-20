import SNKit
import SwiftUI
import UIKit

/// You > Mentions: the inbox, the same panel the message bar's tray opens, here as a screen of its
/// own. A row opens the chat at its message; a long press mutes its chat or removes it.
struct MentionsScreen: View {
    let model: AppModel
    /// Shown in a sheet: opening a row closes the sheet first.
    var close: (() -> Void)?

    var body: some View {
        InboxPanel(store: model.mentions, variant: .screen, onOpenRow: close)
            .background(SN.background)
            .navigationTitle("Inbox")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar(.hidden, for: .tabBar)
            .toolbar {
                if let close {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Done", action: close).font(.sn(14, 500))
                    }
                }
            }
            .onAppear {
                model.mentions.appeared()
                if !model.settingsStore.loaded { Task { await model.settingsStore.refresh() } }
            }
            .onDisappear { model.mentions.disappeared() }
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
