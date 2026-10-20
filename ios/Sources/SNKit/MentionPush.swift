import Foundation

/// Mentions while you're away on the iPhone: notify.streamnook.app sends an APNs alert when its
/// mention watcher finds a mention of you in a followed live channel the app is not in. The core
/// keeps the server in step (`services/mention_watch.rs`) and decides, while the app is in front,
/// whether the banner shows (`mention_push_arrived`); a tap opens it through `open_mention`, which
/// pulls it from the server first when it is not in Mentions yet. These are the parts with no
/// UIKit in them.
public enum MentionPush {
    /// A mention push, as the server sends it under the `sn` key (`type` is `mention`).
    public struct Alert: Equatable, Sendable {
        /// The message id: the inbox entry's id, and the push's collapse id.
        public let id: String
        /// The chat's room key (a Twitch login).
        public let room: String

        public init(id: String, room: String) {
            self.id = id
            self.room = room
        }
    }

    /// The mention push a notification carries, or nil for any other notification.
    public static func alert(in userInfo: [AnyHashable: Any]) -> Alert? {
        guard let sn = userInfo["sn"] as? [String: Any], sn["type"] as? String == "mention",
              let id = sn["mention_id"] as? String, !id.isEmpty,
              let room = sn["room"] as? String, !room.isEmpty else { return nil }
        return Alert(id: id, room: room)
    }
}
