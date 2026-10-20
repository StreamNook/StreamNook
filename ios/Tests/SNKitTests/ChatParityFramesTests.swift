import Foundation
import Testing
@testable import SNKit

/// The bridge frames and moderation settings the shared web store reads, read the same way here.
@Suite struct ChatParityFramesTests {
    private func row(_ id: String, user: String = "1", login: String? = nil, display: String = "",
                     paint: ChatPaint? = nil) -> ChatSocketFrame {
        var metadata = ChatMetadata()
        if let paint { metadata.view = ChatRowView(nameColor: "#ffffff", paint: paint) }
        return .row(ChatMessage(id: id, userID: user, username: login ?? "u\(user)", displayName: display, content: "hi",
                                metadata: metadata))
    }

    private func lines(_ change: ChatBufferChange) -> [String] {
        change.appended.filter(\.message.isSystem).map(\.message.content)
    }

    // MARK: Own color

    @Test func ownColorNamesItsRoom() {
        let frame = ChatSocketFrame.decode("USER_COLOR:#XQC:#FF4500")
        #expect(frame == .ownColor(channel: "xqc", color: "#FF4500"))
        #expect(frame.room == "xqc")
    }

    @Test func theOlderOwnColorNamesNoRoom() {
        #expect(ChatSocketFrame.decode("USER_COLOR:#1E90FF") == .ownColor(channel: nil, color: "#1E90FF"))
        #expect(ChatSocketFrame.decode("USER_COLOR:#1E90FF").room == nil)
        // Nothing after the prefix is no color at all.
        #expect(ChatSocketFrame.decode("USER_COLOR:") == .status("USER_COLOR"))
    }

    // MARK: Moderation settings

    @Test func theModerationSettingsAreRead() {
        let style = ChatStyle(settingsJSON: Data(#"{"moderation":{"ignore_clear_chat":true,"show_mod_messages":true}}"#.utf8))
        #expect(style.ignoreClearChat && style.showModMessages)
        let plain = ChatStyle(settingsJSON: Data("{}".utf8))
        #expect(!plain.ignoreClearChat && !plain.showModMessages)
    }

    @Test func ignoringClearsLeavesEveryRowUnmarked() {
        var buffer = ChatRoomBuffer(cap: 100)
        buffer.ignoreClearChat = true
        _ = buffer.apply([row("a", user: "7"), row("b", user: "8")])
        let change = buffer.apply([
            .clearChat(channel: "xqc", targetUserID: "7", targetUser: "u7", duration: 600),
            .clearMessage(channel: "xqc", targetID: "b"),
        ])
        #expect(change.isEmpty)
        #expect(buffer.row("a")?.moderation == nil)
        #expect(buffer.row("b")?.moderation == nil)
    }

    @Test func modMessagesSayWhatAModeratorDid() {
        var buffer = ChatRoomBuffer(cap: 100)
        buffer.showModMessages = true
        _ = buffer.apply([row("a", user: "7", display: "Seven"), row("b", user: "8")])
        let change = buffer.apply([
            .clearChat(channel: "xqc", targetUserID: "7", targetUser: "seven", duration: 600),
            .clearChat(channel: "xqc", targetUserID: "8", targetUser: nil, duration: nil),
            .clearChat(channel: "xqc", targetUserID: nil, targetUser: nil, duration: nil),
            .clearMessage(channel: "xqc", targetID: "a"),
            .clearMessage(channel: "kick:foo", targetID: "zz", login: "bob", reason: "spam"),
        ])
        #expect(lines(change) == [
            "seven was timed out for 10m.",
            "A user was banned.",
            "Chat was cleared by a moderator.",
            "Seven's message was deleted by a moderator.",
            "bob's message was deleted (spam).",
        ])
        // The marks still land.
        #expect(buffer.row("a")?.moderation == .deleted)
        #expect(buffer.row("b")?.moderation == .banned)
    }

    @Test func modMessagesNeedTheirRoom() {
        var buffer = ChatRoomBuffer(cap: 100)
        buffer.showModMessages = true
        let change = buffer.apply([.clearChat(channel: nil, targetUserID: nil, targetUser: nil, duration: nil)])
        #expect(change.isEmpty)
    }

    @Test func timeoutLengthsRoundToOneUnit() {
        #expect(ChatRoomBuffer.shortDuration(45) == "45s")
        #expect(ChatRoomBuffer.shortDuration(90) == "2m")
        #expect(ChatRoomBuffer.shortDuration(3600) == "1h")
        #expect(ChatRoomBuffer.shortDuration(1_209_600) == "14d")
    }

    @Test func aDeleteCarriesItsAuthorAndReason() {
        let frame = ChatSocketFrame.decode(#"{"type":"CLEARMSG","channel":"kick:foo","target_msg_id":"m1","login":"bob","reason":"spam"}"#)
        #expect(frame == .clearMessage(channel: "kick:foo", targetID: "m1", login: "bob", reason: "spam"))
    }

    // MARK: Hidden users

    @Test func hiddenUsersNameTheirRoom() {
        let twitch = ChatSocketFrame.decode(#"{"type":"HIDE_USERS","channel_key":"twitch:XQC","logins":["Bob"]}"#)
        #expect(twitch == .hideUsers(room: "xqc", logins: ["Bob"]))
        #expect(twitch.room == "xqc")
        let kick = ChatSocketFrame.decode(#"{"type":"HIDE_USERS","channel_key":"kick:foo","logins":["bob"]}"#)
        #expect(kick.room == "kick:foo")
        let everywhere = ChatSocketFrame.decode(#"{"type":"HIDE_USERS","channel_key":null,"logins":["bob"]}"#)
        #expect(everywhere == .hideUsers(room: nil, logins: ["bob"]))
        #expect(everywhere.room == nil)
    }

    @Test func hidingAUserTakesTheirRowsAway() {
        var buffer = ChatRoomBuffer(cap: 100)
        _ = buffer.apply([row("a", user: "1", login: "bob"), row("b", user: "2", login: "amy"),
                          row("c", user: "3", login: "x", display: "BOB")])
        let change = buffer.apply([.hideUsers(room: "xqc", logins: ["Bob"]), row("d", user: "2", login: "amy")])
        #expect(change.reset)
        #expect(buffer.rows.map(\.id) == ["b", "d"])
        #expect(buffer.row("a") == nil && buffer.row("d")?.id == "d")
        let none = buffer.apply([.hideUsers(room: nil, logins: ["nobody"])])
        #expect(none.isEmpty)
    }

    // MARK: Personal emotes and pins

    @Test func aLatePersonalSetNamesItsRows() {
        let frame = ChatSocketFrame.decode(#"{"type":"PERSONAL_EMOTES","channel":"xqc","user_id":"9","message_ids":["m1","m2"]}"#)
        #expect(frame == .personalEmotes(channel: "xqc", userID: "9", messageIDs: ["m1", "m2"]))
        #expect(frame.room == "xqc")
    }

    @Test func aPlatformPinAndItsRemovalDecode() {
        let pinned = ChatSocketFrame.decode("""
        {"type":"PINNED","provider":"kick","channel":"kick:foo","pin":{"id":"p1","message_id":"p1","type":"kick",
         "message_text":"hello","sender_id":"5","sender_name":"amy","sender_color":"#ff0000"}}
        """)
        guard case .pinned(let channel, let pin) = pinned else {
            Issue.record("\(pinned)")
            return
        }
        #expect(channel == "kick:foo" && pinned.room == "kick:foo")
        #expect(pin?.text == "hello" && pin?.senderName == "amy")
        #expect(ChatSocketFrame.decode(#"{"type":"UNPINNED","provider":"kick","channel":"kick:foo"}"#)
            == .pinned(channel: "kick:foo", pin: nil))
    }

    // MARK: Mention paints

    @Test func chattersPaintsAreKeptForTheirMentions() {
        let paint = ChatPaint(id: "p", kind: "linear", stops: [.init(at: 0, color: "#ff0000"), .init(at: 1, color: "#0000ff")])
        var buffer = ChatRoomBuffer(cap: 1)
        _ = buffer.apply([row("a", login: "Bob", paint: paint), row("b", user: "2", login: "amy")])
        // Kept after the row scrolled away; a chatter with no paint has none.
        #expect(buffer.row("a") == nil)
        #expect(buffer.chatterPaints["bob"] == paint)
        #expect(buffer.chatterPaints["amy"] == nil)
    }
}
