import Observation
import SNKit
import SwiftUI

/// The stream manager behind the chat menu's "Manage channel" row: Rust's view of what this
/// account may do here (services/stream_manager.rs), as a page of sections that each open
/// their form. Every write goes through `stream_manager_act`; nothing is decided here.
@Observable @MainActor
final class ManageChannelModel {
    let core: CoreBridge
    let channelID: String
    let channelLogin: String
    @ObservationIgnored weak var toasts: Toasts?
    private(set) var view: StreamManagerView?
    var queue: RewardQueue?
    private(set) var error: String?
    private(set) var busy = false

    init(core: CoreBridge, channelID: String, channelLogin: String, toasts: Toasts?) {
        self.core = core
        self.channelID = channelID
        self.channelLogin = channelLogin
        self.toasts = toasts
    }

    func load() async {
        do {
            view = try await core.invoke("get_stream_manager", args: ["channelId": channelID, "channelLogin": channelLogin],
                                         as: StreamManagerView.self)
            error = nil
        } catch {
            self.error = "Could not read the channel. Pull to try again."
        }
    }

    func loadQueue() async {
        queue = try? await core.invoke("get_reward_queue", args: ["channelId": channelID], as: RewardQueue.self)
    }

    /// Runs one action; true when Twitch took it. A refusal shows as a toast in Rust's words.
    @discardableResult
    func act(_ action: ManagerAction) async -> Bool {
        guard !busy else { return false }
        busy = true
        defer { busy = false }
        do {
            let outcome = try await core.invoke("stream_manager_act", args: [
                "channelId": channelID, "channelLogin": channelLogin, "action": action.argument,
            ], as: ManagerOutcome.self)
            if let v = outcome.view { view = v }
            if let q = outcome.queue { queue = q }
            if !outcome.ok { toasts?.show(outcome.message) }
            return outcome.ok
        } catch {
            toasts?.show("That did not go through")
            return false
        }
    }

    func check(_ draft: StreamInfoDraft) async -> InfoCheck? {
        try? await core.invoke("check_stream_info", args: ["channelId": channelID, "draft": draft.argument], as: InfoCheck.self)
    }

    func searchRaid(_ query: String) async -> [RaidTarget] {
        (try? await core.invoke("search_raid_targets", args: ["query": query, "channelId": channelID], as: [RaidTarget].self)) ?? []
    }

    func searchCategories(_ query: String) async -> [ManagerCategory] {
        (try? await core.invoke("search_categories", args: ["query": query, "limit": 8], as: [ManagerCategory].self)) ?? []
    }
}

enum ManagePage: String, Hashable {
    case info, poll, prediction, raid, ads, rewards, announce
}

struct ManageChannelSheet: View {
    @State private var model: ManageChannelModel
    @State private var path: [ManagePage] = []
    @State private var confirmShield = false

    init(core: CoreBridge, channelID: String, channelLogin: String, toasts: Toasts?) {
        _model = State(initialValue: ManageChannelModel(core: core, channelID: channelID, channelLogin: channelLogin, toasts: toasts))
    }

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                if let view = model.view {
                    home(view)
                } else {
                    VStack {
                        Spacer()
                        if let error = model.error {
                            Text(error).font(.sn(14)).foregroundStyle(SN.textSecondary)
                        } else {
                            ProgressView()
                        }
                        Spacer()
                    }
                    .frame(maxWidth: .infinity)
                }
            }
            .navigationTitle("Manage channel")
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(for: ManagePage.self) { page in
                destination(page)
            }
        }
        .task { await model.load() }
        .confirmationDialog("Turn on Shield Mode?", isPresented: $confirmShield, titleVisibility: .visible) {
            Button("Turn on Shield Mode", role: .destructive) { Task { await model.act(.shield(on: true)) } }
        } message: {
            Text("Twitch's Shield Mode can clear chat and tighten the chat modes.")
        }
    }

    @ViewBuilder
    private func destination(_ page: ManagePage) -> some View {
        switch page {
        case .info: StreamInfoFormView(model: model) { path.removeLast() }
        case .poll: PollFormView(model: model, prediction: false) { path.removeLast() }
        case .prediction: PollFormView(model: model, prediction: true) { path.removeLast() }
        case .raid: RaidFormView(model: model)
        case .ads: AdsFormView(model: model) { path.removeLast() }
        case .rewards: RewardQueueView(model: model)
        case .announce: AnnounceFormView(model: model) { path.removeLast() }
        }
    }

    private func home(_ view: StreamManagerView) -> some View {
        List {
            if !view.streamRows.isEmpty {
                Section {
                    ForEach(view.streamRows) { row in
                        streamRow(row, view)
                    }
                } header: {
                    Text("Stream")
                } footer: {
                    Text(view.roleLine).font(.sn(12.5)).foregroundStyle(SN.textSecondary)
                }
            }
            if let chat = view.chat {
                Section("Chat") {
                    ForEach(chat.modes) { mode in
                        ChatModeRowView(mode: mode, busy: model.busy) { on, value in
                            Task { await model.act(.chatMode(mode: mode.kind, on: on, value: value)) }
                        }
                    }
                    if let shield = chat.shield {
                        Toggle("Shield Mode", isOn: Binding(
                            get: { shield },
                            set: { on in
                                if on { confirmShield = true } else { Task { await model.act(.shield(on: false)) } }
                            }
                        ))
                        .disabled(model.busy)
                    }
                    if view.canAnnounce {
                        NavigationLink("Announce", value: ManagePage.announce)
                    }
                }
            }
            if !view.problems.isEmpty {
                Section {
                    ForEach(view.problems, id: \.self) { Text($0).font(.sn(13)).foregroundStyle(SN.textSecondary) }
                }
            }
        }
        .refreshable { await model.load() }
        .font(.sn(15.5))
        .tint(SN.accent)
    }

    @ViewBuilder
    private func streamRow(_ row: ManagerSectionRow, _ view: StreamManagerView) -> some View {
        if row.id == "poll", view.poll != nil {
            VStack(alignment: .leading, spacing: 10) {
                RowLabels(row: row)
                HStack(spacing: 8) {
                    PillButton("End", busy: model.busy) { await model.act(.endPoll(showResults: true)) }
                    PillButton("Remove", danger: true, busy: model.busy) { await model.act(.endPoll(showResults: false)) }
                }
            }
            .padding(.vertical, 4)
        } else if row.id == "prediction", let p = view.prediction {
            VStack(alignment: .leading, spacing: 10) {
                RowLabels(row: row, label: p.status == "locked" ? "Prediction locked" : row.label)
                FlowRow {
                    if p.status == "active" {
                        PillButton("Lock", busy: model.busy) { await model.act(.lockPrediction) }
                    } else {
                        ForEach(p.outcomes) { o in
                            PillButton("\(o.title) won", busy: model.busy) { await model.act(.resolvePrediction(outcomeID: o.id)) }
                        }
                    }
                    PillButton("Refund", danger: true, busy: model.busy) { await model.act(.cancelPrediction) }
                }
            }
            .padding(.vertical, 4)
        } else if let page = ManagePage(rawValue: row.id) {
            NavigationLink(value: page) {
                HStack {
                    RowLabels(row: row)
                    Spacer()
                    if let badge = row.badge {
                        Text(badge).font(.sn(13.5)).foregroundStyle(SN.textSecondary).monospacedDigit()
                    }
                }
            }
        }
    }
}

private struct RowLabels: View {
    let row: ManagerSectionRow
    var label: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label ?? row.label).foregroundStyle(SN.textPrimary)
            if let detail = row.detail {
                Text(detail).font(.sn(13)).foregroundStyle(SN.textSecondary).lineLimit(1)
            }
        }
    }
}

private struct PillButton: View {
    let title: String
    var danger = false
    let busy: Bool
    let action: () async -> Void

    init(_ title: String, danger: Bool = false, busy: Bool, action: @escaping () async -> Void) {
        self.title = title
        self.danger = danger
        self.busy = busy
        self.action = action
    }

    var body: some View {
        Button {
            Task { await action() }
        } label: {
            Text(title)
                .font(.sn(14, 600))
                .padding(.horizontal, 14)
                .frame(minHeight: 36)
                .foregroundStyle(danger ? SN.error : SN.textPrimary)
                .background(danger ? SN.error.opacity(0.15) : Color.white.opacity(0.1), in: Capsule())
        }
        .buttonStyle(.plain)
        .disabled(busy)
    }
}

/// Wraps its buttons onto more lines rather than squeezing them.
private struct FlowRow<Content: View>: View {
    @ViewBuilder let content: Content

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) { content }
            VStack(alignment: .leading, spacing: 8) { content }
        }
    }
}

private struct ChatModeRowView: View {
    let mode: ManagerChat.Mode
    let busy: Bool
    let change: (Bool, UInt32?) -> Void

    var body: some View {
        HStack {
            Toggle(mode.label, isOn: Binding(get: { mode.on }, set: { change($0, mode.value) }))
        }
        .disabled(busy)
        if mode.on, !mode.options.isEmpty {
            Picker("Length", selection: Binding(
                get: { mode.value ?? mode.options.first?.value ?? 0 },
                set: { change(true, $0) }
            )) {
                ForEach(mode.options, id: \.value) { Text($0.label).tag($0.value) }
            }
            .disabled(busy)
            .foregroundStyle(SN.textSecondary)
        }
    }
}

// MARK: - Stream info

private struct StreamInfoFormView: View {
    let model: ManageChannelModel
    let done: () -> Void
    @State private var draft = StreamInfoDraft()
    @State private var check: InfoCheck?
    @State private var newTag = ""
    @State private var categoryQuery = ""
    @State private var categories: [ManagerCategory] = []
    @State private var picking = false

    var body: some View {
        Form {
            Section {
                TextField("What are you streaming?", text: $draft.title, axis: .vertical)
                    .keyboardHideButton()
            } header: {
                HStack { Text("Title"); Spacer(); Text(check?.titleCount ?? "").monospacedDigit() }
            } footer: {
                if let p = check?.problem("title") { Text(p).foregroundStyle(SN.error) }
            }
            Section("Category") {
                Button {
                    picking = true
                } label: {
                    HStack(spacing: 10) {
                        if let url = draft.category?.boxArt {
                            AsyncImage(url: url) { $0.resizable().scaledToFill() } placeholder: { Color.white.opacity(0.08) }
                                .frame(width: 24, height: 32)
                                .clipShape(RoundedRectangle(cornerRadius: 3))
                        }
                        Text(draft.category?.name ?? "Pick a category")
                            .foregroundStyle(draft.category == nil ? SN.textSecondary : SN.textPrimary)
                        Spacer()
                        Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(SN.textMuted)
                    }
                }
            }
            Section {
                ForEach(Array(draft.tags.enumerated()), id: \.offset) { i, tag in
                    HStack {
                        Text(tag).foregroundStyle(check?.problem("tag:\(i)") == nil ? SN.textPrimary : SN.error)
                        Spacer()
                        Button {
                            draft.tags.remove(at: i)
                        } label: {
                            Image(systemName: "xmark.circle.fill").foregroundStyle(SN.textMuted)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Remove \(tag)")
                    }
                }
                if draft.tags.count < (model.view?.limits.tags ?? 10) {
                    TextField("Add a tag", text: $newTag)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .onSubmit(addTag)
                }
            } header: {
                HStack { Text("Tags"); Spacer(); Text(check?.tagsCount ?? "").monospacedDigit() }
            } footer: {
                if let p = check?.tagProblem { Text(p).foregroundStyle(SN.error) }
            }
            if let languages = model.view?.info?.languages, !languages.isEmpty {
                Section("Language") {
                    Picker("Language", selection: $draft.language) {
                        ForEach(languages, id: \.code) { Text($0.name).tag($0.code) }
                    }
                }
            }
            if let preview = check?.preview {
                Section("What viewers see") {
                    HStack(alignment: .top, spacing: 12) {
                        RoundedRectangle(cornerRadius: 8)
                            .fill(Color.white.opacity(0.06))
                            .overlay {
                                if let t = preview.thumbnailURL, let url = URL(string: t) {
                                    AsyncImage(url: url) { $0.resizable().scaledToFill() } placeholder: { Color.clear }
                                }
                            }
                            .clipShape(RoundedRectangle(cornerRadius: 8))
                            .frame(width: 120, height: 68)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(preview.title.isEmpty ? "Your title" : preview.title)
                                .font(.sn(14, 600)).foregroundStyle(SN.textPrimary).lineLimit(2)
                            Text(preview.channelName).font(.sn(13)).foregroundStyle(SN.textSecondary)
                            Text(preview.categoryName).font(.sn(13)).foregroundStyle(SN.textSecondary)
                        }
                    }
                    .padding(.vertical, 4)
                }
            }
        }
        .navigationTitle("Stream info")
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") {
                    Task { if await model.act(.saveInfo(draft)) { done() } }
                }
                .disabled(model.busy || !(check?.ok ?? false) || !(check?.changed ?? false))
            }
        }
        .onAppear { if draft == StreamInfoDraft() { draft = StreamInfoDraft(model.view?.info) } }
        .task(id: draft) {
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled else { return }
            check = await model.check(draft)
        }
        .sheet(isPresented: $picking) {
            NavigationStack {
                List(categories, id: \.id) { c in
                    Button {
                        draft.category = c
                        picking = false
                    } label: {
                        HStack(spacing: 10) {
                            AsyncImage(url: c.boxArt) { $0.resizable().scaledToFill() } placeholder: { Color.white.opacity(0.08) }
                                .frame(width: 24, height: 32)
                                .clipShape(RoundedRectangle(cornerRadius: 3))
                            Text(c.name).foregroundStyle(SN.textPrimary)
                        }
                    }
                }
                .searchable(text: $categoryQuery, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search categories")
                .navigationTitle("Category")
                .navigationBarTitleDisplayMode(.inline)
                .task(id: categoryQuery) {
                    try? await Task.sleep(for: .milliseconds(180))
                    guard !Task.isCancelled, !categoryQuery.trimmingCharacters(in: .whitespaces).isEmpty else { return }
                    categories = await model.searchCategories(categoryQuery)
                }
            }
            .presentationDetents([.large])
        }
    }

    private func addTag() {
        let tag = newTag.trimmingCharacters(in: .whitespaces)
        guard !tag.isEmpty else { return }
        draft.tags.append(tag)
        newTag = ""
    }
}

// MARK: - Poll and prediction

private struct PollFormView: View {
    let model: ManageChannelModel
    let prediction: Bool
    let done: () -> Void
    @State private var title = ""
    @State private var options = ["", ""]
    @State private var seconds: UInt32 = 120
    @State private var pointsPerVote = ""

    private var limits: ManagerLimits? { model.view?.limits }
    private var maxOptions: Int { (prediction ? limits?.predictionOutcomesMax : limits?.pollChoicesMax) ?? 5 }
    private var minOptions: Int { (prediction ? limits?.predictionOutcomesMin : limits?.pollChoicesMin) ?? 2 }
    private var filled: [String] { options.map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty } }
    private var minSeconds: UInt32 { (prediction ? limits?.predictionSecondsMin : limits?.pollSecondsMin) ?? 15 }

    var body: some View {
        Form {
            Section(prediction ? "Prediction" : "Question") {
                TextField(prediction ? "Do we win this one?" : "What should we play next?", text: $title)
            }
            Section(prediction ? "Outcomes" : "Choices") {
                ForEach(options.indices, id: \.self) { i in
                    TextField("Option \(i + 1)", text: $options[i])
                }
                .onDelete { idx in if options.count > minOptions { options.remove(atOffsets: idx) } }
                if options.count < maxOptions {
                    Button(prediction ? "Add outcome" : "Add choice") { options.append("") }
                }
            }
            Section(prediction ? "Open for" : "Runs for") {
                Picker("Length", selection: $seconds) {
                    ForEach((limits?.durations ?? []).filter { $0.value >= minSeconds }, id: \.value) { Text($0.label).tag($0.value) }
                }
                .pickerStyle(.segmented)
            }
            if !prediction {
                Section("Extra votes with channel points") {
                    TextField("Off", text: $pointsPerVote).keyboardType(.numberPad).keyboardHideButton()
                }
            }
        }
        .navigationTitle(prediction ? "Start a prediction" : "Start a poll")
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Start") {
                    Task {
                        let ok = prediction
                            ? await model.act(.createPrediction(title: title, outcomes: filled, seconds: seconds))
                            : await model.act(.createPoll(title: title, choices: filled, seconds: seconds,
                                                          pointsPerVote: UInt32(pointsPerVote).flatMap { $0 > 0 ? $0 : nil }))
                        if ok { done() }
                    }
                }
                .disabled(model.busy || title.trimmingCharacters(in: .whitespaces).isEmpty || filled.count < minOptions)
            }
        }
    }
}

// MARK: - Raid

private struct RaidFormView: View {
    let model: ManageChannelModel
    @State private var query = ""
    @State private var results: [RaidTarget] = []

    var body: some View {
        Group {
            if let raid = model.view?.raid {
                VStack(spacing: 18) {
                    Text("Raiding \(raid.targetName)").font(.sn(15)).foregroundStyle(SN.textSecondary)
                    TimelineView(.periodic(from: .now, by: 0.5)) { ctx in
                        Text(managerCountdown(raid.deadlineMS, now: ctx.date))
                            .font(.sn(44, 700)).monospacedDigit().foregroundStyle(SN.textPrimary)
                    }
                    HStack(spacing: 10) {
                        PillButton("Cancel raid", danger: true, busy: model.busy) { await model.act(.cancelRaid) }
                        PillButton("Raid now", busy: model.busy) { await model.act(.raidNow) }
                    }
                    Spacer()
                }
                .padding(.top, 40)
                .frame(maxWidth: .infinity)
            } else {
                List(results) { t in
                    Button {
                        Task { await model.act(.raid(t)) }
                    } label: {
                        HStack(spacing: 12) {
                            AsyncImage(url: t.avatarURL.flatMap(URL.init(string:))) { $0.resizable() } placeholder: { Color.white.opacity(0.1) }
                                .frame(width: 32, height: 32)
                                .clipShape(Circle())
                            VStack(alignment: .leading, spacing: 2) {
                                Text(t.name).font(.sn(15, 600)).foregroundStyle(SN.textPrimary)
                                Text(t.line).font(.sn(13)).foregroundStyle(SN.textSecondary)
                            }
                        }
                    }
                    .disabled(!t.live || model.busy)
                }
                .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search for a channel")
                .task(id: query) {
                    try? await Task.sleep(for: .milliseconds(220))
                    guard !Task.isCancelled else { return }
                    results = query.trimmingCharacters(in: .whitespaces).isEmpty ? [] : await model.searchRaid(query)
                }
            }
        }
        .navigationTitle("Raid a channel")
    }
}

// MARK: - Ads

private struct AdsFormView: View {
    let model: ManageChannelModel
    let done: () -> Void
    @State private var seconds: UInt32 = 60

    var body: some View {
        Form {
            if let ads = model.view?.ads {
                if let detail = model.view?.sections.first(where: { $0.id == "ads" })?.detail ?? ads.scheduleHint {
                    Section {
                        Text(detail).foregroundStyle(SN.textPrimary)
                        if let preroll = ads.schedule?.prerollLine { Text(preroll).foregroundStyle(SN.textSecondary) }
                        if ads.schedule != nil, let hint = ads.scheduleHint { Text(hint).foregroundStyle(SN.textSecondary) }
                    }
                }
                Section("Length") {
                    Picker("Length", selection: $seconds) {
                        ForEach(ads.lengths, id: \.value) { Text($0.label).tag($0.value) }
                    }
                    .pickerStyle(.segmented)
                }
                Section {
                    Button("Run ad break") { Task { if await model.act(.runAd(seconds: seconds)) { done() } } }
                        .disabled(model.busy)
                    if let s = ads.schedule, s.snoozesLeft > 0 {
                        Button("Snooze next ad") { Task { await model.act(.snoozeAd) } }
                            .disabled(model.busy)
                    }
                }
            }
        }
        .navigationTitle("Ad break")
    }
}

// MARK: - Rewards queue

private struct RewardQueueView: View {
    let model: ManageChannelModel

    var body: some View {
        List {
            if let queue = model.queue {
                if let empty = queue.emptyLine {
                    Text(empty).foregroundStyle(SN.textSecondary)
                }
                ForEach(queue.groups) { group in
                    Section {
                        ForEach(group.rows) { row in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(row.line).font(.sn(13)).foregroundStyle(SN.textSecondary)
                                if let input = row.input { Text(input).foregroundStyle(SN.textPrimary) }
                            }
                            .swipeActions(edge: .trailing, allowsFullSwipe: true) {
                                Button("Complete") { Task { await model.act(.redemptions(ids: [row.id], fulfill: true)) } }
                                    .tint(SN.success)
                            }
                            .swipeActions(edge: .leading, allowsFullSwipe: false) {
                                Button("Reject", role: .destructive) {
                                    Task { await model.act(.redemptions(ids: [row.id], fulfill: false)) }
                                }
                            }
                        }
                    } header: {
                        HStack {
                            Text(group.heading)
                            Spacer()
                            Button(group.paused ? "Resume" : "Pause") {
                                Task { await model.act(.reward(rewardID: group.rewardID, paused: !group.paused, enabled: nil)) }
                            }
                            .font(.sn(13, 600))
                            .textCase(nil)
                            .disabled(model.busy)
                        }
                    }
                }
                if !queue.groups.isEmpty {
                    Section {
                        Text("Swipe left to complete. Swipe right to reject and refund the points.")
                            .font(.sn(12.5)).foregroundStyle(SN.textSecondary)
                    }
                }
            } else {
                ProgressView()
            }
        }
        .navigationTitle("Rewards queue")
        .task { await model.loadQueue() }
        .refreshable { await model.loadQueue() }
    }
}

// MARK: - Announce

private struct AnnounceFormView: View {
    let model: ManageChannelModel
    let done: () -> Void
    @State private var text = ""
    @State private var color = "primary"

    var body: some View {
        Form {
            Section {
                TextField("Your announcement", text: $text, axis: .vertical).lineLimit(3...6).keyboardHideButton()
            }
            if let colors = model.view?.chat?.announceColors {
                Section("Color") {
                    Picker("Color", selection: $color) {
                        ForEach(colors) { Text($0.label).tag($0.id) }
                    }
                }
            }
        }
        .navigationTitle("Announce")
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Announce") { Task { if await model.act(.announce(text: text, color: color)) { done() } } }
                    .disabled(model.busy || text.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
    }
}
