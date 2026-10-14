import Foundation

/// The chat settings a row and the list read (the `chat_design`, `chat_render` and
/// `chat_input` blocks of the core's settings), with a phone's first-run values for anything
/// absent.
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
    /// "normal" | "medium" | "semibold" | "bold"
    public var mentionWeight = "medium"
    /// "inherit" | "italic" | "normal"
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
    public var paintShadows = "all"

    /// Rows a room keeps, 50 to 300 on a phone.
    public var bufferCap = 100
    public var smoothScrollOnResume = true

    public var bypassDuplicate = false
    public var spellcheck = true
    public var hidePlaceholder = false
    public var hideCommandButton = false
    public var hideEmoteButton = false

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
            hideCommandButton = i["hide_command_button"] as? Bool ?? hideCommandButton
            hideEmoteButton = i["hide_emote_button"] as? Bool ?? hideEmoteButton
        }
        if let c = root["cosmetics"] as? [String: Any], let shadows = c["paint_shadows"] as? String {
            paintShadows = shadows
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
    }

    /// Emotes animate unless the setting asks for stills; a touch never hovers, so `hover`
    /// draws stills too.
    public var animatesEmotes: Bool { animateEmotes == "always" }

    /// Vertical padding of a plain row: `max(4, spacing / 2)`.
    public var rowPadding: Double { max(4, messageSpacing / 2) }
}
