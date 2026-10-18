import Foundation
import Testing
@testable import SNKit

@Suite struct ModToolsTests {
    private static func heldFrame(status: String, line: String? = nil, by: String? = nil) -> String {
        var frame: [String: Any] = [
            "type": "AUTOMOD_HELD",
            "channel": "zackrawrr",
            "held": [
                "message_id": "m9", "channel": "zackrawrr", "broadcaster_id": "1", "user_id": "42",
                "user_login": "grumpy_gus", "user_name": "Grumpy_Gus", "text": "this game is broken",
                "reason": "automod", "category": "swearing", "level": 2, "held_at_ms": 1_760_000_000_000,
                "status": status, "label": "Held by AutoMod \u{00B7} Swearing, level 2",
            ] as [String: Any],
        ]
        if let line { frame["line"] = line }
        if let by { frame["by"] = by }
        let data = try! JSONSerialization.data(withJSONObject: frame)
        return String(decoding: data, as: UTF8.self)
    }

    @Test func aHeldFrameDecodesForItsRoom() throws {
        let frame = ChatSocketFrame.decode(Self.heldFrame(status: "held"))
        guard case .held(let channel, let row) = frame else {
            Issue.record("not a held frame: \(frame)")
            return
        }
        #expect(channel == "zackrawrr")
        #expect(frame.room == "zackrawrr")
        #expect(row.held.isPending)
        #expect(row.held.shownName == "Grumpy_Gus")
        #expect(row.held.label == "Held by AutoMod \u{00B7} Swearing, level 2")
        #expect(row.line == nil)
    }

    @Test func aHeldRowFoldsInPlaceOnceDecided() {
        var buffer = ChatRoomBuffer()
        let first = buffer.apply([ChatSocketFrame.decode(Self.heldFrame(status: "held"))])
        #expect(first.appended.count == 1)
        let id = ChatRoomBuffer.heldRowID("m9")
        #expect(buffer.row(id)?.held?.held.isPending == true)
        #expect(buffer.row(id)?.message.content == "this game is broken")

        let decided = ChatSocketFrame.decode(Self.heldFrame(status: "denied", line: "Denied Grumpy_Gus\u{2019}s held message", by: "kay"))
        let second = buffer.apply([decided])
        #expect(second.appended.isEmpty)
        #expect(second.updated.map(\.id) == [id])
        #expect(buffer.row(id)?.held?.line == "Denied Grumpy_Gus\u{2019}s held message")
        #expect(buffer.row(id)?.held?.by == "kay")
        #expect(buffer.rows.count == 1)
    }

    @Test func aDecisionWithNoHeldRowAddsNothing() {
        var buffer = ChatRoomBuffer()
        let change = buffer.apply([ChatSocketFrame.decode(Self.heldFrame(status: "approved", line: "Allowed"))])
        #expect(change.isEmpty)
    }

    @Test func roomStateCarriesAPendingWarning() {
        let raw = #"{"type":"ROOMSTATE","channel":"zackrawrr","modes":{"modes":[],"slow_seconds":0,"warning":{"title":"A moderator warned you","detail":"“no slurs” You can chat again after you acknowledge it.","action":"Acknowledge","prompt":"Acknowledge the warning to chat","short":"Warned"}}}"#
        guard case .roomState(_, let modes) = ChatSocketFrame.decode(raw) else {
            Issue.record("not a room state")
            return
        }
        #expect(modes?.warning?.action == "Acknowledge")
        #expect(modes?.warning?.prompt == "Acknowledge the warning to chat")
        let plain = #"{"type":"ROOMSTATE","channel":"zackrawrr","modes":{"modes":[],"slow_seconds":0}}"#
        guard case .roomState(_, let open) = ChatSocketFrame.decode(plain) else { return }
        #expect(open?.warning == nil)
    }

    @Test func rowViewCarriesTheViewersNickname() throws {
        let raw = #"{"name_color":"#ff0000","nickname":"Cake"}"#
        let view = try JSONDecoder().decode(ChatRowView.self, from: Data(raw.utf8))
        #expect(view.nickname == "Cake")
        #expect(view.nameColor == "#ff0000")
    }

    @Test func historyDecodesFromTheFake() async throws {
        let core = FakeCore()
        try core.start(dirs: CoreDirs(data: "/tmp", cache: "/tmp", logs: "/tmp"), version: "test",
                       vaultKey: Data(repeating: 1, count: 32), codecs: [])
        let history = try await core.modHistory(channelID: "1", userID: "42")
        #expect(history.isAvailable)
        #expect(history.entries.map(\.kind) == ["comment", "timeout", "deleted", "warn"])
        #expect(history.entries[1].seconds == 600)
        #expect(history.entries[2].by == nil)
        try await core.acknowledgeWarning(roomID: "1")
        try await core.resolveHeld(messageID: "m9", allow: false)
        #expect(core.calls.suffix(3) == ["get_chatter_mod_history", "acknowledge_chat_warning", "resolve_automod_message"])
    }

    @Test func historyStatusesOtherThanAvailableCarryANote() throws {
        let raw = #"{"status":"no_access","entries":[],"note":"Only moderators of this channel can see its history."}"#
        let history = try JSONDecoder().decode(ModHistory.self, from: Data(raw.utf8))
        #expect(!history.isAvailable)
        #expect(history.note != nil)
        #expect(history.current == nil)
    }
}
