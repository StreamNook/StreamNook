import Foundation

/// FakeCore's moderation answers: a chatter's history, acknowledging a warning and deciding a
/// held message. Nothing here reaches Twitch, so no test ever acts on a real chatter or channel.
public enum FakeModTools {
    /// A history for any chatter: a timeout, a deleted message, a warning and a mod comment.
    public static var history: [String: Any] { [
        "status": "available",
        "entries": [
            ["kind": "comment", "id": "c1", "title": "Mod comment", "by": "mod_kay", "at_ms": 1_760_000_000_000,
             "when": "1 hour ago", "quote": "alt of spam_bot99?"],
            ["kind": "timeout", "id": "a1", "title": "Timed out 10 minutes", "by": "nightbot_mod",
             "at_ms": 1_759_740_000_000, "when": "3 days ago", "quote": "nightbot_mod timed out gus for 10m. Reason: spoilers",
             "seconds": 600],
            ["kind": "deleted", "id": "m1", "title": "Deleted message", "at_ms": 1_759_700_000_000,
             "when": "sent 3 days ago", "quote": "the boss dies at the end lol"],
            ["kind": "warn", "id": "a2", "title": "Warned", "by": "zoox", "at_ms": 1_758_800_000_000,
             "when": "2 weeks ago", "quote": "zoox warned gus. Reason: keep it friendly"],
        ] as [[String: Any]],
    ] }

    static func button(_ id: String, _ label: String, _ kind: String, _ action: [String: Any], danger: Bool = false,
                       reason: Any = NSNull(), asksReason: Bool = false, spoken: String) -> [String: Any] {
        ["id": id, "label": label, "kind": kind, "action": action, "reason": reason, "asks_reason": asksReason,
         "danger": danger, "spoken": spoken]
    }

    /// The card a moderator sees for any chatter: the four default lengths and Ban, two of their
    /// own buttons, the history above, no note and no color yet.
    public static var cardTools: [String: Any] { [
        "note": NSNull(), "nickname": NSNull(), "color": NSNull(),
        "swatches": ["#7FD3FF", "#5AA9FF", "#C49BFF", "#FF7AD9", "#FF7A90", "#FF9F43", "#FFD166", "#7BE08E", "#4FD1C5", "#E6E6E6"],
        "moderate": [
            "buttons": [
                button("t1", "1s", "timeout", ["kind": "timeout", "seconds": 1], spoken: "Purge their messages"),
                button("t600", "10m", "timeout", ["kind": "timeout", "seconds": 600], spoken: "Time out for 10 minutes"),
                button("t3600", "1h", "timeout", ["kind": "timeout", "seconds": 3600], spoken: "Time out for 1 hour"),
                button("t86400", "1d", "timeout", ["kind": "timeout", "seconds": 86400], spoken: "Time out for 1 day"),
                button("ban", "Ban", "ban", ["kind": "ban"], danger: true, spoken: "Ban"),
            ],
            "custom": [
                button("c0", "Spoilers 30m", "timeout", ["kind": "timeout", "seconds": 1800], reason: "spoilers",
                       spoken: "Time out for 30 minutes"),
                button("c1", "Warn", "warn", ["kind": "warn"], asksReason: true, spoken: "Warn"),
            ],
            "history": history,
        ] as [String: Any],
    ] }

    /// The slider's saved timeouts in the fake (Rust's defaults).
    static let sliderTimeouts = [60, 300, 600, 1_800, 3_600, 86_400, 604_800]

    /// The swipe slider for a phone-wide row: Delete, 1m, 10m, 1h, a gap, Ban (Rust's own test).
    public static var swipeSlider: [String: Any] {
        func step(_ action: String, _ seconds: Int?, _ label: String, _ chip: String, _ tone: Any) -> [String: Any] {
            ["action": action, "seconds": seconds as Any? ?? NSNull(), "label": label, "chip": chip, "tone": tone]
        }
        return [
            "enabled": true, "style": "preview",
            "steps": [
                step("delete", nil, "Delete", "Delete Grumpy_Gus\u{2019}s message", NSNull()),
                step("timeout", 60, "1m", "Time out Grumpy_Gus 1m", 0.0),
                step("timeout", 600, "10m", "Time out Grumpy_Gus 10m", 0.325),
                step("timeout", 3600, "1h", "Time out Grumpy_Gus 1h", 0.65),
                step("ban", nil, "Ban", "Ban Grumpy_Gus", 1.0),
            ],
            "dead": 72.0, "spacing": 34.0, "starts": [72.0, 106.0, 140.0, 174.0, 242.0], "end": 293.0,
            "ban_dwell_ms": 350, "ban_still": 10.0, "hysteresis": 4.0, "leash": 120.0, "engage": 10.0,
            "cancel_words": "Let go to cancel", "hold_ban_words": "Hold for Ban, let go to cancel",
            "nudge_words": "Swipe right on a message to moderate it", "start": 0.0, "quick_seconds": 60,
        ]
    }

    /// A room's recent actions, newest first.
    static func roomLog(_ channel: String) -> [[String: Any]] {
        [
            ["id": "l3", "channel": channel, "action": "timeout", "moderator": "Kay_Lee", "target": "Grumpy_Gus",
             "target_id": "42", "line": "Kay_Lee timed out Grumpy_Gus for 10m", "quote": "spoilers",
             "at_ms": 1_760_000_300_000, "liftable": true],
            ["id": "l2", "channel": channel, "action": "delete", "moderator": "Kay_Lee", "target": "spam_bot99",
             "target_id": "77", "line": "Kay_Lee deleted a message from spam_bot99", "quote": "free followers at bit.ly",
             "at_ms": 1_760_000_200_000, "liftable": false],
            ["id": "l1", "channel": channel, "action": "slow", "moderator": "zoox", "target": NSNull(), "target_id": NSNull(),
             "line": "zoox turned on slow mode (30s)", "quote": NSNull(), "at_ms": 1_760_000_100_000, "liftable": false],
        ]
    }

    /// One message AutoMod is holding.
    static func heldRow(_ channel: String) -> [String: Any] {
        ["message_id": "fake-held-1", "channel": channel, "broadcaster_id": "1", "user_id": "42", "user_login": "grumpy_gus",
         "user_name": "Grumpy_Gus", "text": "this game is so broken, what a joke", "reason": "automod",
         "category": "swearing", "level": 2, "held_at_ms": 1_760_000_350_000, "status": "held",
         "label": "Held by AutoMod \u{00B7} Swearing, level 2"]
    }

    static func answer(_ cmd: String, _ args: [String: Any]) throws -> String {
        let value: Any
        switch cmd {
        case "get_chatter_mod_history": value = history
        case "get_touch_mod_slider": value = swipeSlider
        case "nuke_active", "nuke_stop": value = 0
        case "remove_blocked_term": return "null"
        case "add_blocked_term": value = ["id": "fake-term", "text": args["text"] as? String ?? ""] as [String: Any]
        case "get_room_mod_log": value = roomLog(args["channel"] as? String ?? "zackrawrr")
        case "get_automod_queue": value = [heldRow(args["channel"] as? String ?? "zackrawrr")]
        case "edit_mod_slider_timeouts":
            let seconds = args["seconds"] as? Int ?? 0
            var list = sliderTimeouts
            if args["op"] as? String == "add" {
                guard (1...1_209_600).contains(seconds), list.count < 8 else {
                    throw CoreFailure.command(json: #""Up to eight lengths, each 1 second to 14 days.""#)
                }
                if !list.contains(seconds) { list.append(seconds) }
            } else {
                list.removeAll { $0 == seconds }
                guard !list.isEmpty else { throw CoreFailure.command(json: #""Keep at least one timeout length.""#) }
            }
            value = list.sorted()
        case "parse_mod_timeout":
            let text = (args["text"] as? String ?? "").trimmingCharacters(in: .whitespaces).lowercased()
            let table: [String: (Int, String)] = ["45m": (2_700, "45m"), "1h30m": (5_400, "1h 30m"), "2d": (172_800, "2d"), "90s": (90, "1m 30s")]
            guard let hit = table[text] else {
                throw CoreFailure.command(json: #""Type a length like 45m, 2h or 3d, up to 14 days.""#)
            }
            value = ["action": "timeout", "seconds": hit.0, "label": hit.1] as [String: Any]
        case "get_chatter_card_tools": value = cardTools
        case "moderate_chatter":
            // Words the real door would say; nothing is sent anywhere.
            let request = args["request"] as? [String: Any] ?? [:]
            let action = request["action"] as? [String: Any] ?? [:]
            let name = request["targetName"] as? String ?? "them"
            switch action["kind"] as? String {
            case "timeout":
                let seconds = action["seconds"] as? Int ?? 600
                value = ["applied_seconds": seconds, "message": "Timed out \(name) for \(seconds)s", "undoable": true] as [String: Any]
            case "ban": value = ["applied_seconds": NSNull(), "message": "Banned \(name)", "undoable": true] as [String: Any]
            case "unban": value = ["applied_seconds": NSNull(), "message": "Unbanned \(name)", "undoable": false] as [String: Any]
            case "warn": value = ["applied_seconds": NSNull(), "message": "Warned \(name)", "undoable": false] as [String: Any]
            default: throw CoreFailure.command(json: #""unknown mod action""#)
            }
        default:
            return "null"
        }
        let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
        return String(decoding: data, as: UTF8.self)
    }
}
