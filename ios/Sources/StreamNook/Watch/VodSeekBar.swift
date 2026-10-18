import SNKit
import SwiftUI

/// A VOD's seek bar over the bottom of the picture: what has played in the accent, the parts
/// Twitch muted striped, a tick where the category changed, and the knob. Dragging anywhere on it
/// scrubs (the time follows the finger), and letting go seeks there. The two times below keep
/// their width as they count.
struct VodSeekBar: View {
    let watch: Watch
    let onTouch: () -> Void
    @State private var scrub: Double?

    private static let track: CGFloat = 4
    private static let knob: CGFloat = 14

    var body: some View {
        let duration = max(watch.vodDuration, 1)
        let position = scrub.map { $0 * duration } ?? min(watch.vodPosition, duration)
        let fraction = position / duration
        VStack(spacing: 2) {
            GeometryReader { geo in
                let width = geo.size.width
                ZStack(alignment: .leading) {
                    Capsule().fill(Color.white.opacity(0.25)).frame(height: Self.track)
                    Capsule().fill(SN.accent).frame(width: max(0, width * fraction), height: Self.track)
                    muted(width: width, duration: duration)
                    ForEach(chapterStarts(duration: duration), id: \.self) { start in
                        Capsule()
                            .fill(Color.white.opacity(0.85))
                            .frame(width: 2, height: 10)
                            .offset(x: width * start - 1)
                    }
                    Circle()
                        .fill(.white)
                        .frame(width: Self.knob, height: Self.knob)
                        .scaleEffect(scrub == nil ? 1 : 1.3)
                        .offset(x: width * fraction - Self.knob / 2)
                }
                .frame(height: 28)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { value in
                            scrub = min(1, max(0, value.location.x / max(width, 1)))
                            onTouch()
                        }
                        .onEnded { _ in
                            if let scrub { watch.seekVod(to: scrub * duration) }
                            scrub = nil
                            onTouch()
                        }
                )
                .animation(.easeOut(duration: 0.12), value: scrub == nil)
            }
            .frame(height: 28)
            HStack {
                Text(PlayerText.clock(position, like: duration))
                Spacer(minLength: 8)
                Text(PlayerText.clock(duration))
            }
            .font(.sn(11.5, 500).monospacedDigit())
            .foregroundStyle(.white.opacity(0.8))
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Position")
        .accessibilityValue("\(PlayerText.clock(position)) of \(PlayerText.clock(duration))")
        .accessibilityAdjustableAction { direction in
            watch.skipVod(by: direction == .increment ? 10 : -10)
        }
    }

    /// Muted stretches, striped so they read as "no sound here" rather than as more progress.
    private func muted(width: CGFloat, duration: Double) -> some View {
        Canvas { context, size in
            for range in watch.playingVod?.mutedSegments ?? [] {
                let from = width * range.startSecs / duration
                let to = width * min(range.endSecs, duration) / duration
                var x = from
                while x < to {
                    let bar = CGRect(x: x, y: (size.height - Self.track) / 2, width: min(2, to - x), height: Self.track)
                    context.fill(Path(bar), with: .color(.white.opacity(0.6)))
                    x += 4
                }
            }
        }
        .frame(width: width, height: 28)
        .allowsHitTesting(false)
    }

    /// Where each chapter after the first begins, as a fraction of the bar.
    private func chapterStarts(duration: Double) -> [Double] {
        (watch.playingVod?.chapters ?? []).map(\.startSecs).filter { $0 > 1 && $0 < duration }.map { $0 / duration }
    }
}
