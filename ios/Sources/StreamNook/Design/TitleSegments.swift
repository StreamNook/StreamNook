import SwiftUI

/// A screen's parts as a compact capsule in the title row ("Drops | Badges"), so choosing between
/// them spends no row of its own. It goes in a trailing toolbar item, whose Liquid Glass is the
/// capsule; the chosen part sits in the darker pill that glides between them, the others are
/// secondary text. Each label keeps its bold width underneath, so the capsule never changes
/// width when the choice moves.
struct TitleSegments<Value: Hashable>: View {
    let segments: [Segment<Value>]
    @Binding var selection: Value
    @Namespace private var pill
    @Environment(\.snStill) private var still

    var body: some View {
        HStack(spacing: 0) {
            ForEach(segments) { segment in
                let on = selection == segment.value
                Button {
                    guard !on else { return }
                    withAnimation(still ? SNMotion.run(.easeOut(duration: 0.15)) : SNMotion.run(SNMotion.glide)) {
                        selection = segment.value
                    }
                } label: {
                    ZStack {
                        Text(segment.label).font(.snLabelStrong).hidden()
                        Text(segment.label).font(on ? .snLabelStrong : .snLabel)
                    }
                    .foregroundStyle(on ? SN.textPrimary : SN.textSecondary)
                    .padding(.horizontal, 12)
                    .frame(height: 36)
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
        .padding(.horizontal, 2)
    }
}
