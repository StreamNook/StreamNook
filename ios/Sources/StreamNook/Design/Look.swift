import Observation
import SNKit
import SwiftUI

/// The look in force: the theme's colours, how glassy our own surfaces are, the motion setting and
/// the interface font, all from the core's settings. One per app; views read it through `SN`.
@Observable @MainActor
final class Look {
    static let shared = Look()

    private(set) var palette = Palette.frostedGlass
    /// Glassiness from 0 (every surface opaque) to 1 (the theme's own see-through tints).
    private(set) var glassiness: Double = 1
    /// The backdrop blur behind glass; off leaves tinted fills with no frost.
    private(set) var glassBlur = true
    private(set) var motion = MotionMode.full
    private(set) var fontID: String?
    /// The typed family the Custom font asks for.
    private(set) var fontCustom: String?
    /// Moves when the interface font's faces change (a Custom family registered), so text drawn
    /// in it redraws.
    private(set) var fontRevision = 0
    private(set) var themeID = "winters-glass"
    /// The theme's light (Prism's), or nil for a palette-only theme.
    private(set) var light: ThemeLight?

    /// Takes the look out of a settings read.
    func apply(_ settings: SettingsLook) {
        glassiness = Double(settings.glassiness) / 100
        ChatTheme.use(glassiness: glassiness)
        glassBlur = settings.glassBlur ?? true
        motion = settings.motion
        fontID = settings.font
        fontCustom = settings.fontCustom
        Typeface.publishChat()
        themeID = settings.theme ?? "winters-glass"
    }

    /// Takes the palette and light of the theme the core resolved.
    func apply(palette: Palette, themeID: String, light: ThemeLight?) {
        if palette != self.palette {
            self.palette = palette
            ChatTheme.use(palette)
        }
        if light != self.light { self.light = light }
        self.themeID = themeID
    }

    func fontFacesChanged() {
        fontRevision += 1
    }

    /// Where the Custom family stands, for the line under its name field.
    private(set) var customFont = CustomFontStatus.idle

    func setCustomFont(_ status: CustomFontStatus) {
        if status != customFont { customFont = status }
    }

    /// Glass shows as Liquid Glass: blur is on and glassiness above zero.
    var frosted: Bool { glassBlur && glassiness > 0 }
    /// How much of the opaque floor shows under a glass surface.
    var floorOpacity: Double { 1 - glassiness }

    /// Shows a state without writing settings: the Glassiness slider while it is dragged, and
    /// drive steps.
    func preview(glassiness: Double? = nil, motion: MotionMode? = nil) {
        if let glassiness {
            self.glassiness = glassiness
            ChatTheme.use(glassiness: glassiness)
        }
        if let motion { self.motion = motion }
    }
}

/// Where a typed Custom family stands: being read, drawn, not a Google Font, or not reachable now.
enum CustomFontStatus: Equatable {
    case idle
    case loading(String)
    case ready(String)
    case notFound(String)
    case unreachable(String)

    /// The family this status is about.
    var family: String? {
        switch self {
        case .idle: nil
        case .loading(let f), .ready(let f), .notFound(let f), .unreachable(let f): f
        }
    }
}

/// The theme's colours by name, read from the look in force, so a theme change repaints whatever
/// read them.
@MainActor
enum SN {
    private static var p: Palette { Look.shared.palette }

    static var background: Color { p.background.color }
    static var backgroundSecondary: Color { p.backgroundSecondary.color }
    static var backgroundTertiary: Color { p.backgroundTertiary.color }
    static var surface: Color { p.surface.color }
    static var surfaceHover: Color { p.surfaceHover.color }
    static var surfaceActive: Color { p.surfaceActive.color }
    static var textPrimary: Color { p.textPrimary.color }
    static var textSecondary: Color { p.textSecondary.color }
    static var textMuted: Color { p.textMuted.color }
    static var accent: Color { p.accent.color }
    static var accentHover: Color { p.accentHover.color }
    static var accentMuted: Color { p.accentMuted.color }
    /// The accent for marks and bars on a card, readable on the card track in every theme.
    static var accentOnSurface: Color { p.accentOnSurface.color }
    static var border: Color { p.border.color }
    static var borderLight: Color { p.borderLight.color }
    static var borderSubtle: Color { p.borderSubtle.color }
    static var success: Color { p.success.color }
    static var warning: Color { p.warning.color }
    static var error: Color { p.error.color }
    static var info: Color { p.info.color }
    static var highlightPurple: Color { p.highlightPurple.color }
    static var highlightBlue: Color { p.highlightBlue.color }
    static var highlightOrange: Color { p.highlightOrange.color }

    static var live: Color { FixedColor.live.color }
    static var twitch: Color { FixedColor.twitch.color }
    static var kick: Color { FixedColor.kick.color }

    /// The darker pill that marks the chosen item inside glass. On a light theme a dark pill
    /// would sink the theme's dark text into it, so there it is a tint of that text instead.
    static var glazeSelected: Color {
        p.lightContent ? Color.black.opacity(0.72) : p.textPrimary.color.opacity(0.1)
    }

    /// The 1 point edge of a layer floating over content (the mini player), so it reads over any
    /// thumbnail, with `floatShadow` under it.
    static let floatEdge = Color.white.opacity(0.12)

    /// The opaque floor under every glass surface, showing more as glassiness falls.
    static var floor: Color { p.backgroundTertiary.color.opacity(Look.shared.floorOpacity) }
}
