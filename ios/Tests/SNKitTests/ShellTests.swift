import Foundation
import Testing
@testable import SNKit

@Suite struct WindowShapeTests {
    @Test func sizeClassesBucketByWidth() {
        #expect(WindowShape(width: 402, height: 874).sizeClass == .compact)
        #expect(WindowShape(width: 700, height: 500).sizeClass == .medium)
        #expect(WindowShape(width: 874, height: 402).sizeClass == .expanded)
    }

    @Test func twoPanesNeedExpandedOrMediumOnItsSide() {
        #expect(WindowShape(width: 700, height: 500).twoPanes)
        #expect(!WindowShape(width: 700, height: 900).twoPanes)
        #expect(WindowShape(width: 915, height: 411).twoPanes)
    }

    @Test func aPhoneOnItsSideIsStillAPhone() {
        #expect(!WindowShape(width: 915, height: 411).largeScreen)
        #expect(!WindowShape(width: 412, height: 915).largeScreen)
        #expect(WindowShape(width: 840, height: 757).largeScreen)
        #expect(WindowShape(width: 1280, height: 800).largeScreen)
    }

    @Test func columnsFollowTheTable() {
        let portrait = WindowShape(width: 402, height: 874)
        #expect(portrait.columns(.card) == 1 && portrait.columns(.row) == 1)
        let side = WindowShape(width: 580, height: 400)
        #expect(side.columns(.card) == 1 && side.columns(.row) == 2)
        #expect(WindowShape(width: 700, height: 500).columns(.card) == 2)
        let wide = WindowShape(width: 874, height: 402)
        #expect(wide.columns(.card) == 3 && wide.columns(.row) == 2)
    }

    @Test func theSplitGivesThePlayer62Percent() {
        #expect(WindowShape(width: 1000, height: 400).splitX == 620)
    }
}

@Suite struct ShellPrefsTests {
    @Test func startTabFallsBackToFollowing() {
        #expect(MobileTab.stored("rewards") == .rewards)
        #expect(MobileTab.stored("nope") == .following)
        #expect(MobileTab.stored(nil) == .following)
        #expect(MobileTab.allCases.map(\.title) == ["Following", "Browse", "Rewards", "You"])
    }

    @Test func streamViewIsCardsUnlessList() {
        #expect(StreamView.stored("list") == .list)
        #expect(StreamView.stored("grid") == .cards)
        #expect(StreamView.stored(nil) == .cards)
    }

    @Test func motionDefaultsToFull() {
        #expect(MotionMode(setting: nil) == .full)
        #expect(MotionMode(setting: "reduced").dropsMovement)
        #expect(MotionMode(setting: "off").stopsLoops)
        #expect(!MotionMode(setting: "full").dropsMovement)
    }

    @Test func toastsOnThePhoneAreErrorsOrActions() {
        #expect(ToastKind.error.shows(hasAction: false))
        #expect(!ToastKind.success.shows(hasAction: false))
        #expect(ToastKind.warning.shows(hasAction: true))
        #expect(ToastKind.seconds() == 5)
        #expect(ToastKind.seconds(live: true) == 8)
        #expect(ToastKind.seconds(sticky: true) == nil)
    }
}

@Suite struct ShellDecodingTests {
    @Test func settingsLookReadsAnySettingsFile() throws {
        let full = #"{"theme":"winters-glass","glass_transparency":40,"glass_blur":false,"setup_complete":true,"motion_mode":"reduced","other":{"x":1}}"#
        let look = try JSONDecoder().decode(SettingsLook.self, from: Data(full.utf8))
        #expect(look.glassiness == 40 && look.glassBlur == false && look.setupComplete == true)
        #expect(look.motion == .reduced)
        let empty = try JSONDecoder().decode(SettingsLook.self, from: Data("{}".utf8))
        #expect(empty.glassiness == 100 && empty.motion == .full)
        let wild = try JSONDecoder().decode(SettingsLook.self, from: Data(#"{"glass_transparency":250}"#.utf8))
        #expect(wild.glassiness == 100)
    }

    @Test func homeListsDecodeMissingSectionsEmpty() throws {
        let json = #"{"recommended":[{"id":"1","user_id":"77","user_name":"A","user_login":"a","title":"t","viewer_count":3,"game_name":"g","thumbnail_url":"u","broadcaster_type":"partner"}],"hype_trains":[{"channel_id":"77","is_active":true,"level":4,"is_golden_kappa":true}],"watch_streaks":{"77":3,"78":0}}"#
        let lists = try JSONDecoder().decode(HomeLists.self, from: Data(json.utf8))
        #expect(lists.followedLive.isEmpty)
        #expect(lists.recommended.first?.isPartner == true)
        #expect(lists.hypeTrain(for: "77")?.level == 4)
        #expect(lists.hypeTrain(for: nil) == nil)
        #expect(lists.streak(for: "77") == 3)
        #expect(lists.streak(for: "78") == nil)
    }

    @Test func imageTemplatesFillTheirSize() {
        let preview = TwitchImage.preview("https://x/live_user_a-{width}x{height}.jpg", stamp: 3)
        #expect(preview?.absoluteString == "https://x/live_user_a-640x360.jpg?sn=3")
        #expect(TwitchImage.preview("") == nil)
        #expect(TwitchImage.boxArt("https://x/509658-{width}x{height}.jpg")?.absoluteString == "https://x/509658-285x380.jpg")
        #expect(TwitchImage.boxArt("https://x/509658-52x72.jpg")?.absoluteString == "https://x/509658-285x380.jpg")
        #expect(TwitchImage.boxArt(nil) == nil)
    }
}

@Suite struct FakeFixturesTests {
    private func started(signedIn: Bool) throws -> FakeCore {
        let core = FakeCore(fixtures: FakeFixtures(signedIn: signedIn))
        try core.start(dirs: CoreDirs(data: "/d", cache: "/c", logs: "/l"), version: "0.1.0",
                       vaultKey: Data(repeating: 7, count: 32), codecs: ["h264"])
        return core
    }

    @Test func signedOutHasNoAccountAndNoFollows() async throws {
        let core = try started(signedIn: false)
        await #expect(throws: CoreFailure.self) { try await core.invoke("get_user_info", "{}") }
        let lists = try await core.invoke("get_home_snapshot", args: [:], as: HomeLists.self)
        #expect(lists.followedLive.isEmpty && !lists.recommended.isEmpty)
    }

    @Test func signedInHasTheFixtureAccount() async throws {
        let core = try started(signedIn: true)
        let user = try await core.invoke("get_user_info", args: [:], as: TwitchUser.self)
        #expect(user.login == "nookviewer")
        let lists = try await core.invoke("get_home_snapshot", args: [:], as: HomeLists.self)
        #expect(lists.followedLive.count == 6)
        #expect(lists.streak(for: lists.followedLive.first?.userID) == 12)
        #expect(lists.hypeTrain(for: lists.followedLive[1].userID)?.level == 3)
    }

    @Test func settingsPatchesAreReadBack() async throws {
        let core = try started(signedIn: false)
        _ = try await core.invoke("patch_settings", args: ["patch": ["glass_transparency": 0, "motion_mode": "off"]])
        let look = try await core.invoke("load_settings", args: [:], as: SettingsLook.self)
        #expect(look.glassiness == 0 && look.motion == .off && look.setupComplete == true)
    }

    @Test func signingOutForgetsTheAccount() async throws {
        let core = try started(signedIn: true)
        _ = try await core.invoke("sign_out_active_twitch_account", "{}")
        await #expect(throws: CoreFailure.self) { try await core.invoke("get_user_info", "{}") }
    }
}
