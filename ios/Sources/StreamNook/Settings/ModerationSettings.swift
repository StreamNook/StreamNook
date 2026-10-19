import SNKit
import SwiftUI

/// Settings > Moderation: swiping a message to moderate it (on or off, its look, its reach
/// and its steps), the four timeout lengths on a user card (the desktop dock's first four, so one
/// edit changes both) and the viewer's own card buttons. The core keeps every list
/// (`edit_mod_slider_timeouts`, `set_card_timeout_length`, `edit_settings_list`) and reads typed
/// lengths (`parse_mod_timeout`); the page sends one change at a time.
struct ModerationSettings: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var refusal: String?
    @State private var stepsRefusal: String?
    /// A "Custom length…" prompt: where its answer goes, and what was typed.
    @State private var typing: TypedLengthTarget?
    @State private var typed = ""
    @State private var typedRefusal: String?

    enum TypedLengthTarget: Equatable {
        case step
        case card(Int)
    }

    static let path = "moderation.card_actions"
    /// Lengths offered for a chip or a timeout button, shortest first.
    static let lengths: [(Int, String)] = [
        (1, "1s"), (30, "30s"), (60, "1m"), (300, "5m"), (600, "10m"), (1800, "30m"), (3600, "1h"), (21600, "6h"),
        (43200, "12h"), (86400, "1d"), (259_200, "3d"), (604_800, "1w"), (1_209_600, "2w"),
    ]

    /// A length as every step names it: one unit when it divides evenly ("10m", "1d"), else the two
    /// largest parts ("1h 30m"), as the core's `duration_label` writes it.
    static func label(_ seconds: Int) -> String {
        let units: [(Int, String)] = [(604_800, "w"), (86_400, "d"), (3_600, "h"), (60, "m"), (1, "s")]
        if let (size, unit) = units.first(where: { seconds >= $0.0 }), seconds % size == 0 {
            return "\(seconds / size)\(unit)"
        }
        var parts: [String] = []
        var rest = seconds
        for (size, unit) in units where rest >= size && parts.count < 2 {
            parts.append("\(rest / size)\(unit)")
            rest %= size
        }
        return parts.isEmpty ? "0s" : parts.joined(separator: " ")
    }

    private var stepTimeouts: [Int] {
        (store.value("chat_design.mod_slider_timeouts") as? [Any] ?? []).compactMap { SettingsPath.double($0).map { Int($0) } }
    }

    /// The four as the card shows them: the saved dock list's first four, topped up from the
    /// defaults (the core's `lengths_from` decides the same).
    private var four: [Int] {
        var out: [Int] = []
        let saved = (store.value("moderation.timeout_presets") as? [Any] ?? []).compactMap { SettingsPath.double($0).map { Int($0) } }
        for s in saved + [1, 600, 3600, 86400] where (1...1_209_600).contains(s) && !out.contains(s) {
            out.append(s)
            if out.count == 4 { break }
        }
        return out
    }

    var body: some View {
        swipeBlock
        cardBlock
            .alert("Custom length", isPresented: Binding(get: { typing != nil }, set: { if !$0 { typing = nil } })) {
                TextField("45m", text: $typed)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                Button("Add") { Task { await submitTyped() } }
                Button("Cancel", role: .cancel) { typing = nil }
            } message: {
                Text(typedRefusal ?? "Type a length like 45m, 2h or 3d, up to 14 days.")
            }
    }

    // MARK: Moderating messages: the swipe and the hold menu, each on or off

    private var swipeBlock: some View {
        SettingsBlock(title: "Moderating Messages", detail: "Two ways to delete, time out or ban from a message in a channel you moderate. With both off, the user card’s Moderate row still has every action.") {
            ToggleRow(title: "Swipe to moderate", path: "chat_design.mod_slider", fallback: true,
                      detail: "Swipe right on a message; pull further for a longer timeout. Ban needs a short hold.") {
                NestedChoice(title: "Style", path: "chat_design.mod_slider_style",
                             choices: [Choice("Preview", "preview"), Choice("Rail", "rail"), Choice("Capsule", "capsule")],
                             fallback: "preview")
                NestedChoice(title: "Length", path: "chat_design.mod_slider_length",
                             choices: [Choice("Short", "short"), Choice("Medium", "medium"), Choice("Long", "long")],
                             fallback: "medium")
            }
            if store.bool("chat_design.mod_slider", true) {
            WideRow(title: "Steps", detail: "What the swipe offers, mildest first. Tap a length to take it out.",
                    paths: ["chat_design.mod_slider_delete", "chat_design.mod_slider_timeouts", "chat_design.mod_slider_ban"]) {
                VStack(alignment: .leading, spacing: 8) {
                    FlowLayout(spacing: 7, lineSpacing: 7) {
                        stepChip("Delete", on: store.bool("chat_design.mod_slider_delete", true)) {
                            Task { await store.set("chat_design.mod_slider_delete", !store.bool("chat_design.mod_slider_delete", true)) }
                        }
                        ForEach(stepTimeouts, id: \.self) { seconds in
                            stepChip(Self.label(seconds), on: true, removable: stepTimeouts.count > 1) {
                                editStep("remove", seconds)
                            }
                        }
                        if stepTimeouts.count < 8 {
                            Menu {
                                Button("Custom length\u{2026}") { typed = ""; typedRefusal = nil; typing = .step }
                                ForEach(Self.lengths.filter { !stepTimeouts.contains($0.0) }, id: \.0) { length, label in
                                    Button(label) { editStep("add", length) }
                                }
                            } label: {
                                Label("Add", systemImage: "plus")
                                    .font(.sn(13, 500))
                                    .foregroundStyle(SN.accent)
                                    .padding(.horizontal, 12)
                                    .frame(minHeight: 32)
                                    .overlay(Capsule().strokeBorder(SN.accent.opacity(0.4), style: StrokeStyle(lineWidth: 1, dash: [3, 3])))
                            }
                        }
                        stepChip("Ban", on: store.bool("chat_design.mod_slider_ban", true), danger: true) {
                            Task { await store.set("chat_design.mod_slider_ban", !store.bool("chat_design.mod_slider_ban", true)) }
                        }
                    }
                    if let stepsRefusal {
                        Text(stepsRefusal).font(.sn(12)).foregroundStyle(SN.error)
                    }
                }
                .padding(.top, 10)
            }
            }
            ToggleRow(title: "Hold menu buttons", path: "chat_design.mod_hold_buttons", fallback: true,
                      detail: "Delete, Timeout and Ban in the press-and-hold menu.")
        }
    }

    private func stepChip(_ title: String, on: Bool, removable: Bool = false, danger: Bool = false,
                          action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 4) {
                Text(title).font(.sn(13, 600))
                if removable { Image(systemName: "xmark").font(.system(size: 9, weight: .bold)).foregroundStyle(SN.textMuted) }
            }
            .foregroundStyle(on ? (danger ? SN.error : SN.textPrimary) : SN.textMuted)
            .padding(.horizontal, 12)
            .frame(minHeight: 32)
            .background(on ? (danger ? SN.error.opacity(0.14) : SN.surface) : Color.clear, in: Capsule())
            .overlay(Capsule().strokeBorder(on ? Color.clear : SN.textMuted.opacity(0.4)))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(removable ? "Remove \(title)" : "\(title), \(on ? "on" : "off")")
    }

    private func editStep(_ op: String, _ seconds: Int) {
        Task {
            do {
                _ = try await model.core.editSliderTimeouts(op, seconds: seconds)
                stepsRefusal = nil
                await store.refresh()
            } catch {
                stepsRefusal = ChatComposerModel.words(error)
            }
        }
    }

    /// Reads the typed length through the core and sends it where it was asked for; a refusal
    /// keeps the prompt open with the core's words.
    private func submitTyped() async {
        guard let target = typing else { return }
        do {
            let step = try await model.core.parseTimeout(typed)
            guard let seconds = step.seconds else { return }
            typing = nil
            switch target {
            case .step: editStep("add", seconds)
            case .card(let index): setLength(index, seconds)
            }
        } catch {
            typedRefusal = ChatComposerModel.words(error)
            typing = nil
            // Ask again with the reason, keeping what was typed.
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.35) { typing = target }
        }
    }

    // MARK: User card

    private var cardBlock: some View {
        SettingsBlock(title: "User Cards", detail: "The buttons on a user card in channels you moderate.") {
            WideRow(title: "Timeout lengths", detail: "Tap one to change it. Every quick timeout uses the same four.",
                    paths: ["moderation.timeout_presets"]) {
                HStack(spacing: 7) {
                    ForEach(Array(four.enumerated()), id: \.offset) { index, seconds in
                        Menu {
                            Button("Custom length\u{2026}") { typed = ""; typedRefusal = nil; typing = .card(index) }
                            ForEach(Self.lengths, id: \.0) { length, label in
                                Button(label) { setLength(index, length) }
                                    .disabled(four.contains(length) && length != seconds)
                            }
                        } label: {
                            Text(Self.label(seconds))
                                .font(.sn(14, 600))
                                .foregroundStyle(SN.textPrimary)
                                .frame(maxWidth: .infinity, minHeight: 36)
                                .background(SN.surface, in: .rect(cornerRadius: 10, style: .continuous))
                        }
                        .accessibilityLabel("Timeout length \(index + 1): \(Self.label(seconds))")
                    }
                }
                .padding(.top, 10)
                if let refusal {
                    Text(refusal).font(.sn(12)).foregroundStyle(SN.error).padding(.top, 6)
                }
            }
            WideRow(title: "Card buttons", detail: "Your own buttons under the lengths: a timeout, ban, unban or warning, with a reason sent along.",
                    paths: [Self.path]) {
                VStack(spacing: 8) {
                    ForEach(Array(store.list(Self.path).enumerated()), id: \.offset) { _, entry in
                        if let id = entry["id"] as? String {
                            CardActionEditor(id: id, entry: entry)
                        }
                    }
                    if store.list(Self.path).count < 6 {
                        Button {
                            Task { _ = try? await store.edit(Self.path, .add()) }
                        } label: {
                            Label("Add a button", systemImage: "plus")
                                .font(.sn(14, 500))
                                .foregroundStyle(SN.accent)
                                .frame(maxWidth: .infinity, minHeight: 36, alignment: .leading)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.top, 10)
            }
        }
    }

    private func setLength(_ index: Int, _ seconds: Int) {
        Task {
            do {
                _ = try await model.core.invoke("set_card_timeout_length", args: ["index": index, "seconds": seconds])
                refusal = nil
                await store.refresh()
            } catch {
                refusal = ChatComposerModel.words(error)
            }
        }
    }
}

/// One card button: its label, what it does, its length (timeouts) and the reason sent with it.
private struct CardActionEditor: View {
    @Environment(SettingsStore.self) private var store
    let id: String
    let entry: [String: Any]

    private var kind: String { entry["kind"] as? String ?? "timeout" }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                EntryTextField(listPath: ModerationSettings.path, id: id, field: "label",
                               value: entry["label"] as? String ?? "", placeholder: "Label, like Spoilers", limit: 24)
                EntryDelete(label: "Remove this button") {
                    Task { _ = try? await store.edit(ModerationSettings.path, .remove(id: id)) }
                }
            }
            HStack(spacing: 8) {
                Menu {
                    ForEach([("timeout", "Timeout"), ("ban", "Ban"), ("unban", "Unban"), ("warn", "Warning")], id: \.0) { value, title in
                        Button(title) { update(["kind": value]) }
                    }
                } label: {
                    menuLabel(["timeout": "Timeout", "ban": "Ban", "unban": "Unban", "warn": "Warning"][kind] ?? "Timeout")
                }
                if kind == "timeout" {
                    let seconds = SettingsPath.double(entry["seconds"]).map { Int($0) } ?? 600
                    Menu {
                        ForEach(ModerationSettings.lengths, id: \.0) { length, label in
                            Button(label) { update(["seconds": length]) }
                        }
                    } label: {
                        menuLabel(ModerationSettings.label(seconds))
                    }
                }
                Spacer(minLength: 0)
            }
            EntryTextField(listPath: ModerationSettings.path, id: id, field: "reason", value: entry["reason"] as? String ?? "",
                           placeholder: kind == "warn" ? "Reason (asked each time when empty)" : "Reason (optional)", limit: 500)
        }
        .padding(10)
        .snPanel()
    }

    private func update(_ patch: [String: Any?]) {
        Task { _ = try? await store.edit(ModerationSettings.path, .update(id: id, patch)) }
    }

    private func menuLabel(_ text: String) -> some View {
        HStack(spacing: 4) {
            Text(text).font(.sn(13, 500))
            Image(systemName: "chevron.up.chevron.down").font(.system(size: 10, weight: .semibold))
        }
        .foregroundStyle(SN.textPrimary)
        .padding(.horizontal, 10)
        .frame(minHeight: 32)
        .background(SN.surface, in: .rect(cornerRadius: 8, style: .continuous))
    }
}
