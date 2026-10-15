import SNKit
import SwiftUI

/// How many of a paint's shadows to draw: the profile surfaces draw every one; chat follows the
/// viewer's `cosmetics.paint_shadows`.
enum PaintShadows: String {
    case all, one, none
}

/// Text drawn in a 7TV paint: its glyphs mask the paint's layers (base colour, gradients and an
/// image layer stretched over the whole text box, animated when it moves), its shadows drop under
/// the result in order, and the paint's opacity applies to all of it. Without a paint, plain text
/// in `fallback`.
struct PaintedText: View {
    let text: String
    let font: Font
    let paint: ChatPaint?
    var fallback: Color = SN.textPrimary
    var shadows: PaintShadows = .all
    var lineLimit: Int? = 1
    var alignment: TextAlignment = .leading

    var body: some View {
        if let paint {
            label
                .foregroundStyle(.clear)
                .overlay {
                    PaintFillView(paint: paint, fallback: fallback)
                        .mask { label }
                }
                .modifier(PaintShadowStack(shadows: picked(paint)))
                .opacity(paint.opacity)
                .accessibilityElement()
                .accessibilityLabel(text)
        } else {
            label.foregroundStyle(fallback)
        }
    }

    private var label: some View {
        Text(text)
            .font(font)
            .lineLimit(lineLimit)
            .multilineTextAlignment(alignment)
    }

    private func picked(_ paint: ChatPaint) -> [ChatPaint.Shadow] {
        switch shadows {
        case .all: paint.shadows
        case .one: Array(paint.shadows.prefix(1))
        case .none: []
        }
    }
}

/// Drop shadows applied one after another, the way CSS chains `drop-shadow()` filters: each
/// shadows everything drawn before it.
private struct PaintShadowStack: ViewModifier {
    let shadows: [ChatPaint.Shadow]

    func body(content: Content) -> some View {
        shadows.reduce(AnyView(content)) { view, s in
            AnyView(view.shadow(color: Color(cgColor: FillPainter.color(s.color) ?? CGColor(gray: 0, alpha: 0.5)),
                                radius: max(s.blur / 2, 0), x: s.x, y: s.y))
        }
    }
}

/// A paint's fill over a box: the base colour and gradients drawn by `PaintPainter`, with an
/// image layer on top as a live (animated) image.
struct PaintFillView: View {
    let paint: ChatPaint
    var fallback: Color = SN.textPrimary
    @Environment(\.displayScale) private var scale

    private var imageLayer: String? {
        paint.layers.isEmpty ? (paint.kind == "image" ? paint.imageURL : nil)
            : paint.layers.first(where: { $0.kind == "image" })?.imageURL
    }

    var body: some View {
        ZStack {
            Canvas { context, size in
                let fallbackColor = UIColor(fallback).cgColor
                var gradientOnly = paint
                gradientOnly.layers.removeAll { $0.kind == "image" }
                context.withCGContext { cg in
                    PaintPainter.fill(gradientOnly, fallback: fallbackColor, in: CGRect(origin: .zero, size: size), cg,
                                      imageScale: scale)
                }
            }
            if let imageLayer {
                CosmeticImage(imageLayer, animate: true, stretch: true)
            }
        }
    }
}

/// A CSS background from the core drawn in a box: an Atmosphere's swatch, a chat row's edge. Image
/// layers load through the pipeline and the box redraws when they arrive.
struct FillView: View {
    let fill: SNFill
    @Environment(\.displayScale) private var scale
    @State private var loaded = 0

    var body: some View {
        Canvas { context, size in
            _ = loaded
            context.withCGContext { cg in
                FillPainter.fill(fill, in: CGRect(origin: .zero, size: size), cg, imageScale: scale)
            }
        }
        .task(id: fill) {
            for case .image(let url, _) in fill.layers {
                let have = ImagePipeline.shared.cachedAnySize(url) != nil
                if have { continue }
                let _: DecodedImage? = await withCheckedContinuation { done in
                    ImagePipeline.shared.load(url, pixelHeight: Int(160 * scale), animated: false) { done.resume(returning: $0) }
                }
                loaded += 1
            }
        }
    }
}
