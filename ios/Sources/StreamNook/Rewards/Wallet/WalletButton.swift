import SNKit
import SwiftUI

/// The channel wallet's button inside the composer field, for a Twitch room with points or bits:
/// the channel's own points art (or Twitch's glyph), a small bits gem when bits are on too, a dot
/// for a waiting bonus chest, and a "+N" for every gain Rust records. A tap collects a waiting
/// chest on the way in and opens the wallet sheet.
struct WalletButton: View {
    /// Owned and started by the composer (ChatComposerModel.wallet); this only draws it.
    let wallet: WalletModel
    let session: ChatSession
    /// Closes the composer's own panels as the wallet opens.
    var onOpen: () -> Void = {}
    @State private var pulse = 0

    var body: some View {
        if wallet.faces.hasAny {
            button(wallet)
                .sheet(isPresented: Binding(get: { wallet.sheetOpen }, set: { wallet.sheetOpen = $0 })) {
                    WalletSheet(wallet: wallet, session: session)
                }
        }
    }

    private func button(_ wallet: WalletModel) -> some View {
        Button {
            onOpen()
            if wallet.chestWaiting { Task { await wallet.claimChest(channelID: session.roomID) } }
            wallet.sheetOpen = true
        } label: {
            ZStack(alignment: .topTrailing) {
                face(wallet)
                    .keyframeAnimator(initialValue: Beat(), trigger: pulse) { content, beat in
                        content
                            .scaleEffect(beat.scale)
                            .brightness(beat.brightness)
                            .shadow(color: Color(red: 0.92, green: 0.70, blue: 0.03).opacity(beat.glow), radius: 7 * beat.glow)
                    } keyframes: { _ in
                        KeyframeTrack(\.scale) {
                            CubicKeyframe(1.3, duration: 0.25)
                            CubicKeyframe(1, duration: 0.27)
                            CubicKeyframe(1.3, duration: 0.25)
                            CubicKeyframe(1, duration: 0.38)
                        }
                        KeyframeTrack(\.glow) {
                            CubicKeyframe(0.95, duration: 0.25)
                            CubicKeyframe(0, duration: 0.27)
                            CubicKeyframe(0.95, duration: 0.25)
                            CubicKeyframe(0, duration: 0.38)
                        }
                        KeyframeTrack(\.brightness) {
                            CubicKeyframe(0.35, duration: 0.25)
                            CubicKeyframe(0, duration: 0.27)
                            CubicKeyframe(0.35, duration: 0.25)
                            CubicKeyframe(0, duration: 0.38)
                        }
                    }
                if wallet.chestWaiting {
                    // The gain's amber, rimmed dark so it reads apart from the glyph's arc.
                    Circle()
                        .fill(GainFloat.amber)
                        .overlay(Circle().strokeBorder(.black.opacity(0.6), lineWidth: 1.5))
                        .frame(width: 9, height: 9)
                        .offset(x: 1, y: 4)
                        .accessibilityHidden(true)
                }
            }
            .frame(width: 32, height: 36)
            .contentShape(Rectangle())
            .overlay(alignment: .top) {
                if let pop = wallet.pops.first {
                    GainFloat(pop: pop) { wallet.popDone(pop.id) }
                        .id(pop.id)
                }
            }
        }
        .buttonStyle(.plain)
        .onChange(of: wallet.pops.first?.id) { _, id in if id != nil { pulse += 1 } }
        .accessibilityLabel(label(wallet))
        .accessibilityHint(wallet.chestWaiting ? "Collects the bonus chest and opens the wallet" : "Opens the wallet")
    }

    @ViewBuilder
    private func face(_ wallet: WalletModel) -> some View {
        if let points = wallet.points {
            ZStack(alignment: .bottomTrailing) {
                PointsArt(iconURL: points.iconURL, size: 19)
                if let gem = wallet.bits?.gemURL {
                    RewardArt(url: gem, width: 9, height: 9, radius: 0, fit: true, placeholder: nil)
                        .offset(x: 4, y: 2)
                }
            }
        } else if let gem = wallet.bits?.gemURL {
            RewardArt(url: gem, width: 19, height: 19, radius: 0, fit: true, placeholder: nil)
        }
    }

    /// "Wallet: 12,850 Nooks, 300 bits".
    private func label(_ wallet: WalletModel) -> String {
        var parts: [String] = []
        if let p = wallet.points, let balance = p.balance { parts.append("\(balance.formatted()) \(p.name ?? "channel points")") }
        if let b = wallet.bits, let balance = b.balance { parts.append("\(balance.formatted()) bits") }
        return parts.isEmpty ? "Wallet" : "Wallet: " + parts.joined(separator: ", ")
    }

    private struct Beat {
        var scale = 1.0
        var glow = 0.0
        var brightness = 0.0
    }
}

/// The amount drifting up off the icon and dissolving: amber, so a glance catches it among the
/// composer's accent-colored controls.
private struct GainFloat: View {
    static let amber = Color(red: 0.98, green: 0.75, blue: 0.14)
    let pop: WalletModel.Pop
    let done: () -> Void
    @Environment(\.snStill) private var still

    var body: some View {
        Text("+\(pop.amount.formatted())")
            .font(.system(size: 15, weight: .bold))
            .foregroundStyle(Self.amber)
            .fixedSize()
            .keyframeAnimator(initialValue: Rise(), repeating: false) { content, f in
                content
                    .opacity(f.opacity)
                    .scaleEffect(f.scale)
                    .offset(y: f.rise)
                    .blur(radius: f.blur)
            } keyframes: { _ in
                KeyframeTrack(\.opacity) {
                    LinearKeyframe(1, duration: 0.23)
                    LinearKeyframe(1, duration: 0.57)
                    LinearKeyframe(0, duration: 0.65)
                }
                KeyframeTrack(\.scale) {
                    CubicKeyframe(1.18, duration: 0.23)
                    CubicKeyframe(1, duration: 0.57)
                    CubicKeyframe(1, duration: 0.65)
                }
                KeyframeTrack(\.rise) {
                    CubicKeyframe(still ? 0 : -38, duration: 1.45)
                }
                KeyframeTrack(\.blur) {
                    LinearKeyframe(0, duration: 0.8)
                    LinearKeyframe(2, duration: 0.65)
                }
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
            .task {
                try? await Task.sleep(for: .seconds(1.5))
                done()
            }
    }

    private struct Rise {
        var opacity = 0.0
        var scale = 0.65
        var rise = 0.0
        var blur = 0.0
    }
}

/// The channel's own points art, or Twitch's points glyph when it has none.
struct PointsArt: View {
    let iconURL: String?
    var size: CGFloat = 19

    var body: some View {
        if let iconURL, !iconURL.isEmpty {
            RewardArt(url: iconURL, width: size, height: size, radius: 0, fit: true, placeholder: nil)
        } else {
            PointsGlyph()
                .fill(style: FillStyle(eoFill: true))
                .foregroundStyle(SN.textSecondary)
                .frame(width: size, height: size)
                .accessibilityHidden(true)
        }
    }
}

/// Twitch's channel points glyph: a ring with a thicker quarter arc inside its top-right, on a
/// 24 point grid (the desktop's ChannelPointsIcon).
struct PointsGlyph: Shape {
    func path(in rect: CGRect) -> Path {
        let s = min(rect.width, rect.height) / 24
        let c = CGPoint(x: rect.midX, y: rect.midY)
        var path = Path()
        // The ring: radius 11 outside, 9 inside (even-odd).
        path.addEllipse(in: CGRect(x: c.x - 11 * s, y: c.y - 11 * s, width: 22 * s, height: 22 * s))
        path.addEllipse(in: CGRect(x: c.x - 9 * s, y: c.y - 9 * s, width: 18 * s, height: 18 * s))
        // The quarter arc from 12 o'clock to 3 o'clock, radii 5 to 7.
        var arc = Path()
        arc.addArc(center: c, radius: 7 * s, startAngle: .degrees(-90), endAngle: .degrees(0), clockwise: false)
        arc.addLine(to: CGPoint(x: c.x + 5 * s, y: c.y))
        arc.addArc(center: c, radius: 5 * s, startAngle: .degrees(0), endAngle: .degrees(-90), clockwise: true)
        arc.closeSubpath()
        path.addPath(arc)
        return path
    }
}
