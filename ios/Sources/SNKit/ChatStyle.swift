import Foundation

/// The chat settings a row and the list read (the `chat_design`, `chat_render`, `chat_input`
/// and `chat_highlights` blocks of the core's settings), with a phone's first-run values for
/// anything absent.
public struct ChatStyle: Sendable, Equatable {
    public var fontSize: Double = 14
    public var fontWeight: Int = 400
    public var messageSpacing: Double = 8
    public var showDividers = false
    public var alternatingBackgrounds = false
    public var backfillOpacity: Double = 100
    /// "none" | "fade" | "slide" | "rise"
    public var messageEntrance = "slide"
    public var showTimestamps = false
    public var showTimestampSeconds = false
    public var showAtSign = false
    /// "none" | "colon" | "dot" | "arrow" | "pipe" | "dash"
    public var usernameSeparator = "none"
    /// "plain" | "bar" | "chip" | "brackets" | "dot"
    public var usernameStyle = "plain"
    /// "user" | "theme"
    public var usernameAccentSource = "user"
    public var badgeScale: Double = 1
    public var emoteScale: Double = 1
    /// Space on each side of an emote, in rem (16 points).
    public var emoteMargin: Double = 0.125
    /// "always" | "never" | "hover"
    public var animateEmotes = "always"
    public var showPersonalEmotes = true
    public var giantEmotes = true
    /// "center" | "left" | "right" | "inline"
    public var giantEmoteAlign = "center"
    public var emojiStyle = "apple"
    public var mentionColor = "#ff4444"
    public var replyColor = "#ff6b6b"
    public var mentionAnimation = true
    /// Keep tinted: a mention or reply to you holds its color behind the row while it is on
    /// screen (`chat_design.mention_hold`), at the Highlight Appearance opacity.
    public var mentionHold = false
    /// "regular" | "medium" | "bold" (older saves may hold "normal" or "semibold")
    public var mentionWeight = "medium"
    /// "inherit" | "never" | "always" (older saves may hold "normal" or "italic")
    public var mentionItalic = "inherit"
    /// "plain" | "pill"
    public var mentionStyle = "plain"
    public var mentionTextColor: String?
    public var paintMentionsInBody = true
    /// "full" | "mention" | "off"
    public var replyStyle = "full"
    /// "italic" | "strikethrough" | "dimmed" | "keep" | "hidden"
    public var deletedMessageStyle = "italic"
    public var linkColor = ""
    public var linkUnderline = true
    public var showChatGifs = true
    public var gifScale: Double = 1
    public var hideSharedChat = false
    /// The press-and-hold menu offers Delete, Timeout and Ban where the viewer moderates
    /// (`chat_design.mod_hold_buttons`).
    public var holdModButtons = true
    /// Swipe to moderate is on (`chat_design.mod_slider`).
    public var swipeModerate = true
    public var paintShadows = "all"
    /// A room opens its pinned banner folded (`chat_design.pinned_start_collapsed`).
    public var pinnedStartCollapsed = true
    /// How a folded pin draws: "bar" (its sender and a preview) or "hidden" (nothing over chat;
    /// the chat menu opens it) (`chat_design.pinned_collapsed_style`).
    public var pinnedCollapsedStyle = "bar"

    /// How highlight matches and built-in event highlights dress their row
    /// (`chat_highlights.appearance.display_style`): "standard" (tint and bar), "minimal" (bar
    /// only) or "none".
    public var highlightDisplay = "standard"
    /// The tint's strength at its left edge, 0 to 100 (`chat_highlights.appearance.opacity`).
    public var highlightOpacity: Double = 20
    /// A first-time chatter's row (`chat_highlights.built_in.first_time_chatter`): "wash" (the
    /// tint and bar) or "ring" (an outline of the row instead).
    public var firstTimeStyle = "wash"
    /// A faint fill inside the ring.
    public var firstTimeFill = false
    /// The glint a live first-time row plays: "none" | "sheen" | "pulse" | "chase".
    public var firstTimeAnimation = "none"
    /// Whether the glint keeps cycling instead of playing once.
    public var firstTimeAnimateRepeat = false

    /// Rows a room keeps, 50 to 300 on a phone.
    public var bufferCap = 100
    public var smoothScrollOnResume = true

    public var bypassDuplicate = false
    public var spellcheck = true
    public var hidePlaceholder = false
    public var hideEmoteButton = false
    /// The inbox tray sits at the message bar's right end and the chat menu at its left
    /// (chat_input.inbox_button_side == "right").
    public var inboxRight = false
    /// The room-mode prompt and icons in the composer (chat_input.show_room_modes).
    public var showRoomModes = true
    /// The suggestion strip over the composer (chat_input.emote_tab_complete_enabled).
    public var suggestionsEnabled = true
    /// The emote panel's Images section (chat_input.image_uploader.show_in_menu).
    public var showImageUploads = true
    /// Chat event kinds turned off, as `provider:category` (chat_events.hidden_provider_events).
    public var hiddenEvents: Set<String> = []
    /// Wallet balance is Hidden (chat_input.hide_points_balance).
    public var hidePointsBalance = false
    /// Wallet balance is Always (chat_input.show_points_balance_inline).
    public var pointsBalanceInline = false
    /// Show gains is Bonus chest only (chat_input.wallet_gains == "chest"): the "+N" floats only
    /// for a chest.
    public var walletGainsChestOnly = false
    /// A moderator's clear leaves the rows as they were: no deleted, timed out or banned marks
    /// (moderation.ignore_clear_chat).
    public var ignoreClearChat = false
    /// Deletes, timeouts, bans and clears each post a line in the room
    /// (moderation.show_mod_messages).
    public var showModMessages = false
    /// One of your own commands runs without a slash (chat_commands.user_commands has one enabled
    /// with require_slash false), so plain text is offered to the command dispatcher first. Without
    /// one, ordinary chat makes no extra call (desktop's and Android's rule).
    public var plainTextCommands = false

    public init() {}

    /// Reads the style out of the core's whole settings object (`get_settings`).
    public init(settingsJSON: Data) {
        self.init()
        guard let root = (try? JSONSerialization.jsonObject(with: settingsJSON)) as? [String: Any] else { return }
        apply(root)
    }

    public mutating func apply(_ root: [String: Any]) {
        if let d = root["chat_design"] as? [String: Any] { applyDesign(d) }
        if let r = root["chat_render"] as? [String: Any] {
            if let cap = (r["message_buffer_cap"] as? NSNumber)?.intValue {
                bufferCap = min(max(cap, 50), 300)
            }
            smoothScrollOnResume = r["smooth_scroll_on_resume"] as? Bool ?? smoothScrollOnResume
        }
        if let i = root["chat_input"] as? [String: Any] {
            bypassDuplicate = i["bypass_duplicate"] as? Bool ?? bypassDuplicate
            spellcheck = i["spellcheck_enabled"] as? Bool ?? spellcheck
            hidePlaceholder = i["hide_placeholder"] as? Bool ?? hidePlaceholder
            hideEmoteButton = i["hide_emote_button"] as? Bool ?? hideEmoteButton
            if let side = i["inbox_button_side"] as? String { inboxRight = side == "right" }
            showRoomModes = i["show_room_modes"] as? Bool ?? showRoomModes
            suggestionsEnabled = i["emote_tab_complete_enabled"] as? Bool ?? suggestionsEnabled
            if let uploader = i["image_uploader"] as? [String: Any] {
                showImageUploads = uploader["show_in_menu"] as? Bool ?? showImageUploads
            }
            hidePointsBalance = i["hide_points_balance"] as? Bool ?? hidePointsBalance
            pointsBalanceInline = i["show_points_balance_inline"] as? Bool ?? pointsBalanceInline
            if let gains = i["wallet_gains"] as? String { walletGainsChestOnly = gains == "chest" }
        }
        if let e = root["chat_events"] as? [String: Any], let hidden = e["hidden_provider_events"] as? [Any] {
            hiddenEvents = Set(hidden.compactMap { $0 as? String })
        }
        if let c = root["cosmetics"] as? [String: Any], let shadows = c["paint_shadows"] as? String {
            paintShadows = shadows
        }
        if let h = root["chat_highlights"] as? [String: Any] { applyHighlights(h) }
        if let m = root["moderation"] as? [String: Any] {
            ignoreClearChat = m["ignore_clear_chat"] as? Bool ?? ignoreClearChat
            showModMessages = m["show_mod_messages"] as? Bool ?? showModMessages
        }
        if let c = root["chat_commands"] as? [String: Any], let own = c["user_commands"] as? [[String: Any]] {
            plainTextCommands = own.contains { ($0["enabled"] as? Bool ?? false) && ($0["require_slash"] as? Bool) == false }
        }
    }

    public mutating func applyHighlights(_ h: [String: Any]) {
        if let a = h["appearance"] as? [String: Any] {
            highlightDisplay = a["display_style"] as? String ?? highlightDisplay
            highlightOpacity = (a["opacity"] as? NSNumber).map { min(max($0.doubleValue, 0), 100) } ?? highlightOpacity
        }
        if let b = h["built_in"] as? [String: Any], let f = b["first_time_chatter"] as? [String: Any] {
            firstTimeStyle = f["style"] as? String ?? firstTimeStyle
            firstTimeFill = f["fill"] as? Bool ?? firstTimeFill
            firstTimeAnimation = f["animation"] as? String ?? firstTimeAnimation
            firstTimeAnimateRepeat = f["animate_repeat"] as? Bool ?? firstTimeAnimateRepeat
        }
    }

    public mutating func applyDesign(_ d: [String: Any]) {
        func num(_ key: String) -> Double? { (d[key] as? NSNumber)?.doubleValue }
        func str(_ key: String) -> String? { d[key] as? String }
        func bool(_ key: String) -> Bool? { d[key] as? Bool }
        fontSize = num("font_size").map { min(max($0, 10), 48) } ?? fontSize
        fontWeight = num("font_weight").map { Int($0) } ?? fontWeight
        messageSpacing = num("message_spacing") ?? messageSpacing
        showDividers = bool("show_dividers") ?? showDividers
        alternatingBackgrounds = bool("alternating_backgrounds") ?? alternatingBackgrounds
        backfillOpacity = num("backfill_opacity").map { min(max($0, 30), 100) } ?? backfillOpacity
        messageEntrance = str("message_entrance") ?? messageEntrance
        showTimestamps = bool("show_timestamps") ?? showTimestamps
        showTimestampSeconds = bool("show_timestamp_seconds") ?? showTimestampSeconds
        showAtSign = bool("show_at_sign") ?? showAtSign
        usernameSeparator = str("username_separator") ?? usernameSeparator
        usernameStyle = str("username_style") ?? usernameStyle
        usernameAccentSource = str("username_accent_source") ?? usernameAccentSource
        badgeScale = num("badge_scale").map { min(max($0, 0.5), 2.5) } ?? badgeScale
        emoteScale = num("emote_scale").map { min(max($0, 0.5), 3) } ?? emoteScale
        emoteMargin = num("emote_margin") ?? emoteMargin
        animateEmotes = str("animate_emotes") ?? animateEmotes
        showPersonalEmotes = bool("show_personal_emotes") ?? showPersonalEmotes
        giantEmotes = bool("giant_emotes") ?? giantEmotes
        giantEmoteAlign = str("giant_emote_align") ?? giantEmoteAlign
        emojiStyle = str("emoji_style") ?? emojiStyle
        mentionColor = str("mention_color") ?? mentionColor
        replyColor = str("reply_color") ?? replyColor
        mentionAnimation = bool("mention_animation") ?? mentionAnimation
        mentionHold = bool("mention_hold") ?? mentionHold
        mentionWeight = str("mention_weight") ?? mentionWeight
        mentionItalic = str("mention_italic") ?? mentionItalic
        mentionStyle = str("mention_style") ?? mentionStyle
        mentionTextColor = str("mention_text_color").flatMap { $0.isEmpty ? nil : $0 } ?? mentionTextColor
        paintMentionsInBody = bool("paint_mentions_in_body") ?? paintMentionsInBody
        replyStyle = str("reply_style") ?? replyStyle
        deletedMessageStyle = str("deleted_message_style") ?? deletedMessageStyle
        linkColor = str("link_color") ?? linkColor
        linkUnderline = bool("link_underline") ?? linkUnderline
        showChatGifs = bool("show_chat_gifs") ?? showChatGifs
        gifScale = num("gif_scale").map { min(max($0, 0.5), 2) } ?? gifScale
        hideSharedChat = bool("hide_shared_chat") ?? hideSharedChat
        holdModButtons = bool("mod_hold_buttons") ?? holdModButtons
        swipeModerate = bool("mod_slider") ?? swipeModerate
        pinnedStartCollapsed = bool("pinned_start_collapsed") ?? pinnedStartCollapsed
        pinnedCollapsedStyle = str("pinned_collapsed_style") ?? pinnedCollapsedStyle
    }

    /// Emotes animate unless the setting asks for stills; a touch never hovers, so `hover`
    /// draws stills too.
    public var animatesEmotes: Bool { animateEmotes == "always" }

    /// Whether a room's pins draw anything: always while open, and while folded unless Style is
    /// Hidden.
    public func pinsShown(open: Bool) -> Bool { open || pinnedCollapsedStyle != "hidden" }

    /// The highlight tint's alpha at the row's left edge.
    public var highlightAlpha: Double { highlightOpacity / 100 }
    public var highlightShowsTint: Bool { highlightDisplay == "standard" }
    public var highlightShowsBar: Bool { highlightDisplay != "none" }

    /// How heavy an @name draws: `body` is the message text's own weight.
    public enum MentionWeight: Sendable, Equatable { case body, medium, semibold, bold }

    public var mentionWeightChoice: MentionWeight {
        switch mentionWeight {
        case "regular", "normal": .body
        case "semibold": .semibold
        case "bold": .bold
        default: .medium
        }
    }

    /// Whether an @name slants. Inherit follows the message (a /me row is italic as a whole);
    /// never keeps it upright even there.
    public func mentionSlants(inItalicMessage italic: Bool) -> Bool {
        switch mentionItalic {
        case "always", "italic": true
        case "never", "normal": false
        default: italic
        }
    }

    /// The wallet button carries the short balance beside its icon: Wallet balance is Always. A
    /// touch has no hover, so a saved desktop On hover reads as Hidden here: just the icon.
    public var walletBalanceShown: Bool { pointsBalanceInline && !hidePointsBalance }

    /// Vertical padding of a plain row: `max(4, spacing / 2)`.
    public var rowPadding: Double { max(4, messageSpacing / 2) }
}
