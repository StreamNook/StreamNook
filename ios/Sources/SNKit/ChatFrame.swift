import Foundation

/// One frame from the core's chat socket, reduced to a line for the spike's frame list. The
/// socket sends what chatConnectionStore.ts reads: a JSON chat message (has `id`), a JSON
/// control frame (has `type`), a `NAME:` prefixed status line, or a raw IRC line.
public struct ChatFrame: Sendable, Equatable {
    public enum Kind: String, Sendable {
        case message, control, status, irc
    }

    public var kind: Kind
    /// Who spoke, for chat messages.
    public var author: String?
    public var text: String

    /// The status prefixes the socket uses, as chatConnectionStore.ts checks them.
    static let statusPrefixes = [
        "PARKED_TICK:", "PARK_END:", "PARK_GAP:", "RECONNECTING:", "RECONNECT_FAILED:",
        "CONNECTION_WARNING:", "USER_BADGES:", "USER_COLOR:",
    ]
    /// The whole-frame status words it sends.
    static let statusWords: Set<String> = [
        "HEARTBEAT", "IRC_CONNECTED", "RECONNECTED", "IRC_CONNECT_RETRY", "IRC_RECONNECTING",
    ]

    /// The signed-in viewer's own chat color from a `USER_COLOR:` line (Twitch's USERSTATE), as
    /// chatConnectionStore.ts reads it: `USER_COLOR:#<channel>:<color>`, or the older
    /// `USER_COLOR:<color>` that names no room. Nil for any other line and for an empty color.
    public static func ownColor(_ line: String) -> (channel: String?, color: String)? {
        let prefix = "USER_COLOR:"
        guard line.hasPrefix(prefix) else { return nil }
        let payload = line.dropFirst(prefix.count)
        var channel: String?
        var color = String(payload)
        if payload.hasPrefix("#"), let colon = payload.firstIndex(of: ":"),
           payload.distance(from: payload.startIndex, to: colon) > 1 {
            channel = payload[payload.index(after: payload.startIndex)..<colon].lowercased()
            color = String(payload[payload.index(after: colon)...])
        }
        return color.isEmpty ? nil : (channel, color)
    }

    public init(kind: Kind, author: String? = nil, text: String) {
        self.kind = kind
        self.author = author
        self.text = text
    }

    public init(raw: String) {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.hasPrefix("{"), let data = trimmed.data(using: .utf8),
           let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] {
            if let type = object["type"] as? String {
                let detail = (object["message"] as? String) ?? (object["channel"] as? String) ?? ""
                self.init(kind: .control, text: detail.isEmpty ? type : "\(type) \(detail)")
                return
            }
            if object["id"] != nil {
                let name = (object["display_name"] as? String).flatMap { $0.isEmpty ? nil : $0 }
                    ?? (object["username"] as? String) ?? "?"
                self.init(kind: .message, author: name, text: (object["content"] as? String) ?? "")
                return
            }
        }
        if Self.statusWords.contains(trimmed) {
            self.init(kind: .status, text: trimmed)
            return
        }
        if let prefix = Self.statusPrefixes.first(where: { trimmed.hasPrefix($0) }) {
            let name = String(prefix.dropLast())
            self.init(kind: .status, text: name)
            return
        }
        self.init(kind: .irc, text: Self.ircSummary(trimmed))
    }

    /// The command and its trailing text of a raw IRC line, without the tags.
    static func ircSummary(_ line: String) -> String {
        var rest = Substring(line)
        if rest.hasPrefix("@"), let space = rest.firstIndex(of: " ") { rest = rest[rest.index(after: space)...] }
        if rest.hasPrefix(":"), let space = rest.firstIndex(of: " ") { rest = rest[rest.index(after: space)...] }
        let short = rest.count > 120 ? String(rest.prefix(120)) + "..." : String(rest)
        return short
    }

    /// The line the frame list shows.
    public var summary: String {
        switch kind {
        case .message: return "\(author ?? "?"): \(text)"
        case .control: return "[\(text)]"
        case .status: return "<\(text)>"
        case .irc: return text
        }
    }
}

/// The last frames, newest last, capped.
public struct FrameLog: Sendable {
    public private(set) var frames: [ChatFrame] = []
    public let limit: Int

    public init(limit: Int = 30) {
        self.limit = limit
    }

    public mutating func append(_ frame: ChatFrame) {
        frames.append(frame)
        if frames.count > limit { frames.removeFirst(frames.count - limit) }
    }
}
