import SNKit
import SwiftUI

/// A live prediction. Open: the sides as tiles (share, return ratio, people), the split, the
/// amount (presets and a typed amount) and the button Rust words. Picking a side or an amount
/// never spends; the button opens the confirm, and only the confirm's Predict calls Rust.
/// Locked: the split and the viewer's stake. Ended: the split and Rust's result line.
struct PredictionCard: View {
    @Bindable var model: LiveVotesModel
    let prediction: PredictionView
    let open: Bool
    @FocusState private var typing: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            CardHead(open: open, label: "Prediction", line: prediction.result?.line ?? prediction.title,
                     action: { model.tap(.prediction) }) {
                VoteKindMark(card: .prediction)
            } status: {
                VoteTimeSlot(deadlineMS: prediction.deadlineMS, badge: prediction.badge)
            }
            CardBody(open: open) {
                VStack(alignment: .leading, spacing: 10) {
                    VoteQuestion(text: prediction.title)
                        .padding(.bottom, 2)
                    if prediction.canPredict {
                        sides
                        SplitBar(outcomes: prediction.outcomes)
                        if let status = prediction.statusLine {
                            VoteLinePill(text: status, icon: prediction.pointsIcon, showsIcon: true)
                        }
                        betControls
                    } else {
                        SplitBar(outcomes: prediction.outcomes)
                        if let result = prediction.result {
                            VoteLinePill(text: result.line, tint: tone(result.tone))
                        } else if let status = prediction.statusLine {
                            VoteLinePill(text: status, icon: prediction.pointsIcon, showsIcon: true)
                        }
                        if !prediction.totalsLine.isEmpty {
                            Text(prediction.totalsLine)
                                .font(.sn(12))
                                .foregroundStyle(SN.textMuted)
                                .lineLimit(1)
                        }
                    }
                }
            }
        }
        .modifier(VoteCardSurface())
        .sheet(isPresented: $model.confirmOpen) {
            PredictConfirm(model: model, iconURL: prediction.pointsIcon)
        }
        .onChange(of: open) { _, open in if !open { typing = false } }
        // The number pad shrinks the chat column; the card being typed in keeps its room.
        .onChange(of: typing) { _, typing in model.holdHeight(typing) }
    }

    private func tone(_ tone: String) -> Color? {
        switch tone {
        case "won": SN.success
        case "lost": SN.error
        default: nil
        }
    }

    // MARK: Sides

    @ViewBuilder private var sides: some View {
        if prediction.outcomes.count <= 2 {
            HStack(spacing: 8) {
                ForEach(prediction.outcomes) { tile($0) }
            }
        } else {
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                ForEach(prediction.outcomes) { tile($0) }
            }
        }
    }

    private func tile(_ outcome: OutcomeView) -> some View {
        let color = LiveVoteLook.color(outcome.color)
        let chosen = model.side == outcome.id
        let open = model.canPick(outcome)
        let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)
        return Button {
            Haptics.tick()
            model.pick(outcome.id)
        } label: {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 4) {
                    Text(outcome.title)
                        .font(.sn(13, 600))
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                    Spacer(minLength: 0)
                    if outcome.mine {
                        PointsArt(iconURL: prediction.pointsIcon, size: 13)
                    }
                }
                Text("\(outcome.percent)%")
                    .font(.sn(22, 700))
                    .monospacedDigit()
                    .foregroundStyle(color)
                    .contentTransition(.numericText())
                HStack(spacing: 4) {
                    Text(outcome.ratio).lineLimit(1)
                    Spacer(minLength: 4)
                    Image(systemName: "person.fill")
                        .font(.system(size: 9))
                        .accessibilityHidden(true)
                    Text(outcome.peopleLabel).lineLimit(1)
                }
                .font(.sn(11))
                .monospacedDigit()
                .foregroundStyle(SN.textMuted)
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background { shape.fill(chosen ? color.opacity(0.16) : SN.surface) }
            .overlay { shape.stroke(chosen ? color : .clear, lineWidth: 1.5) }
            .contentShape(shape)
            .opacity(open ? 1 : 0.45)
            .animation(SNMotion.run(.easeOut(duration: 0.15)), value: chosen)
            .animation(SNMotion.run(.easeOut(duration: 0.3)), value: outcome.percent)
        }
        .buttonStyle(.plain)
        .disabled(!open)
        .accessibilityLabel("\(outcome.title), \(outcome.percent) percent, returns \(outcome.ratio), \(outcome.peopleLabel) people")
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }

    // MARK: Amount and button

    @ViewBuilder private var betControls: some View {
        let bet = model.bet
        HStack(spacing: 6) {
            ForEach(Array((bet?.presets ?? []).enumerated()), id: \.offset) { index, preset in
                Button {
                    typing = false
                    Haptics.tick()
                    model.choosePreset(index)
                } label: {
                    chip(selected: model.chosenPreset == index) {
                        Text(preset.label).font(.sn(12.5, 600)).monospacedDigit().lineLimit(1)
                    }
                }
                .buttonStyle(.plain)
                .accessibilityAddTraits(model.chosenPreset == index ? .isSelected : [])
            }
            amountField
        }
        .frame(height: 34)
        Group {
            if let bet, let problem = bet.problem {
                // Rust's reason in the button's place, the same height, so nothing jumps.
                Text(problem)
                    .font(.sn(14, 500))
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
                    .minimumScaleFactor(0.85)
                    .frame(maxWidth: .infinity)
                    .frame(height: SNButtonStyle.height)
                    .background(Capsule().fill(SN.surface))
            } else {
                Button {
                    typing = false
                    model.openConfirm()
                } label: {
                    HStack(spacing: 7) {
                        PointsArt(iconURL: prediction.pointsIcon, size: 17)
                        Text(bet?.button ?? " ")
                    }
                }
                .buttonStyle(.snPrimary)
                .disabled(bet?.ok != true)
            }
        }
        HStack(spacing: 8) {
            Text(bet?.ok == true ? bet?.winLine ?? "" : "")
                .lineLimit(1)
            Spacer(minLength: 8)
            Text(bet?.balanceLine ?? "")
                .lineLimit(1)
        }
        .font(.sn(12))
        .monospacedDigit()
        .foregroundStyle(SN.textMuted)
        .frame(minHeight: 14)
    }

    /// The typed amount, as the last chip: a number pad opens on a tap, and what is typed goes to
    /// Rust as typed (Rust clamps it and words the button).
    private var amountField: some View {
        let selected = !model.typed.isEmpty
        return chip(selected: selected) {
            ZStack {
                if model.typed.isEmpty && !typing {
                    Image(systemName: "ellipsis")
                        .font(.system(size: 13, weight: .bold))
                        .foregroundStyle(SN.textSecondary)
                        .accessibilityHidden(true)
                }
                TextField("", text: Binding(get: { model.typed }, set: { model.type($0) }))
                    .keyboardType(.numberPad)
                    .multilineTextAlignment(.center)
                    .font(.sn(12.5, 600))
                    .monospacedDigit()
                    .foregroundStyle(SN.textPrimary)
                    .focused($typing)
                    .accessibilityLabel("Amount")
            }
        }
        .toolbar {
            if typing {
                ToolbarItemGroup(placement: .keyboard) {
                    Spacer()
                    Button {
                        typing = false
                    } label: {
                        Image(systemName: "keyboard.chevron.compact.down")
                    }
                    .accessibilityLabel("Hide keyboard")
                }
            }
        }
    }

    private func chip<Label: View>(selected: Bool, @ViewBuilder label: () -> Label) -> some View {
        let shape = RoundedRectangle(cornerRadius: 8, style: .continuous)
        return label()
            .foregroundStyle(SN.textPrimary)
            .padding(.horizontal, 4)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background { shape.fill(selected ? SN.accentOnSurface.opacity(0.22) : SN.surface) }
            .overlay { shape.stroke(selected ? SN.accentOnSurface : .clear, lineWidth: 1) }
            .contentShape(shape)
    }
}

/// The share of each side as one bar, a segment per side in its color.
private struct SplitBar: View {
    let outcomes: [OutcomeView]

    var body: some View {
        GeometryReader { geo in
            let shown = outcomes.filter { $0.percent > 0 }
            let gaps = CGFloat(max(shown.count - 1, 0)) * 2
            HStack(spacing: 2) {
                ForEach(shown) { outcome in
                    Capsule()
                        .fill(LiveVoteLook.color(outcome.color))
                        .frame(width: max(0, (geo.size.width - gaps) * CGFloat(min(outcome.percent, 100)) / 100))
                }
            }
            .animation(SNMotion.run(.easeOut(duration: 0.3)), value: outcomes.map(\.percent))
        }
        .frame(height: 4)
        .accessibilityHidden(true)
    }
}

/// The confirm before any points are spent: Rust's question, what it would win, the note, then
/// Cancel and Predict. Only Predict here places the prediction.
private struct PredictConfirm: View {
    let model: LiveVotesModel
    let iconURL: String?

    var body: some View {
        VStack(spacing: 6) {
            if let bet = model.bet {
                Text(bet.confirmTitle)
                    .font(.snSheetTitle)
                    .foregroundStyle(SN.textPrimary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 22)
                    .accessibilityAddTraits(.isHeader)
                Text(bet.confirmLine)
                    .font(.sn(14))
                    .foregroundStyle(SN.textSecondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                if let note = bet.confirmNote {
                    Text(note)
                        .font(.sn(12))
                        .foregroundStyle(SN.textMuted)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: 10) {
                    Button("Cancel") { model.confirmOpen = false }
                        .buttonStyle(.snSecondary)
                    Button {
                        Task {
                            await model.confirmPredict()
                        }
                    } label: {
                        HStack(spacing: 7) {
                            PointsArt(iconURL: iconURL, size: 17)
                            Text("Predict")
                        }
                    }
                    .buttonStyle(.snPrimary)
                    .disabled(model.predicting || !bet.ok)
                }
                .padding(.top, 16)
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 16)
        .fittedSheet()
    }
}
