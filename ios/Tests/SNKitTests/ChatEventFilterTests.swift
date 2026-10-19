import Foundation
import Testing
@testable import SNKit

@Suite struct ChatEventFilterTests {
    private func row(_ id: String, event: String?) -> ChatSocketFrame {
        var message = ChatMessage(id: id, userID: "1", username: "u1", content: "hi")
        var view = ChatRowView(nameColor: "#fff")
        view.event = event
        message.metadata.view = view
        return .row(message)
    }

    @Test func hiddenKindsNeverJoinTheRoom() {
        var buffer = ChatRoomBuffer(cap: 100)
        buffer.setHiddenEvents(["twitch:subscription", "kick:follow"])
        let change = buffer.apply([
            row("a", event: nil),
            row("b", event: "twitch:subscription"),
            row("c", event: "twitch:gift"),
            row("d", event: "kick:follow"),
        ])
        #expect(change.appended.map(\.id) == ["a", "c"])
        #expect(buffer.rows.map(\.id) == ["a", "c"])
    }

    @Test func turningAKindOffDropsTheRowsHeldAndOnShowsTheNextOnes() {
        var buffer = ChatRoomBuffer(cap: 100)
        _ = buffer.apply([row("a", event: "twitch:raid"), row("b", event: nil), row("c", event: "twitch:raid")])
        let dropped = buffer.setHiddenEvents(["twitch:raid"])
        #expect(dropped)
        #expect(buffer.rows.map(\.id) == ["b"])
        #expect(buffer.row("b")?.id == "b")
        #expect(buffer.row("a") == nil)
        // The same set again changes nothing.
        let again = buffer.setHiddenEvents(["twitch:raid"])
        #expect(!again)
        let cleared = buffer.setHiddenEvents([])
        #expect(!cleared)
        _ = buffer.apply([row("d", event: "twitch:raid")])
        #expect(buffer.rows.map(\.id) == ["b", "d"])
    }

    @Test func outageBackfillSkipsHiddenKinds() {
        var buffer = ChatRoomBuffer(cap: 100)
        buffer.setHiddenEvents(["twitch:cheer"])
        var cheer = ChatMessage(id: "x", userID: "1", username: "u1", content: "Cheer100")
        var view = ChatRowView(nameColor: "#fff")
        view.event = "twitch:cheer"
        cheer.metadata.view = view
        let plain = ChatMessage(id: "y", userID: "1", username: "u1", content: "hi")
        let added = buffer.insertInTimeOrder([cheer, plain])
        #expect(added)
        #expect(buffer.rows.map(\.id) == ["y"])
    }

    @Test func theRowViewDecodesItsEventKey() throws {
        let view = try JSONDecoder().decode(ChatRowView.self, from: Data(##"{"name_color":"#fff","event":"kick:gift"}"##.utf8))
        #expect(view.event == "kick:gift")
        let plain = try JSONDecoder().decode(ChatRowView.self, from: Data(##"{"name_color":"#fff"}"##.utf8))
        #expect(plain.event == nil)
    }

    @Test func theStyleReadsTheHiddenKinds() {
        let json = #"{"chat_events":{"hidden_provider_events":["twitch:subscription","kick:raid"]}}"#
        let style = ChatStyle(settingsJSON: Data(json.utf8))
        #expect(style.hiddenEvents == ["twitch:subscription", "kick:raid"])
        #expect(ChatStyle().hiddenEvents.isEmpty)
    }

    @Test func theSettingsChipsWriteTheSharedKeysForWatchablePlatformsOnly() {
        let lines = ChatEventKinds.platforms(watchable: ["twitch", "kick"])
        #expect(lines.map(\.id) == ["twitch", "kick"])
        let keys = lines.flatMap { $0.kinds.map(\.key) }
        #expect(keys.contains("twitch:subscription"))
        #expect(keys.contains("twitch:cheer"))
        #expect(keys.contains("twitch:redemption"))
        #expect(keys.contains("kick:follow"))
        #expect(!keys.contains { $0.hasSuffix("s") && !$0.hasSuffix("ss") }, "no plural keys")
        #expect(ChatEventKinds.platforms(watchable: ["twitch", "kick", "youtube", "tiktok"]).count == 4)
        #expect(ChatEventKinds.platforms(watchable: []).isEmpty)
    }

    @Test func theRosterSaysWhichPlatformsThisBuildWatches() throws {
        let roster = try JSONDecoder().decode(AccountRoster.self, from: Data("""
        {"main":[{"id":"twitch","status":"connected","can_watch":true},{"id":"youtube","status":"not_connected","can_watch":false},
        {"id":"kick","status":"not_connected","can_watch":true},{"id":"tiktok","status":"not_connected"}],"extras":[]}
        """.utf8))
        #expect(roster.watchable == ["twitch", "kick"])
    }
}

@Suite struct WalletBalanceStyleTests {
    private func style(_ input: String) -> ChatStyle {
        ChatStyle(settingsJSON: Data(#"{"chat_input":\#(input)}"#.utf8))
    }

    @Test func theBalanceShowsBesideTheIconOnlyForAlways() {
        #expect(style(#"{"show_points_balance_inline":true}"#).walletBalanceShown)
        // A saved desktop On hover (neither key set) reads as Hidden on a touch screen.
        #expect(!style("{}").walletBalanceShown)
        #expect(!style(#"{"hide_points_balance":true}"#).walletBalanceShown)
        #expect(!style(#"{"show_points_balance_inline":true,"hide_points_balance":true}"#).walletBalanceShown)
    }

    @Test func showGainsBonusChestOnlyFloatsJustTheChest() {
        let chest = WalletGain(seq: 2, amount: 50, reason: "from the bonus chest", chest: true)
        let watching = WalletGain(seq: 3, amount: 10, reason: "for watching", chest: false)
        let chestOnly = style(#"{"wallet_gains":"chest"}"#)
        #expect(chestOnly.walletGainsChestOnly)
        #expect(chest.floats(chestOnly: chestOnly.walletGainsChestOnly))
        #expect(!watching.floats(chestOnly: chestOnly.walletGainsChestOnly))
        // Every gain is the default, and what any other saved value reads as.
        #expect(!style("{}").walletGainsChestOnly)
        #expect(!style(#"{"wallet_gains":"all"}"#).walletGainsChestOnly)
        #expect(watching.floats(chestOnly: false))
    }
}
