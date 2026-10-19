import SwiftUI

// StreamNook's own surfaces follow the Glassiness setting: under each one sits the theme's
// opaque tertiary floor, showing more as glassiness falls, so at 0 nothing is see-through and no
// frost remains. System chrome (the tab bar, sheets, menus, popovers) stays Apple's Liquid Glass.

extension View {
    /// A panel in the page: the opaque floor, the theme's faint wash and its share of the theme's
    /// light, light along its top edge, a dark contour and a small shadow. No blur: its backdrop is
    /// the page, whose light is already soft. A panel that is a text field passes `light: nil`:
    /// a field being typed into stays plain. A `stage` (a card showing someone's cosmetics) takes
    /// no light and, under a theme that throws light, hides the page's light behind it too, so
    /// the theme's light is never read as part of the cosmetic.
    /// Inside a grouped list the row is already the card, so the panel draws nothing.
    @MainActor
    func snPanel(radius: CGFloat = 12, light: ThemeLight.Surface? = .panel, stage: Bool = false) -> some View {
        modifier(PanelSurface(radius: radius, light: stage ? nil : light, stage: stage))
    }

    /// A media card (stream, category): the floor, the wash, a faint accent tint and the panel's
    /// share of the theme's light, and nothing else. A grid of outlined, lit rectangles reads as
    /// noise.
    @MainActor
    func snMediaCard(radius: CGFloat = SNRadius.card) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        return background {
            ZStack {
                shape.fill(SN.floor)
                shape.fill(SN.backgroundSecondary)
                shape.fill(SN.accent.opacity(0.04))
                ThemeLightLayer(surface: .panel).clipShape(shape)
            }
        }
    }

    /// Glass floating over content (a toast, the segment capsules, the mini player's close): Liquid
    /// Glass with the opaque floor laid over it as glassiness falls, solid at 0 or with blur off.
    /// `light` is the share of the theme's light it catches; small controls take none.
    @MainActor
    func snGlass<S: Shape>(_ shape: S, tint: Color? = nil, interactive: Bool = false,
                           light: ThemeLight.Surface? = nil) -> some View {
        modifier(SNGlass(shape: shape, tint: tint, interactive: interactive, light: light))
    }

    /// A card chip's veil over a thumbnail: frosted, a thin dark film, solid at 0.
    @MainActor
    func snChipVeil<S: Shape>(_ shape: S, density: Double = 0.22) -> some View {
        let g = Look.shared.glassiness
        let film = FixedColor.chipVeil.color.opacity(density + (1 - g) * (1 - density))
        return background {
            ZStack {
                if Look.shared.frosted { shape.fill(.ultraThinMaterial) }
                shape.fill(film)
            }
        }
    }

    /// Elevation 1: a close shadow and a soft one.
    func elevation1() -> some View {
        shadow(color: .black.opacity(0.20), radius: 1, y: 1)
            .shadow(color: .black.opacity(0.12), radius: 3, y: 2)
    }

    /// The shadow a floating layer casts (the mini player, a toast).
    func floatShadow() -> some View {
        shadow(color: .black.opacity(0.65), radius: 16, y: 12)
    }
}

private struct SNGlass<S: Shape>: ViewModifier {
    let shape: S
    let tint: Color?
    let interactive: Bool
    let light: ThemeLight.Surface?

    func body(content: Content) -> some View {
        let look = Look.shared
        if look.frosted {
            let glass = tint.map { Glass.regular.tint($0) } ?? .regular
            content
                .background {
                    ZStack {
                        shape.fill(SN.floor)
                        if let light { ThemeLightLayer(surface: light).clipShape(shape) }
                    }
                }
                .glassEffect(interactive ? glass.interactive() : glass, in: shape)
        } else {
            content.background {
                ZStack {
                    shape.fill(SN.backgroundTertiary)
                    if let tint { shape.fill(tint) }
                    if let light { ThemeLightLayer(surface: light).clipShape(shape) }
                }
            }
        }
    }
}

/// The one filled action on a glass surface (the desktop's ACCENT_BUTTON): a capsule of the accent
/// mixed into black with white text, lit like the glaze rather than outlined: light along the top
/// edge, a half point of bounce along the bottom, a dark half-point contour. It dims a little while
/// pressed.
struct GlazeAccentButton: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(.white)
            .background {
                Capsule().fill(SN.accentFill)
                    .overlay {
                        Capsule().strokeBorder(
                            LinearGradient(stops: [.init(color: .white.opacity(0.18), location: 0),
                                                   .init(color: .clear, location: 0.35),
                                                   .init(color: .clear, location: 0.8),
                                                   .init(color: .white.opacity(0.08), location: 1)],
                                           startPoint: .top, endPoint: .bottom),
                            lineWidth: 1)
                    }
                    .overlay { Capsule().stroke(.black.opacity(0.38), lineWidth: 0.5) }
            }
            .brightness(configuration.isPressed ? -0.05 : 0)
            .contentShape(Capsule())
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

/// Corner sizes. A corner inside another is the outer one less the gap between them, so the two
/// curves run parallel (ios/docs/design-eye.md).
enum SNRadius {
    /// Stream cards: the 10 point thumbnail plus the card's 8 point inset.
    static let card: CGFloat = 18
    /// A thumbnail inside a card, concentric with the card's corner.
    static let thumb: CGFloat = 10
    /// A category tile, which is its box art with nothing around it.
    static let tile: CGFloat = 12
    static let panel: CGFloat = 12
    static let chip: CGFloat = 8
    static let row: CGFloat = 8
    static let mini: CGFloat = 12
}

/// The darker pill that marks the chosen item inside glass: a plain shade, unlit.
struct GlazeSelected: View {
    var body: some View {
        Capsule().fill(SN.glazeSelected)
    }
}

/// `snPanel`'s surface: the opaque floor, the theme's faint wash and light, a dark contour and a
/// lit top edge. Skipped inside a grouped list, where the system card is the panel.
private struct PanelSurface: ViewModifier {
    @Environment(\.snInGroupedList) private var inGroupedList
    let radius: CGFloat
    let light: ThemeLight.Surface?
    let stage: Bool

    func body(content: Content) -> some View {
        if inGroupedList {
            content
        } else {
            let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
            content.background {
                ZStack {
                    if stage && Look.shared.light != nil { shape.fill(SN.background) }
                    shape.fill(SN.floor)
                    shape.fill(SN.backgroundSecondary)
                    if let light { ThemeLightLayer(surface: light).clipShape(shape) }
                }
                .overlay { shape.stroke(Color.black.opacity(0.4), lineWidth: 0.5) }
                .overlay {
                    // Top-lit: light along the top edge, fading out down the sides.
                    shape.strokeBorder(LinearGradient(stops: [.init(color: .white.opacity(0.09), location: 0),
                                                              .init(color: .clear, location: min(1, 14 / 120))],
                                                      startPoint: .top, endPoint: .bottom), lineWidth: 1)
                }
                .elevation1()
            }
        }
    }
}
