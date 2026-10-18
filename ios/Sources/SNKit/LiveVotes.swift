import Foundation

// Polls and predictions as Rust decides them: every word, number and verdict arrives finished
// (`prediction-state` / `poll-state` pushes, `get_channel_prediction`, `prediction_bet`,
// `predict`, `watch_channel_poll_view`), and so does how they stack with the drop at the top of
// chat (`chat_top_stack`). The shells only draw them, tick a countdown to `deadline_ms`, and
// remember which card the viewer opened or folded.

// MARK: Predictions

/// One prediction card, decided in full.
public struct PredictionView: Decodable, Sendable, Equatable {
    /// The room id the card belongs to.
    public var channelID: String
    public var predictionID: String
    public var title: String
    /// `open`, `locked` (waiting for the result), `resolving` or `ended`.
    public var phase: String
    /// Unix ms the window closes, only while open.
    public var deadlineMS: Int64?
    /// The time slot's words when there is no countdown: "Closed", "Ended".
    public var badge: String?
    public var outcomes: [OutcomeView]
    /// Picking a side and an amount is offered.
    public var canPredict: Bool
    /// The side the viewer already staked; the other sides are closed to them.
    public var picked: String?
    /// "100 on Yes", "100 on Yes · waiting for the result", "Working out the result".
    public var statusLine: String?
    /// The ending, once there is one.
    public var result: ResultView?
    /// "1,100 people · 10K Nooks".
    public var totalsLine: String
    /// The channel's name for its points.
    public var pointsName: String
    public var pointsIcon: String?

    enum CodingKeys: String, CodingKey {
        case title, phase, badge, outcomes, picked, result
        case channelID = "channel_id"
        case predictionID = "prediction_id"
        case deadlineMS = "deadline_ms"
        case canPredict = "can_predict"
        case statusLine = "status_line"
        case totalsLine = "totals_line"
        case pointsName = "points_name"
        case pointsIcon = "points_icon"
    }

    public var isOpen: Bool { phase == "open" }

    /// The outcome with this id.
    public func outcome(_ id: String?) -> OutcomeView? {
        guard let id else { return nil }
        return outcomes.first { $0.id == id }
    }
}

public struct OutcomeView: Decodable, Sendable, Equatable, Identifiable, Hashable {
    public var id: String
    public var title: String
    /// Twitch's color for the side, lowercase: `blue`, `pink`, others as sent.
    public var color: String
    /// Share of the points, 0 to 100.
    public var percent: Int
    /// What one point returns if this side wins: "1:2.78".
    public var ratio: String
    public var pointsLabel: String
    public var peopleLabel: String
    public var winner: Bool
    public var mine: Bool

    enum CodingKeys: String, CodingKey {
        case id, title, color, percent, ratio, winner, mine
        case pointsLabel = "points_label"
        case peopleLabel = "people_label"
    }
}

public struct ResultView: Decodable, Sendable, Equatable {
    /// `won`, `lost`, `refunded` or `ended` (the viewer had no known stake).
    public var tone: String
    /// "Yes won · you won 280 Nooks".
    public var line: String
}

/// The `prediction-state` push, and `get_channel_prediction`'s answer. A null view takes the
/// card away.
public struct PredictionPush: Decodable, Sendable, Equatable {
    public var channelID: String
    public var view: PredictionView?

    enum CodingKeys: String, CodingKey {
        case view
        case channelID = "channel_id"
    }
}

public struct BetPreset: Decodable, Sendable, Equatable, Hashable {
    public var label: String
    public var amount: Int64
}

/// What a bet of an amount on one side would do, in the words the card and its confirm show.
/// Asking spends nothing.
public struct BetView: Decodable, Sendable, Equatable {
    /// The amount as Rust clamped it.
    public var amount: Int64
    public var max: Int64
    public var presets: [BetPreset]
    /// Predicting is possible as asked.
    public var ok: Bool
    /// Why not, when `ok` is false: shown in place of the button.
    public var problem: String?
    /// "Predict 100 on Yes" / "Add 100 to Yes".
    public var button: String
    /// "Win up to 272".
    public var winLine: String
    public var confirmTitle: String
    public var confirmLine: String
    public var confirmNote: String?
    /// "12,850 Nooks", when the balance is known.
    public var balanceLine: String?

    enum CodingKeys: String, CodingKey {
        case amount, max, presets, ok, problem, button
        case winLine = "win_line"
        case confirmTitle = "confirm_title"
        case confirmLine = "confirm_line"
        case confirmNote = "confirm_note"
        case balanceLine = "balance_line"
    }
}

/// What `predict` answered: the toast's words, and the card as it now stands.
public struct PredictOutcome: Decodable, Sendable, Equatable {
    public var success: Bool
    public var message: String
    public var view: PredictionView?
}

// MARK: Polls

public struct PollView: Decodable, Sendable, Equatable {
    public var channelID: String
    public var pollID: String
    public var title: String
    /// `open`, `closed` or `ended`.
    public var phase: String
    public var deadlineMS: Int64?
    public var badge: String?
    public var choices: [ChoiceView]
    public var canVote: Bool
    public var myChoiceID: String?
    /// "Tap a choice to vote", "You voted Malenia".
    public var footer: String
    /// "1,204 voters".
    public var votersLine: String
    /// "Malenia won · 58%", once ended.
    public var resultLine: String?

    enum CodingKeys: String, CodingKey {
        case title, phase, badge, choices, footer
        case channelID = "channel_id"
        case pollID = "poll_id"
        case deadlineMS = "deadline_ms"
        case canVote = "can_vote"
        case myChoiceID = "my_choice_id"
        case votersLine = "voters_line"
        case resultLine = "result_line"
    }

    public var isOpen: Bool { phase == "open" }
}

public struct ChoiceView: Decodable, Sendable, Equatable, Identifiable, Hashable {
    public var id: String
    public var title: String
    public var percent: Int
    public var votesLabel: String
    public var mine: Bool
    public var winner: Bool

    enum CodingKeys: String, CodingKey {
        case id, title, percent, mine, winner
        case votesLabel = "votes_label"
    }
}

/// The `poll-state` push. A null view takes the card away.
public struct PollPush: Decodable, Sendable, Equatable {
    public var channelID: String
    public var view: PollView?

    enum CodingKeys: String, CodingKey {
        case view
        case channelID = "channel_id"
    }
}

// MARK: Drawing helpers (no decisions: only the clock and the settings read)

/// The countdown a card ticks to its deadline.
public enum LiveVoteClock {
    /// `m:ss` left until `deadlineMS`, never below 0:00.
    public static func text(deadlineMS: Int64, nowMS: Int64) -> String {
        // Rounded up, so the last second shows 0:01 rather than 0:00 while time is left.
        let left = max(0, (deadlineMS - nowMS + 999) / 1000)
        let seconds = left % 60
        return "\(left / 60):\(seconds < 10 ? "0" : "")\(seconds)"
    }

    public static func nowMS(_ date: Date = Date()) -> Int64 {
        Int64((date.timeIntervalSince1970 * 1000).rounded())
    }
}

/// The settings the cards read: presentation only.
public struct LiveVoteSettings: Sendable, Equatable {
    public var showPolls = true
    public var showPredictions = true
    /// `chat_overlay_order` is `poll-first`.
    public var pollFirst = false
    /// `chat_design.polls_start_collapsed`: a new poll opens folded.
    public var pollsStartCollapsed = false

    public init(showPolls: Bool = true, showPredictions: Bool = true, pollFirst: Bool = false,
                pollsStartCollapsed: Bool = false) {
        self.showPolls = showPolls
        self.showPredictions = showPredictions
        self.pollFirst = pollFirst
        self.pollsStartCollapsed = pollsStartCollapsed
    }

    /// From `get_settings_view`'s `values` (defaults already filled in by Rust).
    public init(values: [String: Any]) {
        showPolls = SettingsPath.value(in: values, "show_polls") as? Bool ?? true
        showPredictions = SettingsPath.value(in: values, "show_predictions") as? Bool ?? true
        pollFirst = SettingsPath.value(in: values, "chat_overlay_order") as? String == "poll-first"
        pollsStartCollapsed = SettingsPath.value(in: values, "chat_design.polls_start_collapsed") as? Bool ?? false
    }
}

// MARK: The top of chat (`chat_top_stack`, services/chat_top_stack.rs)

/// A card the top of chat can hold.
public enum TopCard: String, Codable, Sendable, Equatable, CaseIterable {
    case prediction, poll, drop
}

/// What the viewer did to the stack: the card they last opened and the ones they folded. Rust
/// reads both to decide which card is open; the shells only remember them.
public struct StackViewer: Sendable, Equatable {
    public private(set) var opened: TopCard?
    public private(set) var folded: Set<TopCard> = []

    public init(opened: TopCard? = nil, folded: Set<TopCard> = []) {
        self.opened = opened
        self.folded = folded
    }

    /// A tap on a card's head. Opening a card folds the others shown, since one card is open at a
    /// time; folding one folds every card shown, so none opens by itself in its place.
    public mutating func tap(_ card: TopCard, wasOpen: Bool, shown: [TopCard]) {
        if wasOpen {
            opened = nil
            folded.formUnion(shown)
            folded.insert(card)
        } else {
            opened = card
            folded.formUnion(shown.filter { $0 != card })
            folded.remove(card)
        }
    }

    /// A new poll, prediction or drop is a new card: what the viewer did to the old one goes.
    public mutating func forget(_ card: TopCard) {
        folded.remove(card)
        if opened == card { opened = nil }
    }
}

/// What `chat_top_stack` is asked: the cards present, the order setting and what the viewer did.
public struct StackRequest: Encodable, Sendable, Equatable {
    public var cards: [TopCard]
    /// `prediction-first` or `poll-first`.
    public var order: String
    public var pollsStartCollapsed: Bool
    public var opened: TopCard?
    public var folded: [TopCard]
    /// The chat column's height in points; 0 while it is not measured yet.
    public var chatHeight: Double

    enum CodingKeys: String, CodingKey {
        case cards, order, opened, folded
        case pollsStartCollapsed = "polls_start_collapsed"
        case chatHeight = "chat_height"
    }

    public init(cards: [TopCard], settings: LiveVoteSettings, viewer: StackViewer, chatHeight: Double) {
        self.cards = cards
        order = settings.pollFirst ? "poll-first" : "prediction-first"
        pollsStartCollapsed = settings.pollsStartCollapsed
        opened = viewer.opened
        // A fixed order, so the same state always asks the same question.
        folded = TopCard.allCases.filter(viewer.folded.contains)
        self.chatHeight = chatHeight
    }

    /// The command's arguments: `{"request": {...}}`.
    public var argsJSON: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        let body = (try? encoder.encode(self)).map { String(decoding: $0, as: UTF8.self) } ?? "{}"
        return #"{"request":\#(body)}"#
    }
}

/// One card of the stack, top to bottom: open with its body, or its one-line strip.
public struct StackSlot: Decodable, Sendable, Equatable, Hashable, Identifiable {
    public var kind: String
    public var open: Bool

    public init(kind: String, open: Bool) {
        self.kind = kind
        self.open = open
    }

    /// Nil for a kind this shell does not draw.
    public var card: TopCard? { TopCard(rawValue: kind) }
    public var id: String { kind }
}

/// Rust's layout of the top of chat.
public struct StackLayout: Decodable, Sendable, Equatable, Hashable {
    public var slots: [StackSlot]
    /// The most height the whole stack may take, in points; 0 when the chat height is unknown.
    public var maxHeight: Double

    enum CodingKeys: String, CodingKey {
        case slots
        case maxHeight = "max_height"
    }

    public init(slots: [StackSlot], maxHeight: Double) {
        self.slots = slots
        self.maxHeight = maxHeight
    }

    /// The same cards with only `card` open (none for nil): what a tap shows at once, before
    /// Rust's answer to the same question lands.
    public func showing(open card: TopCard?) -> StackLayout {
        StackLayout(slots: slots.map { StackSlot(kind: $0.kind, open: $0.card == card && card != nil) }, maxHeight: maxHeight)
    }

    /// Every card as a strip, uncapped: drawn only when the core cannot answer, so a card never
    /// goes missing.
    public static func strips(_ cards: [TopCard]) -> StackLayout {
        StackLayout(slots: cards.map { StackSlot(kind: $0.rawValue, open: false) }, maxHeight: 0)
    }
}
