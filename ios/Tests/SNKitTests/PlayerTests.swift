import Foundation
import Testing
@testable import SNKit

@Suite struct PlayerTextTests {
    @Test func uptimeReadsHoursAndMinutes() {
        let start = "2026-10-14T10:00:00Z"
        let now = ISO8601DateFormatter().date(from: "2026-10-14T12:14:59Z")!
        #expect(PlayerText.uptime(startedAt: start, now: now) == "2h 14m")
        let soon = ISO8601DateFormatter().date(from: "2026-10-14T10:09:00Z")!
        #expect(PlayerText.uptime(startedAt: start, now: soon) == "9m")
    }

    @Test func uptimeTakesFractionalSecondsAndNeverGoesNegative() {
        let now = ISO8601DateFormatter().date(from: "2026-10-14T10:00:00Z")!
        #expect(PlayerText.uptime(startedAt: "2026-10-14T09:30:00.123Z", now: now) == "29m")
        #expect(PlayerText.uptime(startedAt: "2026-10-14T10:05:00Z", now: now) == "0m")
    }

    @Test func anUnknownOrUnreadableStartShowsNothing() {
        #expect(PlayerText.uptime(startedAt: nil) == "")
        #expect(PlayerText.uptime(startedAt: "") == "")
        #expect(PlayerText.uptime(startedAt: "yesterday") == "")
    }

    @Test func qualityLabelsMatchTheSheet() {
        #expect(PlayerText.qualityLabel("best") == "Auto")
        #expect(PlayerText.qualityLabel("1080p60") == "1080p60")
        #expect(PlayerText.qualityLabel("audio_only") == "Audio only")
        #expect(PlayerText.qualityLabel("worst") == "Lowest")
    }

    @Test func autoLeadsTheSheet() {
        #expect(PlayerText.qualityRows(["1080p60", "720p60", "160p", "best", "audio_only", "worst"])
            == ["best", "1080p60", "720p60", "160p", "audio_only", "worst"])
        #expect(PlayerText.qualityRows(["best", "720p"]) == ["best", "720p"])
    }

    @Test func theTickIsThePickThenTheSavedOneThenAuto() {
        #expect(PlayerText.tickedQuality(picked: "720p60", saved: "480p") == "720p60")
        #expect(PlayerText.tickedQuality(picked: nil, saved: "480p") == "480p")
        #expect(PlayerText.tickedQuality(picked: nil, saved: "") == "best")
        #expect(PlayerText.tickedQuality(picked: nil, saved: nil) == "best")
    }
}

@Suite struct ScreenCapTests {
    @Test func snapsDownToARung() {
        #expect(ScreenCap.height(shortEdgePixels: 1290) == 1080)
        #expect(ScreenCap.height(shortEdgePixels: 1179) == 1080)
        #expect(ScreenCap.height(shortEdgePixels: 1080) == 1080)
        #expect(ScreenCap.height(shortEdgePixels: 1079) == 720)
        #expect(ScreenCap.height(shortEdgePixels: 2400) == 2160)
    }

    @Test func belowTheLadderStaysAtTheBottomAndUnknownIsUncapped() {
        #expect(ScreenCap.height(shortEdgePixels: 200) == 360)
        #expect(ScreenCap.height(shortEdgePixels: 0) == 0)
    }
}

@Suite struct PlayerDecodingTests {
    @Test func startAnswersCarryTheQualityList() throws {
        let json = #"{"url":"http://localhost:1/stream.m3u8","quality":"1080p60","available":["best","1080p60","720p60","audio_only","worst"],"mode":"auth-only"}"#
        let start = try JSONDecoder().decode(StreamStart.self, from: Data(json.utf8))
        #expect(start.available.first == "best" && start.available.count == 5)
        let bare = try JSONDecoder().decode(StreamStart.self, from: Data(#"{"url":"u","quality":"best"}"#.utf8))
        #expect(bare.available.isEmpty)
    }

    @Test func onlyTheViewersSectionIsRead() {
        let viewers = #"{"section":"viewers","login":"jynxzi","viewer_count":41234,"started_at":"2026-10-14T10:00:00Z","title":"t","game_name":"Rainbow Six Siege","at":1}"#
        let parsed = ChannelViewers.parse(viewers)
        #expect(parsed?.viewerCount == 41234 && parsed?.gameName == "Rainbow Six Siege")
        #expect(ChannelViewers.parse(#"{"section":"points","login":"jynxzi","points":null,"at":1}"#) == nil)
    }

    @Test func prefsReadTheQualityAndBackgroundMode() throws {
        let json = #"{"quality":"720p60","video_player":{"background_mode":"audio","muted":false}}"#
        let prefs = try JSONDecoder().decode(PlayerPrefs.self, from: Data(json.utf8))
        #expect(prefs.quality == "720p60" && prefs.keepsAudio)
        let bare = try JSONDecoder().decode(PlayerPrefs.self, from: Data(#"{"quality":"best"}"#.utf8))
        #expect(!bare.keepsAudio)
    }
}

@Suite struct WatchStartTests {
    @Test func aStartCarriesTheCompletedRowAndTheRelay() throws {
        let json = #"{"superseded":false,"row":{"id":"1","user_login":"caster","user_name":"Caster","title":"Live","viewer_count":9,"game_name":"Game","thumbnail_url":"","user_id":"42","started_at":"","profile_image_url":"pic"},"stream":{"url":"http://127.0.0.1:1/stream.m3u8","quality":"best","available":["best"],"entitled":false}}"#
        let start = try JSONDecoder().decode(WatchStart.self, from: Data(json.utf8))
        #expect(!start.superseded)
        #expect(start.row?.userID == "42" && start.row?.profileImageURL == "pic")
        #expect(start.stream?.quality == "best")
    }

    @Test func aSupersededStartCarriesNothing() throws {
        let start = try JSONDecoder().decode(WatchStart.self, from: Data(#"{"superseded":true}"#.utf8))
        #expect(start.superseded && start.row == nil && start.stream == nil)
    }

    @Test func theSeedUsesTheCoreFieldNames() {
        let row = TwitchStream(id: "1", userLogin: "caster", userName: "Caster", title: "t", viewerCount: 3,
                               gameName: "g", userID: "42", startedAt: "2026-10-16T10:00:00Z")
        let seed = row.seed
        #expect(seed["user_id"] as? String == "42")
        #expect(seed["started_at"] as? String == "2026-10-16T10:00:00Z")
        #expect(seed["viewer_count"] as? Int == 3)
    }

    @Test func theFakeCoreAnswersAStart() async throws {
        let core = FakeCore()
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "t",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        let start = try await core.invoke("watch_start", args: ["login": "Caster", "seed": ["user_id": "42"]],
                                          as: WatchStart.self)
        #expect(start.row?.userLogin == "caster" && start.row?.userID == "42")
        #expect(start.stream?.url == FakeCore.testStreamURL)
    }
}

@Suite struct LandscapeChatTests {
    @Test func theSideAndModeComeFromTheSettings() {
        let auto = LandscapeChatLayout(layout: "overlay", side: "auto", width: 340, opacity: 55)
        #expect(!auto.beside && !auto.leading)
        let left = LandscapeChatLayout(layout: "beside", side: "left", width: 340, opacity: 55)
        #expect(left.beside && left.leading)
    }

    @Test func theWidthStaysBetween240AndHalfTheScreen() {
        let chat = LandscapeChatLayout(layout: "overlay", side: "auto", width: 600, opacity: 55)
        #expect(chat.width(screenWidth: 932) == 600)
        #expect(LandscapeChatLayout(layout: "beside", side: "auto", width: 900, opacity: 55)
            .width(screenWidth: 812) == 572)
        #expect(chat.width(screenWidth: 932, live: 100) == 240)
        #expect(chat.width(screenWidth: 932, live: 301.4) == 301)
    }

    @Test func pullingTowardThePictureWidensTheColumn() {
        let right = LandscapeChatLayout(layout: "overlay", side: "right", width: 340, opacity: 55)
        #expect(right.dragged(from: 340, by: -40, screenWidth: 932) == 380)
        let left = LandscapeChatLayout(layout: "overlay", side: "left", width: 340, opacity: 55)
        #expect(left.dragged(from: 340, by: -40, screenWidth: 932) == 300)
    }

    @Test func theBackgroundIsClampedToAPercent() {
        let side = LandscapeChatLayout(layout: "beside", side: "right", width: 340, opacity: 55)
        #expect(side.textInset(edgeInset: 59, cutoutOnChatSide: true) == 59)
        #expect(side.textInset(edgeInset: 59, cutoutOnChatSide: false) == 24)
        #expect(side.textInset(edgeInset: 44, cutoutOnChatSide: false) == 18)
        #expect(side.textInset(edgeInset: 20, cutoutOnChatSide: false) == 12)
        #expect(side.textInset(edgeInset: 8, cutoutOnChatSide: false) == 8)
        #expect(side.textInset(edgeInset: 0, cutoutOnChatSide: false) == 0)
        #expect(LandscapeChatLayout(layout: "overlay", side: "", width: 340, opacity: 140).opacity == 100)
        #expect(LandscapeChatLayout(layout: "overlay", side: "", width: 340, opacity: 0).opacity == 0)
    }
}

@Suite struct DialTests {
    @Test func upRaisesOverEightyPercentOfThePicture() {
        #expect(Dial.level(start: 0.5, dy: -200, height: 400) > 0.99)
        #expect(Dial.level(start: 0.5, dy: 160, height: 400) == 0)
        // A short picture still needs 140 points for the full range.
        #expect(abs(Dial.level(start: 0, dy: -70, height: 100) - 0.5) < 0.0001)
    }

    @Test func hundredthsAndTenthsAreWhereTheyChange() {
        #expect(Dial.hundredth(0.504) == 50)
        #expect(Dial.tenth(0.29) == 2)
        #expect(Dial.tenth(0.3) == 3)
    }
}

@Suite struct TogetherTests {
    @Test func theCollabPushNamesTheGroupOrItsEnd() {
        let json = #"{"section":"collab","login":"Caster","at":1,"collab":{"shared_viewers":1500,"label":"1,500 watching together with B","members":[{"user_id":"1","login":"caster","display_name":"Caster","viewer_count":900,"is_leader":true,"is_self":true},{"user_id":"2","login":"b","display_name":"B","viewer_count":600,"is_leader":false,"is_self":false}]}}"#
        let group = ChannelGroup.collabPush(json, login: "caster")
        #expect(group??.members.first?.isLeader == true)
        #expect(group??.members.last?.viewerCount == 600)
        #expect(group??.summary == "1,500 viewers combined")
        let ended = ChannelGroup.collabPush(#"{"section":"collab","login":"caster","at":2,"collab":null}"#, login: "caster")
        #expect(ended != nil && ended! == nil)
        #expect(ChannelGroup.collabPush(#"{"section":"viewers","login":"caster","at":1}"#, login: "caster") == nil)
        #expect(ChannelGroup.collabPush(json, login: "someone") == nil)
    }

    @Test func theCollabPushCarriesASharedChatOutsideAGroup() {
        let json = #"{"section":"collab","login":"caster","at":3,"collab":null,"shared_chat":{"label":"Sharing chat with Myth","members":[{"user_id":"1","login":"caster","display_name":"Caster","viewer_count":900,"is_leader":true,"is_self":true},{"user_id":"2","login":"myth","display_name":"Myth","avatar_url":"https://x/y.png","viewer_count":5000,"is_leader":false,"is_self":false}]}}"#
        let group = ChannelGroup.collabPush(json, login: "caster")
        #expect(group??.members.map(\.login) == ["caster", "myth"])
        #expect(group??.summary == "One chat across 2 channels")
    }

    @Test func aSharedChatSaysHowManyChannelsShareIt() {
        let group = ChannelGroup(members: [.init(userID: "1", login: "a", displayName: "A"),
                                           .init(userID: "2", login: "b", displayName: "B")], label: "")
        #expect(group.summary == "One chat across 2 channels")
    }
}

@Suite struct OfflineRoomTests {
    @Test func theCoresRoomDecodes() throws {
        let json = #"{"superseded":false,"room":{"user_id":"1","login":"caster","display_name":"Caster","avatar_url":"a","offline_image_url":null,"live":null,"last_live_at":"2026-10-16T08:00:00Z","last_title":"Last time","last_category":"Chess","latest_vod":{"id":"99","title":"VOD","created_at":"2026-10-16T05:00:00Z","length_seconds":3600,"thumbnail_url":"t","category":"Chess"},"next_stream":{"start_at":"2026-10-17T19:00:00Z","end_at":null,"title":"Next","category":null},"sad_emote_url":"s","happy_emote_url":"h","shiny":false,"chatters":12},"row":{"id":"","user_login":"caster","user_name":"Caster","title":"Last time","viewer_count":0,"game_name":"Chess","thumbnail_url":"","is_live":false}}"#
        let opened = try JSONDecoder().decode(RoomOpen.self, from: Data(json.utf8))
        #expect(opened.room?.latestVod?.id == "99" && opened.room?.chatters == 12)
        #expect(opened.row?.isOffline == true && opened.live == nil)
    }

    @Test func theRoomSaysWhenAndHowMany() {
        let now = ISO8601DateFormatter().date(from: "2026-10-16T11:00:00Z")!
        #expect(RoomText.lastLive("2026-10-16T08:00:00Z", now: now) == "Last live 3 hours ago")
        #expect(RoomText.ago(30) == "just now")
        #expect(RoomText.ago(60) == "1 minute ago")
        #expect(RoomText.ago(3 * 86_400) == "3 days ago")
        #expect(RoomText.lastLive(nil) == nil)
        #expect(RoomText.waiting(12) == "12 waiting in chat")
        #expect(RoomText.waiting(0) == nil)
        #expect(RoomText.next(nil) == nil)
        #expect(RoomText.next(.init(startAt: "2026-10-17T19:00:00Z"), now: now)?.hasPrefix("Next stream") == true)
    }
}

@Suite struct StreamUpdateTests {
    @Test func aCategoryChangeCarriesTheGameID() {
        let update = ChannelViewers.streamUpdate(#"{"title":"t","game_name":"Valorant","game_id":"516575"}"#)
        #expect(update?.gameName == "Valorant")
        #expect(update?.gameID == "516575")
        #expect(ChannelViewers.streamUpdate(#"{"title":"t","game_name":"Just Chatting","game_id":""}"#)?.gameID == nil)
    }
}
