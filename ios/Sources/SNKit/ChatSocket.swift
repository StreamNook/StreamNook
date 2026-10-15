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

    enum CodingKeys: String, CodingKey {
        case modes, prompt, promptShort = "prompt_short", standing, slowSeconds = "slow_seconds", blocked, reason
    }

    public init(modes: [ChatRoomMode] = [], prompt: String? = nil, promptShort: String? = nil, standing: String? = nil,
                slowSeconds: Int = 0, blocked: String? = nil, reason: String? = nil) {
        self.modes = modes
        self.prompt = prompt
        self.promptShort = promptShort
        self.standing = standing
        self.slowSeconds = slowSeconds
        self.blocked = blocked
        self.reason = reason
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
    }
}

/// A row patch the core sends when it re-tokenised rows (`HISTORY_SEGMENTS`).
public struct ChatRowPatch: Sendable, Equatable {
    public var id: String
    public var segments: [ChatSegment]
    public var view: ChatRowView?
}

/// One frame from the core's chat socket, decoded. The socket sends JSON chat rows (they have
/// `id`), JSON control frames (they have `type`), status words and `NAME:` prefixed status lines.
public enum ChatSocketFrame: Sendable, Equatable {
    case row(ChatMessage)
    case roomState(channel: String?, modes: ChatRoomModes?)
    case clearMessage(channel: String?, targetID: String)
    /// A timeout (`duration` seconds), a ban (`duration` nil) or, with no target, a full clear.
    case clearChat(channel: String?, targetUserID: String?, targetUser: String?, duration: Int?)
    /// A chatter's 7TV cosmetics that arrived after their first row.
    case cosmetics(channel: String?, userID: String, paint: ChatPaint?, badge: ChatViewBadge?)
    case patch(channel: String?, rows: [ChatRowPatch])
    case notice(channel: String?, msgID: String?, message: String?)
    case status(String)
    case other(String)

    private static let decoder = JSONDecoder()

    public static func decode(_ raw: String) -> ChatSocketFrame {
        let text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard text.hasPrefix("{"), let data = text.data(using: .utf8) else {
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
            return .clearMessage(channel: head.channel, targetID: frame.targetMsgID)
        case "CLEARCHAT":
            let frame = try? decoder.decode(ClearChatFrame.self, from: data)
            return .clearChat(channel: head.channel, targetUserID: frame?.targetUserID, targetUser: frame?.targetUser,
                              duration: frame?.banDuration)
        case "ROW_COSMETICS":
            guard let frame = try? decoder.decode(CosmeticsFrame.self, from: data) else { return .other(text) }
            return .cosmetics(channel: head.channel, userID: frame.userID, paint: frame.paint, badge: frame.badge)
        case "HISTORY_SEGMENTS":
            guard let frame = try? decoder.decode(PatchFrame.self, from: data) else { return .other(text) }
            return .patch(channel: head.channel, rows: frame.rows.map {
                ChatRowPatch(id: $0.id, segments: $0.segments ?? [], view: $0.view)
            })
        case "NOTICE":
            let frame = try? decoder.decode(NoticeFrame.self, from: data)
            return .notice(channel: head.channel, msgID: frame?.msgID, message: frame?.message)
        default:
            return .other(text)
        }
    }

    private struct FrameHead: Decodable {
        var type: String?
        var id: String?
        var channel: String?
    }

    private struct RoomStateFrame: Decodable {
        var modes: ChatRoomModes?
    }

    private struct ClearMsgFrame: Decodable {
        var targetMsgID: String
        enum CodingKeys: String, CodingKey { case targetMsgID = "target_msg_id" }
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
        var paint: ChatPaint?
        var badge: ChatViewBadge?
        enum CodingKeys: String, CodingKey { case userID = "user_id", paint, badge }
    }

    private struct PatchFrame: Decodable {
        struct Row: Decodable {
            var id: String
            var segments: [ChatSegment]?
            var view: ChatRowView?
        }
        var rows: [Row]
    }

    private struct NoticeFrame: Decodable {
        var msgID: String?
        var message: String?
        enum CodingKeys: String, CodingKey { case msgID = "msg_id", message }
    }
}
