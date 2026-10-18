import Foundation

/// Landscape chat from the `fullscreen_chat` settings: over the video (`overlay`, a see-through
/// column) or beside it (`beside`, a column taken out of the picture), on the left when `side` is
/// "left" and on the right otherwise ("auto" included).
public struct LandscapeChatLayout: Equatable, Sendable {
    public static let minWidth = 240.0

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

    /// The column's width on a screen this wide: the saved (or live, mid-drag) width, never under
    /// 240 nor past half the screen, rounded.
    public func width(screenWidth: Double, live: Double? = nil) -> Double {
        Self.clamp(live ?? width, screenWidth: screenWidth)
    }

    public static func clamp(_ width: Double, screenWidth: Double) -> Double {
        let most = max(minWidth, (screenWidth / 2).rounded(.down))
        return min(max(width, minWidth), most).rounded()
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
}
