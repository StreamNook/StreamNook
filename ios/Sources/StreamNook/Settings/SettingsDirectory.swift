import SNKit
import SwiftUI
import UniformTypeIdentifiers

/// A settings row the core's catalog names (`search_settings`, `get_changed_settings`).
struct CatalogRow: Decodable, Identifiable, Hashable {
    let page: String
    let section: String
    let title: String
    let paths: [String]
    let detail: String
    /// The brand whose logo leads the title ("Paint" with 7TV's mark).
    let logo: String?

    var id: String { "\(page)|\(section)|\(title)|\(paths.joined(separator: ","))" }
    var settingsSection: SettingsSection? { SettingsSection(rawValue: page) }
    /// "Chat › Message Layout": where the row lives.
    var place: String { [settingsSection?.title ?? page, section == settingsSection?.title ? nil : section].compactMap { $0 }.joined(separator: " › ") }
}

extension CatalogRow {
    /// The rows a catalog command answers (`search_settings`, `get_changed_settings`), every one
    /// this build can read. A row it cannot read, or one on a page it has no section for, is
    /// logged and left out, so one row never blanks the list; Android draws what it knows the
    /// same way.
    @MainActor
    static func load(_ cmd: String, args: [String: Any?] = [:], core: CoreBridge) async -> [CatalogRow] {
        let json: String
        do {
            json = try await core.invoke(cmd, args: args)
        } catch {
            log.error("\(cmd, privacy: .public) failed: \(String(describing: error), privacy: .public)")
            return []
        }
        guard let items = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [Any] else {
            log.error("\(cmd, privacy: .public) answered no list: \(String(json.prefix(300)), privacy: .public)")
            return []
        }
        return items.compactMap { item in
            guard JSONSerialization.isValidJSONObject(item),
                  let data = try? JSONSerialization.data(withJSONObject: item),
                  let row = try? JSONDecoder().decode(CatalogRow.self, from: data) else {
                log.error("settings row skipped, unreadable: \(String(describing: item), privacy: .public)")
                return nil
            }
            guard row.settingsSection != nil else {
                log.error("settings row skipped, no page \(row.page, privacy: .public): \(row.title, privacy: .public)")
                return nil
            }
            return row
        }
    }
}

/// Opens the page a catalog row lives on and brings its section into view.
@MainActor
func openSettingsRow(_ row: CatalogRow, model: AppModel) {
    guard let section = row.settingsSection else { return }
    model.nav.push(.settings(section))
    Task {
        try? await Task.sleep(for: .milliseconds(450))
        SettingsAnchor.shared.target = nil
        SettingsAnchor.shared.target = row.section
    }
}

/// The You tab's settings: a search over every row (the core ranks titles, aliases, sections and
/// descriptions), and the section rows while nothing is typed.
struct SettingsDirectory: View {
    let model: AppModel
    @State private var search = SettingsSearch.shared
    @State private var results: [CatalogRow] = []
    @FocusState private var focused: Bool

    /// The field sits low on the You page; on focus it rises to the top so the results show above
    /// the keyboard instead of under it.
    var body: some View {
        ScrollViewReader { proxy in
            content
                // A search hit opens on the tap that puts the keyboard away.
                .tapsThroughKeyboard()
                .onChange(of: focused) { _, isFocused in
                    guard isFocused else { return }
                    withAnimation(.snappy) { proxy.scrollTo(Self.fieldID, anchor: .top) }
                }
        }
    }

    private static let fieldID = "settings-search-field"

    private var content: some View {
        VStack(spacing: 8) {
            HStack(spacing: 8) {
                Image(systemName: Symbol.search).font(.system(size: 15)).foregroundStyle(SN.textMuted)
                TextField("Search settings", text: $search.query)
                    .font(.sn(15))
                    .foregroundStyle(SN.textPrimary)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.search)
                    .focused($focused)
                if !search.query.isEmpty {
                    Button { search.query = "" } label: {
                        Image(systemName: Symbol.clear).foregroundStyle(SN.textMuted).frame(width: 32, height: 32)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Clear search")
                }
            }
            .padding(.horizontal, 12)
            .frame(height: 44)
            .snGlass(Capsule())
            // Clearing the search keeps the keyboard for the next one.
            .keepsKeyboard()
            .padding(.horizontal, 16)
            .id(Self.fieldID)

            if search.query.trimmingCharacters(in: .whitespaces).isEmpty {
                GroupedCard {
                    ForEach(SettingsSection.listed) { section in
                        NavigationLink(value: Route.settings(section)) {
                            RowLabel(symbol: section.symbol, title: section.title, detail: section.detail)
                        }
                        .buttonStyle(RowPress())
                    }
                }
                .padding(.horizontal, 16)
            } else if results.isEmpty {
                Text("No setting matches \"\(search.query)\".")
                    .font(.sn(14, 500)).foregroundStyle(SN.textSecondary)
                    .frame(maxWidth: .infinity).padding(.vertical, 24)
            } else {
                GroupedCard {
                    ForEach(results) { row in
                        Button { openSettingsRow(row, model: model) } label: {
                            CatalogRowLabel(row: row)
                        }
                        .buttonStyle(RowPress())
                    }
                }
                .padding(.horizontal, 16)
            }
        }
        .task(id: search.query) {
            let text = search.query.trimmingCharacters(in: .whitespaces)
            guard !text.isEmpty else { results = []; return }
            try? await Task.sleep(for: .milliseconds(120))
            guard !Task.isCancelled else { return }
            results = await CatalogRow.load("search_settings", args: ["query": text], core: model.core)
        }
    }
}

/// What the settings search holds, kept while you open a result and come back.
@Observable @MainActor
final class SettingsSearch {
    static let shared = SettingsSearch()
    var query = ""
}

/// A catalog row: its title, where it lives, and its one-line description.
struct CatalogRowLabel: View {
    let row: CatalogRow
    var trailing: AnyView?

    var body: some View {
        HStack(alignment: .settingTitle, spacing: 12) {
            Image(systemName: row.settingsSection?.symbol ?? Symbol.disclosure)
                .font(.system(size: 17)).foregroundStyle(SN.textSecondary).frame(width: 24)
                .alignmentGuide(.settingTitle) { $0[VerticalAlignment.center] }
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 5) {
                    if let logo = row.logo { BrandLogo(id: logo, size: 14) }
                    Text(row.title).font(.snRowTitle).foregroundStyle(SN.textPrimary).lineLimit(1)
                }
                .alignmentGuide(.settingTitle) { $0[VerticalAlignment.center] }
                Text(row.place).font(.sn(12.5, 500)).foregroundStyle(SN.textSecondary).lineLimit(1)
                if !row.detail.isEmpty {
                    Text(row.detail).font(.snRowDetail).foregroundStyle(SN.textMuted).lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            Group {
                if let trailing { trailing } else {
                    Image(systemName: Symbol.disclosure).font(.system(size: 13, weight: .semibold)).foregroundStyle(SN.textMuted)
                }
            }
            .alignmentGuide(.settingTitle) { $0[VerticalAlignment.center] }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .contentShape(Rectangle())
    }
}

/// Settings > Changed settings: every row you set differently from a fresh install, grouped by
/// page, each with its reset arrow and a way to its page.
struct ChangedPanel: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var rows: [CatalogRow]?

    var body: some View {
        Section {
            Label {
                Text("Everything you set differently from a fresh install. The arrow puts one back; tap a row to open its page.")
                    .font(.sn(12.5)).foregroundStyle(SN.textSecondary)
            } icon: {
                Image(systemName: "slider.horizontal.below.square.and.square.filled")
            }
            .task(id: store.changed.count) { await load() }
        }
        if let rows {
            if rows.isEmpty {
                Section { EmptyNote(text: "Every setting is at its default.", hint: nil) }
            }
            ForEach(SettingsSection.allCases.filter { page in rows.contains { $0.page == page.rawValue } }) { page in
                SettingsBlock(title: page.title) {
                    ForEach(rows.filter { $0.page == page.rawValue }) { row in
                        HStack(spacing: 4) {
                            Button { openSettingsRow(row, model: model) } label: {
                                Label {
                                    VStack(alignment: .leading, spacing: 2) {
                                        HStack(spacing: 5) {
                                            if let logo = row.logo { BrandLogo(id: logo, size: 13) }
                                            Text(row.title).foregroundStyle(SN.textPrimary)
                                        }
                                        Text(row.section).font(.sn(12)).foregroundStyle(SN.textSecondary)
                                    }
                                } icon: {
                                    Image(systemName: SettingIcons.symbol(paths: row.paths, title: row.title))
                                }
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            if !row.paths.isEmpty {
                                IconButton(symbol: "arrow.counterclockwise", label: "Reset \(row.title) to default", size: 14,
                                           tint: SN.textSecondary) {
                                    Task {
                                        await store.reset(row.paths)
                                        await load()
                                    }
                                }
                                .padding(.trailing, -8)
                            }
                        }
                    }
                }
            }
        } else {
            ProgressView().frame(maxWidth: .infinity).padding(.vertical, 30)
                .listRowBackground(Color.clear)
        }
    }

    private func load() async {
        rows = await CatalogRow.load("get_changed_settings", core: model.core)
        log.notice("changed settings: \(rows?.count ?? 0) rows")
    }
}

/// Settings > Backup: the daily copies the core keeps (and restores from on its own when the
/// settings file will not load), and a file you can keep or bring back.
struct BackupPanel: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var backups: [Backup] = []
    @State private var armed: String?
    @State private var restored: String?
    @State private var shareURL: URL?
    @State private var importing = false
    @State private var importNote: String?

    struct Backup: Decodable, Identifiable {
        let id: String
        let bytes: Int
    }

    var body: some View {
        SettingsBlock(title: "Daily copies", detail: "StreamNook keeps a copy of your settings each day you use it, for the last 7 days. If your settings ever fail to load, the newest copy is put back on its own.") {
            if backups.isEmpty {
                Text("The first copy is saved the next time a setting changes.")
                    .font(.sn(12.5)).foregroundStyle(SN.textSecondary)
            }
            ForEach(backups) { backup in
                SettingRow(title: Self.day(backup.id),
                           detail: restored == backup.id ? "Restored. Your settings now match this day." : Self.size(backup.bytes),
                           icon: "clock.arrow.circlepath", centersControl: true) {
                    ArmedPill(label: "Restore", armedLabel: "Replace current",
                              armed: Binding(get: { armed == backup.id }, set: { armed = $0 ? backup.id : nil })) {
                        Task { await restore(backup.id) }
                    }
                }
            }
        }
        SettingsBlock(title: "Settings file", detail: "Save every setting to one file (your accounts stay out of it), or bring one back on this phone or another device.") {
            // The page's loader, share sheet and file picker sit on this row, which is always there.
            WideRow(title: "Export", detail: "Opens the share sheet, so you can save it to Files or send it on.") {
                SettingButton(title: "Export settings", symbol: "square.and.arrow.up") { Task { await export() } }
                    .padding(.top, 10)
            }
            .task { await load() }
            .sheet(item: Binding(get: { shareURL.map(ShareItem.init) }, set: { shareURL = $0?.url })) { item in
                ActivitySheet(url: item.url).ignoresSafeArea()
            }
            .fileImporter(isPresented: $importing, allowedContentTypes: [.json]) { result in
                guard case .success(let url) = result else { return }
                Task { await importFile(url) }
            }
            WideRow(title: "Import", detail: importNote ?? "Replaces your settings with the ones in the file. Your accounts stay signed in.") {
                SettingButton(title: "Import a file", symbol: "square.and.arrow.down") { importing = true }
                    .padding(.top, 10)
            }
        }
    }

    struct ShareItem: Identifiable {
        let url: URL
        var id: String { url.path }
    }

    static func day(_ id: String) -> String {
        let parser = DateFormatter()
        parser.dateFormat = "yyyy-MM-dd"
        guard let date = parser.date(from: id) else { return id }
        if Calendar.current.isDateInToday(date) { return "Today" }
        if Calendar.current.isDateInYesterday(date) { return "Yesterday" }
        return date.formatted(.dateTime.weekday(.wide).month(.abbreviated).day())
    }

    static func size(_ bytes: Int) -> String {
        bytes < 1024 ? "\(bytes) bytes" : "\(max(1, bytes / 1024)) KB"
    }

    private func load() async {
        backups = (try? await model.core.invoke("list_settings_backups", args: [:], as: [Backup].self)) ?? []
    }

    private func restore(_ id: String) async {
        do {
            _ = try await model.core.invoke("restore_settings_backup", args: ["id": id])
            restored = id
            await store.refresh()
            await model.loadSettings()
        } catch {
            model.toasts.show("\(error)")
        }
    }

    private func export() async {
        let stamp = Date().formatted(.iso8601.year().month().day())
        let url = FileManager.default.temporaryDirectory.appending(path: "StreamNook-settings-\(stamp).json")
        do {
            _ = try await model.core.invoke("export_settings", args: ["path": url.path])
            shareURL = url
        } catch {
            model.toasts.show("Could not export your settings: \(error)")
        }
    }

    private func importFile(_ url: URL) async {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        let copy = FileManager.default.temporaryDirectory.appending(path: "import-\(UUID().uuidString).json")
        do {
            try FileManager.default.copyItem(at: url, to: copy)
            _ = try await model.core.invoke("import_settings", args: ["path": copy.path])
            importNote = "Imported \(url.lastPathComponent)."
            await store.refresh()
            await model.loadSettings()
        } catch {
            model.toasts.show("\(error)")
        }
        try? FileManager.default.removeItem(at: copy)
    }
}

/// The system share sheet for one file.
struct ActivitySheet: UIViewControllerRepresentable {
    let url: URL

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: [url], applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
