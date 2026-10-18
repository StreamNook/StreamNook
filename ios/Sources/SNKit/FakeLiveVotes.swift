import Foundation
import Synchronization

/// FakeCore's polls and predictions: a poll ("Which boss next?") and a prediction ("Beats it
/// first try?") in every state the cards draw, worded the way Rust words them. Votes and
/// predictions are only recorded; nothing here reaches Twitch. Drive steps put a card in a state
/// with `pollPush` / `predictionPush` and emit the answer as the core's push.
public final class FakeLiveVotes: Sendable {
    /// The poll's states: none, open, voted (the viewer's vote counted), ended.
    public enum PollState: String, Sendable { case off, open, voted, ended }
    /// The prediction's states: none, open, picked (the viewer staked), locked, won, refunded.
    public enum PredictionState: String, Sendable { case off, open, picked, locked, won, refunded }

    private struct State {
        var poll = PollState.off
        var prediction = PredictionState.off
        var vote: String?
        var stake: (outcome: String, amount: Int64)?
        var pollDeadline: Int64 = 0
        var predictionDeadline: Int64 = 0
        var votes: [String] = []
        var predicts: [String] = []
        var bets: [String] = []
    }

    private let state = Mutex(State())

    public init() {}

    /// `pollId/choiceId` handed to `vote_on_poll`, oldest first.
    public var votes: [String] { state.withLock { $0.votes } }
    /// `outcomeId:amount` handed to `predict`, oldest first.
    public var predicts: [String] { state.withLock { $0.predicts } }
    /// `outcomeId:amount` asked of `prediction_bet`, oldest first.
    public var bets: [String] { state.withLock { $0.bets } }

    public var pollState: PollState { state.withLock { $0.poll } }
    public var predictionState: PredictionState { state.withLock { $0.prediction } }

    /// A command's answer, and the push the core would send after it. Nil for other commands.
    public func answer(_ cmd: String, _ args: [String: Any]) throws -> (json: String, push: (name: String, payload: String)?)? {
        let channel = args["channelId"] as? String ?? Self.channel
        switch cmd {
        case "get_channel_prediction":
            return (try FakeCore.json(["channel_id": channel, "view": predictionView(channel: channel)]), nil)
        case "watch_channel_poll_view":
            return (try Self.fragment(pollView(channel: channel)), nil)
        case "chat_top_stack":
            return (try FakeCore.json(Self.stackLayout(args["request"] as? [String: Any] ?? [:])), nil)
        case "prediction_bet":
            let outcome = args["outcomeId"] as? String ?? ""
            let typed = args["amount"] as? String
            state.withLock { $0.bets.append("\(outcome):\(typed ?? "")") }
            guard let bet = bet(outcome: outcome, typed: typed) else { return ("null", nil) }
            return (try FakeCore.json(bet), nil)
        case "predict":
            let outcome = args["outcomeId"] as? String ?? ""
            let amount = (args["amount"] as? Int).map(Int64.init) ?? (args["amount"] as? NSNumber)?.int64Value ?? 0
            let title = Self.outcomes.first { $0.id == outcome }?.title
            let refused: String? = state.withLock { s in
                if s.prediction != .open && s.prediction != .picked { return "Predictions are closed" }
                if title == nil { return "That side is gone" }
                if let stake = s.stake, stake.outcome != outcome { return "You can't switch sides" }
                if amount < 1 { return "Pick an amount" }
                s.predicts.append("\(outcome):\(amount)")
                s.stake = (outcome, (s.stake?.amount ?? 0) + amount)
                s.prediction = .picked
                return nil
            }
            if let refused {
                return (try FakeCore.json(["success": false, "message": refused, "view": NSNull()] as [String: Any]), nil)
            }
            let view = predictionView(channel: channel)
            let push = try FakeCore.json(["channel_id": channel, "view": view])
            return (try FakeCore.json(["success": true, "message": "Predicted \(Self.grouped(amount)) on \(title ?? "")",
                                       "view": view] as [String: Any]), ("prediction-state", push))
        case "vote_on_poll":
            let choice = args["choiceId"] as? String ?? ""
            let refused: String? = state.withLock { s in
                guard s.poll == .open else { return "This poll is closed" }
                guard Self.choices.contains(where: { $0.id == choice }) else { return "That choice is gone" }
                s.votes.append("\(args["pollId"] as? String ?? "")/\(choice)")
                s.vote = choice
                s.poll = .voted
                return nil
            }
            if let refused { throw CoreFailure.command(json: try Self.fragment(refused)) }
            let push = try FakeCore.json(["channel_id": channel, "view": pollView(channel: channel)])
            return (try FakeCore.json(["ok": true]), ("poll-state", push))
        default:
            return nil
        }
    }

    // MARK: Pushes for drive steps

    /// Puts the poll in `state` and returns the `poll-state` push that announces it.
    public func pollPush(channel: String, _ next: PollState) -> String {
        state.withLock { s in
            if next == .open { s.vote = nil; s.pollDeadline = LiveVoteClock.nowMS() + 42_000 }
            if next == .voted, s.vote == nil { s.vote = "a" }
            if next == .off { s.vote = nil }
            s.poll = next
        }
        return (try? FakeCore.json(["channel_id": channel, "view": pollView(channel: channel)])) ?? "{}"
    }

    /// Puts the prediction in `state` and returns the `prediction-state` push that announces it.
    public func predictionPush(channel: String, _ next: PredictionState) -> String {
        state.withLock { s in
            switch next {
            case .open:
                s.stake = nil
                s.predictionDeadline = LiveVoteClock.nowMS() + 130_000
            case .picked, .locked, .won:
                if s.stake == nil { s.stake = ("yes", 100) }
                if s.predictionDeadline <= LiveVoteClock.nowMS() { s.predictionDeadline = LiveVoteClock.nowMS() + 130_000 }
            case .refunded, .off:
                break
            }
            s.prediction = next
        }
        return (try? FakeCore.json(["channel_id": channel, "view": predictionView(channel: channel)])) ?? "{}"
    }

    // MARK: The stack, as Rust lays it out

    /// `chat_top_stack`'s answer, the way services/chat_top_stack.rs decides it, so the fake chat
    /// stage stacks its cards like the real core: votes in the order setting, the drop after them,
    /// one card open at most (the one the viewer opened, else on a tall enough column the first
    /// vote they did not fold, else a lone drop), capped at 40 percent of the column.
    public static func stackLayout(_ request: [String: Any]) -> [String: Any] {
        let cards = request["cards"] as? [String] ?? []
        let folded = request["folded"] as? [String] ?? []
        let height = (request["chat_height"] as? NSNumber)?.doubleValue ?? 0
        let collapsed = request["polls_start_collapsed"] as? Bool ?? false
        let votes = request["order"] as? String == "poll-first" ? ["poll", "prediction"] : ["prediction", "poll"]
        let order = (votes + ["drop"]).filter(cards.contains)
        let compact = height > 0 && height < 460
        var open = (request["opened"] as? String).flatMap { cards.contains($0) ? $0 : nil }
        if open == nil, !compact {
            open = order.first { kind in
                switch kind {
                case "poll": !collapsed && !folded.contains(kind)
                case "prediction": !folded.contains(kind)
                default: !folded.contains(kind) && order.allSatisfy { $0 == "drop" }
                }
            }
        }
        let maxHeight = height > 0 ? max(height * 0.4, 76).rounded() : 0
        return ["slots": order.map { ["kind": $0, "open": $0 == open] as [String: Any] }, "max_height": maxHeight]
    }

    // MARK: The views, as Rust words them

    /// The fake wallet channel's room.
    static let channel = "4242"
    static let balance: Int64 = 12_850
    private static let pool: Int64 = 10_000
    private static let outcomes: [(id: String, title: String, color: String, points: Int64, people: Int)] = [
        ("yes", "Yes", "blue", 3_600, 412),
        ("no", "No", "pink", 6_400, 688),
    ]
    private static let choices: [(id: String, title: String, percent: Int, votes: Int)] = [
        ("a", "Malenia", 58, 698), ("b", "Radahn", 31, 373), ("c", "Mohg", 11, 133),
    ]

    private func predictionView(channel: String) -> Any {
        let (phase, stake, deadline) = state.withLock { ($0.prediction, $0.stake, $0.predictionDeadline) }
        guard phase != .off else { return NSNull() }
        let open = phase == .open || phase == .picked
        let added = stake?.amount ?? 0
        let total = Self.pool + added
        let outcomes: [[String: Any]] = Self.outcomes.map { o in
            let points = o.points + (stake?.outcome == o.id ? added : 0)
            let people = o.people + (stake?.outcome == o.id ? 1 : 0)
            return [
                "id": o.id, "title": o.title, "color": o.color,
                "percent": Int((Double(points) / Double(total) * 100).rounded()),
                "ratio": String(format: "1:%.2f", Double(total) / Double(points)),
                "points_label": Self.compact(points), "people_label": Self.grouped(Int64(people)),
                "winner": phase == .won && o.id == "yes", "mine": stake?.outcome == o.id,
            ]
        }
        let mineTitle = stake.flatMap { s in Self.outcomes.first { $0.id == s.outcome }?.title }
        let stakeLine = stake.map { "\(Self.grouped($0.amount)) on \(mineTitle ?? "")" }
        let status: Any = switch phase {
        case .picked: stakeLine.map { $0 as Any } ?? NSNull()
        case .locked: stakeLine.map { "\($0) · waiting for the result" as Any } ?? NSNull()
        default: NSNull()
        }
        let result: Any = switch phase {
        case .won:
            ["tone": stake?.outcome == "yes" ? "won" : "lost",
             "line": stake.map { s in
                 s.outcome == "yes" ? "Yes won · you won \(Self.grouped(payout(stake: s.amount, side: 3_600 + s.amount, pool: total))) Nooks"
                     : "Yes won · you lost \(Self.grouped(s.amount)) Nooks"
             } ?? "Yes won"]
        case .refunded: ["tone": "refunded", "line": "Canceled · points refunded"]
        default: NSNull()
        }
        let people = Self.outcomes.reduce(0) { $0 + $1.people } + (stake == nil ? 0 : 1)
        return [
            "channel_id": channel, "prediction_id": "fake-prediction", "title": "Beats it first try?",
            "phase": open ? "open" : phase == .locked ? "locked" : "ended",
            "deadline_ms": open ? deadline as Any : NSNull(),
            "badge": open ? NSNull() : (phase == .locked ? "Closed" : "Ended") as Any,
            "outcomes": outcomes, "can_predict": open,
            "picked": stake.map { $0.outcome as Any } ?? NSNull(),
            "status_line": status, "result": result,
            "totals_line": "\(Self.grouped(Int64(people))) people · \(Self.compact(total)) Nooks",
            "points_name": "Nooks", "points_icon": NSNull(),
        ] as [String: Any]
    }

    private func pollView(channel: String) -> Any {
        let (phase, vote, deadline) = state.withLock { ($0.poll, $0.vote, $0.pollDeadline) }
        guard phase != .off else { return NSNull() }
        let ended = phase == .ended
        let mine = Self.choices.first { $0.id == vote }?.title
        let choices: [[String: Any]] = Self.choices.map { c in
            ["id": c.id, "title": c.title, "percent": c.percent, "votes_label": Self.grouped(Int64(c.votes + (vote == c.id ? 1 : 0))),
             "mine": vote == c.id, "winner": ended && c.id == "a"]
        }
        let voters = Self.choices.reduce(0) { $0 + $1.votes } + (vote == nil ? 0 : 1)
        return [
            "channel_id": channel, "poll_id": "fake-poll", "title": "Which boss next?",
            "phase": ended ? "ended" : "open",
            "deadline_ms": ended ? NSNull() : deadline as Any,
            "badge": ended ? "Final" as Any : NSNull(),
            "choices": choices, "can_vote": phase == .open,
            "my_choice_id": vote.map { $0 as Any } ?? NSNull(),
            "footer": mine.map { "You voted \($0)" } ?? (ended ? "Voting is closed" : "Tap a choice to vote"),
            "voters_line": "\(Self.grouped(Int64(voters))) voters",
            "result_line": ended ? "Malenia won · 58%" as Any : NSNull(),
        ] as [String: Any]
    }

    private func bet(outcome: String, typed: String?) -> [String: Any]? {
        let (phase, stake) = state.withLock { ($0.prediction, $0.stake) }
        guard phase != .off, let side = Self.outcomes.first(where: { $0.id == outcome }) else { return nil }
        let max = Self.balance
        let digits = (typed ?? "").filter(\.isASCII).filter(\.isNumber)
        let amount = Swift.max(1, Swift.min(Int64(digits) ?? 10, max))
        let topUp = stake?.outcome == outcome
        let problem: String? = if phase != .open && phase != .picked {
            "Predictions are closed"
        } else if let stake, stake.outcome != outcome {
            "You already picked \(Self.outcomes.first { $0.id == stake.outcome }?.title ?? "the other side")"
        } else {
            nil
        }
        let held = topUp ? stake?.amount ?? 0 : 0
        let sidePoints = side.points + held
        let win = payout(stake: held + amount, side: sidePoints + amount, pool: Self.pool + held + amount)
        let words = Self.grouped(amount)
        let steps: [Int64] = [10, 100, 1_000]
        var presets: [[String: Any]] = steps.filter { $0 <= max }.map { ["label": Self.compact($0), "amount": $0] }
        if max > 0 && max != 1_000 { presets.append(["label": "Max", "amount": max]) }
        return [
            "amount": amount, "max": max, "presets": presets, "ok": problem == nil,
            "problem": problem.map { $0 as Any } ?? NSNull(),
            "button": topUp ? "Add \(words) to \(side.title)" : "Predict \(words) on \(side.title)",
            "win_line": "Win up to \(Self.grouped(win))",
            "confirm_title": topUp ? "Add \(words) Nooks to \(side.title)?" : "Predict \(words) Nooks on \(side.title)?",
            "confirm_line": "Win up to \(Self.grouped(win)) if \(side.title) wins.",
            "confirm_note": topUp ? NSNull() : "You can add more later, but you can't switch sides." as Any,
            "balance_line": "\(Self.grouped(max)) Nooks",
        ]
    }

    /// JSON for any value, a bare null or string included.
    static func fragment(_ value: Any) throws -> String {
        let data = try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .fragmentsAllowed])
        return String(decoding: data, as: UTF8.self)
    }

    private func payout(stake: Int64, side: Int64, pool: Int64) -> Int64 {
        guard side > 0 else { return stake }
        return Int64((Double(stake) * Double(pool) / Double(side)).rounded(.down))
    }

    /// 12850 as "12,850", independent of the phone's locale (Rust's `grouped`).
    static func grouped(_ n: Int64) -> String {
        let digits = String(n.magnitude)
        var out = ""
        for (i, ch) in digits.enumerated() {
            if i > 0, (digits.count - i) % 3 == 0 { out.append(",") }
            out.append(ch)
        }
        return (n < 0 ? "-" : "") + out
    }

    /// 1000 as "1K", 3600 as "3.6K" (Rust's `compact`).
    static func compact(_ n: Int64) -> String {
        guard n >= 1_000 else { return "\(n)" }
        let (scale, suffix): (Double, String) = n >= 1_000_000 ? (1_000_000, "M") : (1_000, "K")
        let value = Double(n) / scale
        let text = value >= 100 || value == value.rounded() ? String(Int(value)) : String(format: "%.1f", value)
        return text.hasSuffix(".0") ? String(text.dropLast(2)) + suffix : text + suffix
    }
}
