import Foundation
import Synchronization

/// FakeCore's Rewards answers: a drops connection, campaigns in progress and running, drops
/// earned. Writes (a claim) are only recorded, so a drive can show every state without touching
/// a real account.
public final class FakeRewards: Sendable {
    private struct State {
        var connected = true
        var claims: [String] = []
        var redeems: [String] = []
        var chestClaims = 0
        var balance: Int64 = 12_850
        var chest = false
        var gainSeq = 0
        var cheers: [String] = []
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
                "image": "", "trailing": ready ? "" : "1h 48m to watch", "line": "Rust Twitch Drops Round 41 · \(ready ? 2 : 3) of 4",
                "percent": ready ? 100.0 : 40.0, "complete": ready, "claim_label": "Claim",
                "spoken": ready ? "Hoodie, ready to claim" : "Pants, 108 minutes left",
            ] as [String: Any])
        case "claim_drop":
            state.withLock { $0.claims.append(args["dropId"] as? String ?? "") }
            return "null"
        case "get_channel_state":
            return try FakeCore.json(FakeWallet.state(balance: walletBalance, chest: chestWaiting))
        case "get_channel_rewards", "get_cached_channel_rewards":
            return try FakeCore.json(FakeWallet.rewards(balance: walletBalance))
        case "get_modifiable_emotes":
            return try FakeCore.json(FakeWallet.emotes)
        case "refresh_channel_state":
            return "null"
        case "redeem_reward":
            let id = args["rewardId"] as? String ?? ""
            state.withLock { $0.redeems.append(id) }
            return try FakeCore.json(FakeWallet.redeem(id, text: args["text"] as? String, emoteName: args["emoteName"] as? String))
        case "check_cheer":
            let check = Self.cheer(in: args["text"] as? String ?? "", channel: args["channelLogin"] as? String ?? "")
            return try check.map { try FakeCore.json($0) } ?? "null"
        case "send_cheer":
            state.withLock { $0.cheers.append(args["text"] as? String ?? "") }
            return try FakeCore.json(["sent": true, "balance": 250, "problem": NSNull(), "can_continue": false, "needs_bits": false] as [String: Any])
        case "get_bits_state":
            return try FakeCore.json(FakeWallet.bitsState)
        case "claim_channel_points":
            state.withLock { $0.chestClaims += 1; $0.chest = false; $0.balance += 50 }
            return "null"
        default:
            return nil
        }
    }

    // MARK: The wallet (FakeWallet answers; writes are only recorded)

    /// Messages handed to `send_cheer`, oldest first. Nothing is spent: this is the fake.
    public var cheers: [String] { state.withLock { $0.cheers } }

    /// What Rust's cheer check would say of `text`: the fake's cheermotes are "Cheer" and
    /// "Nook", the balance 300 bits. Nil when the text does not cheer.
    static func cheer(in text: String, channel: String) -> [String: Any]? {
        let words = text.split(separator: " ")
        var total = 0
        var plain: [Substring] = []
        for word in words {
            let lower = word.lowercased()
            if let prefix = ["cheer", "nook"].first(where: { lower.hasPrefix($0) }),
               let n = Int(lower.dropFirst(prefix.count)), n > 0 {
                total += n
            } else {
                plain.append(word)
            }
        }
        guard total > 0 else { return nil }
        let balance = 300
        let short = total > balance
        return [
            "channel": channel.lowercased(), "channel_id": "4242", "text": text, "total": total, "balance": balance,
            "problems": short ? ["You have 300 bits. This cheer needs \(total)."] : [String](),
            "needs_bits": short,
            "icon_url": "https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/\(total >= 100 ? 100 : 1)/2.gif",
            "plain_text": plain.joined(separator: " "),
        ]
    }

    /// Rust holds a message that cheers rather than sending it: the held answer, or nil.
    public static func heldCheer(_ text: String, channel: String) -> String? {
        guard let check = cheer(in: text, channel: channel) else { return nil }
        return try? FakeCore.json(["is_sent": false, "message_id": NSNull(), "drop_reason": NSNull(), "cheer": check] as [String: Any])
    }

    /// Reward ids handed to `redeem_reward`, oldest first.
    public var redeems: [String] { state.withLock { $0.redeems } }
    /// Chests collected through `claim_channel_points`.
    public var chestClaims: Int { state.withLock { $0.chestClaims } }
    /// The fake channel's points balance.
    public var walletBalance: Int64 {
        get { state.withLock { $0.balance } }
        set { state.withLock { $0.balance = newValue } }
    }
    /// A bonus chest waits on the button.
    public var chestWaiting: Bool {
        get { state.withLock { $0.chest } }
        set { state.withLock { $0.chest = newValue } }
    }

    /// A gain as the core announces it: the balance rises and a `channel-state` points push
    /// carries the new `last_gain`.
    public func gainPush(login: String, amount: Int, chest: Bool) -> String {
        let (balance, seq, waiting) = state.withLock { s -> (Int64, Int, Bool) in
            s.balance += Int64(amount)
            s.gainSeq += 1
            return (s.balance, s.gainSeq, s.chest)
        }
        let push = FakeWallet.pointsPush(login: login, balance: balance, chest: waiting, gain: (seq: seq, amount: amount, chest: chest))
        return (try? FakeCore.json(push)) ?? "{}"
    }

    /// The points push for the current balance and chest, with no new gain.
    public func statePush(login: String) -> String {
        let (balance, waiting, seq) = state.withLock { ($0.balance, $0.chest, $0.gainSeq) }
        let push = FakeWallet.pointsPush(login: login, balance: balance, chest: waiting,
                                         gain: seq > 0 ? (seq: seq, amount: 0, chest: false) : nil)
        return (try? FakeCore.json(push)) ?? "{}"
    }

    private static let art = "https://static-cdn.jtvnw.net/ttv-boxart/"

    private static func drop(_ id: String, _ name: String, need: Int, have: Int, claimed: Bool = false,
                             detail: String? = nil) -> [String: Any] {
        let done = claimed || (need > 0 && have >= need)
        let status = claimed ? "Claimed" : done ? "Ready" : need > 0 ? "\(need - min(have, need))m to watch" : ""
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
            "meta_label": claimed.isEmpty ? "\(drops.count) rewards" : claimed,
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
                     "icons_only": false, "random_note": "1 of 3 at random", "channels_note": "Only on shroud, hjune", "time_left": "5h left", "urgent": true],
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
                     claimed: "1 of 4 claimed", percent: 42, detail: "4 rewards · 3d left",
                     channels: ["Shroud", "Hjune", "Willjum"],
                     drops: [drop("d1", "Garage Door", need: 60, have: 60, claimed: true),
                             drop("d2", "Hoodie", need: 120, have: 72),
                             drop("d3", "Pants", need: 180, have: 72),
                             drop("d4", "Rock Skin", need: 240, have: 72)]),
                card("c2", "Rust Streamer Drops", game: "Rust", gameID: "263490", boxArt: rustArt,
                     claimed: "0 of 2 claimed", percent: 15, detail: "2 rewards · 20h left",
                     drops: [drop("d5", "Sleeping Bag", need: 120, have: 18),
                             drop("d6", "Door", need: 0, have: 0, detail: "Subscribe or gift a sub")]),
            ]),
            group("progress", "VALORANT", art: valArt, [
                card("c3", "Champions Watch Party", game: "VALORANT", gameID: "516575", boxArt: valArt,
                     claimed: "0 of 3 claimed", percent: 66, detail: "3 rewards · 5d left", link: true,
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
        let earned: [[String: Any]] = [
            ["id": "e1", "name": "Garage Door", "image": "", "game": "Rust"],
            ["id": "e2", "name": "Twitch Glory Badge", "image": "", "game": NSNull()],
        ].filter { row in
            query.isEmpty || [row["name"], row["game"]].contains { ($0 as? String ?? "").lowercased().contains(query.lowercased()) }
        }
        return [
            "connected": true, "query": query, "in_progress": progress, "available": available,
            "earned": earned,
            "no_match": !query.isEmpty && progress.isEmpty && available.isEmpty && earned.isEmpty
                ? "Nothing matches \"\(query)\"." : "",
            "in_progress_empty": query.isEmpty
                ? "No drop campaigns in progress. Watch a drops-enabled stream to start earning."
                : "No drops in progress match that search.",
        ]
    }
}
