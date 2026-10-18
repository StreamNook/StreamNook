import SNKit
import SwiftUI

/// One settings section, pushed from its You-tab row over the tab bar.
struct SettingsPage: View {
    let model: AppModel
    let section: SettingsSection

    var body: some View {
        ScrollViewReader { reader in
            List {
                if model.settingsStore.loaded {
                    switch section {
                    case .player: PlayerPanel()
                    case .chat: ChatSettingsPanel()
                    case .adFree: AdFreePanel()
                    case .notifications: NotificationsPanel()
                    case .cache: CachePanel()
                    case .theme: ThemePanel()
                    case .support: SupportPanel()
                    case .changed: ChangedPanel()
                    case .backup: BackupPanel()
                    default: EmptyNote(text: "\(section.title) settings are built next.", hint: nil)
                    }
                } else {
                    ProgressView().frame(maxWidth: .infinity).padding(.vertical, 40)
                        .listRowBackground(Color.clear)
                }
                // "page-end" brings the bottom of a page without named sections into view.
                Color.clear.frame(height: 1)
                    .listRowBackground(Color.clear)
                    .listRowSeparator(.hidden)
                    .listRowInsets(EdgeInsets())
                    .id("page-end")
            }
            .snGroupedList()
            .scrollIndicators(.hidden)
            .onChange(of: SettingsAnchor.shared.target) { _, target in
                guard let target else { return }
                // "NAME@end" brings a section's last rows into view.
                let end = target.hasSuffix("@end")
                withAnimation { reader.scrollTo(end ? String(target.dropLast(4)) : target, anchor: end ? .bottom : .top) }
            }
        }
        .snShellBackground()
        .navigationTitle(section.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .environment(model.settingsStore)
        .environment(model)
        .task {
            let toasts = model.toasts
            model.settingsStore.onError = { text in toasts.show(text) }
            await model.settingsStore.refresh()
        }
    }
}

/// Where a drive step asks the open settings page to scroll (a section's title).
@Observable @MainActor
final class SettingsAnchor {
    static let shared = SettingsAnchor()
    var target: String?
}

/// A settings group: its name above, its rows in a panel.
struct SettingsGroup<Content: View>: View {
    let title: String
    var footer: String?
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).snSectionStyle().padding(.horizontal, 4)
            VStack(alignment: .leading, spacing: 0) { content }
                .snPanel()
            if let footer {
                Text(footer).font(.snRowDetail).foregroundStyle(SN.textMuted).padding(.horizontal, 4)
            }
        }
    }
}

/// A settings row: a short noun for the property, a plain line on what it does, the control.
struct SettingsRow<Control: View>: View {
    let title: String
    var detail: String?
    @ViewBuilder let control: Control

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.snRowTitle).foregroundStyle(SN.textPrimary)
                if let detail {
                    Text(detail).font(.snRowDetail).foregroundStyle(SN.textMuted).fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 8)
            control
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }
}
