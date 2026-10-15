import SNKit
import UIKit

/// What a released fan bucket does. Moderator actions go to the core with the room's numeric
/// Twitch id and answer with the toast the phone shows.
@MainActor
enum ChatModActions {
    static func run(_ bucket: FanBucket, message: ChatMessage, seconds: Int?, session: ChatSession,
                    onProfile: (ProfileTarget) -> Void) async -> String? {
        let core = session.core
        let room = session.roomID ?? ""
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
                return "Could not delete that message."
            }
        case .timeout:
            let duration = seconds ?? 600
            do {
                _ = try await core.invoke("ban_user", args: [
                    "broadcasterId": room, "targetUserId": message.userID, "duration": duration, "reason": nil,
                ])
                return "Timed out \(message.username) for \(ChatRowLayoutBuilder.duration(duration))"
            } catch {
                return "Could not time out that user."
            }
        case .ban:
            do {
                _ = try await core.invoke("ban_user", args: [
                    "broadcasterId": room, "targetUserId": message.userID, "duration": nil, "reason": nil,
                ])
                return "Banned \(message.username)"
            } catch {
                return "Could not ban that user."
            }
        case .pin:
            do {
                _ = try await core.invoke("pin_chat_message", args: [
                    "broadcasterId": room, "messageId": message.id, "durationSeconds": nil,
                ])
                return nil
            } catch {
                return "Could not pin that message."
            }
        }
    }
}
