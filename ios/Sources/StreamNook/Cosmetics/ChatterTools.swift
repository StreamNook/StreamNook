import Observation
import SNKit
import SwiftUI
import UIKit

/// What the user card holds beyond the profile: the core's card tools (Moderate buttons and the
/// chatter's history in a channel the viewer moderates, the viewer's own note and name color), a
/// button's answer while it shows, and the note being edited. Every write goes through the core.
@Observable @MainActor
final class ChatterToolsModel {
    let target: ProfileTarget
    private(set) var tools: ChatterCardTools?
    /// The last button's answer, said under the buttons with Undo when it can be undone.
    private(set) var result: (text: String, undo: CardButton?)?
    /// The button whose action is on its way.
    private(set) var running: String?
    /// History past its first three entries.
    var historyOpen = false
    var noteDraft = ""
    /// A warning waiting for its reason.
    var warnPending: CardButton?
    var warnReason = ""
    /// A Ban waiting for its confirmation.
    var banPending: CardButton?

    @ObservationIgnored private let core: CoreBridge
    @ObservationIgnored private var savedNote = ""

    static let historyPreview = 3

    init(target: ProfileTarget, core: CoreBridge) {
        self.target = target
        self.core = core
    }

    func load() async {
        guard !target.userID.isEmpty else { return }
        do {
            let tools = try await core.cardTools(provider: "twitch", channel: target.roomLogin, channelID: target.roomID,
                                                 userID: target.userID)
            self.tools = tools
            savedNote = tools.note ?? ""
            noteDraft = savedNote
        } catch {
            log.debug("card tools unavailable: \(String(describing: error), privacy: .public)")
        }
    }

    /// A tap on a card button: Ban asks first, a warning without a saved reason asks for one,
    /// everything else runs at once.
    func tap(_ button: CardButton) {
        if button.danger {
            banPending = button
        } else if button.asksReason {
            warnReason = ""
            warnPending = button
        } else {
            Task { await run(button) }
        }
    }

    func run(_ button: CardButton, reason: String? = nil) async {
        guard running == nil, let room = target.roomID else { return }
        running = button.id
        defer { running = nil }
        do {
            let outcome = try await core.moderate(button, broadcasterID: room, targetUserID: target.userID,
                                                  targetName: target.displayName, reason: reason)
            result = (outcome.message, outcome.undoable ? Self.lift(after: button) : nil)
            // The history and Ban or Unban follow what is now in force.
            await load()
        } catch {
            result = (ChatComposerModel.words(error), nil)
        }
    }

    /// Lifts what `button` did (Unban reverses a ban and a timeout alike).
    func undo() {
        guard let lift = result?.undo else { return }
        Task { await run(lift) }
    }

    private static func lift(after button: CardButton) -> CardButton? {
        let timeout = button.kind == "timeout"
        let raw = #"{"id":"undo","label":"Undo","kind":"unban","action":{"kind":"unban","timeout":\#(timeout)},"reason":null,"asks_reason":false,"danger":false,"spoken":"Undo"}"#
        return try? JSONDecoder().decode(CardButton.self, from: Data(raw.utf8))
    }

    /// Saves the note when it changed; an empty note removes it.
    func saveNote() {
        let note = noteDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard note != savedNote else { return }
        savedNote = note
        let core = core
        let id = target.userID
        Task { try? await core.setNote(userID: id, note: note) }
    }

    func setColor(_ color: String?) {
        tools?.color = color
        let core = core
        let target = target
        Task { try? await core.setNameColor(userID: target.userID, login: target.login, color: color) }
    }
}

/// The card's own sections, between the person's sub standing and their badges: Moderate (the
/// four timeout lengths and Ban, then the viewer's own buttons) and History in a channel the
/// viewer moderates; then the note and name color only the viewer sees, everywhere.
struct ChatterToolsSections: View {
    @State var model: ChatterToolsModel
    @FocusState private var noteFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let moderate = model.tools?.moderate {
                moderateSection(moderate).padding(.top, 20)
                historySection(moderate.history).padding(.top, 20)
            }
            if model.tools != nil {
                privateSection.padding(.top, 20)
            }
        }
        .task { await model.load() }
        .onChange(of: noteFocused) { _, focused in if !focused { model.saveNote() } }
        .onDisappear { model.saveNote() }
        .confirmationDialog("Ban \(model.target.displayName)?", isPresented: Binding(
            get: { model.banPending != nil }, set: { if !$0 { model.banPending = nil } }
        ), titleVisibility: .visible) {
            Button("Ban", role: .destructive) {
                if let ban = model.banPending { Task { await model.run(ban) } }
                model.banPending = nil
            }
        } message: {
            Text("They can't chat here until a moderator unbans them.")
        }
        .alert("Warn \(model.target.displayName)", isPresented: Binding(
            get: { model.warnPending != nil }, set: { if !$0 { model.warnPending = nil } }
        )) {
            TextField("Reason they will see", text: $model.warnReason)
            Button("Warn") {
                if let warn = model.warnPending {
                    let reason = model.warnReason
                    Task { await model.run(warn, reason: reason) }
                }
                model.warnPending = nil
            }
            .disabled(model.warnReason.trimmingCharacters(in: .whitespaces).isEmpty)
            Button("Cancel", role: .cancel) { model.warnPending = nil }
        } message: {
            Text("They see your reason and must acknowledge it before they can chat here again.")
        }
    }

    // MARK: Moderate

    private func moderateSection(_ moderate: CardModerate) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            ProfileSectionHeader("Moderate")
            buttonRow(moderate.buttons)
            ForEach(Array(stride(from: 0, to: moderate.custom.count, by: 3)), id: \.self) { start in
                buttonRow(Array(moderate.custom[start..<min(start + 3, moderate.custom.count)]), fill: 3)
            }
            if let result = model.result {
                HStack(spacing: 8) {
                    Text(result.text)
                        .font(.snRowDetail)
                        .foregroundStyle(SN.textSecondary)
                        .lineLimit(2)
                    Spacer(minLength: 8)
                    if result.undo != nil {
                        Button("Undo") { model.undo() }
                            .font(.sn(13, 600))
                            .foregroundStyle(SN.accent)
                            .disabled(model.running != nil)
                    }
                }
                .padding(.top, 2)
                .transition(.opacity)
            }
        }
        .animation(.easeOut(duration: 0.15), value: model.result?.text)
    }

    /// Equal-width buttons on one line; `fill` keeps short custom rows the width of a full one.
    private func buttonRow(_ buttons: [CardButton], fill: Int? = nil) -> some View {
        HStack(spacing: 7) {
            ForEach(buttons) { button in
                Button { model.tap(button) } label: {
                    ZStack {
                        Text(button.label).opacity(model.running == button.id ? 0 : 1)
                        if model.running == button.id { ProgressView().controlSize(.small) }
                    }
                    .font(.sn(14, 600))
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
                    .foregroundStyle(button.danger ? SN.error : SN.textPrimary)
                    .frame(maxWidth: .infinity, minHeight: 38)
                    .background(button.danger ? SN.error.opacity(0.12) : SN.surface,
                                in: .rect(cornerRadius: 12, style: .continuous))
                    .overlay {
                        RoundedRectangle(cornerRadius: 12, style: .continuous)
                            .strokeBorder(button.danger ? SN.error.opacity(0.25) : Color.white.opacity(0.06))
                    }
                }
                .buttonStyle(BadgePress())
                .disabled(model.running != nil)
                .accessibilityLabel(button.spoken)
            }
            if let fill, buttons.count < fill {
                ForEach(buttons.count..<fill, id: \.self) { _ in Color.clear.frame(maxWidth: .infinity, minHeight: 38) }
            }
        }
    }

    // MARK: History

    @ViewBuilder
    private func historySection(_ history: ModHistory) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            ProfileSectionHeader("History", detail: history.entries.isEmpty ? nil : "\(history.entries.count)")
            if let current = history.current {
                Label(current.label, systemImage: current.permanent ? "nosign" : "clock")
                    .font(.sn(13, 600))
                    .foregroundStyle(SN.error)
            }
            if !history.isAvailable {
                Text(history.note ?? "History isn't available right now.")
                    .font(.snRowDetail)
                    .foregroundStyle(SN.textMuted)
            } else if history.entries.isEmpty {
                Text("Nothing on record here.")
                    .font(.snRowDetail)
                    .foregroundStyle(SN.textMuted)
            } else {
                let shown = model.historyOpen ? history.entries : Array(history.entries.prefix(ChatterToolsModel.historyPreview))
                VStack(spacing: 0) {
                    ForEach(Array(shown.enumerated()), id: \.element.id) { index, entry in
                        if index > 0 { Divider().overlay(Color.white.opacity(0.06)) }
                        HistoryRow(entry: entry)
                    }
                    if history.entries.count > ChatterToolsModel.historyPreview {
                        Divider().overlay(Color.white.opacity(0.06))
                        Button {
                            withAnimation(.snappy) { model.historyOpen.toggle() }
                        } label: {
                            Text(model.historyOpen ? "Show fewer" : "Show all \(history.entries.count)")
                                .font(.sn(13, 500))
                                .foregroundStyle(SN.accent)
                                .frame(maxWidth: .infinity, minHeight: 40, alignment: .leading)
                                .padding(.horizontal, 12)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
                .snPanel()
            }
        }
    }

    // MARK: Only you see this

    private var privateSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            ProfileSectionHeader("Only you see this")
            VStack(spacing: 0) {
                TextField("Add a note", text: $model.noteDraft, axis: .vertical)
                    .font(.sn(14))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1...5)
                    .focused($noteFocused)
                    .submitLabel(.done)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 11)
                Divider().overlay(Color.white.opacity(0.06))
                colorRow
            }
            .snPanel()
        }
    }

    private var colorRow: some View {
        HStack(spacing: 8) {
            Text("Name color").font(.sn(14)).foregroundStyle(SN.textPrimary)
            Spacer(minLength: 4)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 7) {
                    ForEach(model.tools?.swatches ?? [], id: \.self) { hex in
                        swatch(hex)
                    }
                    ColorPicker("More colors", selection: Binding(
                        get: { nameColor(model.tools?.color ?? "#FFFFFF") },
                        set: { model.setColor(hexString($0)) }
                    ), supportsOpacity: false)
                    .labelsHidden()
                    .frame(width: 26, height: 26)
                }
                .padding(.vertical, 2)
            }
            .frame(maxWidth: 210)
            if model.tools?.color != nil {
                Button("Reset") { model.setColor(nil) }
                    .font(.sn(13, 500))
                    .foregroundStyle(SN.textSecondary)
            }
        }
        .padding(.horizontal, 12)
        .frame(minHeight: 46)
    }

    private func swatch(_ hex: String) -> some View {
        let on = model.tools?.color?.caseInsensitiveCompare(hex) == .orderedSame
        return Button { model.setColor(on ? nil : hex) } label: {
            Circle()
                .fill(nameColor(hex))
                .frame(width: 20, height: 20)
                .padding(3)
                .overlay { Circle().strokeBorder(on ? SN.textPrimary : .clear, lineWidth: 2) }
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Color \(hex)")
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// One history entry: a small mark for its kind, what happened, who and when, and the quote.
private struct HistoryRow: View {
    let entry: ModHistoryEntry

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            mark.frame(width: 26, height: 26)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.title).font(.sn(14, 600)).foregroundStyle(SN.textPrimary)
                let meta = [entry.by.map { "by \($0)" }, entry.when.isEmpty ? nil : entry.when].compactMap { $0 }
                if !meta.isEmpty {
                    Text(meta.joined(separator: " · ")).font(.sn(12.5)).foregroundStyle(SN.textMuted)
                }
                if let quote = entry.quote, !quote.isEmpty {
                    Text("\u{201C}\(quote)\u{201D}")
                        .font(.sn(13))
                        .foregroundStyle(SN.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .accessibilityElement(children: .combine)
    }

    private var tint: Color {
        switch entry.kind {
        case "ban": return SN.error
        case "timeout": return SN.warning
        case "warn", "acknowledged", "comment": return SN.info
        default: return SN.textMuted
        }
    }

    private var symbol: String {
        switch entry.kind {
        case "ban": return "nosign"
        case "timeout": return "clock.fill"
        case "unban", "untimeout": return "arrow.uturn.backward"
        case "warn": return "exclamationmark"
        case "acknowledged": return "checkmark"
        case "deleted": return "trash"
        default: return "text.bubble"
        }
    }

    private var mark: some View {
        RoundedRectangle(cornerRadius: 8, style: .continuous)
            .fill(tint.opacity(0.14))
            .overlay {
                Image(systemName: symbol).font(.system(size: 12, weight: .bold)).foregroundStyle(tint)
            }
    }
}

/// A picked color as `#RRGGBB`, the form the core stores.
private func hexString(_ color: Color) -> String {
    var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
    UIColor(color).getRed(&r, green: &g, blue: &b, alpha: &a)
    let byte = { (v: CGFloat) in Int((min(max(v, 0), 1) * 255).rounded()) }
    return String(format: "#%02X%02X%02X", byte(r), byte(g), byte(b))
}
