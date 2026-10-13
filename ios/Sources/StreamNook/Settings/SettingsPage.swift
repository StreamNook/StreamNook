import SNKit
import SwiftUI

/// One settings section, pushed from its You-tab row over the tab bar.
struct SettingsPage: View {
    let model: AppModel
    let section: SettingsSection

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                switch section {
                case .theme: ThemePanel(model: model)
                case .support: SupportPanel(model: model)
                default: EmptyNote(text: "\(section.title) settings are built next.", hint: nil)
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 12)
            .padding(.bottom, 16)
        }
        .scrollIndicators(.hidden)
        .background(SN.background)
        .navigationTitle(section.title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
    }
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

/// Theme: the glass and motion the whole app follows. The theme cards and fonts come once the
/// core hands over the theme list.
private struct ThemePanel: View {
    let model: AppModel
    @State private var glassiness: Double = 100
    @State private var editing = false

    private var look: Look { model.look }

    var body: some View {
        SettingsGroup(title: "Glassiness") {
            GlassPreview(glassiness: glassiness / 100)
                .padding(12)
            SettingsRow(title: "Glassiness", detail: "How much of the page shows through StreamNook's panels. At 0% every panel is solid.") {
                Text("\(Int(glassiness))%").font(.sn(13, 600).monospacedDigit()).foregroundStyle(SN.textSecondary)
            }
            Slider(value: $glassiness, in: 0...100, step: 5) { editing in
                self.editing = editing
                if !editing { Task { await model.patchSettings(["glass_transparency": Int(glassiness)]) } }
            }
            .padding(.horizontal, 14)
            .padding(.bottom, 14)
        }
        .onAppear { glassiness = look.glassiness * 100 }
        .onChange(of: look.glassiness) { _, value in if !editing { glassiness = value * 100 } }

        SettingsGroup(title: "Motion") {
            SettingsRow(title: "Motion", detail: "Reduced keeps fades and drops movement; Off stills everything but loaders.") {
                Picker("Motion", selection: Binding(get: { look.motion }, set: { mode in
                    Task { await model.patchSettings(["motion_mode": mode.rawValue]) }
                })) {
                    Text("Full").tag(MotionMode.full)
                    Text("Reduced").tag(MotionMode.reduced)
                    Text("Off").tag(MotionMode.off)
                }
                .pickerStyle(.menu)
                .tint(SN.textSecondary)
            }
        }
    }
}

/// What a glassiness reads as before it is picked: a panel and a chip over a busy picture, at the
/// value under the finger.
private struct GlassPreview: View {
    let glassiness: Double

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        ZStack {
            LinearGradient(colors: [SN.accent, SN.highlightPurple, SN.live.opacity(0.8)], startPoint: .topLeading, endPoint: .bottomTrailing)
            HStack(spacing: 10) {
                Circle().fill(.white.opacity(0.9)).frame(width: 26, height: 26)
                VStack(alignment: .leading, spacing: 3) {
                    Text("A panel over the page").font(.sn(13, 600)).foregroundStyle(SN.textPrimary)
                    Text("\(Int(glassiness * 100))% glass").font(.sn(12)).foregroundStyle(SN.textSecondary)
                }
                Spacer()
            }
            .padding(12)
            .background {
                ZStack {
                    shape.fill(SN.backgroundTertiary.opacity(1 - glassiness))
                    shape.fill(SN.backgroundSecondary)
                    if glassiness > 0 { shape.fill(.ultraThinMaterial).opacity(glassiness) }
                }
            }
            .padding(14)
        }
        .frame(height: 96)
        .clipShape(shape)
        .accessibilityHidden(true)
    }
}

/// Support: what this build is and how its core is doing.
private struct SupportPanel: View {
    let model: AppModel

    var body: some View {
        SettingsGroup(title: "Diagnostics") {
            SettingsRow(title: "Version") { value(model.version) }
            Divider().overlay(SN.borderSubtle)
            SettingsRow(title: "Core") { value(model.core.kind) }
            Divider().overlay(SN.borderSubtle)
            SettingsRow(title: "Status") { value(status) }
            Divider().overlay(SN.borderSubtle)
            SettingsRow(title: "Events") { value(model.lastEvent.map { "\(model.eventCount), last \($0)" } ?? "None yet") }
        }
    }

    private var status: String {
        if let error = model.startError { return error }
        if let ms = model.startMS { return "Started in \(ms) ms" }
        return "Starting"
    }

    private func value(_ text: String) -> some View {
        Text(text).font(.sn(13)).foregroundStyle(SN.textSecondary).lineLimit(1).truncationMode(.middle)
    }
}

/// Cosmetics: the badges, atmosphere and 7TV paint you wear. Built with the profile screens.
struct CosmeticsFrame: View {
    let model: AppModel

    var body: some View {
        ScrollView {
            EmptyNote(text: "Your cosmetics are built next.", hint: nil)
        }
        .background(SN.background)
        .navigationTitle("Cosmetics")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
    }
}
