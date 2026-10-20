import Foundation
import Testing
@testable import SNKit

@Suite struct ComposerTextTests {
    @Test func theLastWordIsTheRunAfterTheLastSpace() {
        #expect(ComposerText.lastWord("hello LU") == "LU")
        #expect(ComposerText.lastWord("hello ") == "")
        #expect(ComposerText.lastWord("") == "")
        #expect(ComposerText.lastWord("@lulu") == "@lulu")
    }

    @Test func everyWordAsksAndTheCoreKeepsShortOnesExact() {
        // "0" asks too: the core offers only an emote named exactly 0 (Rust `typing`).
        #expect(ComposerText.wantsSuggestions("0"))
        #expect(ComposerText.wantsSuggestions(":0"))
        #expect(ComposerText.wantsSuggestions("00"))
        #expect(!ComposerText.wantsSuggestions("LU "))
        #expect(!ComposerText.wantsSuggestions(""))
    }

    @Test func theFakeCoreKeepsOneCharacterWordsExactWhileTyping() throws {
        let emotes = Data(#"{"7tv":[{"id":"a","name":"0","url":"u"},{"id":"b","name":"00","url":"u"},{"id":"c","name":"0head","url":"u"}]}"#.utf8)
        let names = { (query: String, typing: Bool) -> [String] in
            let answer = FakeCore.emoteMatches(emotes, args: ["query": query, "profile": "cycle", "typing": typing])
            return (answer["rows"] as? [[String: Any]] ?? []).compactMap { $0["name"] as? String }
        }
        #expect(names("0", true) == ["0"])
        #expect(names(":0", true) == ["0"])
        #expect(names("00", true) == ["00"])
        #expect(names("0", false) == ["0", "00", "0head"])
    }

    @Test func anInsertIsSpacedFromWhatCameBefore() {
        #expect(ComposerText.inserting("Kappa", into: "") == "Kappa ")
        #expect(ComposerText.inserting("Kappa", into: "hi") == "hi Kappa ")
        #expect(ComposerText.inserting("Kappa", into: "hi ") == "hi Kappa ")
    }

    @Test func aCompletionReplacesTheLastWord() {
        #expect(ComposerText.completing("so LU", with: "LULW") == "so LULW ")
        #expect(ComposerText.completing("@lu", with: "@Lulu") == "@Lulu ")
    }

    @Test func aCommandPickReplacesTheWholeDraft() {
        #expect(ComposerText.command("ban") == "/ban ")
    }

    @Test func sendsRouteSlashTextToTheDispatcher() {
        #expect(ComposerRoute.of("   ") == .none)
        #expect(ComposerRoute.of(" hi chat ") == .chat("hi chat"))
        #expect(ComposerRoute.of("/ban someone ") == .slash("/ban someone"))
    }

    @Test func aCommandWhoseDispatchFailedNeverLeavesAsChat() {
        // The dispatcher threw: /mods must stay in the draft, never reach chat as text.
        #expect(SlashAfterDispatch.of(text: "/mods", handled: false, dispatchFailed: true) == .keepDraft)
        #expect(SlashAfterDispatch.of(text: "/vips", handled: false, dispatchFailed: true) == .keepDraft)
        // The core ran it: done, whatever it says to send.
        #expect(SlashAfterDispatch.of(text: "/mods", handled: true, dispatchFailed: false) == .done)
        // The core does not run it (Twitch runs /me): as typed.
        #expect(SlashAfterDispatch.of(text: "/me waves", handled: false, dispatchFailed: false) == .sendTyped)
        // Plain text whose custom-command check failed is still the viewer's chat line.
        #expect(SlashAfterDispatch.of(text: "hi chat", handled: false, dispatchFailed: true) == .sendTyped)
    }
}

@Suite struct ComposerDecodingTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(T.self, from: Data(json.utf8))
    }

    @Test func chatterRowsComeAfterTheEmotes() throws {
        let result = try decode(EmoteMatchResult.self, #"""
        {"rows":[{"id":"1","name":"LULW","url":"https://x/1","provider":"bttv","source_label":"BetterTTV"},
                 {"id":"9","name":"ルナ","insert_text":"lunar","url":"","provider":"user","source_label":"Chatter","login":"lunar"}],
         "total":1,"ready":true}
        """#)
        #expect(result.rows.map(\.isChatter) == [false, true])
        #expect(result.rows[1].text == "lunar", "a localized name inserts the login")
        #expect(result.rows[0].text == "LULW")
        #expect(result.rows[0].key != result.rows[1].key)
    }

    @Test func aDroppedMessageSaysWhy() throws {
        let dropped = try decode(SendResult.self, #"{"is_sent":false,"drop_reason":"msg_duplicate"}"#)
        #expect(dropped.dropLine == "Your message was not sent: msg_duplicate")
        let refused = try decode(SendResult.self, #"{"is_sent":false}"#)
        #expect(refused.dropLine == "Your message was not sent: the room refused it")
        let sent = try decode(SendResult.self, #"{"is_sent":true,"message_id":"abc"}"#)
        #expect(sent.dropLine == nil)
    }

    @Test func slashOutcomesDecodeWithDefaults() throws {
        let banned = try decode(SlashOutcome.self, #"""
        {"handled":true,"send":null,"clear_draft":true,"toasts":[{"text":"Banned bob","kind":"success"}],"notices":[],"ui":null}
        """#)
        #expect(banned.handled && banned.clearDraft)
        #expect(banned.toasts == [.init(text: "Banned bob", kind: "success")])
        let passthrough = try decode(SlashOutcome.self, #"{"handled":false}"#)
        #expect(!passthrough.handled && !passthrough.clearDraft)
        let card = try decode(SlashOutcome.self, #"{"handled":true,"ui":{"kind":"open_user_card","login":"bob"}}"#)
        #expect(card.ui == .init(kind: "open_user_card", login: "bob"))
        #expect(card.clearDraft, "a handled command clears unless told otherwise")
    }

    @Test func roomModesCarryTheBlock() throws {
        let modes = try decode(ChatRoomModes.self, #"""
        {"modes":[{"kind":"emote_only","label":"Emote-only chat","compact":"Emote-only","short":"Emote-only",
                   "detail":"Messages can only contain emotes.","applies_to_you":true}],
         "prompt":"Emote-only chat","prompt_short":"Emote-only","standing":null,"slow_seconds":0,
         "blocked":"emote_only","reason":"Emote-only: pick an emote"}
        """#)
        #expect(modes.blocked == "emote_only")
        #expect(modes.reason == "Emote-only: pick an emote")
        let open = try decode(ChatRoomModes.self, #"{"modes":[],"slow_seconds":0}"#)
        #expect(open.blocked == nil)
    }

    @Test func pickerAnswersDecodeWithTheirFlags() throws {
        let answer = try decode(PickerAnswer.self, #"""
        {"ready":true,"tab":"7tv",
         "tabs":[{"id":"favorites","count":1},{"id":"twitch","count":3},{"id":"7tv","count":2}],
         "sections":[{"key":"personal","title":"Personal","columns":7,"emotes":[
            {"id":"mine","name":"buh","url":"https://cdn.7tv.app/emote/mine/1x.avif","provider":"7tv","favorite":true}]},
           {"key":"wide","title":"Wide","columns":4,"emotes":[
            {"id":"chan","name":"buh","url":"u","provider":"7tv","is_zero_width":true,"width":70,
             "replaced_by":{"id":"mine","url":"https://cdn.7tv.app/emote/mine/1x.avif"}},
            {"id":"fx","name":"ffzSpin","url":"u","provider":"ffz","modifier_flags":262144,"locked":true}]}]}
        """#)
        #expect(answer.count("twitch") == 3)
        #expect(answer.count("bttv") == 0)
        #expect(answer.sections.map(\.columns) == [7, 4])
        let mine = answer.sections[0].emotes[0]
        #expect(mine.favorite && !mine.locked && mine.replacedBy == nil)
        let chan = answer.sections[1].emotes[0]
        #expect(chan.isZeroWidth && chan.replacedBy?.id == "mine")
        #expect(chan.detailLines == ["Zero-Width", "Your personal buh shows instead"])
        let fx = answer.sections[1].emotes[1]
        #expect(fx.isModifier && fx.locked)
        #expect(fx.detailLines == ["Modifier - applies to the next emote", "FFZ subscriber effect - locked"])
    }

    @Test func pickerTabsPutFavoritesAndEmojiFirst() {
        #expect(PickerTabs.order(coreTabs: ["favorites", "twitch", "bttv", "7tv", "ffz"], gif: false)
            == ["favorites", "emoji", "twitch", "bttv", "7tv", "ffz"])
        #expect(PickerTabs.defaultTab(provider: "twitch") == "twitch")
        #expect(PickerTabs.defaultTab(provider: "youtube") == "emoji")
    }

    @Test func searchingLeadsTheTabsWithAllAndOpensOnIt() {
        let tabs = PickerTabs.order(coreTabs: ["favorites", "twitch", "bttv", "7tv", "ffz"], gif: false)
        #expect(PickerTabs.searchOrder(tabs) == ["all", "favorites", "emoji", "twitch", "bttv", "7tv", "ffz"])
        #expect(PickerTabs.title("all") == "All")
        // A search typed on any tab looks everywhere; from Emoji it stays on the emoji.
        #expect(PickerTabs.searchScope(from: "7tv") == "all")
        #expect(PickerTabs.searchScope(from: "favorites") == "all")
        #expect(PickerTabs.searchScope(from: "emoji") == "emoji")
    }

    @Test func tabNumbersFitTheirSlot() {
        #expect([0, 7, 999, 1000, 1999, 12_345, 999_999, 1_000_000].map(PickerTabs.countLabel)
            == ["0", "7", "999", "1K", "1K", "12K", "999K", "1M"])
    }

    /// The core's picker-mode search (`match_emote_tokens`), as its rows and counts come.
    private let searchJSON = #"""
    {"rows":[
       {"id":"t1","name":"catKISS","url":"u","provider":"twitch","source_label":"Twitch Global","source_detail":"Global"},
       {"id":"s1","name":"catJAM","url":"u","provider":"7tv","local_path":"/cache/s1.webp","width":32,"favorite":true},
       {"id":"s2","name":"catWIDE","url":"u","provider":"7tv","width":70},
       {"id":"b1","name":"catHUGE","url":"u","provider":"bttv","width":200},
       {"id":"f1","name":"catSub","url":"u","provider":"ffz","locked":true}],
     "total":9,"ready":true,
     "counts":{"twitch":1,"7tv":4,"bttv":1,"ffz":3,"favorites":2}}
    """#

    @Test func searchRowsDecodeAsPickerCells() throws {
        let search = try decode(PickerSearch.self, searchJSON)
        #expect(search.rows.map(\.name) == ["catKISS", "catJAM", "catWIDE", "catHUGE", "catSub"])
        let jam = search.rows[1]
        #expect(jam.favorite && jam.localPath == "/cache/s1.webp")
        #expect(search.rows[4].locked)
        #expect(search.rows.map(\.span) == [1, 1, 2, 3, 1])
        // Rows from an older core without counts still decode.
        let bare = try decode(PickerSearch.self, #"{"rows":[],"total":0,"ready":false}"#)
        #expect(!bare.ready && bare.counts.isEmpty)
    }

    @Test func theTabsNarrowTheResultsAndCountTheirHits() throws {
        let search = try decode(PickerSearch.self, searchJSON)
        #expect(search.cells(in: "all").count == 5)
        #expect(search.cells(in: "7tv").map(\.name) == ["catJAM", "catWIDE"], "rank order kept")
        #expect(search.cells(in: "favorites").map(\.name) == ["catJAM"])
        #expect(search.cells(in: "emoji").isEmpty)
        #expect(search.count("all", emojiHits: 3) == 12, "All counts the emoji too")
        #expect(search.count("7tv", emojiHits: 3) == 4)
        #expect(search.count("favorites", emojiHits: 3) == 2, "the core's count, before the cap")
        #expect(search.count("kick", emojiHits: 3) == 0)
        #expect(search.count("emoji", emojiHits: 3) == 3)
        // More matched than the rows shown: the results say they are the best of that many.
        #expect(search.cappedTotal("all") == 9)
        #expect(search.cappedTotal("ffz") == 3)
        #expect(search.cappedTotal("twitch") == nil)
        #expect(search.cappedTotal("emoji") == nil)
    }

    @Test func theSearchGridPacksWideEmotesInRankOrder() throws {
        func cell(_ name: String, _ width: Int) throws -> PickerCell {
            try decode(PickerCell.self, #"{"id":"\#(name)","name":"\#(name)","url":"u","provider":"7tv","width":\#(width)}"#)
        }
        // 1+1+3+1 = 6, the 2-wide does not fit and opens a line, the narrow after it fills the
        // first line's last cell.
        let cells = try [cell("a", 32), cell("b", 32), cell("huge", 120), cell("c", 32), cell("wide", 70),
                         cell("d", 32), cell("e", 32)]
        let lines = SearchGrid.lines(cells)
        #expect(lines.map { $0.map(\.name) } == [["a", "b", "huge", "c", "d"], ["wide", "e"]])
        #expect(lines.allSatisfy { $0.map(\.span).reduce(0, +) <= SearchGrid.columns })
        #expect(SearchGrid.lines([]).isEmpty)
    }
}

@Suite struct EmojiCatalogTests {
    @Test func theBundledCatalogLoads() throws {
        let catalog = try #require(EmojiCatalog.bundled())
        #expect(catalog.categories.count >= 8)
        #expect(catalog.categories[0].emoji.contains("😀"))
    }

    @Test func searchFindsKeywordsNotOnlyCategoryNames() throws {
        let catalog = try #require(EmojiCatalog.bundled())
        // The Android picker drew a blank pane for a keyword-only search.
        let hearts = catalog.filtered("heart")
        #expect(!hearts.isEmpty)
        #expect(hearts.flatMap(\.emoji).contains("😍"))
        #expect(catalog.filtered("zzzz-no-such-emoji").isEmpty)
        #expect(catalog.filtered("").count == catalog.categories.count)
    }

    @Test func searchMatchesAreOneListInCatalogOrder() throws {
        let catalog = try #require(EmojiCatalog.bundled())
        let hearts = catalog.matches("heart")
        #expect(hearts == catalog.filtered("heart").flatMap(\.emoji))
        #expect(hearts.contains("😍"))
        #expect(catalog.matches("zzzz-no-such-emoji").isEmpty)
    }
}

@Suite struct FakeComposerTests {
    @Test func theFakeDispatcherNeverSends() async throws {
        let fake = FakeCore()
        try fake.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "t",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        let json = try await fake.invoke("run_slash_command", args: ["text": "/ban bob", "channel": "x", "broadcasterId": "1"])
        let outcome = try JSONDecoder().decode(SlashOutcome.self, from: Data(json.utf8))
        #expect(outcome.toasts.first?.text == "Banned bob")
        #expect(fake.sentMessages.isEmpty)
        let me = try JSONDecoder().decode(SlashOutcome.self, from: Data(try await fake.invoke(
            "run_slash_command", args: ["text": "/me waves"]).utf8))
        #expect(!me.handled)
    }

    @Test func theFakePickerServesTheFixtureSet() async throws {
        let fake = FakeCore()
        try fake.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "t",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        fake.chatFixture = try #require(ChatFixture.bundled())
        let answer = try await fake.invoke("emote_picker", args: ["tab": "twitch", "query": ""], as: PickerAnswer.self)
        #expect(answer.ready)
        #expect(answer.tab == "twitch")
        #expect(answer.tabs.first?.id == "favorites")
    }

    @Test func theFakeSearchCountsEachSource() async throws {
        let fake = FakeCore()
        try fake.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "t",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        fake.chatFixture = try #require(ChatFixture.bundled())
        let all = try await fake.invoke("match_emote_tokens", args: [
            "query": "", "profile": "search", "limit": 1, "picker": true,
        ], as: PickerSearch.self)
        #expect(all.rows.count == 1)
        #expect(all.counts.values.reduce(0, +) == all.total)
    }
}
