#if DEBUG
import Foundation
import SNKit

/// Drive steps for Mentions. Every step that shows rows or writes runs on a fixture inbox (a
/// FakeCore of its own, `mentions:fake`), never the signed-in account's:
///
/// - `mentions` pushes Mentions on the You tab; `mentions:sheet` opens it as the chat menu does
/// - `mentions:fake` points Mentions at the fixture inbox (unread, read, removed, a long message,
///   a Kick room); `mentions:empty` at an empty one
/// - `mentions:readall`, `mentions:read:ID`, `mentions:remove:ID`, `mentions:clearask` (the Clear
///   all confirmation)
/// - `mentionnotify` files a new fixture mention and posts its notification the way the core's
///   `mention-notify` does (only with notifications allowed)
/// - `mentioncard` pins an old fixture mention over the chat on screen ("Older than chat
///   history"); `mentioncard:close` dismisses it
/// - `mentionjump` brings the chat's tenth newest row into view as a mention open does
/// - `mentionlog` logs the unread count and the rows held
/// - `inbox:open` / `inbox:open:chats` opens the inbox sheet over the chat under the player (or the
///   Chats tab's), as the tray's tap does; `inbox:close` closes it
/// - `inbox:arrive` shows a new mention's card over the message bar (on fixtures, through the
///   fake core's `mention-arrived`, so the card takes the real event path)
/// - `inboxlog` logs the sheet, the card, the chat on screen and the unread count
@MainActor
enum MentionsDrive {
    static func run(_ name: String, _ value: String, model: AppModel) async -> Bool {
        switch name {
        case "mentions":
            let (what, rest) = split(value)
            switch what {
            case "": model.nav.tab = .you; model.nav.push(.mentions)
            case "sheet": model.chat.mentionsOpen = true
            case "fake": await useFixtures(model, empty: false)
            case "empty": await useFixtures(model, empty: true)
            case "readall": await model.mentions.markRead(nil)
            case "read": await model.mentions.markRead([rest])
            case "remove": await model.mentions.remove(rest)
            case "clearask": model.mentions.confirmingClear = true
            default: log.error("drive mentions: unknown \(what, privacy: .public)")
            }
        case "mentionnotify":
            guard let fake = fixtureCore else {
                log.error("drive mentionnotify: run mentions:fake first")
                return true
            }
            let ms = Int64(Date().timeIntervalSince1970 * 1000)
            fake.simulateMention(MentionEntry(
                id: "fm-live-\(ms)", room: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242", senderID: "1006",
                senderLogin: "midnightmod", senderName: "MidnightMod", senderColor: "#4ade80",
                text: "@NookViewer you are up next in the viewer games, get ready", kind: .mention, atMs: ms))
        case "mentioncard":
            guard let room = model.chat.activeSession?.channel else {
                log.error("drive mentioncard: no chat on screen")
                return true
            }
            if value == "close" {
                model.chat.dismissMentionCard(room)
            } else {
                let ms = Int64(Date().timeIntervalSince1970 * 1000)
                let channel = MentionText.channel(ofRoom: room)
                model.chat.mentionCards[room] = MentionEntry(
                    id: "fm-old", platform: MentionText.platform(ofRoom: room), room: room, channelLogin: channel,
                    senderLogin: "glassowl", senderName: "GlassOwl", senderColor: "#2dd4bf",
                    text: "@NookViewer that build you suggested yesterday worked, thank you!", kind: .reply,
                    atMs: ms - 26 * 3_600_000, read: true)
            }
        case "mentionjump":
            guard let session = model.chat.activeSession,
                  let target = session.pipeline.recentMessages(10).first else {
                log.error("drive mentionjump: no rows on screen")
                return true
            }
            model.chat.focus(MentionEntry(id: target.id, room: session.channel, channelLogin: session.login,
                                          senderLogin: target.username, senderName: target.displayName,
                                          text: target.content, atMs: 0, read: true))
        case "mentionlog":
            let list = model.mentions.list
            log.notice("drive mentions unread \(list.unreadTotal) rows \(list.entries.count) more \(list.hasMore)")
        case "inbox":
            let store = model.mentions
            switch value {
            case "open": store.sheetIn = .watch
            case "open:chats": store.sheetIn = .chats
            case "close": store.sheetIn = nil
            case "arrive":
                let ms = Int64(Date().timeIntervalSince1970 * 1000)
                let entry = MentionEntry(
                    id: "fm-arrive-\(ms)", room: "nookstreamer", channelLogin: "nookstreamer", channelID: "4242",
                    senderID: "1001", senderLogin: "glassowl", senderName: "GlassOwl", senderColor: "#2dd4bf",
                    text: "@NookViewer are you coming to the watch party tonight?", kind: .mention, atMs: ms)
                if let fake = fixtureCore {
                    fake.simulateMention(entry)
                } else {
                    store.showArrival(MentionArrival(entry: entry, avatar: nil))
                }
            default: log.error("drive inbox: unknown \(value, privacy: .public)")
            }
        case "inboxlog":
            let store = model.mentions
            log.notice("drive inbox sheet \(String(describing: store.sheetIn), privacy: .public) card \(store.arrival?.arrival.entry.id ?? "none", privacy: .public) on screen \(String(describing: store.onScreen), privacy: .public) unread \(store.unreadTotal)")
        default:
            return false
        }
        return true
    }

    private static var fixtureCore: FakeCore?

    private static func useFixtures(_ model: AppModel, empty: Bool) async {
        let fake = FakeCore(fixtures: FakeFixtures(signedIn: true))
        try? fake.start(dirs: CoreDirs(data: NSTemporaryDirectory(), cache: NSTemporaryDirectory(), logs: NSTemporaryDirectory()),
                        version: "drive", vaultKey: Data(repeating: 7, count: 32), codecs: [])
        if empty { _ = try? await fake.invoke("clear_mention_inbox", args: [:]) }
        fixtureCore = fake
        model.mentions.use(fake)
        log.notice("drive mentions on fixtures\(empty ? " (empty)" : "", privacy: .public)")
    }

    private static func split(_ step: String) -> (String, String) {
        guard let colon = step.firstIndex(of: ":") else { return (step, "") }
        return (String(step[..<colon]), String(step[step.index(after: colon)...]))
    }
}
#endif
