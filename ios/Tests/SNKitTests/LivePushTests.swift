import Foundation
import Testing
@testable import SNKit

@Suite struct LivePushTests {
    private let live = AlertType(id: "live", title: "Live channels", detail: "", setting: "live_notifications.show_live_notifications",
                                 channel: "live-channels", setup: true)
    private let mentions = AlertType(id: "mentions", title: "Mentions and replies", detail: "",
                                     setting: "live_notifications.show_mention_notifications", channel: "mentions", setup: true)

    /// A profile as Apple ships it: signed DER around an XML plist.
    private func profile(aps: String?) -> Data {
        let entitlement = aps.map { "<key>aps-environment</key><string>\($0)</string>" } ?? ""
        let xml = """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0"><dict><key>Name</key><string>dev</string>
        <key>Entitlements</key><dict><key>get-task-allow</key><true/>\(entitlement)</dict></dict></plist>
        """
        return Data([0x30, 0x82, 0x1f, 0x00, 0x06, 0x09]) + Data(xml.utf8) + Data([0xa0, 0x82, 0x00, 0x01])
    }

    @Test func aDevelopmentProfileGetsSandboxTokens() {
        #expect(LivePush.environment(profile: profile(aps: "development")) == "sandbox")
    }

    @Test func distributionBuildsGetProductionTokens() {
        #expect(LivePush.environment(profile: nil) == "production", "App Store and TestFlight installs carry no profile")
        #expect(LivePush.environment(profile: profile(aps: "production")) == "production")
        #expect(LivePush.environment(profile: profile(aps: nil)) == "production")
        #expect(LivePush.environment(profile: Data("not a profile".utf8)) == "production")
    }

    @Test func theServersAlertReadsBack() throws {
        let info: [AnyHashable: Any] = [
            "aps": ["alert": ["title": "Alpha is live", "body": "hello"]],
            "sn": ["type": "live", "channel_id": "123", "login": "alpha", "started_at": "2026-10-19T12:00:00Z"],
        ]
        let alert = try #require(LivePush.alert(in: info))
        #expect(alert == LivePush.Alert(channelID: "123", login: "alpha", startedAt: "2026-10-19T12:00:00Z"))
        #expect(LivePush.alert(in: ["mention_id": "m1"]) == nil, "a mention is not a go-live")
        #expect(LivePush.alert(in: ["sn": ["type": "badge", "login": "alpha"]]) == nil)
        #expect(LivePush.alert(in: ["sn": ["type": "live", "login": ""]]) == nil)
    }

    @Test func theChannelPlayingIsMatchedByIdThenLogin() {
        let alert = LivePush.Alert(channelID: "123", login: "Alpha")
        #expect(LivePush.isPlaying(alert, login: "alpha", channelID: "123"))
        #expect(!LivePush.isPlaying(alert, login: "alpha", channelID: "999"), "the id decides when both have one")
        #expect(LivePush.isPlaying(alert, login: "ALPHA", channelID: nil))
        #expect(!LivePush.isPlaying(alert, login: "bravo", channelID: nil))
        #expect(!LivePush.isPlaying(alert, login: nil, channelID: nil), "nothing playing")
        #expect(LivePush.isPlaying(LivePush.Alert(channelID: "", login: "alpha"), login: "alpha", channelID: "123"))
    }

    @Test func aTokenIsAskedForOnlyWhenGoLivesCanArrive() {
        let all: (String) -> Bool = { _ in true }
        #expect(LivePush.wantsToken(types: [live, mentions], allowed: true, isOn: all))
        #expect(!LivePush.wantsToken(types: [live, mentions], allowed: false, isOn: all), "notifications not allowed")
        #expect(!LivePush.wantsToken(types: [mentions], allowed: true, isOn: all), "this build lists no go-live type")
        #expect(!LivePush.wantsToken(types: [live], allowed: true, isOn: { $0 != "live_notifications.enabled" }))
        #expect(!LivePush.wantsToken(types: [live], allowed: true, isOn: { $0 != live.setting }))
    }

    @Test func aPolledAlertReadsBackAsThePushDoes() throws {
        let found = LivePush.Found(channelID: "123", login: "alpha", startedAt: "2026-10-19T18:00:00Z", title: "Alpha is live",
                                   body: "hello")
        let alert = try #require(LivePush.alert(in: LivePush.userInfo(found)))
        #expect(alert == LivePush.Alert(channelID: "123", login: "alpha", startedAt: "2026-10-19T18:00:00Z", polled: true))
        #expect(LivePush.notificationID(channelID: "123") == "live-123", "the push's collapse id")
    }

    @Test func goLivesOnScreenAreHandedToTheCore() throws {
        let push: [AnyHashable: Any] = ["sn": ["type": "live", "channel_id": "123", "login": "alpha",
                                               "started_at": "2026-10-19T18:00:00.123Z"]]
        let date = Date(timeIntervalSince1970: 1_800_000_000)
        let sighting = try #require(LivePush.sighting(identifier: "live-123", userInfo: push, date: date))
        #expect(sighting["id"] as? String == "live-123")
        #expect(sighting["started_at"] as? String == "2026-10-19T18:00:00.123Z")
        #expect(sighting["date"] as? Int == 1_800_000_000)
        #expect(sighting["polled"] as? Bool == false)

        // A push whose identifier is not the collapse id still names its channel.
        let other = try #require(LivePush.sighting(identifier: "8F2C", userInfo: push, date: nil))
        #expect(other["id"] as? String == "live-123")
        #expect(other["date"] == nil)

        // The app's own alert says so; a mention is no go-live.
        let found = LivePush.Found(channelID: "7", login: "bravo", startedAt: "", title: "Bravo is live", body: "")
        let ours = try #require(LivePush.sighting(identifier: "live-7", userInfo: LivePush.userInfo(found), date: nil))
        #expect(ours["polled"] as? Bool == true)
        #expect(ours["started_at"] == nil, "an empty start time is left out")
        #expect(LivePush.sighting(identifier: "m1", userInfo: ["mention_id": "m1"], date: date) == nil)
        #expect(JSONSerialization.isValidJSONObject(["delivered": [sighting, other, ours]]))
    }

    @Test func theCoresCheckDecodes() throws {
        let json = #"{"active":true,"alerts":[{"channel_id":"1","login":"alpha","started_at":"2026-10-19T18:00:00Z","title":"Alpha is live","body":"hi"}]}"#
        let check = try JSONDecoder().decode(LivePush.Check.self, from: Data(json.utf8))
        #expect(check.active)
        #expect(check.alerts == [LivePush.Found(channelID: "1", login: "alpha", startedAt: "2026-10-19T18:00:00Z",
                                                title: "Alpha is live", body: "hi")])
        let off = try JSONDecoder().decode(LivePush.Check.self, from: Data(#"{"active":false,"alerts":[]}"#.utf8))
        #expect(!off.active && off.alerts.isEmpty)
    }

    @Test func theRefreshIsAskedForNoSoonerThanFifteenMinutes() {
        #expect(LivePush.refreshTask == "app.streamnook.live-check")
        #expect(LivePush.refreshInterval == 15 * 60)
    }

    /// iOS refuses to schedule a refresh whose identifier Info.plist does not permit, or without
    /// the `fetch` background mode, and says nothing about it.
    @Test func infoPlistPermitsTheRefresh() throws {
        let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("../../Resources/Info.plist")
        let plist = try #require(try PropertyListSerialization.propertyList(from: Data(contentsOf: url), format: nil) as? [String: Any])
        #expect((plist["BGTaskSchedulerPermittedIdentifiers"] as? [String])?.contains(LivePush.refreshTask) == true)
        #expect((plist["UIBackgroundModes"] as? [String])?.contains("fetch") == true)
        #expect((plist["UIBackgroundModes"] as? [String])?.contains("audio") == true, "background audio stays")
    }

    @Test func tokensAreLowercaseHex() {
        #expect(LivePush.hex(Data([0x00, 0xAB, 0x10, 0xff])) == "00ab10ff")
    }
}
