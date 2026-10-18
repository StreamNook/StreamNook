import Foundation
import Synchronization

/// FakeCore's stream manager: the broadcaster's own channel with a title, a category, two tags,
/// an ad schedule, chat modes and three redemptions waiting. Writes change only this fake's
/// state and are recorded, so a drive shows every state without touching a real channel.
/// Shapes follow services/stream_manager.rs.
public final class FakeStreamManager: Sendable {
    private struct State {
        var title = "Ranked grind to Diamond, then viewer games"
        var category = ["id": "516575", "name": "VALORANT", "box_art_url": "https://static-cdn.jtvnw.net/ttv-boxart/516575-{width}x{height}.jpg"]
        var tags = ["English", "Ranked"]
        var language = "en"
        var modes: [String: (on: Bool, value: UInt32?)] = ["followers": (true, 10), "slow": (false, nil), "subs": (false, nil),
                                                           "emotes": (false, nil), "unique": (false, nil)]
        var shield = false
        var queue: [(id: String, reward: String, title: String, user: String, input: String?, agoMin: Int)] = [
            ("r1", "song", "Song request", "pixelfox", "Daft Punk, Veridis Quo", 2),
            ("r2", "song", "Song request", "mossyrock", "anything lofi please", 5),
            ("r3", "hydrate", "Hydrate!", "lunarbyte", nil, 0),
        ]
        var paused: Set<String> = []
        var raid: (name: String, deadline: Int64)?
        var poll: String?
        var acts: [String] = []
    }

    private let state = Mutex(State())

    public init() {}

    /// The `kind` of every action handed to `stream_manager_act`, oldest first.
    public var acts: [String] { state.withLock { $0.acts } }

    func answer(_ cmd: String, _ args: [String: Any]) throws -> String? {
        switch cmd {
        case "get_stream_manager":
            return try FakeCore.json(view())
        case "get_reward_queue":
            return try FakeCore.json(queue())
        case "check_stream_info":
            return try FakeCore.json(check(args["draft"] as? [String: Any] ?? [:]))
        case "search_raid_targets":
            let q = (args["query"] as? String ?? "").lowercased()
            let all: [[String: Any]] = [
                ["id": "5001", "login": "mossyrock", "name": "mossyrock", "avatar_url": NSNull(), "game_name": "VALORANT", "live": true, "line": "VALORANT"],
                ["id": "5002", "login": "mossmoth", "name": "MossMoth", "avatar_url": NSNull(), "game_name": "Minecraft", "live": true, "line": "Minecraft"],
                ["id": "5003", "login": "mosslight", "name": "mosslight", "avatar_url": NSNull(), "game_name": "", "live": false, "line": "Offline"],
            ]
            return try FakeCore.json(q.isEmpty ? [] : all.filter { ($0["login"] as? String ?? "").contains(q) })
        case "stream_manager_act":
            let action = args["action"] as? [String: Any] ?? [:]
            let (ok, message) = act(action)
            let queueWrite = ["redemptions", "reward_queue", "reward"].contains(action["kind"] as? String ?? "")
            return try FakeCore.json(["ok": ok, "message": message, "view": view(),
                                      "queue": queueWrite ? queue() : NSNull()] as [String: Any])
        default:
            return nil
        }
    }

    private func act(_ a: [String: Any]) -> (Bool, String) {
        let kind = a["kind"] as? String ?? ""
        return state.withLock { s in
            s.acts.append(kind)
            switch kind {
            case "save_info":
                let d = a["draft"] as? [String: Any] ?? [:]
                let title = (d["title"] as? String ?? "").trimmingCharacters(in: .whitespaces)
                guard !title.isEmpty else { return (false, "Add a title") }
                s.title = title
                if let c = d["category"] as? [String: Any] {
                    s.category = ["id": c["id"] as? String ?? "", "name": c["name"] as? String ?? "", "box_art_url": c["box_art_url"] as? String ?? ""]
                }
                s.tags = d["tags"] as? [String] ?? s.tags
                s.language = d["language"] as? String ?? s.language
                return (true, "Stream info saved")
            case "create_poll":
                s.poll = a["title"] as? String
                return (true, "Poll started")
            case "end_poll":
                s.poll = nil
                return (true, (a["show_results"] as? Bool ?? true) ? "Poll ended" : "Poll removed")
            case "create_prediction":
                return (true, "Prediction started")
            case "raid":
                s.raid = (a["target_name"] as? String ?? "", Int64(Date().timeIntervalSince1970 * 1000) + 90_000)
                return (true, "Raiding \(a["target_name"] as? String ?? "")")
            case "cancel_raid", "raid_now":
                s.raid = nil
                return (true, kind == "raid_now" ? "Raid sent" : "Raid canceled")
            case "run_ad":
                return (true, "Ad break started")
            case "snooze_ad":
                return (true, "Next ad snoozed")
            case "redemptions":
                let ids = a["ids"] as? [String] ?? []
                s.queue.removeAll { ids.contains($0.id) }
                return (true, (a["fulfill"] as? Bool ?? true) ? "Completed" : "Rejected, points refunded")
            case "reward_queue":
                let reward = a["reward_id"] as? String ?? ""
                s.queue.removeAll { $0.reward == reward }
                return (true, "Completed all")
            case "reward":
                let reward = a["reward_id"] as? String ?? ""
                if let p = a["paused"] as? Bool {
                    if p { s.paused.insert(reward) } else { s.paused.remove(reward) }
                    return (true, p ? "Reward paused" : "Reward resumed")
                }
                return (true, "Reward turned off")
            case "chat_mode":
                let mode = a["mode"] as? String ?? ""
                let on = a["on"] as? Bool ?? false
                s.modes[mode] = (on, on ? (a["value"] as? UInt32 ?? (a["value"] as? Int).map(UInt32.init) ?? (mode == "slow" ? 30 : 0)) : nil)
                return (true, "Chat updated")
            case "shield":
                s.shield = a["on"] as? Bool ?? false
                return (true, s.shield ? "Shield Mode on" : "Shield Mode off")
            case "announce":
                let text = (a["text"] as? String ?? "").trimmingCharacters(in: .whitespaces)
                return text.isEmpty ? (false, "Write the announcement first") : (true, "Announced")
            default:
                return (false, "That did not go through")
            }
        }
    }

    private func view() -> [String: Any] {
        state.withLock { s in
            let now = Int64(Date().timeIntervalSince1970 * 1000)
            let raid = s.raid.flatMap { $0.deadline > now ? $0 : nil }
            func mode(_ kind: String, _ label: String, _ options: [(String, UInt32)]) -> [String: Any] {
                let m = s.modes[kind] ?? (false, nil)
                return ["kind": kind, "label": label, "on": m.on, "value": m.value.map { $0 as Any } ?? NSNull(),
                        "value_label": m.value.flatMap { v in options.first { $0.1 == v }?.0 }.map { $0 as Any } ?? NSNull(),
                        "options": options.map { ["label": $0.0, "value": $0.1] as [String: Any] }]
            }
            let summary = "\(s.title) · \(s.category["name"] ?? "")"
            let count = s.queue.count
            return [
                "channel_id": "424242", "channel_login": "nookviewer", "role": "broadcaster", "role_line": "Your channel",
                "sections": [
                    ["id": "info", "label": "Stream info", "detail": summary, "badge": NSNull()],
                    ["id": "poll", "label": s.poll == nil ? "Start a poll" : "Poll running", "detail": s.poll.map { $0 as Any } ?? NSNull(), "badge": NSNull()],
                    ["id": "prediction", "label": "Start a prediction", "detail": NSNull(), "badge": NSNull()],
                    ["id": "raid", "label": raid == nil ? "Raid a channel" : "Raid counting down", "detail": raid.map { $0.name as Any } ?? NSNull(), "badge": NSNull()],
                    ["id": "ads", "label": "Run an ad break", "detail": "Next ad in 14 min · 2 snoozes left", "badge": NSNull()],
                    ["id": "rewards", "label": "Rewards queue", "detail": NSNull(), "badge": count > 0 ? "\(count) waiting" as Any : NSNull()],
                    ["id": "announce", "label": "Announce", "detail": NSNull(), "badge": NSNull()],
                ] as [[String: Any]],
                "info": [
                    "title": s.title, "category": s.category, "tags": s.tags, "language": s.language,
                    "language_name": s.language == "en" ? "English" : s.language,
                    "languages": [["code": "en", "name": "English"], ["code": "es", "name": "Spanish"], ["code": "other", "name": "Other"]],
                    "summary": summary,
                ] as [String: Any],
                "poll": s.poll.map { ["id": "fake-poll", "title": $0] as Any } ?? NSNull(),
                "prediction": NSNull(),
                "raid": raid.map { ["target_id": "5001", "target_login": $0.name.lowercased(), "target_name": $0.name, "deadline_ms": $0.deadline] as Any } ?? NSNull(),
                "ads": [
                    "lengths": [30, 60, 90, 120, 150, 180].map { (s: Int) -> [String: Any] in
                        ["label": s < 60 ? "\(s)s" : s % 60 == 0 ? "\(s / 60) min" : "\(s / 60):30", "value": s]
                    },
                    "schedule": ["next_ad_ms": now + 14 * 60_000, "next_ad_seconds": 60, "preroll_free_seconds": 0, "snoozes_left": 2,
                                 "snooze_refresh_ms": NSNull(), "snooze_line": "2 snoozes left", "preroll_line": NSNull()] as [String: Any],
                    "schedule_hint": NSNull(),
                ] as [String: Any],
                "chat": [
                    "modes": [
                        mode("followers", "Followers-only", [("Any follower", 0), ("10 min", 10), ("1 hour", 60), ("1 day", 1440)]),
                        mode("slow", "Slow mode", [("3s", 3), ("10s", 10), ("30s", 30), ("120s", 120)]),
                        mode("subs", "Subscriber-only", []),
                        mode("emotes", "Emote-only", []),
                        mode("unique", "Unique chat", []),
                    ],
                    "shield": s.shield,
                    "announce_colors": ["primary", "blue", "green", "orange", "purple"].map {
                        ["id": $0, "label": $0 == "primary" ? "Channel color" : $0.capitalized]
                    },
                ] as [String: Any],
                "queue_count": count,
                "limits": [
                    "title": 140, "tags": 10, "tag": 25, "poll_title": 60, "poll_choice": 25, "poll_choices_min": 2,
                    "poll_choices_max": 5, "poll_seconds_min": 15, "poll_seconds_max": 1800, "prediction_title": 45,
                    "prediction_outcome": 25, "prediction_outcomes_min": 2, "prediction_outcomes_max": 10,
                    "prediction_seconds_min": 30, "prediction_seconds_max": 1800,
                    "durations": [("30s", 30), ("1m", 60), ("2m", 120), ("5m", 300), ("10m", 600), ("30m", 1800)].map { (d: (String, Int)) -> [String: Any] in
                        ["label": d.0, "value": d.1]
                    },
                ] as [String: Any],
                "problems": [String](),
            ]
        }
    }

    private func queue() -> [String: Any] {
        state.withLock { s in
            var order: [String] = []
            var rows: [String: [[String: Any]]] = [:]
            var titles: [String: String] = [:]
            for q in s.queue {
                if rows[q.reward] == nil { order.append(q.reward) }
                titles[q.reward] = q.title
                rows[q.reward, default: []].append([
                    "id": q.id, "user_login": q.user, "user_name": q.user, "avatar_url": NSNull(),
                    "input": q.input.map { $0 as Any } ?? NSNull(), "at_ms": 0,
                    "line": "\(q.user) · \(q.agoMin == 0 ? "just now" : "\(q.agoMin) min ago")",
                ])
            }
            let groups: [[String: Any]] = order.map { id in
                ["reward_id": id, "title": titles[id] ?? "", "cost": 500, "paused": s.paused.contains(id), "enabled": true,
                 "image_url": NSNull(), "color": NSNull(), "heading": "\(titles[id] ?? "") · \(rows[id]?.count ?? 0)", "rows": rows[id] ?? []]
            }
            return ["groups": groups, "count": s.queue.count, "empty_line": s.queue.isEmpty ? "Nothing waiting" as Any : NSNull(), "more": false]
        }
    }

    private func check(_ d: [String: Any]) -> [String: Any] {
        let title = (d["title"] as? String ?? "").trimmingCharacters(in: .whitespaces)
        let tags = d["tags"] as? [String] ?? []
        var problems: [[String: String]] = []
        if title.isEmpty { problems.append(["field": "title", "line": "Add a title"]) }
        for (i, t) in tags.enumerated() where !t.allSatisfy({ $0.isLetter || $0.isNumber }) {
            problems.append(["field": "tag:\(i)", "line": "Tags use letters and numbers only, no spaces or symbols"])
        }
        let category = d["category"] as? [String: Any]
        let changed = state.withLock { s in title != s.title || tags != s.tags || (category?["id"] as? String) != s.category["id"] }
        return [
            "ok": problems.isEmpty, "problems": problems, "title_count": "\(title.count) / 140", "tags_count": "\(tags.count) / 10",
            "changed": changed,
            "preview": ["title": title, "channel_name": "NookViewer", "avatar_url": NSNull(),
                        "category_name": category?["name"] as? String ?? "", "box_art_url": NSNull(), "tags": tags,
                        "thumbnail_url": NSNull()] as [String: Any],
        ]
    }
}
