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
        VStack(alignment: .leading, spacing: 20) {
            header(current)
            GlassinessCard()
            FontCard()
            if let catalog {
                if !catalog.custom.isEmpty {
                    category(symbol: "paintbrush.pointed", name: "Your Themes", description: "Your own creations", cards: catalog.custom,
                             current: current)
                }
                ForEach(catalog.categories, id: \.id) { cat in
                    category(symbol: Self.symbol(cat.id), name: cat.name, description: cat.description,
                             cards: catalog.themes.filter { $0.category == cat.id }, current: current)
                    if cat.id == "signature", current == catalog.oledID {
                        OledAccentCard(accents: catalog.accents)
                    }
                }
            } else {
                ProgressView().frame(maxWidth: .infinity).padding(.vertical, 20)
            }
            HStack(spacing: 8) {
                Image(systemName: "sparkles").font(.system(size: 14)).foregroundStyle(SN.accent)
                Text("Create a custom theme to match your setup exactly.").font(.sn(12)).foregroundStyle(SN.textSecondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background { RoundedRectangle(cornerRadius: 8, style: .continuous).fill(SN.surface.opacity(0.5)) }
            .overlay { RoundedRectangle(cornerRadius: 8, style: .continuous).stroke(SN.borderSubtle, lineWidth: 1) }
        }
        .task(id: "\(store.string("oled_accent"))|\((store.value("custom_themes") as? [Any])?.count ?? 0)") { await load() }
    }

    private func header(_ current: String) -> some View {
        let cards = (catalog?.themes ?? []) + (catalog?.custom ?? [])
        let name = cards.first { $0.id == current }?.name ?? "Frosted Glass"
        return HStack(spacing: 8) {
            Image(systemName: "paintpalette").font(.system(size: 18)).foregroundStyle(SN.accent)
            SettingTitle(title: "Theme", paths: ["theme"], helpOpen: .constant(false), font: .sn(16, 600)) {
                await model.loadTheme()
            }
            Spacer()
            HStack(spacing: 6) {
                Circle().fill(SN.accent).frame(width: 16, height: 16)
                Text(name).font(.sn(12)).foregroundStyle(SN.textSecondary).lineLimit(1)
            }
        }
    }

    private func category(symbol: String, name: String, description: String, cards: [Card], current: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: symbol).font(.system(size: 16)).foregroundStyle(SN.textSecondary)
                VStack(alignment: .leading, spacing: 1) {
                    Text(name).font(.sn(14, 600)).foregroundStyle(SN.textSecondary)
                    Text(description).font(.sn(12)).foregroundStyle(SN.textMuted)
                }
            }
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 3), spacing: 12) {
                ForEach(cards) { card in
                    ThemeCard(card: card, picked: card.id == current) {
                        Task {
                            await store.set("theme", card.id)
                            await model.loadTheme()
                        }
                    }
                }
            }
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

/// A theme drawn in its own colors: five swatches, its name and description, a check when picked.
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
                Text(card.description).font(.sn(11)).foregroundStyle(p.textSecondary.color).lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(10)
            .frame(maxWidth: .infinity, minHeight: 104, alignment: .topLeading)
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
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "drop").font(.system(size: 16)).foregroundStyle(SN.textSecondary)
                SettingTitle(title: "Glassiness", paths: ["glass_transparency"], helpOpen: .constant(false), font: .sn(14, 600))
                Spacer()
                Text("\(Int(live ?? stored))%").font(.sn(13, 500).monospacedDigit()).foregroundStyle(SN.textSecondary)
            }
            // Typed out: the release build's type checker otherwise reads this as Binding<Double?>.
            Slider(value: Binding<Double>(get: { live ?? stored }, set: { (value: Double) in
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
        .padding(14)
        .snPanel()
    }
}

/// Font: a tile per face, each name drawn in it where the iPhone has the face.
private struct FontCard: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model

    static let fonts = [("Satoshi", "satoshi"), ("Twitch", "twitch"), ("Geist", "geist"), ("Manrope", "manrope"), ("Outfit", "outfit"),
                        ("Space Grotesk", "space-grotesk"), ("Serif", "serif"), ("System", "system")]

    var body: some View {
        let picked = store.string("font", "satoshi").isEmpty ? "satoshi" : store.string("font", "satoshi")
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "textformat").font(.system(size: 16)).foregroundStyle(SN.textSecondary)
                SettingTitle(title: "Font", paths: ["font"], helpOpen: .constant(false), font: .sn(14, 600))
            }
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                ForEach(Self.fonts, id: \.1) { name, id in
                    let on = id == picked
                    Button {
                        Task {
                            await store.set("font", id)
                            await model.loadSettings()
                        }
                    } label: {
                        HStack {
                            Text(name)
                                .font(Typeface.font(15, 400, face: id))
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
            }
        }
        .padding(14)
        .snPanel()
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
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "drop").font(.system(size: 14)).foregroundStyle(SN.accent)
                VStack(alignment: .leading, spacing: 1) {
                    SettingTitle(title: "OLED accent", paths: ["oled_accent"], helpOpen: .constant(false), font: .sn(14, 600)) {
                        await model.loadTheme()
                    }
                    Text("The glow color on pure black").font(.sn(12)).foregroundStyle(SN.textMuted)
                }
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
        .padding(14)
        .snPanel()
    }

    private func save(_ hex: String) {
        Task {
            await store.set("oled_accent", hex)
            await model.loadTheme()
        }
    }
}
