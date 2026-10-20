import Foundation
import Testing
@testable import SNKit

@Suite struct ChatFixtureTests {
    private func fixture() throws -> ChatFixture {
        try #require(ChatFixture.bundled())
    }

    @Test func theBundledFixtureMatchesTheOneRustWrote() throws {
        let tests = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
        let written = try Data(contentsOf: tests.appending(path: "Fixtures/chat-frames.json"))
        let bundled = try Data(contentsOf: try #require(Bundle.module.url(forResource: "chat-frames", withExtension: "json")))
        #expect(written == bundled)
    }

    @Test func everyFrameDecodes() throws {
        let frames = try fixture().frames.map(ChatSocketFrame.decode)
        #expect(!frames.contains { if case .other = $0 { return true } else { return false } })
        let rows = frames.compactMap { if case .row(let m) = $0 { return m } else { return nil } }
        #expect(rows.count >= 30)
        #expect(rows.allSatisfy { $0.metadata.view != nil })
    }

    @Test func overlaysArriveGroupedOnTheirBase() throws {
        let rows = try fixture().frames.compactMap { if case .row(let m) = ChatSocketFrame.decode($0) { return m } else { return nil } }
        let pet = try #require(rows.first { $0.id == "fx-0003" })
        guard case .emote(let base, let overlays, _) = pet.metadata.view?.spans.first else {
            Issue.record("no emote span")
            return
        }
        #expect(base.code == "PepePls")
        #expect(base.width == 22 && base.height == 32)
        #expect(overlays.map(\.code) == ["PETPET"])
    }

    @Test func roomStateCarriesTheModesInWords() throws {
        let frames = try fixture().frames.map(ChatSocketFrame.decode)
        guard case .roomState(_, let modes)? = frames.first(where: { if case .roomState = $0 { return true } else { return false } })
        else {
            Issue.record("no room state")
            return
        }
        #expect(modes?.modes.map(\.kind) == ["followers_only", "slow"])
        #expect(modes?.prompt == "Followers-only (10m)")
    }

    @Test func replayGivesFreshIdsOnEveryLoop() {
        let frame = #"{"id":"fx-0003","metadata":{"reply_info":{"parent_msg_id":"fx-clap"}}}"#
        let next = ChatFixture.frame(frame, loop: 2)
        #expect(next.contains(#""fx2-0003""#))
        #expect(next.contains(#""fx2-clap""#))
        #expect(ChatFixture.frame(frame, loop: 0) == frame)
    }
}

@Suite struct ChatRoomBufferTests {
    private func row(_ id: String, user: String = "1") -> ChatSocketFrame {
        .row(ChatMessage(id: id, userID: user, username: "u\(user)", content: "hi"))
    }

    @Test func rowsPastTheCapLeaveFromTheTop() {
        var buffer = ChatRoomBuffer(cap: 50)
        let change = buffer.apply((0..<60).map { row("m\($0)") })
        #expect(change.appended.count == 60)
        #expect(change.trimmed == 10)
        #expect(buffer.rows.first?.id == "m10")
    }

    @Test func pausedRoomsGrowAHundredAndFiftyOverTheCapThenDecay() {
        var buffer = ChatRoomBuffer(cap: 50)
        buffer.paused = true
        _ = buffer.apply((0..<250).map { row("m\($0)") })
        #expect(buffer.rows.count == 200)
        buffer.paused = false
        let change = buffer.apply([row("x")])
        #expect(change.trimmed == 6)
        #expect(buffer.rows.count == 195)
    }

    @Test func timeoutsMarkEveryRowOfThatUser() {
        var buffer = ChatRoomBuffer(cap: 100)
        _ = buffer.apply([row("a", user: "7"), row("b", user: "8"), row("c", user: "7")])
        let change = buffer.apply([.clearChat(channel: nil, targetUserID: "7", targetUser: "u7", duration: 600)])
        #expect(Set(change.updated.map(\.id)) == ["a", "c"])
        #expect(buffer.row("a")?.moderation == .timedOut(seconds: 600))
        #expect(buffer.row("b")?.moderation == nil)
        let ban = buffer.apply([.clearChat(channel: nil, targetUserID: "8", targetUser: "u8", duration: nil)])
        #expect(ban.updated.map(\.id) == ["b"])
        #expect(buffer.row("b")?.moderation == .banned)
    }

    @Test func aFullClearMarksNothing() {
        var buffer = ChatRoomBuffer(cap: 100)
        _ = buffer.apply([row("a")])
        let change = buffer.apply([.clearChat(channel: nil, targetUserID: nil, targetUser: nil, duration: nil)])
        #expect(change.isEmpty)
    }

    @Test func deletionsMarkOneRowAndBumpItsRevision() {
        var buffer = ChatRoomBuffer(cap: 100)
        _ = buffer.apply([row("a"), row("b")])
        let change = buffer.apply([.clearMessage(channel: nil, targetID: "b")])
        #expect(change.updated.map(\.id) == ["b"])
        #expect(buffer.row("b")?.moderation == .deleted)
        #expect(buffer.row("b")?.revision == 1)
    }

    @Test func aRowSeenTwiceIsListedOnce() {
        var buffer = ChatRoomBuffer(cap: 100)
        _ = buffer.apply([row("a")])
        let change = buffer.apply([row("a")])
        #expect(change.appended.isEmpty)
        #expect(buffer.rows.count == 1)
    }

    @Test func anOutageBackfillFillsTheGapInTimeOrderAndKeepsMarks() {
        func at(_ id: String, _ ms: Int, user: String = "1") -> ChatMessage {
            ChatMessage(id: id, userID: user, username: "u", content: "x", tags: ["tmi-sent-ts": "\(ms)"])
        }
        var buffer = ChatRoomBuffer(cap: 100)
        _ = buffer.apply([.row(at("a", 100, user: "7")), .row(at("d", 400))])
        _ = buffer.apply([.clearChat(channel: nil, targetUserID: "7", targetUser: "u7", duration: 60)])
        let added = buffer.insertInTimeOrder([at("c", 300), at("b", 200), at("a", 100)])
        #expect(added)
        #expect(buffer.rows.map(\.id) == ["a", "b", "c", "d"])
        #expect(buffer.row("a")?.moderation == .timedOut(seconds: 60))
        #expect(buffer.row("c")?.live == false)
        let again = buffer.insertInTimeOrder([at("b", 200)])
        #expect(!again)
    }
}

@Suite struct ImageCachingTests {
    @Test func tiersPickTheSmallestFileThatCovers() {
        #expect(ImageTier.url("https://cdn.7tv.app/emote/abc/1x.avif", pixelHeight: 84) == "https://cdn.7tv.app/emote/abc/3x.avif")
        #expect(ImageTier.url("https://cdn.7tv.app/emote/abc/1x.webp", pixelHeight: 20) == "https://cdn.7tv.app/emote/abc/1x.webp")
        #expect(ImageTier.url("https://cdn.betterttv.net/emote/abc/1x", pixelHeight: 84) == "https://cdn.betterttv.net/emote/abc/3x")
        #expect(ImageTier.url("https://cdn.frankerfacez.com/emote/1/1", pixelHeight: 84) == "https://cdn.frankerfacez.com/emote/1/4")
        #expect(ImageTier.url("https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/1.0", pixelHeight: 56)
            == "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/2.0")
        #expect(ImageTier.url("https://example.com/a.png", pixelHeight: 84) == "https://example.com/a.png")
    }

    @Test func theCacheEvictsTheLeastRecentlyUsedByCost() {
        let cache = CostCache<String, Int>(limit: 100)
        cache.insert(1, for: "a", cost: 40)
        cache.insert(2, for: "b", cost: 40)
        _ = cache.value("a")
        cache.insert(3, for: "c", cost: 40)
        #expect(cache.value("b") == nil)
        #expect(cache.value("a") == 1)
        #expect(cache.totalCost == 80)
        cache.trim(toFraction: 0.5)
        #expect(cache.totalCost <= 50)
    }

    @Test func everyCopyOfAnAnimationPicksTheSameFrame() {
        let timeline = FrameTimeline(durations: [0.1, 0.2, 0.01])
        #expect(timeline.total == 0.4)
        #expect(timeline.frame(at: 0.05) == 0)
        #expect(timeline.frame(at: 0.15) == 1)
        #expect(timeline.frame(at: 0.35) == 2)
        #expect(timeline.frame(at: 0.45) == 0)
    }

    @Test func cacheFileNamesAreStableAndKeepTheFormat() {
        let name = ImageCacheKey.fileName(for: "https://cdn.7tv.app/emote/abc/3x.webp")
        #expect(name == ImageCacheKey.fileName(for: "https://cdn.7tv.app/emote/abc/3x.webp"))
        #expect(name.hasSuffix(".webp"))
        #expect(ImageCacheKey.fileName(for: "https://cdn.betterttv.net/emote/abc/3x").count == 16)
    }
}

@Suite struct LateCosmeticsTests {
    private func row(_ id: String, user: String, provider: String = "twitch", badges: [ChatViewBadge]) -> ChatSocketFrame {
        var metadata = ChatMetadata()
        var view = ChatRowView(nameColor: "#ffffff")
        view.badges = badges
        metadata.view = view
        return .row(ChatMessage(id: id, userID: user, username: "u", content: "x", provider: provider, metadata: metadata))
    }

    private let mod = ChatViewBadge(url: "m", provider: "twitch")
    private let ffz = ChatViewBadge(url: "f", provider: "ffz")
    private let member = ChatViewBadge(url: "s", provider: "streamnook")

    @Test func aLate7TVBadgeSitsAfterThePlatformsAndBeforeTheChatAppsAndStreamNook() {
        var buffer = ChatRoomBuffer(cap: 50)
        _ = buffer.apply([row("a", user: "1", badges: [mod, ffz, member])])
        let first = ChatViewBadge(url: "7a", provider: "7tv")
        _ = buffer.apply([.cosmetics(channel: nil, userID: "1", paint: nil, badge: first)])
        #expect(buffer.row("a")?.message.metadata.view?.badges.map(\.url) == ["m", "7a", "f", "s"])
        // A changed 7TV badge replaces the old one rather than joining it.
        _ = buffer.apply([.cosmetics(channel: nil, userID: "1", paint: nil, badge: ChatViewBadge(url: "7b", provider: "7tv"))])
        #expect(buffer.row("a")?.message.metadata.view?.badges.map(\.url) == ["m", "7b", "f", "s"])
    }

    @Test func aTwitchChattersCosmeticsNeverDressAKickChatterWithTheSameNumber() {
        var buffer = ChatRoomBuffer(cap: 50)
        _ = buffer.apply([row("t", user: "676", badges: []), row("k", user: "676", provider: "kick", badges: [])])
        _ = buffer.apply([.cosmetics(channel: nil, userID: "676", paint: nil, badge: ChatViewBadge(url: "7", provider: "7tv"))])
        #expect(buffer.row("t")?.message.metadata.view?.badges.count == 1)
        #expect(buffer.row("k")?.message.metadata.view?.badges.isEmpty == true)
    }

    @Test func theFrameNamesAKickAuthorByTheirPlatform() {
        let frame = ChatSocketFrame.decode(#"{"type":"ROW_COSMETICS","channel":"x","provider":"kick","user_id":"676","paint":null,"badge":null}"#)
        guard case .cosmetics(_, let userID, _, _) = frame else {
            Issue.record("not a cosmetics frame")
            return
        }
        #expect(userID == "kick:676")
    }
}

@Suite struct ChatStyleTests {
    @Test func absentKeysKeepThePhonesFirstRunLook() {
        let style = ChatStyle(settingsJSON: Data("{}".utf8))
        #expect(style.fontSize == 14)
        #expect(style.messageEntrance == "slide")
        #expect(style.deletedMessageStyle == "italic")
        #expect(style.rowPadding == 4)
    }

    @Test func settingsAreReadAndClamped() {
        let json = #"{"chat_design":{"font_size":60,"badge_scale":9,"animate_emotes":"hover"},"chat_render":{"message_buffer_cap":900}}"#
        let style = ChatStyle(settingsJSON: Data(json.utf8))
        #expect(style.fontSize == 48)
        #expect(style.badgeScale == 2.5)
        #expect(style.animatesEmotes == false)
        #expect(style.bufferCap == 300)
    }
}

@Suite struct LinkPreviewTests {
    private let invite = "https://discord.gg/discord-developers"

    private func row(_ id: String, previews: [ChatRowPreview]) -> ChatSocketFrame {
        var metadata = ChatMetadata()
        var view = ChatRowView(nameColor: "#ffffff")
        view.previews = previews
        metadata.view = view
        return .row(ChatMessage(id: id, username: "u", content: "x", metadata: metadata))
    }

    @Test func theFixtureInviteCarriesItsServerCard() throws {
        let rows = try #require(ChatFixture.bundled()).frames.compactMap {
            if case .row(let m) = ChatSocketFrame.decode($0) { return m } else { return nil }
        }
        let card = try #require(rows.first { $0.id == "fx-discord" }?.metadata.view?.previews.first)
        #expect(card.url == invite)
        #expect(card.preview?.kind == "discord")
        #expect(card.preview?.title == "Discord Developers")
        #expect(card.preview?.memberCount == 308559)
        #expect(card.preview?.onlineCount == 70761)
    }

    @Test func aPreviewFrameDecodesForEveryRoom() {
        let frame = ChatSocketFrame.decode(#"{"type":"ROW_PREVIEW","url":"https://x.com/a","preview":{"url":"https://x.com/a","kind":"tweet","title":"Hi"}}"#)
        guard case .preview(let url, let preview) = frame else {
            Issue.record("\(frame)")
            return
        }
        #expect(url == "https://x.com/a")
        #expect(preview?.title == "Hi")
        #expect(frame.room == nil)
        let none = ChatSocketFrame.decode(#"{"type":"ROW_PREVIEW","url":"https://x.com/a","preview":null}"#)
        guard case .preview(_, nil) = none else {
            Issue.record("\(none)")
            return
        }
    }

    @Test func theAnswerFillsEveryRowWithTheLinkAndANoneDropsTheHeldCard() {
        var buffer = ChatRoomBuffer(cap: 100)
        _ = buffer.apply([
            row("a", previews: [ChatRowPreview(url: invite, reserve: true)]),
            row("b", previews: [ChatRowPreview(url: invite, reserve: true)]),
            row("c", previews: [ChatRowPreview(url: "https://youtu.be/x")]),
        ])
        let card = ChatLinkPreview(url: invite, kind: "discord", title: "Discord Developers", onlineCount: 1, memberCount: 2)
        let filled = buffer.apply([.preview(url: invite, preview: card)])
        #expect(Set(filled.updated.map(\.id)) == ["a", "b"])
        #expect(buffer.row("a")?.message.metadata.view?.previews.first?.preview == card)
        #expect(buffer.row("a")?.revision == 1)
        let dropped = buffer.apply([.preview(url: "https://youtu.be/x", preview: nil)])
        #expect(dropped.updated.map(\.id) == ["c"])
        #expect(buffer.row("c")?.message.metadata.view?.previews.isEmpty == true)
    }
}
