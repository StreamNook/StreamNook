import Foundation
import SNKit

/// Cheering from the composer. While the box cheers, Rust's live check (`check_cheer`, the
/// cached balance, no network per key) says what Send will spend. Send goes through
/// `send_chat_message` as always, and Rust holds any message that cheers. When the bar already
/// showed that very message with nothing in the way, Send was the yes and `send_cheer` runs at
/// once; otherwise the bar asks first. Nothing is spent anywhere else.
extension ChatComposerModel {
    /// The box cheers and Rust says it cannot go out as it is: Send waits.
    var cheerBlocked: Bool { canSend && !(cheer.draft?.problems.isEmpty ?? true) }

    /// A cheer word always ends in its amount, so a box with no digit is no cheer.
    func checkCheer() {
        cheerCheck?.cancel()
        let text = draft
        guard !session.isKick, let roomID = session.roomID,
              !text.trimmingCharacters(in: .whitespaces).isEmpty, text.contains(where: \.isNumber) else {
            if cheer.draft != nil { cheer.draft = nil }
            return
        }
        let core = session.core
        let channel = session.channel
        cheerCheck = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            let json = try? await core.invoke("check_cheer", args: ["channelLogin": channel, "channelId": roomID, "text": text])
            guard !Task.isCancelled, let self, self.draft == text else { return }
            self.cheer.draft = json.flatMap { try? JSONDecoder().decode(CheerCheck?.self, from: Data($0.utf8)) }
        }
    }

    /// Rust held the message on Send.
    func heldCheer(_ check: CheerCheck) async {
        switch cheer.held(check) {
        case .send(let check): await sendCheer(check)
        case .ask, .restore: break
        }
    }

    /// The bar's yes, or "Cheer without the message".
    func sendCheer(_ check: CheerCheck, continueAnyway: Bool = false) async {
        cheer.sending(check)
        let outcome: CheerOutcome
        do {
            outcome = try await session.core.invoke(
                "send_cheer",
                args: ["channelLogin": check.channel, "channelId": check.channelID, "text": check.text, "continueAnyway": continueAnyway],
                as: CheerOutcome.self)
        } catch {
            outcome = CheerOutcome(sent: false, problem: Self.words(error))
        }
        if cheer.answered(outcome, for: check) == nil {
            // It went out: the cheer's own chat row comes back from Twitch.
            if draft.trimmingCharacters(in: .whitespacesAndNewlines) == check.text.trimmingCharacters(in: .whitespacesAndNewlines) {
                draft = ""
            }
        }
    }

    /// The bar's close: a held or refused cheer is set aside (the words stay in the box);
    /// a live one has its cheers taken out of the box.
    func closeCheer() {
        if cheer.status != nil {
            cheer.dismiss()
        } else if let live = cheer.draft {
            draft = live.plainText
            cheer.draft = nil
        }
    }

    /// The Bits tab's pick: the cheer word joins the box.
    func insertCheer(_ word: String) {
        let trimmed = draft.trimmingCharacters(in: .whitespaces)
        draft = trimmed.isEmpty ? "\(word) " : "\(trimmed) \(word) "
        requestFocus(true)
    }
}
