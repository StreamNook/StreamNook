import AVFoundation
import SNKit
import SwiftUI

// The Chat page's list editors: trusted link sites, the dictionary, hidden users and phrases,
// custom sounds, highlight rules, custom commands and nicknames. Every edit is one entry
// changed by the core (`edit_settings_list` and the hide/override commands), never a list the
// page rebuilt; nothing typed by you carries a reset arrow.

/// A text field over one field of a list entry, saved a moment after typing stops.
struct EntryTextField: View {
    @Environment(SettingsStore.self) private var store
    let listPath: String
    let id: String
    let field: String
    let value: String
    let placeholder: String
    var limit = 200
    var monospaced = false
    var multiline = false
    var label = ""
    @State private var text: String?
    @State private var save: Task<Void, Never>?

    var body: some View {
        let binding = Binding(get: { text ?? value }, set: { typed in
            let capped = String(typed.prefix(limit))
            text = capped
            save?.cancel()
            save = Task {
                try? await Task.sleep(for: .milliseconds(400))
                guard !Task.isCancelled else { return }
                _ = try? await store.edit(listPath, .update(id: id, [field: capped]))
                text = nil
            }
        })
        Group {
            if multiline {
                TextField(placeholder, text: binding, axis: .vertical).lineLimit(2...5).keyboardHideButton()
            } else {
                TextField(placeholder, text: binding).lineLimit(1)
            }
        }
        .font(monospaced ? .system(size: 13, design: .monospaced) : .sn(13))
        .foregroundStyle(SN.textPrimary)
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .padding(.horizontal, 10)
        .padding(.vertical, 8)
        .background { RoundedRectangle(cornerRadius: 8, style: .continuous).fill(SN.surface) }
        .accessibilityLabel(label.isEmpty ? placeholder : label)
    }
}

/// A switch over one boolean field of a list entry.
struct EntryToggle: View {
    @Environment(SettingsStore.self) private var store
    let listPath: String
    let id: String
    let field: String
    let on: Bool
    let label: String

    var body: some View {
        Toggle(label, isOn: Binding(get: { on }, set: { value in
            Task { _ = try? await store.edit(listPath, .update(id: id, [field: value])) }
        }))
        .labelsHidden()
        .tint(SN.accent)
        .controlSize(.mini)
        .accessibilityLabel(label)
    }
}

/// A color over one field of a list entry.
struct EntryColor: View {
    @Environment(SettingsStore.self) private var store
    let listPath: String
    let id: String
    let hex: String

    var body: some View {
        ColorPicker("Highlight color", selection: Binding(
            get: { RGBA(css: hex)?.color ?? .gray },
            set: { color in
                let text = SettingColor.hex(color)
                guard text != hex.lowercased() else { return }
                Task { _ = try? await store.edit(listPath, .update(id: id, ["color": text])) }
            }
        ), supportsOpacity: false)
        .labelsHidden()
        .frame(width: 32, height: 28)
    }
}

/// A trash button for a list entry.
struct EntryDelete: View {
    let label: String
    let action: () -> Void

    var body: some View {
        IconButton(symbol: "trash", label: label, size: 14, tint: SN.textMuted, action: action)
    }
}

/// A removable chip.
struct RemovableChip: View {
    let text: String
    var monospaced = false
    var detail: String?
    let removeLabel: String
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 4) {
            Text(text).font(monospaced ? .system(size: 12, design: .monospaced) : .sn(12)).foregroundStyle(SN.textPrimary)
            if let detail { Text(detail).font(.sn(11)).foregroundStyle(SN.textMuted) }
            Button(action: remove) {
                Image(systemName: "xmark").font(.system(size: 10, weight: .semibold)).foregroundStyle(SN.textMuted)
                    .frame(width: 28, height: 28).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(removeLabel)
        }
        .padding(.leading, 12)
        .background { Capsule().fill(SN.surface) }
    }
}

/// A text field with an Add button; Return adds too. `add` answers an error sentence or nil.
struct AddField: View {
    let label: String?
    let placeholder: String
    var monospaced = false
    var keyboard: UIKeyboardType = .default
    var limit = 200
    var buttonSymbol: String?
    let add: (String) async -> String?
    @State private var text = ""
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let label { Text(label).font(.sn(11)).foregroundStyle(SN.textSecondary) }
            HStack(spacing: 8) {
                TextField("", text: $text, prompt: Text(placeholder).foregroundStyle(SN.textMuted))
                    .font(monospaced ? .system(size: 13, design: .monospaced) : .sn(13))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(keyboard)
                    .keyboardHideButton()
                    .submitLabel(.done)
                    .onSubmit { commit() }
                    .onChange(of: text) { _, value in
                        if value.count > limit { text = String(value.prefix(limit)) }
                        error = nil
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 8)
                    .background { RoundedRectangle(cornerRadius: 8, style: .continuous).fill(SN.surface) }
                let empty = text.trimmingCharacters(in: .whitespaces).isEmpty
                Button(action: commit) {
                    Group {
                        if let buttonSymbol {
                            Image(systemName: buttonSymbol).font(.system(size: 14, weight: .semibold))
                        } else {
                            Text("Add").font(.sn(13, 600))
                        }
                    }
                    .foregroundStyle(empty ? SN.textMuted : SN.accent)
                    .padding(.horizontal, 12)
                    .frame(minWidth: 44, minHeight: 36)
                    .background {
                        RoundedRectangle(cornerRadius: 8, style: .continuous).fill(empty ? SN.surface : SN.accent.opacity(0.15))
                    }
                }
                .buttonStyle(.plain)
                .disabled(empty)
                .accessibilityLabel(buttonSymbol == nil ? "Add" : "Add word")
            }
            if let error { Text(error).font(.sn(12)).foregroundStyle(SN.error) }
        }
    }

    private func commit() {
        let value = text
        guard !value.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        Task {
            if let refused = await add(value) {
                error = refused
            } else {
                text = ""
            }
        }
    }
}

/// An empty list's card: a line and its first Add button.
struct EmptyListCard: View {
    let text: String
    let button: String
    let add: () -> Void

    var body: some View {
        VStack(spacing: 10) {
            Text(text).font(.sn(14)).foregroundStyle(SN.textSecondary)
            Button(action: add) {
                Label(button, systemImage: "plus").font(.sn(13, 600)).foregroundStyle(SN.accent)
                    .frame(minHeight: 36)
            }
            .buttonStyle(.plain)
        }
        .frame(maxWidth: .infinity)
        .padding(16)
        .snPanel()
    }
}

/// "Add another ..." under a list.
struct AddAnother: View {
    let title: String
    let add: () -> Void

    var body: some View {
        Button(action: add) {
            Label(title, systemImage: "plus").font(.sn(13, 600)).foregroundStyle(SN.accent).frame(minHeight: 36)
        }
        .buttonStyle(.plain)
    }
}

extension SettingsStore {
    /// Adds an entry, showing the core's sentence when it refuses.
    func add(_ path: String, _ item: Any? = nil) async -> String? {
        do {
            _ = try await edit(path, .add(item))
            return nil
        } catch {
            return "\(error)"
        }
    }
}

// MARK: Links

struct LinkSettings: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        SettingsBlock(title: "Links", detail: "How links in chat look.") {
            ToggleRow(title: "Short links", path: "chat_design.shorten_links", fallback: true,
                      detail: "Shows each link as a compact label, the site plus a short path, instead of the full raw URL.",
                      help: "The full link still opens on tap and shows when you press and hold.")
            ColorRow(title: "Color", path: "chat_design.link_color", unset: "#8ab4ff",
                     detail: "The color of links in chat. Leave it on the default to follow the theme.")
            ToggleRow(title: "Underline", path: "chat_design.link_underline", fallback: true,
                      detail: "Off leaves links colored but not underlined.")
        }
    }
}

struct HiddenMessagesSettings: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var exact = false
    @State private var errors: [String: String] = [:]

    var body: some View {
        SettingsBlock(title: "Hidden Messages", detail: "Keep chosen users, bots, commands and phrases out of your chat. Only affects what you see; nothing is sent to the platform, and your own messages are never hidden.") {
            ToggleRow(title: "Known bots", path: "chat_filters.hide_bots",
                      detail: "Hides StreamElements, Nightbot, Moobot and the other well-known chat bots, in every channel.")
            WideRow(title: "Everywhere", detail: "Messages from these names never appear, on any platform. Add someone here, or from their user card in chat.") {
                VStack(alignment: .leading, spacing: 8) {
                    AddField(label: "Username to hide", placeholder: "Type a username") { name in
                        await hide(name.trimmingCharacters(in: .whitespaces), channel: nil, hidden: true)
                        return nil
                    }
                    FlowLayout(spacing: 6) {
                        ForEach(store.strings("chat_filters.hidden_users"), id: \.self) { name in
                            RemovableChip(text: name, removeLabel: "Unhide \(name)") {
                                Task { await hide(name, channel: nil, hidden: false) }
                            }
                        }
                    }
                }
                .padding(.top, 10)
            }
            let perChannel = perChannelGroups
            if !perChannel.isEmpty {
                WideRow(title: "Per channel", detail: "Added from user cards while watching. Removing a name shows their messages again in that channel.") {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(perChannel, id: \.key) { group in
                            HStack(alignment: .top, spacing: 8) {
                                Text(group.label).font(.sn(12, 600)).foregroundStyle(SN.textSecondary)
                                    .frame(minWidth: 96, alignment: .leading).padding(.top, 6)
                                FlowLayout(spacing: 6) {
                                    ForEach(group.names, id: \.self) { name in
                                        RemovableChip(text: name, removeLabel: "Unhide \(name) in \(group.label)") {
                                            Task { await hide(name, channel: group.key, hidden: false) }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    .padding(.top, 10)
                }
            }
            ToggleRow(title: "Bot commands", path: "chat_filters.hide_commands",
                      detail: "Hides messages that are bot commands, so a chat full of !drops and !uptime reads as a chat.",
                      help: "Patterns: a prefix hides every command starting with it; an exact pattern hides only that word at the start of a message. With no patterns, anything starting with ! is hidden. Your own messages are never hidden.") {
                NestedLine(title: "Patterns", stacked: true) { patterns }
            }
            WideRow(title: "Ignored phrases", detail: "Never see messages that contain these words, in any chat. The sender is not told. Plain words work; turn on regex or whole-word per phrase when you need precision.") {
                ignoredPhrases.padding(.top, 10)
            }
        }
        .task(id: store.list("chat_filters.ignored_phrases").count) { await readErrors() }
    }

    private var patterns: some View {
        let path = "chat_filters.command_filters"
        let list = store.list(path)
        return VStack(alignment: .leading, spacing: 8) {
            if list.isEmpty {
                Text("Using the default: anything starting with !").font(.sn(12)).foregroundStyle(SN.textMuted)
            } else {
                FlowLayout(spacing: 6) {
                    ForEach(Array(list.enumerated()), id: \.offset) { index, filter in
                        let value = filter["value"] as? String ?? ""
                        RemovableChip(text: value, monospaced: true, detail: (filter["mode"] as? String) == "exact" ? "exact" : "prefix",
                                      removeLabel: "Remove") {
                            Task { _ = try? await store.edit(path, .removeAt(index)) }
                        }
                    }
                }
            }
            SettingSegments(choices: [Choice("Prefix", "prefix"), Choice("Exact", "exact")], selected: exact ? "exact" : "prefix") {
                exact = $0 == "exact"
            }
            .frame(maxWidth: 200)
            AddField(label: nil, placeholder: "!", monospaced: true, limit: 40) { text in
                await store.add(path, ["value": text, "mode": exact ? "exact" : "prefix"])
            }
        }
    }

    private var ignoredPhrases: some View {
        let path = "chat_filters.ignored_phrases"
        return VStack(alignment: .leading, spacing: 8) {
            ForEach(store.list(path).indices, id: \.self) { index in
                let phrase = store.list(path)[index]
                if let id = phrase["id"] as? String {
                    PhraseCard(listPath: path, id: id, phrase: phrase, error: errors["ignore:\(id)"] ?? errors[id], ignore: true)
                }
            }
            AddAnother(title: "Add phrase") { Task { _ = await store.add(path) } }
        }
    }

    private struct ChannelGroup {
        let key: String
        let label: String
        let names: [String]
    }

    private var perChannelGroups: [ChannelGroup] {
        let per = store.value("chat_filters.per_channel") as? [String: Any] ?? [:]
        return per.compactMap { key, value -> ChannelGroup? in
            guard let names = value as? [String], !names.isEmpty else { return nil }
            let parts = key.split(separator: ":", maxSplits: 1).map(String.init)
            let label = parts.count == 2 ? (parts[0] == "twitch" ? parts[1] : "\(parts[1]) (\(parts[0].capitalized))") : key
            return ChannelGroup(key: key, label: label, names: names)
        }
        .sorted { $0.label < $1.label }
    }

    private func hide(_ name: String, channel: String?, hidden: Bool) async {
        guard !name.isEmpty else { return }
        do {
            _ = try await model.core.invoke("set_chat_user_hidden", args: ["name": name, "channelKey": channel, "hidden": hidden])
        } catch {
            log.error("set_chat_user_hidden failed: \(String(describing: error), privacy: .public)")
        }
        await store.refresh()
    }

    private func readErrors() async {
        errors = await RuleErrors.read(model.core)
    }
}

/// The compile errors the chat rule engine kept from its last refresh, by `scope:id` and id.
enum RuleErrors {
    static func read(_ core: CoreBridge) async -> [String: String] {
        guard let json = try? await core.invoke("get_chat_rule_errors", args: [:]),
              let list = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[String: Any]]
        else { return [:] }
        var out: [String: String] = [:]
        for entry in list {
            guard let id = entry["id"] as? String, let error = entry["error"] as? String else { continue }
            out[id] = error
            if let scope = entry["scope"] as? String { out["\(scope):\(id)"] = error }
        }
        return out
    }
}

/// One highlight or ignored phrase: switch, pattern, options, and (for highlights) color,
/// order, sound and cooldown.
struct PhraseCard: View {
    @Environment(SettingsStore.self) private var store
    let listPath: String
    let id: String
    let phrase: [String: Any]
    let error: String?
    var ignore = false
    var index = 0
    var count = 1

    var body: some View {
        let regex = phrase["is_regex"] as? Bool ?? false
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                EntryToggle(listPath: listPath, id: id, field: "enabled", on: phrase["enabled"] as? Bool ?? true,
                            label: (phrase["enabled"] as? Bool ?? true) ? "Disable phrase" : "Enable phrase")
                EntryTextField(listPath: listPath, id: id, field: "pattern", value: phrase["pattern"] as? String ?? "",
                               placeholder: regex ? "Regular expression" : (ignore ? "Word or phrase to hide" : "Word or phrase"),
                               monospaced: regex)
                if !ignore {
                    EntryColor(listPath: listPath, id: id, hex: phrase["color"] as? String ?? "#fbbf24")
                }
            }
            HStack(spacing: 6) {
                option("Aa", "case_sensitive", label: "Case sensitive")
                option("word", "whole_word", label: "Match whole words only", disabled: regex && !ignore)
                option("regex", "is_regex", label: "Treat as a regular expression")
                Spacer()
                if !ignore {
                    IconButton(symbol: "chevron.up", label: "Move up", size: 13, tint: SN.textMuted) {
                        Task { _ = try? await store.edit(listPath, .move(id: id, to: index - 1)) }
                    }
                    .disabled(index == 0).opacity(index == 0 ? 0.3 : 1)
                    IconButton(symbol: "chevron.down", label: "Move down", size: 13, tint: SN.textMuted) {
                        Task { _ = try? await store.edit(listPath, .move(id: id, to: index + 1)) }
                    }
                    .disabled(index >= count - 1).opacity(index >= count - 1 ? 0.3 : 1)
                }
                EntryDelete(label: ignore ? "Remove phrase" : "Delete phrase") {
                    Task { _ = try? await store.edit(listPath, .remove(id: id)) }
                }
            }
            if let error {
                Text(error).font(.sn(12)).foregroundStyle(SN.error)
            }
        }
        .padding(10)
        .snPanel()
    }

    private func option(_ text: String, _ field: String, label: String, disabled: Bool = false) -> some View {
        let on = (phrase[field] as? Bool ?? false) && !disabled
        return Button {
            Task { _ = try? await store.edit(listPath, .update(id: id, [field: !(phrase[field] as? Bool ?? false)])) }
        } label: {
            Text(text).font(.sn(11, 600))
                .foregroundStyle(on ? SN.accent : SN.textSecondary)
                .padding(.horizontal, 8).padding(.vertical, 4)
                .background { RoundedRectangle(cornerRadius: 4).fill(on ? SN.accent.opacity(0.25) : SN.surface) }
                .frame(minHeight: 32).contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.5 : 1)
        .accessibilityLabel(label)
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

// MARK: Highlights

struct HighlightPhrasesSettings: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model
    @State private var errors: [String: String] = [:]
    private let path = "chat_highlights.phrases"

    var body: some View {
        let phrases = store.list(path)
        SettingsBlock(title: "Highlight Phrases", detail: "Flash chat messages that match specific words, names, or patterns. Mentions of your own name and replies to you are always highlighted; these are extra.", bare: true) {
            if phrases.isEmpty {
                EmptyListCard(text: "No highlight phrases yet.", button: "Add your first phrase") { Task { _ = await store.add(path) } }
            } else {
                ForEach(phrases.indices, id: \.self) { index in
                    if let id = phrases[index]["id"] as? String {
                        PhraseCard(listPath: path, id: id, phrase: phrases[index], error: errors["highlight:\(id)"] ?? errors[id],
                                   index: index, count: phrases.count)
                    }
                }
                AddAnother(title: "Add another phrase") { Task { _ = await store.add(path) } }
            }
        }
        .task(id: phrases.map { "\($0["pattern"] ?? "")\($0["is_regex"] ?? "")" }.joined()) {
            errors = await RuleErrors.read(model.core)
        }
    }
}

// MARK: Event Highlights

struct EventHighlightsSettings: View {
    @Environment(SettingsStore.self) private var store

    var body: some View {
        SettingsBlock(title: "Event Highlights", detail: "Highlight messages from certain kinds of chatters. Works alongside your phrase highlights and never changes the mention or reply flash.") {
            rule("first_time_chatter", "First-time chatters",
                 "Highlight a user's first ever message in this channel.",
                 help: "Style is a tinted wash with a bar down the left, or a ring around the message; Ring fill adds a faint color-matched fill inside it. Glint is a short highlight as the row lands (a sheen, a pulse, or a spark around the edge), and Loop keeps it going.")
            rule("returning_chatter", "Returning chatters", "Highlight users coming back after a long absence.")
            rule("self_message", "Your own messages", "Tint your own outgoing messages so they're easy to spot in fast chats.")
            rule("raider", "Raid announcements", "Highlight the \"X is raiding with Y viewers\" notice row when a raid lands.")
        }
    }

    private func rule(_ key: String, _ title: String, _ detail: String, help: String? = nil) -> some View {
        let base = "chat_highlights.built_in.\(key)"
        return SettingRow(title: title, detail: detail, help: help, paths: ["\(base).enabled", "\(base).color"]) {
            SettingToggle(path: "\(base).enabled", label: title)
        } nested: {
            if store.bool("\(base).enabled", true) {
                NestedSettings {
                    NestedColor(title: "Color", path: "\(base).color", unset: "#a855f7")
                    if key == "first_time_chatter" {
                    NestedChoice(title: "Style", path: "\(base).style", choices: [Choice("Wash", "wash"), Choice("Ring", "ring")], fallback: "wash")
                    if store.string("\(base).style", "wash") == "ring" {
                        NestedToggle(title: "Ring fill", path: "\(base).fill")
                    }
                    NestedLine(title: "Glint", paths: ["\(base).animation"]) {
                        SettingMenu(path: "\(base).animation",
                                    choices: [Choice("None", "none"), Choice("Sheen", "sheen"), Choice("Pulse", "pulse"), Choice("Chase", "chase")],
                                    fallback: "none", accessibility: "First-time glint", fullWidth: false)
                    }
                    if store.string("\(base).animation", "none") != "none" {
                        NestedToggle(title: "Loop", path: "\(base).animate_repeat")
                    }
                    }
                }
            }
        }
    }
}

// MARK: Username and Badge Highlights

struct UserHighlightsSettings: View {
    @Environment(SettingsStore.self) private var store
    private let path = "chat_highlights.users"

    var body: some View {
        let users = store.list(path)
        SettingsBlock(title: "Username Highlights", detail: "Always highlight messages from specific users by login. Match is case-insensitive.", bare: true) {
            if users.isEmpty {
                EmptyListCard(text: "No user highlights yet.", button: "Add your first user") { Task { _ = await store.add(path) } }
            } else {
                ForEach(users.indices, id: \.self) { index in
                    let user = users[index]
                    if let id = user["id"] as? String {
                        HStack(spacing: 8) {
                            EntryToggle(listPath: path, id: id, field: "enabled", on: user["enabled"] as? Bool ?? true,
                                        label: (user["enabled"] as? Bool ?? true) ? "Disable user highlight" : "Enable user highlight")
                            EntryTextField(listPath: path, id: id, field: "username", value: user["username"] as? String ?? "",
                                           placeholder: "twitch_login")
                            EntryColor(listPath: path, id: id, hex: user["color"] as? String ?? "#22d3ee")
                            EntryDelete(label: "Delete user highlight") { Task { _ = try? await store.edit(path, .remove(id: id)) } }
                        }
                        .padding(10).snPanel()
                    }
                }
                AddAnother(title: "Add another user") { Task { _ = await store.add(path) } }
            }
        }
    }
}

struct BadgeHighlightsSettings: View {
    @Environment(SettingsStore.self) private var store
    private let path = "chat_highlights.badges"

    static let presets: [(label: String, key: String, color: String)] = [
        ("Broadcaster", "broadcaster/1", "#ef4444"), ("Moderator", "moderator/1", "#22c55e"), ("VIP", "vip/1", "#ec4899"),
        ("Any subscriber", "subscriber/*", "#a855f7"), ("Twitch staff", "staff/1", "#f59e0b"), ("Twitch admin", "admin/1", "#f97316"),
        ("Partner", "partner/1", "#06b6d4"),
    ]

    var body: some View {
        let badges = store.list(path)
        let present = Set(badges.compactMap { ($0["badge_key"] as? String)?.lowercased() })
        SettingsBlock(title: "Badge Highlights", detail: "Highlight every message from users carrying a specific Twitch badge. Use name/version (e.g. moderator/1) or name/* to match any version.", bare: true) {
            FlowLayout(spacing: 6) {
                ForEach(Self.presets, id: \.key) { preset in
                    let has = present.contains(preset.key)
                    Button {
                        Task { _ = await store.add(path, ["badge_key": preset.key, "color": preset.color, "label": preset.label]) }
                    } label: {
                        Label(preset.label, systemImage: "plus").font(.sn(11, 500)).foregroundStyle(SN.textSecondary)
                            .padding(.horizontal, 10).padding(.vertical, 5)
                            .background { Capsule().fill(SN.surface) }
                            .frame(minHeight: 32)
                    }
                    .buttonStyle(.plain)
                    .disabled(has)
                    .opacity(has ? 0.4 : 1)
                }
            }
            if badges.isEmpty {
                EmptyListCard(text: "No badge highlights yet.", button: "Add a custom badge") { Task { _ = await store.add(path) } }
            } else {
                ForEach(badges.indices, id: \.self) { index in
                    let badge = badges[index]
                    if let id = badge["id"] as? String {
                        HStack(spacing: 8) {
                            EntryToggle(listPath: path, id: id, field: "enabled", on: badge["enabled"] as? Bool ?? true,
                                        label: (badge["enabled"] as? Bool ?? true) ? "Disable badge highlight" : "Enable badge highlight")
                            EntryTextField(listPath: path, id: id, field: "badge_key", value: badge["badge_key"] as? String ?? "",
                                           placeholder: "moderator/1 or subscriber/*", monospaced: true)
                            EntryColor(listPath: path, id: id, hex: badge["color"] as? String ?? "#a855f7")
                            EntryDelete(label: "Delete badge highlight") { Task { _ = try? await store.edit(path, .remove(id: id)) } }
                        }
                        .padding(10).snPanel()
                    }
                }
                AddAnother(title: "Add custom badge") { Task { _ = await store.add(path) } }
            }
        }
    }
}

// MARK: Custom Commands

struct CustomCommandsSettings: View {
    @Environment(SettingsStore.self) private var store
    @State private var reference = false
    private let path = "chat_commands.user_commands"

    var body: some View {
        let commands = store.list(path)
        let triggers = commands.map { ($0["trigger"] as? String ?? "").lowercased() }
        SettingsBlock(title: "Custom Commands", accessory: AnyView(
            Button {
                withAnimation(SNMotion.run(.easeOut(duration: 0.2))) { reference.toggle() }
            } label: {
                HStack(spacing: 4) {
                    Image(systemName: "book").font(.system(size: 11))
                    Text(reference ? "Hide reference" : "Show reference")
                }
                .font(.sn(12, 500)).foregroundStyle(SN.textSecondary)
                .frame(minHeight: 32)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        ), bare: true) {
            Text(verbatim: "Define your own commands. A trigger of \"lurk\" with the text \"I'm lurking, BRB\" means typing /lurk sends that text instead.")
                .font(.sn(12)).foregroundStyle(SN.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if reference { CommandReference() }
            if commands.isEmpty {
                EmptyListCard(text: "No custom commands yet.", button: "Add your first command") { Task { _ = await store.add(path) } }
            } else {
                ForEach(commands.indices, id: \.self) { index in
                    let command = commands[index]
                    if let id = command["id"] as? String {
                        let trigger = triggers[index]
                        let duplicate = !trigger.isEmpty && triggers.prefix(index).contains(trigger)
                        commandCard(command, id: id, index: index, count: commands.count, duplicate: duplicate)
                    }
                }
                AddAnother(title: "Add another command") { Task { _ = await store.add(path) } }
            }
        }
    }

    private func commandCard(_ command: [String: Any], id: String, index: Int, count: Int, duplicate: Bool) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                EntryToggle(listPath: path, id: id, field: "enabled", on: command["enabled"] as? Bool ?? true,
                            label: (command["enabled"] as? Bool ?? true) ? "Disable command" : "Enable command")
                Text("/").font(.system(size: 14, design: .monospaced)).foregroundStyle(SN.textMuted)
                EntryTextField(listPath: path, id: id, field: "trigger", value: command["trigger"] as? String ?? "",
                               placeholder: "lurk", limit: 32, monospaced: true, label: "Trigger")
                IconButton(symbol: "chevron.up", label: "Move up", size: 13, tint: SN.textMuted) {
                    Task { _ = try? await store.edit(path, .move(id: id, to: index - 1)) }
                }
                .disabled(index == 0).opacity(index == 0 ? 0.3 : 1)
                IconButton(symbol: "chevron.down", label: "Move down", size: 13, tint: SN.textMuted) {
                    Task { _ = try? await store.edit(path, .move(id: id, to: index + 1)) }
                }
                .disabled(index >= count - 1).opacity(index >= count - 1 ? 0.3 : 1)
                EntryDelete(label: "Delete command") { Task { _ = try? await store.edit(path, .remove(id: id)) } }
            }
            Text("Message it sends").font(.sn(11, 600)).foregroundStyle(SN.textMuted)
            EntryTextField(listPath: path, id: id, field: "expansion", value: command["expansion"] as? String ?? "",
                           placeholder: "What this command sends to chat. Supports {1}, {2}, {*}, {user}, {channel}.", multiline: true)
            Text("Description (optional)").font(.sn(11, 600)).foregroundStyle(SN.textMuted)
            EntryTextField(listPath: path, id: id, field: "description", value: command["description"] as? String ?? "",
                           placeholder: "Shown next to the command in the picker", limit: 120)
            HStack {
                Text("Require leading /").font(.sn(12)).foregroundStyle(SN.textSecondary)
                Spacer()
                EntryToggle(listPath: path, id: id, field: "require_slash", on: command["require_slash"] as? Bool ?? true, label: "Require leading /")
            }
            HStack {
                Text("Also match at end of message").font(.sn(12)).foregroundStyle(SN.textSecondary)
                Spacer()
                EntryToggle(listPath: path, id: id, field: "also_match_suffix", on: command["also_match_suffix"] as? Bool ?? false,
                            label: "Also match at end of message")
            }
            if duplicate, let trigger = command["trigger"] as? String {
                Label("Another command already uses \"/\(trigger)\". Only the first will fire.", systemImage: "exclamationmark.triangle")
                    .font(.sn(12)).foregroundStyle(SN.warning)
            }
        }
        .padding(12)
        .snPanel()
    }
}

private struct CommandReference: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            part("Words from what they typed", [
                "Anything typed after your command gets numbered. If you make a command so and someone types /so jane is great, then {1} is jane, {2} is is, and {3} is great.",
                "For everything from a certain word onward, use {2+} (gives is great) or {*} (gives jane is great, same as {1+}).",
            ])
            part("Auto-filled info", [
                "These get replaced automatically. No typing needed.",
                "{user}  your display name",
                "{user.id}  your numeric Twitch ID",
                "{channel}  the channel you're watching",
                "{channel.id}  that channel's numeric Twitch ID",
                "{stream.title}  the streamer's title",
                "{stream.game}  the streamer's category",
                "{stream.uptime}  how long they've been live, like 2h 14m",
                "The stream ones come back empty when the channel is offline.",
            ])
            part("Sending a literal { or }", ["Double them up. {{ sends a single {, and }} sends a single }."])
            part("Examples", [
                "Trigger so with expansion shoutout to @{1}, go follow them at twitch.tv/{1}. Typing /so jane sends shoutout to @jane, go follow them at twitch.tv/jane.",
                "Trigger title with expansion {stream.title} (playing {stream.game}, live for {stream.uptime}). Typing /title sends the current title, game, and uptime. No words needed after the trigger.",
                "Trigger day with expansion Have a {1+} day!. Typing /day really really great sends Have a really really great day!",
            ])
            part("The two checkboxes on each command", [
                "Require leading / is on by default. Turn it off to fire the trigger from plain messages with no slash. A trigger of lurk with this off means just typing lurk sends your message.",
                "Also match at end of message additionally fires when the trigger is the last word of a message. Handy for catchphrase shortcuts at the end of what you're saying.",
            ])
        }
        .padding(12)
        .snPanel()
    }

    private func part(_ title: String, _ lines: [String]) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.sn(12, 600)).foregroundStyle(SN.textPrimary)
            ForEach(lines, id: \.self) { line in
                Text(line).font(.sn(12)).foregroundStyle(SN.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// Plays a custom sound file once.
@MainActor
enum SoundPlayer {
    private static var player: AVAudioPlayer?

    static func play(_ path: String?) {
        guard let path else { return }
        try? AVAudioSession.sharedInstance().setCategory(.ambient, options: .mixWithOthers)
        player = try? AVAudioPlayer(contentsOf: URL(fileURLWithPath: path))
        player?.play()
    }
}

// MARK: Nicknames

struct NicknamesSettings: View {
    @Environment(SettingsStore.self) private var store
    @Environment(AppModel.self) private var model

    var body: some View {
        let overrides = (store.value("chat_customization.user_overrides") as? [String: [String: Any]] ?? [:])
            .map { (id: $0.key, entry: $0.value) }
            .sorted { (($0.entry["username"] as? String) ?? $0.id).lowercased() < (($1.entry["username"] as? String) ?? $1.id).lowercased() }
        SettingsBlock(title: "Nicknames", detail: "Nicknames you've set for individual chatters. Only visible to you. Set or clear a nickname from the user's profile card in chat.", bare: true) {
            if overrides.isEmpty {
                Text("No overrides yet. Tap any user's name in chat to open their profile card, then set a nickname.")
                    .font(.sn(13)).foregroundStyle(SN.textSecondary).multilineTextAlignment(.center)
                    .frame(maxWidth: .infinity).padding(16).snPanel()
            } else {
                ForEach(overrides, id: \.id) { item in
                    let name = item.entry["username"] as? String ?? item.id
                    let nickname = (item.entry["nickname"] as? String).flatMap { $0.isEmpty ? nil : $0 }
                    HStack(spacing: 6) {
                        Text(nickname ?? "(no nickname)").font(.sn(14, 500)).foregroundStyle(SN.textPrimary)
                        Text(" · @\(name)").font(.sn(12)).foregroundStyle(SN.textMuted)
                        if let color = item.entry["color"] as? String, let rgba = RGBA(css: color) {
                            Circle().fill(rgba.color).frame(width: 12, height: 12)
                            Text(color).font(.system(size: 11, design: .monospaced)).foregroundStyle(SN.textMuted)
                        }
                        Spacer()
                        EntryDelete(label: "Clear override for \(name)") {
                            Task {
                                _ = try? await model.core.invoke("set_chat_user_override",
                                                                 args: ["userId": item.id, "username": nil, "field": nil, "value": nil])
                                await store.refresh()
                            }
                        }
                    }
                    .padding(.horizontal, 12).padding(.vertical, 6)
                    .snPanel()
                }
            }
        }
    }
}
