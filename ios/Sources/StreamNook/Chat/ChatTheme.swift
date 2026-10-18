import Synchronization
import UIKit

/// Chat's colours as UIKit takes them: the theme in force (text, surface, border, accent and
/// status tones, the same palette the rest of the app draws with) plus chat's own highlight tones.
/// The look hands each new palette over with `use`; the pipeline lays rows out on its own queue,
/// so the colours sit behind a lock, and a change posts `changed` so laid-out rows are redone.
enum ChatTheme {
    private static let current = Mutex(Colors(Palette.frostedGlass))
    /// Posted after `use` takes a new palette.
    static let changed = Notification.Name("app.streamnook.chat.theme")

    /// Takes the palette the look now draws with.
    static func use(_ palette: Palette) {
        let next = Colors(palette)
        current.withLock { $0 = next }
        NotificationCenter.default.post(name: changed, object: nil)
    }

    private static var now: Colors { current.withLock { $0 } }
    private static let glass = Mutex(1.0)

    /// Takes Glassiness (0 to 1) for panels drawn inside rows; a change redoes laid-out rows.
    static func use(glassiness: Double) {
        let moved = glass.withLock { old in
            defer { old = glassiness }
            return old != glassiness
        }
        if moved { NotificationCenter.default.post(name: changed, object: nil) }
    }

    static var glassiness: Double { glass.withLock { $0 } }
    /// The opaque floor under a panel, showing more as glassiness falls.
    static var floor: UIColor { now.backgroundTertiary.withAlphaComponent(CGFloat(1 - glassiness)) }
    /// The theme's faint wash over that floor.
    static var wash: UIColor { now.backgroundSecondary }
    static var textPrimary: UIColor { now.textPrimary }
    static var textSecondary: UIColor { now.textSecondary }
    static var textMuted: UIColor { now.textMuted }
    static var borderSubtle: UIColor { now.borderSubtle }
    static var surface: UIColor { now.surface }
    static var accent: UIColor { now.accent }
    static var info: UIColor { now.info }
    static var warning: UIColor { now.warning }
    static var error: UIColor { now.error }
    static var success: UIColor { now.success }
    static var background: UIColor { now.background }
    static let highlightOrange = color("#ff8c1a")!
    static let highlightRed = color("#ff4d4d")!
    static let highlightCyan = color("#22d3ee")!
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

    /// One palette's chat colours, made once and only read after.
    private final class Colors: @unchecked Sendable {
        let textPrimary, textSecondary, textMuted, borderSubtle, surface, accent: UIColor
        let info, warning, error, success, background: UIColor
        let backgroundSecondary, backgroundTertiary: UIColor

        init(_ p: Palette) {
            textPrimary = p.textPrimary.uiColor
            textSecondary = p.textSecondary.uiColor
            textMuted = p.textMuted.uiColor
            borderSubtle = p.borderSubtle.uiColor
            surface = p.surface.uiColor
            accent = p.accent.uiColor
            info = p.info.uiColor
            warning = p.warning.uiColor
            error = p.error.uiColor
            success = p.success.uiColor
            background = p.background.uiColor
            backgroundSecondary = p.backgroundSecondary.uiColor
            backgroundTertiary = p.backgroundTertiary.uiColor
        }
    }
}
