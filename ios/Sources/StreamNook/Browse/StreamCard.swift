import SNKit
import SwiftUI

/// A live stream in a list, in StreamNook's card language: a media card holding the preview with
/// its chips (LIVE, the hype train, viewers, the watch streak), then the title, the channel and
/// the category. The row variant puts a smaller preview beside the text. Long press opens the
/// card's menu.
struct StreamCard: View {
    let stream: TwitchStream
    var variant = StreamView.cards
    var hypeTrain: HypeTrain?
    var streak: Int?
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
        Chip(kind: train.isGolden ? .golden : .hype) {
            Image(systemName: Symbol.hypeTrain).font(.system(size: 9, weight: .bold))
            Text("LVL \(train.level)")
        }
        .accessibilityLabel("Hype train level \(train.level)")
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
