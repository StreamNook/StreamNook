import CoreText
import SNKit
import UIKit

/// What a run delegate reserves on a line: the width of an image (with its margins) and how far
/// it reaches above and below the baseline.
final class RunMetrics {
    let ascent: CGFloat
    let descent: CGFloat
    let width: CGFloat
    init(ascent: CGFloat, descent: CGFloat, width: CGFloat) {
        self.ascent = ascent
        self.descent = descent
        self.width = width
    }
}

/// An inline image or shape waiting for its line position.
struct PendingInline {
    enum Kind {
        case image(RowAttachment.Kind, url: String, pixelHeight: Int, overlays: [String], effects: UInt32, animate: Bool)
        case shape(RowShape.Kind)
        /// A moderation capsule drawn as a shape with its own label.
        case pill(String, UIColor, String)
    }
    var kind: Kind
    var metrics: RunMetrics
    /// Space on each side inside the reserved width.
    var marginLeft: CGFloat
    var marginRight: CGFloat
}

/// A paragraph of a row: attributed text with run delegates standing in for images, laid out
/// into lines by hand so text keeps its CSS-like half-leading and tall images grow only their
/// own line.
struct RowText {
    let builder: ChatRowLayoutBuilder
    private(set) var string = NSMutableAttributedString()
    private var inlines: [PendingInline] = []
    private var taps: [RowTapAction] = []
    /// Kept alive for CoreText, which holds the metrics by pointer.
    private var keep: [RunMetrics] = []

    init(builder: ChatRowLayoutBuilder) {
        self.builder = builder
    }

    var fontSize: CGFloat { builder.fontSize }

    // MARK: Appending

    mutating func append(_ raw: String, _ font: UIFont, _ color: UIColor, extra: [NSAttributedString.Key: Any] = [:]) {
        // Unicode tag characters (the invisible suffix chat clients add to send a line twice) draw
        // nothing in a browser but as wide blanks through Core Text's font fallback.
        let text = raw.unicodeScalars.contains { (0xE0000...0xE007F).contains($0.value) }
            ? String(String.UnicodeScalarView(raw.unicodeScalars.filter { !(0xE0000...0xE007F).contains($0.value) }))
            : raw
        guard !text.isEmpty else { return }
        var attrs: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: color]
        attrs.merge(extra) { $1 }
        string.append(NSAttributedString(string: text, attributes: attrs))
    }

    /// Marks everything appended so far that has no tap yet as a tap target.
    mutating func tap(_ action: RowTapAction) {
        let index = taps.count
        taps.append(action)
        string.addAttribute(ChatRowLayoutBuilder.tapKey, value: index, range: NSRange(location: 0, length: string.length))
    }

    private mutating func inline(_ kind: PendingInline.Kind, width: CGFloat, height: CGFloat, centerAboveBaseline: CGFloat,
                                 marginLeft: CGFloat = 0, marginRight: CGFloat = 0, tapIndex: Int? = nil) {
        let ascent = height / 2 + centerAboveBaseline
        let metrics = RunMetrics(ascent: ascent, descent: height - ascent, width: width + marginLeft + marginRight)
        keep.append(metrics)
        let index = inlines.count
        inlines.append(PendingInline(kind: kind, metrics: metrics, marginLeft: marginLeft, marginRight: marginRight))
        var callbacks = CTRunDelegateCallbacks(
            version: kCTRunDelegateVersion1,
            dealloc: { _ in },
            getAscent: { Unmanaged<RunMetrics>.fromOpaque($0).takeUnretainedValue().ascent },
            getDescent: { Unmanaged<RunMetrics>.fromOpaque($0).takeUnretainedValue().descent },
            getWidth: { Unmanaged<RunMetrics>.fromOpaque($0).takeUnretainedValue().width }
        )
        guard let delegate = CTRunDelegateCreate(&callbacks, Unmanaged.passUnretained(metrics).toOpaque()) else { return }
        var attrs: [NSAttributedString.Key: Any] = [
            NSAttributedString.Key(kCTRunDelegateAttributeName as String): delegate,
            ChatRowLayoutBuilder.attachmentKey: index,
            .foregroundColor: UIColor.clear,
            .font: builder.font(fontSize, .regular),
        ]
        if let tapIndex { attrs[ChatRowLayoutBuilder.tapKey] = tapIndex }
        string.append(NSAttributedString(string: "\u{FFFC}", attributes: attrs))
    }

    /// The text's x-height centre: where CSS `vertical-align: middle` centres an image.
    private func middle(_ size: CGFloat) -> CGFloat {
        builder.font(size, .regular).xHeight / 2
    }

    /// A badge whose image could not load takes no room at all.
    mutating func badge(_ badge: ChatViewBadge, gapAfter: CGFloat) {
        guard !ImagePipeline.shared.hasFailed(badge.url) else { return }
        let size = builder.badgeSize
        inline(.image(.badge, url: badge.url, pixelHeight: Int(size * builder.scale), overlays: [], effects: 0, animate: false),
               width: size, height: size, centerAboveBaseline: 0.38 * fontSize, marginRight: gapAfter)
    }

    /// A round face at badge size: the channel a Shared Chat row came from.
    mutating func face(_ url: String, gapAfter: CGFloat) {
        guard !ImagePipeline.shared.hasFailed(url) else { return }
        let size = builder.badgeSize
        inline(.image(.avatar, url: url, pixelHeight: Int(size * builder.scale), overlays: [], effects: 0, animate: false),
               width: size, height: size, centerAboveBaseline: 0.38 * fontSize, marginRight: gapAfter)
    }

    mutating func symbol(_ name: String, size: CGFloat, color: UIColor) {
        inline(.shape(.symbol(name, color)), width: size, height: size, centerAboveBaseline: middle(size), marginRight: 0)
    }

    mutating func bar(_ color: UIColor) {
        inline(.shape(.bar(color)), width: 2, height: (0.95 * fontSize).rounded(), centerAboveBaseline: middle(fontSize),
               marginRight: (0.4 * fontSize).rounded())
    }

    mutating func dot(_ color: UIColor) {
        let d = (0.45 * fontSize).rounded()
        inline(.shape(.dot(color)), width: d, height: d, centerAboveBaseline: middle(fontSize), marginRight: (0.4 * fontSize).rounded())
    }

    /// The author's name: bold, in their colour or 7TV paint, a tap that opens their profile.
    mutating func name(_ text: String, font: UIFont, color: UIColor, paint: ChatPaint?, chip: UIColor?) {
        let tapIndex = taps.count
        taps.append(.name)
        var extra: [NSAttributedString.Key: Any] = [ChatRowLayoutBuilder.tapKey: tapIndex]
        if let paint, paint.kind != "image" { extra[ChatRowLayoutBuilder.paintKey] = PaintBox(paint) }
        if let chip { extra[ChatRowLayoutBuilder.capsuleKey] = chip }
        append(text, font, color, extra: extra)
    }

    mutating func pill(_ mark: ChatModeration) {
        let label: String
        let tone: UIColor
        let icon: String
        switch mark {
        case .deleted:
            label = "Deleted"; tone = ChatTheme.textSecondary; icon = "trash"
        case .timedOut(let s):
            label = s > 0 ? "Timed out · \(ChatRowLayoutBuilder.duration(s))" : "Timed out"; tone = ChatTheme.warning; icon = "timer"
        case .banned:
            label = "Banned"; tone = ChatTheme.error; icon = "nosign"
        }
        let size = (0.78 * fontSize).rounded()
        let textWidth = (label as NSString).size(withAttributes: [.font: builder.font(size, .medium)]).width
        let height = (size * 1.6).rounded()
        let width = ceil(textWidth + size * 1.1 + 4 + height * 0.8)
        inline(.pill(label, tone, icon), width: width, height: height, centerAboveBaseline: middle(fontSize))
    }

    /// Body spans: text, mentions, links, emoji, emotes with their overlays, cheers and GIFs.
    mutating func spans(_ spans: [ChatSpan], size: CGFloat, color: UIColor, emoteHeight: CGFloat, weight: UIFont.Weight,
                        italic: Bool = false, paint: ChatPaint? = nil, strike: Bool = false) {
        let style = builder.style
        let body = builder.font(size, weight, italic: italic)
        var common: [NSAttributedString.Key: Any] = [:]
        if strike { common[ChatRowLayoutBuilder.strikeKey] = true }
        if let paint, paint.kind != "image" { common[ChatRowLayoutBuilder.paintKey] = PaintBox(paint) }
        let margin = builder.emoteMargin
        for span in spans {
            switch span {
            case .text(let text):
                append(text, body, color, extra: common)
            case .mention(let text, let login, let mentionColor):
                let mentionWeight: UIFont.Weight = switch style.mentionWeight {
                case "normal": weight
                case "semibold": .semibold
                case "bold": .bold
                default: .medium
                }
                let slanted = style.mentionItalic == "italic" || (style.mentionItalic == "inherit" && italic)
                let tint = ChatTheme.color(style.mentionTextColor) ?? ChatTheme.color(mentionColor) ?? color
                let tapIndex = taps.count
                taps.append(.mention(login))
                var extra = common
                extra[ChatRowLayoutBuilder.tapKey] = tapIndex
                if style.mentionStyle == "pill" { extra[ChatRowLayoutBuilder.capsuleKey] = tint }
                append(text, builder.font(size, mentionWeight, italic: slanted), tint, extra: extra)
            case .link(let label, let url):
                let tint = ChatTheme.color(style.linkColor.isEmpty ? nil : style.linkColor) ?? ChatTheme.info
                var extra = common
                if let target = URL(string: url) {
                    extra[ChatRowLayoutBuilder.tapKey] = taps.count
                    taps.append(.link(target))
                }
                if style.linkUnderline { extra[ChatRowLayoutBuilder.underlineKey] = true }
                append(label, body, tint, extra: extra)
            case .emoji(let text, _):
                // Apple's own glyphs serve the default emoji style, at the 20 px the images use.
                append(text, builder.font((size * 20 / 14).rounded(), .regular), color, extra: common)
            case .emote(let emote, let overlays, let effects):
                if emote.personal && !style.showPersonalEmotes {
                    append(emote.code, body, color, extra: common)
                    continue
                }
                let pixelHeight = Double(emoteHeight * builder.scale)
                let url = Self.emoteURL(emote.url, pixelHeight: pixelHeight)
                var h = emoteHeight
                var w = h * CGFloat(emote.aspect ?? EmoteAspects.shared.aspect(url) ?? 1)
                let rotated = effects & (Modifier.rotateLeft | Modifier.rotateRight | Modifier.rotate90) != 0
                if effects & Modifier.growX != 0 { w = min(w * 2, 256 * CGFloat(style.emoteScale)) }
                if effects & Modifier.wide != 0 && !rotated { w = min(h * 4, 512 * CGFloat(style.emoteScale)) }
                if rotated { h = (1.75 * 16 * CGFloat(style.emoteScale)).rounded(); w = h }
                w = min(w, 9 * size * CGFloat(style.emoteScale))
                let noSpace = effects & Modifier.noSpace != 0 && string.length > 0
                inline(.image(.emote, url: url, pixelHeight: Int(pixelHeight),
                              overlays: overlays.filter { !$0.hidden }.map { Self.emoteURL($0.url, pixelHeight: pixelHeight) },
                              effects: effects, animate: style.animatesEmotes),
                       width: w, height: h, centerAboveBaseline: middle(size),
                       marginLeft: noSpace ? -4 : margin, marginRight: margin)
            case .cheer(_, let bits, let tierColor, let url):
                let h = (28 * size / 14).rounded()
                inline(.image(.cheer, url: url, pixelHeight: Int(h * builder.scale), overlays: [], effects: 0, animate: style.animatesEmotes),
                       width: h, height: h, centerAboveBaseline: middle(size))
                append("\(bits)", builder.font(size, .bold), ChatTheme.color(tierColor) ?? ChatTheme.color("#979797")!, extra: common)
            case .gif(let text, let url):
                if style.showChatGifs && style.animatesEmotes {
                    let h = (80 * CGFloat(style.gifScale)).rounded()
                    let w = h * CGFloat(EmoteAspects.shared.aspect(url) ?? 1.0)
                    inline(.image(.gif, url: url, pixelHeight: Int(h * builder.scale), overlays: [], effects: 0, animate: true),
                           width: w, height: h, centerAboveBaseline: h - builder.font(size, .regular).ascender)
                } else {
                    append("GIF ", builder.font(12, .semibold), ChatTheme.textSecondary, extra: common)
                    append(text, builder.font(12, .regular), ChatTheme.textSecondary, extra: common)
                }
            }
        }
    }

    /// The URL of the CDN size the drawn height needs, in a format ImageIO decodes reliably
    /// (7TV's WebP over its AVIF).
    static func emoteURL(_ url: String, pixelHeight: Double) -> String {
        var tiered = ImageTier.url(url, pixelHeight: pixelHeight)
        if tiered.contains("cdn.7tv.app/emote/"), tiered.hasSuffix(".avif") {
            tiered = String(tiered.dropLast(5)) + ".webp"
        }
        return tiered
    }

    // MARK: Layout

    struct Placed {
        var lines: [PlacedLine] = []
        var attachments: [RowAttachment] = []
        var shapes: [RowShape] = []
        var taps: [RowTap] = []
        var bottom: CGFloat
    }

    /// Breaks the paragraph into lines `width` wide starting at `top`, each line as tall as CSS
    /// line-height 1.625 makes it, or taller where an image needs it.
    func layout(top: CGFloat, left: CGFloat, width: CGFloat, maxLines: Int, alignRight: Bool) -> Placed {
        var placed = Placed(bottom: top)
        guard string.length > 0, width > 10 else { return placed }
        let reference = builder.font(fontSize, .regular)
        let leading = max(0, builder.lineHeight - (reference.ascender - reference.descender)) / 2
        let textAbove = reference.ascender + leading
        let textBelow = -reference.descender + leading
        let typesetter = CTTypesetterCreateWithAttributedString(string)
        var start = 0
        var y = top
        var count = 0
        let length = string.length
        while start < length {
            count += 1
            let last = maxLines > 0 && count == maxLines
            var lineLength = CTTypesetterSuggestLineBreak(typesetter, start, Double(width))
            if lineLength <= 0 { lineLength = 1 }
            var line = CTTypesetterCreateLine(typesetter, CFRange(location: start, length: last ? length - start : lineLength))
            if last, CTLineGetTypographicBounds(line, nil, nil, nil) > width,
               let truncated = CTLineCreateTruncatedLine(line, Double(width), .end, Self.ellipsis(reference)) {
                line = truncated
            }
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            let lineWidth = CGFloat(CTLineGetTypographicBounds(line, &ascent, &descent, nil))
            let above = max(textAbove, ascent)
            let below = max(textBelow, descent)
            let baseline = y + above
            let x = alignRight ? left + max(0, width - lineWidth) : left
            placed.lines.append(PlacedLine(line: line, origin: CGPoint(x: x, y: baseline)))
            collect(line, origin: CGPoint(x: x, y: baseline), lineTop: y, lineBottom: baseline + below, into: &placed)
            y = baseline + below
            start += last ? length - start : lineLength
        }
        placed.bottom = y
        return placed
    }

    private static func ellipsis(_ font: UIFont) -> CTLine {
        CTLineCreateWithAttributedString(NSAttributedString(string: "…", attributes: [.font: font, .foregroundColor: ChatTheme.textSecondary]))
    }

    /// Images, shapes and tap regions of one line, from its runs.
    private func collect(_ line: CTLine, origin: CGPoint, lineTop: CGFloat, lineBottom: CGFloat, into placed: inout Placed) {
        let runs = CTLineGetGlyphRuns(line) as? [CTRun] ?? []
        var tapRects: [Int: CGRect] = [:]
        for run in runs {
            let attrs = CTRunGetAttributes(run) as NSDictionary
            let range = CTRunGetStringRange(run)
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            let runWidth = CGFloat(CTRunGetTypographicBounds(run, CFRange(location: 0, length: 0), &ascent, &descent, nil))
            let x = origin.x + CTLineGetOffsetForStringIndex(line, range.location, nil)
            if let index = attrs[ChatRowLayoutBuilder.attachmentKey] as? Int, index < inlines.count {
                let item = inlines[index]
                let m = item.metrics
                let rect = CGRect(x: x + item.marginLeft, y: origin.y - m.ascent,
                                  width: m.width - item.marginLeft - item.marginRight, height: m.ascent + m.descent)
                switch item.kind {
                case .image(let kind, let url, let pixelHeight, let overlays, let effects, let animate):
                    placed.attachments.append(RowAttachment(kind: kind, url: url, pixelHeight: pixelHeight, rect: rect,
                                                            overlays: overlays, effects: effects, animate: animate))
                case .shape(let kind):
                    placed.shapes.append(RowShape(kind: kind, rect: rect))
                case .pill(let label, let tone, let icon):
                    placed.shapes.append(RowShape(kind: .capsule(fill: tone.withAlphaComponent(0.12), stroke: tone.withAlphaComponent(0.24)), rect: rect))
                    let size = (0.78 * fontSize).rounded()
                    let iconRect = CGRect(x: rect.minX + rect.height * 0.4, y: rect.midY - size * 0.55, width: size * 1.1, height: size * 1.1)
                    placed.shapes.append(RowShape(kind: .symbol(icon, tone), rect: iconRect))
                    placed.shapes.append(RowShape(kind: .symbol("text:" + label, tone),
                                                  rect: CGRect(x: iconRect.maxX + 4, y: rect.minY, width: rect.maxX - iconRect.maxX - 4, height: rect.height)))
                }
            }
            if let capsule = attrs[ChatRowLayoutBuilder.capsuleKey] as? UIColor {
                let pad = (0.45 * fontSize).rounded() / 2
                placed.shapes.insert(RowShape(kind: .capsule(fill: capsule.withAlphaComponent(0.16), stroke: capsule.withAlphaComponent(0.34)),
                                              rect: CGRect(x: x - pad, y: origin.y - ascent - 1, width: runWidth + 2 * pad, height: ascent + descent + 2)),
                                     at: 0)
            }
            if let tap = attrs[ChatRowLayoutBuilder.tapKey] as? Int {
                let rect = CGRect(x: x, y: lineTop, width: runWidth, height: lineBottom - lineTop)
                tapRects[tap] = tapRects[tap].map { $0.union(rect) } ?? rect
            }
        }
        for (index, rect) in tapRects where index < taps.count {
            placed.taps.append(RowTap(rect: rect.insetBy(dx: -4, dy: 0), action: taps[index]))
        }
    }
}

/// Emote modifier bits the renderer draws (the table in `emoteModifiers.ts`).
enum Modifier {
    static let flipX: UInt32 = 1 << 1
    static let flipY: UInt32 = 1 << 2
    static let growX: UInt32 = 1 << 3
    static let slide: UInt32 = 1 << 4
    static let appear: UInt32 = 1 << 5
    static let leave: UInt32 = 1 << 6
    static let rotate: UInt32 = 1 << 7
    static let rotate90: UInt32 = 1 << 8
    static let greyscale: UInt32 = 1 << 9
    static let sepia: UInt32 = 1 << 10
    static let rainbow: UInt32 = 1 << 11
    static let hyperRed: UInt32 = 1 << 12
    static let shake: UInt32 = 1 << 13
    static let cursed: UInt32 = 1 << 14
    static let jam: UInt32 = 1 << 15
    static let bounce: UInt32 = 1 << 16
    static let noSpace: UInt32 = 1 << 17
    static let wide: UInt32 = 1 << 19
    static let rotateLeft: UInt32 = 1 << 20
    static let rotateRight: UInt32 = 1 << 21
    static let party: UInt32 = 1 << 22
    static let bttvShake: UInt32 = 1 << 23
}

/// Shapes of emotes the core did not size, learned when their image first loads, so later rows
/// lay them out right from the start.
final class EmoteAspects: @unchecked Sendable {
    static let shared = EmoteAspects()
    private let lock = NSLock()
    private var aspects: [String: Double] = [:]

    func aspect(_ url: String) -> Double? {
        lock.withLock { aspects[url] }
    }

    func note(_ url: String, aspect: Double) {
        lock.withLock {
            if aspects.count > 4000 { aspects.removeAll() }
            aspects[url] = aspect
        }
    }
}

/// A paint carried on attributed text runs.
final class PaintBox: NSObject {
    let paint: ChatPaint
    init(_ paint: ChatPaint) { self.paint = paint }
}
