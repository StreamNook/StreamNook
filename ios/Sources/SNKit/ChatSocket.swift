import Foundation

/// A room mode as the core words it (Rust `RoomMode`).
public struct ChatRoomMode: Codable, Sendable, Equatable, Hashable {
    /// "subs_only" | "emote_only" | "followers_only" | "slow" | "unique"
    public var kind: String
    public var label: String
    public var compact: String
    public var short: String
    public var detail: String
    public var appliesToYou: Bool?

    enum CodingKeys: String, CodingKey {
        case kind, label, compact, short, detail, appliesToYou = "applies_to_you"
    }

    public init(kind: String, label: String, compact: String, short: String, detail: String, appliesToYou: Bool? = nil) {
        self.kind = kind
        self.label = label
        self.compact = compact
        self.short = short
        self.detail = detail
        self.appliesToYou = appliesToYou
    }
}

/// The room's modes in words (Rust `RoomModeView`), shipped on every ROOMSTATE frame.
public struct ChatRoomModes: Codable, Sendable, Equatable {
    public var modes: [ChatRoomMode]
    public var prompt: String?
    public var promptShort: String?
    public var standing: String?
    public var slowSeconds: Int
    /// The mode that certainly keeps the viewer from sending ("subs_only", "emote_only",
    /// "followers_only"); nil when nothing proves it.
    public var blocked: String?
    /// What the disabled message box says while `blocked` is set.
    public var reason: String?
    /// A moderator's warning the viewer has not acknowledged: the box stays locked behind its
    /// Acknowledge button.
    public var warning: ChatWarning?

    enum CodingKeys: String, CodingKey {
        case modes, prompt, promptShort = "prompt_short", standing, slowSeconds = "slow_seconds", blocked, reason, warning
    }

    public init(modes: [ChatRoomMode] = [], prompt: String? = nil, promptShort: String? = nil, standing: String? = nil,
                slowSeconds: Int = 0, blocked: String? = nil, reason: String? = nil, warning: ChatWarning? = nil) {
        self.modes = modes
        self.prompt = prompt
        self.promptShort = promptShort
        self.standing = standing
        self.slowSeconds = slowSeconds
        self.blocked = blocked
        self.reason = reason
        self.warning = warning
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        modes = try c.decodeIfPresent([ChatRoomMode].self, forKey: .modes) ?? []
        prompt = try c.decodeIfPresent(String.self, forKey: .prompt)
        promptShort = try c.decodeIfPresent(String.self, forKey: .promptShort)
        standing = try c.decodeIfPresent(String.self, forKey: .standing)
        slowSeconds = try c.decodeIfPresent(Int.self, forKey: .slowSeconds) ?? 0
        blocked = try c.decodeIfPresent(String.self, forKey: .blocked)
        reason = try c.decodeIfPresent(String.self, forKey: .reason)
        warning = try? c.decodeIfPresent(ChatWarning.self, forKey: .warning)
    }
}

/// A row patch the core sends when it re-tokenised rows (`HISTORY_SEGMENTS`).
public struct ChatRowPatch: Sendable, Equatable {
    public var id: String
    public var segments: [ChatSegment]
    public var view: ChatRowView?

    public init(id: String, segments: [ChatSegment], view: ChatRowView? = nil) {
        self.id = id
        self.segments = segments
        self.view = view
    }
}

/// One frame from the core's chat socket, decoded. The socket sends JSON chat rows (they have
/// `id`), JSON control frames (they have `type`), status words and `NAME:` prefixed status lines.
public enum ChatSocketFrame: Sendable, Equatable {
    case row(ChatMessage)
    case roomState(channel: String?, modes: ChatRoomModes?)
    /// One message deleted. `login` names its author when the frame does; `reason` is the rule a
    /// platform's automatic delete broke (Kick's auto-mod).
    case clearMessage(channel: String?, targetID: String, login: String? = nil, reason: String? = nil)
    /// A timeout (`duration` seconds), a ban (`duration` nil) or, with no target, a full clear.
    case clearChat(channel: String?, targetUserID: String?, targetUser: String?, duration: Int?)
    /// The signed-in viewer's own chat color (`USER_COLOR:`), for the room named or, when the
    /// line names none, for the only room open.
    case ownColor(channel: String?, color: String)
    /// Users a settings save just hid (`HIDE_USERS`): their rows already held go. `room` is the
    /// room's key (the bare login on Twitch, `kick:slug` on Kick); nil means every room.
    case hideUsers(room: String?, logins: [String])
    /// A chatter's 7TV personal set landed after some of their rows (`PERSONAL_EMOTES`): those
    /// rows have the set applied again.
    case personalEmotes(channel: String?, userID: String, messageIDs: [String])
    /// A platform's own pin (`PINNED`, Kick's and YouTube's), or its removal (`UNPINNED`, nil).
    case pinned(channel: String?, pin: PinnedChat?)
    /// A chatter's 7TV cosmetics that arrived after their first row. `userID` names the author as
    /// `ROW_MEMBER` does: a bare Twitch id, `kick:<id>` on another platform.
    case cosmetics(channel: String?, userID: String, paint: ChatPaint?, badge: ChatViewBadge?)
    /// A StreamNook member's look changed (their Atmosphere, Cologne look or member badge), for
    /// every row of theirs in every room.
    case member(userID: String, atmosphere: String?, cologne: CologneLook?, badge: ChatViewBadge?)
    case patch(channel: String?, rows: [ChatRowPatch])
    /// A link's card read after its rows were sent (`ROW_PREVIEW`), for every room: nil when
    /// the link has none, so its held room goes.
    case preview(url: String, preview: ChatLinkPreview?)
    case notice(channel: String?, msgID: String?, message: String?)
    /// A message AutoMod held, or the decision on one (`AUTOMOD_HELD`).
    case held(channel: String?, row: HeldRow)
    case status(String)
    case other(String)

    private static let decoder = JSONDecoder()

    public static func decode(_ raw: String) -> ChatSocketFrame {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.hasPrefix("{"), let data = text.data(using: .utf8) else {
            if let own = ChatFrame.ownColor(text) { return .ownColor(channel: own.channel, color: own.color) }
            if ChatFrame.statusWords.contains(text) { return .status(text) }
            if let prefix = ChatFrame.statusPrefixes.first(where: { text.hasPrefix($0) }) {
                return .status(String(prefix.dropLast()))
            }
            return .other(text)
        }
        guard let head = try? decoder.decode(FrameHead.self, from: data) else { return .other(text) }
        switch head.type {
        case nil:
            if head.id != nil, let message = try? decoder.decode(ChatMessage.self, from: data) {
                return .row(message)
            }
            return .other(text)
        case "ROOMSTATE":
            let frame = try? decoder.decode(RoomStateFrame.self, from: data)
            return .roomState(channel: head.channel, modes: frame?.modes)
        case "CLEARMSG":
            guard let frame = try? decoder.decode(ClearMsgFrame.self, from: data) else { return .other(text) }
            return .clearMessage(channel: head.channel, targetID: frame.targetMsgID,
                                 login: frame.login.flatMap { $0.isEmpty ? nil : $0 },
                                 reason: frame.reason.flatMap { $0.isEmpty ? nil : $0 })
        case "CLEARCHAT":
            let frame = try? decoder.decode(ClearChatFrame.self, from: data)
            return .clearChat(channel: head.channel, targetUserID: frame?.targetUserID, targetUser: frame?.targetUser,
                              duration: frame?.banDuration)
        case "ROW_COSMETICS":
            guard let frame = try? decoder.decode(CosmeticsFrame.self, from: data) else { return .other(text) }
            let provider = frame.provider ?? ""
            let author = provider.isEmpty || provider == "twitch" ? frame.userID : "\(provider):\(frame.userID)"
            return .cosmetics(channel: head.channel, userID: author, paint: frame.paint, badge: frame.badge)
        case "ROW_MEMBER":
            guard let frame = try? decoder.decode(MemberFrame.self, from: data) else { return .other(text) }
            return .member(userID: frame.userID, atmosphere: frame.atmosphere, cologne: frame.cologne, badge: frame.badge)
        case "ROW_PREVIEW":
            guard let frame = try? decoder.decode(PreviewFrame.self, from: data) else { return .other(text) }
            return .preview(url: frame.url, preview: frame.preview)
        case "HISTORY_SEGMENTS":
            guard let frame = try? decoder.decode(PatchFrame.self, from: data) else { return .other(text) }
            return .patch(channel: head.channel, rows: frame.rows.map {
                ChatRowPatch(id: $0.id, segments: $0.segments ?? [], view: $0.view)
            })
        case "NOTICE":
            let frame = try? decoder.decode(NoticeFrame.self, from: data)
            // A line StreamNook writes into one room itself (a command's later result, a
            // reminder that could not post) is a system row in that room.
            if frame?.msgID == localNoticeID, let line = frame?.message, !line.isEmpty,
               let row = systemRow(line, channel: head.channel ?? "") {
                return .row(row)
            }
            return .notice(channel: head.channel, msgID: frame?.msgID, message: frame?.message)
        case "AUTOMOD_HELD":
            guard let frame = try? decoder.decode(HeldFrame.self, from: data) else { return .other(text) }
            return .held(channel: head.channel, row: HeldRow(held: frame.held, line: frame.line, by: frame.by))
        case "HIDE_USERS":
            guard let frame = try? decoder.decode(HideUsersFrame.self, from: data) else { return .other(text) }
            return .hideUsers(room: frame.channelKey.flatMap { Self.roomKey($0) }, logins: frame.logins)
        case "PERSONAL_EMOTES":
            guard let frame = try? decoder.decode(PersonalEmotesFrame.self, from: data) else { return .other(text) }
            return .personalEmotes(channel: head.channel, userID: frame.userID, messageIDs: frame.messageIDs)
        case "PINNED":
            guard let frame = try? decoder.decode(PinnedFrame.self, from: data), let pin = frame.pin else {
                return .other(text)
            }
            return .pinned(channel: head.channel, pin: pin)
        case "UNPINNED":
            return .pinned(channel: head.channel, pin: nil)
        default:
            return .other(text)
        }
    }

    /// The `msg_id` of the NOTICE frames StreamNook posts into a room.
    public static let localNoticeID = "streamnook_notice"

    /// A row only the viewer sees, posted by the app in `channel`.
    public static func systemRow(_ line: String, channel: String, at date: Date = Date()) -> ChatMessage? {
        let row: [String: Any] = [
            "id": "system-local-\(UUID().uuidString)",
            "user_id": "",
            "username": "System",
            "display_name": "System",
            "color": NSNull(),
            "badges": [Any](),
            "timestamp": ISO8601DateFormatter().string(from: date),
            "content": line,
            "provider": channel.contains(":") ? String(channel.split(separator: ":")[0]) : "twitch",
            "channel": channel,
            "tags": [String: String](),
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: row) else { return nil }
        return try? decoder.decode(ChatMessage.self, from: data)
    }

    private struct PreviewFrame: Decodable {
        var url: String
        var preview: ChatLinkPreview?
    }

    private struct FrameHead: Decodable {
        var type: String?
        var id: String?
        var channel: String?
    }

    private struct RoomStateFrame: Decodable {
        var modes: ChatRoomModes?
    }

    /// A room as the core's filters key it (`twitch:xqc`, `kick:slug`) as a room here: the bare
    /// login on Twitch, the key itself elsewhere.
    static func roomKey(_ channelKey: String) -> String? {
        let key = channelKey.trimmingCharacters(in: .whitespaces).lowercased()
        let room = key.hasPrefix("twitch:") ? String(key.dropFirst("twitch:".count)) : key
        return room.isEmpty ? nil : room
    }

    private struct ClearMsgFrame: Decodable {
        var targetMsgID: String
        var login: String?
        var reason: String?
        enum CodingKeys: String, CodingKey { case targetMsgID = "target_msg_id", login, reason }
    }

    private struct HideUsersFrame: Decodable {
        var channelKey: String?
        var logins: [String]
        enum CodingKeys: String, CodingKey { case channelKey = "channel_key", logins }
    }

    private struct PersonalEmotesFrame: Decodable {
        var userID: String
        var messageIDs: [String]
        enum CodingKeys: String, CodingKey { case userID = "user_id", messageIDs = "message_ids" }
    }

    private struct PinnedFrame: Decodable {
        var pin: PinnedChat?
    }

    private struct ClearChatFrame: Decodable {
        var targetUserID: String?
        var targetUser: String?
        var banDuration: Int?
        enum CodingKeys: String, CodingKey {
            case targetUserID = "target_user_id", targetUser = "target_user", banDuration = "ban_duration"
        }
    }

    private struct CosmeticsFrame: Decodable {
        var userID: String
        var provider: String?
        var paint: ChatPaint?
        var badge: ChatViewBadge?
        enum CodingKeys: String, CodingKey { case userID = "user_id", provider, paint, badge }
    }

    private struct MemberFrame: Decodable {
        var userID: String
        var atmosphere: String?
        var cologne: CologneLook?
        var badge: ChatViewBadge?
        enum CodingKeys: String, CodingKey { case userID = "user_id", atmosphere, cologne, badge }
    }

    private struct PatchFrame: Decodable {
        struct Row: Decodable {
            var id: String
            var segments: [ChatSegment]?
            var view: ChatRowView?
        }
        var rows: [Row]
    }

    private struct HeldFrame: Decodable {
        var held: HeldMessage
        var line: String?
        var by: String?
    }

    private struct NoticeFrame: Decodable {
        var msgID: String?
        var message: String?
        enum CodingKeys: String, CodingKey { case msgID = "msg_id", message }
    }
}
