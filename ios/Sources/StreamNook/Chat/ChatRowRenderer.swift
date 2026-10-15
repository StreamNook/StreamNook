import CoreText
import SNKit
import UIKit

/// Draws a row's text, shapes and box into one bitmap, off the main thread. Images go on layers
/// over it, so the bitmap is drawn once per row and never again while the row lives.
enum ChatRowRenderer {
    static func render(_ layout: ChatRowLayout, scale: CGFloat) -> CGImage? {
        guard layout.height > 0, layout.width > 0 else { return nil }
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        format.opaque = false
        format.preferredRange = .standard
        let size = CGSize(width: layout.width, height: layout.height)
        let image = UIGraphicsImageRenderer(size: size, format: format).image { context in
            draw(layout, in: context.cgContext, size: size)
        }
        return image.cgImage
    }

    static func draw(_ layout: ChatRowLayout, in ctx: CGContext, size: CGSize) {
        let decor = layout.decor
        let box = CGRect(origin: .zero, size: size)
        switch decor.fill {
        case .solid(let color):
            ctx.setFillColor(color.cgColor)
            ctx.fill(box)
        case .tint(let color, let alpha):
            fillHorizontal(ctx, box, [color.withAlphaComponent(alpha), color.withAlphaComponent(alpha / 2), color.withAlphaComponent(0)],
                           locations: [0, 0.5, 1])
        case .gradient(let colors):
            fillHorizontal(ctx, box, colors, locations: nil)
        case nil:
            break
        }
        if let bar = decor.leadingBar {
            ctx.setFillColor(bar.color.cgColor)
            ctx.fill(CGRect(x: 0, y: 0, width: bar.width, height: size.height))
        }
        if let top = decor.topHairline {
            ctx.setFillColor(top.cgColor)
            ctx.fill(CGRect(x: 0, y: 0, width: size.width, height: 1 / max(ctx.ctm.a, 1)))
        }
        if let bottom = decor.bottomHairline {
            ctx.setFillColor(bottom.cgColor)
            let h = 1 / max(ctx.ctm.a, 1)
            ctx.fill(CGRect(x: 0, y: size.height - h, width: size.width, height: h))
        }
        for frost in decor.frost {
            ctx.setFillColor(UIColor(red: 5 / 255, green: 6 / 255, blue: 13 / 255, alpha: 0.66 * frost.alpha).cgColor)
            ctx.addPath(UIBezierPath(roundedRect: frost.rect, cornerRadius: frost.radius).cgPath)
            ctx.fillPath()
        }
        for shape in layout.shapes { drawShape(shape, ctx) }

        ctx.textMatrix = CGAffineTransform(scaleX: 1, y: -1)
        for placed in layout.lines {
            let runs = CTLineGetGlyphRuns(placed.line) as? [CTRun] ?? []
            for run in runs {
                let attrs = CTRunGetAttributes(run) as NSDictionary
                if attrs[ChatRowLayoutBuilder.attachmentKey] != nil { continue }
                let range = CTRunGetStringRange(run)
                var ascent: CGFloat = 0
                var descent: CGFloat = 0
                let width = CGFloat(CTRunGetTypographicBounds(run, CFRange(location: 0, length: 0), &ascent, &descent, nil))
                let x = placed.origin.x + CTLineGetOffsetForStringIndex(placed.line, range.location, nil)
                let bounds = CGRect(x: x, y: placed.origin.y - ascent, width: width, height: ascent + descent)
                ctx.textPosition = placed.origin
                if let box = attrs[ChatRowLayoutBuilder.paintKey] as? PaintBox {
                    if box.paint.layers.isEmpty {
                        drawPainted(run, box.paint, bounds: bounds, ctx)
                    } else {
                        PaintedRun.draw(run, box.paint, bounds: bounds, shadows: layout.paintShadows, ctx)
                    }
                } else {
                    CTRunDraw(run, ctx, CFRange(location: 0, length: 0))
                }
                let color = (attrs[NSAttributedString.Key.foregroundColor] as? UIColor) ?? ChatTheme.textPrimary
                let font = attrs[NSAttributedString.Key.font] as? UIFont
                if attrs[ChatRowLayoutBuilder.underlineKey] != nil {
                    ctx.setFillColor(color.cgColor)
                    ctx.fill(CGRect(x: x, y: placed.origin.y + max(1, (font?.pointSize ?? 14) * 0.09), width: width, height: 1))
                }
                if attrs[ChatRowLayoutBuilder.strikeKey] != nil {
                    ctx.setFillColor(color.cgColor)
                    let mid = placed.origin.y - (font?.xHeight ?? 7) / 2
                    ctx.fill(CGRect(x: x, y: mid, width: width, height: 1))
                }
            }
        }
    }

    private static func fillHorizontal(_ ctx: CGContext, _ rect: CGRect, _ colors: [UIColor], locations: [CGFloat]?) {
        guard let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors.map(\.cgColor) as CFArray,
                                        locations: locations) else { return }
        ctx.saveGState()
        ctx.clip(to: rect)
        ctx.drawLinearGradient(gradient, start: CGPoint(x: rect.minX, y: rect.midY), end: CGPoint(x: rect.maxX, y: rect.midY), options: [])
        ctx.restoreGState()
    }

    private static func drawShape(_ shape: RowShape, _ ctx: CGContext) {
        let r = shape.rect
        switch shape.kind {
        case .capsule(let fill, let stroke):
            let path = UIBezierPath(roundedRect: r, cornerRadius: min(r.height / 2, 8))
            ctx.setFillColor(fill.cgColor)
            ctx.addPath(path.cgPath)
            ctx.fillPath()
            if let stroke {
                ctx.setStrokeColor(stroke.cgColor)
                ctx.setLineWidth(1)
                ctx.addPath(UIBezierPath(roundedRect: r.insetBy(dx: 0.5, dy: 0.5), cornerRadius: min(r.height / 2, 8)).cgPath)
                ctx.strokePath()
            }
        case .bar(let color):
            ctx.setFillColor(color.cgColor)
            ctx.addPath(UIBezierPath(roundedRect: r, cornerRadius: 1).cgPath)
            ctx.fillPath()
        case .dot(let color):
            ctx.setFillColor(color.cgColor)
            ctx.fillEllipse(in: r)
        case .symbol(let name, let color):
            if name.hasPrefix("text:") {
                let label = String(name.dropFirst(5))
                let font = UIFont.systemFont(ofSize: (r.height / 1.6).rounded(), weight: .medium)
                let attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
                let size = (label as NSString).size(withAttributes: attrs)
                UIGraphicsPushContext(ctx)
                (label as NSString).draw(at: CGPoint(x: r.minX, y: r.midY - size.height / 2), withAttributes: attrs)
                UIGraphicsPopContext()
                return
            }
            let config = UIImage.SymbolConfiguration(pointSize: r.height * 0.9, weight: .semibold)
            guard let image = UIImage(systemName: name, withConfiguration: config)?.withTintColor(color, renderingMode: .alwaysOriginal)
            else { return }
            let s = image.size
            let fit = min(r.width / s.width, r.height / s.height)
            let drawn = CGSize(width: s.width * fit, height: s.height * fit)
            UIGraphicsPushContext(ctx)
            image.draw(in: CGRect(x: r.midX - drawn.width / 2, y: r.midY - drawn.height / 2, width: drawn.width, height: drawn.height))
            UIGraphicsPopContext()
        }
    }

    /// A run filled with a 7TV paint: its glyphs become the clip and the paint fills them,
    /// after the paint's shadows are drawn under it.
    private static func drawPainted(_ run: CTRun, _ paint: ChatPaint, bounds: CGRect, _ ctx: CGContext) {
        let first = paint.stops.first.flatMap { ChatTheme.color($0.color) } ?? ChatTheme.textPrimary
        if let shadow = paint.shadows.first, let color = ChatTheme.color(shadow.color) {
            ctx.saveGState()
            ctx.setShadow(offset: CGSize(width: shadow.x, height: shadow.y), blur: max(shadow.blur * 2, 1), color: color.cgColor)
            ctx.setFillColor(first.cgColor)
            CTRunDraw(run, ctx, CFRange(location: 0, length: 0))
            ctx.restoreGState()
        }
        ctx.saveGState()
        ctx.setTextDrawingMode(.clip)
        CTRunDraw(run, ctx, CFRange(location: 0, length: 0))
        ctx.setTextDrawingMode(.fill)
        PaintFill.fill(paint, in: bounds, ctx)
        ctx.restoreGState()
    }
}

/// Fills a rect with a 7TV paint the way CSS backgrounds do: linear gradients along the CSS
/// angle over the box, radial ones from the centre to the farthest corner.
enum PaintFill {
    nonisolated(unsafe) private static let cache = NSCache<NSString, CGGradient>()

    static func fill(_ paint: ChatPaint, in rect: CGRect, _ ctx: CGContext) {
        if paint.kind == "solid" || paint.stops.count < 2 {
            ctx.setFillColor((paint.stops.first.flatMap { ChatTheme.color($0.color) } ?? ChatTheme.textPrimary).cgColor)
            ctx.fill(rect)
            return
        }
        guard let gradient = gradient(paint) else { return }
        if paint.kind == "radial" {
            let center = CGPoint(x: rect.midX, y: rect.midY)
            let radius = hypot(rect.width, rect.height) / 2
            ctx.drawRadialGradient(gradient, startCenter: center, startRadius: 0, endCenter: center, endRadius: radius,
                                   options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
            return
        }
        let theta = paint.angle * .pi / 180
        let dir = CGVector(dx: sin(theta), dy: -cos(theta))
        let length = abs(rect.width * dir.dx) + abs(rect.height * dir.dy)
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let start = CGPoint(x: center.x - dir.dx * length / 2, y: center.y - dir.dy * length / 2)
        let end = CGPoint(x: center.x + dir.dx * length / 2, y: center.y + dir.dy * length / 2)
        ctx.drawLinearGradient(gradient, start: start, end: end, options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
    }

    /// One gradient per paint, made once. A repeating paint has its stop pattern repeated across
    /// the whole line.
    static func gradient(_ paint: ChatPaint) -> CGGradient? {
        if let hit = cache.object(forKey: paint.id as NSString) { return hit }
        var stops = paint.stops.sorted { $0.at < $1.at }
        if paint.repeating, let first = stops.first, let last = stops.last, last.at - first.at > 0.02 {
            let period = last.at - first.at
            var expanded: [ChatPaint.Stop] = []
            var shift = -ceil(first.at / period) * period
            while first.at + shift < 1 {
                for stop in stops { expanded.append(ChatPaint.Stop(at: stop.at + shift, color: stop.color)) }
                shift += period
            }
            stops = expanded.filter { $0.at >= -0.001 && $0.at <= 1.001 }
            if stops.count < 2 { stops = paint.stops }
        }
        let colors = stops.map { (ChatTheme.color($0.color) ?? .white).cgColor }
        let locations = stops.map { CGFloat(min(max($0.at, 0), 1)) }
        guard let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors as CFArray, locations: locations)
        else { return nil }
        cache.setObject(gradient, forKey: paint.id as NSString)
        return gradient
    }
}
