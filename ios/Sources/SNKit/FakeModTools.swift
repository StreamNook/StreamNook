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

    static func answer(_ cmd: String, _ args: [String: Any]) throws -> String {
        let value: Any
        switch cmd {
        case "get_chatter_mod_history": value = history
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
