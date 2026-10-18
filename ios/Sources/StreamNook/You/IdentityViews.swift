import SNKit
import SwiftUI
import UIKit

// The pieces every identity surface shares (the You card, the Edit profile preview, a chatter's
// profile sheet): the Atmosphere glowing behind the identity, the picture standing alone, and the
// name below it.

/// A member's Atmosphere as the air around their identity (Brandon: it read like a cover, not an
/// atmosphere): the chat row's own wash, held soft and low behind the picture and name and faded
/// out at every edge, so it is a glow in the card and never a block of art across it. It breathes
/// (opacity only) where the Atmosphere moves. Without one, a faint glow of `tint`.
struct AtmosphereGlow: View {
    let look: AtmosphereLook?
    var tint: Color = SN.accent

    var body: some View {
        ZStack {
            if let look {
                WashView(look: look, breathe: true).opacity(0.7)
            } else {
                RadialGradient(colors: [tint.opacity(0.28), .clear], center: UnitPoint(x: 0.25, y: 0.3),
                               startRadius: 0, endRadius: 260)
            }
        }
        // Faded on every side: in from both edges across, and down from just under the top to
        // nothing well before the bottom, so no edge of the art ever shows.
        .mask {
            LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .black, location: 0.18),
                                   .init(color: .black, location: 0.62), .init(color: .clear, location: 1)],
                           startPoint: .leading, endPoint: .trailing)
        }
        .mask {
            LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .black, location: 0.2),
                                   .init(color: .black.opacity(0.75), location: 0.5), .init(color: .clear, location: 1)],
                           startPoint: .top, endPoint: .bottom)
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// A person's picture, standing alone on a black disc (Brandon: black behind a clear picture, not
/// gray), with a hairline edge and a soft shadow. No marks on your own: which accounts you hold
/// and what you wear show in the card's rows. On another person's, their StreamNook badge sits on
/// it as a medal (`medal`), because their card shows it nowhere else. With `explains`, a tap says
/// which picture shows where.
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

    private var showsSevenTv: Bool { local == nil && picture != nil }

    var body: some View {
        Group {
            if let local {
                Image(uiImage: local).resizable().scaledToFill().frame(width: size, height: size)
                    .background(Color.black)
                    .clipShape(Circle())
            } else {
                SevenTvAvatar(platformURL: platformURL, picture: picture, size: size)
            }
        }
            .shadow(color: .black.opacity(0.35), radius: 8, y: 4)
            .overlay(alignment: .bottomTrailing) {
                if let medal {
                    // Another person's StreamNook badge, on a black disc like the picture's own,
                    // so it reads as a medal and not a mark scratched on the photo.
                    CosmeticImage(medal, animate: true)
                        .frame(width: size * 0.24, height: size * 0.24)
                        .frame(width: size * 0.32, height: size * 0.32)
                        .background(Circle().fill(Color.black))
                        .overlay { Circle().stroke(Color.white.opacity(0.1), lineWidth: 0.5) }
                        .offset(x: size * 0.02, y: size * 0.02)
                        .accessibilityLabel(medalLabel)
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
