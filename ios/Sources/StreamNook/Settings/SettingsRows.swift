import SNKit
import SwiftUI

// Whole rows over one setting each, built from the kit: a switch, a segmented or menu choice,
// a slider whose title carries its value, a color. Nested content shows only while it means
// something (by default, while a switch is on).

/// A row with a switch on the right; `nested` shows while it is on.
struct ToggleRow<Nested: View>: View {
    @Environment(SettingsStore.self) private var store
    let title: String
    let path: String
    var fallback = false
    var detail: String?
    var help: String?
    /// The detail when it depends on the state.
    var detailFor: ((Bool) -> String)?
    var write: ((Bool) -> [String: Any?])?
    var isOn: ((SettingsStore) -> Bool)?
    var onChange: ((Bool) async -> Void)?
    var showsNestedWhenOff = false
    var logo: String?
    @ViewBuilder var nested: Nested

    var body: some View {
        let on = isOn?(store) ?? store.bool(path, fallback)
        SettingRow(title: title, detail: detailFor?(on) ?? detail, help: help, paths: [path], logo: logo,
                   afterReset: onChange.map { change in { await change(store.bool(path, fallback)) } }) {
            SettingToggle(path: path, fallback: fallback, label: title, write: write, isOn: isOn, onChange: onChange)
        } nested: {
            if on || showsNestedWhenOff {
                NestedSettings { nested }
            }
        }
    }
}

extension ToggleRow where Nested == EmptyView {
    init(title: String, path: String, fallback: Bool = false, detail: String? = nil, help: String? = nil,
         detailFor: ((Bool) -> String)? = nil, write: ((Bool) -> [String: Any?])? = nil,
         isOn: ((SettingsStore) -> Bool)? = nil, onChange: ((Bool) async -> Void)? = nil, logo: String? = nil) {
        self.init(title: title, path: path, fallback: fallback, detail: detail, help: help, detailFor: detailFor,
                  write: write, isOn: isOn, onChange: onChange, logo: logo, nested: { EmptyView() })
    }
}

/// A row whose control is a segmented choice (or a menu, for long lists) under it.
struct ChoiceRow<Nested: View>: View {
    @Environment(SettingsStore.self) private var store
    let title: String
    let path: String
    let choices: [Choice]
    var fallback: String?
    var menu = false
    /// The setting is a number: the picked choice is stored as one, never as its text.
    var numeric = false
    var detail: String?
    var help: String?
    var detailFor: ((String?) -> String)?
    var accessibility: String?
    var shown: ((String?) -> String?)?
    var resetPaths: [String]?
    @ViewBuilder var nested: Nested

    var body: some View {
        let raw = store.value(path).map { "\($0)" } ?? fallback
        WideRow(title: title, detail: detailFor?(raw) ?? detail, help: help, paths: resetPaths ?? [path]) {
            Group {
                if menu {
                    SettingMenu(path: path, choices: choices, fallback: fallback, accessibility: accessibility ?? title,
                                numeric: numeric)
                } else {
                    SettingSegmentsBound(path: path, choices: choices, fallback: fallback,
                                         accessibility: accessibility ?? title, shown: shown, numeric: numeric)
                }
            }
            .padding(.top, 12)
        } nested: {
            nestedBlock
        }
    }

    @ViewBuilder private var nestedBlock: some View {
        if Nested.self != EmptyView.self {
            NestedSettings { nested }
        }
    }
}

extension ChoiceRow where Nested == EmptyView {
    init(title: String, path: String, choices: [Choice], fallback: String? = nil, menu: Bool = false,
         numeric: Bool = false, detail: String? = nil, help: String? = nil, detailFor: ((String?) -> String)? = nil,
         accessibility: String? = nil, shown: ((String?) -> String?)? = nil, resetPaths: [String]? = nil) {
        self.init(title: title, path: path, choices: choices, fallback: fallback, menu: menu, numeric: numeric,
                  detail: detail, help: help, detailFor: detailFor, accessibility: accessibility, shown: shown,
                  resetPaths: resetPaths, nested: { EmptyView() })
    }
}

/// A row whose title carries the slider's value ("Spacing: 8px") with the slider under it.
struct SliderRow: View {
    @Environment(SettingsStore.self) private var store
    let title: (Double) -> String
    let path: String
    let range: ClosedRange<Double>
    let step: Double
    let fallback: Double
    var detail: String?
    var help: String?
    var integer = true
    var accessibility = ""
    @State private var live: Double?

    var body: some View {
        let stored = min(max(store.double(path, fallback), range.lowerBound), range.upperBound)
        WideRow(title: title(live ?? stored), detail: detail, help: help, paths: [path]) {
            SettingSlider(path: path, range: range, step: step, fallback: fallback, accessibility: accessibility,
                          integer: integer, live: $live)
                .padding(.top, 10)
        }
    }
}

/// A nested switch.
struct NestedToggle: View {
    let title: String
    let path: String
    var fallback = false
    var help: String?
    var logo: String?

    var body: some View {
        NestedLine(title: title, paths: [path], help: help, logo: logo) {
            SettingToggle(path: path, fallback: fallback, label: title)
        }
    }
}

/// A nested segmented choice, under its title.
struct NestedChoice: View {
    let title: String
    let path: String
    let choices: [Choice]
    var fallback: String?
    var menu = false
    var resetPaths: [String]?
    var shown: ((String?) -> String?)?

    var body: some View {
        NestedLine(title: title, paths: resetPaths ?? [path], stacked: true) {
            if menu {
                SettingMenu(path: path, choices: choices, fallback: fallback, accessibility: title)
            } else {
                SettingSegmentsBound(path: path, choices: choices, fallback: fallback, accessibility: title, shown: shown)
            }
        }
    }
}

/// A nested slider with its value beside it.
struct NestedSlider: View {
    let title: String
    let path: String
    let range: ClosedRange<Double>
    let step: Double
    let fallback: Double
    let readout: (Double) -> String
    var integer = true
    @State private var live: Double?

    @Environment(SettingsStore.self) private var store

    var body: some View {
        let stored = min(max(store.double(path, fallback), range.lowerBound), range.upperBound)
        VStack(alignment: .leading, spacing: 8) {
            NestedLine(title: title, paths: [path]) {
                SliderReadout(text: readout(live ?? stored))
            }
            SettingSlider(path: path, range: range, step: step, fallback: fallback, accessibility: title,
                          integer: integer, live: $live)
        }
    }
}

/// A nested color.
struct NestedColor: View {
    let title: String
    let path: String
    let unset: String

    var body: some View {
        NestedLine(title: title, paths: [path]) {
            SettingColor(path: path, unset: unset, label: title)
        }
    }
}

/// A row with a color swatch on the right.
struct ColorRow<Nested: View>: View {
    let title: String
    let path: String
    let unset: String
    var detail: String?
    var help: String?
    @ViewBuilder var nested: Nested

    var body: some View {
        SettingRow(title: title, detail: detail, help: help, paths: [path]) {
            SettingColor(path: path, unset: unset, label: title)
        } nested: {
            if Nested.self != EmptyView.self { NestedSettings { nested } }
        }
    }
}

extension ColorRow where Nested == EmptyView {
    init(title: String, path: String, unset: String, detail: String? = nil, help: String? = nil) {
        self.init(title: title, path: path, unset: unset, detail: detail, help: help, nested: { EmptyView() })
    }
}

extension Double {
    /// "1.00x" style readouts.
    func fixed(_ digits: Int) -> String { String(format: "%.\(digits)f", self) }
}
