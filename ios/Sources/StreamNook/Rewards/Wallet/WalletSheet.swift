import SNKit
import SwiftUI

/// The channel wallet as a sheet: the balance (tap to refresh), the bits held here, and the
/// channel's rewards with Rust's verdict on each. A reward opens its detail, where it is redeemed
/// through Rust; an emote unlock picks its emote first.
struct WalletSheet: View {
    let wallet: WalletModel
    let session: ChatSession
    @State private var menu: RewardsMenuModel
    @State private var path: [WalletRoute] = []

    init(wallet: WalletModel, session: ChatSession) {
        self.wallet = wallet
        self.session = session
        _menu = State(initialValue: RewardsMenuModel(login: wallet.login, core: session.core))
    }

    var body: some View {
        NavigationStack(path: $path) {
            RewardsList(wallet: wallet, menu: menu, session: session) { path.append(.detail($0)) }
                .navigationDestination(for: WalletRoute.self) { route in
                    switch route {
                    case .detail(let reward):
                        RewardDetail(menu: menu, reward: reward, channelID: session.roomID) { picks in
                            path.append(.pick(reward, modify: picks == "modify_emote"))
                        } done: { path.removeAll() }
                    case .pick(let reward, let modify):
                        EmoteUnlockPicker(menu: menu, reward: reward, modify: modify, channelID: session.roomID) {
                            path.removeAll()
                        }
                    }
                }
        }
        .overlay(alignment: .top) { toast }
        .overlay { if let emote = menu.revealed { EmoteReveal(emote: emote) { menu.revealed = nil } } }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .task { await menu.start() }
        .task(id: menu.rewards.count) {
            // A drive step asked for one reward's detail.
            guard let id = wallet.pendingReward, let reward = menu.rewards.first(where: { $0.id == id }) else { return }
            wallet.pendingReward = nil
            path = [.detail(reward)]
        }
        .onDisappear { menu.stop() }
        .onChange(of: wallet.points?.balance) { _, balance in menu.balanceChanged(balance) }
    }

    @ViewBuilder
    private var toast: some View {
        if let toast = menu.toast {
            Text(toast.text)
                .font(.snCaption)
                .foregroundStyle(toast.failed ? SN.error : SN.textPrimary)
                .padding(.horizontal, 12)
                .padding(.vertical, 7)
                .glassEffect()
                .padding(.top, 10)
                .transition(.opacity.combined(with: .offset(y: -6)))
                .accessibilityAddTraits(.updatesFrequently)
        }
    }
}

enum WalletRoute: Hashable {
    case detail(ChannelReward)
    case pick(ChannelReward, modify: Bool)
}

/// The header and the rewards.
private struct RewardsList: View {
    let wallet: WalletModel
    let menu: RewardsMenuModel
    let session: ChatSession
    let open: (ChannelReward) -> Void
    @State private var refreshing = false
    @State private var tab = Tab.points

    enum Tab: Hashable { case points, bits }

    /// The tab shown: the one picked, unless the channel lacks it.
    private var shown: Tab {
        if tab == .bits, wallet.bits == nil { return .points }
        if tab == .points, wallet.points == nil { return .bits }
        return tab
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                if wallet.points != nil, wallet.bits != nil {
                    GlazeSegments([Segment(value: Tab.points, label: "Points"), Segment(value: Tab.bits, label: "Bits")],
                                  selection: Binding(get: { shown }, set: { if let next = $0 { tab = next } }), fill: true)
                        .padding(.bottom, 16)
                }
                if shown == .bits {
                    BitsPane(login: wallet.login, channelID: session.roomID, core: session.core) { word in
                        ComposerRegistry.shared.models[ObjectIdentifier(session)]?.insertCheer(word)
                        wallet.sheetOpen = false
                    }
                } else {
                    points
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 20)
            .padding(.bottom, 24)
        }
        .scrollIndicators(.hidden)
        .scrollDismissesKeyboard(.never)
        .toolbar(.hidden, for: .navigationBar)
    }

    @ViewBuilder
    private var points: some View {
        VStack(alignment: .leading, spacing: 0) {
                header.padding(.bottom, 14)
                if menu.loading && menu.rewards.isEmpty {
                    ProgressView().frame(maxWidth: .infinity).padding(.vertical, 40)
                } else if let error = menu.error, menu.rewards.isEmpty {
                    VStack(spacing: 8) {
                        Text(error).font(.sn(13)).foregroundStyle(SN.textMuted).multilineTextAlignment(.center)
                        Button("Try again") { Task { await menu.reload(initial: true) } }
                            .font(.sn(13, 600))
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 32)
                } else if menu.rewards.isEmpty {
                    VStack(spacing: 4) {
                        Text("No rewards available").font(.sn(15, 500)).foregroundStyle(SN.textSecondary)
                        Text("This channel hasn't set up any rewards yet").font(.sn(13)).foregroundStyle(SN.textMuted)
                    }
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 32)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(menu.rewards.enumerated()), id: \.element.id) { index, reward in
                            Button { open(reward) } label: { RewardRow(reward: reward, iconURL: wallet.points?.iconURL) }
                                .buttonStyle(RowPress())
                            if index < menu.rewards.count - 1 { Divider().overlay(SN.borderSubtle).padding(.leading, 56) }
                        }
                    }
                    .snPanel()
                }
        }
    }

    /// The channel's points name over the balance; a tap re-reads it. Bits held here beside it.
    private var header: some View {
        HStack(alignment: .center, spacing: 12) {
            if let points = wallet.points {
                VStack(alignment: .leading, spacing: 2) {
                    Text(points.name.flatMap { $0.isEmpty ? nil : $0 } ?? "Channel Points")
                        .font(.sn(13, 500))
                        .foregroundStyle(SN.textSecondary)
                    Button {
                        Task {
                            refreshing = true
                            await wallet.refresh()
                            await menu.reload()
                            refreshing = false
                        }
                    } label: {
                        HStack(spacing: 6) {
                            PointsArt(iconURL: points.iconURL, size: 18)
                                .rotationEffect(.degrees(refreshing ? 360 : 0))
                                .animation(refreshing ? .linear(duration: 0.8).repeatForever(autoreverses: false) : .default, value: refreshing)
                            Text(points.balance.map { $0.formatted() } ?? "--")
                                .font(.sn(20, 700))
                                .monospacedDigit()
                                .foregroundStyle(SN.textPrimary)
                                .contentTransition(.numericText())
                        }
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Refresh \(points.name ?? "channel points") balance")
                }
            }
            Spacer(minLength: 8)
            if let bits = wallet.bits, let balance = bits.balance {
                HStack(spacing: 5) {
                    if let gem = bits.gemURL { RewardArt(url: gem, width: 16, height: 16, radius: 0, fit: true, placeholder: nil) }
                    Text("\(balance.formatted()) bits").font(.sn(13, 500)).monospacedDigit().foregroundStyle(SN.textSecondary)
                }
                .accessibilityElement(children: .combine)
            }
        }
    }
}

/// One reward: its tile, title, why it is blocked (or what it does), and its cost.
private struct RewardRow: View {
    let reward: ChannelReward
    let iconURL: String?

    var body: some View {
        HStack(spacing: 12) {
            RewardTile(reward: reward, size: 32)
                .opacity(reward.availability.redeemable ? 1 : 0.5)
            VStack(alignment: .leading, spacing: 2) {
                Text(reward.title)
                    .font(.sn(13, 500))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(2)
                    .multilineTextAlignment(.leading)
                if !reward.availability.redeemable, let label = reward.availability.label {
                    StatusLine(words: label, reason: reward.availability.reason, cooldownEnd: reward.availability.cooldownEnd)
                } else if let about = reward.about {
                    Text(about).font(.sn(11.5)).foregroundStyle(SN.textMuted).lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            CostPill(reward: reward, iconURL: iconURL)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// A blocked reward's reason as a colored dot and words, a running cooldown counting down.
struct StatusLine: View {
    let words: String
    let reason: String?
    let cooldownEnd: Date?
    var size: CGFloat = 11.5

    var body: some View {
        HStack(spacing: 5) {
            Circle().fill(dot).frame(width: 6, height: 6)
            if let end = cooldownEnd, end > .now {
                (Text("\(words) · ") + Text(timerInterval: Date.now...end, countsDown: true))
                    .monospacedDigit()
            } else {
                Text(words)
            }
        }
        .font(.sn(size))
        .foregroundStyle(SN.textMuted)
        .lineLimit(2)
        .accessibilityAddTraits(.updatesFrequently)
    }

    /// Red for points, amber for time, the accent for a sub perk, gray for the streamer's switches.
    private var dot: Color {
        switch reason {
        case "not_enough_points": Color(red: 0.97, green: 0.44, blue: 0.44)
        case "on_cooldown", "max_per_stream_reached", "only_while_live": SN.warning
        case "subscribers_only", "needs_sub_only_mode", "already_can_chat": SN.accent
        default: SN.textMuted
        }
    }
}

/// The reward's tile: its own color with its image, or the points glyph.
struct RewardTile: View {
    let reward: ChannelReward
    var size: CGFloat = 32

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: size * 0.25, style: .continuous).fill(RGBA(css: reward.backgroundColor)?.color ?? Color(red: 0.57, green: 0.28, blue: 1))
            if let image = reward.imageURL, !image.isEmpty {
                RewardArt(url: image, width: size * 0.7, height: size * 0.7, radius: 0, fit: true, placeholder: nil)
            } else {
                PointsGlyph().fill(style: FillStyle(eoFill: true)).foregroundStyle(.white).frame(width: size * 0.55, height: size * 0.55)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

/// The cost: accent when it can be redeemed, neutral when affordable but blocked, red when short.
struct CostPill: View {
    let reward: ChannelReward
    let iconURL: String?

    var body: some View {
        let a = reward.availability
        let tint: Color = !a.affordable ? Color(red: 0.97, green: 0.44, blue: 0.44) : a.redeemable ? SN.accent : SN.textSecondary
        HStack(spacing: 4) {
            PointsArt(iconURL: iconURL, size: 12)
            Text(reward.view.costLabel).font(.sn(12, 600)).monospacedDigit()
        }
        .foregroundStyle(tint)
        .padding(.horizontal, 8)
        .frame(height: 24)
        .background(Capsule().fill(tint.opacity(0.14)))
        .accessibilityLabel("\(reward.cost.formatted()) points")
    }
}

/// One reward up close: what it does, why it is blocked, the box for its text, and its action.
private struct RewardDetail: View {
    let menu: RewardsMenuModel
    let reward: ChannelReward
    let channelID: String?
    /// The reward picks an emote first: its kind.
    let pick: (String) -> Void
    let done: () -> Void
    @State private var text = ""
    @FocusState private var focused: Bool

    private static let limit = 500

    var body: some View {
        let live = menu.live(reward)
        let view = live.view
        let busy = menu.redeeming == live.id
        let canAct = live.availability.redeemable && menu.redeeming == nil && channelID != nil
            && (!view.takesText || !text.trimmingCharacters(in: .whitespaces).isEmpty)
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                HStack(alignment: .center, spacing: 12) {
                    RewardTile(reward: live, size: 40)
                    Text(live.title)
                        .font(.sn(16, 600))
                        .foregroundStyle(view.kind == "highlight" ? Color(red: 0.98, green: 0.80, blue: 0.08) : SN.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 8)
                    CostPill(reward: live, iconURL: nil)
                }
                if let about = live.about {
                    Text(about).font(.sn(13)).foregroundStyle(SN.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
                if !live.availability.redeemable, let words = live.availability.detail ?? live.availability.label {
                    StatusLine(words: words, reason: live.availability.reason, cooldownEnd: live.availability.cooldownEnd, size: 12.5)
                }
                if view.takesText {
                    VStack(alignment: .trailing, spacing: 4) {
                        TextField("", text: $text, prompt: Text(view.placeholder ?? "Type your message...").foregroundStyle(SN.textMuted),
                                  axis: .vertical)
                            .keyboardHideButton()
                            .font(.sn(14))
                            .lineLimit(3...5)
                            .focused($focused)
                            .padding(12)
                            .snPanel(radius: 10, light: nil)
                            .onChange(of: text) { _, next in if next.count > Self.limit { text = String(next.prefix(Self.limit)) } }
                        Text("\(text.count)/\(Self.limit)").font(.sn(11)).monospacedDigit().foregroundStyle(SN.textMuted)
                    }
                }
                VStack(spacing: 8) {
                    Button {
                        if view.picksEmote {
                            pick(view.kind)
                            return
                        }
                        guard let channelID else { return }
                        Task {
                            if await menu.redeem(live, channelID: channelID, text: view.takesText ? text : nil) { done() }
                        }
                    } label: {
                        Text(busy ? view.busy : view.action)
                    }
                    .buttonStyle(.snPrimary)
                    .disabled(!canAct)
                    if let hint = view.hint {
                        Text(hint).font(.sn(11.5)).foregroundStyle(SN.textMuted)
                    }
                }
                .padding(.top, 4)
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)
            .padding(.bottom, 24)
        }
        .scrollIndicators(.hidden)
        .navigationTitle("Rewards")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { if view.takesText { focused = true } }
    }
}

/// An emote unlock's pick: the channel's emotes, then for a modified one the effect.
private struct EmoteUnlockPicker: View {
    let menu: RewardsMenuModel
    let reward: ChannelReward
    let modify: Bool
    let channelID: String?
    let done: () -> Void
    @State private var emotes: [ModifiableEmote]?
    @State private var failed = false
    @State private var chosen: ModifiableEmote?
    @State private var effect: EmoteModification?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                Text(modify ? (chosen == nil ? "Select an Emote" : "Choose Modification") : "Choose an Emote")
                    .font(.sn(15, 600))
                    .foregroundStyle(SN.textPrimary)
                if let emotes {
                    if modify, let chosen {
                        effects(chosen)
                    } else if emotes.isEmpty {
                        Text(modify ? "No emotes available to modify" : "No emotes available to unlock")
                            .font(.sn(13)).foregroundStyle(SN.textMuted)
                    } else {
                        grid(emotes)
                    }
                } else if failed {
                    Text("Failed to load emotes").font(.sn(13)).foregroundStyle(SN.textMuted)
                } else {
                    ProgressView().frame(maxWidth: .infinity).padding(.vertical, 24)
                }
                confirm
            }
            .padding(16)
        }
        .scrollIndicators(.hidden)
        .navigationTitle(reward.title)
        .navigationBarTitleDisplayMode(.inline)
        .task {
            do {
                emotes = try await menu.loadEmotes()
            } catch {
                failed = true
            }
        }
    }

    private func grid(_ list: [ModifiableEmote]) -> some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 6), spacing: 8) {
            ForEach(list) { emote in
                Button { chosen = emote } label: {
                    RewardArt(url: emote.imageURL, width: 40, height: 40, radius: 0, fit: true, placeholder: nil)
                        .padding(6)
                        .background(RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .fill(chosen?.id == emote.id ? SN.accent.opacity(0.25) : SN.surface))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(emote.token)
                .accessibilityAddTraits(chosen?.id == emote.id ? .isSelected : [])
            }
        }
    }

    private func effects(_ emote: ModifiableEmote) -> some View {
        LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
            ForEach(emote.modifications) { mod in
                Button { effect = mod } label: {
                    HStack(spacing: 8) {
                        RewardArt(url: mod.imageURL, width: 28, height: 28, radius: 0, fit: true, placeholder: nil)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(mod.words.name).font(.sn(12.5, 600)).foregroundStyle(SN.textPrimary)
                            Text(mod.words.detail).font(.sn(11)).foregroundStyle(SN.textMuted).lineLimit(1)
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(8)
                    .background(RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .fill(effect?.id == mod.id ? SN.accent.opacity(0.25) : SN.surface))
                }
                .buttonStyle(.plain)
            }
        }
    }

    @ViewBuilder
    private var confirm: some View {
        let busy = menu.redeeming == reward.id
        if modify, chosen != nil {
            HStack(spacing: 10) {
                Button("Back") { chosen = nil; effect = nil }.buttonStyle(.snSecondary)
                Button(busy ? "Unlocking..." : "Confirm") { unlock() }
                    .buttonStyle(.snPrimary)
                    .disabled(effect == nil || busy || channelID == nil)
            }
        } else if !modify {
            Button(busy ? "Unlocking..." : "Unlock") { unlock() }
                .buttonStyle(.snPrimary)
                .disabled(chosen == nil || busy || channelID == nil)
        }
    }

    private func unlock() {
        guard let channelID, let chosen else { return }
        let pick = modify ? effect.map { ($0.id, $0.token) } : (chosen.id, chosen.token)
        guard let pick else { return }
        Task {
            if await menu.redeem(menu.live(reward), channelID: channelID, emote: (id: pick.0, name: pick.1)) { done() }
        }
    }
}

/// An unlocked emote, full panel, until tapped.
private struct EmoteReveal: View {
    let emote: UnlockedEmote
    let dismiss: () -> Void
    @Environment(\.snStill) private var still
    @State private var spin = false

    var body: some View {
        ZStack {
            Color.black.opacity(0.7).ignoresSafeArea()
            VStack(spacing: 14) {
                Text("🎉 You unlocked an emote! 🎉")
                    .font(.sn(18, 700))
                    .foregroundStyle(Color(red: 0.98, green: 0.80, blue: 0.08))
                ZStack {
                    Circle()
                        .fill(AngularGradient(colors: [.purple, .pink, .yellow, .purple], center: .center))
                        .frame(width: 130, height: 130)
                        .blur(radius: 18)
                        .rotationEffect(.degrees(spin ? 360 : 0))
                    RewardArt(url: emote.imageURL, width: 96, height: 96, radius: 0, fit: true, placeholder: nil)
                }
                Text(emote.name).font(.sn(24, 700)).foregroundStyle(.white)
                Text("Tap to dismiss").font(.sn(12)).foregroundStyle(.white.opacity(0.6))
            }
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: dismiss)
        .onAppear {
            guard !still else { return }
            withAnimation(.linear(duration: 4).repeatForever(autoreverses: false)) { spin = true }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityHint("Dismisses")
    }
}
