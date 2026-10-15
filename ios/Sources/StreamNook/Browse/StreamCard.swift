import SNKit
import SwiftUI

/// A live stream in a list, in StreamNook's card language: a media card holding the preview with
/// its chips (LIVE, DROPS, the hype train, viewers, the watch streak), then the title, the channel
/// and who it streams with, and the category. The row variant puts a smaller preview beside the
/// text. Long press opens the card's menu.
struct StreamCard: View {
    let stream: TwitchStream
    var variant = StreamView.cards
    var hypeTrain: HypeTrain?
    var streak: Int?
    var group: ChannelGroup?
    var stamp = 0
    let open: () -> Void
    var openCategory: (() -> Void)?

    var body: some View {
        Button(action: open) {
            Group {
                if variant == .list { row } else { card }
            }
            .padding(8)
            .snMediaCard()
            .contentShape(RoundedRectangle(cornerRadius: SNRadius.card, style: .continuous))
        }
        .buttonStyle(CardPress())
        .contextMenu { StreamMenu(stream: stream, open: open, openCategory: openCategory) }
        .accessibilityElement(children: .combine)
        .accessibilityHint("Opens the stream")
    }

    // MARK: Card

    private var card: some View {
        VStack(alignment: .leading, spacing: 8) {
            Preview(url: TwitchImage.preview(stream.thumbnailURL, stamp: stamp))
                .overlay(alignment: .topLeading) {
                    HStack(spacing: 4) {
                        Chip(kind: .live) {
                            LiveDot()
                            Text("LIVE")
                        }
                        if stream.dropsEarnable == true { DropsChip() }
                        if let hypeTrain { HypeChip(train: hypeTrain) }
                    }
                    .padding(6)
                }
                .overlay(alignment: .bottomLeading) {
                    Chip(kind: .neutral) {
                        Image(systemName: Symbol.viewers).font(.system(size: 10, weight: .bold)).opacity(0.8)
                        Text(stream.viewerCount.formatted()).monospacedDigit()
                    }
                    .padding(6)
                    .accessibilityLabel("\(stream.viewerCount.formatted()) viewers")
                }
                .overlay(alignment: .bottomTrailing) {
                    if let streak { StreakChip(days: streak).padding(6) }
                }
            VStack(alignment: .leading, spacing: 2) {
                Text(stream.title)
                    .font(.snCardTitle)
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 4) {
                    if let avatar = stream.profileImageURL.flatMap(URL.init(string:)) {
                        AsyncImage(url: avatar) { image in
                            image.resizable().scaledToFill()
                        } placeholder: {
                            SN.surface
                        }
                        .frame(width: 16, height: 16)
                        .clipShape(Circle())
                        .overlay(Circle().stroke(SN.borderSubtle, lineWidth: 1))
                    }
                    Text(stream.userName).lineLimit(1)
                    if let group { TogetherTag(group: group) }
                    if stream.isPartner { PartnerMark(size: 12) }
                }
                .font(.snCardLine)
                .foregroundStyle(SN.textSecondary)
                Text(stream.gameName.isEmpty ? " " : stream.gameName)
                    .font(.snCardLine)
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
                    .frame(minHeight: 16)
            }
            .padding(.horizontal, 2)
            .padding(.bottom, 2)
        }
    }

    // MARK: Row

    private var row: some View {
        HStack(spacing: 10) {
            Preview(url: TwitchImage.preview(stream.thumbnailURL, stamp: stamp))
                .frame(width: 156)
                .overlay(alignment: .topLeading) { PingDot().padding(6) }
                .overlay(alignment: .topTrailing) {
                    if stream.dropsEarnable == true {
                        Chip(kind: .drops, compact: true) {
                            Image(systemName: Symbol.drops).font(.system(size: 8, weight: .bold))
                        }
                        .padding(4)
                        .accessibilityLabel("Drops enabled")
                    }
                }
                .overlay(alignment: .bottomLeading) {
                    if let hypeTrain {
                        Chip(kind: hypeTrain.isGolden ? .golden : .hype, compact: true) {
                            Image(systemName: Symbol.hypeTrain).font(.system(size: 7, weight: .bold))
                            Text("\(hypeTrain.level)")
                        }
                        .padding(4)
                    }
                }
            VStack(alignment: .leading, spacing: 2) {
                Text(stream.title)
                    .font(.snCardTitle)
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(2)
                HStack(spacing: 4) {
                    Text(stream.userName).lineLimit(1)
                    if let group { TogetherTag(group: group, compact: true) }
                    if stream.isPartner { PartnerMark(size: 10) }
                }
                .font(.sn(12))
                .foregroundStyle(SN.textSecondary)
                if !stream.gameName.isEmpty {
                    Text(stream.gameName).font(.sn(12)).foregroundStyle(SN.textMuted).lineLimit(1)
                }
                HStack(spacing: 6) {
                    Text("\(stream.viewerCount.formatted()) viewers")
                        .font(.sn(11.5))
                        .foregroundStyle(SN.textMuted)
                    if let streak { StreakChip(days: streak) }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

/// A card's long-press menu: watch, the category, and sharing the channel.
struct StreamMenu: View {
    let stream: TwitchStream
    let open: () -> Void
    var openCategory: (() -> Void)?

    private var link: URL { URL(string: "https://www.twitch.tv/\(stream.userLogin)")! }

    var body: some View {
        MenuPick("Watch", symbol: Symbol.watch, action: open)
        if let openCategory, !stream.gameName.isEmpty {
            MenuPick("Open \(stream.gameName)", symbol: Symbol.category, action: openCategory)
        }
        Divider()
        ShareLink(item: link) { Label("Share channel", systemImage: Symbol.share) }
        MenuPick("Copy channel link", symbol: Symbol.copy) { UIPasteboard.general.url = link }
    }

    /// The same items, named, for a drive step's stand-in sheet.
    static func items(_ stream: TwitchStream, category: Bool) -> [(label: String, symbol: String)] {
        var items: [(label: String, symbol: String)] = [("Watch", Symbol.watch)]
        if category, !stream.gameName.isEmpty { items.append(("Open \(stream.gameName)", Symbol.category)) }
        items += [("Share channel", Symbol.share), ("Copy channel link", Symbol.copy)]
        return items
    }
}

/// A 16:9 preview, filled and clipped, with the surface showing while it loads.
private struct Preview: View {
    let url: URL?

    var body: some View {
        Color.clear
            .aspectRatio(16 / 9, contentMode: .fit)
            .background {
                AsyncImage(url: url, transaction: Transaction(animation: .easeOut(duration: 0.2))) { phase in
                    if let image = phase.image {
                        image.resizable().scaledToFill()
                    } else {
                        SN.surface
                    }
                }
            }
            .clipShape(RoundedRectangle(cornerRadius: SNRadius.thumb, style: .continuous))
            .accessibilityHidden(true)
    }
}

/// The chip over a preview: 22 points tall, a frosted dark veil, 11 point semibold text, tinted by
/// what it is about.
struct Chip<Content: View>: View {
    enum Kind { case live, neutral, hype, golden, streak, drops }

    let kind: Kind
    var compact = false
    /// Seconds between shine sweeps across the chip; none when nil.
    var shine: Double?
    @ViewBuilder let content: Content

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: compact ? 5 : SNRadius.chip, style: .continuous)
        HStack(spacing: compact ? 2 : 4) { content }
            .font(compact ? .sn(8, 700) : .snChip)
            .tracking(0.2)
            .foregroundStyle(kind == .streak ? FixedColor.streak.color : .white)
            .padding(.horizontal, compact ? 4 : 8)
            .frame(height: compact ? 14 : 22)
            .snChipVeil(shape, density: kind == .neutral ? 0.34 : kind == .streak ? 0.82 : 0.22)
            .background { tint.map { shape.fill($0) } }
            .overlay { tint.map { shape.stroke($0.opacity(0.9), lineWidth: 0.5) } }
            .overlay { if let shine { ShineSweep(period: shine).clipShape(shape) } }
    }

    private var tint: LinearGradient? {
        let colors: [Color]
        switch kind {
        case .neutral: return nil
        case .live: colors = [SN.live.opacity(0.16), SN.live.opacity(0.08)]
        case .hype: colors = [SN.highlightPurple.opacity(0.16), SN.highlightBlue.opacity(0.08)]
        case .golden: colors = [FixedColor.goldenTrain.color.opacity(0.18), FixedColor.goldenTrainEnd.color.opacity(0.1)]
        case .streak: colors = [SN.warning.opacity(0.12), SN.warning.opacity(0.06)]
        case .drops: colors = [SN.accent.opacity(0.16), SN.accent.opacity(0.08)]
        }
        return LinearGradient(colors: colors, startPoint: .topLeading, endPoint: .bottomTrailing)
    }
}

private struct HypeChip: View {
    let train: HypeTrain

    var body: some View {
        Chip(kind: train.isGolden ? .golden : .hype, shine: 2.5) {
            Image(systemName: Symbol.hypeTrain).font(.system(size: 9, weight: .bold))
            Text("LVL \(train.level)")
        }
        .accessibilityLabel("Hype train level \(train.level)")
    }
}

/// Watching this channel earns drops right now.
private struct DropsChip: View {
    var body: some View {
        Chip(kind: .drops, shine: 3) {
            Image(systemName: Symbol.drops).font(.system(size: 9, weight: .bold))
            Text("DROPS")
        }
        .accessibilityLabel("Drops enabled")
    }
}

/// A band of light crossing a chip once every `period` seconds, still when movement is dropped
/// or the player covers the list.
private struct ShineSweep: View {
    let period: Double
    @Environment(\.snStill) private var still
    @Environment(\.shellCovered) private var covered

    /// How long one crossing takes; the rest of the period the chip rests.
    private static let crossing = 0.9

    var body: some View {
        if !still && !covered {
            GeometryReader { geo in
                let width = geo.size.width
                let rest = max(period - Self.crossing, 0)
                LinearGradient(colors: [.clear, .white.opacity(0.32), .clear], startPoint: .leading, endPoint: .trailing)
                    .frame(width: width * 0.6)
                    .keyframeAnimator(initialValue: -0.6, repeating: true) { band, x in
                        band.offset(x: x * width)
                    } keyframes: { _ in
                        KeyframeTrack {
                            LinearKeyframe(-0.6, duration: rest)
                            CubicKeyframe(1.0, duration: Self.crossing)
                        }
                    }
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}

/// Who a channel is live with, beside its name: up to three faces of the others (then "+N"),
/// then "Together" for Shared Viewership or a chat mark and "Shared chat" for Shared Chat. The
/// compact form keeps the faces only. The card stays the tap target.
struct TogetherTag: View {
    let group: ChannelGroup
    var compact = false

    private static let faces = 3

    var body: some View {
        let others = group.others
        let extra = others.count - Self.faces
        HStack(spacing: 6) {
            if !group.together {
                Image(systemName: "bubble.left.and.bubble.right.fill")
                    .font(.system(size: 11))
                    .foregroundStyle(SN.textSecondary)
                    .padding(.leading, 4)
            }
            HStack(spacing: -4) {
                ForEach(others.prefix(Self.faces), id: \.userID) { member in
                    AsyncImage(url: member.avatarURL.flatMap(URL.init(string:))) { image in
                        image.resizable().scaledToFill()
                    } placeholder: {
                        SN.surface
                    }
                    .frame(width: 18, height: 18)
                    .clipShape(Circle())
                }
                if extra > 0 {
                    Text("+\(extra)")
                        .font(.sn(9, 600))
                        .monospacedDigit()
                        .foregroundStyle(SN.textPrimary)
                        .padding(.horizontal, 4)
                        .frame(minWidth: 18, minHeight: 18)
                        .background(Capsule().fill(.white.opacity(0.15)))
                }
            }
            if !compact {
                Text(group.together ? "Together" : "Shared chat")
                    .font(.sn(11, 600))
                    .foregroundStyle(SN.textPrimary)
            }
        }
        .padding(.leading, 2)
        .padding(.trailing, compact ? 2 : 8)
        .frame(height: 22)
        .background(Capsule().fill(.white.opacity(0.12)))
        .padding(.vertical, -3)
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(group.label)
    }
}

private struct StreakChip: View {
    let days: Int

    var body: some View {
        Chip(kind: .streak) {
            Image(systemName: Symbol.streak).font(.system(size: 9, weight: .bold))
            Text("\(days)")
        }
        .accessibilityLabel("Watch streak \(days)")
    }
}

/// The partner verified mark in Twitch purple.
struct PartnerMark: View {
    var size: CGFloat = 12

    var body: some View {
        Image(systemName: Symbol.partner)
            .font(.system(size: size))
            .foregroundStyle(SN.twitch)
            .accessibilityLabel("Partner")
    }
}

/// The row's bare live dot with a halo pinging out of it, still when movement is dropped or the
/// player covers the list.
private struct PingDot: View {
    @Environment(\.snStill) private var still
    @Environment(\.shellCovered) private var covered
    @State private var ping = false

    var body: some View {
        ZStack {
            if !still && !covered {
                Circle()
                    .fill(SN.live.opacity(0.6))
                    .scaleEffect(ping ? 2 : 1)
                    .opacity(ping ? 0 : 1)
                    .onAppear {
                        withAnimation(.easeOut(duration: 1).repeatForever(autoreverses: false)) { ping = true }
                    }
                    .onDisappear { ping = false }
            }
            Circle()
                .fill(SN.live)
                .overlay(Circle().stroke(Color.black.opacity(0.4), lineWidth: 1))
        }
        .frame(width: 8, height: 8)
        .accessibilityLabel("Live")
    }
}
