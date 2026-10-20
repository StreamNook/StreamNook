import CoreText
import SNKit
import UIKit

/// An image inside a row: an emote with what stacks over it, a badge, a cheer gem, a GIF or a
/// gigantified emote. Its rect is fixed before the image loads, so a row never reflows.
struct RowAttachment {
    enum Kind { case emote, badge, cheer, gif, giant, avatar, preview }
    var kind: Kind
    var url: String
    /// The pixel height to decode at.
    var pixelHeight: Int
    var rect: CGRect
    var overlays: [String]
    var effects: UInt32
    var animate: Bool
    /// A link card's picture: its corner radius, filled to its box.
    var radius: CGFloat = 0
}

/// Where a tap on a row goes.
enum RowTapAction: Equatable {
    case name
    case mention(String)
    case link(URL)
    case replyParent(String)
    /// Allow (`allow`) or deny the held message with this Twitch message id.
    case held(String, allow: Bool)
}

struct RowTap {
    var rect: CGRect
    var action: RowTapAction
}

/// How a row's box is dressed, drawn into its bitmap except the arrival flash.
struct RowDecor {
    enum Fill {
        case solid(UIColor)
        /// A left-to-right fade from `alpha` through half of it to clear, in `color`.
        case tint(UIColor, alpha: CGFloat)
        case gradient([UIColor])
    }
    var fill: Fill?
    var leadingBar: (color: UIColor, width: CGFloat)?
    var topHairline: UIColor?
    var bottomHairline: UIColor?
    /// The flash a live row plays as it lands, under its text, tint and bar (run by the cell).
    var flash: RowFlash?
    var alpha: CGFloat = 1
    /// The author's StreamNook Atmosphere (or Cologne look), drawn behind the row by the cell.
    var atmosphere: AtmosphereLook?
    var cologne: CologneLook?
    /// Frosted blocks behind the text when the Atmosphere asks for them (drawn into the bitmap).
    var frost: [RowFrost] = []
    /// An outline of the whole row in `color` (a first-time chatter's ring), with a faint fill
    /// inside when `fill` is set. Drawn into the bitmap.
    var ring: (color: UIColor, fill: Bool)?
    /// The glint a live row plays over its ring or left bar, run by the cell.
    var glint: RowGlint?
}

/// The color a live row lights up with as it lands, fading to clear over two seconds: desktop's
/// keyframes, an opacity step at even times with each step easing out (CSS `ease-out` runs per
/// keyframe). The cell animates only the opacity of one layer, so nothing redraws per frame.
struct RowFlash {
    var color: UIColor
    /// The strength at even times from the start (0) to the end (clear).
    var alphas: [CGFloat]

    static let duration: CFTimeInterval = 2

    /// A mention of you: 50, 40, 30, 20 percent, then clear (`mention-flash`).
    static func mention(_ color: UIColor) -> RowFlash { RowFlash(color: color, alphas: [0.5, 0.4, 0.3, 0.2, 0]) }
    /// A reply to you: 30, 15 percent, then clear (`reply-flash`).
    static func reply(_ color: UIColor) -> RowFlash { RowFlash(color: color, alphas: [0.3, 0.15, 0]) }
    /// A phrase, user or badge highlight: 35, 18 percent, then clear (`phrase-highlight-flash`).
    static func highlight(_ color: UIColor) -> RowFlash { RowFlash(color: color, alphas: [0.35, 0.18, 0]) }
}

/// A highlight that crosses a row's ring or left bar when it arrives live, as Core Animation
/// layers on the cell so nothing redraws per frame.
struct RowGlint: Equatable {
    enum Kind: String { case sheen, pulse, chase }
    var kind: Kind
    /// Over the ring; otherwise down the bar on the left.
    var onRing: Bool
    /// Keeps cycling instead of playing once.
    var loop: Bool

    /// The ring's corner radius, shared by the bitmap and the glint's mask.
    static let ringRadius: CGFloat = 8
    static let barWidth: CGFloat = 4
}

/// A readability block behind a row's text over a busy Atmosphere: the tint without a per-row
/// blur, as phones draw it (`atm-frost--flat`).
struct RowFrost {
    var rect: CGRect
    var radius: CGFloat
    var alpha: CGFloat
}

/// A laid-out line: CoreText line plus where its baseline sits.
struct PlacedLine {
    var line: CTLine
    var origin: CGPoint
}

/// Shapes a row draws that are not glyphs or images: capsules behind text, SF Symbols.
struct RowShape {
    enum Kind {
        case capsule(fill: UIColor, stroke: UIColor?)
        case bar(UIColor)
        case dot(UIColor)
        case symbol(String, UIColor)
        /// One of the desktop's event glyphs (the Prime crown, the raid group, the streak flame).
        case glyph(EventGlyph, UIColor)
        /// A link card's panel: the floor and wash, a dark contour and light along the top.
        case card(radius: CGFloat)
    }
    var kind: Kind
    var rect: CGRect
}

/// One row worked out off the main thread: its height, lines, image rects, tap regions and
/// decoration. The bitmap of its text is drawn once and kept while the row is near the screen.
final class ChatRowLayout: @unchecked Sendable {
    let id: String
    let revision: Int
    let width: CGFloat
    let height: CGFloat
    let lines: [PlacedLine]
    let attachments: [RowAttachment]
    let shapes: [RowShape]
    let taps: [RowTap]
    let decor: RowDecor
    let accessibilityText: String
    let live: Bool
    let mentionsMe: Bool
    /// How many of a paint's shadows names on this row draw (`cosmetics.paint_shadows`).
    var paintShadows: PaintShadows = .all

    private let lock = NSLock()
    private var _bitmap: CGImage?

    init(id: String, revision: Int, width: CGFloat, height: CGFloat, lines: [PlacedLine], attachments: [RowAttachment],
         shapes: [RowShape], taps: [RowTap], decor: RowDecor, accessibilityText: String,
         live: Bool, mentionsMe: Bool) {
        self.id = id
        self.revision = revision
        self.width = width
        self.height = height
        self.lines = lines
        self.attachments = attachments
        self.shapes = shapes
        self.taps = taps
        self.decor = decor
        self.accessibilityText = accessibilityText
        self.live = live
        self.mentionsMe = mentionsMe
    }

    var bitmap: CGImage? {
        lock.withLock { _bitmap }
    }

    var hasBitmap: Bool { lock.withLock { _bitmap != nil } }

    /// Draws the bitmap if it is not there yet. Safe from any thread.
    @discardableResult
    func renderBitmap(scale: CGFloat) -> CGImage? {
        if let done = bitmap { return done }
        let image = ChatRowRenderer.render(self, scale: scale)
        lock.withLock { _bitmap = image }
        return image
    }

    func dropBitmap() {
        lock.withLock { _bitmap = nil }
    }

    var bitmapBytes: Int {
        lock.withLock { _bitmap.map { $0.bytesPerRow * $0.height } ?? 0 }
    }
}

// MARK: - Building

/// Turns a room row into a layout at a width. Pure and thread-safe: runs on the chat queue.
struct ChatRowLayoutBuilder {
    let style: ChatStyle
    let width: CGFloat
    let scale: CGFloat
    /// The 7TV paint of each chatter the room has seen, by lowercased login: an @mention of them
    /// wears it while Chat > Paint mentions is on.
    var mentionPaints: [String: ChatPaint] = [:]

    static let attachmentKey = NSAttributedString.Key("sn.att")
    static let tapKey = NSAttributedString.Key("sn.tap")
    static let paintKey = NSAttributedString.Key("sn.paint")
    static let strikeKey = NSAttributedString.Key("sn.strike")
    static let underlineKey = NSAttributedString.Key("sn.underline")
    static let capsuleKey = NSAttributedString.Key("sn.capsule")
    static let shadowKey = NSAttributedString.Key("sn.textshadow")

    var fontSize: CGFloat { CGFloat(style.fontSize) }
    var lineHeight: CGFloat { (fontSize * 1.625).rounded(.toNearestOrEven) }
    var sideInset: CGFloat { 12 }
    var emoteHeight: CGFloat { (2 * fontSize * CGFloat(style.emoteScale)).rounded() }
    var badgeSize: CGFloat { (1.4 * fontSize * CGFloat(style.badgeScale)).rounded() }
    var emoteMargin: CGFloat { CGFloat(style.emoteMargin) * 16 }

    func font(_ size: CGFloat, _ weight: UIFont.Weight, italic: Bool = false) -> UIFont {
        let base = ChatFont.font(size, weight)
        guard italic, let descriptor = base.fontDescriptor.withSymbolicTraits(.traitItalic) else { return base }
        return UIFont(descriptor: descriptor, size: size)
    }

    var bodyWeight: UIFont.Weight { ChatFont.bodyWeight(textWeight: style.fontWeight) }

    func build(_ row: ChatRowModel) -> ChatRowLayout {
        let message = row.message
        let meta = message.metadata
        let view = meta.view
        let hidden = row.moderation != nil && style.deletedMessageStyle == "hidden"
            || (style.hideSharedChat && meta.isFromSharedChat)
        if hidden {
            return ChatRowLayout(id: row.id, revision: row.revision, width: width, height: 0, lines: [], attachments: [],
                                 shapes: [], taps: [], decor: RowDecor(), accessibilityText: "",
                                 live: row.live, mentionsMe: false)
        }

        var decor = RowDecor()
        var top: CGFloat = 0
        var lines: [PlacedLine] = []
        var attachments: [RowAttachment] = []
        var shapes: [RowShape] = []
        var taps: [RowTap] = []
        let padding = CGFloat(style.rowPadding)
        let msgID = message.tags["msg-id"] ?? ""
        let kind = Self.kind(of: message, msgID: msgID)

        func place(_ block: RowText, indent: CGFloat = 0, maxLines: Int = 0, alignRight: Bool = false) {
            let placed = block.layout(top: top, left: sideInset + indent, width: width - 2 * sideInset - indent,
                                      maxLines: maxLines, alignRight: alignRight)
            lines += placed.lines
            attachments += placed.attachments
            taps += placed.taps
            shapes += placed.shapes
            top = placed.bottom
        }

        // A message AutoMod held: the moderator's Allow and Deny under it, one quiet line once decided.
        if let held = row.held {
            return buildHeld(row, held)
        }

        // System rows: a notice with no name or badges.
        if kind == .system {
            decor.fill = .solid(ChatTheme.warning.withAlphaComponent(0.10))
            decor.topHairline = ChatTheme.warning.withAlphaComponent(0.20)
            decor.bottomHairline = ChatTheme.warning.withAlphaComponent(0.20)
            top = 8
            var block = RowText(builder: self)
            block.append(message.content, font(12, .medium), ChatTheme.warning)
            let iconSize: CGFloat = 16
            shapes.append(RowShape(kind: .symbol("info.circle", ChatTheme.warning.withAlphaComponent(0.8)),
                                   rect: CGRect(x: sideInset, y: top + 1, width: iconSize, height: iconSize)))
            place(block, indent: iconSize + 8)
            top += 8
            return ChatRowLayout(id: row.id, revision: row.revision, width: width, height: ceil(top), lines: lines,
                                 attachments: attachments, shapes: shapes, taps: taps, decor: decor,
                                 accessibilityText: message.content, live: row.live, mentionsMe: false)
        }

        top = padding
        if style.showDividers { decor.topHairline = ChatTheme.borderSubtle }
        // The author's Atmosphere: Cologne first (it takes precedence), then the catalog look.
        if let cologne = view?.cologne, let look = AtmosphereCatalog.shared.look("cs2-major-cologne") {
            decor.atmosphere = look
            decor.cologne = cologne
        } else if let look = AtmosphereCatalog.shared.look(view?.atmosphere) {
            decor.atmosphere = look
        }
        let frosted = decor.cologne == nil && decor.atmosphere?.chatFrost == true
        if meta.fromBackfill { decor.alpha = CGFloat(style.backfillOpacity) / 100 }

        // Tints and the caption over the row.
        var label: (String, UIColor)?
        // A mention or reply to you always wears its color's edge; Flash lights the row as it
        // lands, and Keep tinted holds the wash behind it at the Highlight Appearance strength.
        if meta.isMentioned || meta.isReplyToMe {
            let color = ChatTheme.color(meta.isMentioned ? style.mentionColor : style.replyColor) ?? ChatTheme.error
            decor.leadingBar = (color, 4)
            if style.mentionHold { decor.fill = .tint(color, alpha: CGFloat(style.highlightAlpha)) }
            if style.mentionAnimation && row.live { decor.flash = meta.isMentioned ? .mention(color) : .reply(color) }
        } else if let highlight = meta.highlight, let color = ChatTheme.color(highlight.color) {
            // A rule's match always flashes; Highlight Appearance decides the bar (minimal and
            // standard draw it, none does not), and the rule's own Keep tinted the held wash.
            if style.highlightShowsBar { decor.leadingBar = (color, 4) }
            if highlight.hold { decor.fill = .tint(color, alpha: CGFloat(style.highlightAlpha)) }
            if row.live { decor.flash = .highlight(color) }
        } else if kind == .announcement {
            let color = Self.announcementColor(message.tags["msg-param-color"], mention: style.mentionColor)
            decor.leadingBar = (color, 4)
            decor.fill = .tint(color, alpha: 0.20)
            label = ("Announcement", color)
        } else if let builtIn = meta.builtIn, let color = ChatTheme.color(builtIn.color) {
            let firstTime = builtIn.kind == "first_time"
            let ring = firstTime && style.firstTimeStyle == "ring"
            if ring {
                // The ring stands in for the wash whatever the Highlight Appearance says.
                decor.ring = (color, style.firstTimeFill)
                // A straight divider would cut across the ring's rounded corners.
                decor.topHairline = nil
            } else {
                if style.highlightShowsBar { decor.leadingBar = (color, RowGlint.barWidth) }
                if style.highlightShowsTint { decor.fill = .tint(color, alpha: CGFloat(style.highlightAlpha)) }
            }
            if firstTime, row.live, let kind = RowGlint.Kind(rawValue: style.firstTimeAnimation) {
                decor.glint = RowGlint(kind: kind, onRing: ring, loop: style.firstTimeAnimateRepeat)
            }
            label = (builtIn.label, color)
        } else if let suspicious = meta.suspicious {
            let restricted = suspicious == "restricted"
            let color = restricted ? ChatTheme.error : ChatTheme.warning
            decor.leadingBar = (color, 4)
            decor.fill = .tint(color, alpha: 0.20)
            label = (restricted ? "Restricted user" : "Monitored user", color)
        } else if meta.isFromSharedChat {
            // A partner's row wears that channel's own color, so two partners read apart at chat
            // speed; the accent stands in until the core knows the channel.
            let color = view?.source.flatMap { ChatTheme.color($0.color) } ?? ChatTheme.accent
            decor.leadingBar = (color, 3)
            decor.fill = .tint(color, alpha: 0.10)
        }
        if let redemption = Self.redemptionLabel(msgID) {
            decor.fill = .gradient([ChatTheme.highlightCyan.withAlphaComponent(0.18), UIColor.systemBlue.withAlphaComponent(0.10)])
            label = (redemption, ChatTheme.highlightCyan)
        }
        if row.live == false, decor.flash != nil { decor.flash = nil }

        // Event cards: subscriptions, gifts, raids, watch streaks and cheers.
        if kind == .event || kind == .cheer {
            return buildEvent(row, kind: kind, msgID: msgID, decor: decor)
        }

        if let (caption, color) = label {
            var block = RowText(builder: self)
            block.append(caption, font(12, .regular), color.withAlphaComponent(0.6))
            place(block, maxLines: 1, alignRight: true)
        }
        if style.showTimestamps, let stamp = style.showTimestampSeconds ? meta.formattedTimestampWithSeconds : meta.formattedTimestamp {
            var block = RowText(builder: self)
            block.append(stamp, font(10, .regular), ChatTheme.textSecondary.withAlphaComponent(frosted ? 1 : 0.5))
            let from = (lines.count, attachments.count)
            place(block, maxLines: 1)
            if frosted, let box = Self.extent(lines[from.0...], attachments[from.1...]) {
                decor.frost.append(RowFrost(rect: box.insetBy(dx: -4, dy: 0), radius: 2, alpha: 0.8))
            }
            top += 2
        }

        // The reply's context line.
        if style.replyStyle == "full", let reply = meta.replyInfo {
            let small = (fontSize * 0.85).rounded()
            var block = RowText(builder: self)
            block.symbol("arrowshape.turn.up.left.fill", size: small * 1.1, color: ChatTheme.textSecondary)
            block.append(" ", font(small, .regular), ChatTheme.textSecondary)
            block.append(reply.shownName, font(small, .semibold), ChatTheme.textSecondary)
            block.append(" ", font(small, .regular), ChatTheme.textSecondary)
            let spans = view?.parentSpans ?? [.text(reply.parentMsgBody)]
            block.spans(spans, size: small, color: ChatTheme.textSecondary, emoteHeight: (small * 1.35).rounded(), weight: .regular)
            block.tap(.replyParent(reply.parentMsgID))
            let lineTop = top
            place(block, indent: 10, maxLines: 1)
            shapes.append(RowShape(kind: .bar(ChatTheme.textSecondary.withAlphaComponent(0.4)),
                                   rect: CGRect(x: sideInset, y: lineTop, width: 2, height: top - lineTop)))
            top += 6
        }

        // Badges, name and body flow as one paragraph. A Shared Chat partner's row leads with the
        // channel's face, round and ringed in its color (the cell draws the ring).
        var block = RowText(builder: self)
        if meta.isFromSharedChat, let face = view?.source?.avatarURL, !face.isEmpty {
            block.face(face, gapAfter: 5)
        }
        // Only badges whose image can load, so the gap before the name follows the last one shown.
        let badges = (view?.badges ?? []).filter { !ImagePipeline.shared.hasFailed($0.url) }
        for (i, badge) in badges.enumerated() {
            block.badge(badge, gapAfter: i == badges.count - 1 ? 6 : 4)
        }
        let action = meta.isAction
        let nameColor = ChatTheme.color(view?.nameColor ?? message.color) ?? ChatTheme.fallbackName
        let accent = style.usernameAccentSource == "theme" ? ChatTheme.accent : nameColor
        let nameFont = font(fontSize, .bold)
        // The viewer's own nickname for the author stands in for their name (Rust sets it).
        let name = (style.showAtSign ? "@" : "") + (view?.nickname ?? message.shownName)
        switch style.usernameStyle {
        case "bar": block.bar(accent)
        case "dot": block.dot(accent)
        case "brackets": block.append("[", nameFont, accent)
        default: break
        }
        block.name(name, font: nameFont, color: nameColor, paint: view?.paint, chip: style.usernameStyle == "chip" ? accent : nil)
        if style.usernameStyle == "brackets" { block.append("]", nameFont, accent) }
        if !action, let separator = Self.separator(style.usernameSeparator) {
            if separator == ":" {
                block.append(":", nameFont, accent)
                block.append(" ", font(fontSize, bodyWeight), ChatTheme.textPrimary)
            } else {
                block.append(" \(separator) ", nameFont, accent)
            }
        } else {
            block.append(" ", font(fontSize, bodyWeight), ChatTheme.textPrimary)
        }
        if style.replyStyle == "mention", let reply = meta.replyInfo {
            block.append("@\(reply.shownName) ", font(fontSize, .semibold), ChatTheme.textPrimary.withAlphaComponent(0.85))
        }

        var spans = view?.spans ?? message.segments.map(Self.span)
        // A link with its card shown is drawn by the card alone, unless the viewer keeps both.
        let cards = (view?.previews ?? []).filter { $0.preview != nil || $0.reserve }
        let carded = Set(cards.filter { !$0.keepLink }.map(\.url))
        if !carded.isEmpty {
            spans.removeAll { span in
                if case .link(_, let url) = span { return carded.contains(Self.absolute(url)) }
                return false
            }
        }
        var giant: ChatViewEmote?
        if msgID == "gigantified-emote-message", style.giantEmotes, style.giantEmoteAlign != "inline",
           let last = spans.lastIndex(where: { if case .emote = $0 { return true } else { return false } }),
           case .emote(let emote, _, _) = spans[last] {
            giant = emote
            spans.remove(at: last)
        }
        let deleted = row.moderation
        let italic = action || (deleted != nil && style.deletedMessageStyle == "italic")
        let bodyColor: UIColor = action ? nameColor
            : (deleted != nil && style.deletedMessageStyle == "italic" ? ChatTheme.textSecondary : ChatTheme.textPrimary)
        block.spans(spans, size: fontSize, color: bodyColor, emoteHeight: emoteHeight, weight: bodyWeight, italic: italic,
                    paint: action ? view?.paint : nil, strike: deleted != nil && style.deletedMessageStyle == "strikethrough")
        if let deleted {
            switch style.deletedMessageStyle {
            case "strikethrough":
                block.append(" " + Self.moderationTag(deleted), font(12, .regular), ChatTheme.error.withAlphaComponent(0.7))
                decor.alpha *= 0.5
            case "dimmed":
                decor.alpha *= 0.5
            case "italic":
                block.append(" ", font(fontSize, bodyWeight), bodyColor)
                block.pill(deleted)
            default: break
            }
        }
        let blockStart = (lines.count, attachments.count)
        place(block)
        if frosted, let box = Self.extent(lines[blockStart.0...], attachments[blockStart.1...]) {
            decor.frost.append(RowFrost(rect: box.insetBy(dx: -6, dy: -2), radius: 6, alpha: 1))
        }

        if let giant {
            let h = (8 * fontSize * CGFloat(style.emoteScale)).rounded()
            let w = min(h * CGFloat(giant.aspect ?? 1), 24 * fontSize * CGFloat(style.emoteScale), width - 2 * sideInset)
            let x: CGFloat
            switch style.giantEmoteAlign {
            case "left": x = sideInset
            case "right": x = width - sideInset - w
            default: x = (width - w) / 2
            }
            top += 4
            attachments.append(RowAttachment(kind: .giant, url: ImageTier.url(Self.giantURL(giant.url), pixelHeight: Double(h * scale)),
                                             pixelHeight: Int(h * scale), rect: CGRect(x: x, y: top, width: w, height: h),
                                             overlays: [], effects: 0, animate: style.animatesEmotes))
            top += h
        }
        var cardWords: [String] = []
        for card in cards {
            top += 6
            let built = previewCard(card, top: top)
            lines += built.lines
            attachments += built.attachments
            shapes += built.shapes
            taps += built.taps
            top = built.bottom
            if let words = built.words { cardWords.append(words) }
        }
        top += padding
        let accessibility = (["\(message.shownName): \(spans.map(\.text).joined())"] + cardWords).joined(separator: ". ")
        let built = ChatRowLayout(id: row.id, revision: row.revision, width: width, height: ceil(top), lines: lines,
                                  attachments: attachments, shapes: shapes, taps: taps, decor: decor,
                                  accessibilityText: accessibility, live: row.live,
                                  mentionsMe: meta.isMentioned)
        built.paintShadows = PaintShadows(rawValue: style.paintShadows) ?? .all
        return built
    }

    /// A held message: a warm card with why it is held, the author and text, and Allow and Deny
    /// side by side under it. Once decided it folds to one quiet line saying what happened.
    func buildHeld(_ row: ChatRowModel, _ held: HeldRow) -> ChatRowLayout {
        var lines: [PlacedLine] = []
        var attachments: [RowAttachment] = []
        var shapes: [RowShape] = []
        var taps: [RowTap] = []
        var decor = RowDecor()
        var top: CGFloat = 0
        func place(_ block: RowText, indent: CGFloat = 0, right: CGFloat = 0, maxLines: Int = 0) {
            let placed = block.layout(top: top, left: sideInset + indent, width: width - 2 * sideInset - indent - right,
                                      maxLines: maxLines, alignRight: false)
            lines += placed.lines
            attachments += placed.attachments
            taps += placed.taps
            shapes += placed.shapes
            top = placed.bottom
        }
        let icon: CGFloat = 13

        if let line = held.line {
            top = 5
            var block = RowText(builder: self)
            block.append(line, font(12, .regular), ChatTheme.textSecondary)
            if let by = held.by {
                block.append(" \u{00B7} \(by)", font(12, .regular), ChatTheme.textMuted)
            }
            shapes.append(RowShape(kind: .symbol("flag.fill", ChatTheme.textMuted),
                                   rect: CGRect(x: sideInset, y: top + 2, width: icon, height: icon)))
            place(block, indent: icon + 6, maxLines: 2)
            top += 5
            let words = [line, held.by.map { "by \($0)" }].compactMap { $0 }.joined(separator: ", ")
            return ChatRowLayout(id: row.id, revision: row.revision, width: width, height: ceil(top), lines: lines,
                                 attachments: attachments, shapes: shapes, taps: taps, decor: decor,
                                 accessibilityText: words, live: row.live, mentionsMe: false)
        }

        let amber = ChatTheme.warning
        let inset: CGFloat = 8
        top = inset + 8
        let cardTop = inset
        // Why it is held.
        var caption = RowText(builder: self)
        caption.append(held.held.label, font(12, .semibold), amber)
        shapes.append(RowShape(kind: .symbol("flag.fill", amber), rect: CGRect(x: sideInset + inset, y: top + 2, width: icon, height: icon)))
        place(caption, indent: inset + icon + 6, right: inset, maxLines: 2)
        top += 4
        // The author and the message, as a row draws them.
        var block = RowText(builder: self)
        let nameColor = ChatTheme.color(row.message.metadata.view?.nameColor ?? row.message.color) ?? ChatTheme.fallbackName
        block.name(row.message.metadata.view?.nickname ?? held.held.shownName, font: font(fontSize, .bold), color: nameColor,
                   paint: nil, chip: nil)
        block.append(": ", font(fontSize, .bold), nameColor)
        block.append(held.held.text, font(fontSize, bodyWeight), ChatTheme.textPrimary)
        place(block, indent: inset, right: inset)
        top += 10
        // Allow and Deny: words, not icons, and big enough to hit in a moving chat.
        let gap: CGFloat = 8
        let buttonHeight: CGFloat = 34
        let buttonWidth = (width - 2 * sideInset - 2 * inset - gap) / 2
        for (i, allow) in [true, false].enumerated() {
            let rect = CGRect(x: sideInset + inset + CGFloat(i) * (buttonWidth + gap), y: top, width: buttonWidth, height: buttonHeight)
            let tint = allow ? ChatTheme.success : ChatTheme.error
            shapes.append(RowShape(kind: .capsule(fill: tint.withAlphaComponent(0.16), stroke: nil), rect: rect))
            let title = allow ? "Allow" : "Deny"
            // The renderer draws `text:` shapes medium weight at height / 1.6.
            let size = (title as NSString).size(withAttributes: [.font: font(15, .medium)])
            shapes.append(RowShape(kind: .symbol("text:\(title)", tint),
                                   rect: CGRect(x: rect.midX - size.width / 2, y: rect.minY, width: size.width, height: 24)
                                       .offsetBy(dx: 0, dy: (buttonHeight - 24) / 2)))
            taps.append(RowTap(rect: rect, action: .held(held.held.messageID, allow: allow)))
        }
        top += buttonHeight + 10
        let card = CGRect(x: sideInset - 2, y: cardTop, width: width - 2 * sideInset + 4, height: top - cardTop)
        shapes.insert(RowShape(kind: .capsule(fill: amber.withAlphaComponent(0.10), stroke: amber.withAlphaComponent(0.25)), rect: card), at: 0)
        top += inset
        decor.flash = row.live ? RowFlash(color: amber, alphas: [0.25, 0]) : nil
        let words = "\(held.held.label). \(held.held.shownName): \(held.held.text)"
        return ChatRowLayout(id: row.id, revision: row.revision, width: width, height: ceil(top), lines: lines,
                             attachments: attachments, shapes: shapes, taps: taps, decor: decor,
                             accessibilityText: words, live: row.live, mentionsMe: false)
    }

    /// The box the given lines and images cover, for a frost block hugging them.
    static func extent(_ lines: ArraySlice<PlacedLine>, _ attachments: ArraySlice<RowAttachment>) -> CGRect? {
        var box: CGRect?
        for placed in lines {
            var ascent: CGFloat = 0
            var descent: CGFloat = 0
            let width = CGFloat(CTLineGetTypographicBounds(placed.line, &ascent, &descent, nil))
            let rect = CGRect(x: placed.origin.x, y: placed.origin.y - ascent, width: width, height: ascent + descent)
            box = box.map { $0.union(rect) } ?? rect
        }
        for attachment in attachments {
            box = box.map { $0.union(attachment.rect) } ?? attachment.rect
        }
        return box
    }

    // MARK: Event cards

    private func buildEvent(_ row: ChatRowModel, kind: RowKind, msgID: String, decor base: RowDecor) -> ChatRowLayout {
        let message = row.message
        let meta = message.metadata
        let view = meta.view
        var decor = base
        decor.leadingBar = nil
        decor.flash = nil
        decor.ring = nil
        decor.glint = nil
        decor.topHairline = ChatTheme.borderSubtle
        var top = CGFloat(style.rowPadding) + 2
        var lines: [PlacedLine] = []
        var attachments: [RowAttachment] = []
        var shapes: [RowShape] = []
        var taps: [RowTap] = []
        // The desktop card's lead: 20 points, the cheer's gem 28; 10 points before the text.
        let icon: CGFloat = kind == .cheer ? 28 : 20
        let indent = icon + 10
        let tags = message.tags

        // What the row leads with, as the desktop card draws it: a glyph or symbol, or art.
        let mark: RowShape.Kind
        var art: (kind: RowAttachment.Kind, url: String, animate: Bool)?
        var header = RowText(builder: self)
        let nameColor = ChatTheme.color(view?.nameColor ?? message.color) ?? ChatTheme.fallbackName
        let small = font(fontSize, .regular)
        let bold = font(fontSize, .bold)
        switch kind == .cheer ? "cheer" : msgID {
        case "cheer":
            let bits = meta.bitsAmount ?? Int(tags["bits"] ?? "") ?? 0
            let (tier, color) = Self.cheerTier(bits)
            art = (.cheer, "https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/\(tier)/2.gif", style.animatesEmotes)
            mark = .symbol("diamond.fill", color)
            decor.fill = .gradient([UIColor.systemPurple, UIColor.systemBlue, UIColor.systemCyan, UIColor.systemPink].map { $0.withAlphaComponent(0.15) })
            for badge in view?.badges ?? [] { header.badge(badge, gapAfter: 4) }
            header.name(message.shownName, font: bold, color: nameColor, paint: view?.paint, chip: nil)
            header.append(" cheered \(bits.formatted()) bits", bold, color)
        case "raid":
            mark = .glyph(.raid, ChatTheme.highlightRed)
            decor.fill = .gradient([UIColor.systemRed, UIColor.systemOrange].map { $0.withAlphaComponent(0.15) })
            for badge in view?.badges ?? [] { header.badge(badge, gapAfter: 4) }
            header.name(message.shownName, font: bold, color: nameColor, paint: view?.paint, chip: nil)
            let viewers = Int(tags["msg-param-viewerCount"] ?? "")
            header.append(viewers.map { " is raiding with \($0.formatted()) viewer\($0 == 1 ? "" : "s")!" } ?? " is raiding!",
                          bold, ChatTheme.highlightRed)
        case "viewermilestone":
            mark = .glyph(.streak, ChatTheme.highlightOrange)
            decor.fill = .gradient([UIColor.systemOrange, UIColor.systemYellow].map { $0.withAlphaComponent(0.12) })
            for badge in view?.badges ?? [] { header.badge(badge, gapAfter: 4) }
            header.name(message.shownName, font: bold, color: nameColor, paint: view?.paint, chip: nil)
            if let reward = tags["msg-param-copoReward"], !reward.isEmpty {
                header.append("  +\(reward)", bold, ChatTheme.highlightOrange)
            }
        default:
            // Twitch's Prime crown for a Prime sub; Twitch's gift art for every other sub and gift
            // (the drawn gift until that art is known).
            if tags["msg-param-sub-plan"] == "Prime" {
                mark = .glyph(.prime, ChatTheme.info)
            } else {
                mark = .symbol("gift.fill", ChatTheme.accent)
                if let gift = view?.eventArt { art = (.badge, gift, false) }
            }
            decor.fill = .gradient([UIColor.systemPink, UIColor.systemPurple, UIColor.systemBlue, UIColor.systemCyan, UIColor.systemYellow]
                .map { $0.withAlphaComponent(0.12) })
            let system = meta.systemMessage.map { $0.replacingOccurrences(of: "\\s", with: " ") }
                ?? Self.subFallback(msgID, message: message)
            header.append(system, font(fontSize, .medium), ChatTheme.textPrimary)
            _ = small
        }
        let blockTop = top
        func place(_ block: RowText) {
            let placed = block.layout(top: top, left: sideInset + indent, width: width - 2 * sideInset - indent,
                                      maxLines: 0, alignRight: false)
            lines += placed.lines
            attachments += placed.attachments
            taps += placed.taps
            shapes += placed.shapes
            top = placed.bottom
        }
        place(header)
        if msgID == "viewermilestone" {
            var second = RowText(builder: self)
            let streak = tags["msg-param-value"] ?? ""
            second.append("Watched \(streak) consecutive streams and sparked a watch streak!", small, ChatTheme.textSecondary)
            place(second)
        }
        let spans = view?.spans ?? message.segments.map(Self.span)
        if !spans.isEmpty, !spans.allSatisfy({ $0.text.trimmingCharacters(in: .whitespaces).isEmpty }) {
            top += 2
            var body = RowText(builder: self)
            body.spans(spans, size: fontSize, color: kind == .cheer ? ChatTheme.textSecondary : ChatTheme.textPrimary,
                       emoteHeight: emoteHeight, weight: bodyWeight)
            place(body)
        }
        // The lead sits centered on the whole text block, however many lines it takes, as the
        // desktop card's `items-center` puts it.
        let lead = CGRect(x: sideInset, y: (blockTop + max(0, (top - blockTop - icon) / 2)).rounded(), width: icon, height: icon)
        if let art {
            attachments.append(RowAttachment(kind: art.kind, url: art.url, pixelHeight: Int(icon * scale), rect: lead,
                                             overlays: [], effects: 0, animate: art.animate))
        } else {
            shapes.append(RowShape(kind: mark, rect: lead))
        }
        top += CGFloat(style.rowPadding) + 2
        let accessibility = (meta.systemMessage ?? message.shownName) + " " + message.content
        return ChatRowLayout(id: row.id, revision: row.revision, width: width, height: ceil(top), lines: lines,
                             attachments: attachments, shapes: shapes, taps: taps, decor: decor,
                             accessibilityText: accessibility, live: row.live, mentionsMe: false)
    }

    // MARK: Row kinds and words

    enum RowKind { case plain, system, event, cheer, announcement }

    static func kind(of message: ChatMessage, msgID: String) -> RowKind {
        if message.isSystem { return .system }
        switch message.metadata.msgType ?? "" {
        case "announcement": return .announcement
        case "sub", "resub", "subgift", "submysterygift", "anonsubgift", "anonsubmysterygift", "giftpaidupgrade",
             "primepaidupgrade", "anongiftpaidupgrade", "communitypayforward", "standardpayforward", "raid":
            return .event
        case "viewermilestone":
            return message.tags["msg-param-category"] == "watch-streak" ? .event : .plain
        default: break
        }
        if (message.metadata.bitsAmount ?? 0) > 0 || message.tags["bits"] != nil { return .cheer }
        return .plain
    }

    static func redemptionLabel(_ msgID: String) -> String? {
        switch msgID {
        case "highlighted-message": return "Redeemed Highlight My Message"
        case "gigantified-emote-message": return "Sent a Gigantified Emote"
        case "animated-message": return "Sent an Animated Message"
        case "skip-subs-mode-message": return "Sent a Message in Sub-Only Mode"
        default: return nil
        }
    }

    static func announcementColor(_ param: String?, mention: String) -> UIColor {
        switch param?.uppercased() {
        case "BLUE": return ChatTheme.color("#3b82f6")!
        case "GREEN": return ChatTheme.color("#22c55e")!
        case "ORANGE": return ChatTheme.color("#f97316")!
        case "PURPLE": return ChatTheme.color("#a855f7")!
        default: return ChatTheme.color(mention) ?? ChatTheme.color("#9147ff")!
        }
    }

    static func cheerTier(_ bits: Int) -> (String, UIColor) {
        switch bits {
        case 10000...: return ("10000", ChatTheme.color("#ff1f1f")!)
        case 5000...: return ("5000", ChatTheme.color("#0099fe")!)
        case 1000...: return ("1000", ChatTheme.color("#1db2a6")!)
        case 100...: return ("100", ChatTheme.color("#9c3ee8")!)
        default: return ("1", ChatTheme.color("#979797")!)
        }
    }

    static func subFallback(_ msgID: String, message: ChatMessage) -> String {
        let user = message.shownName
        let tags = message.tags
        let recipient = tags["msg-param-recipient-display-name"] ?? tags["msg-param-recipient-user-name"] ?? ""
        let count = tags["msg-param-mass-gift-count"] ?? ""
        switch msgID {
        case "resub": return "\(user) subscribed for \(tags["msg-param-cumulative-months"] ?? "") months!"
        case "subgift": return "\(user) gifted a subscription to \(recipient)!"
        case "submysterygift": return "\(user) is gifting \(count) subscriptions to the community!"
        case "anonsubmysterygift": return "An anonymous user is gifting \(count) subscriptions to the community!"
        case "anonsubgift": return "An anonymous user gifted a subscription to \(recipient)!"
        default: return "\(user) subscribed!"
        }
    }

    static func separator(_ value: String) -> String? {
        switch value {
        case "colon": return ":"
        case "dot": return "·"
        case "arrow": return "›"
        case "pipe": return "|"
        case "dash": return "\u{2013}"
        default: return nil
        }
    }

    static func moderationTag(_ mark: ChatModeration) -> String {
        switch mark {
        case .deleted: return "[deleted by mod]"
        case .timedOut(let s): return s > 0 ? "[timed out for \(duration(s))]" : "[timed out]"
        case .banned: return "[banned]"
        }
    }

    /// "45s", "10m", "1h 30m", "2d 3h".
    static func duration(_ seconds: Int) -> String {
        let d = seconds / 86400, h = (seconds % 86400) / 3600, m = (seconds % 3600) / 60, s = seconds % 60
        if d > 0 { return h > 0 ? "\(d)d \(h)h" : "\(d)d" }
        if h > 0 { return m > 0 ? "\(h)h \(m)m" : "\(h)h" }
        if m > 0 { return "\(m)m" }
        return "\(s)s"
    }

    /// A row the core sent without a view (an old core): its segments as spans.
    static func span(_ segment: ChatSegment) -> ChatSpan {
        switch segment {
        case .text(let t), .unknown(let t): return .text(t)
        case .emote(let e):
            return .emote(ChatViewEmote(code: e.code, url: e.url, width: e.width, height: e.height, personal: e.isPersonal),
                          overlays: [], effects: 0)
        case .emoji(let content, let url): return .emoji(text: content, url: url)
        case .link(let content, let url): return .link(label: content, url: url)
        case .cheermote(let c): return .cheer(text: c.content, bits: c.bits, color: c.color, url: c.url)
        case .gif(let content, _, let url): return .gif(text: content, url: url)
        }
    }

    /// The 4x file of an emote for the gigantified block.
    static func giantURL(_ url: String) -> String { url }

    /// A link as the core keys its card: a bare `www.` link gains its scheme.
    static func absolute(_ url: String) -> String {
        url.hasPrefix("http://") || url.hasPrefix("https://") ? url : "https://\(url)"
    }

    // MARK: Link cards

    struct BuiltCard {
        var lines: [PlacedLine] = []
        var attachments: [RowAttachment] = []
        var shapes: [RowShape] = []
        var taps: [RowTap] = []
        var bottom: CGFloat
        var words: String?
    }

    /// Every card is this tall, read or still pending, so a held card never changes the row's height.
    static let cardHeight: CGFloat = 66
    static let discordBlurple = ChatTheme.color("#5865F2")!
    static let discordGreen = ChatTheme.color("#23A55A")!

    /// One link card under the body, as desktop draws it: a Discord invite as its server (icon,
    /// name, live online and member counts, a Join pill), anything else as its picture beside its
    /// site, title and byline. A card not read yet holds the same room with faint bars.
    func previewCard(_ card: ChatRowPreview, top: CGFloat) -> BuiltCard {
        let pad: CGFloat = 10
        let x = sideInset
        let cardWidth = min(width - 2 * sideInset, 380)
        let rect = CGRect(x: x, y: top, width: cardWidth, height: Self.cardHeight)
        var built = BuiltCard(bottom: rect.maxY)
        built.shapes.append(RowShape(kind: .card(radius: 12), rect: rect))
        if let url = URL(string: card.url) { built.taps.append(RowTap(rect: rect, action: .link(url))) }
        let picture: CGFloat = Self.cardHeight - 2 * pad
        let host = URL(string: card.url)?.host?.replacingOccurrences(of: "www.", with: "") ?? card.url

        guard let preview = card.preview else {
            // Pending: the room held, the host named, two faint bars for what comes.
            let discord = host.contains("discord")
            built.shapes.append(RowShape(kind: .capsule(fill: ChatTheme.textMuted.withAlphaComponent(0.12), stroke: nil),
                                         rect: CGRect(x: x + pad, y: top + pad, width: picture, height: picture)))
            let left = x + pad + picture + 12
            let textWidth = cardWidth - (left - x) - pad
            built.lines.append(cardLine(discord ? "Discord" : host, font(11, .semibold),
                                        discord ? Self.discordBlurple : ChatTheme.textSecondary,
                                        x: left, baseline: top + pad + 10, width: textWidth))
            for (i, share) in [0.62, 0.38].enumerated() {
                built.shapes.append(RowShape(kind: .bar(ChatTheme.textMuted.withAlphaComponent(0.16)),
                                             rect: CGRect(x: left, y: top + pad + 19 + CGFloat(i) * 14, width: textWidth * share, height: 8)))
            }
            return built
        }

        if preview.kind == "discord" {
            let icon = CGRect(x: x + pad, y: top + pad, width: picture, height: picture)
            if let image = preview.image, !ImagePipeline.shared.hasFailed(image) {
                built.shapes.append(RowShape(kind: .capsule(fill: UIColor.white.withAlphaComponent(0.04), stroke: nil), rect: icon))
                built.attachments.append(RowAttachment(kind: .preview, url: image, pixelHeight: Int(picture * scale), rect: icon,
                                                       overlays: [], effects: 0, animate: style.animatesEmotes, radius: 14))
            } else {
                built.shapes.append(RowShape(kind: .capsule(fill: Self.discordBlurple, stroke: nil), rect: icon))
                let glyph = picture * 0.5
                built.shapes.append(RowShape(kind: .symbol("person.2.fill", .white),
                                             rect: CGRect(x: icon.midX - glyph / 2, y: icon.midY - glyph / 2, width: glyph, height: glyph)))
            }
            // The Join pill: what the tap does, at the card's right.
            let joinFont = font(12, .semibold)
            let joinWidth = ("Join" as NSString).size(withAttributes: [.font: joinFont]).width + 24
            let join = CGRect(x: rect.maxX - pad - joinWidth, y: rect.midY - 15, width: joinWidth, height: 30)
            built.shapes.append(RowShape(kind: .capsule(fill: Self.discordBlurple, stroke: nil), rect: join))
            built.lines.append(cardLine("Join", joinFont, .white, x: join.minX + 12, baseline: join.midY + 4.5, width: joinWidth))
            let left = icon.maxX + 12
            let textWidth = join.minX - 10 - left
            built.lines.append(cardLine("Discord", font(11, .semibold), Self.discordBlurple, x: left, baseline: top + pad + 10, width: textWidth))
            built.lines.append(cardLine(preview.title ?? host, font(14, .semibold), ChatTheme.textPrimary,
                                        x: left, baseline: top + pad + 27, width: textWidth))
            let counts = NSMutableAttributedString()
            let small = font(11, .regular)
            let strong = font(11, .semibold)
            func count(_ n: Int, _ label: String, dot: UIColor) {
                if counts.length > 0 { counts.append(NSAttributedString(string: "   ", attributes: [.font: small])) }
                counts.append(NSAttributedString(string: "\u{25CF} ", attributes: [.font: font(8, .regular), .foregroundColor: dot]))
                counts.append(NSAttributedString(string: n.formatted(), attributes: [.font: strong, .foregroundColor: ChatTheme.textPrimary]))
                counts.append(NSAttributedString(string: " \(label)", attributes: [.font: small, .foregroundColor: ChatTheme.textSecondary]))
            }
            if let online = preview.onlineCount { count(online, "Online", dot: Self.discordGreen) }
            if let members = preview.memberCount { count(members, "Members", dot: ChatTheme.textMuted.withAlphaComponent(0.6)) }
            if counts.length > 0 {
                built.lines.append(cardLine(counts, x: left, baseline: top + pad + 42, width: textWidth))
            }
            let tally = [preview.onlineCount.map { "\($0.formatted()) online" }, preview.memberCount.map { "\($0.formatted()) members" }]
                .compactMap { $0 }.joined(separator: ", ")
            built.words = "Discord server \(preview.title ?? host)" + (tally.isEmpty ? "" : ", \(tally)")
            return built
        }

        // Everything else: the picture (16:9, or square for a face) beside three lines.
        var left = x + pad
        if let image = preview.image ?? preview.authorAvatar, !ImagePipeline.shared.hasFailed(image) {
            let square = preview.image == nil || ["tweet", "instagram", "spotify", "youtube_channel"].contains(preview.kind)
            let w = square ? picture : (picture * 16 / 9).rounded()
            let box = CGRect(x: left, y: top + pad, width: w, height: picture)
            built.shapes.append(RowShape(kind: .capsule(fill: UIColor.white.withAlphaComponent(0.04), stroke: nil), rect: box))
            built.attachments.append(RowAttachment(kind: .preview, url: image, pixelHeight: Int(picture * scale), rect: box,
                                                   overlays: [], effects: 0, animate: style.animatesEmotes, radius: 8))
            left = box.maxX + 12
        }
        let textWidth = rect.maxX - pad - left
        let site = preview.siteName ?? host
        let title = preview.title ?? preview.description ?? host
        var byline = preview.author
        if let views = preview.viewCount { byline = [byline, "\(views.formatted()) views"].compactMap { $0 }.joined(separator: " \u{00B7} ") }
        if byline == nil, preview.title != nil { byline = preview.description }
        built.lines.append(cardLine(site, font(11, .semibold), ChatTheme.textSecondary, x: left, baseline: top + pad + 10, width: textWidth))
        built.lines.append(cardLine(title, font(14, .semibold), ChatTheme.textPrimary, x: left, baseline: top + pad + 27, width: textWidth))
        if let byline, !byline.isEmpty {
            built.lines.append(cardLine(byline, font(12, .regular), ChatTheme.textSecondary, x: left, baseline: top + pad + 42, width: textWidth))
        }
        built.words = "\(site): \(title)"
        return built
    }

    /// One line of card text on `baseline`, cut with an ellipsis to `width`.
    func cardLine(_ text: String, _ font: UIFont, _ color: UIColor, x: CGFloat, baseline: CGFloat, width: CGFloat) -> PlacedLine {
        let flat = text.replacingOccurrences(of: "\n", with: " ")
        return cardLine(NSAttributedString(string: flat, attributes: [.font: font, .foregroundColor: color]),
                        x: x, baseline: baseline, width: width)
    }

    func cardLine(_ text: NSAttributedString, x: CGFloat, baseline: CGFloat, width: CGFloat) -> PlacedLine {
        var line = CTLineCreateWithAttributedString(text)
        if CTLineGetTypographicBounds(line, nil, nil, nil) > Double(width) {
            let font = text.length > 0 ? text.attribute(.font, at: 0, effectiveRange: nil) as? UIFont : nil
            let ellipsis = CTLineCreateWithAttributedString(NSAttributedString(string: "\u{2026}", attributes: [
                .font: font ?? self.font(12, .regular), .foregroundColor: ChatTheme.textSecondary,
            ]))
            if let cut = CTLineCreateTruncatedLine(line, Double(max(width, 1)), .end, ellipsis) { line = cut }
        }
        return PlacedLine(line: line, origin: CGPoint(x: x, y: baseline))
    }
}
