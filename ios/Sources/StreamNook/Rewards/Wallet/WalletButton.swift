import SNKit
import SwiftUI

/// The channel wallet's button inside the composer field, for a Twitch room with points or bits:
/// the channel's own points art (or Twitch's glyph), split on the diagonal with the Bits gem when
/// bits are on too, a dot
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
                        .snSheetLight()
                }
        }
    }

    private func button(_ wallet: WalletModel) -> some View {
        Button {
            onOpen()
            if wallet.chestWaiting { Task { await wallet.claimChest(channelID: session.roomID) } }
            wallet.sheetOpen = true
        } label: {
            HStack(spacing: 4) {
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
                }
                ForEach(Array(shownBalances(wallet).enumerated()), id: \.offset) { index, balance in
                    if index > 0 {
                        Rectangle()
                            .fill(SN.textSecondary.opacity(0.35))
                            .frame(width: 1, height: 12)
                            .accessibilityHidden(true)
                    }
                    Text(balance)
                        .font(.sn(12, 600))
                        .monospacedDigit()
                        .foregroundStyle(SN.textPrimary)
                        .lineLimit(1)
                        .fixedSize()
                }
            }
            .padding(.horizontal, shownBalances(wallet).isEmpty ? 0 : 4)
            .frame(minWidth: 32, minHeight: 36)
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
        .onChange(of: session.style.walletGainsChestOnly, initial: true) { _, only in wallet.chestOnly = only }
        .accessibilityLabel(label(wallet))
        .accessibilityHint(wallet.chestWaiting ? "Collects the bonus chest and opens the wallet" : "Opens the wallet")
    }

    @ViewBuilder
    private func face(_ wallet: WalletModel) -> some View {
        if wallet.chestWaiting {
            // The whole button becomes the chest, like the desktop's, never a mark on a corner.
            WalletChest()
        } else if let points = wallet.points, let gem = wallet.bits?.gemURL {
            SplitWalletGlyph(pointsIconURL: points.iconURL, gemURL: gem, layout: wallet.split)
        } else if let points = wallet.points {
            PointsArt(iconURL: points.iconURL, size: 19)
        } else if let gem = wallet.bits?.gemURL {
            RewardArt(url: gem, width: 19, height: 19, radius: 0, fit: true, placeholder: nil)
        }
    }

    /// The short balances beside the icon while Wallet balance is Always and the sheet is closed:
    /// points, then Bits, in the glyph's own order.
    private func shownBalances(_ wallet: WalletModel) -> [String] {
        guard session.style.walletBalanceShown, !wallet.sheetOpen, !wallet.chestWaiting else { return [] }
        let short = { (n: Int64) in n.formatted(.number.notation(.compactName)) }
        return [wallet.points?.balance, wallet.bits?.balance].compactMap { $0 }.map(short)
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

/// The wallet's mark for a channel with both channel points and Bits: one square split on the
/// diagonal, the points art in the upper-left half and the Bits gem in the lower-right, both whole
/// and the same size, with a hairline across the gap between them. Nothing sits on a corner, so it
/// never reads as an unread badge. Where each part goes is the core's (`wallet_split_layout`): it
/// measures the channel's art, so round, full-bleed and padded art all sit the same gap from the
/// gem. Same layout as the desktop and Android `SplitWalletGlyph`.
struct SplitWalletGlyph: View {
    let pointsIconURL: String?
    let gemURL: String
    var layout: SplitLayout = .fallback
    var size: CGFloat = 24

    var body: some View {
        let u = size / CGFloat(layout.grid)
        let mark = CGFloat(layout.mark) * u
        ZStack(alignment: .topLeading) {
            PointsArt(iconURL: pointsIconURL, size: mark)
                .offset(x: CGFloat(layout.artAt) * u, y: CGFloat(layout.artAt) * u)
            RewardArt(url: gemURL, width: mark, height: mark, radius: 0, fit: true, placeholder: nil)
                .offset(x: CGFloat(layout.gemAt) * u, y: CGFloat(layout.gemAt) * u)
            Path { path in
                path.move(to: CGPoint(x: CGFloat(layout.x1) * u, y: CGFloat(layout.y1) * u))
                path.addLine(to: CGPoint(x: CGFloat(layout.x2) * u, y: CGFloat(layout.y2) * u))
            }
            .stroke(SN.textSecondary.opacity(0.45), lineWidth: 1)
            .accessibilityHidden(true)
        }
        .frame(width: size, height: size, alignment: .topLeading)
        .offset(x: CGFloat(layout.shiftX) * u, y: CGFloat(layout.shiftY) * u)
    }
}

/// A bonus chest waiting: the chest in the theme's warning amber with a soft glow, and every few
/// seconds the desktop's wiggle (rotation and scale only, so it stays on the compositor), held
/// still under Reduce Motion or the Motion setting.
struct WalletChest: View {
    @Environment(\.snStill) private var still

    var body: some View {
        let chest = ChestGlyph()
            .fill(style: FillStyle(eoFill: true))
            .foregroundStyle(SN.warning)
            .frame(width: 19, height: 19)
            .shadow(color: SN.warning.opacity(0.4), radius: 1.5)
            .accessibilityHidden(true)
        if still {
            chest
        } else {
            chest.keyframeAnimator(initialValue: Wiggle(), repeating: true) { content, w in
                content.scaleEffect(w.scale).rotationEffect(.degrees(w.angle))
            } keyframes: { _ in
                // The desktop's chest-attention: 3.4 s, still for the first 70%.
                KeyframeTrack(\.angle) {
                    LinearKeyframe(0, duration: 2.38)
                    CubicKeyframe(-13, duration: 0.136)
                    CubicKeyframe(11, duration: 0.17)
                    CubicKeyframe(-8, duration: 0.17)
                    CubicKeyframe(5, duration: 0.17)
                    CubicKeyframe(0, duration: 0.17)
                    LinearKeyframe(0, duration: 0.204)
                }
                KeyframeTrack(\.scale) {
                    LinearKeyframe(1, duration: 2.38)
                    CubicKeyframe(1.14, duration: 0.136)
                    CubicKeyframe(1.12, duration: 0.17)
                    CubicKeyframe(1.08, duration: 0.17)
                    CubicKeyframe(1.04, duration: 0.17)
                    CubicKeyframe(1, duration: 0.17)
                    LinearKeyframe(1, duration: 0.204)
                }
            }
        }
    }

    private struct Wiggle {
        var angle = 0.0
        var scale = 1.0
    }
}

/// The bonus chest on a 24 point grid (the desktop's chest glyph), filled even-odd.
struct ChestGlyph: Shape {
    func path(in rect: CGRect) -> Path {
        let s = min(rect.width, rect.height) / 24
        let o = CGPoint(x: rect.midX - 12 * s, y: rect.midY - 12 * s)
        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: o.x + x * s, y: o.y + y * s) }
        var path = Path()
        // The chest's outline, its top corners rounded.
        path.move(to: p(2, 22))
        path.addLine(to: p(2, 13))
        path.addLine(to: p(4.417, 8.971))
        path.addQuadCurve(to: p(6.132, 8), control: p(5, 8))
        path.addLine(to: p(17.868, 8))
        path.addQuadCurve(to: p(19.583, 8.971), control: p(19, 8))
        path.addLine(to: p(22, 13))
        path.addLine(to: p(22, 22))
        path.closeSubpath()
        // The body, with the latch notch.
        path.move(to: p(20, 20))
        path.addLine(to: p(20, 15))
        path.addLine(to: p(13, 15))
        path.addLine(to: p(13, 17))
        path.addLine(to: p(11, 17))
        path.addLine(to: p(11, 15))
        path.addLine(to: p(4, 15))
        path.addLine(to: p(4, 20))
        path.closeSubpath()
        // The lid's band.
        path.move(to: p(17.868, 10))
        path.addLine(to: p(19.668, 13))
        path.addLine(to: p(4.332, 13))
        path.addLine(to: p(6.132, 10))
        path.closeSubpath()
        return path
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
