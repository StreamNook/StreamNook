import Foundation

/// A Twitch (or provider) badge on a chat row as the core sends it.
public struct ChatBadge: Codable, Sendable, Equatable, Hashable {
    public var name: String
    public var version: String
    public var imageURL1x: String?
    public var imageURL2x: String?
    public var imageURL4x: String?
    public var title: String?

    enum CodingKeys: String, CodingKey {
        case name, version, title
        case imageURL1x = "image_url_1x"
        case imageURL2x = "image_url_2x"
        case imageURL4x = "image_url_4x"
    }

    public init(name: String, version: String, imageURL1x: String? = nil, imageURL2x: String? = nil,
                imageURL4x: String? = nil, title: String? = nil) {
        self.name = name
        self.version = version
        self.imageURL1x = imageURL1x
        self.imageURL2x = imageURL2x
        self.imageURL4x = imageURL4x
        self.title = title
    }

    /// The image a row draws: the 2x file, else whichever the core had.
    public var drawURL: String? { imageURL2x ?? imageURL4x ?? imageURL1x }
}

/// One piece of a message body, already tokenised by the core (`MessageSegment` in Rust).
public enum ChatSegment: Sendable, Equatable {
    case text(String)
    case emote(ChatEmote)
    case emoji(content: String, url: String)
    case link(content: String, url: String)
    case cheermote(ChatCheermote)
    case gif(content: String, id: String, url: String)
    /// A segment type this build does not know; drawn as its text.
    case unknown(String)

    /// The text the segment stands for, for copy and fallbacks.
    public var content: String {
        switch self {
        case .text(let text), .unknown(let text): return text
        case .emote(let emote): return emote.code
        case .emoji(let content, _), .link(let content, _), .gif(let content, _, _): return content
        case .cheermote(let cheer): return cheer.content
        }
    }
}

public struct ChatEmote: Sendable, Equatable, Hashable {
    public var code: String
    public var id: String?
    public var url: String
    public var isZeroWidth: Bool
    public var modifierFlags: UInt32?
    public var isPersonal: Bool
    /// The emote's own size in CSS pixels where the core knows it, so a row lays out before the
    /// image arrives.
    public var width: Int?
    public var height: Int?

    public init(code: String, id: String? = nil, url: String, isZeroWidth: Bool = false,
                modifierFlags: UInt32? = nil, isPersonal: Bool = false, width: Int? = nil, height: Int? = nil) {
        self.code = code
        self.id = id
        self.url = url
        self.isZeroWidth = isZeroWidth
        self.modifierFlags = modifierFlags
        self.isPersonal = isPersonal
        self.width = width
        self.height = height
    }

    /// Width over height, 1 when the core did not say.
    public var aspect: Double {
        guard let width, let height, width > 0, height > 0 else { return 1 }
        return Double(width) / Double(height)
    }
}

public struct ChatCheermote: Sendable, Equatable, Hashable {
    public var content: String
    public var prefix: String
    public var bits: Int
    public var tier: String
    public var color: String
    public var url: String
}

extension ChatSegment: Codable {
    enum CodingKeys: String, CodingKey {
        case type, content
        case emoteID = "emote_id", emoteURL = "emote_url", isZeroWidth = "is_zero_width"
        case modifierFlags = "modifier_flags", isPersonal = "is_personal", width, height
        case emojiURL = "emoji_url", url
        case prefix, bits, tier, color, cheermoteURL = "cheermote_url"
        case gifID = "gif_id", gifURL = "gif_url"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decodeIfPresent(String.self, forKey: .type) ?? "text"
        let content = try c.decodeIfPresent(String.self, forKey: .content) ?? ""
        switch type {
        case "text":
            self = .text(content)
        case "emote":
            self = .emote(ChatEmote(
                code: content,
                id: try c.decodeIfPresent(String.self, forKey: .emoteID),
                url: try c.decodeIfPresent(String.self, forKey: .emoteURL) ?? "",
                isZeroWidth: try c.decodeIfPresent(Bool.self, forKey: .isZeroWidth) ?? false,
                modifierFlags: try c.decodeIfPresent(UInt32.self, forKey: .modifierFlags),
                isPersonal: try c.decodeIfPresent(Bool.self, forKey: .isPersonal) ?? false,
                width: try c.decodeIfPresent(Int.self, forKey: .width),
                height: try c.decodeIfPresent(Int.self, forKey: .height)
            ))
        case "emoji":
            self = .emoji(content: content, url: try c.decodeIfPresent(String.self, forKey: .emojiURL) ?? "")
        case "link":
            self = .link(content: content, url: try c.decodeIfPresent(String.self, forKey: .url) ?? content)
        case "cheermote":
            self = .cheermote(ChatCheermote(
                content: content,
                prefix: try c.decodeIfPresent(String.self, forKey: .prefix) ?? "",
                bits: try c.decodeIfPresent(Int.self, forKey: .bits) ?? 0,
                tier: try c.decodeIfPresent(String.self, forKey: .tier) ?? "1",
                color: try c.decodeIfPresent(String.self, forKey: .color) ?? "#979797",
                url: try c.decodeIfPresent(String.self, forKey: .cheermoteURL) ?? ""
            ))
        case "gif":
            self = .gif(content: content, id: try c.decodeIfPresent(String.self, forKey: .gifID) ?? "",
                        url: try c.decodeIfPresent(String.self, forKey: .gifURL) ?? "")
        default:
            self = .unknown(content)
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .text(let text):
            try c.encode("text", forKey: .type)
            try c.encode(text, forKey: .content)
        case .unknown(let text):
            try c.encode("text", forKey: .type)
            try c.encode(text, forKey: .content)
        case .emote(let e):
            try c.encode("emote", forKey: .type)
            try c.encode(e.code, forKey: .content)
            try c.encodeIfPresent(e.id, forKey: .emoteID)
            try c.encode(e.url, forKey: .emoteURL)
            if e.isZeroWidth { try c.encode(true, forKey: .isZeroWidth) }
            try c.encodeIfPresent(e.modifierFlags, forKey: .modifierFlags)
            if e.isPersonal { try c.encode(true, forKey: .isPersonal) }
            try c.encodeIfPresent(e.width, forKey: .width)
            try c.encodeIfPresent(e.height, forKey: .height)
        case .emoji(let content, let url):
            try c.encode("emoji", forKey: .type)
            try c.encode(content, forKey: .content)
            try c.encode(url, forKey: .emojiURL)
        case .link(let content, let url):
            try c.encode("link", forKey: .type)
            try c.encode(content, forKey: .content)
            try c.encode(url, forKey: .url)
        case .cheermote(let cheer):
            try c.encode("cheermote", forKey: .type)
            try c.encode(cheer.content, forKey: .content)
            try c.encode(cheer.prefix, forKey: .prefix)
            try c.encode(cheer.bits, forKey: .bits)
            try c.encode(cheer.tier, forKey: .tier)
            try c.encode(cheer.color, forKey: .color)
            try c.encode(cheer.url, forKey: .cheermoteURL)
        case .gif(let content, let id, let url):
            try c.encode("gif", forKey: .type)
            try c.encode(content, forKey: .content)
            try c.encode(id, forKey: .gifID)
            try c.encode(url, forKey: .gifURL)
        }
    }
}

/// The message a reply points at.
public struct ChatReplyInfo: Codable, Sendable, Equatable {
    public var parentMsgID: String
    public var parentDisplayName: String
    public var parentMsgBody: String
    public var parentUserID: String
    public var parentUserLogin: String
    public var parentShownName: String?
    public var parentSegments: [ChatSegment]?

    enum CodingKeys: String, CodingKey {
        case parentMsgID = "parent_msg_id"
        case parentDisplayName = "parent_display_name"
        case parentMsgBody = "parent_msg_body"
        case parentUserID = "parent_user_id"
        case parentUserLogin = "parent_user_login"
        case parentShownName = "parent_shown_name"
        case parentSegments = "parent_segments"
    }

    /// The parent author as rows show it.
    public var shownName: String {
        if let parentShownName, !parentShownName.isEmpty { return parentShownName }
        return parentDisplayName.isEmpty ? parentUserLogin : parentDisplayName
    }
}

public struct ChatHighlightStamp: Codable, Sendable, Equatable {
    public var ruleID: String
    public var kind: String
    public var color: String
    public var soundID: String?
    public var cooldownMS: Int

    enum CodingKeys: String, CodingKey {
        case ruleID = "rule_id", kind, color, soundID = "sound_id", cooldownMS = "cooldown_ms"
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        ruleID = try c.decodeIfPresent(String.self, forKey: .ruleID) ?? ""
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? ""
        color = try c.decodeIfPresent(String.self, forKey: .color) ?? ""
        soundID = try c.decodeIfPresent(String.self, forKey: .soundID)
        cooldownMS = try c.decodeIfPresent(Int.self, forKey: .cooldownMS) ?? 0
    }
}

public struct ChatBuiltInStamp: Codable, Sendable, Equatable {
    /// "raider" | "returning" | "first_time" | "self"
    public var kind: String
    public var color: String
    public var label: String
}

public struct ChatScrollbarMark: Codable, Sendable, Equatable {
    public var kind: String
    public var color: String?
}

/// What the core worked out about a message once (Rust `MessageMetadata`).
public struct ChatMetadata: Codable, Sendable, Equatable {
    public var isAction = false
    public var isMentioned = false
    public var isFirstMessage = false
    public var formattedTimestamp: String?
    public var formattedTimestampWithSeconds: String?
    public var replyInfo: ChatReplyInfo?
    public var sourceRoomID: String?
    public var isFromSharedChat = false
    public var msgType: String?
    public var bitsAmount: Int?
    public var systemMessage: String?
    public var rulesEvaluated = false
    public var isReplyToMe = false
    public var highlight: ChatHighlightStamp?
    public var builtIn: ChatBuiltInStamp?
    public var suspicious: String?
    public var fromBackfill = false
    public var shownName: String?
    public var scrollbarMark: ChatScrollbarMark?
    /// The row worked out by the core for a native renderer.
    public var view: ChatRowView?

    public init() {}

    enum CodingKeys: String, CodingKey {
        case isAction = "is_action", isMentioned = "is_mentioned", isFirstMessage = "is_first_message"
        case formattedTimestamp = "formatted_timestamp"
        case formattedTimestampWithSeconds = "formatted_timestamp_with_seconds"
        case replyInfo = "reply_info", sourceRoomID = "source_room_id"
        case isFromSharedChat = "is_from_shared_chat", msgType = "msg_type", bitsAmount = "bits_amount"
        case systemMessage = "system_message", rulesEvaluated = "rules_evaluated"
        case isReplyToMe = "is_reply_to_me", highlight, builtIn = "built_in", suspicious
        case fromBackfill = "from_backfill", shownName = "shown_name", scrollbarMark = "scrollbar_mark"
        case view
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        isAction = try c.decodeIfPresent(Bool.self, forKey: .isAction) ?? false
        isMentioned = try c.decodeIfPresent(Bool.self, forKey: .isMentioned) ?? false
        isFirstMessage = try c.decodeIfPresent(Bool.self, forKey: .isFirstMessage) ?? false
        formattedTimestamp = try c.decodeIfPresent(String.self, forKey: .formattedTimestamp)
        formattedTimestampWithSeconds = try c.decodeIfPresent(String.self, forKey: .formattedTimestampWithSeconds)
        replyInfo = try c.decodeIfPresent(ChatReplyInfo.self, forKey: .replyInfo)
        sourceRoomID = try c.decodeIfPresent(String.self, forKey: .sourceRoomID)
        isFromSharedChat = try c.decodeIfPresent(Bool.self, forKey: .isFromSharedChat) ?? false
        msgType = try c.decodeIfPresent(String.self, forKey: .msgType)
        bitsAmount = try c.decodeIfPresent(Int.self, forKey: .bitsAmount)
        systemMessage = try c.decodeIfPresent(String.self, forKey: .systemMessage)
        rulesEvaluated = try c.decodeIfPresent(Bool.self, forKey: .rulesEvaluated) ?? false
        isReplyToMe = try c.decodeIfPresent(Bool.self, forKey: .isReplyToMe) ?? false
        highlight = try c.decodeIfPresent(ChatHighlightStamp.self, forKey: .highlight)
        builtIn = try c.decodeIfPresent(ChatBuiltInStamp.self, forKey: .builtIn)
        suspicious = try c.decodeIfPresent(String.self, forKey: .suspicious)
        fromBackfill = try c.decodeIfPresent(Bool.self, forKey: .fromBackfill) ?? false
        shownName = try c.decodeIfPresent(String.self, forKey: .shownName)
        scrollbarMark = try c.decodeIfPresent(ChatScrollbarMark.self, forKey: .scrollbarMark)
        view = try c.decodeIfPresent(ChatRowView.self, forKey: .view)
    }
}

// MARK: The native row view (Rust `RowView`)

/// The partner channel a Shared Chat row came from (Rust `SharedSource`): its face and its own
/// color, floored by the core for the theme.
public struct SharedChatSource: Codable, Sendable, Equatable {
    public var id: String
    public var login: String
    public var displayName: String
    public var avatarURL: String?
    public var color: String

    enum CodingKeys: String, CodingKey {
        case id, login, color
        case displayName = "display_name", avatarURL = "avatar_url"
    }

    public init(id: String, login: String, displayName: String, avatarURL: String? = nil, color: String) {
        self.id = id
        self.login = login
        self.displayName = displayName
        self.avatarURL = avatarURL
        self.color = color
    }
}

/// A link's card (Rust `LinkPreview`): what the core read from the page, or for a Discord
/// invite the server's name, icon and counts.
public struct ChatLinkPreview: Codable, Sendable, Equatable {
    public var url: String
    /// "discord", "youtube", "clip", "vod", "tweet", "image", "media", "generic" and more.
    public var kind: String
    public var title: String?
    public var description: String?
    public var image: String?
    public var siteName: String?
    /// A byline; the `discord.gg/<code>` handle on a Discord card, the price on a Steam one.
    public var author: String?
    public var authorAvatar: String?
    public var viewCount: Int?
    public var duration: Double?
    public var onlineCount: Int?
    public var memberCount: Int?

    enum CodingKeys: String, CodingKey {
        case url, kind, title, description, image, author, duration
        case siteName = "site_name", authorAvatar = "author_avatar", viewCount = "view_count"
        case onlineCount = "online_count", memberCount = "member_count"
    }

    public init(url: String, kind: String, title: String? = nil, description: String? = nil, image: String? = nil,
                siteName: String? = nil, author: String? = nil, onlineCount: Int? = nil, memberCount: Int? = nil) {
        self.url = url
        self.kind = kind
        self.title = title
        self.description = description
        self.image = image
        self.siteName = siteName
        self.author = author
        self.onlineCount = onlineCount
        self.memberCount = memberCount
    }
}

/// One link's card on a row (Rust `RowPreview`). A card still being read has no `preview`;
/// `reserve` says its room is held now, so the row does not grow when it lands.
public struct ChatRowPreview: Codable, Sendable, Equatable {
    public var url: String
    public var preview: ChatLinkPreview?
    public var reserve: Bool
    /// The link stays in the text beside its card; otherwise the card stands for it.
    public var keepLink: Bool

    enum CodingKeys: String, CodingKey { case url, preview, reserve, keepLink = "keep_link" }

    public init(url: String, preview: ChatLinkPreview? = nil, reserve: Bool = false, keepLink: Bool = false) {
        self.url = url
        self.preview = preview
        self.reserve = reserve
        self.keepLink = keepLink
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        url = try c.decode(String.self, forKey: .url)
        preview = try? c.decodeIfPresent(ChatLinkPreview.self, forKey: .preview)
        reserve = try c.decodeIfPresent(Bool.self, forKey: .reserve) ?? false
        keepLink = try c.decodeIfPresent(Bool.self, forKey: .keepLink) ?? false
    }
}

/// What a row draws, worked out once by the core: readable name colour, badge art, paint and
/// the grouped body.
public struct ChatRowView: Codable, Sendable, Equatable {
    public var nameColor: String
    public var badges: [ChatViewBadge]
    public var paint: ChatPaint?
    public var spans: [ChatSpan]
    public var parentSpans: [ChatSpan]?
    /// The StreamNook Atmosphere behind the author's rows (an id in the core's catalog).
    public var atmosphere: String?
    /// The CS2 Major Cologne look behind them instead.
    public var cologne: CologneLook?
    /// A Shared Chat partner's row: the channel it came from.
    public var source: SharedChatSource?
    /// The viewer's own nickname for the author, drawn in place of their name.
    public var nickname: String?
    /// Link cards under the body, in link order.
    public var previews: [ChatRowPreview] = []
    /// The chat event this row is (`twitch:subscription`, `kick:follow`), nil for an ordinary
    /// message: the key `chat_events.hidden_provider_events` lists to hide it.
    public var event: String?

    enum CodingKeys: String, CodingKey {
        case nameColor = "name_color", badges, paint, spans, parentSpans = "parent_spans", atmosphere, cologne, source, nickname
        case previews, event
    }

    public init(nameColor: String, badges: [ChatViewBadge] = [], paint: ChatPaint? = nil, spans: [ChatSpan] = [],
                parentSpans: [ChatSpan]? = nil) {
        self.nameColor = nameColor
        self.badges = badges
        self.paint = paint
        self.spans = spans
        self.parentSpans = parentSpans
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        nameColor = try c.decodeIfPresent(String.self, forKey: .nameColor) ?? "#9147FF"
        badges = try c.decodeIfPresent([ChatViewBadge].self, forKey: .badges) ?? []
        paint = try c.decodeIfPresent(ChatPaint.self, forKey: .paint)
        spans = try c.decodeIfPresent([ChatSpan].self, forKey: .spans) ?? []
        parentSpans = try c.decodeIfPresent([ChatSpan].self, forKey: .parentSpans)
        source = try? c.decodeIfPresent(SharedChatSource.self, forKey: .source)
        atmosphere = try c.decodeIfPresent(String.self, forKey: .atmosphere)
        cologne = try c.decodeIfPresent(CologneLook.self, forKey: .cologne)
        nickname = try c.decodeIfPresent(String.self, forKey: .nickname)
        previews = (try? c.decodeIfPresent([ChatRowPreview].self, forKey: .previews)) ?? []
        event = try c.decodeIfPresent(String.self, forKey: .event)
    }
}

public struct ChatViewBadge: Codable, Sendable, Equatable, Hashable {
    public var url: String
    public var title: String
    public var provider: String

    public init(url: String, title: String = "", provider: String) {
        self.url = url
        self.title = title
        self.provider = provider
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        url = try c.decode(String.self, forKey: .url)
        title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
        provider = try c.decodeIfPresent(String.self, forKey: .provider) ?? "twitch"
    }
}

/// An emote as a row draws it: its 1x size in CSS pixels when the core knows it.
public struct ChatViewEmote: Codable, Sendable, Equatable, Hashable {
    public var code: String
    public var url: String
    public var width: Int?
    public var height: Int?
    public var personal: Bool
    public var hidden: Bool

    public init(code: String, url: String, width: Int? = nil, height: Int? = nil, personal: Bool = false, hidden: Bool = false) {
        self.code = code
        self.url = url
        self.width = width
        self.height = height
        self.personal = personal
        self.hidden = hidden
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        code = try c.decodeIfPresent(String.self, forKey: .code) ?? ""
        url = try c.decodeIfPresent(String.self, forKey: .url) ?? ""
        width = try c.decodeIfPresent(Int.self, forKey: .width)
        height = try c.decodeIfPresent(Int.self, forKey: .height)
        personal = try c.decodeIfPresent(Bool.self, forKey: .personal) ?? false
        hidden = try c.decodeIfPresent(Bool.self, forKey: .hidden) ?? false
    }

    /// Width over height; nil when the core did not know the size.
    public var aspect: Double? {
        guard let width, let height, width > 0, height > 0 else { return nil }
        return Double(width) / Double(height)
    }
}

/// One piece of a drawn body (Rust `ViewSpan`).
public enum ChatSpan: Sendable, Equatable {
    case text(String)
    case mention(text: String, login: String, color: String?)
    case link(label: String, url: String)
    case emoji(text: String, url: String)
    case emote(ChatViewEmote, overlays: [ChatViewEmote], effects: UInt32)
    case cheer(text: String, bits: Int, color: String, url: String)
    case gif(text: String, url: String)

    /// The text the span stands for, for copy and fallbacks.
    public var text: String {
        switch self {
        case .text(let text), .mention(let text, _, _), .emoji(let text, _), .cheer(let text, _, _, _), .gif(let text, _):
            return text
        case .link(let label, _): return label
        case .emote(let emote, let overlays, _): return ([emote.code] + overlays.map(\.code)).joined(separator: " ")
        }
    }
}

extension ChatSpan: Codable {
    enum CodingKeys: String, CodingKey {
        case type, text, login, color, label, url, overlays, effects, bits
        case code, width, height, personal, hidden
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        let type = try c.decodeIfPresent(String.self, forKey: .type) ?? "text"
        func string(_ key: CodingKeys) throws -> String { try c.decodeIfPresent(String.self, forKey: key) ?? "" }
        switch type {
        case "mention":
            self = .mention(text: try string(.text), login: try string(.login), color: try c.decodeIfPresent(String.self, forKey: .color))
        case "link":
            self = .link(label: try string(.label), url: try string(.url))
        case "emoji":
            self = .emoji(text: try string(.text), url: try string(.url))
        case "emote":
            self = .emote(try ChatViewEmote(from: decoder),
                          overlays: try c.decodeIfPresent([ChatViewEmote].self, forKey: .overlays) ?? [],
                          effects: try c.decodeIfPresent(UInt32.self, forKey: .effects) ?? 0)
        case "cheer":
            self = .cheer(text: try string(.text), bits: try c.decodeIfPresent(Int.self, forKey: .bits) ?? 0,
                          color: try string(.color), url: try string(.url))
        case "gif":
            self = .gif(text: try string(.text), url: try string(.url))
        default:
            self = .text(try string(.text))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        switch self {
        case .text(let text):
            try c.encode("text", forKey: .type)
            try c.encode(text, forKey: .text)
        case .mention(let text, let login, let color):
            try c.encode("mention", forKey: .type)
            try c.encode(text, forKey: .text)
            try c.encode(login, forKey: .login)
            try c.encodeIfPresent(color, forKey: .color)
        case .link(let label, let url):
            try c.encode("link", forKey: .type)
            try c.encode(label, forKey: .label)
            try c.encode(url, forKey: .url)
        case .emoji(let text, let url):
            try c.encode("emoji", forKey: .type)
            try c.encode(text, forKey: .text)
            try c.encode(url, forKey: .url)
        case .emote(let emote, let overlays, let effects):
            try c.encode("emote", forKey: .type)
            try emote.encode(to: encoder)
            if !overlays.isEmpty { try c.encode(overlays, forKey: .overlays) }
            if effects != 0 { try c.encode(effects, forKey: .effects) }
        case .cheer(let text, let bits, let color, let url):
            try c.encode("cheer", forKey: .type)
            try c.encode(text, forKey: .text)
            try c.encode(bits, forKey: .bits)
            try c.encode(color, forKey: .color)
            try c.encode(url, forKey: .url)
        case .gif(let text, let url):
            try c.encode("gif", forKey: .type)
            try c.encode(text, forKey: .text)
            try c.encode(url, forKey: .url)
        }
    }
}

/// A 7TV paint reduced by the core to one fill (Rust `PaintView`).
public struct ChatPaint: Codable, Sendable, Equatable, Hashable {
    public struct Stop: Codable, Sendable, Equatable, Hashable {
        public var at: Double
        public var color: String

        public init(at: Double, color: String) {
            self.at = at
            self.color = color
        }
    }

    public struct Shadow: Codable, Sendable, Equatable, Hashable {
        public var x: Double
        public var y: Double
        public var blur: Double
        public var color: String

        public init(x: Double, y: Double, blur: Double, color: String) {
            self.x = x
            self.y = y
            self.blur = blur
            self.color = color
        }
    }

    /// One gradient or image layer of the whole paint (Rust `PaintLayerView`).
    public struct Layer: Codable, Sendable, Equatable, Hashable {
        /// "linear" | "radial" | "image"
        public var kind: String
        public var angle: Double
        public var repeating: Bool
        public var circle: Bool
        public var stops: [Stop]
        public var imageURL: String?

        enum CodingKeys: String, CodingKey { case kind, angle, repeating, circle, stops, imageURL = "image_url" }

        public init(kind: String, angle: Double = 0, repeating: Bool = false, circle: Bool = false, stops: [Stop] = [],
                    imageURL: String? = nil) {
            self.kind = kind
            self.angle = angle
            self.repeating = repeating
            self.circle = circle
            self.stops = stops
            self.imageURL = imageURL
        }

        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? "linear"
            angle = try c.decodeIfPresent(Double.self, forKey: .angle) ?? 0
            repeating = try c.decodeIfPresent(Bool.self, forKey: .repeating) ?? false
            circle = try c.decodeIfPresent(Bool.self, forKey: .circle) ?? false
            stops = try c.decodeIfPresent([Stop].self, forKey: .stops) ?? []
            imageURL = try c.decodeIfPresent(String.self, forKey: .imageURL)
        }
    }

    public var id: String
    /// "linear" | "radial" | "image" | "solid"
    public var kind: String
    public var angle: Double
    public var repeating: Bool
    public var stops: [Stop]
    public var imageURL: String?
    public var shadows: [Shadow]
    /// The paint's name.
    public var name: String
    /// Every gradient and image layer, top first; empty from an older core.
    public var layers: [Layer]
    /// The first single colour, under the layers.
    public var baseColor: String?
    /// Applied to the whole name.
    public var opacity: Double

    enum CodingKeys: String, CodingKey {
        case id, kind, angle, repeating, stops, imageURL = "image_url", shadows, name, layers, baseColor = "base_color"
        case opacity
    }

    public init(id: String, kind: String, angle: Double = 0, repeating: Bool = false, stops: [Stop] = [],
                imageURL: String? = nil, shadows: [Shadow] = [], name: String = "", layers: [Layer] = [],
                baseColor: String? = nil, opacity: Double = 1) {
        self.id = id
        self.kind = kind
        self.angle = angle
        self.repeating = repeating
        self.stops = stops
        self.imageURL = imageURL
        self.shadows = shadows
        self.name = name
        self.layers = layers
        self.baseColor = baseColor
        self.opacity = opacity
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? "solid"
        angle = try c.decodeIfPresent(Double.self, forKey: .angle) ?? 0
        repeating = try c.decodeIfPresent(Bool.self, forKey: .repeating) ?? false
        stops = try c.decodeIfPresent([Stop].self, forKey: .stops) ?? []
        imageURL = try c.decodeIfPresent(String.self, forKey: .imageURL)
        shadows = try c.decodeIfPresent([Shadow].self, forKey: .shadows) ?? []
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        layers = try c.decodeIfPresent([Layer].self, forKey: .layers) ?? []
        baseColor = try c.decodeIfPresent(String.self, forKey: .baseColor)
        opacity = try c.decodeIfPresent(Double.self, forKey: .opacity) ?? 1
    }
}

/// One chat row as the core's socket sends it (Rust `ChatMessage`).
public struct ChatMessage: Codable, Sendable, Equatable, Identifiable {
    public var id: String
    public var userID: String
    public var username: String
    public var displayName: String
    public var color: String?
    public var badges: [ChatBadge]
    public var timestamp: String
    public var content: String
    public var provider: String
    public var channel: String
    public var tags: [String: String]
    public var segments: [ChatSegment]
    public var metadata: ChatMetadata

    enum CodingKeys: String, CodingKey {
        case id, userID = "user_id", username, displayName = "display_name", color, badges, timestamp
        case content, provider, channel, tags, segments, metadata
    }

    public init(id: String, userID: String = "", username: String, displayName: String = "", color: String? = nil,
                badges: [ChatBadge] = [], timestamp: String = "", content: String, provider: String = "twitch",
                channel: String = "", tags: [String: String] = [:], segments: [ChatSegment] = [],
                metadata: ChatMetadata = ChatMetadata()) {
        self.id = id
        self.userID = userID
        self.username = username
        self.displayName = displayName
        self.color = color
        self.badges = badges
        self.timestamp = timestamp
        self.content = content
        self.provider = provider
        self.channel = channel
        self.tags = tags
        self.segments = segments
        self.metadata = metadata
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        userID = try c.decodeIfPresent(String.self, forKey: .userID) ?? ""
        username = try c.decodeIfPresent(String.self, forKey: .username) ?? ""
        displayName = try c.decodeIfPresent(String.self, forKey: .displayName) ?? ""
        color = try c.decodeIfPresent(String.self, forKey: .color)
        badges = try c.decodeIfPresent([ChatBadge].self, forKey: .badges) ?? []
        timestamp = try c.decodeIfPresent(String.self, forKey: .timestamp) ?? ""
        content = try c.decodeIfPresent(String.self, forKey: .content) ?? ""
        provider = try c.decodeIfPresent(String.self, forKey: .provider) ?? "twitch"
        channel = try c.decodeIfPresent(String.self, forKey: .channel) ?? ""
        tags = try c.decodeIfPresent([String: String].self, forKey: .tags) ?? [:]
        segments = try c.decodeIfPresent([ChatSegment].self, forKey: .segments) ?? []
        metadata = try c.decodeIfPresent(ChatMetadata.self, forKey: .metadata) ?? ChatMetadata()
    }

    /// The author as the row shows it: the core's stamped name, else display name, else login.
    public var shownName: String {
        if let shown = metadata.shownName, !shown.isEmpty { return shown }
        return displayName.isEmpty ? username : displayName
    }

    /// A row the core posts itself (connection notices, mod messages): no name, no badges.
    public var isSystem: Bool { username == "System" || displayName == "System" || id.hasPrefix("system-") }
}
