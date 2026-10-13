import UIKit

/// Chat's own colours until the app's design system takes them over: the dark theme's text,
/// surface and status tones the shared row components use.
enum ChatTheme {
    static let textPrimary = UIColor(white: 0.93, alpha: 1)
    static let textSecondary = UIColor(white: 0.70, alpha: 1)
    static let textMuted = UIColor(white: 0.52, alpha: 1)
    static let borderSubtle = UIColor(white: 1, alpha: 0.08)
    static let surface = UIColor(white: 1, alpha: 0.045)
    static let accent = color("#9147ff")!
    static let info = color("#60a5fa")!
    static let warning = color("#f59e0b")!
    static let error = color("#ef4444")!
    static let success = color("#22c55e")!
    static let highlightOrange = color("#ff8c1a")!
    static let highlightRed = color("#ff4d4d")!
    static let highlightCyan = color("#22d3ee")!
    static let background = UIColor(red: 0.055, green: 0.055, blue: 0.063, alpha: 1)
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
