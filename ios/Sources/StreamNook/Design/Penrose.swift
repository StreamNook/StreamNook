import SwiftUI

/// The StreamNook mark: nine isometric cubes forming a Penrose triangle, each marching one slot
/// around it on a 1.5 s loop, colored from the theme accent. It keeps marching whatever the
/// Motion setting says, because a frozen loader reads as a hung app, and draws at the display's
/// full rate.
///
/// The drawing order is the desktop mark's, hand-tuned: several cubes are deliberately drawn in
/// the wrong depth order, which is what makes the triangle impossible. Each internal seam is
/// backed by a narrow strip painted with the darker face's gradient, so the antialiased shared
/// edges never show what is behind them.
struct PenroseMark: View {
    var width: CGFloat = 56
    /// Drawn centered on something: the mark is a triangle pointing right, its cubes massed toward
    /// its upright left side (their mean sits a tenth of the width left of the box's middle), so a
    /// mark centered by its box looks pushed left. It moves right by most of that.
    var centered = true
    /// The logo at rest, as StreamNook's brand mark beside words (never as a loader).
    var still = false

    private static let box = CGRect(x: 14.68, y: 12, width: 128.64, height: 146)
    private static let loop = 1.5
    /// How far right a centered mark moves, as a share of its width.
    static let opticalShift: CGFloat = 0.06

    var body: some View {
        let accent = Look.shared.palette.accent
        TimelineView(.animation(paused: still)) { timeline in
            let raw = still ? 0 : timeline.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: Self.loop) / Self.loop
            Canvas { context, size in
                Self.draw(&context, size: size, raw: raw, accent: accent)
            }
        }
        .frame(width: width, height: width * Self.box.height / Self.box.width)
        .offset(x: centered ? width * Self.opticalShift : 0)
        .accessibilityLabel(still ? "StreamNook" : "Loading")
    }

    private enum Flow { case a, b, c }
    private enum Phase { case a, b }
    private enum Part { case top, right, left, seamV, seamTR, seamTL }

    private struct Layer {
        let flow: Flow
        var phase: Phase?
        let at: CGPoint
        let parts: [Part]
    }

    private static let cube: [Part] = [.seamV, .seamTR, .seamTL, .top, .right, .left]

    private static let layers: [Layer] = [
        Layer(flow: .b, at: CGPoint(x: 66, y: 95), parts: [.seamTL, .left]),
        Layer(flow: .c, at: CGPoint(x: 40, y: 110), parts: [.seamTR, .seamTL, .top]),
        Layer(flow: .c, at: CGPoint(x: 40, y: 80), parts: [.seamTR, .seamTL, .top]),
        Layer(flow: .c, phase: .b, at: CGPoint(x: 40, y: 80), parts: [.seamV, .right]),
        Layer(flow: .c, at: CGPoint(x: 40, y: 50), parts: cube),
        Layer(flow: .a, at: CGPoint(x: 40, y: 20), parts: cube),
        Layer(flow: .a, at: CGPoint(x: 66, y: 35), parts: cube),
        Layer(flow: .a, at: CGPoint(x: 92, y: 50), parts: cube),
        Layer(flow: .b, at: CGPoint(x: 118, y: 65), parts: cube),
        Layer(flow: .b, at: CGPoint(x: 92, y: 80), parts: cube),
        Layer(flow: .b, at: CGPoint(x: 66, y: 95), parts: [.seamV, .seamTR, .top, .right]),
        Layer(flow: .c, at: CGPoint(x: 40, y: 110), parts: [.seamV, .right, .left]),
        Layer(flow: .c, phase: .a, at: CGPoint(x: 40, y: 80), parts: [.seamV, .right]),
        Layer(flow: .c, at: CGPoint(x: 40, y: 80), parts: [.left]),
    ]

    private static func polygon(_ points: [(CGFloat, CGFloat)]) -> Path {
        var path = Path()
        path.addLines(points.map { CGPoint(x: $0.0, y: $0.1) })
        path.closeSubpath()
        return path
    }

    private static let shapes: [Part: Path] = [
        .top: polygon([(0, 0), (17.32, 10), (0, 20), (-17.32, 10)]),
        .right: polygon([(17.32, 10), (17.32, 30), (0, 40), (0, 20)]),
        .left: polygon([(-17.32, 10), (0, 20), (0, 40), (-17.32, 30)]),
        .seamV: polygon([(-0.75, 20), (0.75, 20), (0.75, 39), (-0.75, 39)]),
        .seamTR: polygon([(0.375, 20.65), (16.825, 11.15), (16.075, 9.85), (-0.375, 19.35)]),
        .seamTL: polygon([(-0.375, 20.65), (-16.825, 11.15), (-16.075, 9.85), (0.375, 19.35)]),
    ]

    private static func draw(_ context: inout GraphicsContext, size: CGSize, raw: Double, accent: RGBA) {
        let scale = size.width / box.width
        context.scaleBy(x: scale, y: scale)
        context.translateBy(x: -box.minX, y: -box.minY)
        let p = cubicBezier(0.6, 0, 0.4, 1, raw)
        let top = Gradient(colors: [accent.color, accent.darkened(keeping: 0.80).color])
        let right = Gradient(colors: [accent.darkened(keeping: 0.74).color, accent.darkened(keeping: 0.45).color])
        let left = Gradient(colors: [accent.darkened(keeping: 0.45).color, accent.darkened(keeping: 0.26).color])
        for layer in layers {
            if let phase = layer.phase {
                let firstHalf = raw < 0.5
                if (phase == .a) != firstHalf { continue }
            }
            let shift: CGSize = switch layer.flow {
            case .a: CGSize(width: 26 * p, height: 15 * p)
            case .b: CGSize(width: -26 * p, height: 15 * p)
            case .c: CGSize(width: 0, height: -30 * p)
            }
            var local = context
            local.translateBy(x: layer.at.x + shift.width, y: layer.at.y + shift.height)
            for part in layer.parts {
                guard let path = shapes[part] else { continue }
                let shading: GraphicsContext.Shading = switch part {
                case .top:
                    .linearGradient(top, startPoint: CGPoint(x: -17.32, y: 0), endPoint: CGPoint(x: 17.32, y: 20))
                case .right, .seamTR:
                    .linearGradient(right, startPoint: CGPoint(x: 8.66, y: 10), endPoint: CGPoint(x: 8.66, y: 40))
                case .left, .seamV, .seamTL:
                    .linearGradient(left, startPoint: CGPoint(x: -8.66, y: 10), endPoint: CGPoint(x: -8.66, y: 40))
                }
                local.fill(path, with: shading)
            }
        }
    }

    /// The y of a CSS cubic bezier from (0, 0) to (1, 1) at the point whose x is `x`.
    static func cubicBezier(_ x1: Double, _ y1: Double, _ x2: Double, _ y2: Double, _ x: Double) -> Double {
        func sample(_ a1: Double, _ a2: Double, _ t: Double) -> Double {
            let u = 1 - t
            return 3 * u * u * t * a1 + 3 * u * t * t * a2 + t * t * t
        }
        var low = 0.0
        var high = 1.0
        var t = x
        for _ in 0..<24 {
            let value = sample(x1, x2, t)
            if abs(value - x) < 0.0001 { break }
            if value < x { low = t } else { high = t }
            t = (low + high) / 2
        }
        return sample(y1, y2, t)
    }
}
