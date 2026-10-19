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

    @Test func tokensAreLowercaseHex() {
        #expect(LivePush.hex(Data([0x00, 0xAB, 0x10, 0xff])) == "00ab10ff")
    }
}
