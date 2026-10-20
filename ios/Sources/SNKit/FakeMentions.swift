import Foundation
import Synchronization

/// FakeCore's mentions inbox: a canned set that covers every row state (unread and read, each
/// kind, a moderator-deleted message, a long message, a Kick room so platform logos show), with
/// the inbox commands' rules (newest first, paging by `beforeMs`, open marks read). Every change
/// is announced the way the core announces it, through `onChange`. Nothing leaves the fake.
public final class FakeMentions: Sendable {
    private let entries: Mutex<[MentionEntry]>
    /// Called with the unread total after every change, as `mention-inbox-changed`.
    let onChange = Mutex<(@Sendable (Int) -> Void)?>(nil)

    public init(entries: [MentionEntry]? = nil) {
        self.entries = Mutex(entries ?? Self.sample())
    }

    /// The canned inbox, newest first, relative to `now`.
    public static func sample(now: Date = Date()) -> [MentionEntry] {
        let ms = Int64(now.timeIntervalSince1970 * 1000)
        let minute: Int64 = 60_000
        return [
            MentionEntry(id: "fm-1", room: "nookstreamer", openKey: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242", senderID: "1001",
                         senderLogin: "glassowl", senderName: "GlassOwl", senderColor: "#2dd4bf",
                         text: "@NookViewer did you catch that clutch at the end?", kind: .mention, atMs: ms - 2 * minute),
            MentionEntry(id: "fm-2", room: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242", senderID: "1002",
                         senderLogin: "pixelpine", senderName: "PixelPine", senderColor: "#f97316",
                         text: "yeah the new layout is way easier to read", kind: .reply, replyParentID: "own-1",
                         atMs: ms - 14 * minute),
            MentionEntry(id: "fm-3", platform: "kick", room: "kick:latebloomer", openKey: "kick:LateBloomer",
                         channelLogin: "LateBloomer", senderID: "77",
                         senderLogin: "kappakid", senderName: "kappakid", senderColor: "#a78bfa",
                         text: "nookviewer you have to try the speedrun category next, it is so much faster than it looks on paper and the community is great, honestly one of the friendliest chats on the site",
                         kind: .mention, atMs: ms - 52 * minute),
            MentionEntry(id: "fm-4", room: "midnightmod", channelLogin: "midnightmod", channelID: "5150", senderID: "1004",
                         senderLogin: "midnightmod", senderName: "MidnightMod", senderColor: "#4ade80",
                         text: "giveaway at 9", kind: .word, atMs: ms - 3 * 60 * minute, read: true),
            MentionEntry(id: "fm-5", room: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242", senderID: "1005",
                         senderLogin: "latebloomer", senderName: "LateBloomer", senderColor: nil,
                         text: "something a moderator removed", kind: .mention, atMs: ms - 26 * 60 * minute, read: true,
                         removed: true),
            MentionEntry(id: "fm-6", room: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242", senderID: "1001",
                         senderLogin: "glassowl", senderName: "GlassOwl", senderColor: "#2dd4bf",
                         text: "@nookviewer welcome back!", kind: .mention, atMs: ms - 4 * 24 * 60 * minute, read: true),
            // A YouTube chat: the iPhone opens the channel's own page for it.
            MentionEntry(id: "fm-7", platform: "youtube", room: "youtube:@nookcasts", openKey: "youtube:@NookCasts",
                         watchURL: "https://www.youtube.com/@NookCasts/live", channelLogin: "@NookCasts", senderID: "UCfake7",
                         senderLogin: "tidepool", senderName: "Tidepool", senderColor: "#60a5fa",
                         text: "nookviewer is here too!", kind: .mention, atMs: ms - 6 * 24 * 60 * minute, read: true),
            // A bot saying the same line three times in a row: the inbox folds them into one row.
            MentionEntry(id: "fm-8", room: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242", senderID: "1008",
                         senderLogin: "botwatcher", senderName: "BotWatcher", senderColor: "#f5a742",
                         text: "@nookviewer hi", kind: .mention, atMs: ms - 7 * 24 * 60 * minute, read: true),
            MentionEntry(id: "fm-9", room: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242", senderID: "1008",
                         senderLogin: "botwatcher", senderName: "BotWatcher", senderColor: "#f5a742",
                         text: "@nookviewer hi", kind: .mention, atMs: ms - 7 * 24 * 60 * minute - minute, read: true),
            MentionEntry(id: "fm-10", room: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242", senderID: "1008",
                         senderLogin: "botwatcher", senderName: "BotWatcher", senderColor: "#f5a742",
                         text: "@nookviewer hi", kind: .mention, atMs: ms - 7 * 24 * 60 * minute - 2 * minute, read: true),
        ]
    }

    public var unreadTotal: Int { entries.withLock { $0.filter { !$0.read }.count } }

    /// Files one more entry, as a live mention would.
    public func file(_ entry: MentionEntry) {
        entries.withLock { list in
            guard !list.contains(where: { $0.id == entry.id }) else { return }
            list.append(entry)
            list.sort { $0.atMs > $1.atMs }
        }
        changed()
    }

    private func changed() {
        let total = unreadTotal
        onChange.withLock { $0 }?(total)
    }

    /// The answer to an inbox command, or nil when `cmd` is not one.
    func answer(_ cmd: String, _ args: [String: Any]) throws -> String? {
        switch cmd {
        case "get_mention_inbox":
            let before = Self.int(args["beforeMs"])
            let limit = Int(min(max(Self.int(args["limit"]) ?? 50, 1), 200))
            let (page, more, unread) = entries.withLock { list -> ([MentionEntry], Bool, Int) in
                let older = list.filter { before == nil || $0.atMs < before! }
                return (Array(older.prefix(limit)), older.count > limit, list.filter { !$0.read }.count)
            }
            return try Self.encode(["entries": page.map(Self.object), "has_more": more, "unread_total": unread] as [String: Any])
        case "mark_mentions_read":
            let ids = (args["ids"] as? [String]).map { Set($0) }
            let changed = entries.withLock { list -> Bool in
                var any = false
                for i in list.indices where !list[i].read && (ids?.contains(list[i].id) ?? true) {
                    list[i].read = true
                    any = true
                }
                return any
            }
            if changed { self.changed() }
            return "null"
        case "remove_mention":
            let id = args["id"] as? String ?? ""
            let removed = entries.withLock { list -> Bool in
                let before = list.count
                list.removeAll { $0.id == id }
                return list.count != before
            }
            if removed { changed() }
            return "null"
        case "clear_mention_inbox":
            let had = entries.withLock { list -> Bool in
                defer { list.removeAll() }
                return !list.isEmpty
            }
            if had { changed() }
            return "null"
        case "open_mention":
            let id = args["id"] as? String ?? ""
            let (found, wasUnread) = entries.withLock { list -> (MentionEntry?, Bool) in
                guard let i = list.firstIndex(where: { $0.id == id }) else { return (nil, false) }
                let unread = !list[i].read
                list[i].read = true
                return (list[i], unread)
            }
            if wasUnread { changed() }
            guard let found else { return "null" }
            return try Self.encode(Self.object(found))
        case "set_visible_mention_rooms":
            return "null"
        case "mention_watch_pull":
            return "0"
        case "mention_push_arrived":
            return "true"
        case "get_inbox_view":
            let filter = args["filter"] as? String ?? "all"
            let before = Self.int(args["beforeMs"])
            let limit = Int(min(max(Self.int(args["limit"]) ?? 50, 1), 200))
            let list = entries.withLock { $0 }
            let (rows, more, cursor) = Self.view(list, filter: filter, before: before, limit: limit)
            let chips: [[String: Any]] = ["all", "mentions", "highlights"].map { chip in
                ["id": chip, "unread": list.filter { !$0.read && !$0.removed && Self.admits(chip, $0.kind) }.count]
            }
            let rowObjects: [Any] = rows.map { row in
                [
                    "entry": Self.object(row.entry),
                    "folded": row.folded.map { ["id": $0.id, "at_ms": $0.atMs, "read": $0.read] as [String: Any] },
                    "unread": row.unread,
                    "avatar": NSNull(),
                ] as [String: Any]
            }
            return try Self.encode([
                "rows": rowObjects, "has_more": more, "before_ms": cursor.map { $0 as Any } ?? NSNull(),
                "unread_total": list.filter { !$0.read && !$0.removed }.count, "chips": chips,
            ] as [String: Any])
        default:
            return nil
        }
    }

    static func admits(_ filter: String, _ kind: MentionEntry.Kind) -> Bool {
        switch filter {
        case "mentions": kind == .mention || kind == .reply
        case "highlights": kind == .word
        default: true
        }
    }

    /// The core's view rules (`Inbox::view`) over the fixture rows: newest first, the chip's
    /// kinds, identical messages in a row folded, pages that never split a fold.
    static func view(_ list: [MentionEntry], filter: String, before: Int64?, limit: Int)
        -> ([InboxRowView], Bool, Int64?) {
        let shown = list.sorted { $0.atMs > $1.atMs }
            .filter { admits(filter, $0.kind) && (before == nil || $0.atMs < before!) }
        func same(_ a: MentionEntry, _ b: MentionEntry) -> Bool {
            !a.removed && !b.removed && a.kind == b.kind && a.room == b.room
                && (a.senderID.isEmpty ? a.senderLogin.lowercased() : a.senderID)
                == (b.senderID.isEmpty ? b.senderLogin.lowercased() : b.senderID)
                && a.text.lowercased().split(separator: " ") == b.text.lowercased().split(separator: " ")
        }
        var rows: [InboxRowView] = []
        var cursor: Int64?
        var i = 0
        while i < shown.count {
            let head = shown[i]
            var row = InboxRowView(entry: head, unread: !head.read && !head.removed)
            cursor = head.atMs
            i += 1
            while i < shown.count, same(head, shown[i]) {
                row.folded.append(InboxFolded(id: shown[i].id, atMs: shown[i].atMs, read: shown[i].read))
                row.unread = row.unread || (!shown[i].read && !shown[i].removed)
                cursor = shown[i].atMs
                i += 1
            }
            rows.append(row)
            if rows.count >= limit { return (rows, i < shown.count, cursor) }
        }
        return (rows, false, cursor)
    }

    /// A JSON number, however this platform's JSONSerialization boxed it.
    static func int(_ value: Any?) -> Int64? {
        switch value {
        case let v as Int: Int64(v)
        case let v as Int64: v
        case let v as Double: Int64(v)
        case let v as NSNumber: v.int64Value
        default: nil
        }
    }

    static func object(_ entry: MentionEntry) -> Any {
        guard let data = try? JSONEncoder().encode(entry), let object = try? JSONSerialization.jsonObject(with: data) else {
            return NSNull()
        }
        return object
    }

    private static func encode(_ value: Any) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .fragmentsAllowed])
        return String(decoding: data, as: UTF8.self)
    }
}
