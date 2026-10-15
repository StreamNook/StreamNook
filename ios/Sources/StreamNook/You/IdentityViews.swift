import SNKit
import SwiftUI
import UIKit

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
/// cut out of the band, with an optional medal (the StreamNook member badge) on a disc at its
/// lower right. It shows the one picture others see in chat; when that is the 7TV picture, 7TV's
/// mark sits on its lower left, so which picture it is shows without words. With `explains`, a tap
/// says which picture shows where.
struct CutoutAvatar: View {
    let platformURL: String?
    let picture: AvatarPick?
    let size: CGFloat
    var medal: String?
    var medalLabel = ""
    /// A picture just picked, shown before Twitch serves it.
    var local: UIImage?
    var explains = false
    @State private var explaining = false

    private var ring: CGFloat { max(3, (size * 0.05).rounded()) }
    private var showsSevenTv: Bool { local == nil && picture != nil }

    var body: some View {
        Group {
            if let local {
                Image(uiImage: local).resizable().scaledToFill().frame(width: size, height: size).clipShape(Circle())
            } else {
                SevenTvAvatar(platformURL: platformURL, picture: picture, size: size)
            }
        }
            .padding(ring)
            .background { Circle().fill(SN.background).overlay { Circle().fill(SN.floor) }.overlay { Circle().fill(SN.backgroundSecondary) } }
            .shadow(color: .black.opacity(0.35), radius: 8, y: 4)
            .overlay(alignment: .bottomTrailing) {
                if let medal {
                    // The badge on a disc of the card's own floor, so it reads as a medal, not a
                    // mark scratched on the photo.
                    CosmeticImage(medal, animate: true)
                        .frame(width: size * 0.24, height: size * 0.24)
                        .frame(width: size * 0.32, height: size * 0.32)
                        .background(Circle().fill(SN.backgroundTertiary))
                        .overlay { Circle().stroke(Color.black.opacity(0.4), lineWidth: 0.5) }
                        .offset(x: -ring * 0.6, y: -ring * 0.6)
                        .accessibilityLabel(medalLabel)
                }
            }
            .overlay(alignment: .bottomLeading) {
                if showsSevenTv {
                    PlatformMark(platform: .sevenTV, size: size * 0.11)
                        .frame(width: size * 0.26, height: size * 0.26)
                        .background(Circle().fill(SN.backgroundTertiary))
                        .overlay { Circle().stroke(Color.black.opacity(0.4), lineWidth: 0.5) }
                        .offset(x: ring * 0.6, y: -ring * 0.6)
                        .accessibilityLabel("7TV picture")
                }
            }
            .contentShape(Circle())
            .onTapGesture { if explains { explaining = true } }
            .glassPopover(isPresented: $explaining, arrowEdge: .top) {
                PictureNote(sevenTv: showsSevenTv)
            }
            .accessibilityAddTraits(explains ? .isButton : [])
            .accessibilityHint(explains ? "Says which picture shows where" : "")
    }
}

/// Which of your two pictures shows where, as the avatar's tap explains it.
private struct PictureNote: View {
    let sevenTv: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            line(.sevenTV, sevenTv ? "Your 7TV picture. StreamNook and 7TV viewers see it beside your name in chat."
                                   : "Set a 7TV picture and StreamNook and 7TV viewers see it in chat instead.")
            line(.twitch, "Your Twitch picture shows on Twitch itself, and to everyone where you have no 7TV picture.")
            Text("Change either in Edit profile.")
                .font(.snCaption)
                .foregroundStyle(SN.textMuted)
        }
        .padding(16)
        .frame(width: 290)
    }

    private func line(_ platform: PlatformMark.Platform, _ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            PlatformMark(platform: platform, size: platform == .sevenTV ? 10 : 13)
                .frame(width: 18)
            Text(text)
                .font(.sn(13.5))
                .foregroundStyle(SN.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
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
