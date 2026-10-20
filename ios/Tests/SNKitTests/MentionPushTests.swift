import Foundation
import SNKit
import Testing

@Suite struct MentionPushTests {
    @Test func readsTheServersMentionPayload() {
        let info: [AnyHashable: Any] = [
            "aps": ["alert": ["title": "Viewer replied to you in bigstreamer", "body": "thanks"], "thread-id": "bigstreamer"],
            "sn": ["type": "mention", "mention_id": "push-1", "platform": "twitch", "room": "bigstreamer",
                   "channel_id": "500", "kind": "reply", "sender_login": "viewer", "sender_name": "Viewer",
                   "at_ms": 1_792_424_183_000],
        ]
        #expect(MentionPush.alert(in: info) == MentionPush.Alert(id: "push-1", room: "bigstreamer"))
    }

    @Test func leavesEverythingElseAlone() {
        #expect(MentionPush.alert(in: ["sn": ["type": "live", "login": "alice"]]) == nil)
        #expect(MentionPush.alert(in: ["mention_id": "local-1"]) == nil)
        #expect(MentionPush.alert(in: ["sn": ["type": "mention", "mention_id": "", "room": "x"]]) == nil)
        #expect(MentionPush.alert(in: ["sn": ["type": "mention", "mention_id": "a"]]) == nil)
        #expect(LivePush.alert(in: ["sn": ["type": "mention", "mention_id": "a", "room": "x"]]) == nil)
    }
}
