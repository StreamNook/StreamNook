import SNKit
import SwiftUI
import UserNotifications

/// The first-run wizard: welcome, accounts, drops, Kick, notifications, done. A full-screen layer on
/// the theme background under a faint accent wash; pages swipe sideways past 70 points and move
/// with a short rise. Finishing saves `setup_complete` through the core; closing early only keeps
/// it away for this session. The notifications page offers the alert types the core marks for
/// setup on this iPhone (`get_alert_types`), as Android's wizard does, and is left out when the
/// core offers none.
struct SetupWizard: View {
    let model: AppModel
    @State private var page = Page.welcome
    @State private var forward = true
    @State private var drag: CGFloat = 0
    @State private var lastTurn = Date.distantPast
    /// The alert types turned on in the notifications page; every one starts off.
    @State private var chosen: Set<String> = []
    /// Where notifications stand for StreamNook, read after each ask and on return to the app.
    @State private var alertStatus: UNAuthorizationStatus?
    @Environment(\.snStill) private var still
    @Environment(\.scenePhase) private var scene

    enum Page: Int, CaseIterable { case welcome, accounts, drops, kick, notifications, done }

    private var signedIn: Bool { model.signedIn }
    private var rewards: RewardsStore { model.rewards }
    private var dropsConnected: Bool { rewards.drops?.connected == true }
    private var kick: KickAccount { model.kick }
    /// The alert types the core offers in setup on this iPhone.
    private var setupAlerts: [AlertType] { model.alertTypes.filter(\.setup) }
    /// The pages this iPhone shows, in order.
    private var pages: [Page] { Page.allCases.filter { $0 != .notifications || !setupAlerts.isEmpty } }
    private func index(_ page: Page) -> Int { pages.firstIndex(of: page) ?? page.rawValue }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                if page != .welcome && page != .done {
                    Button { turn(back: true) } label: {
                        Label("Back", systemImage: Symbol.back)
                            .font(.snBody)
                            .foregroundStyle(SN.textSecondary)
                            .frame(minHeight: 44)
                    }
                    .buttonStyle(.plain)
                }
                Spacer()
            }
            .frame(height: 44)
            .padding(.horizontal, 16)

            ZStack {
                pageView(page)
                    .id(page)
                    .transition(still ? .opacity : .asymmetric(
                        insertion: .opacity.combined(with: .offset(y: 14)),
                        removal: .opacity.combined(with: .offset(y: -14))))
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .offset(x: drag)
            .contentShape(Rectangle())
            .gesture(swipe)

            footer
        }
        .background { Wash() }
        .task {
            rewards.listen(toasts: model.toasts)
            await rewards.load(refresh: false)
        }
        .onChange(of: model.host.sheet == nil) { _, closed in if closed { rewards.sheetClosed() } }
        .onChange(of: model.nav.setupPage, initial: true) { _, asked in
            if let asked { go(to: asked) }
        }
        .onChange(of: model.nav.setupAlert, initial: true) { _, id in
            guard let id, let type = setupAlerts.first(where: { $0.id == id }) else { return }
            model.nav.setupAlert = nil
            Task { await choose(type, true) }
        }
        .onChange(of: scene) { _, now in
            if now == .active, page == .notifications { Task { alertStatus = await MentionAlerts.status() } }
        }
        .onChange(of: signedIn) { _, now in
            guard now, page == .accounts else { return }
            Task {
                try? await Task.sleep(for: .milliseconds(500))
                turn(back: false)
            }
        }
    }

    // MARK: Pages

    @ViewBuilder
    private func pageView(_ page: Page) -> some View {
        switch page {
        case .welcome:
            PageBody(title: "Welcome to\nStreamNook", titleSize: 44,
                     text: "Yeah yeah, another setup wizard. A few clicks and we'll get out of your way, promise.") {
                PenroseMark(width: 72)
            }
        case .accounts:
            if signedIn {
                PageBody(title: "You're signed in", text: "Your follows, chat, and channel actions are connected.") {
                    Image(systemName: Symbol.checkCircle).font(.system(size: 52)).foregroundStyle(SN.success)
                }
            } else {
                PageBody(title: "Connect your accounts",
                         text: "Connect the platforms you watch on to see your follows, chat, and use channel features.") {
                    PlatformMark(platform: .twitch, size: 56)
                }
            }
        case .drops:
            PageBody(title: "Drops and inventory",
                     text: "Sign in to track Twitch Drops, watch your inventory, and auto-claim rewards.") {
                Image(systemName: Symbol.gift).font(.system(size: 48)).foregroundStyle(SN.accent)
            } extra: {
                dropsAction
            }
        case .kick:
            PageBody(title: "Kick",
                     text: "Sign in with Kick to bring in the channels you follow there and chat in Kick rooms.") {
                PlatformMark(platform: .kick, size: 56)
            } extra: {
                kickAction
            }
        case .notifications:
            PageBody(title: "Notifications",
                     text: "Choose what StreamNook alerts you about. You can change these any time in Settings.") {
                Image(systemName: Symbol.notifications).font(.system(size: 48)).foregroundStyle(SN.accent)
            } extra: {
                notificationsAction
            }
        case .done:
            PageBody(title: "You're all set", text: "StreamNook is ready. Pick a stream and dive in.") {
                PenroseMark(width: 64)
            } extra: {
                VStack(spacing: 0) {
                    summary("Twitch", mark: .twitch, done: signedIn)
                    Divider().overlay(SN.borderSubtle)
                    summary("Drops sign-in", done: dropsConnected)
                    Divider().overlay(SN.borderSubtle)
                    summary("Kick", mark: .kick, done: kick.connected)
                }
                .snPanel()
                .frame(maxWidth: 320)
            }
        }
    }

    /// Drops sign-in, the same as the Rewards tab's: the core opens Twitch's grant in the web
    /// sheet and announces the outcome. Connected, the page says so instead.
    @ViewBuilder
    private var dropsAction: some View {
        if dropsConnected {
            Label("Drops connected", systemImage: Symbol.checkCircle)
                .font(.snBody)
                .foregroundStyle(SN.success)
                .frame(minHeight: 50)
        } else {
            VStack(spacing: 10) {
                Button { Task { await rewards.connect() } } label: {
                    HStack(spacing: 10) {
                        if rewards.connecting {
                            ProgressView()
                            Text("Waiting for Twitch")
                        } else {
                            Text("Sign in for Drops")
                        }
                    }
                }
                .buttonStyle(.snSecondary)
                .frame(maxWidth: 320)
                .disabled(rewards.connecting || !model.started)
                if let error = rewards.connectError {
                    Text(error)
                        .font(.snCaption)
                        .foregroundStyle(SN.textMuted)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 320)
                }
            }
        }
    }

    /// Kick sign-in, the same as the You tab's Kick row: the core runs the kick.com login and
    /// Kick's Authorize page in the web sheet. Signed in, the page says who.
    @ViewBuilder
    private var kickAction: some View {
        if kick.connected {
            Label(kick.name.map { "Signed in as \($0)" } ?? "Kick connected", systemImage: Symbol.checkCircle)
                .font(.snBody)
                .foregroundStyle(SN.success)
                .frame(minHeight: 50)
        } else {
            VStack(spacing: 10) {
                Button { Task { await kick.connect() } } label: {
                    HStack(spacing: 10) {
                        if kick.busy {
                            ProgressView()
                            Text("Signing in")
                        } else {
                            PlatformMark(platform: .kick, size: 18)
                            Text("Sign in with Kick")
                        }
                    }
                }
                .buttonStyle(.snSecondary)
                .frame(maxWidth: 320)
                .disabled(kick.busy || !model.started)
                // The step while it runs, or why it did not finish.
                if let step = kick.step, !kick.busy {
                    Text(step)
                        .font(.snCaption)
                        .foregroundStyle(SN.textMuted)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 320)
                }
            }
        }
    }

    /// A switch per alert type the core offers here, all off until chosen. Turning one on saves it
    /// and only then asks the iPhone for permission (the first time; an answer given stays). A
    /// refusal is said plainly under the switches, with the way to Settings; its room is kept
    /// while hidden, so the switches never move on the centered page.
    private var notificationsAction: some View {
        let refused = !chosen.isEmpty && alertStatus == .denied
        return VStack(spacing: 16) {
            VStack(spacing: 0) {
                ForEach(Array(setupAlerts.enumerated()), id: \.element.id) { offset, type in
                    if offset > 0 { Divider().overlay(SN.borderSubtle) }
                    alertRow(type)
                }
            }
            .snPanel()
            // Mentions while you're away: its own card, since it is a server watching for you
            // rather than an alert type. Its pushes follow the Mentions and replies alert.
            awayRow
                .snPanel()
            VStack(spacing: 10) {
                Text("Notifications are turned off for StreamNook in iPhone Settings, so these alerts will not arrive until you allow them there.")
                    .font(.snCaption)
                    .foregroundStyle(SN.textSecondary)
                    .multilineTextAlignment(.center)
                    .fixedSize(horizontal: false, vertical: true)
                Button { MentionAlerts.openSettings() } label: {
                    Label("Open iPhone Settings", systemImage: "arrow.up.right.square")
                }
                .buttonStyle(.snSecondary)
            }
            .opacity(refused ? 1 : 0)
            .allowsHitTesting(refused)
            .accessibilityHidden(!refused)
        }
        .frame(maxWidth: 320)
        .task { alertStatus = await MentionAlerts.status() }
    }

    private var awayRow: some View {
        let title = "Mentions while you're away"
        return HStack(alignment: .top, spacing: 12) {
            Image(systemName: SettingIcons.symbol(paths: ["mentions.away_alerts"], title: title))
                .font(.system(size: 16))
                .foregroundStyle(SN.textSecondary)
                .frame(width: 22, height: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.snRowTitle).foregroundStyle(SN.textPrimary)
                Text("StreamNook's server watches chat in channels you follow for your name and replies, and keeps matches for 7 days.")
                    .font(.snRowDetail).foregroundStyle(SN.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Toggle(title, isOn: Binding(get: { model.settingsStore.bool("mentions.away_alerts", false) },
                                        set: { on in Task { await model.settingsStore.set(["mentions.away_alerts": on]) } }))
                .labelsHidden()
                .tint(SN.accent)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    private func alertRow(_ type: AlertType) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: SettingIcons.symbol(paths: [type.setting], title: type.title))
                .font(.system(size: 16))
                .foregroundStyle(SN.textSecondary)
                .frame(width: 22, height: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(type.title).font(.snRowTitle).foregroundStyle(SN.textPrimary)
                Text(type.detail).font(.snRowDetail).foregroundStyle(SN.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Toggle(type.title, isOn: Binding(get: { chosen.contains(type.id) },
                                             set: { on in Task { await choose(type, on) } }))
                .labelsHidden()
                .tint(SN.accent)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    /// Saves one choice; turning a type on also turns notifications on and asks for permission.
    private func choose(_ type: AlertType, _ on: Bool) async {
        if on { chosen.insert(type.id) } else { chosen.remove(type.id) }
        var changes: [String: Any?] = [type.setting: on]
        if on { changes["live_notifications.enabled"] = true }
        await model.settingsStore.set(changes)
        guard on else { return }
        await MentionAlerts.askIfUndecided()
        alertStatus = await MentionAlerts.status()
        await LiveAlerts.sync(model)
    }

    private func summary(_ title: String, mark: PlatformMark.Platform? = nil, done: Bool) -> some View {
        HStack(spacing: 8) {
            if let mark { PlatformMark(platform: mark, size: 15) }
            Text(title).font(.snRowTitle).foregroundStyle(SN.textPrimary)
            Spacer()
            if done {
                Image(systemName: Symbol.checkCircle).foregroundStyle(SN.success)
            } else {
                Text("skipped").font(.snRowDetail).foregroundStyle(SN.textMuted)
            }
        }
        .padding(.horizontal, 14)
        .frame(height: 46)
    }

    // MARK: Footer

    private var footer: some View {
        VStack(spacing: 18) {
            HStack(spacing: 6) {
                ForEach(pages, id: \.self) { step in
                    let past = index(step) < index(page)
                    Capsule()
                        .fill(step == page ? SN.accent : past ? SN.accent.opacity(0.5) : SN.borderSubtle)
                        .frame(width: step == page ? 40 : 8, height: 8)
                        .shadow(color: step == page ? SN.accent.opacity(0.5) : .clear, radius: 4)
                        .onTapGesture { if past { go(to: step) } }
                        .accessibilityLabel("Step \(index(step) + 1)")
                }
            }
            primary
        }
        .padding(.horizontal, 24)
        .padding(.bottom, 20)
    }

    @ViewBuilder
    private var primary: some View {
        if page == .accounts && !signedIn {
            Button { Task { await model.signIn.begin(core: model.core) } } label: {
                HStack(spacing: 10) {
                    if model.signIn.busy {
                        ProgressView().tint(.white)
                        Text("Waiting for sign-in")
                    } else {
                        PlatformMark(platform: .twitch, size: 18, color: .white)
                        Text("Sign in with Twitch")
                    }
                }
            }
            .buttonStyle(.snBrand(SN.twitch))
            .disabled(model.signIn.busy || !model.started)
        } else {
            // The way on is the one filled button on the page; a page's own action above it is
            // glass. Both are the same height, so the step dots never jump between pages.
            Button(primaryTitle) {
                if page == .done {
                    Task { await model.finishSetup() }
                } else {
                    turn(back: false)
                }
            }
            .buttonStyle(.snPrimary)
        }
    }

    /// The filled button's words: the notifications page is a choice, so with nothing chosen it
    /// reads Skip.
    private var primaryTitle: String {
        switch page {
        case .welcome: "Get started"
        case .done: "Start watching"
        case .notifications: chosen.isEmpty ? "Skip" : "Continue"
        default: "Continue"
        }
    }

    // MARK: Moving between pages

    /// Forward is never allowed off the last page (finishing is a deliberate tap) or past
    /// accounts while Twitch is signed out; back is offered between the first and last pages.
    private func canGo(back: Bool) -> Bool {
        if back { return page != .welcome && page != .done }
        if page == .done { return false }
        if page == .accounts && !signedIn { return false }
        return true
    }

    private func turn(back: Bool) {
        let at = index(page) + (back ? -1 : 1)
        guard canGo(back: back), pages.indices.contains(at) else { return }
        go(to: pages[at])
    }

    private func go(to next: Page) {
        guard pages.contains(next) else { return }
        forward = index(next) > index(page)
        // Leaving the notifications page forward saves each offered type as it showed: chosen
        // on, the rest off.
        if page == .notifications && forward {
            let changes = AlertTypes.setupChoices(setupAlerts, chosen: chosen)
            let store = model.settingsStore
            Task { await store.set(changes) }
        }
        lastTurn = .now
        withAnimation(SNMotion.run(SNMotion.page)) { page = next }
    }

    /// A sideways drag anywhere on the page, rubber-banded; past 70 points it turns the page. A
    /// second swipe within 400 ms of a turn is ignored.
    private var swipe: some Gesture {
        DragGesture(minimumDistance: 12)
            .onChanged { value in
                guard abs(value.translation.width) > abs(value.translation.height) else { return }
                drag = value.translation.width * 0.15
            }
            .onEnded { value in
                let dx = value.translation.width
                withAnimation(SNMotion.run(SNMotion.settleBack)) { drag = 0 }
                guard Date.now.timeIntervalSince(lastTurn) > 0.4, abs(dx) > 70,
                      abs(dx) > abs(value.translation.height) else { return }
                turn(back: dx > 0)
            }
    }

}

/// A wizard page: the picture, the title, the line, and anything the page adds under them.
private struct PageBody<Art: View, Extra: View>: View {
    let title: String
    var titleSize: CGFloat = 30
    let text: String
    @ViewBuilder let art: Art
    @ViewBuilder let extra: Extra

    init(title: String, titleSize: CGFloat = 30, text: String, @ViewBuilder art: () -> Art,
         @ViewBuilder extra: () -> Extra = { EmptyView() }) {
        self.title = title
        self.titleSize = titleSize
        self.text = text
        self.art = art()
        self.extra = extra()
    }

    var body: some View {
        // The title and its line are one thought, so they sit close; the picture and the page's
        // action stand further off.
        VStack(spacing: 0) {
            art.frame(height: 84)
                .padding(.bottom, 24)
            Text(title)
                .font(.sn(titleSize, 600))
                .foregroundStyle(SN.textPrimary)
                .multilineTextAlignment(.center)
                .lineSpacing(-2)
                .padding(.bottom, 10)
            Text(text)
                .font(.sn(15))
                .foregroundStyle(SN.textSecondary)
                .multilineTextAlignment(.center)
                .lineSpacing(2)
                .frame(maxWidth: 320)
                .fixedSize(horizontal: false, vertical: true)
            extra.padding(.top, 28)
        }
        .padding(.horizontal, 28)
    }
}

/// The wizard's background: the theme background under a faint accent wash from the top.
private struct Wash: View {
    var body: some View {
        ZStack {
            SN.background
            ThemeLightLayer(surface: .shell)
            RadialGradient(colors: [SN.accent.opacity(0.16), .clear], center: .top, startRadius: 0, endRadius: 520)
            RadialGradient(colors: [SN.accent.opacity(0.06), .clear], center: .bottomTrailing, startRadius: 0, endRadius: 420)
        }
        .ignoresSafeArea()
    }
}
