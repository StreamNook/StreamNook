import Observation
import SNKit
import SwiftUI

/// The Cosmetics screen's state: the core's two halves (StreamNook's own badge and Atmosphere; 7TV
/// and Twitch), with each change shown at once and put back if the core refuses it.
@Observable @MainActor
final class CosmeticsScreenModel {
    private(set) var sn: StreamNookPickerState?
    private(set) var ext: ExternalPickerState?
    var signingIn = false
    var showSevenTvSignIn = false
    @ObservationIgnored private var afterSignIn: ((String) async -> Void)?
    @ObservationIgnored let app: AppModel

    init(app: AppModel) {
        self.app = app
    }

    private var core: CoreBridge { app.core }
    private var user: TwitchUser? { app.signIn.user }

    /// Every open re-reads everything; each half lands on its own, with no spinners.
    func load() async {
        guard let user else { return }
        async let snRead: StreamNookPickerState? = try? core.invoke("cosmetics_picker_state", args: ["userId": user.id],
                                                                     as: StreamNookPickerState.self)
        async let extRead: ExternalPickerState? = try? core.invoke("external_cosmetics_state",
                                                                    args: ["userId": user.id, "login": user.login],
                                                                    as: ExternalPickerState.self)
        if let value = await snRead { sn = value }
        if let value = await extRead { ext = value }
    }

    // MARK: StreamNook

    /// Wears a badge; tapping the one worn takes it off (back to the plain mark).
    func equipBadge(_ badge: PickerBadge) async {
        guard let user, var state = sn else { return }
        let next: String? = badge.worn ? nil : badge.slug
        let before = state
        for i in state.badges.indices { state.badges[i].worn = state.badges[i].slug == next }
        sn = state
        do {
            _ = try await core.invoke("equip_streamnook_badge", args: ["userId": user.id, "slug": next])
        } catch {
            sn = before
            let text = (error as? CoreFailure)?.description ?? ""
            app.toasts.show(text.isEmpty ? "Could not equip that badge." : text)
        }
    }

    func equipTheme(_ atmosphere: PickerAtmosphere) async {
        guard let user, var state = sn, atmosphere.allowed else { return }
        let before = state
        state.theme = atmosphere.id
        for i in state.atmospheres.indices { state.atmospheres[i].worn = state.atmospheres[i].id == atmosphere.id }
        state.showChatToggle = atmosphere.id != "tier" && atmosphere.id != "paint"
        sn = state
        do {
            _ = try await core.invoke("set_profile_theme", args: ["userId": user.id, "theme": atmosphere.id])
        } catch {
            sn = before
            app.toasts.show("Could not apply that atmosphere.")
        }
    }

    func setChatAtmosphere(_ show: Bool) async {
        guard let user, var state = sn else { return }
        state.chatAtmosphere = show
        sn = state
        let saved = (try? await core.invoke("set_chat_atmosphere", args: ["userId": user.id, "show": show],
                                            as: Bool.self)) ?? false
        if !saved {
            state.chatAtmosphere = !show
            sn = state
            app.toasts.show("Could not save that. Try again in a moment.")
        }
    }

    // MARK: 7TV

    func signInToSevenTv() {
        guard !signingIn else { return }
        afterSignIn = nil
        signingIn = true
        showSevenTvSignIn = true
    }

    func signOutOfSevenTv() async {
        do {
            _ = try await core.invoke("logout_seventv", args: [:])
            ext?.sevenTvSignedIn = false
        } catch {
            app.toasts.show("Could not sign out of 7TV.")
        }
    }

    /// The sign-in sheet closed: stored or not, then read the status back.
    func sevenTvSignInFinished(_ stored: Bool) async {
        defer { signingIn = false }
        let next = afterSignIn
        afterSignIn = nil
        guard stored else { return }
        guard let status = try? await core.invoke("get_seventv_auth_status", args: [:], as: SevenTvStatus.self) else {
            app.toasts.show("Could not sign in to 7TV.")
            return
        }
        ext?.sevenTvSignedIn = status.isAuthenticated
        if let id = status.userID { ext?.sevenTvUserID = id }
        guard status.isAuthenticated else {
            app.toasts.show("7TV did not finish signing you in. Try again.")
            return
        }
        guard let id = status.userID ?? ext?.sevenTvUserID else {
            app.toasts.show("7TV did not identify your account.")
            return
        }
        await next?(id)
    }

    /// Runs `apply` with the 7TV account id, signing in first when needed; a tap while signing in
    /// is ignored.
    private func withSevenTv(_ apply: @escaping (String) async -> Void) async {
        guard let ext else { return }
        if !ext.sevenTvSignedIn {
            guard !signingIn else { return }
            afterSignIn = apply
            signingIn = true
            showSevenTvSignIn = true
            return
        }
        guard let id = ext.sevenTvUserID else {
            app.toasts.show("7TV did not identify your account.")
            return
        }
        await apply(id)
    }

    func equipPaint(_ paint: PickerPaint) async {
        await withSevenTv { [weak self] sevenTvID in
            guard let self, var state = self.ext else { return }
            let current = state.paints.first(where: \.worn)?.id
            let next: String? = current == paint.id ? nil : paint.id
            let before = state
            for i in state.paints.indices { state.paints[i].worn = state.paints[i].id == next }
            self.ext = state
            do {
                let result = try await self.core.invoke("set_seventv_paint", args: ["userId": sevenTvID, "paintId": next],
                                                        as: SevenTvWrite.self)
                if !result.success { throw CoreFailure.command(json: "\"rejected\"") }
            } catch {
                self.ext = before
                self.sevenTvWriteFailed(error, what: "paint")
            }
        }
    }

    func equipSevenTvBadge(_ badge: PickerSevenTvBadge) async {
        await withSevenTv { [weak self] sevenTvID in
            guard let self, var state = self.ext else { return }
            let current = state.sevenTvBadges.first(where: \.worn)?.id
            let next: String? = current == badge.id ? nil : badge.id
            let before = state
            for i in state.sevenTvBadges.indices { state.sevenTvBadges[i].worn = state.sevenTvBadges[i].id == next }
            self.ext = state
            do {
                let result = try await self.core.invoke("set_seventv_badge", args: ["userId": sevenTvID, "badgeId": next],
                                                        as: SevenTvWrite.self)
                if !result.success { throw CoreFailure.command(json: "\"rejected\"") }
            } catch {
                self.ext = before
                self.sevenTvWriteFailed(error, what: "badge")
            }
        }
    }

    private func sevenTvWriteFailed(_ error: Error, what: String) {
        if "\(error)".contains("SESSION_EXPIRED") {
            ext?.sevenTvSignedIn = false
            app.toasts.show("Your 7TV sign-in expired. Tap again to sign back in.")
        } else {
            app.toasts.show("Could not equip that \(what).")
        }
    }

    // MARK: Twitch

    /// Wears a Twitch global badge. There is no taking one off: Twitch only swaps which is worn.
    func equipTwitchBadge(_ badge: TwitchIdentityBadge) async {
        guard !badge.isSelected, var state = ext else { return }
        let before = state
        for i in state.twitchSelectable.indices {
            state.twitchSelectable[i].isSelected = state.twitchSelectable[i].setID == badge.setID
        }
        ext = state
        do {
            _ = try await core.invoke("update_chat_identity_gql", args: ["badgeId": badge.setID, "badgeVersion": badge.version])
        } catch {
            ext = before
            app.toasts.show("\(error)".contains("NEEDS_DROPS_AUTH") ? "Sign in to Drops to change your Twitch badge."
                                                                     : "Could not equip that badge.")
        }
    }

    private struct SevenTvStatus: Decodable {
        var isAuthenticated: Bool
        var userID: String?
        enum CodingKeys: String, CodingKey { case isAuthenticated = "is_authenticated", userID = "user_id" }
    }

    private struct SevenTvWrite: Decodable {
        var success: Bool
    }
}

/// Cosmetics: your StreamNook badge, your Atmosphere (and whether it shows in chat), your 7TV paint
/// and badge, and your Twitch global badge. Pushed from the You tab, the tab bar hidden.
struct CosmeticsScreen: View {
    let model: AppModel
    @State private var screen: CosmeticsScreenModel

    init(model: AppModel) {
        self.model = model
        _screen = State(initialValue: CosmeticsScreenModel(app: model))
    }

    private var store: CosmeticsStore { model.cosmetics }

    var body: some View {
        ScrollViewReader { reader in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    badgeSection.id("badge")
                    atmosphereSection.id("atmosphere")
                    sevenTvSection.id("seventv")
                    twitchSection.id("twitch")
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
                .padding(.bottom, 16)
            }
            .onChange(of: store.scrollTarget) { _, target in
                guard let target else { return }
                withAnimation { reader.scrollTo(target, anchor: .top) }
                store.scrollTarget = nil
            }
        }
        .scrollIndicators(.hidden)
        .background(SN.background)
        .navigationTitle("Cosmetics")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .task {
            store.activeScreen = screen
            await screen.load()
        }
        .sheet(isPresented: $screen.showSevenTvSignIn) {
            SevenTvSignInSheet(core: model.core) { stored in
                Task { await screen.sevenTvSignInFinished(stored) }
            }
        }
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .font(.sn(12, 600))
            .textCase(.uppercase)
            .tracking(0.6)
            .foregroundStyle(SN.textMuted)
            .padding(.bottom, 6)
    }

    // MARK: Badge

    @ViewBuilder
    private var badgeSection: some View {
        caption("Badge")
        if let badges = screen.sn?.badges {
            if badges.isEmpty {
                Text("No badges owned yet. Earned and event badges land here.")
                    .font(.sn(13))
                    .foregroundStyle(SN.textMuted)
                    .padding(16)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .snPanel()
                    .padding(.bottom, 16)
            } else {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 4), spacing: 8) {
                    ForEach(badges) { badge in
                        Button { Task { await screen.equipBadge(badge) } } label: {
                            BadgeTile(url: badge.imageURL, name: badge.name, worn: badge.worn)
                        }
                        .buttonStyle(PressScale())
                    }
                }
                .padding(.bottom, 16)
            }
        }
    }

    // MARK: Atmosphere

    @ViewBuilder
    private var atmosphereSection: some View {
        caption("Atmosphere")
        if let state = screen.sn {
            LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                ForEach(state.atmospheres) { atmosphere in
                    Button { Task { await screen.equipTheme(atmosphere) } } label: {
                        AtmosphereTile(atmosphere: atmosphere)
                    }
                    .buttonStyle(PressScale())
                    .disabled(!atmosphere.allowed)
                }
            }
            if state.showChatToggle {
                HStack(spacing: 12) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Show in my chat messages").font(.sn(13.5)).foregroundStyle(SN.textPrimary)
                        Text("Off keeps it on your profile only.").font(.sn(12)).foregroundStyle(SN.textMuted)
                    }
                    Spacer(minLength: 8)
                    Toggle("", isOn: Binding(get: { state.chatAtmosphere },
                                             set: { show in Task { await screen.setChatAtmosphere(show) } }))
                        .labelsHidden()
                        .tint(SN.accent)
                        .accessibilityLabel(state.chatAtmosphere ? "Hide your atmosphere in chat" : "Show your atmosphere in chat")
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .snPanel()
                .padding(.top, 8)
            }
        }
    }

    // MARK: 7TV

    @ViewBuilder
    private var sevenTvSection: some View {
        if let ext = screen.ext, !ext.paints.isEmpty || !ext.sevenTvBadges.isEmpty {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text("7TV").font(.sn(12, 600)).textCase(.uppercase).tracking(0.6).foregroundStyle(SN.textMuted)
                Text(ext.sevenTvSignedIn ? "Signed in" : "Not signed in").font(.sn(12)).foregroundStyle(SN.textMuted)
                Spacer()
                Button(screen.signingIn ? "Signing in…" : ext.sevenTvSignedIn ? "Sign out" : "Sign in") {
                    if ext.sevenTvSignedIn { Task { await screen.signOutOfSevenTv() } } else { screen.signInToSevenTv() }
                }
                .font(.sn(12.5))
                .foregroundStyle(SN.accent)
                .disabled(screen.signingIn)
                .opacity(screen.signingIn ? 0.5 : 1)
            }
            .padding(.top, 20)
            if !ext.paints.isEmpty {
                Collapsible(title: "7TV paints", count: ext.paints.count, key: "paints", store: store) {
                    LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                        ForEach(ext.paints) { paint in
                            Button { Task { await screen.equipPaint(paint) } } label: {
                                HStack(spacing: 6) {
                                    PaintedText(text: paint.name, font: .sn(14, 700), paint: paint.paint,
                                                fallback: Color(red: 0x91 / 255, green: 0x46 / 255, blue: 1))
                                    Spacer(minLength: 0)
                                    if paint.worn { WornCheck() }
                                }
                                .padding(10)
                                .snPanel()
                                .overlay { if paint.worn { WornRing() } }
                            }
                            .buttonStyle(PressScale())
                        }
                    }
                }
            }
            if !ext.sevenTvBadges.isEmpty {
                Collapsible(title: "7TV badges", count: ext.sevenTvBadges.count, key: "seventvBadges", store: store) {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 4), spacing: 8) {
                        ForEach(ext.sevenTvBadges) { entry in
                            Button { Task { await screen.equipSevenTvBadge(entry) } } label: {
                                BadgeTile(url: entry.badge.url, fallbacks: entry.badge.fallbacks,
                                          name: entry.badge.name ?? entry.badge.title, worn: entry.worn)
                            }
                            .buttonStyle(PressScale())
                        }
                    }
                }
            }
        }
    }

    // MARK: Twitch

    @ViewBuilder
    private var twitchSection: some View {
        if let ext = screen.ext {
            if !ext.twitchSelectable.isEmpty {
                Collapsible(title: "Twitch badges", count: ext.twitchSelectable.count, key: "twitch", store: store) {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 4), spacing: 8) {
                        ForEach(ext.twitchSelectable) { badge in
                            Button { Task { await screen.equipTwitchBadge(badge) } } label: {
                                BadgeTile(url: badge.imageURL, name: badge.title, worn: badge.isSelected)
                            }
                            .buttonStyle(PressScale())
                        }
                    }
                }
            } else if !ext.twitchEarned.isEmpty {
                Collapsible(title: "Twitch badges", count: ext.twitchEarned.count, key: "twitch", store: store) {
                    VStack(alignment: .leading, spacing: 10) {
                        FlowLayout(spacing: 10) {
                            ForEach(ext.twitchEarned) { badge in
                                CosmeticImage(badge.url, fallbacks: badge.fallbacks, animate: true)
                                    .frame(width: 32, height: 32)
                                    .accessibilityLabel(badge.label)
                            }
                        }
                        if !ext.dropsSignedIn {
                            Text("Sign in to Drops to choose which badge you wear.")
                                .font(.sn(11.5))
                                .foregroundStyle(SN.textMuted)
                        }
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .snPanel()
                }
            }
        }
    }
}

/// A named section with its count that opens on tap; collapsed on first open and kept as left for
/// the session. Not shown at all when empty (the caller decides).
private struct Collapsible<Content: View>: View {
    let title: String
    let count: Int
    let key: String
    let store: CosmeticsStore
    @ViewBuilder let content: () -> Content

    private var open: Bool { store.openSections.contains(key) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.timingCurve(0.16, 1, 0.3, 1, duration: 0.26)) {
                    if open { store.openSections.remove(key) } else { store.openSections.insert(key) }
                }
            } label: {
                HStack(spacing: 8) {
                    Text(title).font(.sn(12, 600)).textCase(.uppercase).tracking(0.6).foregroundStyle(SN.textMuted)
                    Text("\(count)").font(.sn(12)).foregroundStyle(SN.textMuted)
                    Spacer()
                    Image(systemName: "chevron.right")
                        .font(.system(size: 14, weight: .bold))
                        .foregroundStyle(SN.textMuted)
                        .rotationEffect(.degrees(open ? 90 : 0))
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(open ? "Expanded" : "Collapsed")
            .padding(.top, 16)
            .padding(.bottom, 6)
            if open {
                content()
                    .padding(.bottom, 4)
                    .transition(.opacity)
            }
        }
    }
}

/// A badge tile: glass, its 36 pt art (or a placeholder square), its name under it in two lines at
/// most, and the worn ring with a check.
private struct BadgeTile: View {
    let url: String?
    var fallbacks: [String] = []
    let name: String
    let worn: Bool

    var body: some View {
        VStack(spacing: 6) {
            if let url {
                CosmeticImage(url, fallbacks: fallbacks, animate: true).frame(width: 36, height: 36)
            } else {
                RoundedRectangle(cornerRadius: 4).fill(SN.surface).frame(width: 36, height: 36)
            }
            Text(name)
                .font(.sn(10.5))
                .foregroundStyle(SN.textSecondary)
                .multilineTextAlignment(.center)
                .lineLimit(2)
                .frame(maxWidth: .infinity)
        }
        .padding(8)
        .frame(maxHeight: .infinity, alignment: .top)
        .snPanel()
        .overlay { if worn { WornRing() } }
        .overlay(alignment: .topTrailing) { if worn { WornCheck().padding(4) } }
    }
}

/// An Atmosphere tile: its swatch drawn from the core's layers (the plain surface for Default),
/// its name, and the worn check or the lock.
private struct AtmosphereTile: View {
    let atmosphere: PickerAtmosphere

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Group {
                if atmosphere.swatch.isEmpty {
                    SN.surface
                } else {
                    FillView(fill: atmosphere.swatch)
                }
            }
            .frame(height: 48)
            .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
            HStack(spacing: 4) {
                Text(atmosphere.name).font(.sn(12.5)).foregroundStyle(SN.textPrimary).lineLimit(1)
                Spacer(minLength: 0)
                if atmosphere.worn {
                    WornCheck()
                } else if !atmosphere.allowed {
                    Image(systemName: "lock.fill").font(.system(size: 12)).foregroundStyle(SN.textMuted)
                }
            }
        }
        .padding(10)
        .snPanel()
        .overlay { if atmosphere.worn { WornRing() } }
        .opacity(atmosphere.allowed ? 1 : 0.45)
    }
}

private struct WornRing: View {
    var body: some View {
        RoundedRectangle(cornerRadius: SNRadius.panel, style: .continuous).strokeBorder(SN.accent, lineWidth: 2)
    }
}

private struct WornCheck: View {
    var body: some View {
        Image(systemName: "checkmark").font(.system(size: 13, weight: .bold)).foregroundStyle(SN.accent)
    }
}
