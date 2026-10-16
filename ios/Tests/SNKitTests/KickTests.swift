import Foundation
import Testing
@testable import SNKit

@Suite struct KickRowTests {
    /// A Kick row as the core sends it (Rust `ProviderStream`).
    static let kickJSON = """
    {"provider":"kick","key":"kick:xqc","id":"","user_id":"676","user_login":"xqc","user_name":"xQc",
     "title":"Kick title","viewer_count":4200,"game_id":"15","game_name":"Just Chatting",
     "thumbnail_url":"https://images.kick.com/thumb.webp","started_at":"2026-10-16T10:00:00Z",
     "profile_image_url":"https://files.kick.com/pic.webp","is_live":true,"watch_url":"https://kick.com/xqc"}
    """

    @Test func aKickRowKeepsItsPlatformAndNeverPassesForTwitch() throws {
        let row = try JSONDecoder().decode(TwitchStream.self, from: Data(Self.kickJSON.utf8))
        #expect(row.platform == "kick")
        #expect(!row.isTwitch)
        #expect(row.twitchID == nil, "a Kick id would name an unrelated Twitch channel")
        #expect(row.userID == "676")
        #expect(row.rowKey == "kick:xqc")
        #expect(row.playbackURL == "https://kick.com/xqc")
        #expect(row.channelLink?.absoluteString == "https://kick.com/xqc")
    }

    @Test func aTwitchRowIsTwitchWithNoProvider() {
        let row = TwitchStream(id: "1", userLogin: "XQC", userName: "xQc", title: "", viewerCount: 0, gameName: "",
                               userID: "71092938")
        #expect(row.isTwitch)
        #expect(row.twitchID == "71092938")
        #expect(row.rowKey == "xqc")
        #expect(row.playbackURL == "https://twitch.tv/XQC")
    }

    @Test func twitchAndKickXqcAreTwoRows() throws {
        let kick = try JSONDecoder().decode(TwitchStream.self, from: Data(Self.kickJSON.utf8))
        let twitch = TwitchStream(id: "1", userLogin: "xqc", userName: "xQc", title: "", viewerCount: 0, gameName: "")
        #expect(kick.rowKey != twitch.rowKey)
    }

    @Test func channelIDsLeaveKickRowsOut() throws {
        let kick = try JSONDecoder().decode(TwitchStream.self, from: Data(Self.kickJSON.utf8))
        let twitch = TwitchStream(id: "1", userLogin: "a", userName: "a", title: "", viewerCount: 0, gameName: "",
                                  userID: "9")
        #expect(HomeLists.channelIDs(of: [kick, twitch]) == ["9"])
    }
}

@Suite struct FollowingListTests {
    @Test func theFollowingTabKeepsThePlatformsTheIPhonePlays() throws {
        let json = """
        {"followed_live":[{"id":"t1","user_id":"1","user_login":"a","user_name":"A","title":"","viewer_count":5,
          "game_name":"","thumbnail_url":"","started_at":""}, \(KickRowTests.kickJSON),
          {"provider":"youtube","key":"youtube:v","id":"v","user_id":"UC1","user_login":"v","user_name":"V","title":"",
           "viewer_count":9,"game_id":"","game_name":"","thumbnail_url":"","started_at":"","is_live":true,"watch_url":"w"}],
         "platforms":["twitch","kick","youtube"]}
        """
        let snapshot = try JSONDecoder().decode(HomeLists.self, from: Data("{\"following\":\(json)}".utf8))
        #expect(snapshot.followingLive.map(\.rowKey) == ["a", "kick:xqc"])
        #expect(snapshot.following.followsKick)
    }

    @Test func anOlderCoreFallsBackToTwitchsOwnList() throws {
        let snapshot = try JSONDecoder().decode(HomeLists.self, from: Data("""
        {"followed_live":[{"id":"t1","user_id":"1","user_login":"a","user_name":"A","title":"","viewer_count":5,
          "game_name":"","thumbnail_url":"","started_at":""}]}
        """.utf8))
        #expect(snapshot.followingLive.map(\.userLogin) == ["a"])
        #expect(!snapshot.following.followsKick)
    }
}

@Suite struct KickWordsTests {
    @Test func outcomesReadAsSentences() {
        #expect(KickWords.outcome("signed in, but couldn't read your Kick follows") == "Signed in, but couldn't read your Kick follows")
        #expect(KickWords.outcome("\"Sign-in was cancelled\"") == "Sign-in was cancelled")
        #expect(KickWords.outcome("the user CANCELLED it") == "Sign-in was cancelled")
        #expect(KickWords.outcome("") == "Sign-in was not completed")
        #expect(KickWords.outcome("undefined") == "Sign-in was not completed")
        #expect(KickWords.outcome(String(repeating: "x", count: 161)) == "Sign-in was not completed")
    }

    @Test func theRowSaysWhereYouStand() {
        #expect(KickWords.status(connected: false, follows: 4) == "Not signed in")
        #expect(KickWords.status(connected: true, follows: 0) == "Signed in")
        #expect(KickWords.status(connected: true, follows: 1) == "1 follow")
        #expect(KickWords.status(connected: true, follows: 14) == "14 follows")
        #expect(KickWords.found(1) == "Found 1 Kick channel you follow")
        #expect(KickWords.found(3) == "Found 3 Kick channels you follow")
    }

    @Test func theRosterReadsKickAsConnectedOnlyWithAToken() throws {
        let roster = try JSONDecoder().decode(AccountRoster.self, from: Data("""
        {"main":[{"id":"twitch","status":"connected"},{"id":"kick","status":"expired","name":"me"}],"extras":[]}
        """.utf8))
        #expect(roster.account("kick")?.connected == false)
        #expect(roster.account("twitch")?.connected == true)
    }

    @Test func theLoginPressRunsOnlyOnKicksLoginPage() throws {
        #expect(KickLogin.opensForm(try #require(URL(string: "https://kick.com/login"))))
        #expect(KickLogin.opensForm(try #require(URL(string: "https://www.kick.com/login?x=1"))))
        #expect(!KickLogin.opensForm(try #require(URL(string: "https://id.kick.com/login"))))
        #expect(!KickLogin.opensForm(try #require(URL(string: "https://kick.com/xqc"))))
    }
}
