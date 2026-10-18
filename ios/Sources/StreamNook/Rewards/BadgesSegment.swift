import SNKit
import SwiftUI

/// The Badges segment: what the account has collected, what it is missing that it can earn
/// right now, the sort, and the wall of every global Twitch badge.
struct BadgesSegment: View {
    let model: AppModel
    @State private var latch = SettleLatch()
    @State private var warmOnAppear = false

    private var rewards: RewardsStore { model.rewards }

    var body: some View {
        @Bindable var rewards = rewards
        // A stable container, not a Group: on a Group the modifiers below ride each branch, so
        // every Loading -> wall switch re-ran the forced load and looped.
        VStack(alignment: .leading, spacing: 0) {
            if let wall = rewards.wall, !wall.tiles.isEmpty {
                content(wall)
            } else if rewards.wallLoading || rewards.wall?.loading ?? true {
                stateLine("Loading badges…")
            } else {
                stateLine("No badges available right now.")
            }
        }
        .sheet(item: $rewards.badgeSheet) { tile in
            BadgeSheet(model: model, tile: tile) { rewards.badgeSheet = nil }
        }
        .onAppear {
            warmOnAppear = !(rewards.wall?.tiles.isEmpty ?? true)
            Task { await rewards.loadWall(force: true) }
        }
        .onChange(of: rewards.badgeSort) { Task { await rewards.loadWall(force: false) } }
        .onChange(of: model.signIn.user?.id) { Task { await rewards.loadWall(force: true) } }
    }

    private func stateLine(_ text: String) -> some View {
        Text(text)
            .font(.sn(14))
            .foregroundStyle(SN.textMuted)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 40)
    }

    @ViewBuilder
    private func content(_ wall: BadgeWall) -> some View {
        let key = "badges:\(rewards.badgeSort.rawValue)"
        let settling = latch.playing(key) && !(warmOnAppear && rewards.badgeSort == .newest)
        VStack(alignment: .leading, spacing: 0) {
            MissingStrip(missing: wall.missing, expanded: rewards.missingExpanded,
                         toggle: { withAnimation(SNMotion.run(SNMotion.settle)) { rewards.missingExpanded.toggle() } },
                         open: rewards.openMissing)
                .padding(.top, 8)

            HStack(alignment: .center, spacing: 8) {
                VStack(alignment: .leading, spacing: 3) {
                    SectionLabel(text: "All badges")
                    HStack(spacing: 6) {
                        Text(wall.countLabel)
                        if wall.syncLeft > 0 {
                            // The metadata backfill, in the strip's own "checking" language.
                            Circle().fill(SN.success).frame(width: 5, height: 5).modifier(Pulse())
                            Text("Updating badges")
                        }
                    }
                    .font(.snCaption)
                    .foregroundStyle(SN.textMuted)
                }
                Spacer(minLength: 8)
                SortMenu(selection: Binding(get: { rewards.badgeSort }, set: { rewards.badgeSort = $0 }))
            }
            .padding(.bottom, 12)

            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 3), spacing: 8) {
                ForEach(Array(wall.tiles.enumerated()), id: \.element.key) { index, tile in
                    Button { rewards.badgeSheet = tile } label: { BadgeTileView(tile: tile) }
                        .buttonStyle(CardPress())
                        .settleIn(index: index, active: settling)
                }
            }
        }
        .settles(latch, key: key, ready: true)
    }
}

/// One badge on the wall: its art, title and earn status.
private struct BadgeTileView: View {
    let tile: BadgeTile

    var body: some View {
        VStack(spacing: 6) {
            RewardArt(url: tile.image, width: 44, height: 44, radius: 0, fit: true, placeholder: nil)
                .grayscale(tile.lit ? 0 : 1)
                .opacity(tile.lit ? 1 : 0.45)
            Text(tile.title)
                .font(.sn(10.5))
                .foregroundStyle(tile.lit ? SN.textPrimary : SN.textMuted)
                .multilineTextAlignment(.center)
                .lineLimit(2, reservesSpace: true)
                .lineSpacing(-1)
                .frame(maxWidth: .infinity)
            Text(tile.word.isEmpty ? " " : tile.word)
                .font(.sn(9.5, 600))
                .foregroundStyle(BadgeWords.color(tile))
                .lineLimit(1)
        }
        .padding(8)
        .frame(maxWidth: .infinity, minHeight: 112, alignment: .top)
        .snPanel()
        .overlay {
            if let ring = BadgeWords.ring(tile) {
                RoundedRectangle(cornerRadius: 12, style: .continuous).strokeBorder(ring, lineWidth: 1)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(tile.word.isEmpty ? tile.title : "\(tile.title), \(tile.word.capitalized)")
        .accessibilityAddTraits(.isButton)
    }
}

/// The colors a status word and a tile's ring take.
@MainActor
enum BadgeWords {
    static func color(_ tile: BadgeTile) -> Color {
        if tile.owned { return SN.accent }
        switch tile.status {
        case "available": return SN.success
        case "coming-soon": return SN.warning
        default: return SN.textMuted
        }
    }

    static func ring(_ tile: BadgeTile) -> Color? {
        if tile.owned { return SN.accent.opacity(0.6) }
        if tile.status == "available" { return SN.success.opacity(0.7) }
        return nil
    }
}

/// The wall's five sorts as a system menu behind a pill naming the current one.
private struct SortMenu: View {
    @Binding var selection: BadgeSort

    var body: some View {
        Menu {
            Picker("Sort", selection: $selection) {
                ForEach(BadgeSort.allCases, id: \.self) { Text($0.label).tag($0) }
            }
        } label: {
            HStack(spacing: 5) {
                Text(selection.label)
                Image(systemName: "chevron.down").font(.system(size: 10, weight: .semibold))
            }
            .font(.sn(12.5, 600))
            .foregroundStyle(SN.textSecondary)
            .padding(.horizontal, 12)
            .frame(height: 30)
            .snGlass(Capsule())
            .contentShape(Capsule())
        }
        .accessibilityLabel("Sort badges, \(selection.label)")
    }
}

/// "Missing, earnable now": the badges the account can still earn, soonest closing first.
private struct MissingStrip: View {
    let missing: MissingNow
    let expanded: Bool
    let toggle: () -> Void
    let open: (String) -> Void

    var body: some View {
        if missing.state != "hidden" {
            VStack(alignment: .leading, spacing: 0) {
                header.padding(.bottom, 10)
                switch missing.state {
                case "loading":
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(SN.surface)
                        .frame(height: 52)
                        .modifier(Pulse())
                case "failed":
                    note("Couldn't read your badges from Twitch. Pull to try again.")
                case "none":
                    note("You have every badge that's earnable right now.")
                default:
                    list
                }
            }
            .padding(.bottom, 24)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text("Missing, earnable now")
                    .font(.sn(16, 600))
                    .tracking(-0.2)
                    .foregroundStyle(SN.textPrimary)
                if missing.state == "list" {
                    Text("\(missing.rows.count)")
                        .font(.sn(11, 600))
                        .monospacedDigit()
                        .foregroundStyle(SN.success)
                        .padding(.horizontal, 7)
                        .frame(height: 20)
                        .background(Capsule().fill(SN.success.opacity(0.16)))
                }
            }
            if missing.refreshing {
                HStack(spacing: 6) {
                    Circle().fill(SN.success).frame(width: 5, height: 5).modifier(Pulse())
                    Text("Checking Twitch")
                }
                .font(.sn(11.5))
                .foregroundStyle(SN.textMuted)
            } else if !missing.closes.isEmpty {
                (Text("The next one closes ") + Text(missing.closes).foregroundStyle(SN.textSecondary).fontWeight(.medium))
                    .font(.sn(11.5))
                    .foregroundStyle(SN.textMuted)
            }
        }
    }

    private func note(_ text: String) -> some View {
        Text(text).font(.sn(12.5)).foregroundStyle(SN.textMuted).fixedSize(horizontal: false, vertical: true)
    }

    private var list: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let catchUp = missing.catchUp {
                CatchUpPanel(view: catchUp).padding(.bottom, 4)
            }
            ForEach(expanded ? missing.rows : Array(missing.rows.prefix(4))) { row in
                Button { open(row.key) } label: { MissingRowView(row: row) }
                    .buttonStyle(CardPress())
            }
            if missing.rows.count > 4 {
                Button(action: toggle) {
                    Text(expanded ? "Show fewer" : "Show all \(missing.rows.count)")
                        .font(.sn(12, 500))
                        .foregroundStyle(SN.accent)
                        .padding(.vertical, 8)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
    }
}

/// What earning everything in the strip takes. Its note says how the totals are counted.
private struct CatchUpPanel: View {
    let view: CatchUpView
    @State private var notesShown = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 4) {
                Text(view.label.uppercased())
                    .font(.sn(10.5, 600))
                    .tracking(0.5)
                    .foregroundStyle(SN.textSecondary)
                Spacer(minLength: 4)
                Button { notesShown = true } label: {
                    Image(systemName: "info.circle")
                        .font(.system(size: 13))
                        .foregroundStyle(SN.textMuted)
                        .frame(width: 28, height: 22)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("How this is counted")
                .popover(isPresented: $notesShown) {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(view.notes, id: \.self) { Text($0).font(.sn(12.5)).foregroundStyle(SN.textSecondary) }
                    }
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(14)
                    .frame(width: 280)
                    .presentationCompactAdaptation(.popover)
                }
            }
            HStack(spacing: 0) {
                ForEach(view.stats, id: \.self) { stat in
                    VStack(spacing: 1) {
                        Text(stat.value).font(.sn(16, 600)).monospacedDigit().foregroundStyle(SN.textPrimary)
                        Text(stat.label).font(.sn(10.5)).foregroundStyle(SN.textSecondary)
                    }
                    .frame(maxWidth: .infinity)
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .padding(.bottom, 10)
        .snPanel()
        .accessibilityElement(children: .combine)
    }
}

/// A badge the account can earn now: art, title, how it is earned, and time left.
private struct MissingRowView: View {
    let row: MissingRow

    var body: some View {
        HStack(spacing: 12) {
            RewardArt(url: row.image, width: 36, height: 36, radius: 0, fit: true, placeholder: nil)
            VStack(alignment: .leading, spacing: 3) {
                // The words of the pill go first, then the name ends in "…": nothing leaves the card.
                ViewThatFits(in: .horizontal) {
                    titleLine(iconsOnly: row.iconsOnly)
                    titleLine(iconsOnly: true)
                }
                if row.randomNote != nil || row.channelsNote != nil {
                    HStack(spacing: 8) {
                        if let random = row.randomNote {
                            Label(random, systemImage: Symbol.dice)
                                .labelStyle(TightLabel())
                                .fixedSize()
                        }
                        // The campaign counts only on these channels.
                        if let channels = row.channelsNote {
                            Label(channels, systemImage: Symbol.broadcast)
                                .labelStyle(TightLabel())
                                .lineLimit(1)
                        }
                    }
                    .font(.sn(11))
                    .foregroundStyle(SN.textMuted)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if !row.timeLeft.isEmpty {
                Text(row.timeLeft)
                    .font(.sn(11, 500))
                    .foregroundStyle(row.urgent == true ? SN.warning : SN.textSecondary)
                    .fixedSize()
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
        .snPanel()
        .accessibilityElement(children: .combine)
    }

    private func titleLine(iconsOnly: Bool) -> some View {
        HStack(spacing: 6) {
            Text(row.title)
                .font(.sn(13, 500))
                .foregroundStyle(SN.textPrimary)
                .lineLimit(1)
            EarnPill(chips: row.chips, iconsOnly: iconsOnly)
        }
    }
}

/// How a badge is earned, as one glazed pill: a segment per step, hairlines between.
struct EarnPill: View {
    let chips: [EarnChip]
    let iconsOnly: Bool

    var body: some View {
        if !chips.isEmpty {
            HStack(spacing: 6) {
                ForEach(Array(chips.enumerated()), id: \.offset) { index, chip in
                    if index > 0 { Rectangle().fill(.white.opacity(0.15)).frame(width: 1, height: 12) }
                    HStack(spacing: 4) {
                        EarnIcon(chip: chip)
                        if !iconsOnly { Text(chip.text) }
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(chip.text)
                    .accessibilityHint(chip.hint)
                }
            }
            .font(.sn(11, 500))
            .foregroundStyle(SN.textSecondary)
            .lineLimit(1)
            .padding(.horizontal, 8)
            .frame(height: 22)
            .background(Capsule().fill(.white.opacity(0.10)))
            .fixedSize()
        }
    }
}

/// Twitch's own art for the kind when given, else a drawn icon tinted per kind.
private struct EarnIcon: View {
    let chip: EarnChip

    var body: some View {
        if let url = chip.iconURL {
            RewardArt(url: url, width: 14, height: 14, radius: 0, fit: true, placeholder: nil)
        } else {
            Image(systemName: symbol)
                .font(.system(size: 10.5, weight: .semibold))
                .foregroundStyle(tint)
        }
    }

    private var symbol: String {
        switch chip.kind {
        case "watch": "eye"
        case "purchase": chip.ticket ? "ticket" : "dollarsign.circle"
        case "create": "film"
        case "attend": "mappin"
        default: "sparkles"
        }
    }

    private var tint: Color {
        switch chip.kind {
        case "watch": Color(red: 0.49, green: 0.83, blue: 0.99)
        case "subscribe": Color(red: 0.77, green: 0.71, blue: 0.99)
        case "purchase": Color(red: 0.99, green: 0.83, blue: 0.30)
        case "cheer": Color(red: 0.94, green: 0.67, blue: 0.99)
        case "create": Color(red: 0.99, green: 0.64, blue: 0.69)
        case "attend": Color(red: 0.43, green: 0.91, blue: 0.72)
        default: SN.textSecondary
        }
    }
}

/// An icon and its words, 3 points apart.
struct TightLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 3) {
            configuration.icon
            configuration.title
        }
    }
}

/// A slow breathing opacity for loading placeholders and the "checking" dot.
private struct Pulse: ViewModifier {
    @Environment(\.snStill) private var still
    @State private var dim = false

    func body(content: Content) -> some View {
        content
            .opacity(dim ? 0.45 : 1)
            .onAppear {
                guard !still else { return }
                withAnimation(.easeInOut(duration: 0.7).repeatForever(autoreverses: true)) { dim = true }
            }
    }
}
