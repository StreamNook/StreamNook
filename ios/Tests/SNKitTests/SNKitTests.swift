import Foundation
import Testing
@testable import SNKit

@Suite struct CoreEventsTests {
    @Test func eventsReachOnlyTheirNameAndTheCatchAll() async {
        let events = CoreEvents()
        let chat = events.stream("chat")
        let other = events.stream("other")
        let all = events.all()
        #expect(events.listenerCount("chat") == 1)
        #expect(events.listenerCount() == 1)

        events.emit(name: "chat", payloadJSON: "1", target: nil)
        events.emit(name: "other", payloadJSON: "2", target: "main")
        events.finish()

        let chatSeen = await collect(chat)
        let otherSeen = await collect(other)
        let allSeen = await collect(all)
        #expect(chatSeen == [CoreEvent(name: "chat", payloadJSON: "1")])
        #expect(otherSeen == [CoreEvent(name: "other", payloadJSON: "2", target: "main")])
        #expect(allSeen.map(\.payloadJSON) == ["1", "2"])
    }

    @Test func twoListenersOfOneNameBothReceive() async {
        let events = CoreEvents()
        let a = events.stream("x")
        let b = events.stream("x")
        events.emit(name: "x", payloadJSON: "{}", target: nil)
        events.finish()
        #expect(await collect(a).count == 1)
        #expect(await collect(b).count == 1)
    }

    @Test func aStoppedListenerIsDropped() async throws {
        let events = CoreEvents()
        let task = Task {
            for await _ in events.stream("gone") {}
        }
        for _ in 0..<50 where events.listenerCount("gone") == 0 { try await Task.sleep(for: .milliseconds(10)) }
        #expect(events.listenerCount("gone") == 1)
        task.cancel()
        for _ in 0..<50 where events.listenerCount("gone") > 0 { try await Task.sleep(for: .milliseconds(10)) }
        #expect(events.listenerCount("gone") == 0)
        events.emit(name: "gone", payloadJSON: "{}", target: nil)
    }

    @Test func hostRequestsHaveTheirOwnStream() async {
        let events = CoreEvents()
        let requests = events.requests()
        let all = events.all()
        events.hostRequest(kind: "open_url", payloadJSON: #"{"url":"https://twitch.tv"}"#)
        events.finish()
        #expect(await collect(requests) == [HostRequest(kind: "open_url", payloadJSON: #"{"url":"https://twitch.tv"}"#)])
        #expect(await collect(all).isEmpty)
    }

    private func collect<T>(_ stream: AsyncStream<T>) async -> [T] {
        var seen: [T] = []
        for await value in stream { seen.append(value) }
        return seen
    }
}

@Suite struct FakeCoreTests {
    private func started() throws -> FakeCore {
        let core = FakeCore()
        try core.start(dirs: CoreDirs(data: "/d", cache: "/c", logs: "/l"), version: "0.1.0",
                       vaultKey: Data(repeating: 7, count: 32), codecs: ["h264"])
        return core
    }

    @Test func commandsWaitForStart() async {
        let core = FakeCore()
        await #expect(throws: CoreFailure.self) { try await core.invoke("start_chat", "{}") }
    }

    @Test func startRefusesAShortVaultKey() {
        let core = FakeCore()
        #expect(throws: CoreFailure.self) {
            try core.start(dirs: CoreDirs(data: "", cache: "", logs: ""), version: "", vaultKey: Data(), codecs: [])
        }
    }

    @Test func startAnnouncesItself() async throws {
        let core = FakeCore()
        let started = core.events.stream("core-started")
        try core.start(dirs: CoreDirs(data: "/d", cache: "/c", logs: "/l"), version: "0.1.0",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        core.events.finish()
        var payloads: [String] = []
        for await event in started { payloads.append(event.payloadJSON) }
        #expect(payloads == [#"{"version":"0.1.0","core":"fake"}"#])
    }

    @Test func browseListsTwentyStreamsAcrossCategories() async throws {
        let core = try started()
        let streams = try await Browse.load(core)
        #expect(streams.count == 20)
        #expect(Set(streams.map(\.id)).count == 20)
        #expect(streams.first?.userLogin == "fakestreamer0")
        #expect(streams.last?.gameName == "League of Legends")
        // FakeCore has no home snapshot, so Browse falls back to the top categories.
        #expect(core.calls == ["refresh_home_section", "get_top_games_paginated", "get_streams_by_game", "get_streams_by_game"])
    }

    @Test func streamsByGameNeedsTheTypeScriptArgumentName() async throws {
        let core = try started()
        await #expect(throws: CoreFailure.command(json: #""missing gameId""#)) {
            try await core.invoke("get_streams_by_game", args: ["game_id": "509658"])
        }
    }

    @Test func startStreamAnswersAPlayableURL() async throws {
        let core = try started()
        let start = try await core.invoke(
            "start_stream", args: ["url": "https://twitch.tv/fakestreamer0", "quality": "best"], as: StreamStart.self
        )
        #expect(start.url == FakeCore.testStreamURL)
        #expect(start.quality == "best")
    }

    @Test func startChatAnswersAPort() async throws {
        let core = try started()
        let port = try await core.invoke("start_chat", args: ["channel": "fakestreamer0", "reattach": false], as: Int.self)
        #expect(port == 0)
    }

    @Test func unknownCommandsFail() async throws {
        let core = try started()
        await #expect(throws: CoreFailure.self) { try await core.invoke("nope", "{}") }
    }

    @Test func chatTickerSpeaksTheSocketFormat() async {
        let core = FakeCore()
        var frames: [ChatFrame] = []
        for await raw in core.chatTicker(channel: "fakestreamer0", interval: .milliseconds(1)) {
            frames.append(ChatFrame(raw: raw))
            if frames.count == 5 { break }
        }
        #expect(frames[0] == ChatFrame(kind: .status, text: "IRC_CONNECTED"))
        #expect(frames[1] == ChatFrame(kind: .control, text: "ROOMSTATE fakestreamer0"))
        #expect(frames[2].kind == .message)
        #expect(frames[2].summary == "Glassowl: that was clean")
    }
}

@Suite struct ChatFrameTests {
    @Test func rawIRCLosesItsTagsAndPrefix() {
        let frame = ChatFrame(raw: "@badge-info=;color=#FF0000 :nick!nick@nick.tmi.twitch.tv PRIVMSG #chan :hello")
        #expect(frame.kind == .irc)
        #expect(frame.summary == "PRIVMSG #chan :hello")
    }

    @Test func statusPrefixesAreNamed() {
        #expect(ChatFrame(raw: "PARKED_TICK:1:2:chan").summary == "<PARKED_TICK>")
        #expect(ChatFrame(raw: "RECONNECTING:5").summary == "<RECONNECTING>")
        #expect(ChatFrame(raw: "HEARTBEAT").summary == "<HEARTBEAT>")
    }

    @Test func controlFramesShowTheirType() {
        #expect(ChatFrame(raw: #"{"type":"NOTICE","message":"slow mode"}"#).summary == "[NOTICE slow mode]")
        #expect(ChatFrame(raw: #"{"type":"CLEARCHAT"}"#).summary == "[CLEARCHAT]")
    }

    @Test func messagesFallBackToTheLogin() {
        let frame = ChatFrame(raw: #"{"id":"1","username":"abc","display_name":"","content":"yo"}"#)
        #expect(frame.summary == "abc: yo")
    }

    @Test func theLogKeepsTheNewestThirty() {
        var log = FrameLog()
        for i in 0..<45 { log.append(ChatFrame(kind: .irc, text: "\(i)")) }
        #expect(log.frames.count == 30)
        #expect(log.frames.first?.text == "15")
        #expect(log.frames.last?.text == "44")
    }
}

@Suite struct DecodingTests {
    @Test func pagesDecodeWithAndWithoutACursor() throws {
        let json = #"[[{"id":"1","name":"A","box_art_url":"x"}],"abc"]"#
        let page = try JSONDecoder().decode(Page<TwitchCategory>.self, from: Data(json.utf8))
        #expect(page.items == [TwitchCategory(id: "1", name: "A", boxArtURL: "x")])
        #expect(page.cursor == "abc")
        let last = try JSONDecoder().decode(Page<TwitchCategory>.self, from: Data(#"[[],null]"#.utf8))
        #expect(last.items.isEmpty && last.cursor == nil)
    }

    @Test func categoriesCarryTheirViewersWhenTheCoreSendsThem() throws {
        let json = #"[[{"id":"1","name":"A","box_art_url":"x","viewer_count":182345},{"id":"2","name":"B"}],null]"#
        let page = try JSONDecoder().decode(Page<TwitchCategory>.self, from: Data(json.utf8))
        #expect(page.items.map(\.viewerCount) == [182345, nil])
    }

    @Test func streamsIgnoreFieldsTheyDoNotUse() throws {
        let json = #"{"id":"9","user_id":"1","user_name":"Name","user_login":"name","title":"t","viewer_count":5,"game_name":"g","thumbnail_url":"u","started_at":"s","tags":["English"]}"#
        let stream = try JSONDecoder().decode(TwitchStream.self, from: Data(json.utf8))
        #expect(stream.userLogin == "name" && stream.viewerCount == 5)
    }

    @Test func commandErrorsReadPlainly() {
        #expect(CoreFailure.command(json: #""not signed in""#).description == "not signed in")
        #expect(CoreFailure.command(json: #"{"error":"bad"}"#).description == "bad")
        #expect(CoreFailure.command(json: "plain").description == "plain")
    }
}
