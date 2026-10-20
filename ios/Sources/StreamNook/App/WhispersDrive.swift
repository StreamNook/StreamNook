#if DEBUG
import Foundation
import SNKit

/// Drive steps for Whispers. Every step that shows rows or writes runs on a fixture inbox (a
/// FakeCore of its own, `whispers:fake`), never the signed-in account's, and a send there never
/// leaves the fake:
///
/// - `whispers` opens Whispers on the You tab; `whisper:KEY` opens one conversation in its sheet
/// - `whispers:fake` points Whispers at the fixture inbox (unread, read, a long thread, a refusing
///   sender); `whispers:empty` at an empty one
/// - `whispernotify` files a new fixture whisper and posts its notification the way the core's
///   `whisper-notify` does (only with notifications allowed)
/// - `whispersend:TEXT` sends TEXT in the open fixture conversation; `whispersend:fail` sends to the
///   refusing fixture (the not sent state); `whispersend:hold` shows a send still on its way
/// - `whisperlog` logs the unread count and the rows held
@MainActor
enum WhispersDrive {
    static func run(_ name: String, _ value: String, model: AppModel) async -> Bool {
        switch name {
        case "whispers":
            switch value {
            case "": model.openWhispers()
            case "fake": await useFixtures(model, empty: false)
            case "empty": await useFixtures(model, empty: true)
            default: log.error("drive whispers: unknown \(value, privacy: .public)")
            }
        case "whisper":
            model.openWhisper(value)
        case "whispernotify":
            guard let fake = fixtureCore else {
                log.error("drive whispernotify: run whispers:fake first")
                return true
            }
            fake.simulateWhisper(from: "1104", text: "heads up, the giveaway starts in five minutes")
        case "whispersend":
            guard fixtureCore != nil else {
                // Never on the signed-in account.
                log.error("drive whispersend: run whispers:fake first")
                return true
            }
            switch value {
            case "fail":
                guard let row = model.whispers.row(FakeWhispers.refusingKey) else { return true }
                model.openWhisper(row.key)
                model.whispers.send("thanks again for the raid!", to: row)
            case "hold":
                guard let row = model.whispers.row("1101") else { return true }
                model.openWhisper(row.key)
                model.whispers.holdForDrive("on my way, five minutes", to: row)
            default:
                guard let key = model.whisperSheet?.key, let row = model.whispers.row(key) else {
                    log.error("drive whispersend: no fixture conversation open")
                    return true
                }
                model.whispers.send(value, to: row)
            }
        case "whisperlog":
            let list = model.whispers.list
            log.notice("drive whispers unread \(list.unreadTotal) rows \(list.conversations.count) pending \(model.whispers.pending.count)")
        default:
            return false
        }
        return true
    }

    private static var fixtureCore: FakeCore?

    private static func useFixtures(_ model: AppModel, empty: Bool) async {
        let fixtures = FakeFixtures(signedIn: true)
        let fake = FakeCore(fixtures: fixtures)
        try? fake.start(dirs: CoreDirs(data: NSTemporaryDirectory(), cache: NSTemporaryDirectory(), logs: NSTemporaryDirectory()),
                        version: "drive", vaultKey: Data(repeating: 7, count: 32), codecs: [])
        fixtureCore = empty ? nil : fake
        if empty {
            let blank = FakeCore(fixtures: FakeFixtures(signedIn: true, whispers: FakeWhispers(threads: [:])))
            try? blank.start(dirs: CoreDirs(data: NSTemporaryDirectory(), cache: NSTemporaryDirectory(), logs: NSTemporaryDirectory()),
                             version: "drive", vaultKey: Data(repeating: 7, count: 32), codecs: [])
            model.whispers.use(blank)
        } else {
            model.whispers.use(fake)
        }
        log.notice("drive whispers on fixtures\(empty ? " (empty)" : "", privacy: .public)")
    }
}
#endif
