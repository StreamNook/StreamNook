import Foundation

/// A moderator's warning the viewer has not acknowledged (Rust `chat_warning::WarningView`), on
/// the room's words. The message box stays locked behind its Acknowledge button.
public struct ChatWarning: Codable, Sendable, Equatable {
    /// "A moderator warned you".
    public var title: String
    /// The reason in quotes and what to do.
    public var detail: String
    /// "Acknowledge".
    public var action: String
    /// The locked field's placeholder.
    public var prompt: String
    /// A word or two for a narrow box: "Warned".
    public var short: String

    public init(title: String, detail: String, action: String, prompt: String, short: String) {
        self.title = title
        self.detail = detail
        self.action = action
        self.prompt = prompt
        self.short = short
    }
}

/// A message AutoMod held for review (Rust `automod_queue::HeldMessage`), as the chat stream's
/// `AUTOMOD_HELD` frame carries it: once when held, again when it is decided.
public struct HeldMessage: Codable, Sendable, Equatable, Identifiable {
    public var messageID: String
    public var channel: String
    public var broadcasterID: String
    public var userID: String
    public var userLogin: String
    public var userName: String
    public var text: String
    /// "Held by AutoMod · Swearing, level 2".
    public var label: String
    public var heldAtMs: Int64
    /// "held" | "approved" | "denied" | "expired"
    public var status: String

    public var id: String { messageID }
    public var isPending: Bool { status == "held" }
    public var shownName: String { userName.isEmpty ? userLogin : userName }

    enum CodingKeys: String, CodingKey {
        case messageID = "message_id", channel, broadcasterID = "broadcaster_id", userID = "user_id",
             userLogin = "user_login", userName = "user_name", text, label, heldAtMs = "held_at_ms", status
    }

    public init(messageID: String, channel: String, broadcasterID: String, userID: String, userLogin: String,
                userName: String, text: String, label: String, heldAtMs: Int64, status: String) {
        self.messageID = messageID
        self.channel = channel
        self.broadcasterID = broadcasterID
        self.userID = userID
        self.userLogin = userLogin
        self.userName = userName
        self.text = text
        self.label = label
        self.heldAtMs = heldAtMs
        self.status = status
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        messageID = try c.decode(String.self, forKey: .messageID)
        channel = try c.decodeIfPresent(String.self, forKey: .channel) ?? ""
        broadcasterID = try c.decodeIfPresent(String.self, forKey: .broadcasterID) ?? ""
        userID = try c.decodeIfPresent(String.self, forKey: .userID) ?? ""
        userLogin = try c.decodeIfPresent(String.self, forKey: .userLogin) ?? ""
        userName = try c.decodeIfPresent(String.self, forKey: .userName) ?? ""
        text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
        label = try c.decodeIfPresent(String.self, forKey: .label) ?? "Held by AutoMod"
        heldAtMs = try c.decodeIfPresent(Int64.self, forKey: .heldAtMs) ?? 0
        status = try c.decodeIfPresent(String.self, forKey: .status) ?? "held"
    }
}

/// A held message as a chat row shows it: the message while it waits, one quiet line once
/// decided ("Allowed Gus's held message", and who decided it).
public struct HeldRow: Sendable, Equatable {
    public var held: HeldMessage
    /// The folded line once decided; nil while it waits.
    public var line: String?
    /// The moderator who decided it.
    public var by: String?

    public init(held: HeldMessage, line: String? = nil, by: String? = nil) {
        self.held = held
        self.line = line
        self.by = by
    }
}

/// One entry of a chatter's moderation history (Rust `mod_history::HistoryEntry`).
public struct ModHistoryEntry: Codable, Sendable, Equatable, Identifiable {
    /// "ban" | "timeout" | "unban" | "untimeout" | "warn" | "acknowledged" | "deleted" | "comment"
    public var kind: String
    public var id: String
    /// "Timed out 10 minutes", "Deleted message", "Mod comment".
    public var title: String
    public var by: String?
    public var atMs: Int64
    /// "3 days ago", or "sent 3 days ago" for a deleted message.
    public var when: String
    public var quote: String?
    public var seconds: Int?

    enum CodingKeys: String, CodingKey {
        case kind, id, title, by, atMs = "at_ms", when, quote, seconds
    }

    public init(kind: String, id: String, title: String, by: String? = nil, atMs: Int64 = 0, when: String = "",
                quote: String? = nil, seconds: Int? = nil) {
        self.kind = kind
        self.id = id
        self.title = title
        self.by = by
        self.atMs = atMs
        self.when = when
        self.quote = quote
        self.seconds = seconds
    }
}

/// A ban or timeout in force now.
public struct CurrentBan: Codable, Sendable, Equatable {
    public var permanent: Bool
    public var endsMs: Int64?
    public var by: String?
    public var reason: String?
    /// "Banned" or "Timed out, 8 minutes left".
    public var label: String

    enum CodingKeys: String, CodingKey {
        case permanent, endsMs = "ends_ms", by, reason, label
    }
}

/// A chatter's moderation history in one channel (Rust `mod_history::ModHistory`).
public struct ModHistory: Codable, Sendable, Equatable {
    /// "available" | "no_access" | "signed_out" | "failed"
    public var status: String
    /// Newest first.
    public var entries: [ModHistoryEntry]
    public var current: CurrentBan?
    /// One line for a status other than "available".
    public var note: String?

    public init(status: String, entries: [ModHistoryEntry] = [], current: CurrentBan? = nil, note: String? = nil) {
        self.status = status
        self.entries = entries
        self.current = current
        self.note = note
    }

    public var isAvailable: Bool { status == "available" }
}

/// A button on the user card (Rust `chatter_card::CardButton`): a timeout length, Ban, Unban or
/// one of the viewer's own, carrying the exact action the core's mod door takes.
public struct CardButton: Decodable, Sendable, Equatable, Identifiable {
    public var id: String
    /// "10m", "Ban", "Spoilers".
    public var label: String
    /// "timeout" | "ban" | "unban" | "warn"
    public var kind: String
    /// What `moderate_chatter` takes as `action`, passed through untouched.
    public var action: JSONValue
    public var reason: String?
    /// A warning with no saved reason: the viewer types one first.
    public var asksReason: Bool
    public var danger: Bool
    public var spoken: String

    enum CodingKeys: String, CodingKey {
        case id, label, kind, action, reason, asksReason = "asks_reason", danger, spoken
    }
}

/// The card's Moderate block: buttons, the viewer's own buttons and the chatter's history.
public struct CardModerate: Decodable, Sendable, Equatable {
    public var buttons: [CardButton]
    public var custom: [CardButton]
    public var history: ModHistory
}

/// What a user card shows the viewer beyond the profile (Rust `chatter_card::ChatterCardTools`).
public struct ChatterCardTools: Decodable, Sendable, Equatable {
    public var note: String?
    public var nickname: String?
    /// The viewer's own color for the chatter, when set.
    public var color: String?
    public var swatches: [String]
    /// Present in a channel where the viewer may moderate this chatter.
    public var moderate: CardModerate?

    public init(note: String? = nil, nickname: String? = nil, color: String? = nil, swatches: [String] = [],
                moderate: CardModerate? = nil) {
        self.note = note
        self.nickname = nickname
        self.color = color
        self.swatches = swatches
        self.moderate = moderate
    }
}

/// The core's answer to a mod action (Rust `moderation::ModOutcome`).
public struct ModOutcome: Decodable, Sendable, Equatable {
    public var appliedSeconds: Int?
    /// Ready for a toast.
    public var message: String
    /// A ban or timeout, which Unban reverses.
    public var undoable: Bool

    enum CodingKeys: String, CodingKey { case appliedSeconds = "applied_seconds", message, undoable }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        appliedSeconds = try c.decodeIfPresent(Int.self, forKey: .appliedSeconds)
        message = try c.decodeIfPresent(String.self, forKey: .message) ?? ""
        undoable = try c.decodeIfPresent(Bool.self, forKey: .undoable) ?? false
    }
}

/// Any JSON value, kept as decoded so it can be sent back unchanged.
public enum JSONValue: Codable, Sendable, Equatable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([JSONValue].self) { self = .array(a) }
        else { self = .object(try c.decode([String: JSONValue].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n): try c.encode(n)
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }

    /// As a Foundation value for `JSONSerialization` (whole numbers stay integers).
    public var foundation: Any {
        switch self {
        case .null: return NSNull()
        case .bool(let b): return b
        case .number(let n): return n.rounded() == n && abs(n) < 9e15 ? Int(n) as Any : n
        case .string(let s): return s
        case .array(let a): return a.map(\.foundation)
        case .object(let o): return o.mapValues(\.foundation)
        }
    }
}

extension CoreBridge {
    /// The user card's tools for a chatter in a room.
    public func cardTools(provider: String, channel: String?, channelID: String?, userID: String) async throws -> ChatterCardTools {
        try await invoke("get_chatter_card_tools", args: [
            "provider": provider, "channel": channel, "channelId": channelID, "userId": userID,
        ], as: ChatterCardTools.self)
    }

    /// Runs a card button through the core's one mod door.
    public func moderate(_ button: CardButton, broadcasterID: String, targetUserID: String, targetName: String,
                         reason: String? = nil) async throws -> ModOutcome {
        var request: [String: Any] = [
            "provider": "twitch",
            "broadcasterId": broadcasterID,
            "targetUserId": targetUserID,
            "targetName": targetName,
            "action": button.action.foundation,
        ]
        if let reason = (reason ?? button.reason)?.trimmingCharacters(in: .whitespacesAndNewlines), !reason.isEmpty {
            request["reason"] = reason
        }
        return try await invoke("moderate_chatter", args: ["request": request], as: ModOutcome.self)
    }

    /// Saves the viewer's private note on a chatter; an empty note removes it.
    public func setNote(userID: String, note: String) async throws {
        _ = try await invoke("set_user_note", args: ["userId": userID, "note": note])
    }

    /// Sets (or with nil clears) the viewer's own name color for a chatter.
    public func setNameColor(userID: String, login: String, color: String?) async throws {
        _ = try await invoke("set_chat_user_override", args: [
            "userId": userID, "username": login, "field": "color", "value": color,
        ])
    }

    /// A chatter's moderation history in the channel whose user id is `channelID`.
    public func modHistory(channelID: String, userID: String) async throws -> ModHistory {
        try await invoke("get_chatter_mod_history", args: ["channelId": channelID, "userId": userID], as: ModHistory.self)
    }

    /// Acknowledges the viewer's warning in the channel whose user id is `roomID`.
    public func acknowledgeWarning(roomID: String) async throws {
        _ = try await invoke("acknowledge_chat_warning", args: ["roomId": roomID])
    }

    /// Allows (`allow`) or denies a held message.
    public func resolveHeld(messageID: String, allow: Bool) async throws {
        _ = try await invoke("resolve_automod_message", args: ["messageId": messageID, "allow": allow])
    }
}
