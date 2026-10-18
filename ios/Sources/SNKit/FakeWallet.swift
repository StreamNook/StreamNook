import Foundation

/// FakeCore's channel wallet: points with the channel's own name, bits, a rewards list that
/// covers every verdict and kind, and the emotes an unlock offers. Shapes follow the core's
/// `channel_state`, `ChannelRewardsSnapshot` and `RedeemOutcome`.
enum FakeWallet {
    static func state(balance: Int64, chest: Bool, gain: (seq: Int, amount: Int, chest: Bool)? = nil) -> [String: Any] {
        [
            "login": "nookstreamer", "channel_id": "4242",
            "points": [
                "enabled": true, "balance": balance, "name": "Nooks", "icon_url": NSNull(),
                "available_claim_id": chest ? "chest-1" as Any : NSNull(),
                "last_gain": gain.map { ["seq": $0.seq, "amount": $0.amount, "reason": $0.chest ? "from the bonus chest" : "for watching",
                                         "chest": $0.chest] as [String: Any] } ?? NSNull(),
            ] as [String: Any],
            "bits": ["enabled": true, "pick_min": 1, "balance": 300,
                     "gem_url": "https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/static/100/2.png",
                     "last_gain": NSNull()] as [String: Any],
        ]
    }

    /// A `channel-state` push for the points section.
    static func pointsPush(login: String, balance: Int64, chest: Bool, gain: (seq: Int, amount: Int, chest: Bool)?) -> [String: Any] {
        var push = ["section": "points", "login": login, "at": 0] as [String: Any]
        push["points"] = state(balance: balance, chest: chest, gain: gain)["points"]
        return push
    }

    private static func reward(_ id: String, _ title: String, cost: Int, color: String = "#9147FF", prompt: String? = nil,
                               type: String? = nil, kind: String = "standard", takesText: Bool = false,
                               block: (reason: String, label: String, detail: String, cooldownMS: Int64?)? = nil,
                               balance: Int64) -> [String: Any] {
        let affordable = Int64(cost) <= balance
        typealias Block = (reason: String, label: String, detail: String, cooldownMS: Int64?)
        let short: Block = (reason: "not_enough_points", label: "Not enough Nooks",
                            detail: "You need \(Int64(cost) - balance) more Nooks", cooldownMS: nil)
        let blocked: Block? = block ?? (affordable ? nil : short)
        let message = kind == "highlight" || kind == "sub_message"
        let picks = kind == "choose_emote" || kind == "modify_emote"
        let costLabel = cost >= 1000 ? (cost % 1000 == 0 ? "\(cost / 1000)k" : String(format: "%.1fk", Double(cost) / 1000)) : "\(cost)"
        return [
            "id": id, "title": title, "cost": cost, "prompt": prompt.map { $0 as Any } ?? NSNull(), "image_url": NSNull(),
            "background_color": color, "is_enabled": true, "is_paused": false, "is_in_stock": true,
            "is_user_input_required": takesText, "cooldown_expires_at": NSNull(),
            "reward_type": type.map { $0 as Any } ?? NSNull(), "description": NSNull(),
            "availability": [
                "redeemable": blocked == nil, "reason": blocked.map { $0.reason as Any } ?? NSNull(),
                "cooldown_until_ms": blocked?.cooldownMS.map { $0 as Any } ?? NSNull(), "affordable": affordable,
                "label": blocked.map { $0.label as Any } ?? NSNull(), "detail": blocked.map { $0.detail as Any } ?? NSNull(),
            ] as [String: Any],
            "view": [
                "kind": kind, "takes_text": takesText, "cost_label": costLabel,
                "action": kind == "highlight" ? "Send highlighted" : kind == "sub_message" ? "Send" : picks ? "Choose emote" : "Redeem",
                "busy": message ? "Sending..." : "Redeeming...",
                "placeholder": takesText ? (kind == "input" ? "Type your answer..." : "Type your message...") as Any : NSNull(),
                "hint": !takesText && !picks ? "Can't be undone" as Any : NSNull(),
            ] as [String: Any],
        ]
    }

    static func rewards(balance: Int64) -> [String: Any] {
        let soon = Int64(Date().timeIntervalSince1970 * 1000) + 4 * 60_000 + 12_000
        return [
            "login": "nookstreamer", "balance": balance, "fetched_at_ms": Int64(Date().timeIntervalSince1970 * 1000),
            "rewards": [
                reward("hydrate", "Hydrate!", cost: 500, color: "#00C7AC", prompt: "The streamer takes a sip of water.", balance: balance),
                reward("song", "Song request", cost: 2_500, color: "#FA2929", prompt: "Name a song and the artist.",
                       kind: "input", takesText: true, balance: balance),
                reward("highlight", "Highlight My Message", cost: 1_000, type: "SEND_HIGHLIGHTED_MESSAGE",
                       kind: "highlight", takesText: true, balance: balance),
                reward("random", "Unlock a Random Sub Emote", cost: 3_200, type: "RANDOM_SUB_EMOTE_UNLOCK",
                       kind: "random_emote", balance: balance),
                reward("choose", "Choose an Emote to Unlock", cost: 6_400, type: "CHOSEN_SUB_EMOTE_UNLOCK",
                       kind: "choose_emote", balance: balance),
                reward("timeout", "Timeout a friend", cost: 5_000, color: "#BF94FF",
                       block: ("on_cooldown", "On cooldown", "On cooldown for everyone", soon), balance: balance),
                reward("vip", "VIP for a week", cost: 150_000, color: "#FFB31A", balance: balance),
            ],
        ]
    }

    static var bitsState: [String: Any] {
        let gem = { (n: Int) in "https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/\(n)/2.gif" }
        let tiers = [1, 100, 1000].map { ["bits": $0, "url": gem($0), "color": "#9c3ee8"] as [String: Any] }
        return [
            "enabled": true, "cheer_min": 1, "emote_min": 1, "pick_min": 1, "pick_max": 10_000,
            "quick_amounts": [1, 100, 500, 1000], "balance": 300,
            "cheermotes": [["prefix": "Nook", "custom": true, "tiers": tiers] as [String: Any],
                           ["prefix": "Cheer", "custom": false, "tiers": tiers] as [String: Any]],
            "icons": tiers,
        ]
    }

    static var emotes: [[String: Any]] { [
        ["id": "25", "token": "nookHype", "emote_type": "SUBSCRIPTIONS",
         "modifications": [["id": "25_BW", "modifier_id": "MOD_BW", "token": "nookHype_BW"],
                           ["id": "25_SG", "modifier_id": "MOD_SG", "token": "nookHype_SG"]]],
        ["id": "1902", "token": "nookWave", "emote_type": "SUBSCRIPTIONS", "modifications": [[String: Any]]()],
        ["id": "354", "token": "nookLove", "emote_type": "SUBSCRIPTIONS", "modifications": [[String: Any]]()],
    ] }

    static func redeem(_ id: String, text: String?, emoteName: String?) -> [String: Any] {
        switch id {
        case "highlight":
            return ["success": true, "message": "Highlighted message sent!", "new_balance": NSNull(), "unlocked_emote": NSNull()]
        case "random", "choose":
            let name = emoteName ?? "nookWave"
            return ["success": true, "message": "Unlocked \(name)", "new_balance": NSNull(),
                    "unlocked_emote": ["id": "1902", "name": name,
                                       "image_url": "https://static-cdn.jtvnw.net/emoticons/v2/1902/default/dark/2.0"]]
        case "vip":
            return ["success": false, "message": "You need 137,150 more Nooks", "new_balance": NSNull(), "unlocked_emote": NSNull()]
        default:
            return ["success": true, "message": "Redeemed: \(id == "song" ? "Song request" : "Hydrate!")", "new_balance": NSNull(),
                    "unlocked_emote": NSNull()]
        }
    }
}
