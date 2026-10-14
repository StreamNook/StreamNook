import UIKit

/// Chat's colours as UIKit takes them: the design system's theme tokens (text, surface, border,
/// accent and status tones) plus chat's own highlight tones. Immutable, so the pipeline's
/// background layout reads them freely.
enum ChatTheme {
    private static let theme = Palette.frostedGlass
    static let textPrimary = theme.textPrimary.uiColor
    static let textSecondary = theme.textSecondary.uiColor
    static let textMuted = theme.textMuted.uiColor
    static let borderSubtle = theme.borderSubtle.uiColor
    static let surface = theme.surface.uiColor
    static let accent = theme.accent.uiColor
    static let info = theme.info.uiColor
    static let warning = theme.warning.uiColor
    static let error = theme.error.uiColor
    static let success = theme.success.uiColor
    static let highlightOrange = color("#ff8c1a")!
    static let highlightRed = color("#ff4d4d")!
    static let highlightCyan = color("#22d3ee")!
    static let background = theme.background.uiColor
    /// The colour Twitch gives a chatter who never picked one.
    static let fallbackName = color("#9147FF")!

    /// `#rgb`, `#rrggbb` or `#rrggbbaa`.
    static func color(_ hex: String?) -> UIColor? {
        guard var s = hex?.trimmingCharacters(in: .whitespaces), s.hasPrefix("#") else { return nil }
        s.removeFirst()
        if s.count == 3 { s = s.map { "\($0)\($0)" }.joined() }
        guard s.count == 6 || s.count == 8, let value = UInt64(s, radix: 16) else { return nil }
        let channel = { (shift: UInt64) in CGFloat((value >> shift) & 0xff) / 255 }
        if s.count == 8 {
            return UIColor(red: channel(24), green: channel(16), blue: channel(8), alpha: channel(0))
        }
        return UIColor(red: channel(16), green: channel(8), blue: channel(0), alpha: 1)
    }
}
