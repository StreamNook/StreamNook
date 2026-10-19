import SNKit
import SwiftUI

// The rows every settings page is built from. A row names one property in a short noun, says
// in one plain sentence what it does, keeps the long explanation behind its help glyph, and
// shows a reset arrow only while its value differs from a fresh install. Settings that only
// mean something with another one sit nested under it. Every control reads and writes the
// core's settings through the SettingsStore in the environment, one field at a time.

/// A feature's section in the grouped list: its name above the card, its rows on the card with
/// separators inset after their symbols, and a line on what it covers under the card.
struct SettingsBlock<Content: View>: View {
    let title: String
    var logo: String?
    /// The words shown when the logo already names the brand ("Chat" beside YouTube's mark);
    /// `title` stays the section's name for scrolling and search.
    var shownTitle: String?
    var detail: String?
    /// A small action on the heading line (a "Show reference" link).
    var accessory: AnyView?
    /// A block of list entries rather than settings rows; drawn the same way.
    var bare = false
    @ViewBuilder let content: Content

    var body: some View {
        Section {
            content
        } header: {
            // The accessory overlays the header line, so a tall tap target never pushes the card down.
            ListHeader(text: shownTitle ?? title, logo: logo)
                .frame(maxWidth: .infinity, alignment: .leading)
                .overlay(alignment: .trailing) { accessory }
        } footer: {
            if let detail { ListFooter(text: detail) }
        }
        .id(title)
    }
}

/// The title line of a row or nested line: the name, the help glyph, the reset arrow.
struct SettingTitle: View {
    @Environment(SettingsStore.self) private var store
    let title: String
    var paths: [String] = []
    var hasHelp = false
    @Binding var helpOpen: Bool
    /// A row's title reads like the chat menu's rows; a nested line passes its own, smaller.
    var font: Font = .snBody
    /// A platform or badge service the row is about: its logo leads the title.
    var logo: String?
    /// Runs after a reset lands (a control with a side effect).
    var afterReset: (() async -> Void)?

    var body: some View {
        HStack(spacing: 4) {
            if let logo { BrandLogo(id: logo, size: 15).padding(.trailing, 2) }
            Text(title).font(font).foregroundStyle(SN.textPrimary)
            if hasHelp {
                Button {
                    withAnimation(SNMotion.run(.easeOut(duration: 0.18))) { helpOpen.toggle() }
                } label: {
                    Image(systemName: "questionmark.circle")
                        .font(.system(size: 12))
                        .foregroundStyle(helpOpen ? SN.accent : SN.textMuted)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .padding(.vertical, -8)
                .padding(.leading, -6)
                .accessibilityLabel("More about this setting")
                .accessibilityValue(helpOpen ? "Shown" : "Hidden")
            }
            if !paths.isEmpty, store.isChanged(paths) {
                Button {
                    Task {
                        await store.reset(paths)
                        await afterReset?()
                    }
                } label: {
                    Image(systemName: "arrow.counterclockwise")
                        .font(.system(size: 11, weight: .semibold))
                        .foregroundStyle(SN.textMuted)
                        .frame(width: 32, height: 32)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .padding(.vertical, -10)
                .padding(.leading, -8)
                .accessibilityLabel("Reset to default")
                .transition(.opacity)
            }
        }
    }
}

/// A row, as the chat menu draws one: its symbol, then the title line and its description with a
/// small control on the right, a wide control under it, the help text when opened, and the nested
/// settings, all on the text's edge so the separator starts after the symbol.
struct SettingRow<Trailing: View, Below: View, Nested: View>: View {
    let title: String
    var detail: String?
    var help: String?
    var paths: [String] = []
    var logo: String?
    /// The leading symbol; by default the one `SettingIcons` gives the row's setting.
    var icon: String?
    var afterReset: (() async -> Void)?
    /// Centers the control on the title and its text together (a row whose control is a button).
    var centersControl = false
    @ViewBuilder var trailing: Trailing
    @ViewBuilder var below: Below
    @ViewBuilder var nested: Nested
    @State private var helpOpen = false

    var body: some View {
        Label {
            VStack(alignment: .leading, spacing: 0) {
                // The control lines up with the title, not the middle of a long description.
                RowTop(centersControl: centersControl) {
                    SettingTitle(title: title, paths: paths, hasHelp: help != nil, helpOpen: $helpOpen, logo: logo,
                                 afterReset: afterReset)
                    VStack(alignment: .leading, spacing: 0) {
                        if let detail {
                            Text(detail)
                                .font(.sn(12))
                                .foregroundStyle(SN.textSecondary)
                                .lineSpacing(2)
                                .padding(.top, 2)
                        }
                        if helpOpen, let help {
                            Text(help)
                                .font(.sn(12))
                                .foregroundStyle(SN.textMuted)
                                .padding(.top, 4)
                                .transition(.opacity)
                        }
                    }
                    ZStack { trailing }
                }
                below
                nested
            }
        } icon: {
            Image(systemName: icon ?? SettingIcons.symbol(paths: paths, title: title))
        }
        .padding(.vertical, 4)
    }
}

/// A row's top: the title line with its text under it on the left, the control on the right
/// centered on the title line (or on the title and text together, for a button). The text is
/// measured at exactly the width it is drawn at. Takes three views: the title, the text under it
/// (a stack, possibly empty), and the control (a stack, possibly empty).
struct RowTop: Layout {
    var centersControl = false
    var spacing: CGFloat = 16

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let width = proposal.width ?? 320
        return CGSize(width: width, height: measure(width, subviews).height)
    }

    /// The title's first baseline, so a row's symbol lines up with its title, not its middle.
    func explicitAlignment(of guide: VerticalAlignment, in bounds: CGRect, proposal: ProposedViewSize,
                           subviews: Subviews, cache: inout ()) -> CGFloat? {
        guard guide == .firstTextBaseline, subviews.count == 3 else { return nil }
        let m = measure(bounds.width, subviews)
        let title = subviews[0].dimensions(in: ProposedViewSize(width: m.text, height: m.title))
        return bounds.minY + m.top + title[.firstTextBaseline]
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard subviews.count == 3 else { return }
        let m = measure(bounds.width, subviews)
        let titleY = bounds.minY + m.top
        subviews[0].place(at: CGPoint(x: bounds.minX, y: titleY), proposal: ProposedViewSize(width: m.text, height: m.title))
        subviews[1].place(at: CGPoint(x: bounds.minX, y: titleY + m.title), proposal: ProposedViewSize(width: m.text, height: m.under))
        let controlY = centersControl
            ? bounds.minY + (m.height - m.control.height) / 2
            : titleY + m.title / 2 - m.control.height / 2
        subviews[2].place(at: CGPoint(x: bounds.maxX - m.control.width, y: controlY),
                          proposal: ProposedViewSize(m.control))
    }

    private func measure(_ width: CGFloat, _ subviews: Subviews)
        -> (text: CGFloat, title: CGFloat, under: CGFloat, control: CGSize, top: CGFloat, height: CGFloat) {
        guard subviews.count == 3 else { return (width, 0, 0, .zero, 0, 0) }
        let control = subviews[2].sizeThatFits(.unspecified)
        let text = control.width > 0 ? max(0, width - control.width - spacing) : width
        let title = subviews[0].sizeThatFits(ProposedViewSize(width: text, height: nil)).height
        let under = subviews[1].sizeThatFits(ProposedViewSize(width: text, height: nil)).height
        // A control taller than the title line pushes the title down so the control stays inside the row.
        if centersControl {
            let top = max(0, (control.height - title - under) / 2)
            return (text, title, under, control, top, max(top + title + under, control.height))
        }
        let top = max(0, (control.height - title) / 2)
        return (text, title, under, control, top, max(top + title + under, top + title / 2 + control.height / 2))
    }
}

extension SettingRow where Below == EmptyView {
    init(title: String, detail: String? = nil, help: String? = nil, paths: [String] = [], logo: String? = nil,
         afterReset: (() async -> Void)? = nil,
         @ViewBuilder trailing: () -> Trailing, @ViewBuilder nested: () -> Nested) {
        self.init(title: title, detail: detail, help: help, paths: paths, logo: logo, afterReset: afterReset,
                  trailing: trailing, below: { EmptyView() }, nested: nested)
    }
}

extension SettingRow where Below == EmptyView, Nested == EmptyView {
    init(title: String, detail: String? = nil, help: String? = nil, paths: [String] = [], logo: String? = nil,
         icon: String? = nil, afterReset: (() async -> Void)? = nil, centersControl: Bool = false,
         @ViewBuilder trailing: () -> Trailing) {
        self.init(title: title, detail: detail, help: help, paths: paths, logo: logo, icon: icon, afterReset: afterReset,
                  centersControl: centersControl, trailing: trailing, below: { EmptyView() }, nested: { EmptyView() })
    }
}

/// A row whose control is too wide to sit beside the title (a segmented choice, a slider, a
/// menu), drawn under it.
struct WideRow<Below: View, Nested: View>: View {
    let title: String
    var detail: String?
    var help: String?
    var paths: [String] = []
    var icon: String?
    @ViewBuilder var below: Below
    @ViewBuilder var nested: Nested

    var body: some View {
        SettingRow(title: title, detail: detail, help: help, paths: paths, icon: icon,
                   trailing: { EmptyView() }, below: { below }, nested: { nested })
    }
}

extension WideRow where Nested == EmptyView {
    init(title: String, detail: String? = nil, help: String? = nil, paths: [String] = [], icon: String? = nil,
         @ViewBuilder below: () -> Below) {
        self.init(title: title, detail: detail, help: help, paths: paths, icon: icon, below: below, nested: { EmptyView() })
    }
}

extension VerticalAlignment {
    private enum SettingTitleAlignment: AlignmentID {
        static func defaultValue(in context: ViewDimensions) -> CGFloat { context[VerticalAlignment.center] }
    }

    /// The middle of a setting row's title line.
    static let settingTitle = VerticalAlignment(SettingTitleAlignment.self)
}

/// Settings that only mean something with the row above: indented behind a thin rule.
struct NestedSettings<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        // Nothing nested (a switch row with no dependents, or dependents that are hidden) draws
        // nothing: an empty block still added its 16 point gap, a blank line under the row.
        Group(subviews: content) { rows in
            if !rows.isEmpty {
                VStack(alignment: .leading, spacing: 14) { ForEach(rows) { $0 } }
                    .padding(.leading, 16)
                    .overlay(alignment: .leading) { Rectangle().fill(SN.borderSubtle).frame(width: 1) }
                    .padding(.top, 16)
            }
        }
    }
}

/// One nested line: a short title with its reset arrow and its control beside it, or (stacked)
/// the control full width under the title.
struct NestedLine<Control: View>: View {
    let title: String
    var paths: [String] = []
    var stacked = false
    var help: String?
    var logo: String?
    @ViewBuilder let control: Control
    @State private var helpOpen = false

    var body: some View {
        if stacked {
            VStack(alignment: .leading, spacing: 8) {
                SettingTitle(title: title, paths: paths, hasHelp: help != nil, helpOpen: $helpOpen, font: .sn(12.5, 500), logo: logo)
                helpText
                control
            }
        } else {
            VStack(alignment: .leading, spacing: 4) {
                // The name keeps its one line; a long menu label beside it ends in "..." first.
                HStack(spacing: 12) {
                    SettingTitle(title: title, paths: paths, hasHelp: help != nil, helpOpen: $helpOpen, font: .sn(12.5, 500), logo: logo)
                        .fixedSize(horizontal: true, vertical: false)
                        .layoutPriority(1)
                    Spacer(minLength: 8)
                    control
                }
                helpText
            }
        }
    }

    @ViewBuilder private var helpText: some View {
        if helpOpen, let help {
            Text(help).font(.sn(12)).foregroundStyle(SN.textMuted).fixedSize(horizontal: false, vertical: true)
        }
    }
}

// MARK: Controls bound to a settings path

/// A switch bound to a boolean setting.
struct SettingToggle: View {
    @Environment(SettingsStore.self) private var store
    let path: String
    var fallback = false
    var label = ""
    /// Writes something other than a plain boolean (a toggle over a string setting).
    var write: ((Bool) -> [String: Any?])?
    var isOn: ((SettingsStore) -> Bool)?
    var onChange: ((Bool) async -> Void)?

    var body: some View {
        Toggle(label, isOn: Binding(
            get: { isOn?(store) ?? store.bool(path, fallback) },
            set: { on in
                Task {
                    await store.set(write?(on) ?? [path: on])
                    await onChange?(on)
                }
            }
        ))
        .labelsHidden()
        .tint(SN.accent)
        .accessibilityLabel(label)
    }
}

/// One choice of a segmented or menu setting.
struct Choice: Identifiable, Hashable {
    let label: String
    let value: String
    var id: String { value }

    init(_ label: String, _ value: String) {
        self.label = label
        self.value = value
    }
}

/// The settings segmented control: a recessed track with one raised thumb that slides to the
/// picked option. Labels shrink rather than wrap, so a long set never takes a second row.
struct SettingSegments: View {
    let choices: [Choice]
    let selected: String?
    var accessibility: String = ""
    let pick: (String) -> Void
    @Namespace private var thumb
    @Environment(\.snStill) private var still

    var body: some View {
        SegmentRow {
            ForEach(choices) { choice in
                let on = choice.value == selected
                Button {
                    guard !on else { return }
                    pick(choice.value)
                } label: {
                    Text(choice.label)
                        .font(.sn(13, 500))
                        .lineLimit(1)
                        .minimumScaleFactor(0.85)
                        .foregroundStyle(on ? SN.textPrimary : SN.textMuted)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 6)
                        .frame(maxWidth: .infinity)
                        .background {
                            if on {
                                RoundedRectangle(cornerRadius: 6, style: .continuous)
                                    .fill(FixedColor.slate.color.opacity(0.15))
                                    .shadow(color: .black.opacity(0.25), radius: 1, y: 1)
                                    .matchedGeometryEffect(id: "thumb", in: thumb)
                            }
                        }
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(choice.label)
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
        .padding(2)
        .background {
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(FixedColor.slate.color.opacity(0.06))
                .overlay {
                    RoundedRectangle(cornerRadius: 8, style: .continuous)
                        .stroke(Color.black.opacity(0.25), lineWidth: 0.5)
                }
        }
        .animation(still ? nil : SNMotion.run(.spring(response: 0.28, dampingFraction: 0.72)), value: selected)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(accessibility)
    }
}

/// Lays segments out in one row: each gets the width its label needs plus an equal share of
/// what is left, so a long label never shrinks while short neighbors have room to spare.
struct SegmentRow: Layout {
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let sizes = subviews.map { $0.sizeThatFits(.unspecified) }
        let need = sizes.reduce(0) { $0 + $1.width }
        return CGSize(width: proposal.width ?? need, height: sizes.map(\.height).max() ?? 0)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        guard !subviews.isEmpty else { return }
        let ideal = subviews.map { $0.sizeThatFits(.unspecified).width }
        let need = ideal.reduce(0, +)
        let widths: [CGFloat] = need <= bounds.width
            ? ideal.map { $0 + (bounds.width - need) / CGFloat(ideal.count) }
            : ideal.map { $0 * bounds.width / need }
        var x = bounds.minX
        for (index, view) in subviews.enumerated() {
            view.place(at: CGPoint(x: x, y: bounds.minY), proposal: ProposedViewSize(width: widths[index], height: bounds.height))
            x += widths[index]
        }
    }
}

/// A segmented control bound to a string setting.
struct SettingSegmentsBound: View {
    @Environment(SettingsStore.self) private var store
    let path: String
    let choices: [Choice]
    var fallback: String?
    var accessibility = ""
    /// Maps the stored value onto a choice (a saved `hover` reads as `never`).
    var shown: ((String?) -> String?)?
    var onChange: ((String) async -> Void)?

    var body: some View {
        let raw = store.value(path).map { "\($0)" } ?? fallback
        SettingSegments(choices: choices, selected: shown?(raw) ?? raw, accessibility: accessibility) { value in
            Task {
                await store.set(path, value)
                await onChange?(value)
            }
        }
    }
}

/// A menu picker bound to a setting, its label the picked option's name.
struct SettingMenu: View {
    @Environment(SettingsStore.self) private var store
    let path: String
    let choices: [Choice]
    var fallback: String?
    var accessibility = ""
    /// Stores a number rather than the choice's text.
    var numeric = false
    var fullWidth = true

    var body: some View {
        let raw = store.value(path).map { value -> String in
            if let number = SettingsPath.double(value) { return number.rounded() == number ? String(Int(number)) : String(number) }
            return "\(value)"
        } ?? fallback
        let picked = choices.first { $0.value == raw }
        Menu {
            Picker(accessibility, selection: Binding(
                get: { raw ?? "" },
                set: { value in
                    Task { await store.set(path, numeric ? (Double(value) as Any) : value) }
                }
            )) {
                ForEach(choices) { Text($0.label).tag($0.value) }
            }
        } label: {
            HStack(spacing: 6) {
                Text(picked?.label ?? raw ?? "").font(.sn(13, 500)).foregroundStyle(SN.textPrimary).lineLimit(1)
                if fullWidth { Spacer(minLength: 4) }
                Image(systemName: "chevron.up.chevron.down").font(.system(size: 11, weight: .semibold)).foregroundStyle(SN.textMuted)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .frame(maxWidth: fullWidth ? .infinity : nil)
            .background {
                RoundedRectangle(cornerRadius: 8, style: .continuous).fill(SN.surface)
            }
            .contentShape(Rectangle())
        }
        .accessibilityLabel(accessibility)
        .accessibilityValue(picked?.label ?? "")
    }
}

/// A slider bound to a number setting. It follows the finger locally and saves once on
/// release, so the stored value is always the thumb's last place.
struct SettingSlider: View {
    @Environment(SettingsStore.self) private var store
    let path: String
    let range: ClosedRange<Double>
    let step: Double
    var fallback: Double
    var accessibility = ""
    /// The value shown beside an inline slider; nil hides the readout.
    var readout: ((Double) -> String)?
    /// Shows the stored value capped to the range (a desktop value past a phone's maximum).
    var integer = true
    /// Live value while dragging, for a title that carries it.
    @Binding var live: Double?
    var onRelease: ((Double) async -> Void)?

    var body: some View {
        let stored = min(max(store.double(path, fallback), range.lowerBound), range.upperBound)
        HStack(spacing: 10) {
            Slider(value: Binding(get: { live ?? stored }, set: { live = $0 }), in: range, step: step) { editing in
                guard !editing, let value = live else { return }
                Task {
                    let saved: Any = integer ? Int(value.rounded()) : value
                    await store.set(path, saved)
                    live = nil
                    await onRelease?(value)
                }
            }
            .tint(SN.accent)
            .accessibilityLabel(accessibility)
            if let readout {
                Text(readout(live ?? stored))
                    .font(.sn(12).monospacedDigit())
                    .foregroundStyle(SN.textMuted)
                    .frame(minWidth: 44, alignment: .trailing)
            }
        }
    }
}

/// A slider's value beside its title: quiet, tabular digits.
struct SliderReadout: View {
    let text: String

    var body: some View {
        Text(text).font(.sn(12).monospacedDigit()).foregroundStyle(SN.textMuted)
    }
}

/// A color swatch bound to a `#rrggbb` setting; an empty setting shows `unset`.
struct SettingColor: View {
    @Environment(SettingsStore.self) private var store
    let path: String
    let unset: String
    var label = ""

    var body: some View {
        let hex = store.string(path).isEmpty ? unset : store.string(path)
        ColorPicker(label, selection: Binding(
            get: { RGBA(css: hex)?.color ?? .gray },
            set: { color in
                let text = Self.hex(color)
                guard text.lowercased() != hex.lowercased() else { return }
                Task { await store.set(path, text) }
            }
        ), supportsOpacity: false)
        .labelsHidden()
        .frame(width: 40, height: 28, alignment: .trailing)
        .accessibilityLabel(label)
    }

    static func hex(_ color: Color) -> String {
        let ui = UIColor(color)
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        ui.getRed(&r, green: &g, blue: &b, alpha: &a)
        func byte(_ v: CGFloat) -> Int { Int((min(max(v, 0), 1) * 255).rounded()) }
        return String(format: "#%02x%02x%02x", byte(r), byte(g), byte(b))
    }
}

/// Lit-or-off chips that toggle values of a set setting (hidden event kinds, badge services,
/// mark kinds). `lit` decides whether a chip shows as on.
struct SettingChips: View {
    @Environment(SettingsStore.self) private var store
    let path: String
    let chips: [Choice]
    /// True when a stored set lists what is OFF (lit = not in the set).
    var storesOff = true
    /// Each chip leads with its provider's logo (the chip value is the provider id).
    var logos = false

    var body: some View {
        let stored = Set(store.strings(path))
        FlowLayout(spacing: 6) {
            ForEach(chips) { chip in
                let lit = storesOff ? !stored.contains(chip.value) : stored.contains(chip.value)
                Button {
                    Task { _ = try? await store.edit(path, .toggle(chip.value)) }
                } label: {
                    HStack(spacing: 5) {
                        if logos { BrandLogo(id: chip.value, size: 14).opacity(lit ? 1 : 0.45) }
                        Text(chip.label)
                    }
                        .font(.sn(12, 500))
                        .foregroundStyle(lit ? SN.textPrimary : SN.textMuted)
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background {
                            Capsule().fill(lit ? FixedColor.slate.color.opacity(0.18) : SN.surface.opacity(0.5))
                        }
                        .overlay { Capsule().stroke(SN.borderSubtle, lineWidth: lit ? 0 : 0.5) }
                        .frame(minHeight: 32)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(chip.label)
                .accessibilityAddTraits(lit ? .isSelected : [])
            }
        }
    }
}

/// A plain paragraph at the top of a panel.
struct PanelIntro: View {
    let text: String

    var body: some View {
        Text(text).font(.sn(14)).foregroundStyle(SN.textSecondary).fixedSize(horizontal: false, vertical: true)
    }
}

/// A full-width action button in a settings panel: the app's one 50 point worded button, glass
/// unless a tint gives it a color.
struct SettingButton: View {
    let title: String
    var symbol: String?
    var role: ButtonRole?
    var tint: Color?
    var enabled = true
    let action: () -> Void

    var body: some View {
        Button(role: role, action: action) {
            HStack(spacing: 6) {
                if let symbol { Image(systemName: symbol).font(.system(size: 14, weight: .semibold)) }
                Text(title)
            }
        }
        .buttonStyle(tint.map { SNButtonStyle(kind: .brand($0)) } ?? SNButtonStyle(kind: .secondary))
        .disabled(!enabled)
    }
}
