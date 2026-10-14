import Foundation
import Observation
import SNKit
import UIKit

/// An emote row as the core's `match_emote_tokens` answers it.
struct EmoteMatch: Decodable, Identifiable, Hashable, Sendable {
    var id: String
    var name: String
    var insertText: String?
    var url: String
    var provider: String
    var sourceLabel: String?
    var group: String?

    enum CodingKeys: String, CodingKey {
        case id, name, url, provider, group
        case insertText = "insert_text", sourceLabel = "source_label"
    }

    var text: String { insertText ?? name }
}

struct EmoteMatchResult: Decodable, Sendable {
    var rows: [EmoteMatch]
    var total: Int
    var ready: Bool
}

/// One suggestion in the strip over the composer: an emote, or a chatter to mention.
enum ComposerSuggestion: Identifiable, Hashable {
    case emote(EmoteMatch)
    case chatter(display: String, login: String)

    var id: String {
        switch self {
        case .emote(let e): return "e:\(e.provider):\(e.id):\(e.name)"
        case .chatter(_, let login): return "c:\(login)"
        }
    }

    var name: String {
        switch self {
        case .emote(let e): return e.name
        case .chatter(let display, _): return display
        }
    }

    var kind: String {
        switch self {
        case .emote(let e): return e.provider
        case .chatter: return "user"
        }
    }
}

/// A command row as the core's `command_catalog` answers it.
struct CommandRow: Decodable, Identifiable, Hashable, Sendable {
    var name: String
    var usage: String
    var description: String
    var category: String
    var label: String
    var lock: String?
    var id: String { "\(category):\(name)" }
}

/// What the composer is doing: the draft, the send in flight, the suggestion strip and which
/// panel is open. Kept apart from the list so arriving messages never rebuild it.
@Observable @MainActor
final class ChatComposerModel {
    enum Panel { case none, emotes }

    let session: ChatSession
    var draft = "" {
        didSet { if draft != oldValue { suggest() } }
    }
    private(set) var sending = false
    private(set) var suggestions: [ComposerSuggestion] = []
    var panel: Panel = .none
    var menuOpen = false
    var commandsOpen = false
    var toast: String?
    private var lastSent: String?
    private var suggestSeq = 0
    /// Asks the field to take or give up focus (drive steps, inserts).
    private(set) var focusRequest: (Int, Bool) = (0, false)

    func requestFocus(_ on: Bool) {
        focusRequest = (focusRequest.0 + 1, on)
    }

    init(session: ChatSession) {
        self.session = session
    }

    var canSend: Bool { !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !sending }

    /// The placeholder: the room mode that may hold the viewer back, else "Send a message"
    /// (nothing at all when the viewer hides it).
    var placeholder: String {
        if let prompt = session.modes?.prompt { return prompt }
        return session.style.hidePlaceholder ? "" : "Send a message"
    }

    // MARK: Sending

    func send() async {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !sending else { return }
        sending = true
        defer { sending = false }
        var message = text
        // Twitch refuses the same message twice in a row; an invisible tag makes the repeat new.
        if session.style.bypassDuplicate, message == lastSent { message += " \u{E0000}" }
        let reply = session.replyTo
        do {
            let json = try await session.core.invoke("send_chat_message", args: [
                "message": message,
                "replyParentMsgId": reply?.id,
                "targetChannel": session.channel,
                "broadcasterId": session.roomID,
                "senderId": nil,
                "senderAccountId": nil,
                "pinAfterSend": nil,
            ])
            let result = (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any]
            if let sent = result?["is_sent"] as? Bool, !sent {
                let reason = result?["drop_reason"] as? String ?? "the room refused it"
                show("Your message was not sent: \(reason)")
            } else {
                await showOwnRow(text: message, reply: reply)
            }
            lastSent = text
            draft = ""
            session.replyTo = nil
        } catch {
            // The draft and the reply stay, so nothing typed is lost.
            show("\(error)")
            chatLog.notice("send failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// Twitch never echoes your own message, so the core builds your row with the same parser.
    private func showOwnRow(text: String, reply: ChatMessage?) async {
        guard let userJSON = try? await session.core.invoke("get_user_info", "{}"),
              let user = try? JSONDecoder().decode(TwitchUser.self, from: Data(userJSON.utf8)) else { return }
        var args: [String: Any] = [
            "channel": session.channel,
            "text": text,
            "local_id": "local-\(UUID().uuidString)",
            "sender_id": user.id,
            "sender_login": user.login,
            "sender_display_name": user.displayName,
            "room_id": session.roomID ?? "",
        ]
        if let reply {
            args["reply_to"] = ["id": reply.id, "user_id": reply.userID, "login": reply.username, "display_name": reply.displayName, "body": reply.content]
        }
        guard let row = try? await session.core.invoke("build_own_chat_message", args: ["message": args]),
              row != "null" else { return }
        session.pipeline.ingest(row)
    }

    func show(_ text: String) {
        toast = text
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            if self?.toast == text { self?.toast = nil }
        }
    }

    // MARK: Inserting

    /// Puts an emote or emoji into the draft with a space after it, and one before it when the
    /// draft does not already end in one.
    func insert(_ text: String) {
        let needsSpace = !draft.isEmpty && !draft.hasSuffix(" ")
        draft += (needsSpace ? " " : "") + text + " "
    }

    /// Replaces the last word with a suggestion and a space.
    func complete(_ suggestion: ComposerSuggestion) {
        let word = Self.lastWord(draft)
        let base = String(draft.dropLast(word.count))
        switch suggestion {
        case .emote(let e):
            draft = base + e.text + " "
        case .chatter(let display, _):
            draft = base + (word.hasPrefix("@") ? "@" : "") + display + " "
        }
    }

    func replaceWithCommand(_ name: String) {
        draft = "/\(name) "
    }

    // MARK: Suggestions

    /// The run of non-space characters at the end of the draft.
    static func lastWord(_ text: String) -> String {
        guard let last = text.last, !last.isWhitespace else { return "" }
        return String(text.reversed().prefix { !$0.isWhitespace }.reversed())
    }

    private func suggest() {
        let word = Self.lastWord(draft)
        suggestSeq += 1
        let seq = suggestSeq
        guard word.count >= 2 else {
            suggestions = []
            return
        }
        let core = session.core
        let channel = session.channel
        let channelID = session.roomID
        let scale = UITraitCollection.current.displayScale
        Task { [weak self] in
            var rows: [ComposerSuggestion] = []
            if !word.hasPrefix("@"),
               let json = try? await core.invoke("match_emote_tokens", args: [
                   "provider": "twitch", "channel": channel, "channelId": channelID, "query": word,
                   "profile": "cycle", "order": "default", "tier": scale >= 3 ? "3x" : "2x", "limit": nil,
               ]),
               let result = try? JSONDecoder().decode(EmoteMatchResult.self, from: Data(json.utf8)) {
                rows = result.rows.map { .emote($0) }
            }
            guard let self, seq == self.suggestSeq else { return }
            if !word.hasPrefix(":"), rows.count < 50 {
                rows += self.chatters(matching: word, excluding: Set(rows.map { $0.name.lowercased() }))
            }
            self.suggestions = Array(rows.prefix(50))
        }
    }

    /// Up to five chatters seen in this room whose name starts with the word, newest first.
    private func chatters(matching word: String, excluding: Set<String>) -> [ComposerSuggestion] {
        let needle = word.trimmingCharacters(in: CharacterSet(charactersIn: "@")).lowercased()
        guard !needle.isEmpty else { return [] }
        var seen = Set<String>()
        var out: [ComposerSuggestion] = []
        for message in session.pipeline.recentMessages(300).reversed() where !message.isSystem {
            let login = message.username.lowercased()
            guard !login.isEmpty, !seen.contains(login) else { continue }
            seen.insert(login)
            let display = message.displayName.isEmpty ? message.username : message.displayName
            guard login.hasPrefix(needle) || display.lowercased().hasPrefix(needle), !excluding.contains(display.lowercased())
            else { continue }
            out.append(.chatter(display: display, login: login))
            if out.count == 5 { break }
        }
        return out
    }
}
