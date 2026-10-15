import SNKit
import SwiftUI

/// The chatter's sub in this room, every word and number from the core. A current sub reads as a
/// glass tile tinted at its edge by the tier: the channel's badge for their tenure, the months as
/// the focal number with the tier beside it, the facts with Twitch's badge for each, and how close
/// the next badge is. Anyone else gets one glass button: the call to gift, with what is known
/// under it. Gift taps open Twitch's gift page in the browser.
struct SubStandingTile: View {
    let standing: SubStanding
    /// The sub or founder badge the row wore, when the core has no art.
    var wornArt: String?
    @Environment(\.openURL) private var openURL
    @State private var filled = false

    private var tint: Color { Color(cgColor: FillPainter.color(standing.tintHex) ?? CGColor(gray: 1, alpha: 1)) }

    var body: some View {
        if standing.state == "current" {
            current
        } else if let label = standing.giftLabel, let url = standing.giftURL {
            giftButton(label: label, url: url)
        }
    }

    private var current: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                Text(standing.title ?? "")
                    .font(.sn(11, 600))
                    .textCase(.uppercase)
                    .tracking(0.8)
                    .foregroundStyle(SN.textSecondary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                if let label = standing.giftLabel, let url = standing.giftURL {
                    Button { open(url) } label: {
                        Image(systemName: "gift")
                            .font(.system(size: 18))
                            .foregroundStyle(SN.accent)
                            .frame(width: 36, height: 36)
                            .contentShape(Circle())
                    }
                    .buttonStyle(BadgePress())
                    .padding(.vertical, -6)
                    .padding(.trailing, -6)
                    .accessibilityLabel(label)
                }
            }
            HStack(spacing: 14) {
                if let art = standing.badgeArt ?? wornArt {
                    CosmeticImage(art, animate: true)
                        .frame(width: 44, height: 44)
                        .shadow(color: .black.opacity(0.45), radius: 3, y: 2)
                } else {
                    Circle().fill(tint).frame(width: 10, height: 10).padding(.horizontal, 6)
                }
                VStack(alignment: .leading, spacing: 6) {
                    if let months = standing.months, let unit = standing.monthsUnit {
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Text("\(months)").font(.sn(30, 700).monospacedDigit()).foregroundStyle(SN.textPrimary)
                            Text(unit).font(.sn(13, 500)).foregroundStyle(SN.textSecondary)
                            if let tier = standing.tierLabel {
                                Text(tier).font(.sn(13, 600)).foregroundStyle(tint).padding(.leading, 4)
                            }
                        }
                    } else if let tier = standing.tierLabel {
                        Text(tier).font(.sn(14, 600)).foregroundStyle(tint)
                    }
                    if !standing.facts.isEmpty {
                        FlowLayout(spacing: 12, lineSpacing: 4) {
                            ForEach(standing.facts, id: \.self) { fact in
                                HStack(spacing: 4) {
                                    factMark(fact)
                                    Text(fact.text)
                                        .font(.sn(12.5, fact.kind == "prime" || fact.kind == "founder" ? 600 : 400))
                                        .foregroundStyle(fact.kind == "prime" || fact.kind == "founder" ? SN.textPrimary : SN.textSecondary)
                                }
                            }
                        }
                    }
                }
                Spacer(minLength: 0)
            }
            .padding(.top, 8)
            if let next = standing.nextBadge {
                VStack(alignment: .leading, spacing: 6) {
                    GeometryReader { geo in
                        ZStack(alignment: .leading) {
                            Capsule().fill(Color.white.opacity(0.08))
                            Capsule()
                                .fill(LinearGradient(colors: [tint.opacity(0.55), tint], startPoint: .leading, endPoint: .trailing))
                                .frame(width: geo.size.width)
                                .scaleEffect(x: filled ? max(0.04, next.progress) : 0, anchor: .leading)
                        }
                    }
                    .frame(height: 3)
                    .clipShape(Capsule())
                    HStack(spacing: 6) {
                        Text(next.text).font(.sn(11.5)).foregroundStyle(SN.textSecondary).lineLimit(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        if let art = next.art {
                            CosmeticImage(art, animate: false).frame(width: 16, height: 16).opacity(0.8)
                        }
                    }
                }
                .padding(.top, 12)
                .onAppear {
                    // One fill on open; the Motion setting stills it.
                    if Look.shared.motion == .full && !UIAccessibility.isReduceMotionEnabled {
                        withAnimation(.easeOut(duration: 0.7)) { filled = true }
                    } else {
                        filled = true
                    }
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.top, 10)
        .padding(.bottom, 12)
        .background { tileBackground }
    }

    /// The glass tile with the tier's light on its edge: a wash from the badge corner and a lit
    /// top rim over a faint tinted ring.
    private var tileBackground: some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        return ZStack {
            Color.clear.snPanel()
            shape.fill(RadialGradient(colors: [tint.opacity(0.2), .clear], center: .topLeading, startRadius: 0, endRadius: 260))
            shape.strokeBorder(tint.opacity(0.18), lineWidth: 1)
            shape.strokeBorder(LinearGradient(stops: [.init(color: tint.opacity(0.5), location: 0),
                                                      .init(color: .clear, location: 0.08)],
                                              startPoint: .top, endPoint: .bottom), lineWidth: 1)
        }
    }

    @ViewBuilder
    private func factMark(_ fact: SubStanding.Fact) -> some View {
        if let art = fact.art {
            CosmeticImage(art, animate: false).frame(width: 17, height: 17)
        } else if fact.kind == "streak" {
            Image(systemName: "flame.fill").font(.system(size: 13)).foregroundStyle(Color.orange).frame(width: 17, height: 17)
        } else if fact.kind == "gift" {
            Image(systemName: "gift").font(.system(size: 13)).foregroundStyle(SN.textSecondary).frame(width: 17, height: 17)
        }
    }

    private func giftButton(label: String, url: String) -> some View {
        let known = [standing.title, standing.detail].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
        return Button { open(url) } label: {
            HStack(spacing: 10) {
                Image(systemName: "gift").font(.system(size: 19)).foregroundStyle(SN.accent)
                VStack(alignment: .leading, spacing: 1) {
                    Text(label).font(.sn(14, 600)).foregroundStyle(SN.textPrimary).lineLimit(1)
                    if !known.isEmpty {
                        Text(known).font(.sn(12.5)).foregroundStyle(SN.textSecondary).lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
                Image(systemName: "chevron.right").font(.system(size: 13, weight: .semibold)).foregroundStyle(SN.accent)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 10)
            .snPanel()
        }
        .buttonStyle(BadgePress())
    }

    private func open(_ url: String) {
        if let url = URL(string: url) { openURL(url) }
    }
}
