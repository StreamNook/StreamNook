import Foundation

/// FakeCore's moderation answers: a chatter's history, acknowledging a warning and deciding a
/// held message. Nothing here reaches Twitch, so no test ever acts on a real chatter or channel.
public enum FakeModTools {
    /// A history for any chatter: a timeout, a deleted message, a warning and a mod comment.
    public static let history: [String: Any] = [
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
    ]

    static func answer(_ cmd: String, _ args: [String: Any]) throws -> String {
        switch cmd {
        case "get_chatter_mod_history":
            let data = try JSONSerialization.data(withJSONObject: history, options: [.sortedKeys])
            return String(decoding: data, as: UTF8.self)
        default:
            return "null"
        }
    }
}
