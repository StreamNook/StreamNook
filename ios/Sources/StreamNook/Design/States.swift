import SNKit
import SwiftUI

/// First-load placeholders shaped like stream cards: a 16:9 block, then a bar at 75 percent and
/// one at 33 percent. They pulse quietly (no shimmer); the pulse stops when movement is dropped.
struct SkeletonCards: View {
    var count = 3
    @Environment(\.snStill) private var still
    @State private var dim = false

    var body: some View {
        VStack(spacing: 12) {
            ForEach(0..<count, id: \.self) { _ in
                VStack(alignment: .leading, spacing: 8) {
                    RoundedRectangle(cornerRadius: SNRadius.thumb, style: .continuous)
                        .fill(SN.surface)
                        .aspectRatio(16 / 9, contentMode: .fit)
                    VStack(alignment: .leading, spacing: 8) {
                        Capsule().fill(SN.surface).frame(height: 16)
                            .containerRelativeFrame(.horizontal, alignment: .leading) { width, _ in width * 0.62 }
                        Capsule().fill(SN.surface).frame(height: 12)
                            .containerRelativeFrame(.horizontal, alignment: .leading) { width, _ in width * 0.28 }
                    }
                    .padding(.horizontal, 4)
                    .padding(.bottom, 4)
                }
                .padding(8)
                .snMediaCard()
            }
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 16)
        .opacity(dim ? 0.5 : 1)
        .onAppear {
            guard !still else { return }
            withAnimation(.easeInOut(duration: 1).repeatForever(autoreverses: true)) { dim = true }
        }
        .accessibilityLabel("Loading")
    }
}

/// First-load placeholders shaped like category tiles: the art's 3:4 block and two bars in a chin
/// as tall as the tile's, in the grid's columns, pulsing like the cards. The rows land where the
/// tiles will, so nothing moves when they arrive.
struct SkeletonTiles: View {
    var rows = 3
    @Environment(\.snStill) private var still
    @State private var dim = false
    @State private var width: CGFloat = 402

    var body: some View {
        let count = WindowShape(width: width, height: 874).categoryColumns
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: CategoryTile.gap, alignment: .top), count: count),
                  spacing: CategoryTile.rowGap) {
            ForEach(0..<rows * count, id: \.self) { _ in
                VStack(alignment: .leading, spacing: 5) {
                    RoundedRectangle(cornerRadius: SNRadius.tile, style: .continuous)
                        .fill(SN.surface)
                        .aspectRatio(3 / 4, contentMode: .fit)
                    // The tile's chin height, taken from its name font on two lines.
                    Text(verbatim: " ")
                        .font(.sn(12.5, 500))
                        .lineLimit(2, reservesSpace: true)
                        .hidden()
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .overlay(alignment: .topLeading) {
                            VStack(alignment: .leading, spacing: 7) {
                                Capsule().fill(SN.surface).frame(height: 10).padding(.trailing, 28)
                                Capsule().fill(SN.surface).frame(height: 8).padding(.trailing, 52)
                            }
                            .padding(.top, 3)
                        }
                }
            }
        }
        .padding(.horizontal, CategoryTile.inset)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
        .padding(.top, CategoryTile.top)
        .padding(.bottom, 16)
        .opacity(dim ? 0.5 : 1)
        .onAppear {
            guard !still else { return }
            withAnimation(.easeInOut(duration: 1).repeatForever(autoreverses: true)) { dim = true }
        }
        .accessibilityLabel("Loading")
    }
}

/// A list with nothing to show: a line saying why, and a quieter line under it saying what to do.
/// The why reads first (brighter and a little heavier); the hint is a step down.
struct EmptyNote: View {
    let text: String
    var hint: String? = "Pull down to refresh."

    var body: some View {
        VStack(spacing: 6) {
            Text(text).font(.sn(15, 500)).foregroundStyle(SN.textSecondary)
            if let hint {
                Text(hint).font(.sn(13)).foregroundStyle(SN.textMuted)
            }
        }
        .frame(maxWidth: 300)
        .multilineTextAlignment(.center)
        .frame(maxWidth: .infinity)
        .padding(.vertical, 80)
        .padding(.horizontal, 24)
    }
}

/// A full-height spinner for a screen still on its way: the Penrose mark marching.
struct LoadingMark: View {
    var width: CGFloat = 30

    var body: some View {
        PenroseMark(width: width)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
