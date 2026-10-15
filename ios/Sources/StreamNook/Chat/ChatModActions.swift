import SNKit
import UIKit

/// What a released fan bucket does. Moderator actions go to the core with the room's numeric
/// Twitch id; there is no confirmation, the far-arc reach is the protection.
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
        let undo: @MainActor () -> Void = {
            Task { _ = try? await core.invoke("unban_user", args: ["broadcasterId": room, "targetUserId": message.userID]) }
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
                _ = try await core.invoke("delete_chat_message", args: ["broadcasterId": room, "messageId": message.id])
                return nil
            } catch {
                return Outcome(text: "Could not delete that message.")
            }
        case .timeout:
            let duration = seconds ?? 600
            do {
                _ = try await core.invoke("ban_user", args: [
                    "broadcasterId": room, "targetUserId": message.userID, "duration": duration, "reason": nil,
                ])
                return Outcome(text: "Timed out \(message.username) for \(ChatRowLayoutBuilder.duration(duration))",
                               success: true, undo: undo)
            } catch {
                return Outcome(text: "Could not time out that user.")
            }
        case .ban:
            do {
                _ = try await core.invoke("ban_user", args: [
                    "broadcasterId": room, "targetUserId": message.userID, "duration": nil, "reason": nil,
                ])
                return Outcome(text: "Banned \(message.username)", success: true, undo: undo)
            } catch {
                return Outcome(text: "Could not ban that user.")
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
