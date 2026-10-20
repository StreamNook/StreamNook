import Foundation
import Testing
@testable import SNKit

@Suite struct MentionModelTests {
    /// The shape the core's `get_mention_inbox` sends (serde, snake_case, optional fields absent).
    static let pageJSON = """
    {"entries":[
      {"id":"m2","platform":"twitch","room":"xqc","channel_login":"xqc","channel_id":"71092938","sender_id":"5",
       "sender_login":"viewer","sender_name":"Viewer","sender_color":"#FF0000","text":"@you hi","kind":"reply",
       "reply_parent_id":"own","at_ms":2000,"read":false,"removed":false},
      {"id":"m1","platform":"kick","room":"kick:slug","open_key":"kick:Slug","channel_login":"Slug","sender_id":"9","sender_login":"kicker",
       "sender_name":"","text":"hello you","kind":"word","at_ms":1000,"read":true}
    ],"has_more":true,"unread_total":1}
    """

    @Test func decodesTheCoresPage() throws {
        let page = try JSONDecoder().decode(MentionPage.self, from: Data(Self.pageJSON.utf8))
        #expect(page.hasMore && page.unreadTotal == 1 && page.entries.count == 2)
        let reply = page.entries[0]
        #expect(reply.kind == .reply && reply.replyParentID == "own" && reply.channelID == "71092938")
        #expect(reply.kindPhrase == "replied to you")
        let kick = page.entries[1]
        #expect(kick.kind == .word && kick.kindPhrase == "highlight" && kick.channelID == nil && !kick.removed)
        // An empty display name falls back to the login.
        #expect(kick.shownSender == "kicker")
        // The chat opens by its own-case key; the room stays the lowercased match key.
        #expect(kick.chatKey == "kick:Slug" && kick.room == "kick:slug" && kick.channelLogin == "Slug")
        // An entry filed before `open_key` existed opens by its room.
        #expect(reply.openKey == nil && reply.chatKey == "xqc")
    }

    @Test func aRemovedMessageSaysSo() {
        var entry = MentionEntry(id: "a", room: "r", channelLogin: "r", senderLogin: "s", senderName: "S", text: "secret", atMs: 0)
        #expect(entry.shownText == "secret")
        entry.removed = true
        #expect(entry.shownText == "Message deleted by a moderator")
    }

    @Test func notificationTitlesMatchTheDesktop() {
        var entry = MentionEntry(id: "a", room: "somechannel", channelLogin: "somechannel", senderLogin: "viewer",
                                 senderName: "Viewer", text: "hi", kind: .reply, atMs: 0)
        #expect(MentionText.notificationTitle(entry) == "Viewer replied to you in somechannel")
        entry.kind = .mention
        #expect(MentionText.notificationTitle(entry) == "Viewer mentioned you in somechannel")
        entry.kind = .word
        #expect(MentionText.notificationTitle(entry) == "Viewer in somechannel")
    }

    @Test func timesAreCompact() {
        let now: Int64 = 1_760_000_000_000
        #expect(MentionText.time(now - 5_000, nowMs: now) == "now")
        #expect(MentionText.time(now - 5 * 60_000, nowMs: now) == "5m")
        #expect(MentionText.time(now - 3 * 3_600_000, nowMs: now) == "3h")
        #expect(MentionText.time(now - 2 * 86_400_000, nowMs: now) == "2d")
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        // 2025-10-09 08:53 UTC is "now"; ten days back is still this year.
        #expect(MentionText.time(now - 10 * 86_400_000, nowMs: now, calendar: utc) == "Sep 29")
        #expect(MentionText.time(now - 400 * 86_400_000, nowMs: now, calendar: utc) == "Sep 4, 2024")
        // A clock behind the message never reads negative.
        #expect(MentionText.time(now + 60_000, nowMs: now) == "now")
    }

    @Test func roomKeysNameTheirPlatform() {
        #expect(MentionText.platform(ofRoom: "xqc") == "twitch")
        #expect(MentionText.platform(ofRoom: "kick:slug") == "kick")
        #expect(MentionText.channel(ofRoom: "kick:slug") == "slug")
        #expect(MentionText.channel(ofRoom: "xqc") == "xqc")
    }
}

@Suite struct MentionListTests {
    static func entry(_ id: String, _ at: Int64, read: Bool = false, platform: String = "twitch") -> MentionEntry {
        MentionEntry(id: id, platform: platform, room: "r", channelLogin: "r", senderLogin: "s", senderName: "S",
                     text: id, atMs: at, read: read)
    }

    @Test func pagingAppendsOlderRows() {
        var list = MentionList()
        #expect(list.nextBefore == nil && !list.loaded)
        list.takeFirst(MentionPage(entries: [Self.entry("c", 30), Self.entry("b", 20)], hasMore: true, unreadTotal: 3))
        #expect(list.loaded && list.nextBefore == 20)
        list.takeMore(MentionPage(entries: [Self.entry("b", 20), Self.entry("a", 10)], hasMore: false, unreadTotal: 3))
        #expect(list.entries.map(\.id) == ["c", "b", "a"])
        #expect(list.nextBefore == nil)
    }

    @Test func aFreshFirstPageKeepsOlderPagesBelowIt() {
        var list = MentionList()
        list.takeFirst(MentionPage(entries: [Self.entry("c", 30), Self.entry("b", 20)], hasMore: true, unreadTotal: 2))
        list.takeMore(MentionPage(entries: [Self.entry("a", 10)], hasMore: false, unreadTotal: 2))
        // A new mention arrives: the first page now starts with it and still has more after.
        list.takeFirst(MentionPage(entries: [Self.entry("d", 40), Self.entry("c", 30)], hasMore: true, unreadTotal: 3))
        #expect(list.entries.map(\.id) == ["d", "c", "b", "a"])
        #expect(list.unreadTotal == 3)
        // Everything older was already loaded, so nothing more is past it.
        #expect(!list.hasMore)
    }

    @Test func aCompleteFirstPageReplacesEverything() {
        var list = MentionList()
        list.takeFirst(MentionPage(entries: [Self.entry("c", 30), Self.entry("b", 20)], hasMore: true, unreadTotal: 2))
        list.takeFirst(MentionPage(entries: [Self.entry("c", 30)], hasMore: false, unreadTotal: 1))
        #expect(list.entries.map(\.id) == ["c"] && !list.hasMore)
    }

    @Test func localMarksShowAtOnce() {
        var list = MentionList()
        list.takeFirst(MentionPage(entries: [Self.entry("c", 30), Self.entry("b", 20), Self.entry("a", 10, read: true)],
                                   hasMore: false, unreadTotal: 2))
        list.markRead(["c"])
        #expect(list.unreadTotal == 1 && list.entries[0].read && !list.entries[1].read)
        list.remove("b")
        #expect(list.unreadTotal == 0 && list.entries.map(\.id) == ["c", "a"])
        list.markRead(nil)
        #expect(list.entries.allSatisfy { $0.read })
        list.clear()
        #expect(list.entries.isEmpty && list.unreadTotal == 0)
    }

    @Test func logosShowOnlyWherePlatformsMix() {
        var list = MentionList()
        list.takeFirst(MentionPage(entries: [Self.entry("b", 20), Self.entry("a", 10)], hasMore: false, unreadTotal: 0))
        #expect(!list.mixedPlatforms)
        list.takeFirst(MentionPage(entries: [Self.entry("b", 20), Self.entry("k", 10, platform: "kick")], hasMore: false,
                                   unreadTotal: 0))
        #expect(list.mixedPlatforms)
    }
}

@Suite struct FakeMentionsTests {
    @Test func theFakeAnswersLikeTheCore() async throws {
        let core = FakeCore(fixtures: FakeFixtures(signedIn: true))
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "test",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        let changes = core.events.stream("mention-inbox-changed")
        let first = try await core.invoke("get_mention_inbox", args: ["limit": 2], as: MentionPage.self)
        #expect(first.entries.count == 2 && first.hasMore && first.unreadTotal == 3)
        let next = try await core.invoke("get_mention_inbox", args: ["beforeMs": first.entries[1].atMs, "limit": 50],
                                         as: MentionPage.self)
        #expect(next.entries.count == 8 && !next.hasMore)
        let opened = try await core.invoke("open_mention", args: ["id": "fm-1"], as: MentionEntry?.self)
        #expect(opened?.read == true)
        var iterator = changes.makeAsyncIterator()
        let event = await iterator.next()
        #expect(MentionInboxChanged.parse(event?.payloadJSON ?? "")?.unreadTotal == 2)
        _ = try await core.invoke("mark_mentions_read", args: [:])
        let after = try await core.invoke("get_mention_inbox", args: [:], as: MentionPage.self)
        #expect(after.unreadTotal == 0)
        _ = try await core.invoke("clear_mention_inbox", args: [:])
        let empty = try await core.invoke("get_mention_inbox", args: [:], as: MentionPage.self)
        #expect(empty.entries.isEmpty)
    }
}

@Suite struct InboxModelTests {
    /// The shape the core's `get_inbox_view` sends.
    static let viewJSON = """
    {"rows":[
      {"entry":{"id":"m3","platform":"twitch","room":"xqc","channel_login":"xqc","sender_id":"5","sender_login":"viewer",
       "sender_name":"Viewer","text":"@you hi Kappa","kind":"mention","at_ms":3000,"read":false,"removed":false,
       "parts":[{"type":"text","text":"@you hi "},{"type":"emote","name":"Kappa","url":"https://e/25"}]},
       "folded":[{"id":"m2","at_ms":2000,"read":true},{"id":"m1","at_ms":1000,"read":false}],
       "unread":true,"avatar":"https://p/5.png"}
    ],"has_more":true,"before_ms":1000,"unread_total":2,
    "chips":[{"id":"all","unread":2},{"id":"mentions","unread":2},{"id":"highlights","unread":0}]}
    """

    @Test func decodesTheCoresView() throws {
        let view = try JSONDecoder().decode(InboxView.self, from: Data(Self.viewJSON.utf8))
        #expect(view.hasMore && view.beforeMs == 1000 && view.unreadTotal == 2)
        #expect(view.chips.map(\.id) == ["all", "mentions", "highlights"])
        let row = try #require(view.rows.first)
        #expect(row.ids == ["m3", "m2", "m1"] && row.unread && row.avatar == "https://p/5.png")
        #expect(row.entry.parts == [.text("@you hi "), .emote(name: "Kappa", url: "https://e/25")])
        #expect(InboxText.folded(row.entry, count: row.folded.count + 1) == "3 identical mentions")
    }

    @Test func anOlderEntryWithoutPartsStillDecodes() throws {
        let page = try JSONDecoder().decode(MentionPage.self, from: Data(MentionModelTests.pageJSON.utf8))
        #expect(page.entries.allSatisfy { $0.parts.isEmpty })
    }

    @Test func pagesAppendWithoutRepeats() throws {
        var view = try JSONDecoder().decode(InboxView.self, from: Data(Self.viewJSON.utf8))
        view.append(InboxView(rows: view.rows, hasMore: false, beforeMs: nil, unreadTotal: 0, chips: []))
        #expect(view.rows.count == 1 && !view.hasMore && view.unreadTotal == 0)
    }

    @Test func picturesAreKeyedInEachPlatformsIdSpace() {
        let twitch = MentionEntry(id: "a", room: "xqc", channelLogin: "xqc", senderID: "123", senderLogin: "Viewer",
                                  senderName: "Viewer", text: "", atMs: 0)
        #expect(InboxText.avatarKey(twitch) == "twitch:123")
        var kick = twitch
        kick.platform = "kick"
        #expect(InboxText.avatarKey(kick) == "kick:viewer")
        var other = twitch
        other.platform = "tiktok"
        #expect(InboxText.avatarKey(other) == nil)
    }

    @Test func chipsAndEmptyListsHaveWords() {
        #expect(InboxText.chip("mentions") == "Mentions" && InboxText.chip("whispers") == "Whispers")
        #expect(InboxText.empty("highlights").text == "No highlights yet.")
    }

    @Test func theFakeFoldsAndFiltersLikeTheCore() async throws {
        let core = FakeCore(fixtures: FakeFixtures(signedIn: true))
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "test",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        let all = try await core.invoke("get_inbox_view", args: ["filter": "all", "limit": 50], as: InboxView.self)
        // Ten fixture entries, the bot's three identical lines folded into one row.
        #expect(all.rows.count == 8 && !all.hasMore)
        #expect(all.rows.last?.folded.map(\.id) == ["fm-9", "fm-10"])
        let words = try await core.invoke("get_inbox_view", args: ["filter": "highlights"], as: InboxView.self)
        #expect(words.rows.map(\.id) == ["fm-4"])
        let first = try await core.invoke("get_inbox_view", args: ["filter": "all", "limit": 2], as: InboxView.self)
        #expect(first.rows.count == 2 && first.hasMore && first.beforeMs == first.rows[1].entry.atMs)
    }
}
