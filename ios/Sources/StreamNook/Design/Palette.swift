import SwiftUI

/// A colour as red, green, blue and alpha from 0 to 1, read from the CSS the themes are written in
/// (`#rrggbb`, `#rgb`, `rgb()`, `rgba()`).
struct RGBA: Equatable, Sendable {
    var r: Double
    var g: Double
    var b: Double
    var a: Double = 1

    init(_ r: Double, _ g: Double, _ b: Double, _ a: Double = 1) {
        self.r = r
        self.g = g
        self.b = b
        self.a = a
    }

    init(hex: UInt32, alpha: Double = 1) {
        self.init(Double((hex >> 16) & 0xFF) / 255, Double((hex >> 8) & 0xFF) / 255, Double(hex & 0xFF) / 255, alpha)
    }

    /// Nil for anything that is not one of the CSS forms the themes use.
    init?(css: String) {
        let text = css.trimmingCharacters(in: .whitespaces).lowercased()
        if text.hasPrefix("#") {
            var digits = String(text.dropFirst())
            if digits.count == 3 { digits = digits.map { "\($0)\($0)" }.joined() }
            guard digits.count == 6 || digits.count == 8, let value = UInt64(digits, radix: 16) else { return nil }
            if digits.count == 8 {
                self.init(hex: UInt32(value >> 8), alpha: Double(value & 0xFF) / 255)
            } else {
                self.init(hex: UInt32(value))
            }
            return
        }
        guard let open = text.firstIndex(of: "("), text.hasSuffix(")"), text.hasPrefix("rgb") else { return nil }
        let inside = text[text.index(after: open)..<text.index(before: text.endIndex)]
        let parts = inside.split(whereSeparator: { $0 == "," || $0 == " " || $0 == "/" }).map(String.init)
        guard parts.count >= 3 else { return nil }
        func channel(_ part: String) -> Double? {
            part.hasSuffix("%") ? Double(part.dropLast()).map { $0 / 100 } : Double(part).map { $0 / 255 }
        }
        func alpha(_ part: String) -> Double? {
            part.hasSuffix("%") ? Double(part.dropLast()).map { $0 / 100 } : Double(part)
        }
        guard let r = channel(parts[0]), let g = channel(parts[1]), let b = channel(parts[2]) else { return nil }
        self.init(r, g, b, parts.count > 3 ? (alpha(parts[3]) ?? 1) : 1)
    }

    /// This colour laid over `under` (straight alpha compositing).
    func over(_ under: RGBA) -> RGBA {
        let out = a + under.a * (1 - a)
        guard out > 0 else { return RGBA(0, 0, 0, 0) }
        func mix(_ top: Double, _ bottom: Double) -> Double { (top * a + bottom * under.a * (1 - a)) / out }
        return RGBA(mix(r, under.r), mix(g, under.g), mix(b, under.b), out)
    }

    /// `amount` of this colour and the rest black, as CSS `color-mix(in srgb, C amount, #000)`.
    func darkened(keeping amount: Double) -> RGBA { RGBA(r * amount, g * amount, b * amount, a) }

    func opacity(_ alpha: Double) -> RGBA { RGBA(r, g, b, alpha) }

    var color: Color { Color(.sRGB, red: r, green: g, blue: b, opacity: a) }

    /// Rec. 709 luminance of the colour, 0 to 1.
    var luminance: Double { 0.2126 * r + 0.7152 * g + 0.0722 * b }
}

/// A theme's colours. The core will hand these over resolved (see ios/docs/rust-requests.md);
/// until it does, the app draws the default theme.
struct Palette: Equatable, Sendable {
    var background: RGBA
    var backgroundSecondary: RGBA
    var backgroundTertiary: RGBA
    var surface: RGBA
    var surfaceHover: RGBA
    var surfaceActive: RGBA
    var textPrimary: RGBA
    var textSecondary: RGBA
    var textMuted: RGBA
    var accent: RGBA
    var accentHover: RGBA
    var accentMuted: RGBA
    var border: RGBA
    var borderLight: RGBA
    var borderSubtle: RGBA
    var success: RGBA
    var warning: RGBA
    var error: RGBA
    var info: RGBA
    var highlightPink: RGBA
    var highlightPurple: RGBA
    var highlightBlue: RGBA
    var highlightCyan: RGBA
    var highlightGreen: RGBA
    var highlightYellow: RGBA
    var highlightOrange: RGBA
    var highlightRed: RGBA

    /// Frosted Glass (`winters-glass`), the default theme.
    static let frostedGlass = Palette(
        background: RGBA(hex: 0x0C0C0D),
        backgroundSecondary: RGBA(1, 1, 1, 0.03),
        backgroundTertiary: RGBA(hex: 0x1A1A1B),
        surface: RGBA(hex: 0x97B1B9, alpha: 0.15),
        surfaceHover: RGBA(hex: 0x97B1B9, alpha: 0.25),
        surfaceActive: RGBA(hex: 0x97B1B9, alpha: 0.35),
        textPrimary: RGBA(hex: 0xFFFFFF),
        textSecondary: RGBA(hex: 0x97B1B9),
        textMuted: RGBA(hex: 0x97B1B9, alpha: 0.6),
        accent: RGBA(hex: 0x97B1B9),
        accentHover: RGBA(hex: 0xADC4CC),
        accentMuted: RGBA(hex: 0x97B1B9, alpha: 0.5),
        border: RGBA(hex: 0x97B1B9, alpha: 0.3),
        borderLight: RGBA(hex: 0x97B1B9, alpha: 0.2),
        borderSubtle: RGBA(hex: 0x97B1B9, alpha: 0.1),
        success: RGBA(hex: 0x22C55E),
        warning: RGBA(hex: 0xEAB308),
        error: RGBA(hex: 0xEF4444),
        info: RGBA(hex: 0x6B9DFF),
        highlightPink: RGBA(hex: 0xFF6B9D),
        highlightPurple: RGBA(hex: 0xC06BFF),
        highlightBlue: RGBA(hex: 0x6B9DFF),
        highlightCyan: RGBA(hex: 0x6BFFC0),
        highlightGreen: RGBA(hex: 0x22C55E),
        highlightYellow: RGBA(hex: 0xFFC06B),
        highlightOrange: RGBA(hex: 0xFF9F6B),
        highlightRed: RGBA(hex: 0xFF6B6B)
    )

    /// Light status bar content on a dark background (luminance under one half).
    var lightContent: Bool { background.luminance < 0.5 }
}

/// Colours that are the same in every theme.
enum FixedColor {
    /// The LIVE red, `--color-live`.
    static let live = RGBA(hex: 0xEF4444)
    static let twitch = RGBA(hex: 0x9146FF)
    static let kick = RGBA(hex: 0x53FC18)
    /// The watch-streak amber.
    static let streak = RGBA(hex: 0xFCD34D)
    static let goldenTrain = RGBA(hex: 0xFFC83D)
    static let goldenTrainEnd = RGBA(hex: 0xFFA500)
    /// The dark veil card chips lay over a thumbnail, `rgb(16 16 22)`.
    static let chipVeil = RGBA(hex: 0x101016)
}
