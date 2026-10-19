import Observation
import SNKit
import SwiftUI

/// What the Moderation sheet holds for one room: the messages AutoMod is holding, the room's
/// recent moderator actions (both read from the core, both followed live while the sheet is open),
/// and whether a /nuke is running. Every action goes through the core.
@Observable @MainActor
final class ModerationSheetModel {
    let channel: String
    let roomID: String
    private(set) var held: [HeldMessage]?
    private(set) var log: [RoomModAction]?
    private(set) var nukes = 0
    /// The row whose action is on its way, by id.
    private(set) var busy: String?
    /// The last refusal or answer, under the section it came from.
    private(set) var heldNote: String?
    private(set) var logNote: String?
    private(set) var phraseNote: String?
    /// The phrase just blocked, while its Undo is offered.
    private(set) var blocked: String?
    var phrase = ""
    /// Actions that arrived while the sheet is open, waiting behind the "N new" pill so rows never
    /// move under a thumb aiming at Unban.
    private(set) var incoming: [RoomModAction] = []
    /// Held messages just decided, shown with their outcome for a moment before they go.
    private(set) var decided: [String: String] = [:]
    /// An Unban or Lift waiting for its confirmation.
    var liftPending: RoomModAction?

    @ObservationIgnored private let core: CoreBridge
    @ObservationIgnored private var tasks: [Task<Void, Never>] = []

    init(channel: String, roomID: String, core: CoreBridge) {
        self.channel = channel
        self.roomID = roomID
        self.core = core
    }

    func start() {
        Task { await reload() }
        let core = core
        let channel = channel
        // Rust pushes each held message, each decision and each moderator action as it happens.
        for name in ["eventsub://automod-hold", "eventsub://automod-update"] {
            let events = core.events.stream(name)
            tasks.append(Task { [weak self] in
                for await _ in events {
                    guard !Task.isCancelled else { return }
                    self?.held = try? await core.heldQueue(channel: channel)
                }
            })
        }
        let actions = core.events.stream("room-mod-log")
        tasks.append(Task { [weak self] in
            for await event in actions {
                guard !Task.isCancelled else { return }
                guard let entry = try? JSONDecoder().decode(RoomModAction.self, from: Data(event.payloadJSON.utf8)),
                      entry.channel == channel, let self else { continue }
                let known = (self.log ?? []).contains { $0.id == entry.id } || self.incoming.contains { $0.id == entry.id }
                if !known { self.incoming.insert(entry, at: 0) }
            }
        })
    }

    func stop() {
        tasks.forEach { $0.cancel() }
        tasks.removeAll()
    }

    /// Shows the actions waiting behind the pill.
    func showIncoming() {
        log = incoming + (log ?? [])
        incoming = []
    }

    func reload() async {
        incoming = []
        held = (try? await core.heldQueue(channel: channel)) ?? []
        log = (try? await core.roomModLog(channel: channel)) ?? []
        nukes = (try? await core.invoke("nuke_active", args: ["channel": channel], as: Int.self)) ?? 0
    }

    func decide(_ message: HeldMessage, allow: Bool) async {
        busy = message.id
        defer { busy = nil }
        do {
            try await core.resolveHeld(messageID: message.messageID, allow: allow)
            heldNote = nil
            // The row says what happened for a moment, so the next row's buttons do not slide
            // under the thumb that just tapped.
            decided[message.id] = allow ? "Allowed" : "Denied"
            try? await Task.sleep(for: .seconds(1.5))
            decided[message.id] = nil
            held?.removeAll { $0.id == message.id }
        } catch {
            heldNote = ChatComposerModel.words(error)
        }
    }

    /// Lifts the ban or timeout an entry describes.
    func lift(_ entry: RoomModAction) async {
        guard let target = entry.targetID else { return }
        busy = entry.id
        defer { busy = nil }
        do {
            let outcome = try await core.moderate(action: ["kind": "unban", "timeout": entry.action == "timeout"], broadcasterID: roomID,
                                                  targetUserID: target, targetName: entry.target ?? "", useSavedReason: false)
            logNote = outcome.message
        } catch {
            logNote = ChatComposerModel.words(error)
        }
    }

    func blockPhrase() async {
        let text = phrase.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        busy = "phrase"
        defer { busy = nil }
        do {
            _ = try await core.invoke("add_blocked_term", args: ["broadcasterId": roomID, "text": text])
            phraseNote = "Blocked \u{201C}\(text)\u{201D}"
            blocked = text
            phrase = ""
        } catch {
            phraseNote = ChatComposerModel.words(error)
        }
    }

    /// Takes the phrase just blocked off the list again.
    func unblock() async {
        guard let text = blocked else { return }
        busy = "phrase"
        defer { busy = nil }
        do {
            _ = try await core.invoke("remove_blocked_term", args: ["broadcasterId": roomID, "text": text])
            phraseNote = "Unblocked \u{201C}\(text)\u{201D}"
            blocked = nil
        } catch {
            phraseNote = ChatComposerModel.words(error)
        }
    }

    func stopNukes() async {
        _ = try? await core.invoke("nuke_stop", args: ["channel": channel])
        nukes = 0
    }
}

/// The room's moderation in one place, from the chat menu: what AutoMod is holding (Allow or
/// Deny), what moderators did lately (with Unban on a ban or timeout), blocking a phrase, stopping
/// a running /nuke, and the way into Manage channel for chat modes and Shield Mode.
struct ModerationSheet: View {
    @State var model: ModerationSheetModel
    var onManage: () -> Void
    @Environment(\.dismiss) private var dismiss

    private static let ago: RelativeDateTimeFormatter = {
        let f = RelativeDateTimeFormatter()
        f.unitsStyle = .short
        return f
    }()

    private var liftTitle: String {
        guard let entry = model.liftPending else { return "" }
        let name = entry.target ?? "them"
        return entry.action == "timeout" ? "Lift \(name)\u{2019}s timeout?" : "Unban \(name)?"
    }

    var body: some View {
        NavigationStack {
            List {
                heldSection
                logSection
                roomSection
            }
            .snGroupedList()
            .navigationTitle("Moderation")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } }
            }
            .refreshable { await model.reload() }
        }
        .onAppear { model.start() }
        .onDisappear { model.stop() }
        .confirmationDialog(liftTitle, isPresented: Binding(get: { model.liftPending != nil },
                                                            set: { if !$0 { model.liftPending = nil } }),
                            titleVisibility: .visible) {
            Button(model.liftPending?.action == "timeout" ? "Lift timeout" : "Unban") {
                if let entry = model.liftPending { Task { await model.lift(entry) } }
                model.liftPending = nil
            }
        }
    }

    // MARK: Held by AutoMod

    @ViewBuilder
    private var heldSection: some View {
        Section {
            if let held = model.held {
                if held.isEmpty {
                    Text("Nothing held right now.").font(.snRowDetail).foregroundStyle(SN.textMuted)
                }
                ForEach(held) { message in
                    VStack(alignment: .leading, spacing: 6) {
                        Label(message.label, systemImage: "flag.fill")
                            .font(.sn(12, 600))
                            .foregroundStyle(SN.warning)
                        Text("\(Text(message.shownName).fontWeight(.bold)): \(message.text)")
                            .font(.sn(14))
                            .foregroundStyle(SN.textPrimary)
                            .fixedSize(horizontal: false, vertical: true)
                        if let outcome = model.decided[message.id] {
                            Text(outcome)
                                .font(.sn(13, 600))
                                .foregroundStyle(outcome == "Allowed" ? SN.success : SN.error)
                                .frame(maxWidth: .infinity, minHeight: 34)
                                .padding(.top, 2)
                        } else {
                            HStack(spacing: 8) {
                                decideButton("Allow", tint: SN.success, message: message, allow: true)
                                decideButton("Deny", tint: SN.error, message: message, allow: false)
                            }
                            .padding(.top, 2)
                        }
                    }
                    .padding(.vertical, 4)
                }
            } else {
                ProgressView().frame(maxWidth: .infinity)
            }
        } header: {
            Text(model.held.map { $0.isEmpty ? "Held by AutoMod" : "Held by AutoMod \u{00B7} \($0.count)" } ?? "Held by AutoMod")
        } footer: {
            if let note = model.heldNote { ListFooter(text: note) }
        }
    }

    private func decideButton(_ title: String, tint: Color, message: HeldMessage, allow: Bool) -> some View {
        Button {
            Task { await model.decide(message, allow: allow) }
        } label: {
            Text(title)
                .font(.sn(14, 600))
                .foregroundStyle(tint)
                .frame(maxWidth: .infinity, minHeight: 34)
                .background(tint.opacity(0.15), in: .rect(cornerRadius: 10, style: .continuous))
        }
        .buttonStyle(.plain)
        .disabled(model.busy != nil)
    }

    // MARK: Recent actions

    @ViewBuilder
    private var logSection: some View {
        Section {
            if !model.incoming.isEmpty {
                Button {
                    withAnimation(.snappy) { model.showIncoming() }
                } label: {
                    Label("\(model.incoming.count) new", systemImage: "arrow.up")
                        .font(.sn(13, 600))
                        .foregroundStyle(SN.accent)
                        .frame(maxWidth: .infinity)
                }
                .buttonStyle(.borderless)
            }
            if let log = model.log {
                if log.isEmpty, model.incoming.isEmpty {
                    Text("No moderator actions since you opened this chat.").font(.snRowDetail).foregroundStyle(SN.textMuted)
                }
                ForEach(log) { entry in
                    HStack(alignment: .top, spacing: 10) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(entry.line).font(.sn(14)).foregroundStyle(SN.textPrimary)
                            if let quote = entry.quote, !quote.isEmpty {
                                Text("\u{201C}\(quote)\u{201D}").font(.sn(13)).foregroundStyle(SN.textSecondary)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                            Text(Self.ago.localizedString(for: Date(timeIntervalSince1970: Double(entry.atMs) / 1000), relativeTo: .now))
                                .font(.sn(12)).foregroundStyle(SN.textMuted)
                        }
                        Spacer(minLength: 8)
                        if entry.liftable, entry.targetID != nil {
                            Button(entry.action == "timeout" ? "Lift" : "Unban") { model.liftPending = entry }
                                .font(.sn(13, 600))
                                .foregroundStyle(SN.accent)
                                .buttonStyle(.borderless)
                                .disabled(model.busy != nil)
                        }
                    }
                    .padding(.vertical, 2)
                }
            }
        } header: {
            Text("Recent actions")
        } footer: {
            if let note = model.logNote { ListFooter(text: note) }
        }
    }

    // MARK: Room

    private var roomSection: some View {
        Section {
            HStack(spacing: 8) {
                TextField("Block a phrase", text: $model.phrase)
                    .font(.sn(15))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .submitLabel(.done)
                    .onSubmit { Task { await model.blockPhrase() } }
                Button("Block") { Task { await model.blockPhrase() } }
                    .font(.sn(14, 600))
                    .buttonStyle(.borderless)
                    .disabled(model.phrase.trimmingCharacters(in: .whitespaces).isEmpty || model.busy != nil)
            }
            if model.nukes > 0 {
                Button(role: .destructive) {
                    Task { await model.stopNukes() }
                } label: {
                    Label("Stop the running /nuke", systemImage: "stop.circle")
                }
            }
            Button {
                dismiss()
                onManage()
            } label: {
                HStack {
                    Label("Chat modes and Shield Mode", systemImage: "slider.horizontal.3")
                    Spacer()
                    Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(SN.textMuted)
                }
            }
        } header: {
            Text("Room")
        } footer: {
            if let note = model.phraseNote {
                HStack(spacing: 8) {
                    ListFooter(text: note)
                    if model.blocked != nil {
                        Button("Undo") { Task { await model.unblock() } }
                            .font(.sn(12.5, 600))
                            .buttonStyle(.borderless)
                    }
                }
            }
        }
    }
}
