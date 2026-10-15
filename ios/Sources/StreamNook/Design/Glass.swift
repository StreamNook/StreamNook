import SwiftUI

// StreamNook's own surfaces follow the Glassiness setting: under each one sits the theme's
// opaque tertiary floor, showing more as glassiness falls, so at 0 nothing is see-through and no
// frost remains. System chrome (the tab bar, sheets, menus, popovers) stays Apple's Liquid Glass.

extension View {
    /// A panel in the page: the opaque floor, the theme's faint wash, light along its top edge, a
    /// dark contour and a small shadow. No blur: its backdrop is the flat page, which blurs to
    /// itself.
    @MainActor
    func snPanel(radius: CGFloat = 12) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        return background {
            ZStack {
                shape.fill(SN.floor)
                shape.fill(SN.backgroundSecondary)
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

    /// A media card (stream, category): the floor, the wash and a faint accent tint, and nothing
    /// else. A grid of outlined, lit rectangles reads as noise.
    @MainActor
    func snMediaCard(radius: CGFloat = SNRadius.card) -> some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        return background {
            ZStack {
                shape.fill(SN.floor)
                shape.fill(SN.backgroundSecondary)
                shape.fill(SN.accent.opacity(0.04))
            }
        }
    }

    /// Glass floating over content (a toast, the segment capsules, the mini player's close): Liquid
    /// Glass with the opaque floor laid over it as glassiness falls, solid at 0 or with blur off.
    @MainActor
    func snGlass<S: Shape>(_ shape: S, tint: Color? = nil, interactive: Bool = false) -> some View {
        modifier(SNGlass(shape: shape, tint: tint, interactive: interactive))
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

    func body(content: Content) -> some View {
        let look = Look.shared
        if look.frosted {
            let glass = tint.map { Glass.regular.tint($0) } ?? .regular
            content
                .background { shape.fill(SN.floor) }
                .glassEffect(interactive ? glass.interactive() : glass, in: shape)
        } else {
            content.background {
                ZStack {
                    shape.fill(SN.backgroundTertiary)
                    if let tint { shape.fill(tint) }
                }
            }
        }
    }
}

/// Corner sizes. A corner inside another is the outer one less the gap between them, so the two
/// curves run parallel (ios/docs/design-eye.md).
enum SNRadius {
    /// Stream cards: the 10 point thumbnail plus the card's 8 point inset.
    static let card: CGFloat = 18
    /// A thumbnail inside a card, concentric with the card's corner.
    static let thumb: CGFloat = 10
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
