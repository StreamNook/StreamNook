import Foundation
import Synchronization

/// FakeCore's whisper inbox: canned conversations that cover every state the screens draw
/// (unread, read, a long thread, one with no messages yet, a sender whose privacy settings refuse
/// a whisper), with the inbox commands' rules (newest first, read on open, a send recorded as
/// yours). Every change is announced the way the core announces it, through `onUpdate`. Nothing
/// leaves the fake: a send never reaches Twitch.
public final class FakeWhispers: Sendable {
    public static let owner = "424242"
    /// The fixture person whose settings refuse whispers, so the failed state can be drawn.
    public static let refusingKey = "1103"
    public static let refusal = "Cannot send: This user's privacy settings prevent them from receiving whispers from you."

    private struct State {
        var threads: [String: WhisperThread]
        var active: String?
        var sent: [String] = []
        var seq = 0
    }

    private let state: Mutex<State>
    /// Called with every change, as `whisper-conversation-updated`.
    let onUpdate = Mutex<(@Sendable (WhisperUpdate) -> Void)?>(nil)

    public init(threads: [String: WhisperThread]? = nil) {
        state = Mutex(State(threads: threads ?? Self.sample()))
    }

    /// Texts handed to `whisper_send`, oldest first.
    public var sentTexts: [String] { state.withLock { $0.sent } }

    public static func sample(now: Date = Date()) -> [String: WhisperThread] {
        let ms = Int64(now.timeIntervalSince1970 * 1000)
        let minute: Int64 = 60_000
        func theirs(_ id: String, _ from: (String, String, String), _ text: String, _ at: Int64) -> WhisperMessage {
            WhisperMessage(id: id, fromUserID: from.0, fromUserLogin: from.1, fromUserName: from.2,
                           toUserID: owner, toUserLogin: "nookviewer", toUserName: "NookViewer", message: text,
                           timestamp: at, isSent: false)
        }
        func mine(_ id: String, _ to: (String, String, String), _ text: String, _ at: Int64) -> WhisperMessage {
            WhisperMessage(id: id, fromUserID: owner, fromUserLogin: "nookviewer", fromUserName: "NookViewer",
                           toUserID: to.0, toUserLogin: to.1, toUserName: to.2, message: text, timestamp: at, isSent: true)
        }
        let owl = ("1101", "glassowl", "GlassOwl")
        let pine = ("1102", "pixelpine", "PixelPine")
        let shy = ("1103", "quietfern", "QuietFern")
        let mod = ("1104", "midnightmod", "MidnightMod")
        // A long thread: two days of back and forth, runs of several lines.
        var long: [WhisperMessage] = []
        let lines = ["did you see the raid last night", "yeah it was wild", "the whole chat lost it",
                     "are you coming to the watch party", "probably, what time", "eight my time",
                     "ok I'll be there", "bring the emote ideas", "on it", "see you there"]
        for (i, line) in lines.enumerated() {
            let at = ms - Int64(26 * 60 - i * 9) * minute
            long.append(i % 3 == 1 ? mine("fw-p\(i)", pine, line, at) : theirs("fw-p\(i)", pine, line, at))
        }
        for i in 0..<14 {
            let at = ms - Int64(40 - i * 2) * minute
            let text = i == 6
                ? "also, the overlay you set up for the tournament looked great on stream, the bracket card especially, everyone in chat was asking where it came from"
                : "quick one \(i + 1)"
            long.append(i % 4 == 3 ? mine("fw-q\(i)", pine, text, at) : theirs("fw-q\(i)", pine, text, at))
        }
        return [
            owl.0: WhisperThread(userID: owl.0, userLogin: owl.1, userName: owl.2, messages: [
                theirs("fw-o1", owl, "hey! are you streaming tonight?", ms - 6 * minute),
                theirs("fw-o2", owl, "I have the new layout ready to show you", ms - 5 * minute),
            ], lastMessageTimestamp: ms - 5 * minute, unreadCount: 2),
            pine.0: WhisperThread(userID: pine.0, userLogin: pine.1, userName: pine.2, messages: long,
                                  lastMessageTimestamp: long.last?.timestamp ?? ms, unreadCount: 0),
            shy.0: WhisperThread(userID: shy.0, userLogin: shy.1, userName: shy.2, messages: [
                mine("fw-s1", shy, "thanks for the raid earlier!", ms - 3 * 60 * minute),
            ], lastMessageTimestamp: ms - 3 * 60 * minute, unreadCount: 0),
            mod.0: WhisperThread(userID: mod.0, userLogin: mod.1, userName: mod.2, messages: [
                theirs("fw-m1", mod, "your message got caught by AutoMod, I approved it", ms - 3 * 24 * 60 * minute),
                mine("fw-m2", mod, "thank you!", ms - 3 * 24 * 60 * minute + minute),
            ], lastMessageTimestamp: ms - 3 * 24 * 60 * minute + minute, unreadCount: 0),
        ]
    }

    private static func row(_ key: String, _ t: WhisperThread) -> WhisperRow {
        let last = t.messages.last
        return WhisperRow(key: key, userID: t.userID, userLogin: t.userLogin, userName: t.userName,
                          profileImageURL: t.profileImageURL, lastMessageTimestamp: t.lastMessageTimestamp,
                          unreadCount: t.unreadCount, lastText: last?.message ?? "", lastIsSent: last?.isSent ?? false)
    }

    /// Newest first, as the core sorts the list; a tie keeps a fixed order.
    private static func newestFirst(_ a: WhisperRow, _ b: WhisperRow) -> Bool {
        if a.lastMessageTimestamp != b.lastMessageTimestamp { return a.lastMessageTimestamp > b.lastMessageTimestamp }
        return a.key < b.key
    }

    private static func meta(_ t: WhisperThread) -> WhisperUpdate.Meta {
        .init(userID: t.userID, userLogin: t.userLogin, userName: t.userName, profileImageURL: t.profileImageURL,
              lastMessageTimestamp: t.lastMessageTimestamp, unreadCount: t.unreadCount)
    }

    public var unreadTotal: Int { state.withLock { $0.threads.values.reduce(0) { $0 + $1.unreadCount } } }

    /// Files an incoming whisper as the socket would (read at once in the open conversation) and
    /// returns the change, for the drive to announce as `whisper-notify`.
    @discardableResult
    public func receive(from key: String, text: String) -> WhisperUpdate? {
        let update = state.withLock { s -> WhisperUpdate? in
            guard var t = s.threads[key] else { return nil }
            s.seq += 1
            let at = Int64(Date().timeIntervalSince1970 * 1000)
            let m = WhisperMessage(id: "fw-in-\(s.seq)-\(at)", fromUserID: t.userID, fromUserLogin: t.userLogin,
                                   fromUserName: t.userName, toUserID: Self.owner, message: text, timestamp: at, isSent: false)
            t.messages.append(m)
            t.lastMessageTimestamp = at
            if s.active != key { t.unreadCount += 1 }
            s.threads[key] = t
            return WhisperUpdate(ownerID: Self.owner, key: key, meta: Self.meta(t), message: m)
        }
        if let update { announce(update) }
        return update
    }

    private func announce(_ update: WhisperUpdate) {
        onUpdate.withLock { $0 }?(update)
    }

    /// The answer to a whisper command, or nil when `cmd` is not one.
    func answer(_ cmd: String, _ args: [String: Any]) throws -> String? {
        switch cmd {
        case "whisper_list":
            let threads: [String: WhisperThread] = state.withLock { $0.threads }
            let rows: [WhisperRow] = threads.map { Self.row($0.key, $0.value) }.sorted(by: Self.newestFirst)
            let unread: Int = rows.reduce(0) { $0 + $1.unreadCount }
            return try Self.encode(WhisperList(ownerID: Self.owner, conversations: rows, unreadTotal: unread))
        case "whisper_thread":
            let key = args["key"] as? String ?? ""
            guard let thread = state.withLock({ $0.threads[key] }) else { return "null" }
            return try Self.encode(thread)
        case "whisper_mark_read":
            let key = args["key"] as? String ?? ""
            state.withLock { s in
                if s.threads[key]?.unreadCount ?? 0 > 0 { s.threads[key]?.unreadCount = 0 }
            }
            return "null"
        case "whisper_set_active":
            state.withLock { $0.active = args["key"] as? String }
            return "null"
        case "whisper_send":
            let target = args["target"] as? [String: Any] ?? [:]
            let key = target["key"] as? String ?? ""
            let text = args["text"] as? String ?? ""
            if key == Self.refusingKey {
                throw CoreFailure.command(json: try Self.encodeString(Self.refusal))
            }
            let update = state.withLock { s -> WhisperUpdate? in
                guard var t = s.threads[key] else { return nil }
                s.sent.append(text)
                s.seq += 1
                let at = Int64(Date().timeIntervalSince1970 * 1000)
                let m = WhisperMessage(id: "sent-\(at)-\(s.seq)", fromUserID: Self.owner, fromUserLogin: "nookviewer",
                                       fromUserName: "NookViewer", toUserID: t.userID, toUserLogin: t.userLogin,
                                       toUserName: t.userName, message: text, timestamp: at, isSent: true)
                t.messages.append(m)
                t.lastMessageTimestamp = at
                s.threads[key] = t
                return WhisperUpdate(ownerID: Self.owner, key: key, meta: Self.meta(t), message: m)
            }
            guard let update else { throw CoreFailure.command(json: #""Conversation vanished while sending""#) }
            return try Self.encode(update)
        default:
            return nil
        }
    }

    private static func encode(_ value: some Encodable) throws -> String {
        String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
    }

    private static func encodeString(_ text: String) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: text, options: [.fragmentsAllowed]), as: UTF8.self)
    }
}
