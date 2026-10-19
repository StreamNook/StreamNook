import Foundation
import Testing
@testable import SNKit

@Suite struct AlertTypesTests {
    private func started() throws -> FakeCore {
        let core = FakeCore()
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "t",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        return core
    }

    @Test func theCoreListsWhatThisIPhoneSends() async throws {
        let types = await AlertTypes.load(core: try started())
        #expect(types.map(\.id) == ["mentions"])
        let mentions = try #require(types.first)
        #expect(mentions.title == "Mentions and replies")
        #expect(mentions.setting == "live_notifications.show_mention_notifications")
        #expect(mentions.setup)
    }

    @Test func anUnstartedCoreListsNothing() async {
        #expect(await AlertTypes.load(core: FakeCore()).isEmpty)
    }

    @Test func leavingTheSetupStepWritesEachOfferedTypeAsShown() {
        let live = AlertType(id: "live", title: "Live channels", detail: "", setting: "live_notifications.show_live_notifications",
                             channel: "live-channels", setup: true)
        let mentions = AlertType(id: "mentions", title: "Mentions and replies", detail: "",
                                 setting: "live_notifications.show_mention_notifications", channel: "mentions", setup: true)
        let drops = AlertType(id: "drops", title: "Drops", detail: "", setting: "live_notifications.show_drops_notifications",
                              channel: "drops", setup: false)

        let none = AlertTypes.setupChoices([live, mentions, drops], chosen: [])
        #expect(none["live_notifications.show_live_notifications"] as? Bool == false)
        #expect(none["live_notifications.show_mention_notifications"] as? Bool == false)
        #expect(none["live_notifications.show_drops_notifications"] == nil, "a type setup does not offer stays as it was")
        #expect(none["live_notifications.enabled"] == nil)

        let one = AlertTypes.setupChoices([live, mentions, drops], chosen: ["mentions"])
        #expect(one["live_notifications.show_mention_notifications"] as? Bool == true)
        #expect(one["live_notifications.show_live_notifications"] as? Bool == false)
        #expect(one["live_notifications.enabled"] as? Bool == true)
    }
}
