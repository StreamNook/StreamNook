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

extension CoreBridge {
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
