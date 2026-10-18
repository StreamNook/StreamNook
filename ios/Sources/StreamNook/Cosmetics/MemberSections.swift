import SNKit
import SwiftUI

/// A StreamNook member's overview, top emotes and accolades on their profile sheet, as the core
/// shaped them (what they hide is already gone). Their StreamNook badges and relics ride the
/// sheet's badge groups. Android draws the same answer in `MemberSections.tsx`.
struct MemberSections: View {
    let profile: MemberProfile
    /// Emotes whose every image failed; they take themselves out, as a badge does.
    @State private var failed: Set<String> = []

    private var shownEmotes: [TopEmote] { profile.topEmotes.filter { !failed.contains($0.id) } }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            if !profile.stats.isEmpty { overview }
            if !shownEmotes.isEmpty { emotes }
            if !profile.accolades.isEmpty { accolades }
        }
    }

    /// The tiles two to a row; an odd last tile keeps its half.
    private var overview: some View {
        VStack(alignment: .leading, spacing: 8) {
            ProfileSectionHeader("Overview")
            Grid(horizontalSpacing: 8, verticalSpacing: 8) {
                ForEach(Array(stride(from: 0, to: profile.stats.count, by: 2)), id: \.self) { start in
                    GridRow {
                        MemberStatTile(stat: profile.stats[start])
                        if start + 1 < profile.stats.count {
                            MemberStatTile(stat: profile.stats[start + 1])
                        } else {
                            Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
                        }
                    }
                }
            }
        }
    }

    private var emotes: some View {
        VStack(alignment: .leading, spacing: 8) {
            ProfileSectionHeader("Top emotes")
            FlowLayout(spacing: 8) {
                ForEach(shownEmotes) { emote in
                    CosmeticImage(emote.url, animate: true, onFailure: { failed.insert(emote.id) })
                        .frame(width: 28, height: 28)
                        .frame(width: 32, height: 32)
                        .accessibilityLabel("\(emote.name), used \(emote.count) times")
                }
            }
        }
    }

    private var accolades: some View {
        VStack(alignment: .leading, spacing: 8) {
            ProfileSectionHeader("Accolades", detail: "\(profile.accolades.count)")
            // Five to a row, as on Android.
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8, alignment: .top), count: 5),
                      alignment: .leading, spacing: 12) {
                ForEach(profile.accolades) { AccoladeMedallion(medal: $0) }
            }
        }
    }
}

/// One overview tile: a colored mark and the label, the value, then its caption.
private struct MemberStatTile: View {
    let stat: MemberStat

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 6) {
                // One width for every symbol, so the labels start on one line.
                Image(systemName: Self.symbol[stat.key] ?? "chart.bar")
                    .font(.system(size: 11, weight: .medium))
                    .foregroundStyle(nameColor(Self.tint[stat.key] ?? "#aebfca"))
                    .frame(width: 14, alignment: .center)
                Text(stat.label).font(.sn(12)).foregroundStyle(SN.textSecondary).lineLimit(1)
            }
            Text(stat.value)
                .font(.sn(17, 700).monospacedDigit())
                .foregroundStyle(SN.textPrimary)
                .lineLimit(1)
                .minimumScaleFactor(0.6)
            // Every tile keeps the caption line, so the grid is one even block; the caption
            // hugs its value.
            Text(stat.caption ?? " ")
                .font(.sn(12))
                .foregroundStyle(SN.textMuted)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .opacity(stat.caption == nil ? 0 : 1)
                .accessibilityHidden(stat.caption == nil)
                .padding(.top, -3)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .padding(12)
        .snPanel()
        .accessibilityElement(children: .combine)
    }

    static let symbol: [String: String] = [
        "hours": "clock", "messages": "bubble.left", "streams": "tv", "favorite_channel": "heart",
        "subs_gifted": "gift", "twitch_age": "birthday.cake", "followers": "person.2", "views": "eye",
    ]

    /// The desktop card's icon colors.
    static let tint: [String: String] = [
        "hours": "#e6c178", "messages": "#8fd4b4", "streams": "#8fc3e8", "favorite_channel": "#e493a0",
        "subs_gifted": "#b9a6ec", "twitch_age": "#e493a0", "followers": "#8fc3e8", "views": "#aebfca",
    ]
}

/// The desktop's earned medallion: its two-stop gradient lit from the top left, a bevel of
/// light and shade at the rim (no outer glow), the icon in white, the name under it.
private struct AccoladeMedallion: View {
    let medal: AccoladeMedal

    var body: some View {
        VStack(spacing: 4) {
            ZStack {
                Circle().fill(LinearGradient(colors: medal.colors.map { nameColor($0) },
                                             startPoint: .topLeading, endPoint: .bottomTrailing))
                Circle().strokeBorder(LinearGradient(colors: [.white.opacity(0.35), .clear, .black.opacity(0.3)],
                                                     startPoint: .topLeading, endPoint: .bottomTrailing),
                                      lineWidth: 1.5)
                Image(systemName: Self.symbol[medal.icon] ?? "star")
                    .font(.system(size: 18, weight: .semibold))
                    .foregroundStyle(.white)
                    .shadow(color: .black.opacity(0.35), radius: 0.5, y: 1)
            }
            .frame(width: 44, height: 44)
            Text(medal.label)
                .font(.sn(11, 500))
                .foregroundStyle(SN.textSecondary)
                .multilineTextAlignment(.center)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(medal.label.replacingOccurrences(of: "\u{AD}", with: ""))
    }

    /// The nearest SF Symbol for each of the desktop's lucide icons.
    static let symbol: [String: String] = [
        "message-circle": "bubble.left", "megaphone": "megaphone", "mic": "mic", "timer": "timer",
        "moon": "moon", "armchair": "sofa", "waves": "water.waves", "ghost": "eye.slash",
        "package": "shippingbox", "flame": "flame", "piggy-bank": "banknote", "gem": "diamond",
        "shapes": "square.on.circle", "droplets": "drop", "medal": "medal", "hourglass": "hourglass",
        "crown": "crown", "star": "star", "layers": "square.3.layers.3d", "repeat": "repeat",
        "smile": "face.smiling", "bell-ring": "bell", "sparkles": "sparkles", "target": "target",
        "party-popper": "party.popper", "heart": "heart", "clover": "leaf", "laugh": "face.smiling.inverse",
        "sword": "bolt", "skull": "theatermasks", "drumstick": "fork.knife", "tree-pine": "tree",
        "flower-2": "camera.macro", "sun": "sun.max", "leaf": "leaf", "snowflake": "snowflake",
        "cake": "birthday.cake", "trophy": "trophy", "flag": "flag",
    ]
}
