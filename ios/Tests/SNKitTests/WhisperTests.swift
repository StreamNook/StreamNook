import Foundation
import Testing
@testable import SNKit

@Suite struct WhisperModelTests {
    /// The shape the core's `whisper_list` sends: the meta flattened into each row.
    static let listJSON = """
    {"owner_id":"1","conversations":[
      {"key":"42","user_id":"42","user_login":"bob","user_name":"Bob","profile_image_url":null,
       "last_message_timestamp":2000,"unread_count":2,"last_text":"hi","last_is_sent":false},
      {"key":"43","user_id":"43","user_login":"amy","user_name":"","profile_image_url":"https://x/a.png",
       "last_message_timestamp":1000,"unread_count":0,"last_text":"ok","last_is_sent":true}
    ],"unread_total":2}
    """

    @Test func decodesTheCoresList() throws {
        let list = try JSONDecoder().decode(WhisperList.self, from: Data(Self.listJSON.utf8))
        #expect(list.ownerID == "1" && list.unreadTotal == 2 && list.conversations.count == 2)
        let bob = list.conversations[0]
        #expect(bob.shownName == "Bob" && bob.unreadCount == 2 && bob.profileImageURL == nil)
        #expect(WhisperText.preview(bob) == "hi")
        // An empty display name falls back to the login; your own last line says so.
        let amy = list.conversations[1]
        #expect(amy.shownName == "amy")
        #expect(WhisperText.preview(amy) == "You: ok")
    }

    @Test func decodesAnUpdateAndAppliesItOnce() throws {
        let json = """
        {"owner_id":"1","key":"42","replaced_key":null,
         "meta":{"user_id":"42","user_login":"bob","user_name":"Bob","profile_image_url":"pic","last_message_timestamp":3000,"unread_count":1},
         "message":{"id":"w1","from_user_id":"42","from_user_login":"bob","from_user_name":"Bob","to_user_id":"1",
                    "to_user_login":"me","to_user_name":"Me","message":"yo","timestamp":3000,"is_sent":false}}
        """
        let update = try #require(WhisperUpdate.parse(json))
        let once = try #require(update.applied(to: nil, key: "42"))
        #expect(once.messages.map(\.id) == ["w1"] && once.profileImageURL == "pic" && once.unreadCount == 1)
        #expect(update.applied(to: once, key: "42")?.messages.count == 1)
        // Another conversation's update changes nothing there.
        #expect(update.applied(to: once, key: "7") == once)
    }

    @Test func theWordsMatchAndroidAndTheDesktop() {
        #expect(WhisperText.notificationTitle("Bob") == "Whisper from Bob")
        #expect(WhisperText.replyFailedTitle("Bob") == "Your reply to Bob was not sent")
        #expect(WhisperText.placeholder("Bob") == "Whisper Bob")
        #expect(WhisperText.name("", "") == "Someone")
        // The core's refusal reads as it was said; anything unreadable gets the plain fallback.
        #expect(WhisperText.reason(CoreFailure.command(json: #""Cannot send: User not found.""#)) == "Cannot send: User not found.")
    }

    @Test func theThreadHeadsDaysAndTimesRuns() {
        var utc = Calendar(identifier: .gregorian)
        utc.timeZone = TimeZone(identifier: "UTC")!
        let now: Int64 = 1_760_886_000_000 // 2025-10-19 15:00 UTC
        let minute: Int64 = 60_000
        func m(_ id: String, _ ago: Int64, mine: Bool) -> WhisperMessage {
            WhisperMessage(id: id, message: "x", timestamp: now - ago * minute, isSent: mine)
        }
        let messages = [m("a", 26 * 60, mine: false), m("b", 30, mine: false), m("c", 29, mine: false),
                        m("d", 28, mine: true), m("e", 2, mine: true)]
        let lines = WhisperThreadItem.lines(messages, pending: [], nowMs: now, calendar: utc)
        #expect(lines.map(\.id) == ["day-\(now - 26 * 60 * minute)", "a", "day-\(now - 30 * minute)", "b", "c", "d", "e"])
        guard case .day(_, let yesterday) = lines[0], case .day(_, let today) = lines[2] else {
            Issue.record("no day headings")
            return
        }
        #expect(yesterday == "Yesterday" && today == "Today")
        guard case .message(_, let bLast, let bTime) = lines[3], case .message(_, let cLast, let cTime) = lines[4] else {
            Issue.record("no messages")
            return
        }
        #expect(!bLast && bTime == nil)
        #expect(cLast && cTime == "2:31 PM")
        // Something still on its way goes after the thread, under Today.
        let pending = PendingWhisper(id: "l1", key: "42", text: "hey", atMs: now)
        let withPending = WhisperThreadItem.lines([m("a", 26 * 60, mine: false)], pending: [pending], nowMs: now, calendar: utc)
        #expect(withPending.map(\.id) == ["day-\(now - 26 * 60 * minute)", "a", "day-pending", "l1"])
    }
}

@Suite struct FakeWhispersTests {
    private func started() throws -> FakeCore {
        let core = FakeCore(fixtures: FakeFixtures(signedIn: true))
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "t",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        return core
    }

    @Test func theFixtureInboxCoversEveryState() async throws {
        let core = try started()
        let list = try await core.invoke("whisper_list", args: [:], as: WhisperList.self)
        #expect(list.ownerID == FakeWhispers.owner)
        #expect(list.unreadTotal == 2)
        // Newest first: the unread conversation leads.
        #expect(list.conversations.first?.userLogin == "glassowl")
        let long = try #require(try await core.invoke("whisper_thread", args: ["ownerId": list.ownerID, "key": "1102"],
                                                      as: WhisperThread?.self))
        #expect(long.messages.count > 20)
    }

    @Test func aSendIsRecordedAndARefusalSaysWhy() async throws {
        let core = try started()
        let target: [String: Any] = ["key": "1101", "user_id": "1101", "user_login": "glassowl", "user_name": "GlassOwl"]
        let update = try await core.invoke("whisper_send", args: ["ownerId": FakeWhispers.owner, "target": target, "text": "on my way"],
                                           as: WhisperUpdate.self)
        #expect(update.message?.isSent == true && update.message?.message == "on my way")
        #expect(core.fixtures?.whispers.sentTexts == ["on my way"])
        let refused: [String: Any] = ["key": FakeWhispers.refusingKey, "user_id": FakeWhispers.refusingKey]
        do {
            _ = try await core.invoke("whisper_send", args: ["ownerId": FakeWhispers.owner, "target": refused, "text": "hi"])
            Issue.record("the refusing fixture accepted a whisper")
        } catch {
            #expect(WhisperText.reason(error) == FakeWhispers.refusal)
        }
    }

    @Test func anIncomingWhisperIsAnnouncedOnlyOutsideTheOpenConversation() async throws {
        let core = try started()
        let notified = core.events.stream("whisper-notify")
        _ = try await core.invoke("whisper_set_active", args: ["key": "1102"])
        core.simulateWhisper(from: "1102", text: "read where it lands")
        core.simulateWhisper(from: "1104", text: "this one notifies")
        var iterator = notified.makeAsyncIterator()
        let first = await iterator.next()
        #expect(first.flatMap { WhisperUpdate.parse($0.payloadJSON) }?.key == "1104")
    }
}
