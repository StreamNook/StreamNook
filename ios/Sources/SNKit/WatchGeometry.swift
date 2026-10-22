import Foundation

/// Landscape chat from the `fullscreen_chat` settings: over the video (`overlay`, a see-through
/// column) or beside it (`beside`, a column taken out of the picture), on the left when `side` is
/// "left" and on the right otherwise ("auto" included).
public struct LandscapeChatLayout: Equatable, Sendable {
    public static let minWidth = 240.0
    /// The video keeps at least this much of the screen's width beside the column.
    public static let minVideo = 240.0
    /// The least text margin on an edge with no camera cutout (the rounded corner alone).
    public static let cornerMargin = 12.0

    public var beside: Bool
    public var leading: Bool
    /// The saved width in points, before the screen clamp.
    public var width: Double
    /// The overlay's background, 0 (see-through) to 100 percent.
    public var opacity: Double

    public init(layout: String, side: String, width: Double, opacity: Double) {
        beside = layout == "beside"
        leading = side == "left"
        self.width = width
        self.opacity = min(max(opacity, 0), 100)
    }

    /// The column's width on a screen this wide, edge to edge: the saved (or live, mid-drag) width,
    /// never under 240 and never leaving the video less than 240, rounded. The range is the whole
    /// screen's, safe areas included: the column's background always runs to the screen edge.
    public func width(screenWidth: Double, live: Double? = nil) -> Double {
        Self.clamp(live ?? width, screenWidth: screenWidth)
    }

    public static func clamp(_ width: Double, screenWidth: Double) -> Double {
        let most = max(minWidth, screenWidth - minVideo)
        return min(max(width, minWidth), most).rounded()
    }

    /// How far the column's text stays in from the screen edge it sits against. The system
    /// reports the same landscape inset on both sides, but only one has the camera cutout: that
    /// side keeps the whole inset, the other only a corner margin, so no band of the column sits
    /// empty. The screen's corners round more on the phones with the bigger inset, so the margin
    /// is two fifths of it (18 pt at 44, 24 at 59), never under 12: the pinned card's corner at
    /// the top and the composer's at the bottom stay clear of the rounded screen corner.
    public func textInset(edgeInset: Double, cutoutOnChatSide: Bool) -> Double {
        if cutoutOnChatSide { return edgeInset }
        return min(edgeInset, max(Self.cornerMargin, (edgeInset * 0.4).rounded()))
    }

    /// A drag on the handle on the column's inner edge: pulling toward the picture widens it.
    public func dragged(from start: Double, by dx: Double, screenWidth: Double) -> Double {
        Self.clamp(leading ? start + dx : start - dx, screenWidth: screenWidth)
    }
}

/// The landscape swipe dials (brightness on the left half of the picture, volume on the right).
public enum Dial {
    /// The level after a vertical drag of `dy` points (down is positive) that began at `start`:
    /// the full range is max(140, 80 percent of the picture's height) of travel, up raises.
    public static func level(start: Double, dy: Double, height: Double) -> Double {
        let travel = max(140, height * 0.8)
        return min(max(start - dy / travel, 0), 1)
    }

    /// Hundredths: a level is pushed only when it lands on a new one.
    public static func hundredth(_ level: Double) -> Int { Int((level * 100).rounded()) }

    /// Tenths: one light step per tenth crossed.
    public static func tenth(_ level: Double) -> Int { Int((level * 10).rounded(.down)) }

    /// Brightness never goes under 2 percent, so it can always be dialled back.
    public static let brightnessFloor = 0.02

    /// The quiet strip along the top and bottom of the picture, where the system's own pulls start
    /// (Notification and Control Center down from the top, the home bar up from the bottom): about
    /// a fingertip, the 44 pt touch minimum. Android's twin is `dialZone.ts` (48 dp).
    public static let edgeGuard = 44.0

    /// Whether a swipe that started `y` points down a picture `height` tall may turn a dial. Each
    /// strip is `edgeGuard`, or the safe area on that edge when that is larger.
    public static func canStart(y: Double, height: Double, edgeTop: Double = 0, edgeBottom: Double = 0) -> Bool {
        y >= max(edgeGuard, edgeTop) && y <= height - max(edgeGuard, edgeBottom)
    }
}
