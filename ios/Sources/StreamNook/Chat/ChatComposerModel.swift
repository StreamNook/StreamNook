import AVFoundation
import Foundation
import Observation
import SNKit
import UIKit

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
/// panel is open. Kept apart from the list so arriving messages never rebuild it. The rules
/// live in the core (sending, slash commands, gating, matching) and in SNKit's `ComposerText`;
/// this only carries them out and keeps the state the screen draws.
@Observable @MainActor
final class ChatComposerModel {
    enum Panel { case none, emotes }

    let session: ChatSession
    var draft = "" {
        didSet {
            guard draft != oldValue else { return }
            suggest()
            // Editing the message that failed is a new message.
            if let failure, draft.trimmingCharacters(in: .whitespacesAndNewlines) != failure.text { self.failure = nil }
        }
    }
    private(set) var sending = false
    private(set) var suggestions: [EmoteMatch] = []
    var panel: Panel = .none {
        didSet { if panel != oldValue { picker.setOpen(panel == .emotes) } }
    }
    var menuOpen = false
    var commandsOpen = false
    var toast: String?
    /// The picker keeps its tab, search and answer between opens.
    let picker: EmotePickerModel
    /// Opens a chatter's profile (`/usercard`), set by the screen around the composer.
    @ObservationIgnored var onProfile: (ProfileTarget) -> Void = { _ in }
    private var lastSent: String?
    private var suggestSeq = 0
    /// Asks the field to take or give up focus (drive steps, inserts).
    private(set) var focusRequest: (Int, Bool) = (0, false)

    func requestFocus(_ on: Bool) {
        focusRequest = (focusRequest.0 + 1, on)
    }

    init(session: ChatSession) {
        self.session = session
        picker = EmotePickerModel(session: session)
    }

    var canSend: Bool { !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !sending }

    /// A message Twitch did not take, kept in the draft with what went wrong, until it is sent
    /// again or discarded.
    struct Failure: Equatable {
        var text: String
        var why: String
    }
    private(set) var failure: Failure?

    func discardFailure() {
        if let failure, draft == failure.text { draft = "" }
        failure = nil
    }

    // MARK: Slow mode

    /// Seconds until slow mode lets the viewer send again (0 when it does now). The wait comes
    /// from the core's words for the room, which leave it at 0 for anyone slow mode spares.
    private(set) var slowWait = 0
    @ObservationIgnored private var slowTask: Task<Void, Never>?

    #if DEBUG
    /// Drive steps: the countdown as if a message had just gone out, and a failed send.
    func debugSlow() { startSlowCountdown() }
    func debugFailure(_ why: String) {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        failure = Failure(text: text, why: why)
    }
    #endif

    /// Counts the slow-mode wait down from a message that just went out.
    private func startSlowCountdown() {
        slowTask?.cancel()
        guard let wait = session.modes?.slowSeconds, wait > 0 else {
            slowWait = 0
            return
        }
        let until = Date().addingTimeInterval(TimeInterval(wait))
        slowTask = Task { [weak self] in
            while !Task.isCancelled {
                let left = max(0, Int(until.timeIntervalSinceNow.rounded(.up)))
                self?.slowWait = left
                if left == 0 { return }
                try? await Task.sleep(for: .milliseconds(250))
            }
        }
    }

    /// Twitch's message limit; a counter shows from `counterFrom` characters, red past the limit.
    static let messageLimit = 500
    static let counterFrom = 400

    // MARK: Gating

    /// Why the viewer cannot type here, when the core proves it (their badges, their follow).
    var blockedReason: String? {
        guard let modes = session.modes, modes.blocked != nil else { return nil }
        return modes.reason ?? "Chat restricted"
    }

    /// The field takes no typing while blocked; the picker still writes into the draft, so an
    /// emote-only message is composed by tapping emotes and sent with the button.
    var fieldDisabled: Bool { blockedReason != nil }

    /// The room's modes as the composer shows them, or nil while the setting hides them.
    var shownModes: ChatRoomModes? {
        guard session.style.showRoomModes, let modes = session.modes, !modes.modes.isEmpty else { return nil }
        return modes
    }

    /// The placeholder: why the viewer cannot send when a mode stops them, else "Send a message"
    /// (nothing at all when the viewer hides it). A mode that only may hold them back is said
    /// once, by its icon in the field; the chat menu names it in full.
    var placeholder: String {
        if let blockedReason { return blockedReason }
        return session.style.hidePlaceholder ? "" : "Send a message"
    }

    // MARK: Sending

    func send() async {
        guard !sending, slowWait == 0 else { return }
        switch ComposerRoute.of(draft) {
        case .none:
            return
        case .slash(let text):
            await runSlash(text)
        case .chat(let text):
            // Plain text can still be one of your own commands (those that need no slash).
            if session.roomID != nil {
                await runSlash(text)
            } else {
                await sendChat(text, reply: session.replyTo, clearsReply: true)
            }
        }
    }

    /// Sends chat text to the room. Returns whether Twitch took it. A drop or an error keeps the
    /// draft, so nothing typed is lost: a drop prints why in the chat, an error shows a toast.
    @discardableResult
    private func sendChat(_ text: String, reply: ChatMessage?, replyID: String? = nil, pin: Bool = false,
                          clearsReply: Bool) async -> Bool {
        sending = true
        defer { sending = false }
        var message = text
        // Twitch refuses the same message twice in a row; an invisible tag makes the repeat new.
        if session.style.bypassDuplicate, message == lastSent { message += " \u{E0000}" }
        do {
            let json = try await session.core.invoke("send_chat_message", args: [
                "message": message,
                "replyParentMsgId": replyID ?? reply?.id,
                "targetChannel": session.channel,
                "broadcasterId": session.roomID,
                "senderId": nil,
                "senderAccountId": nil,
                "pinAfterSend": pin ? true : nil,
            ])
            let result = (try? JSONDecoder().decode(SendResult.self, from: Data(json.utf8))) ?? SendResult(isSent: true)
            if result.dropLine != nil {
                failure = Failure(text: text, why: result.dropReason ?? "the room refused it")
                return false
            }
            await showOwnRow(text: message, reply: reply)
            lastSent = text
            startSlowCountdown()
            failure = nil
            draft = ""
            if clearsReply { session.replyTo = nil }
            return true
        } catch {
            failure = Failure(text: text, why: Self.words(error))
            chatLog.notice("send failed: \(String(describing: error), privacy: .public)")
            return false
        }
    }

    /// A command goes to the core's dispatcher with the room it was typed in (a slash command, or
    /// plain text that may be one of your own). Whatever it does not run goes to chat as typed
    /// (Twitch still runs /me).
    private func runSlash(_ text: String) async {
        guard let roomID = session.roomID else {
            show(SlashOutcome.noRoomToast)
            draft = ""
            return
        }
        sending = true
        var outcome = SlashOutcome(handled: false)
        do {
            let json = try await session.core.invoke("run_slash_command", args: [
                "channel": session.channel, "broadcasterId": roomID, "text": text, "depth": 0, "platform": "ios",
            ])
            if let decoded = try? JSONDecoder().decode(SlashOutcome.self, from: Data(json.utf8)) { outcome = decoded }
        } catch {
            chatLog.notice("slash dispatch failed: \(String(describing: error), privacy: .public)")
        }
        sending = false
        // The core's order: toasts, then the page action, then notices (so a clear prints into
        // the emptied chat), then the text it hands back to send.
        if let last = outcome.toasts.last { show(last.text) }
        if let ui = outcome.ui { perform(ui, typed: text) }
        outcome.notices.forEach(notice)
        guard outcome.handled else {
            await sendChat(text, reply: session.replyTo, clearsReply: true)
            return
        }
        if let line = outcome.send {
            // The drafted reply stays: a command's own text replies only when it says so (/reply).
            let parent = outcome.replyTo.flatMap { session.pipeline.message($0) }
            await sendChat(line, reply: parent, replyID: outcome.replyTo, pin: outcome.pinAfterSend, clearsReply: false)
        }
        if outcome.clearDraft { draft = "" }
    }

    @ObservationIgnored private let speech = AVSpeechSynthesizer()

    /// Page actions a command asks for. Those with no iPhone surface say so.
    private func perform(_ action: SlashOutcome.UIAction, typed: String) {
        switch action.kind {
        case "open_user_card":
            if let login = action.login, !login.isEmpty {
                onProfile(ProfileTarget(userID: action.userID ?? "", login: login,
                                        displayName: action.displayName ?? login,
                                        roomID: session.roomID, roomLogin: session.channel))
            }
        case "open_url":
            if let raw = action.url, let url = URL(string: raw) { UIApplication.shared.open(url) }
        case "copy_text":
            UIPasteboard.general.string = action.text
        case "reload_chat":
            ChatReload.reload(session)
        case "refresh_emotes":
            picker.reload(debounce: false)
        case "speak":
            if let text = action.text, !text.isEmpty { speech.speak(AVSpeechUtterance(string: text)) }
        default:
            let command = typed.split(separator: " ").first.map(String.init) ?? typed
            show("\(command) is not on iPhone yet")
            chatLog.notice("slash ui \(action.kind, privacy: .public) has no iPhone surface")
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

    /// A line only the viewer sees, in this room's chat.
    func notice(_ line: String) {
        #if DEBUG
        chatLog.notice("composer notice: \(line, privacy: .public)")
        #endif
        let row: [String: Any] = [
            "id": "system-local-\(UUID().uuidString)",
            "user_id": "",
            "username": "System",
            "display_name": "System",
            "color": NSNull(),
            "badges": [Any](),
            "timestamp": ISO8601DateFormatter().string(from: Date()),
            "content": line,
            "provider": "twitch",
            "channel": session.channel,
            "tags": [String: String](),
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: row) else { return }
        session.pipeline.ingest(String(decoding: data, as: UTF8.self))
    }

    func show(_ text: String) {
        #if DEBUG
        chatLog.notice("composer toast: \(text, privacy: .public)")
        #endif
        toast = text
        Task { [weak self] in
            try? await Task.sleep(for: .seconds(3))
            if self?.toast == text { self?.toast = nil }
        }
    }

    /// A core error in words: the core's own sentence when it sent one.
    static func words(_ error: Error) -> String {
        if case CoreFailure.command(let json) = error,
           let text = try? JSONDecoder().decode(String.self, from: Data(json.utf8)) {
            return text
        }
        return "\(error)"
    }

    // MARK: Inserting

    /// Puts an emote or emoji into the draft with a space after it, and one before it when the
    /// draft does not already end in one. Works while the field is disabled (emote-only).
    func insert(_ text: String) {
        draft = ComposerText.inserting(text, into: draft)
    }

    /// Replaces the last word with a suggestion and a space.
    func complete(_ suggestion: EmoteMatch) {
        draft = ComposerText.completing(draft, with: suggestion.text)
    }

    func replaceWithCommand(_ name: String) {
        draft = ComposerText.command(name)
    }

    // MARK: Suggestions

    private func suggest() {
        let word = ComposerText.lastWord(draft)
        suggestSeq += 1
        let seq = suggestSeq
        guard ComposerText.wantsSuggestions(draft), session.style.suggestionsEnabled else {
            suggestions = []
            return
        }
        let core = session.core
        let channel = session.channel
        let channelID = session.roomID
        let tier = EmotePickerModel.tier
        Task { [weak self] in
            // One answer: the emotes, then the room's recent chatters (provider "user").
            guard let json = try? await core.invoke("match_emote_tokens", args: [
                "provider": "twitch", "channel": channel, "channelId": channelID, "query": word,
                "profile": "cycle", "order": "default", "tier": tier, "limit": nil,
            ]), let result = try? JSONDecoder().decode(EmoteMatchResult.self, from: Data(json.utf8)) else { return }
            guard let self, seq == self.suggestSeq else { return }
            self.suggestions = result.rows
        }
    }
}
