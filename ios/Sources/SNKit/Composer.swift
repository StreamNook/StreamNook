import Foundation

// The composer's shapes and its pure rules: what the core answers for suggestions, the emote
// picker and slash commands, and how the draft changes on an insert, a completion or a command
// pick. Foundation only, so it is tested on Linux; the screens only draw it.

// MARK: Suggestions (`match_emote_tokens`)

/// A row of the core's emote match: an emote, or (provider "user") a chatter it offers after
/// the emotes for the word being typed.
public struct EmoteMatch: Decodable, Identifiable, Hashable, Sendable {
    public var id: String
    public var name: String
    public var insertText: String?
    public var url: String
    public var localPath: String?
    public var provider: String
    public var sourceLabel: String?
    public var group: String?
    public var isZeroWidth: Bool?
    /// A chatter row's login.
    public var login: String?

    enum CodingKeys: String, CodingKey {
        case id, name, url, provider, group, login
        case insertText = "insert_text", sourceLabel = "source_label", localPath = "local_path",
             isZeroWidth = "is_zero_width"
    }

    public init(id: String, name: String, insertText: String? = nil, url: String = "", localPath: String? = nil,
                provider: String, sourceLabel: String? = nil, group: String? = nil, isZeroWidth: Bool? = nil,
                login: String? = nil) {
        self.id = id
        self.name = name
        self.insertText = insertText
        self.url = url
        self.localPath = localPath
        self.provider = provider
        self.sourceLabel = sourceLabel
        self.group = group
        self.isZeroWidth = isZeroWidth
        self.login = login
    }

    /// What goes into the draft.
    public var text: String { insertText ?? name }
    public var isChatter: Bool { provider == "user" }
    /// Unique within one answer (the same emote id can sit under two names).
    public var key: String { "\(provider):\(id):\(name)" }
}

public struct EmoteMatchResult: Decodable, Sendable {
    public var rows: [EmoteMatch]
    public var total: Int
    public var ready: Bool
}

// MARK: Draft editing

public enum ComposerText {
    /// Any word asks: the core answers a one-character word with only an emote of exactly that
    /// name ("0"), since a prefix list for it would be most of the set.
    public static let minimumSuggestLength = 1

    /// The run of non-space characters at the end of the draft (empty after a space).
    public static func lastWord(_ text: String) -> String {
        guard let last = text.last, !last.isWhitespace else { return "" }
        return String(text.reversed().prefix { !$0.isWhitespace }.reversed())
    }

    /// Whether the word at the end of `draft` asks for suggestions.
    public static func wantsSuggestions(_ draft: String) -> Bool {
        lastWord(draft).count >= minimumSuggestLength
    }

    /// An emote or emoji put into the draft: a space before it unless the draft is empty or
    /// already ends in one, and a space after it.
    public static func inserting(_ token: String, into draft: String) -> String {
        let needsSpace = !draft.isEmpty && !draft.hasSuffix(" ")
        return draft + (needsSpace ? " " : "") + token + " "
    }

    /// The last word replaced by a suggestion, and a space.
    public static func completing(_ draft: String, with text: String) -> String {
        let word = lastWord(draft)
        return String(draft.dropLast(word.count)) + text + " "
    }

    /// A command picked from the commands sheet replaces the whole draft.
    public static func command(_ name: String) -> String {
        "/\(name) "
    }
}

// MARK: Sending

/// Where a send goes.
public enum ComposerRoute: Equatable, Sendable {
    /// Nothing to send.
    case none
    /// A slash command: the core's dispatcher runs it.
    case slash(String)
    /// Chat text.
    case chat(String)

    /// The draft trimmed, then routed: "/" starts a command.
    public static func of(_ draft: String) -> ComposerRoute {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty { return .none }
        return text.hasPrefix("/") ? .slash(text) : .chat(text)
    }
}

/// What becomes of the typed text once the core's dispatcher has answered, or failed to.
public enum SlashAfterDispatch: Equatable, Sendable {
    /// The core ran it: nothing goes to chat but what its answer says to send.
    case done
    /// The core does not run it (Twitch runs /me), or it is plain text: it goes to chat as typed.
    case sendTyped
    /// The dispatcher failed: a command stays in the draft and never reaches chat as text.
    case keepDraft

    public static func of(text: String, handled: Bool, dispatchFailed: Bool) -> SlashAfterDispatch {
        if dispatchFailed { return text.hasPrefix("/") ? .keepDraft : .sendTyped }
        return handled ? .done : .sendTyped
    }
}

/// The core's answer to a Twitch send (`send_chat_message`): accepted or dropped with a reason.
public struct SendResult: Decodable, Sendable, Equatable {
    public var isSent: Bool
    public var dropReason: String?
    public var messageID: String?
    /// The message cheers, so Rust held it rather than sending it as chat.
    public var cheer: CheerCheck?

    enum CodingKeys: String, CodingKey {
        case isSent = "is_sent", dropReason = "drop_reason", messageID = "message_id", cheer
    }

    public init(isSent: Bool, dropReason: String? = nil, messageID: String? = nil, cheer: CheerCheck? = nil) {
        self.isSent = isSent
        self.dropReason = dropReason
        self.messageID = messageID
        self.cheer = cheer
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        isSent = try c.decodeIfPresent(Bool.self, forKey: .isSent) ?? true
        dropReason = try c.decodeIfPresent(String.self, forKey: .dropReason)
        messageID = try c.decodeIfPresent(String.self, forKey: .messageID)
        cheer = try c.decodeIfPresent(CheerCheck.self, forKey: .cheer)
    }

    /// The chat line for a dropped message. A held cheer is no drop: the cheer bar takes it.
    public var dropLine: String? {
        isSent || cheer != nil ? nil : "Your message was not sent: \(dropReason ?? "the room refused it")"
    }
}

/// A slash command's outcome from the core (`run_slash_command`).
public struct SlashOutcome: Decodable, Sendable, Equatable {
    public struct Toast: Decodable, Sendable, Equatable {
        public var text: String
        /// "success" | "error" | "info"
        public var kind: String

        public init(text: String, kind: String) {
            self.text = text
            self.kind = kind
        }
    }

    /// A page action the core cannot do itself.
    public struct UIAction: Decodable, Sendable, Equatable {
        public var kind: String
        public var login: String?
        public var userID: String?
        public var displayName: String?
        public var url: String?
        public var text: String?
        /// `create_clip`: the room the line was typed in, its broadcaster id and the clip's title.
        public var channel: String?
        public var channelID: String?
        public var title: String?

        enum CodingKeys: String, CodingKey {
            case kind, login, url, text, channel, title, userID = "user_id", displayName = "display_name"
            case channelID = "channel_id"
        }

        public init(kind: String, login: String? = nil, userID: String? = nil, displayName: String? = nil,
                    url: String? = nil, text: String? = nil) {
            self.kind = kind
            self.login = login
            self.userID = userID
            self.displayName = displayName
            self.url = url
            self.text = text
        }
    }

    /// False: not a command the core runs; the text goes to chat as typed (Twitch runs /me).
    public var handled: Bool
    /// Text to send as an ordinary chat message in the room (custom command expansions,
    /// /say, /shrug...).
    public var send: String?
    /// `/reply`: the message id `send` replies to.
    public var replyTo: String?
    /// `/pin <message>`: pin `send` once it is sent.
    public var pinAfterSend: Bool
    public var clearDraft: Bool
    public var toasts: [Toast]
    /// Lines printed in the room's chat that only the viewer sees.
    public var notices: [String]
    public var ui: UIAction?

    enum CodingKeys: String, CodingKey {
        case handled, send, toasts, notices, ui, clearDraft = "clear_draft", replyTo = "reply_to",
             pinAfterSend = "pin_after_send"
    }

    public init(handled: Bool, send: String? = nil, replyTo: String? = nil, pinAfterSend: Bool = false,
                clearDraft: Bool = true, toasts: [Toast] = [], notices: [String] = [], ui: UIAction? = nil) {
        self.handled = handled
        self.send = send
        self.replyTo = replyTo
        self.pinAfterSend = pinAfterSend
        self.clearDraft = clearDraft
        self.toasts = toasts
        self.notices = notices
        self.ui = ui
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        handled = try c.decodeIfPresent(Bool.self, forKey: .handled) ?? false
        send = try c.decodeIfPresent(String.self, forKey: .send)
        replyTo = try c.decodeIfPresent(String.self, forKey: .replyTo)
        pinAfterSend = try c.decodeIfPresent(Bool.self, forKey: .pinAfterSend) ?? false
        clearDraft = try c.decodeIfPresent(Bool.self, forKey: .clearDraft) ?? handled
        toasts = try c.decodeIfPresent([Toast].self, forKey: .toasts) ?? []
        notices = try c.decodeIfPresent([String].self, forKey: .notices) ?? []
        ui = try c.decodeIfPresent(UIAction.self, forKey: .ui)
    }

    /// A room with no broadcaster id cannot run commands yet.
    public static let noRoomToast = "Open this channel's stream to run commands here"
}

// MARK: Emote picker (`emote_picker`)

/// The core's answer for the open picker: tabs with counts and the selected tab's sections.
public struct PickerAnswer: Decodable, Sendable, Equatable {
    public struct Tab: Decodable, Sendable, Equatable {
        public var id: String
        public var count: Int
    }

    public var ready: Bool
    public var tabs: [Tab]
    public var tab: String
    public var sections: [PickerSection]

    public init(ready: Bool, tabs: [Tab], tab: String, sections: [PickerSection]) {
        self.ready = ready
        self.tabs = tabs
        self.tab = tab
        self.sections = sections
    }

    public func count(_ id: String) -> Int {
        tabs.first { $0.id == id }?.count ?? 0
    }
}

public struct PickerSection: Decodable, Sendable, Equatable, Identifiable {
    public var key: String
    public var title: String
    /// Grid columns: 7, or 4 and 3 for wide and ultra wide emotes.
    public var columns: Int
    public var emotes: [PickerCell]

    public var id: String { key }
}

/// One picker cell: the emote and what the picker shows on it.
public struct PickerCell: Decodable, Sendable, Equatable, Identifiable, Hashable {
    public struct ReplacedBy: Decodable, Sendable, Equatable, Hashable {
        public var id: String
        public var url: String
    }

    public var emoteID: String
    public var name: String
    public var url: String
    public var provider: String
    public var localPath: String?
    public var isZeroWidth: Bool
    /// Set on modifier emotes (FFZ and BetterTTV).
    public var modifierFlags: Int?
    public var width: Int?
    public var height: Int?
    public var animated: Bool?
    public var ownerName: String?
    public var favorite: Bool
    /// Shown dimmed with a lock and cannot be picked.
    public var locked: Bool
    /// The viewer's personal emote that shows instead when this name is typed.
    public var replacedBy: ReplacedBy?

    enum CodingKeys: String, CodingKey {
        case name, url, provider, width, height, animated, favorite, locked
        case emoteID = "id", localPath = "local_url", isZeroWidth = "is_zero_width", modifierFlags = "modifier_flags",
             ownerName = "owner_name", replacedBy = "replaced_by"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        emoteID = try c.decode(String.self, forKey: .emoteID)
        name = try c.decode(String.self, forKey: .name)
        url = try c.decode(String.self, forKey: .url)
        provider = try c.decode(String.self, forKey: .provider)
        // A picker cell says `local_url`, a search row (`match_emote_tokens`) `local_path`.
        localPath = try c.decodeIfPresent(String.self, forKey: .localPath)
            ?? decoder.container(keyedBy: SearchKeys.self).decodeIfPresent(String.self, forKey: .localPath)
        isZeroWidth = try c.decodeIfPresent(Bool.self, forKey: .isZeroWidth) ?? false
        modifierFlags = try c.decodeIfPresent(Int.self, forKey: .modifierFlags)
        width = try c.decodeIfPresent(Int.self, forKey: .width)
        height = try c.decodeIfPresent(Int.self, forKey: .height)
        animated = try c.decodeIfPresent(Bool.self, forKey: .animated)
        ownerName = try c.decodeIfPresent(String.self, forKey: .ownerName)
        favorite = try c.decodeIfPresent(Bool.self, forKey: .favorite) ?? false
        locked = try c.decodeIfPresent(Bool.self, forKey: .locked) ?? false
        replacedBy = try c.decodeIfPresent(ReplacedBy.self, forKey: .replacedBy)
    }

    private enum SearchKeys: String, CodingKey {
        case localPath = "local_path"
    }

    public var id: String { "\(provider):\(emoteID)" }
    /// Unique within a search answer, where one emote id can sit under two names.
    public var key: String { "\(provider):\(emoteID):\(name)" }
    public var isModifier: Bool { modifierFlags != nil }

    /// Grid cells the emote takes in the search results' 7-across grid, by its width as the
    /// tabs group it: 1 up to 48 px, 2 up to 80 px, else 3 (the desktop menu's spans).
    public var span: Int {
        let w = width.flatMap { $0 > 0 ? $0 : nil } ?? 32
        return w <= 48 ? 1 : w <= 80 ? 2 : 3
    }

    /// The words a long press shows: the name, then who made it and what kind it is.
    public var detailLines: [String] {
        var lines: [String] = []
        if let ownerName, !ownerName.isEmpty { lines.append("by \(ownerName)") }
        if let modifierFlags {
            // BetterTTV modifiers apply to the emote after them, FFZ ones to the one before.
            lines.append(modifierFlags & Self.modifierPrefix != 0
                ? "Modifier - applies to the next emote"
                : "Modifier - applies to the previous emote")
        } else if isZeroWidth {
            lines.append("Zero-Width")
        }
        if locked { lines.append("FFZ subscriber effect - locked") }
        if replacedBy != nil { lines.append("Your personal \(name) shows instead") }
        return lines
    }

    /// `MOD_PREFIX`: the modifier attaches to the following emote.
    static let modifierPrefix = 1 << 18
}

/// The picker's tabs in order: the core's emote tabs between the app's own Favorites,
/// Emoji and GIF tabs.
public enum PickerTabs {
    /// The tab a room opens on.
    public static func defaultTab(provider: String) -> String {
        switch provider {
        case "twitch": return "twitch"
        case "kick": return "kick"
        default: return "emoji"
        }
    }

    /// Favorites, Emoji, then the emote tabs the core lists for the room (after Favorites).
    public static func order(coreTabs: [String], gif: Bool) -> [String] {
        var out = ["favorites", "emoji"]
        if gif { out.append("gifs") }
        // The core's order is Twitch, BetterTTV, 7TV, FFZ; the picker shows 7TV before FFZ too.
        out += coreTabs.filter { $0 != "favorites" }
        return out
    }

    /// The tabs while searching: All first, then the same tabs, which narrow the results.
    public static func searchOrder(_ tabs: [String]) -> [String] {
        ["all"] + tabs.filter { $0 != "all" && $0 != "gifs" }
    }

    /// A tab's number in at most four characters, floored as the desktop tabs show it
    /// (`fitAmount`): 999, then 1K, 12K, 999K, 1M.
    public static func countLabel(_ count: Int) -> String {
        guard count > 0 else { return "0" }
        if count < 1000 { return String(count) }
        for (size, unit) in [(1_000, "K"), (1_000_000, "M"), (1_000_000_000, "B")] where count / size < 1000 {
            return "\(count / size)\(unit)"
        }
        return "999B"
    }

    /// The scope a fresh search opens on: everything, except from the Emoji tab, where the
    /// viewer is plainly after an emoji.
    public static func searchScope(from tab: String) -> String {
        tab == "emoji" ? "emoji" : "all"
    }

    public static func title(_ id: String) -> String {
        switch id {
        case "all": return "All"
        case "favorites": return "Favorites"
        case "emoji": return "Emoji"
        case "gifs": return "GIF"
        case "twitch": return "Twitch"
        case "bttv": return "BetterTTV"
        case "7tv": return "7TV"
        case "ffz": return "FrankerFaceZ"
        case "kick": return "Kick"
        default: return id
        }
    }
}

// MARK: Picker search (`match_emote_tokens`, picker mode)

/// The core's ranked search across every emote source the room has, as the picker asks it
/// (`profile` "search", `picker` true): the best rows first, how many matched in all, and how
/// many in each source and among the favorites, all counted before the row cap.
public struct PickerSearch: Decodable, Sendable, Equatable {
    /// Rows the picker asks for, as the desktop menu does.
    public static let limit = 200
    /// Emoji shown after the emotes under All; the Emoji tab shows every match.
    public static let emojiInAll = 24

    public var rows: [PickerCell]
    public var total: Int
    /// False while the room's emote set is not in memory yet.
    public var ready: Bool
    /// Hits per source (`twitch`, `bttv`, `7tv`, `ffz`, `kick`, `youtube`) and `favorites`.
    public var counts: [String: Int]

    enum CodingKeys: String, CodingKey {
        case rows, total, ready, counts
    }

    public init(rows: [PickerCell], total: Int, ready: Bool = true, counts: [String: Int]) {
        self.rows = rows
        self.total = total
        self.ready = ready
        self.counts = counts
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        rows = try c.decodeIfPresent([PickerCell].self, forKey: .rows) ?? []
        total = try c.decodeIfPresent(Int.self, forKey: .total) ?? rows.count
        ready = try c.decodeIfPresent(Bool.self, forKey: .ready) ?? true
        counts = try c.decodeIfPresent([String: Int].self, forKey: .counts) ?? [:]
    }

    /// The rows a tab shows, in rank order: all of them, the favorites, or one source's.
    public func cells(in scope: String) -> [PickerCell] {
        switch scope {
        case "all": return rows
        case "emoji": return []
        case "favorites": return rows.filter(\.favorite)
        default: return rows.filter { $0.provider == scope }
        }
    }

    /// A tab's hit count while searching. All counts the emoji too.
    public func count(_ scope: String, emojiHits: Int) -> Int {
        switch scope {
        case "all": return total + emojiHits
        case "emoji": return emojiHits
        default: return counts[scope] ?? 0
        }
    }

    /// How many emotes the tab matched, when more than the rows it shows: the results say they
    /// are the best of that many.
    public func cappedTotal(_ scope: String) -> Int? {
        guard scope != "emoji" else { return nil }
        let matched = scope == "all" ? total : counts[scope] ?? 0
        return matched > cells(in: scope).count ? matched : nil
    }
}

/// The search results' grid: rank order, 7 cells across, a wide emote taking 2 or 3 of them.
/// A narrow emote that fits a gap an earlier wide one left goes back to fill it (the desktop
/// grid's dense packing), so rows stay full without losing the order much.
public enum SearchGrid {
    public static let columns = 7

    public static func lines(_ cells: [PickerCell], columns: Int = columns) -> [[PickerCell]] {
        var lines: [[PickerCell]] = []
        var used: [Int] = []
        for cell in cells {
            let span = min(cell.span, columns)
            if let i = used.firstIndex(where: { $0 + span <= columns }) {
                lines[i].append(cell)
                used[i] += span
            } else {
                lines.append([cell])
                used.append(span)
            }
        }
        return lines
    }
}

// MARK: Emoji

/// The emoji picker's data: categories in order with their emoji, and search keywords.
public struct EmojiCatalog: Decodable, Sendable {
    public struct Category: Decodable, Sendable, Identifiable {
        public var name: String
        public var emoji: [String]
        public var id: String { name }
    }

    public var categories: [Category]
    public var keywords: [String: [String]]

    public init(categories: [Category], keywords: [String: [String]]) {
        self.categories = categories
        self.keywords = keywords
    }

    /// The bundled catalog.
    public static func bundled() -> EmojiCatalog? {
        guard let url = Bundle.module.url(forResource: "emoji", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder().decode(EmojiCatalog.self, from: data)
    }

    /// The categories with only the emoji a search matches: the category name, the emoji
    /// itself, or one of its keywords. Empty categories are left out.
    public func filtered(_ query: String) -> [Category] {
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return categories }
        return categories.compactMap { category in
            if category.name.lowercased().contains(q) { return category }
            let hits = category.emoji.filter { emoji in
                emoji.contains(q) || (keywords[emoji] ?? []).contains { $0.contains(q) }
            }
            return hits.isEmpty ? nil : Category(name: category.name, emoji: hits)
        }
    }

    /// Every emoji a search matches, in catalog order, for the search results' Emoji block.
    public func matches(_ query: String) -> [String] {
        filtered(query).flatMap(\.emoji)
    }
}
