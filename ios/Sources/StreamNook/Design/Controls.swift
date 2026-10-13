import SwiftUI

/// One choice in a glaze segment group.
struct Segment<Value: Hashable>: Identifiable {
    let value: Value
    let label: String
    var symbol: String?
    var accessibility: String?

    var id: Value { value }
}

/// Segments in a glass capsule, the chosen one a darker pill set into the glass that glides
/// between them (the look of the desktop Home tab strip). With `allowsNone`, tapping the chosen
/// segment again chooses nothing.
struct GlazeSegments<Value: Hashable>: View {
    let segments: [Segment<Value>]
    @Binding var selection: Value?
    var allowsNone = false
    @Namespace private var pill
    @Environment(\.snStill) private var still

    init(_ segments: [Segment<Value>], selection: Binding<Value?>, allowsNone: Bool = false) {
        self.segments = segments
        _selection = selection
        self.allowsNone = allowsNone
    }

    var body: some View {
        HStack(spacing: 4) {
            ForEach(segments) { segment in
                let on = selection == segment.value
                Button {
                    let next: Value? = on && allowsNone ? nil : segment.value
                    guard next != selection else { return }
                    withAnimation(still ? SNMotion.run(.easeOut(duration: 0.15)) : SNMotion.run(SNMotion.glide)) { selection = next }
                } label: {
                    HStack(spacing: 6) {
                        if let symbol = segment.symbol {
                            Image(systemName: symbol).font(.system(size: 13, weight: .semibold))
                        }
                        Text(segment.label).font(on ? .snLabelStrong : .snLabel)
                    }
                    .foregroundStyle(on ? SN.textPrimary : SN.textMuted)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 7)
                    .background {
                        if on { GlazeSelected().matchedGeometryEffect(id: "pill", in: pill) }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(segment.accessibility ?? segment.label)
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
        .padding(4)
        .snGlass(Capsule())
    }
}

/// A 44 point square button around a smaller symbol.
struct IconButton: View {
    let symbol: String
    let label: String
    var size: CGFloat = 20
    var tint: Color?
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: size, weight: .regular))
                .foregroundStyle(tint ?? SN.textSecondary)
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// A row that opens something: a symbol, a title over a one-line description, and a chevron.
struct RowLabel: View {
    let symbol: String
    let title: String
    var detail: String?
    var symbolTint: Color?

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 18))
                .foregroundStyle(symbolTint ?? SN.textSecondary)
                .frame(width: 24)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.snRowTitle).foregroundStyle(SN.textPrimary)
                if let detail {
                    Text(detail).font(.snRowDetail).foregroundStyle(SN.textMuted).lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            Image(systemName: Symbol.disclosure)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(SN.textMuted)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 12)
        .contentShape(Rectangle())
    }
}

/// A row's press: the theme's active surface behind it while the finger is down.
struct RowPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background {
                RoundedRectangle(cornerRadius: SNRadius.row, style: .continuous)
                    .fill(SN.surfaceActive.opacity(configuration.isPressed ? 1 : 0))
            }
    }
}

/// A card's press: 80 percent opacity while the finger is down.
struct CardPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.8 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

/// A pill that needs two taps: the first arms it (it turns red and says "Confirm"), a second
/// within three seconds acts, and left alone it disarms itself.
struct ArmedPill: View {
    let label: String
    var armedLabel = "Confirm"
    var enabled = true
    @Binding var armed: Bool
    let action: () -> Void
    @State private var disarm: Task<Void, Never>?

    var body: some View {
        Button {
            if armed {
                disarm?.cancel()
                armed = false
                action()
            } else {
                Haptics.destructive()
                withAnimation(.easeOut(duration: 0.15)) { armed = true }
                disarm = Task {
                    try? await Task.sleep(for: .seconds(3))
                    guard !Task.isCancelled else { return }
                    withAnimation(.easeOut(duration: 0.15)) { armed = false }
                }
            }
        } label: {
            Text(armed ? armedLabel : label)
                .font(.sn(12.5, 600))
                .lineLimit(1)
                .foregroundStyle(armed ? SN.error : SN.textSecondary)
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background {
                    if armed {
                        Capsule().fill(SN.error.opacity(0.15))
                    }
                }
                .snGlassIf(!armed, Capsule())
                .frame(minWidth: 44, minHeight: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .opacity(enabled ? 1 : 0.5)
        .onDisappear { disarm?.cancel() }
    }
}

extension View {
    @MainActor @ViewBuilder
    func snGlassIf<S: Shape>(_ on: Bool, _ shape: S) -> some View {
        if on { snGlass(shape, interactive: true) } else { self }
    }
}
