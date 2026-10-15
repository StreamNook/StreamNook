import SNKit
import SwiftUI

// The pieces every identity surface shares (the You card, the Edit profile preview, a chatter's
// profile sheet): the Atmosphere as a band across the top, the avatar cut into the card's edge
// under it, and the name below the art, never on it.

/// A member's Atmosphere as a band: its swatch, the full-strength art the picker shows. (The chat
/// row's wash is built to sit dim behind text, so as a banner it reads near black.) Without one, a
/// quiet wash of `tint` so the card keeps its shape.
struct AtmosphereBand: View {
    let look: AtmosphereLook?
    var tint: Color = SN.accent

    var body: some View {
        ZStack {
            if let look, !look.swatch.isEmpty {
                FillView(fill: look.swatch)
            } else if let look {
                WashView(look: look)
            } else {
                ZStack {
                    SN.backgroundTertiary
                    RadialGradient(colors: [tint.opacity(0.42), .clear], center: .topLeading, startRadius: 0, endRadius: 280)
                    RadialGradient(colors: [tint.opacity(0.18), .clear], center: .bottomTrailing, startRadius: 0, endRadius: 240)
                }
            }
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// An avatar set into the card's edge: a ring of the card's own surface around it, so it reads as
/// cut out of the band, with an optional medal (the StreamNook member badge) on its corner.
struct CutoutAvatar: View {
    let platformURL: String?
    let picture: AvatarPick?
    let size: CGFloat
    var medal: String?
    var medalLabel = ""

    private var ring: CGFloat { max(3, (size * 0.05).rounded()) }

    var body: some View {
        SevenTvAvatar(platformURL: platformURL, picture: picture, size: size)
            .padding(ring)
            .background { Circle().fill(SN.background).overlay { Circle().fill(SN.floor) }.overlay { Circle().fill(SN.backgroundSecondary) } }
            .shadow(color: .black.opacity(0.35), radius: 8, y: 4)
            .overlay(alignment: .bottomTrailing) {
                if let medal {
                    CosmeticImage(medal, animate: true)
                        .frame(width: size * 0.34, height: size * 0.34)
                        .shadow(color: .black.opacity(0.6), radius: 2, y: 1)
                        .offset(x: -ring * 0.2, y: -ring * 0.2)
                        .accessibilityLabel(medalLabel)
                }
            }
    }
}

/// A brand's mark in a round tile, the picture of an account row that has no avatar.
struct BrandTile: View {
    let platform: PlatformMark.Platform
    var size: CGFloat = 44

    var body: some View {
        Circle()
            .fill(platform.color.opacity(0.14))
            .overlay { Circle().strokeBorder(platform.color.opacity(0.22), lineWidth: 1) }
            .overlay { PlatformMark(platform: platform, size: size * (platform == .sevenTV ? 0.3 : 0.42)) }
            .frame(width: size, height: size)
    }
}

/// A section's label: the brand's mark where a brand owns the section, then the words.
struct ProfileSectionHeader<Trailing: View>: View {
    let title: String
    var platform: PlatformMark.Platform?
    var streamNook = false
    var detail: String?
    @ViewBuilder var trailing: () -> Trailing

    init(_ title: String, platform: PlatformMark.Platform? = nil, streamNook: Bool = false, detail: String? = nil,
         @ViewBuilder trailing: @escaping () -> Trailing = { EmptyView() }) {
        self.title = title
        self.platform = platform
        self.streamNook = streamNook
        self.detail = detail
        self.trailing = trailing
    }

    var body: some View {
        HStack(alignment: .center, spacing: 6) {
            if let platform {
                PlatformMark(platform: platform, size: platform == .sevenTV ? 10 : 12)
            } else if streamNook {
                PenroseMark(width: 12, centered: false, still: true)
            }
            Text(title).snSectionStyle()
            if let detail {
                Text(detail).font(.sn(12)).foregroundStyle(SN.textMuted).lineLimit(1)
            }
            Spacer(minLength: 8)
            trailing()
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// The worn mark on a picker tile: an accent ring around it and a check on its corner.
struct WornMarks: ViewModifier {
    let worn: Bool
    var radius: CGFloat = SNRadius.panel

    func body(content: Content) -> some View {
        content
            .overlay {
                if worn {
                    RoundedRectangle(cornerRadius: radius, style: .continuous).strokeBorder(SN.accent, lineWidth: 2)
                }
            }
            .overlay(alignment: .topTrailing) {
                if worn {
                    Image(systemName: Symbol.check)
                        .font(.system(size: 10, weight: .heavy))
                        .foregroundStyle(SN.background)
                        .frame(width: 18, height: 18)
                        .background(Circle().fill(SN.accent))
                        .offset(x: 5, y: -5)
                }
            }
    }
}

extension View {
    func wornMarks(_ worn: Bool, radius: CGFloat = SNRadius.panel) -> some View { modifier(WornMarks(worn: worn, radius: radius)) }
}

/// A name's color from the core's `#rrggbb`, Twitch purple when it has none.
func nameColor(_ hex: String?) -> Color {
    Color(cgColor: FillPainter.color(hex ?? "#9147ff") ?? CGColor(red: 0.57, green: 0.28, blue: 1, alpha: 1))
}
