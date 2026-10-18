import SNKit
import UIKit

/// What a released fan bucket does. Delete, timeout, ban and Undo go through the core's one mod
/// door (`moderate_chatter`), which words the toast; there is no confirmation, the far-arc reach
/// is the protection.
@MainActor
enum ChatModActions {
    /// The toast a bucket answers with: a failure, or a timeout or ban that offers Undo.
    struct Outcome {
        var text: String
        var success = false
        /// Lifts the timeout or ban again.
        var undo: (@MainActor () -> Void)?
    }

    static func run(_ bucket: FanBucket, message: ChatMessage, seconds: Int?, session: ChatSession,
                    onProfile: (ProfileTarget) -> Void) async -> Outcome? {
        let core = session.core
        let room = session.roomID ?? ""
        /// One action through the core's mod door (`moderate_chatter`), which words the answer.
        func moderate(_ action: [String: Any]) async throws -> ModOutcome {
            try await core.invoke("moderate_chatter", args: ["request": [
                "provider": "twitch", "broadcasterId": room, "targetUserId": message.userID,
                "targetName": message.shownName, "action": action,
            ] as [String: Any]], as: ModOutcome.self)
        }
        let undo: @MainActor (Bool) -> Void = { timeout in
            Task { _ = try? await moderate(["kind": "unban", "timeout": timeout]) }
        }
        switch bucket {
        case .reply:
            session.replyTo = message
            return nil
        case .profile:
            // The fan-out names the chatter by login alone, in Twitch purple, with no badges.
            onProfile(ProfileTarget(userID: message.userID, login: message.username, displayName: message.username,
                                    color: "#9147FF", roomID: session.roomID, roomLogin: session.channel))
            return nil
        case .copy:
            UIPasteboard.general.string = message.content
            return nil
        case .delete:
            do {
                _ = try await moderate(["kind": "delete", "messageId": message.id])
                return nil
            } catch {
                return Outcome(text: ChatComposerModel.words(error))
            }
        case .timeout:
            do {
                let outcome = try await moderate(["kind": "timeout", "seconds": seconds ?? 600])
                return Outcome(text: outcome.message, success: true, undo: { undo(true) })
            } catch {
                return Outcome(text: ChatComposerModel.words(error))
            }
        case .ban:
            do {
                let outcome = try await moderate(["kind": "ban"])
                return Outcome(text: outcome.message, success: true, undo: { undo(false) })
            } catch {
                return Outcome(text: ChatComposerModel.words(error))
            }
        case .pin:
            do {
                // The core re-reads the room's pins until they show this one, so the banner
                // updates within a moment of Twitch reflecting it.
                _ = try await core.invoke("pin_chat_message", args: [
                    "broadcasterId": room, "messageId": message.id, "durationSeconds": nil,
                ])
                return nil
            } catch {
                return Outcome(text: "Could not pin that message.")
            }
        }
    }
}
