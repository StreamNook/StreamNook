import CoreText
import SNKit
import UIKit

/// An image inside a row: an emote with what stacks over it, a badge, a cheer gem, a GIF or a
/// gigantified emote. Its rect is fixed before the image loads, so a row never reflows.
struct RowAttachment {
    enum Kind { case emote, badge, cheer, gif, giant, avatar }
    var kind: Kind
    var url: String
    /// The pixel height to decode at.
    var pixelHeight: Int
    var rect: CGRect
    var overlays: [String]
    var effects: UInt32
    var animate: Bool
}

/// Where a tap on a row goes.
enum RowTapAction: Equatable {
    case name
    case mention(String)
    case link(URL)
    case replyParent(String)
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
    /// The live flash: from `alpha` of `color` to clear over two seconds.
    var flash: (color: UIColor, alpha: CGFloat)?
    var alpha: CGFloat = 1
    /// The author's StreamNook Atmosphere (or Cologne look), drawn behind the row by the cell.
    var atmosphere: AtmosphereLook?
    var cologne: CologneLook?
    /// Frosted blocks behind the text when the Atmosphere asks for them (drawn into the bitmap).
    var frost: [RowFrost] = []
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
        let base = UIFont.systemFont(ofSize: size, weight: weight)
        guard italic, let descriptor = base.fontDescriptor.withSymbolicTraits(.traitItalic) else { return base }
        return UIFont(descriptor: descriptor, size: size)
    }

    var bodyWeight: UIFont.Weight { .light }
    var baseWeight: UIFont.Weight {
        switch style.fontWeight {
        case ..<350: return .light
        case ..<450: return .regular
        case ..<550: return .medium
        case ..<650: return .semibold
        default: return .bold
        }
    }

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
        if meta.isMentioned {
            decor.leadingBar = (ChatTheme.color(style.mentionColor) ?? ChatTheme.error, 4)
            if style.mentionAnimation && row.live { decor.flash = (ChatTheme.color(style.mentionColor) ?? ChatTheme.error, 0.5) }
        } else if meta.isReplyToMe {
            decor.leadingBar = (ChatTheme.color(style.replyColor) ?? ChatTheme.error, 4)
            if style.mentionAnimation && row.live { decor.flash = (ChatTheme.color(style.replyColor) ?? ChatTheme.error, 0.3) }
        } else if let highlight = meta.highlight, let color = ChatTheme.color(highlight.color) {
            decor.leadingBar = (color, 4)
            decor.fill = .tint(color, alpha: 0.20)
            if row.live { decor.flash = (color, 0.35) }
        } else if kind == .announcement {
            let color = Self.announcementColor(message.tags["msg-param-color"], mention: style.mentionColor)
            decor.leadingBar = (color, 4)
            decor.fill = .tint(color, alpha: 0.20)
            label = ("Announcement", color)
        } else if let builtIn = meta.builtIn, let color = ChatTheme.color(builtIn.color) {
            decor.leadingBar = (color, 4)
            decor.fill = .tint(color, alpha: 0.20)
            label = (builtIn.label, color)
        } else if let suspicious = meta.suspicious {
            let restricted = suspicious == "restricted"
            let color = restricted ? ChatTheme.error : ChatTheme.warning
            decor.leadingBar = (color, 4)
            decor.fill = .tint(color, alpha: 0.20)
            label = (restricted ? "Restricted user" : "Monitored user", color)
        } else if meta.isFromSharedChat {
            decor.leadingBar = (ChatTheme.accent.withAlphaComponent(0.5), 2)
            decor.fill = .solid(ChatTheme.accent.withAlphaComponent(0.05))
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

        // Badges, name and body flow as one paragraph.
        var block = RowText(builder: self)
        let badges = view?.badges ?? []
        for (i, badge) in badges.enumerated() {
            block.badge(badge, gapAfter: i == badges.count - 1 ? 6 : 4)
        }
        let action = meta.isAction
        let nameColor = ChatTheme.color(view?.nameColor ?? message.color) ?? ChatTheme.fallbackName
        let accent = style.usernameAccentSource == "theme" ? ChatTheme.accent : nameColor
        let nameFont = font(fontSize, .bold)
        let name = (style.showAtSign ? "@" : "") + message.shownName
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
        top += padding
        let accessibility = "\(message.shownName): \(spans.map(\.text).joined())"
        let built = ChatRowLayout(id: row.id, revision: row.revision, width: width, height: ceil(top), lines: lines,
                                  attachments: attachments, shapes: shapes, taps: taps, decor: decor,
                                  accessibilityText: accessibility, live: row.live,
                                  mentionsMe: meta.isMentioned)
        built.paintShadows = PaintShadows(rawValue: style.paintShadows) ?? .all
        return built
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
        decor.topHairline = ChatTheme.borderSubtle
        var top = CGFloat(style.rowPadding) + 2
        var lines: [PlacedLine] = []
        var attachments: [RowAttachment] = []
        var shapes: [RowShape] = []
        var taps: [RowTap] = []
        let icon: CGFloat = 20
        let indent = icon + 10
        let tags = message.tags

        let symbol: (String, UIColor)
        var gem: String?
        var header = RowText(builder: self)
        let nameColor = ChatTheme.color(view?.nameColor ?? message.color) ?? ChatTheme.fallbackName
        let small = font(fontSize, .regular)
        let bold = font(fontSize, .bold)
        switch kind == .cheer ? "cheer" : msgID {
        case "cheer":
            let bits = meta.bitsAmount ?? Int(tags["bits"] ?? "") ?? 0
            let (tier, color) = Self.cheerTier(bits)
            gem = "https://d3aqoihi2n8ty8.cloudfront.net/actions/cheer/dark/animated/\(tier)/2.gif"
            symbol = ("diamond.fill", color)
            decor.fill = .gradient([UIColor.systemPurple, UIColor.systemBlue, UIColor.systemCyan, UIColor.systemPink].map { $0.withAlphaComponent(0.15) })
            for badge in view?.badges ?? [] { header.badge(badge, gapAfter: 4) }
            header.name(message.shownName, font: bold, color: nameColor, paint: view?.paint, chip: nil)
            header.append(" cheered \(bits.formatted()) bits", bold, color)
        case "raid":
            symbol = ("person.3.fill", ChatTheme.highlightRed)
            decor.fill = .gradient([UIColor.systemRed, UIColor.systemOrange].map { $0.withAlphaComponent(0.15) })
            for badge in view?.badges ?? [] { header.badge(badge, gapAfter: 4) }
            header.name(message.shownName, font: bold, color: nameColor, paint: view?.paint, chip: nil)
            let viewers = Int(tags["msg-param-viewerCount"] ?? "")
            header.append(viewers.map { " is raiding with \($0.formatted()) viewer\($0 == 1 ? "" : "s")!" } ?? " is raiding!",
                          bold, ChatTheme.highlightRed)
        case "viewermilestone":
            symbol = ("flame.fill", ChatTheme.highlightOrange)
            decor.fill = .gradient([UIColor.systemOrange, UIColor.systemYellow].map { $0.withAlphaComponent(0.12) })
            for badge in view?.badges ?? [] { header.badge(badge, gapAfter: 4) }
            header.name(message.shownName, font: bold, color: nameColor, paint: view?.paint, chip: nil)
            if let reward = tags["msg-param-copoReward"], !reward.isEmpty {
                header.append("  +\(reward)", bold, ChatTheme.highlightOrange)
            }
        default:
            let prime = tags["msg-param-sub-plan"] == "Prime"
            symbol = (prime ? "crown.fill" : "gift.fill", prime ? UIColor.systemTeal : ChatTheme.accent)
            decor.fill = .gradient([UIColor.systemPink, UIColor.systemPurple, UIColor.systemBlue, UIColor.systemCyan, UIColor.systemYellow]
                .map { $0.withAlphaComponent(0.12) })
            let system = meta.systemMessage.map { $0.replacingOccurrences(of: "\\s", with: " ") }
                ?? Self.subFallback(msgID, message: message)
            header.append(system, font(fontSize, .medium), ChatTheme.textPrimary)
            _ = small
        }
        shapes.append(RowShape(kind: .symbol(symbol.0, symbol.1), rect: CGRect(x: sideInset, y: top + 1, width: icon, height: icon)))
        if let gem {
            attachments.append(RowAttachment(kind: .cheer, url: gem, pixelHeight: Int(icon * scale),
                                             rect: CGRect(x: sideInset, y: top + 1, width: icon, height: icon),
                                             overlays: [], effects: 0, animate: style.animatesEmotes))
            shapes.removeLast()
        }
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
}
