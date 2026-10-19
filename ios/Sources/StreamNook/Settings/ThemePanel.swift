import SNKit
import SwiftUI

/// Settings > Theme: the color theme, how glassy every surface is, and the interface font. The
/// gallery, the OLED accent and every palette come from the core; picking repaints the app.
struct ThemePanel: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var catalog: Catalog?

    struct Catalog {
        var categories: [(id: String, name: String, description: String)]
        var themes: [Card]
        var custom: [Card]
        var oledID: String
        var defaultID: String
        var accents: [(name: String, value: String)]
    }

    struct Card: Identifiable {
        let id: String
        let name: String
        let description: String
        let category: String
        let palette: Palette
    }

    var body: some View {
        let current = store.string("theme", "winters-glass")
        Section {
            header(current)
                .task(id: "\(store.string("oled_accent"))|\((store.value("custom_themes") as? [Any])?.count ?? 0)") { await load() }
            GlassinessCard()
            FontCard()
        } header: {
            ListHeader(text: "Theme")
        }
        .id("Theme")
        if let catalog {
            if !catalog.custom.isEmpty {
                category(name: "Your Themes", description: "Your own creations", cards: catalog.custom, current: current)
            }
            ForEach(catalog.categories, id: \.id) { cat in
                category(name: cat.name, description: cat.description,
                         cards: catalog.themes.filter { $0.category == cat.id }, current: current,
                         oledAccents: cat.id == "signature" && current == catalog.oledID ? catalog.accents : nil)
            }
        } else {
            Section { ProgressView().frame(maxWidth: .infinity).padding(.vertical, 20) }
        }
        Section {
            Label {
                Text("Create a custom theme to match your setup exactly.").font(.sn(12.5)).foregroundStyle(SN.textSecondary)
            } icon: {
                Image(systemName: "sparkles")
            }
        }
    }

    /// The picked theme's name beside the row's title.
    private func header(_ current: String) -> some View {
        let cards = (catalog?.themes ?? []) + (catalog?.custom ?? [])
        let name = cards.first { $0.id == current }?.name ?? "Frosted Glass"
        return Label {
            HStack(spacing: 8) {
                SettingTitle(title: "Theme", paths: ["theme"], helpOpen: .constant(false)) {
                    await model.loadTheme()
                }
                Spacer()
                HStack(spacing: 6) {
                    Circle().fill(SN.accent).frame(width: 14, height: 14)
                    Text(name).font(.sn(12.5)).foregroundStyle(SN.textSecondary).lineLimit(1)
                }
            }
        } icon: {
            Image(systemName: SettingIcons.symbol(paths: ["theme"], title: "Theme"))
        }
    }

    /// A category: its swatch grid on the card, its line under it, and the OLED accent beside the OLED theme.
    private func category(name: String, description: String, cards: [Card], current: String,
                          oledAccents: [(name: String, value: String)]? = nil) -> some View {
        Section {
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 10), count: 3), spacing: 10) {
                ForEach(cards) { card in
                    ThemeCard(card: card, picked: card.id == current) {
                        Task {
                            await store.set("theme", card.id)
                            await model.loadTheme()
                        }
                    }
                }
            }
            .padding(.vertical, 6)
            if let oledAccents { OledAccentCard(accents: oledAccents) }
        } header: {
            ListHeader(text: name)
        } footer: {
            ListFooter(text: description)
        }
        .id(name)
    }

    static func symbol(_ category: String) -> String {
        switch category {
        case "signature": "star"
        case "universal": "globe"
        case "modern": "moon.stars"
        case "classic": "book.closed"
        default: "cup.and.saucer"
        }
    }

    private func load() async {
        guard let json = try? await model.core.invoke("get_theme_catalog", args: [:]),
              let root = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any] else { return }
        let card = { (t: [String: Any], created: Bool) -> Card? in
            guard let id = t["id"] as? String else { return nil }
            var description = t["description"] as? String ?? ""
            if created, let ms = t["created_at"] as? Double {
                description = "Custom theme created \(Date(timeIntervalSince1970: ms / 1000).formatted(date: .numeric, time: .omitted))"
            }
            return Card(id: id, name: t["name"] as? String ?? id, description: description,
                        category: t["category"] as? String ?? "signature", palette: Palette(tokens: t["palette"] as? [String: Any] ?? [:]))
        }
        catalog = Catalog(
            categories: (root["categories"] as? [[String: Any]] ?? []).map {
                (id: $0["id"] as? String ?? "", name: $0["name"] as? String ?? "", description: $0["description"] as? String ?? "")
            },
            themes: (root["themes"] as? [[String: Any]] ?? []).compactMap { card($0, false) },
            custom: (root["custom"] as? [[String: Any]] ?? []).compactMap { card($0, true) },
            oledID: root["oled_theme_id"] as? String ?? "",
            defaultID: root["default_theme_id"] as? String ?? "winters-glass",
            accents: (root["oled_accent_presets"] as? [[String: Any]] ?? []).map {
                (name: $0["name"] as? String ?? "", value: $0["value"] as? String ?? "")
            }
        )
    }
}

/// A theme drawn in its own colors: five swatches and its name, a check when picked. A third of
/// the width cut every description mid-word, so the description is read out, not drawn.
private struct ThemeCard: View {
    let card: ThemePanel.Card
    let picked: Bool
    let pick: () -> Void

    var body: some View {
        let p = card.palette
        Button(action: pick) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(spacing: 3) {
                    ForEach([p.accent, p.highlightPink, p.highlightPurple, p.highlightBlue, p.highlightGreen].indices, id: \.self) { i in
                        Circle().fill([p.accent, p.highlightPink, p.highlightPurple, p.highlightBlue, p.highlightGreen][i].color)
                            .frame(width: 10, height: 10)
                    }
                }
                Text(card.name).font(.sn(13, 600)).foregroundStyle(p.textPrimary.color).lineLimit(1).minimumScaleFactor(0.8)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .background { RoundedRectangle(cornerRadius: 8, style: .continuous).fill(p.background.color) }
            .overlay {
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .stroke(picked ? SN.accent : SN.borderSubtle, lineWidth: picked ? 1.5 : 1)
            }
            .overlay {
                if picked {
                    RoundedRectangle(cornerRadius: 10, style: .continuous).stroke(SN.accent.opacity(0.3), lineWidth: 2).padding(-2)
                }
            }
            .overlay(alignment: .topTrailing) {
                if picked {
                    Image(systemName: "checkmark.circle.fill").font(.system(size: 18)).foregroundStyle(.white, SN.accent).padding(5)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(CardPress())
        .accessibilityLabel(card.name)
        .accessibilityHint(card.description)
        .accessibilityAddTraits(picked ? .isSelected : [])
    }
}

/// Glassiness: a live preview while dragging, saved on release.
private struct GlassinessCard: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var live: Double?

    var body: some View {
        let stored = store.double("glass_transparency", 100)
        Label {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 8) {
                    SettingTitle(title: "Glassiness", paths: ["glass_transparency"], helpOpen: .constant(false))
                    Spacer()
                    Text("\(Int(live ?? stored))%").font(.sn(13, 500).monospacedDigit()).foregroundStyle(SN.textSecondary)
                }
                Slider(value: Binding(get: { live ?? stored }, set: { value in
                    live = value
                    model.look.preview(glassiness: value / 100)
                }), in: 0...100, step: 1) { editing in
                    guard !editing, let value = live else { return }
                    Task {
                        if Int(value) != Int(stored) { await store.set("glass_transparency", Int(value)) }
                        await model.loadSettings()
                        live = nil
                    }
                }
                .tint(SN.accent)
                .accessibilityLabel("Glassiness")
                HStack {
                    Text("Solid")
                    Spacer()
                    Text("Frosted glass")
                }
                .font(.sn(12)).foregroundStyle(SN.textMuted)
                Text("How see-through and frosted every surface is, for every theme. 100% is the signature glass; 0% removes all transparency and blur for a completely flat, solid look.")
                    .font(.sn(12)).foregroundStyle(SN.textMuted).fixedSize(horizontal: false, vertical: true)
            }
        } icon: {
            Image(systemName: SettingIcons.symbol(paths: ["glass_transparency"], title: "Glassiness"))
        }
        .padding(.vertical, 4)
    }
}

/// Font: a tile per face, each name drawn in it where the iPhone has the face, and Custom: any
/// family from Google Fonts, typed or picked from the suggestions, kept on the phone once read.
private struct FontCard: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model

    static let fonts = [("Satoshi", "satoshi"), ("Twitch", "twitch"), ("Geist", "geist"), ("Manrope", "manrope"), ("Outfit", "outfit"),
                        ("Space Grotesk", "space-grotesk"), ("Serif", "serif"), ("System", "system")]

    /// The Custom family as typed, saved a moment after the last keystroke.
    @State private var draft: String?
    @State private var saving: Task<Void, Never>?

    var body: some View {
        let picked = store.string("font", "satoshi").isEmpty ? "satoshi" : store.string("font", "satoshi")
        let custom = draft ?? store.string("font_custom")
        Label {
            VStack(alignment: .leading, spacing: 10) {
                SettingTitle(title: "Font", paths: ["font"], helpOpen: .constant(false))
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                    ForEach(Self.fonts, id: \.1) { name, id in
                        tile(name, face: id, on: id == picked) { await pick(id) }
                    }
                    // Once a family is typed, the tile names it, in its own face when found.
                    let named = CustomFont.sanitize(custom)
                    tile(picked == "custom" && !named.isEmpty ? named : "Custom", face: "custom", on: picked == "custom") {
                        await pick("custom")
                    }
                }
                if picked == "custom" {
                    customFields(custom)
                }
            }
        } icon: {
            Image(systemName: SettingIcons.symbol(paths: ["font"], title: "Font"))
        }
        .padding(.vertical, 4)
    }

    private func tile(_ name: String, face: String, on: Bool, action: @escaping () async -> Void) -> some View {
        Button {
            Task { await action() }
        } label: {
            HStack {
                Text(name)
                    .font(Typeface.font(15, 400, face: face))
                    .foregroundStyle(SN.textPrimary).lineLimit(1)
                Spacer(minLength: 4)
                if on { Image(systemName: "checkmark").font(.system(size: 13, weight: .semibold)).foregroundStyle(SN.accent) }
            }
            .padding(.horizontal, 12).padding(.vertical, 8)
            .frame(minHeight: 44)
            .background { RoundedRectangle(cornerRadius: 8, style: .continuous).fill(on ? SN.accent.opacity(0.1) : .clear) }
            .overlay { RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(on ? SN.accent : SN.borderSubtle, lineWidth: 1) }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }

    /// The family's name, the suggestions as one-tap fills, and where fonts come from.
    private func customFields(_ custom: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("e.g. Poppins", text: Binding(get: { custom }, set: { type($0) }))
                .font(.sn(15))
                .foregroundStyle(SN.textPrimary)
                .textInputAutocapitalization(.words)
                .autocorrectionDisabled()
                .submitLabel(.done)
                .onSubmit { save(custom, after: .zero) }
                .padding(.horizontal, 12).padding(.vertical, 10)
                .overlay { RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(SN.borderSubtle, lineWidth: 1) }
                .accessibilityLabel("Custom font name")
            FlowLayout(spacing: 6) {
                ForEach(CustomFont.suggested, id: \.self) { name in
                    let on = CustomFont.sanitize(custom) == name
                    Button {
                        draft = name
                        save(name, after: .zero)
                    } label: {
                        Text(name)
                            .font(.sn(13))
                            .foregroundStyle(on ? SN.accent : SN.textSecondary)
                            .padding(.horizontal, 10).padding(.vertical, 6)
                            .overlay { Capsule().stroke(on ? SN.accent : SN.borderSubtle, lineWidth: 1) }
                            .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                }
            }
            Text("Any free font from fonts.google.com works, just type its exact name. It is kept on this phone after the first time.")
                .font(.sn(12)).foregroundStyle(SN.textMuted)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func pick(_ id: String) async {
        saving?.cancel()
        await store.set("font", id)
        await model.loadSettings()
    }

    private func type(_ text: String) {
        draft = text
        save(text, after: .milliseconds(700))
    }

    /// Saves the family (and Custom as the font) once typing pauses; the font follows the save.
    private func save(_ text: String, after delay: Duration) {
        saving?.cancel()
        let family = CustomFont.sanitize(text)
        saving = Task {
            try? await Task.sleep(for: delay)
            guard !Task.isCancelled else { return }
            await store.set(["font": "custom", "font_custom": family])
            await model.loadSettings()
        }
    }
}

/// The OLED theme's accent: eight swatches and a custom color, saved a moment after the last
/// change.
private struct OledAccentCard: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    let accents: [(name: String, value: String)]

    var body: some View {
        let accent = store.string("oled_accent", "#a064ff").lowercased()
        Label {
            VStack(alignment: .leading, spacing: 10) {
                VStack(alignment: .leading, spacing: 1) {
                    SettingTitle(title: "OLED accent", paths: ["oled_accent"], helpOpen: .constant(false)) {
                        await model.loadTheme()
                    }
                    Text("The glow color on pure black").font(.sn(12)).foregroundStyle(SN.textSecondary)
                }
                HStack(spacing: 8) {
                    ForEach(accents, id: \.value) { preset in
                        Button { save(preset.value) } label: {
                            Circle().fill(RGBA(css: preset.value)?.color ?? .gray).frame(width: 28, height: 28)
                                .overlay { Circle().stroke(accent == preset.value ? SN.textPrimary : .clear, lineWidth: 2) }
                                .frame(width: 34, height: 44).contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(preset.name)
                        .accessibilityAddTraits(accent == preset.value ? .isSelected : [])
                    }
                }
                ColorPicker("Custom color", selection: Binding(get: { RGBA(css: accent)?.color ?? .purple }, set: { color in
                    save(SettingColor.hex(color))
                }), supportsOpacity: false)
                .font(.sn(13)).foregroundStyle(SN.textSecondary)
            }
        } icon: {
            Image(systemName: SettingIcons.symbol(paths: ["oled_accent"], title: "OLED accent"))
        }
        .padding(.vertical, 4)
    }

    private func save(_ hex: String) {
        Task {
            await store.set("oled_accent", hex)
            await model.loadTheme()
        }
    }
}
