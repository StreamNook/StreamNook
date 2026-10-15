import CoreGraphics
import CoreText
import SNKit
import UIKit

/// Draws the core's CSS backgrounds (`SNFill`) and whole 7TV paints into a Core Graphics context,
/// the way CSS paints them: layers bottom to top, linear gradients along the CSS angle over the
/// box, radial ones from their centre, `cover` images centred. Pure drawing, safe off the main
/// thread; images draw only once the shared pipeline already holds them.
enum FillPainter {
    private static let space = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()

    /// `#rrggbb` or `#rrggbbaa` as a colour.
    static func color(_ hex: String) -> CGColor? {
        var s = hex.trimmingCharacters(in: .whitespaces)
        guard s.hasPrefix("#") else { return nil }
        s.removeFirst()
        if s.count == 3 || s.count == 4 { s = s.map { "\($0)\($0)" }.joined() }
        guard s.count == 6 || s.count == 8, let v = UInt64(s, radix: 16) else { return nil }
        let rgba = s.count == 8 ? v : (v << 8) | 0xFF
        return CGColor(colorSpace: space, components: [
            CGFloat((rgba >> 24) & 0xFF) / 255, CGFloat((rgba >> 16) & 0xFF) / 255,
            CGFloat((rgba >> 8) & 0xFF) / 255, CGFloat(rgba & 0xFF) / 255,
        ])
    }

    /// Fills `rect` with every layer of `fill`.
    static func fill(_ fill: SNFill, in rect: CGRect, _ ctx: CGContext, imageScale: CGFloat = 2) {
        for layer in fill.layers.reversed() {
            draw(layer, in: rect, ctx, imageScale: imageScale)
        }
    }

    static func draw(_ layer: SNFill.Layer, in rect: CGRect, _ ctx: CGContext, imageScale: CGFloat = 2) {
        switch layer {
        case .solid(let hex):
            guard let c = color(hex) else { return }
            ctx.setFillColor(c)
            ctx.fill(rect)
        case .linear(let angle, let repeating, let stops):
            linear(angle: angle, repeating: repeating, stops: stops.map { ($0.color, $0.at, $0.px) }, in: rect, ctx)
        case .radial(let circle, let size, let center, let repeating, let stops):
            radial(circle: circle, size: size, center: center, repeating: repeating,
                   stops: stops.map { ($0.color, $0.at, $0.px) }, in: rect, ctx)
        case .image(let url, let cover):
            image(url, cover: cover, in: rect, ctx, scale: imageScale)
        }
    }

    // MARK: Gradients

    typealias RawStop = (color: String, at: Double, px: Bool)

    /// The CSS gradient line for `angle` across `rect`: its start, end and length.
    static func line(angle: Double, in rect: CGRect) -> (CGPoint, CGPoint, CGFloat) {
        let theta = angle * .pi / 180
        let dx = sin(theta)
        let dy = -cos(theta)
        let length = abs(rect.width * dx) + abs(rect.height * dy)
        let c = CGPoint(x: rect.midX, y: rect.midY)
        return (CGPoint(x: c.x - dx * length / 2, y: c.y - dy * length / 2),
                CGPoint(x: c.x + dx * length / 2, y: c.y + dy * length / 2), length)
    }

    /// Stops as fractions of a line `length` points long, repeated across the line when asked.
    static func resolve(_ stops: [RawStop], length: CGFloat, repeating: Bool) -> [(CGColor, CGFloat)] {
        var out: [(CGColor, CGFloat)] = stops.compactMap { s in
            guard let c = color(s.color) else { return nil }
            let at = s.px ? CGFloat(s.at) / max(length, 1) : CGFloat(s.at)
            return (c, at)
        }
        guard repeating, let first = out.first, let last = out.last, last.1 - first.1 > 0.001 else { return out }
        let period = last.1 - first.1
        var expanded: [(CGColor, CGFloat)] = []
        var shift = -ceil(first.1 / period) * period
        while first.1 + shift < 1, expanded.count < 4096 {
            for stop in out { expanded.append((stop.0, stop.1 + shift)) }
            shift += period
        }
        out = expanded
        return out
    }

    static func gradient(_ stops: [(CGColor, CGFloat)]) -> CGGradient? {
        guard !stops.isEmpty else { return nil }
        let list = stops.count == 1 ? [stops[0], (stops[0].0, 1)] : stops
        return CGGradient(colorsSpace: space, colors: list.map(\.0) as CFArray, locations: list.map(\.1))
    }

    static func linear(angle: Double, repeating: Bool, stops: [RawStop], in rect: CGRect, _ ctx: CGContext) {
        let (start, end, length) = line(angle: angle, in: rect)
        guard let g = gradient(resolve(stops, length: length, repeating: repeating)) else { return }
        ctx.saveGState()
        ctx.clip(to: rect)
        ctx.drawLinearGradient(g, start: start, end: end, options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
        ctx.restoreGState()
    }

    static func radial(circle: Bool, size: [Double]?, center: [Double], repeating: Bool, stops: [RawStop],
                       in rect: CGRect, _ ctx: CGContext) {
        let c = CGPoint(x: rect.minX + rect.width * CGFloat(center.first ?? 0.5),
                        y: rect.minY + rect.height * CGFloat(center.count > 1 ? center[1] : 0.5))
        // The ending shape's radii: given, else the farthest corner (ellipse keeps the box's ratio).
        var rx: CGFloat
        var ry: CGFloat
        if let size, size.count == 2 {
            rx = rect.width * CGFloat(size[0])
            ry = rect.height * CGFloat(size[1])
        } else {
            let fx = max(c.x - rect.minX, rect.maxX - c.x)
            let fy = max(c.y - rect.minY, rect.maxY - c.y)
            if circle {
                rx = hypot(fx, fy)
                ry = rx
            } else {
                rx = fx * sqrt(2)
                ry = fy * sqrt(2)
            }
        }
        if circle { ry = rx }
        guard rx > 0, ry > 0, let g = gradient(resolve(stops, length: rx, repeating: repeating)) else { return }
        ctx.saveGState()
        ctx.clip(to: rect)
        // Drawn as a circle of radius rx, squashed vertically into the ellipse.
        ctx.translateBy(x: c.x, y: c.y)
        ctx.scaleBy(x: 1, y: ry / rx)
        ctx.drawRadialGradient(g, startCenter: .zero, startRadius: 0, endCenter: .zero, endRadius: rx,
                               options: [.drawsAfterEndLocation])
        ctx.restoreGState()
    }

    // MARK: Images

    /// The image's first frame, scaled to cover (or fit) `rect`, when the pipeline has it.
    static func image(_ url: String, cover: Bool, in rect: CGRect, _ ctx: CGContext, scale: CGFloat) {
        let pixels = Int((rect.height * scale).rounded(.up))
        guard let image = ImagePipeline.shared.cached(url, pixelHeight: pixels, animated: false)?.first
            ?? ImagePipeline.shared.cachedAnySize(url)?.first else { return }
        draw(image, cover: cover, in: rect, ctx)
    }

    static func draw(_ image: CGImage, cover: Bool, in rect: CGRect, _ ctx: CGContext) {
        let w = CGFloat(image.width)
        let h = CGFloat(image.height)
        guard w > 0, h > 0 else { return }
        let s = cover ? max(rect.width / w, rect.height / h) : min(rect.width / w, rect.height / h)
        let size = CGSize(width: w * s, height: h * s)
        let target = CGRect(x: rect.midX - size.width / 2, y: rect.midY - size.height / 2, width: size.width, height: size.height)
        ctx.saveGState()
        ctx.clip(to: rect)
        // Core Graphics draws images bottom-up; flip into the context's top-down space.
        ctx.translateBy(x: 0, y: target.maxY + target.minY)
        ctx.scaleBy(x: 1, y: -1)
        ctx.draw(image, in: target)
        ctx.restoreGState()
    }
}

/// A whole 7TV paint: its base colour, then every gradient and image layer over it. Paints from an
/// older core (no layers) fall back to their one fill.
enum PaintPainter {
    static func fill(_ paint: ChatPaint, fallback: CGColor, in rect: CGRect, _ ctx: CGContext, imageScale: CGFloat = 2) {
        guard !paint.layers.isEmpty else {
            PaintFill.fill(paint, in: rect, ctx)
            return
        }
        ctx.setFillColor(paint.baseColor.flatMap(FillPainter.color) ?? fallback)
        ctx.fill(rect)
        for layer in paint.layers.reversed() {
            let stops = layer.stops.map { (color: $0.color, at: Double($0.at), px: false) }
            switch layer.kind {
            case "linear":
                FillPainter.linear(angle: layer.angle, repeating: layer.repeating, stops: stops, in: rect, ctx)
            case "radial":
                FillPainter.radial(circle: layer.circle, size: nil, center: [0.5, 0.5], repeating: layer.repeating,
                                   stops: stops, in: rect, ctx)
            case "image":
                if let url = layer.imageURL {
                    // CSS `background-size: 100% 100%`: the image stretched over the name.
                    let pixels = Int((rect.height * imageScale).rounded(.up))
                    if let image = ImagePipeline.shared.cached(url, pixelHeight: pixels, animated: false)?.first
                        ?? ImagePipeline.shared.cachedAnySize(url)?.first {
                        ctx.saveGState()
                        ctx.clip(to: rect)
                        ctx.translateBy(x: 0, y: rect.maxY + rect.minY)
                        ctx.scaleBy(x: 1, y: -1)
                        ctx.draw(image, in: rect)
                        ctx.restoreGState()
                    }
                }
            default:
                break
            }
        }
    }

    /// The image layers a paint draws, so a surface can load them before it draws.
    static func imageURLs(_ paint: ChatPaint) -> [String] {
        paint.layers.compactMap(\.imageURL) + (paint.layers.isEmpty ? [paint.imageURL].compactMap { $0 } : [])
    }
}

/// A run of glyphs (a chat row's name) in a whole 7TV paint: each shadow drops in order under
/// the glyphs, then the glyphs become the clip and the paint fills them, at the paint's opacity.
enum PaintedRun {
    static func draw(_ run: CTRun, _ paint: ChatPaint, bounds: CGRect, shadows: PaintShadows, _ ctx: CGContext) {
        let picked: [ChatPaint.Shadow] = switch shadows {
        case .all: paint.shadows
        case .one: Array(paint.shadows.prefix(1))
        case .none: []
        }
        let base = paint.baseColor.flatMap(FillPainter.color) ?? CGColor(gray: 1, alpha: 1)
        ctx.saveGState()
        if paint.opacity < 1 {
            ctx.setAlpha(paint.opacity)
            ctx.beginTransparencyLayer(auxiliaryInfo: nil)
        }
        for shadow in picked {
            guard let color = FillPainter.color(shadow.color) else { continue }
            ctx.saveGState()
            ctx.setShadow(offset: CGSize(width: shadow.x, height: shadow.y), blur: max(shadow.blur, 0.5), color: color)
            ctx.setFillColor(base)
            CTRunDraw(run, ctx, CFRange(location: 0, length: 0))
            ctx.restoreGState()
        }
        ctx.saveGState()
        ctx.setTextDrawingMode(.clip)
        CTRunDraw(run, ctx, CFRange(location: 0, length: 0))
        ctx.setTextDrawingMode(.fill)
        PaintPainter.fill(paint, fallback: base, in: bounds, ctx)
        ctx.restoreGState()
        if paint.opacity < 1 { ctx.endTransparencyLayer() }
        ctx.restoreGState()
    }
}
