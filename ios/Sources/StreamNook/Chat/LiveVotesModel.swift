import Foundation
import Observation
import SNKit

/// One room's top of chat: the poll and prediction views Rust pushes for the room, the drop shown
/// over it, the settings that decide which cards show, what the viewer opened or folded, Rust's
/// layout of the stack, a vote waiting on Twitch, and the bet being shaped. Every word, number
/// and layout comes from Rust; this only keeps the viewer's picks and asks Rust what they do.
@Observable @MainActor
final class LiveVotesModel {
    @ObservationIgnored private(set) weak var session: ChatSession?
    @ObservationIgnored let core: CoreBridge

    private(set) var poll: PollView?
    private(set) var prediction: PredictionView?
    private(set) var settings = LiveVoteSettings()

    /// Rust's layout of the stack (`chat_top_stack`), nil until it first answers.
    private(set) var layout: StackLayout?
    /// What the viewer opened and folded, which Rust reads.
    private(set) var viewer = StackViewer()
    /// The drop shown over this room, by id: the watched stream's, only on its own room.
    private(set) var dropID: String?
    /// The chat column's height as last asked about, in points.
    private(set) var chatHeight: Double = 0
    /// A height a drive step forces, to check the short-column rule on any phone.
    var forcedHeight: Double? {
        didSet { applyHeight() }
    }

    /// The choice just tapped, shown as the viewer's at once until Rust's next push confirms it.
    private(set) var pendingChoice: String?

    /// The side the bet is on.
    private(set) var side: String?
    /// The amount as typed in the field, digits only; empty when a preset (or Rust's default) holds.
    private(set) var typed = ""
    /// The preset chip last tapped, by position (two presets can carry the same amount).
    private(set) var presetIndex: Int?
    /// Rust's words for the bet as it stands.
    private(set) var bet: BetView?
    /// The confirm before any points are spent.
    var confirmOpen = false
    private(set) var predicting = false
    /// Why the last placement was refused (not enough points, locked), shown as one line in the
    /// card until the side or the amount changes.
    private(set) var failure: String?

    /// Shows a line from Rust as a toast: true for a success, false for a failure.
    @ObservationIgnored var onToast: ((String, Bool) -> Void)?

    @ObservationIgnored private var tasks: [Task<Void, Never>] = []
    @ObservationIgnored private var betTask: Task<Void, Never>?
    @ObservationIgnored private var stackTask: Task<Void, Never>?
    @ObservationIgnored private var stackAsks = 0
    /// Each card's identity, so a new poll, prediction or drop starts fresh.
    @ObservationIgnored private var identities: [TopCard: String] = [:]
    /// The column's height as measured, and whether a card's own field holds the height.
    @ObservationIgnored private var measuredHeight: Double = 0
    @ObservationIgnored private var heightHeld = false
    @ObservationIgnored private var stackFailed = false

    init(session: ChatSession) {
        self.session = session
        core = session.core
    }

    /// The cards to draw, top to bottom, each open or a strip: Rust's layout, less any card that
    /// left since it answered (a card that just arrived waits for Rust's next answer).
    var slots: [StackSlot] {
        let present = Set(presentCards)
        return (layout?.slots ?? []).filter { $0.card.map(present.contains) ?? false }
    }

    /// The most height the stack may take; 0 when uncapped.
    var maxHeight: CGFloat { CGFloat(layout?.maxHeight ?? 0) }

    /// The cards there are to show, before Rust orders them.
    var presentCards: [TopCard] {
        var cards: [TopCard] = []
        if settings.showPredictions, prediction != nil { cards.append(.prediction) }
        if settings.showPolls, poll != nil { cards.append(.poll) }
        if dropID != nil { cards.append(.drop) }
        return cards
    }

    var stackRequest: StackRequest {
        StackRequest(cards: presentCards, settings: settings, viewer: viewer, chatHeight: chatHeight)
    }

    /// The room's id, which every push and command is keyed by.
    private var roomID: String? { session?.roomID }

    // MARK: Lifecycle

    /// Listens for both pushes and the settings, then reads the current cards once the room's id
    /// is known. Only Twitch rooms have either.
    func start() {
        guard tasks.isEmpty, let session, session.provider == "twitch" else { return }
        let events = core.events
        let predictions = events.stream("prediction-state")
        let polls = events.stream("poll-state")
        let settingsChanges = events.stream("streamnook-settings-updated")
        tasks.append(Task { [weak self] in
            for await event in predictions {
                guard let self else { return }
                guard let push = try? JSONDecoder().decode(PredictionPush.self, from: Data(event.payloadJSON.utf8)),
                      push.channelID == self.roomID else { continue }
                self.take(prediction: push.view)
            }
        })
        tasks.append(Task { [weak self] in
            for await event in polls {
                guard let self else { return }
                guard let push = try? JSONDecoder().decode(PollPush.self, from: Data(event.payloadJSON.utf8)),
                      push.channelID == self.roomID else { continue }
                self.take(poll: push.view)
            }
        })
        tasks.append(Task { [weak self] in
            for await _ in settingsChanges {
                guard let self else { return }
                await self.loadSettings()
            }
        })
        tasks.append(Task { [weak self] in
            await self?.loadSettings()
            // The room's id can arrive with the first history or live row.
            for _ in 0..<120 {
                guard let self else { return }
                if self.roomID != nil { break }
                try? await Task.sleep(for: .milliseconds(500))
            }
            await self?.refresh()
        })
    }

    func stop() {
        tasks.forEach { $0.cancel() }
        tasks = []
        betTask?.cancel()
        stackTask?.cancel()
    }

    /// Reads both cards fresh (also asks Rust to follow the room's poll).
    func refresh() async {
        guard let session, let roomID = session.roomID else { return }
        let login = session.login
        if let push = try? await core.invoke("get_channel_prediction", args: ["channelId": roomID, "channelLogin": login],
                                             as: PredictionPush.self), push.channelID == roomID {
            take(prediction: push.view)
        }
        if let json = try? await core.invoke("watch_channel_poll_view", args: ["channelId": roomID]),
           let view = try? JSONDecoder().decode(PollView?.self, from: Data(json.utf8)) {
            take(poll: view)
        }
    }

    private func loadSettings() async {
        guard let json = try? await core.invoke("get_settings_view", "{}"),
              let view = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
              let values = view["values"] as? [String: Any] else { return }
        let next = LiveVoteSettings(values: values)
        guard next != settings else { return }
        settings = next
        cardsChanged()
    }

    // MARK: The stack

    /// A tap on a card's head: opens a strip (folding the rest) or folds the open card. The tap
    /// shows at once; Rust's answer to the same question follows.
    func tap(_ card: TopCard) {
        let shown = slots
        let wasOpen = shown.first { $0.card == card }?.open ?? false
        viewer.tap(card, wasOpen: wasOpen, shown: shown.compactMap(\.card))
        layout = layout?.showing(open: wasOpen ? nil : card)
        askStack()
    }

    /// The drop shown over this room changed (nil: none).
    func setDrop(_ id: String?) {
        guard id != dropID else { return }
        dropID = id
        cardsChanged()
    }

    /// The chat column was measured. Resizes settle a beat before Rust is asked.
    func setChatHeight(_ height: CGFloat) {
        measuredHeight = Double(height.rounded())
        applyHeight()
    }

    /// While a card's own field is being typed in, the number pad's shrink is not a short column:
    /// the card keeps the room it had, and the real height applies once typing stops.
    func holdHeight(_ held: Bool) {
        heightHeld = held
        applyHeight()
    }

    private func applyHeight() {
        let next = forcedHeight ?? measuredHeight
        guard next > 0, next != chatHeight, !(heightHeld && forcedHeight == nil && next < chatHeight) else { return }
        let first = chatHeight == 0
        chatHeight = next
        askStack(after: first || forcedHeight != nil ? .zero : .milliseconds(150))
    }

    /// A card came, went or changed its identity: a new one starts with nothing opened or folded.
    private func cardsChanged() {
        let now: [TopCard: String] = [
            .poll: poll?.pollID ?? "", .prediction: prediction?.predictionID ?? "", .drop: dropID ?? "",
        ]
        for (card, id) in now where identities[card, default: ""] != id { viewer.forget(card) }
        identities = now
        askStack()
    }

    /// Asks Rust for the layout; a newer question makes an older answer moot.
    private func askStack(after delay: Duration = .zero) {
        stackTask?.cancel()
        stackAsks += 1
        let ask = stackAsks
        let request = stackRequest
        guard !request.cards.isEmpty else {
            layout = StackLayout(slots: [], maxHeight: layout?.maxHeight ?? 0)
            return
        }
        stackTask = Task { [weak self] in
            if delay > .zero { try? await Task.sleep(for: delay) }
            guard !Task.isCancelled, let self else { return }
            do {
                let json = try await self.core.invoke("chat_top_stack", request.argsJSON)
                let answer = try JSONDecoder().decode(StackLayout.self, from: Data(json.utf8))
                guard ask == self.stackAsks else { return }
                self.layout = answer
            } catch {
                guard ask == self.stackAsks else { return }
                if !self.stackFailed {
                    self.stackFailed = true
                    log.error("chat_top_stack failed: \(String(describing: error), privacy: .public)")
                }
                // A core without the command still shows every card, as strips the viewer opens.
                self.layout = StackLayout.strips(request.cards).showing(open: self.viewer.opened)
            }
        }
    }

    // MARK: Poll

    func take(poll next: PollView?) {
        // Rust's push confirms the vote (or the poll moved on); until then the tap stays shown.
        if pendingChoice != nil,
           next == nil || next?.pollID != poll?.pollID || next?.myChoiceID != nil || next?.isOpen == false {
            pendingChoice = nil
        }
        poll = next
        cardsChanged()
    }

    /// Whether the viewer's mark is on this choice (theirs per Rust, or just tapped).
    func isMine(_ choice: ChoiceView) -> Bool { choice.mine || pendingChoice == choice.id }

    /// A vote is offered: Rust says so and no tap is waiting.
    var canVote: Bool { (poll?.canVote ?? false) && pendingChoice == nil }

    /// A tap on a choice: no confirm, since a vote spends nothing. Marked at once; a refusal takes
    /// the mark back with Rust's reason.
    func vote(_ choice: ChoiceView) {
        guard canVote, let poll, let roomID else { return }
        pendingChoice = choice.id
        Task { [weak self] in
            guard let self else { return }
            do {
                _ = try await self.core.invoke("vote_on_poll", args: [
                    "pollId": poll.pollID, "choiceId": choice.id, "channelId": roomID,
                ])
            } catch {
                if self.pendingChoice == choice.id { self.pendingChoice = nil }
                self.onToast?("\(error)", false)
            }
        }
    }

    // MARK: Prediction

    func take(prediction next: PredictionView?) {
        let previous = prediction
        prediction = next
        cardsChanged()
        guard let next else {
            resetBet()
            confirmOpen = false
            return
        }
        if previous?.predictionID != next.predictionID { resetBet() }
        guard next.canPredict else {
            bet = nil
            betTask?.cancel()
            confirmOpen = false
            return
        }
        // A staked side is the only one open to the viewer; otherwise keep the pick, or start on
        // the first side so the card always has a bet to show.
        if let picked = next.picked {
            side = picked
        } else if next.outcome(side) == nil {
            side = next.outcomes.first?.id
        }
        // The pool moved: Rust's "win up to" may have too.
        askBet()
    }

    private func resetBet() {
        failure = nil
        side = nil
        typed = ""
        presetIndex = nil
        bet = nil
        betTask?.cancel()
    }

    /// Whether this side can be picked now.
    func canPick(_ outcome: OutcomeView) -> Bool {
        guard let prediction, prediction.canPredict else { return false }
        return prediction.picked == nil || prediction.picked == outcome.id
    }

    func pick(_ outcomeID: String) {
        guard let prediction, let outcome = prediction.outcome(outcomeID), canPick(outcome), side != outcomeID else { return }
        failure = nil
        side = outcomeID
        askBet()
    }

    func choosePreset(_ index: Int) {
        guard let presets = bet?.presets, presets.indices.contains(index) else { return }
        presetIndex = index
        typed = ""
        failure = nil
        askBet()
    }

    func type(_ text: String) {
        let digits = text.filter(\.isASCII).filter(\.isNumber)
        guard digits != typed else { return }
        typed = digits
        failure = nil
        if !digits.isEmpty { presetIndex = nil }
        askBet()
    }

    /// The amount Rust is asked about: what was typed, else the preset's amount, else nothing
    /// (Rust's own starting amount).
    private var askedAmount: String? {
        if !typed.isEmpty { return typed }
        if let presetIndex, let presets = bet?.presets, presets.indices.contains(presetIndex) {
            return String(presets[presetIndex].amount)
        }
        return bet.map { String($0.amount) }
    }

    /// The preset chip drawn as chosen.
    var chosenPreset: Int? {
        guard typed.isEmpty, let bet else { return nil }
        if let presetIndex, bet.presets.indices.contains(presetIndex), bet.presets[presetIndex].amount == bet.amount {
            return presetIndex
        }
        return bet.presets.firstIndex { $0.amount == bet.amount }
    }

    /// Asks Rust what the bet would do, a beat after the last change so typing does not flood it.
    private func askBet() {
        betTask?.cancel()
        guard let side, let roomID, prediction?.canPredict == true else { return }
        let amount = askedAmount
        betTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled, let self else { return }
            let answer = try? await self.core.invoke("prediction_bet", args: [
                "channelId": roomID, "outcomeId": side, "amount": amount,
            ], as: BetView?.self)
            guard !Task.isCancelled, self.side == side else { return }
            self.bet = answer
        }
    }

    /// The card's button: the confirm, never the bet itself.
    func openConfirm() {
        guard bet?.ok == true, prediction?.canPredict == true else { return }
        confirmOpen = true
    }

    /// The confirm's Predict: the only place points are spent.
    func confirmPredict() async {
        guard let bet, bet.ok, let side, let roomID, !predicting else { return }
        predicting = true
        defer { predicting = false }
        do {
            let outcome = try await core.invoke("predict", args: [
                "channelId": roomID, "outcomeId": side, "amount": bet.amount,
            ], as: PredictOutcome.self)
            confirmOpen = false
            // Placed: Rust's status line says so in the card, and a toast once. Refused: one line
            // in the card, beside the button.
            if outcome.success {
                onToast?(outcome.message, true)
                Haptics.commit()
                typed = ""
                presetIndex = nil
                failure = nil
            } else {
                failure = outcome.message
            }
            if let view = outcome.view { take(prediction: view) }
        } catch {
            confirmOpen = false
            failure = "\(error)"
        }
    }
}

/// Each room's cards model, kept for the room's life so what the viewer opened or folded and a
/// half-shaped bet survive switching tabs or turning the phone.
@MainActor
final class LiveVotesRegistry {
    static let shared = LiveVotesRegistry()
    private var models: [ObjectIdentifier: LiveVotesModel] = [:]

    func model(for session: ChatSession) -> LiveVotesModel {
        let key = ObjectIdentifier(session)
        if let known = models[key], known.session === session { return known }
        // Rooms that closed: their models stop listening.
        for (id, model) in models where model.session == nil || id == key {
            model.stop()
            models[id] = nil
        }
        let made = LiveVotesModel(session: session)
        models[key] = made
        return made
    }

    /// The model of a room, if its cards were ever drawn.
    func existing(for session: ChatSession) -> LiveVotesModel? {
        models[ObjectIdentifier(session)].flatMap { $0.session === session ? $0 : nil }
    }
}
