import Foundation
import Synchronization

/// FakeCore's Rewards answers: a drops connection, campaigns in progress and running, drops
/// earned. Writes (a claim) are only recorded, so a drive can show every state without touching
/// a real account.
public final class FakeRewards: Sendable {
    private struct State {
        var connected = true
        var claims: [String] = []
    }

    private let state = Mutex(State())

    public init() {}

    /// Whether a Drops sign-in is stored.
    public var connected: Bool {
        get { state.withLock { $0.connected } }
        set { state.withLock { $0.connected = newValue } }
    }

    /// Drop ids handed to `claim_drop`, oldest first.
    public var claims: [String] { state.withLock { $0.claims } }

    func answer(_ cmd: String, _ args: [String: Any]) throws -> String? {
        switch cmd {
        case "is_drops_authenticated":
            return connected ? "true" : "false"
        case "open_drops_sheet":
            connected = true
            return "null"
        case "get_rewards_drops":
            guard connected else { return try FakeCore.json(["connected": false, "query": "", "in_progress": [Any](),
                                                             "available": [Any](), "earned": [Any](),
                                                             "in_progress_empty": ""] as [String: Any]) }
            return try FakeCore.json(Self.drops(query: (args["query"] as? String ?? "").trimmingCharacters(in: .whitespaces)))
        case "get_badge_wall":
            return try FakeCore.json(Self.wall(sort: args["sort"] as? String ?? "newest"))
        case "get_watch_drop":
            guard connected else { return "null" }
            let ready = claims.isEmpty
            return try FakeCore.json([
                "campaign_id": "c1", "drop_id": ready ? "d2" : "d3", "instance_id": "inst", "name": ready ? "Hoodie" : "Pants",
                "image": "", "trailing": ready ? "Ready to claim" : "108m left", "line": "Rust Twitch Drops Round 41 · \(ready ? 2 : 3) of 4",
                "percent": ready ? 100.0 : 40.0, "complete": ready, "claim_label": "Claim",
                "spoken": ready ? "Hoodie, ready to claim" : "Pants, 108 minutes left",
            ] as [String: Any])
        case "claim_drop":
            state.withLock { $0.claims.append(args["dropId"] as? String ?? "") }
            return "null"
        default:
            return nil
        }
    }

    private static let art = "https://static-cdn.jtvnw.net/ttv-boxart/"

    private static func drop(_ id: String, _ name: String, need: Int, have: Int, claimed: Bool = false,
                             detail: String? = nil) -> [String: Any] {
        let done = claimed || (need > 0 && have >= need)
        let status = claimed ? "Claimed" : done ? "Ready" : need > 0 ? "\(need - min(have, need))m left" : ""
        return ["id": id, "name": name, "image": "", "claimed": claimed, "done": done, "status": status,
                "detail": claimed ? NSNull() : (detail.map { $0 as Any } ?? NSNull()),
                "bar": need > 0 ? (claimed ? 100.0 : min(100.0, Double(have) / Double(need) * 100)) as Any : NSNull()]
    }

    private static func card(_ id: String, _ name: String, game: String, gameID: String, boxArt: String,
                             claimed: String = "", percent: Double = 0, detail: String, link: Bool = false,
                             channels: [String] = [], drops: [[String: Any]]) -> [String: Any] {
        [
            "id": id, "name": name, "game_id": gameID, "game_name": game, "art": "",
            "claimed_label": claimed, "percent": percent, "detail": detail, "needs_link": link,
            "link_url": link ? "https://example.com/link" : NSNull(),
            "meta_label": claimed.isEmpty ? "\(drops.count) rewards" : "\(claimed) claimed",
            "channels": channels.enumerated().map { ["id": "\(9000 + $0.offset)", "name": $0.element] },
            "channels_label": channels.isEmpty ? "" : "Earn on \(channels.count) channel\(channels.count == 1 ? "" : "s")",
            "find_stream": ["id": gameID, "name": game, "box_art_url": boxArt] as [String: Any],
            "drops": drops,
        ]
    }

    private static func group(_ section: String, _ game: String, art: String, _ campaigns: [[String: Any]]) -> [String: Any] {
        ["key": "\(section):\(game)", "game": game, "art": art,
         "count_label": "\(campaigns.count) campaign\(campaigns.count == 1 ? "" : "s")", "campaigns": campaigns]
    }

    private static let badgeCDN = "https://static-cdn.jtvnw.net/badges/v1/"

    private static func tile(_ id: String, _ title: String, image: String, owned: Bool = false, status: String? = nil,
                             usage: Int = 0) -> [String: Any] {
        let (word, sheet): (String, String) = owned ? ("OWNED", "OWNED")
            : status == "available" ? ("AVAILABLE", "AVAILABLE NOW")
            : status == "coming-soon" ? ("SOON", "COMING SOON")
            : status == "expired" ? ("ENDED", "NO LONGER EARNABLE") : ("", "")
        return [
            "key": "\(id)-1", "keys": ["\(id)/1"], "title": title, "image": badgeCDN + image + "/3",
            "owned": owned, "status": status.map { $0 as Any } ?? NSNull(), "word": word, "sheet_word": sheet,
            "prose": [["text": "Watch 30 minutes of a participating stream during the event.", "ms": NSNull()]],
            "window": status == nil ? [Any]() : [["text": NSNull(), "ms": 1_791_000_000_000], ["text": " to Oct 30", "ms": NSNull()]],
            "caveat": status == "available" ? "Prime subs don't count" as Any : NSNull(),
            "usage_label": usage > 0 ? "\(usage.formatted()) users have this badge" as Any : NSNull(),
            "info_url": "https://example.com/badge",
        ]
    }

    static func wall(sort: String) -> [String: Any] {
        var tiles: [[String: Any]] = [
            tile("glhf", "GLHF Pledge", image: "3158e758-3cb4-43c5-94b3-7639810451c5", owned: true, usage: 120_442),
            tile("twitchcon", "TwitchCon 2026", image: "e33d8b46-f63b-4e67-996d-4a7dcec0ad33", status: "available", usage: 8_210),
            tile("hype", "Hype Train Conductor", image: "fae4086c-3190-44d4-83c8-8ef0cbe1a515", status: "coming-soon"),
            tile("gold", "Golden Kappa", image: "c7d6b8fe-0e5c-4b30-8f9c-c4d1a1a0b5b1", status: "expired"),
            tile("sub", "Sub Gifter", image: "a5ef6c17-2e5b-4d8f-9b80-2779fd722414", owned: true, usage: 2_000_000),
            tile("bits", "Bits Leader", image: "8bedf8c3-7a6d-4df2-b62f-791b96a5dd31"),
            tile("mouse", "Mouse Pad", image: "4a5d5b74-2c3e-4b67-9e1e-4cbe1c44ce4a", status: "available"),
        ]
        if sort == "available" {
            tiles.sort { ($0["status"] as? String == "available" ? 0 : 1) < ($1["status"] as? String == "available" ? 0 : 1) }
        } else if sort == "oldest" {
            tiles.reverse()
        }
        let chip: (String, String, String) -> [String: Any] = { kind, text, hint in
            ["kind": kind, "ticket": kind == "purchase", "text": text, "hint": hint, "icon_url": NSNull()]
        }
        return [
            "loading": false, "tiles": tiles, "count_label": "2 of 7 collected", "sync_left": 0,
            "next_change_ms": NSNull(),
            "missing": [
                "state": "list", "refreshing": false, "closes": "in 3d",
                "catch_up": ["label": "To catch up, at least",
                             "stats": [["value": "$5.99", "label": "in 1 sub"], ["value": "1h 30m", "label": "of watching"]],
                             "notes": ["Subs and watch time in the same category count toward every badge there, so they are counted once. Subs at the US price of $5.99.",
                                       "One badge is drawn at random, so it may take more."]] as [String: Any],
                "rows": [
                    ["key": "twitchcon/1", "title": "TwitchCon 2026", "image": badgeCDN + "e33d8b46-f63b-4e67-996d-4a7dcec0ad33/3",
                     "chips": [chip("purchase", "Ticket", "Granted with a ticket purchase")], "icons_only": false,
                     "random_note": NSNull(), "time_left": "3d left"],
                    ["key": "mouse/1", "title": "Mouse Pad", "image": badgeCDN + "4a5d5b74-2c3e-4b67-9e1e-4cbe1c44ce4a/3",
                     "chips": [chip("subscribe", "Sub", "Subscribe or gift a subscription"), chip("watch", "30 min", "Watch 30 min")],
                     "icons_only": false, "random_note": "1 of 3 at random", "time_left": "5h left"],
                ],
            ] as [String: Any],
        ]
    }

    static func drops(query: String) -> [String: Any] {
        let rustArt = art + "263490_IGDB-144x192.jpg"
        let valArt = art + "516575-144x192.jpg"
        let apexArt = art + "511224-144x192.jpg"
        var progress = [
            group("progress", "Rust", art: rustArt, [
                card("c1", "Rust Twitch Drops Round 41", game: "Rust", gameID: "263490", boxArt: rustArt,
                     claimed: "1/4", percent: 42, detail: "4 rewards · 3d left",
                     channels: ["Shroud", "Hjune", "Willjum"],
                     drops: [drop("d1", "Garage Door", need: 60, have: 60, claimed: true),
                             drop("d2", "Hoodie", need: 120, have: 72),
                             drop("d3", "Pants", need: 180, have: 72),
                             drop("d4", "Rock Skin", need: 240, have: 72)]),
                card("c2", "Rust Streamer Drops", game: "Rust", gameID: "263490", boxArt: rustArt,
                     claimed: "0/2", percent: 15, detail: "2 rewards · 20h left",
                     drops: [drop("d5", "Sleeping Bag", need: 120, have: 18),
                             drop("d6", "Door", need: 0, have: 0, detail: "Subscribe or gift a sub")]),
            ]),
            group("progress", "VALORANT", art: valArt, [
                card("c3", "Champions Watch Party", game: "VALORANT", gameID: "516575", boxArt: valArt,
                     claimed: "0/3", percent: 66, detail: "3 rewards · 5d left", link: true,
                     drops: [drop("d7", "Gun Buddy", need: 30, have: 30),
                             drop("d8", "Spray", need: 60, have: 40, detail: "Day 2 of 3 · 5/20 min today · resets in 23h"),
                             drop("d9", "Player Card", need: 90, have: 40)]),
            ]),
        ]
        var available = [
            group("available", "Apex Legends", art: apexArt, [
                card("c4", "Apex Legends Split 2", game: "Apex Legends", gameID: "511224", boxArt: apexArt,
                     detail: "2 rewards · 2d left",
                     drops: [drop("d10", "Charm", need: 60, have: 0), drop("d11", "Banner", need: 120, have: 0)]),
            ]),
        ]
        if !query.isEmpty {
            let q = query.lowercased()
            let keep: ([String: Any]) -> [String: Any]? = { group in
                let gameMatch = (group["game"] as? String ?? "").lowercased().contains(q)
                let campaigns = (group["campaigns"] as? [[String: Any]] ?? []).filter {
                    gameMatch || ($0["name"] as? String ?? "").lowercased().contains(q)
                }
                guard !campaigns.isEmpty else { return nil }
                var out = group
                out["campaigns"] = campaigns
                out["count_label"] = "\(campaigns.count) campaign\(campaigns.count == 1 ? "" : "s")"
                return out
            }
            progress = progress.compactMap(keep)
            available = available.compactMap(keep)
        }
        return [
            "connected": true, "query": query, "in_progress": progress, "available": available,
            "earned": [
                ["id": "e1", "name": "Garage Door", "image": "", "game": "Rust"],
                ["id": "e2", "name": "Twitch Glory Badge", "image": "", "game": NSNull()],
            ],
            "in_progress_empty": query.isEmpty
                ? "No drop campaigns in progress. Watch a drops-enabled stream to start earning."
                : "No drops in progress match that search.",
        ]
    }
}
