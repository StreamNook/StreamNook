import Foundation
import Testing
@testable import SNKit

@Suite struct ChatRoomsTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(type, from: Data(json.utf8))
    }

    @Test func aTabChangeDecodesWithItsJoinsAndLeaves() throws {
        let change = try decode(ChatTabsState.self, """
        {"tabs":[{"channel":"xqc","provider":"twitch","login":"xqc","channel_id":"71092938","label":"xQc","avatar":null,
          "pinned_to_stream":true},{"channel":"kick:joe","provider":"kick","login":"joe","channel_id":null,"label":"joe",
          "avatar":null,"pinned_to_stream":false}],"active_channel":"kick:joe","mixed_platforms":true,"revision":7,
         "acquire":[{"channel":"kick:joe","provider":"kick","login":"joe","channel_id":null,"label":"joe","avatar":null,
          "pinned_to_stream":false}],"release":[]}
        """)
        #expect(change.tabs.map(\.channel) == ["xqc", "kick:joe"])
        #expect(change.tabs[0].channelID == "71092938" && change.tabs[0].pinnedToStream)
        #expect(change.active?.login == "joe")
        #expect(change.mixedPlatforms && change.revision == 7)
        #expect(change.acquire.map(\.channel) == ["kick:joe"] && change.release.isEmpty)
    }

    @Test func aPlainViewDecodesWithoutEffects() throws {
        let view = try decode(ChatTabsState.self, #"{"tabs":[],"active_channel":null,"mixed_platforms":false,"revision":2}"#)
        #expect(view.tabs.isEmpty && view.acquire.isEmpty && view.active == nil && view.revision == 2)
    }

    @Test func roomRowsSayWhatTheyAre() throws {
        let rows = try decode([ChatRoomRow].self, """
        [{"provider":"twitch","key":"a","user_login":"a","user_id":"1","user_name":"A","profile_image_url":null,
          "is_live":true,"viewer_count":12,"game_name":"","pasted":false},
         {"provider":"twitch","key":"b","user_login":"b","user_id":"2","user_name":"B","profile_image_url":null,
          "is_live":false,"viewer_count":0,"game_name":"Chess","pasted":false},
         {"provider":"kick","key":"kick:c","user_login":"c","user_id":"","user_name":"c","profile_image_url":null,
          "is_live":false,"viewer_count":0,"game_name":"","pasted":true}]
        """)
        #expect(rows.map(\.subline) == ["Live", "Offline", "Open this Kick chat"])
        let followed = TwitchStream(id: "s", userLogin: "Foo", userName: "Foo", title: "", viewerCount: 9, gameName: "Art",
                                    thumbnailURL: "", userID: "5")
        let row = ChatRoomRow(followed: followed)
        #expect(row.key == "foo" && row.isLive && row.subline == "Art" && row.userID == "5")
    }

    @Test func pinsCarryTheirSenderView() throws {
        let update = try #require(PinnedUpdate.parse("""
        {"section":"pinned","login":"xqc","at":1,"pinned":[{"id":"p1","message_id":"m1","message_text":"hello",
          "sender_id":"9","sender_name":"Mod","sender_shown_name":"Mod","sender_color":"#1E90FF",
          "sender_view":{"name_color":"#1e90ff","badges":[{"url":"https://b/mod","title":"Moderator","provider":"twitch"}],
          "role_badge":{"url":"https://b/mod","title":"Moderator","provider":"twitch"},"paint":null}}]}
        """))
        #expect(update.login == "xqc")
        #expect(update.pinned.first?.text == "hello")
        #expect(update.pinned.first?.senderView?.roleBadge?.url == "https://b/mod")
        #expect(PinnedUpdate.parse(#"{"section":"viewers","login":"xqc","viewer_count":3,"at":1}"#) == nil)
    }

    @Test func streakAndResubTitlesMatchThePhone() throws {
        #expect(WatchStreakMilestone(milestoneID: "m", streakCount: 5).title == "5 stream watch streak")
        #expect(WatchStreakMilestone(milestoneID: "m", streakCount: 0).title == "Watch streak milestone")
        let resub = try decode(ResubNotice.self, """
        {"id":"t","token":"x","cumulative_tenure_months":1,"streak_tenure_months":1,"months":1,
         "is_gift_subscription":false,"gifter_display_name":null}
        """)
        #expect(resub.title == "1 month subscribed")
        #expect(ResubNotice(id: "t", cumulativeMonths: 14).title == "14 months subscribed")
        #expect(ResubNotice(id: "t", cumulativeMonths: 3, isGift: true, gifter: "Pal").title == "Gift sub from Pal")
    }

    @Test func framesNameTheirRoom() {
        let row = ChatSocketFrame.row(ChatMessage(id: "1", username: "a", content: "x", channel: "#XQC"))
        #expect(row.room == "xqc")
        #expect(ChatSocketFrame.clearChat(channel: "foo", targetUserID: nil, targetUser: nil, duration: nil).room == "foo")
        #expect(ChatSocketFrame.status("HEARTBEAT").room == nil)
        #expect(ChatSocketFrame.roomState(channel: nil, modes: nil).room == nil)
    }
}
