import SNKit
import SwiftUI

/// The tabs (the system tab bar, one navigation stack each, all kept alive while another is in
/// front) with the watch layer over them. Landscape watching is always immersive: no status bar,
/// no home indicator.
struct MainShell: View {
    @Bindable var model: AppModel
    @State private var landscape = false
    @State private var insets = EdgeInsets()
    /// The You tab's icon: your picture once it is drawn.
    @State private var youAvatar = YouTabAvatar()

    private var nav: Navigator { model.nav }

    var body: some View {
        @Bindable var nav = model.nav
        @Bindable var cosmetics = model.cosmetics
        let immersive = landscape && nav.shellCovered
        ZStack {
            TabView(selection: $nav.tab) {
                Tab(MobileTab.following.title, systemImage: Symbol.following, value: MobileTab.following) {
                    stack(.following) { FollowingScreen(model: model) }
                }
                Tab(MobileTab.browse.title, systemImage: Symbol.browse, value: MobileTab.browse) {
                    stack(.browse) { BrowseScreen(model: model) }
                }
                Tab(MobileTab.rewards.title, systemImage: Symbol.rewards, value: MobileTab.rewards) {
                    stack(.rewards) { RewardsScreen(model: model) }
                }
                Tab(value: MobileTab.you) {
                    stack(.you) { YouScreen(model: model) }
                } label: {
                    YouTabLabel(avatar: youAvatar, selected: nav.tab == .you)
                }
            }
            .tint(SN.accent)
            .environment(\.shellCovered, nav.shellCovered)
            .accessibilityHidden(nav.shellCovered)

            if nav.playing != nil {
                WatchLayer(model: model, insets: insets)
                    .transition(.asymmetric(insertion: .opacity.combined(with: .offset(y: 40)).animation(SNMotion.run(SNMotion.watchIn)),
                                            removal: .opacity.combined(with: .offset(y: 40)).animation(SNMotion.run(SNMotion.watchOut))))
                    .zIndex(1)
            }
        }
        .onGeometryChange(for: Bool.self) { $0.size.width > $0.size.height } action: { landscape = $0 }
        .onGeometryChange(for: EdgeInsets.self) { $0.safeAreaInsets } action: { insets = $0 }
        .statusBarHidden(immersive)
        .persistentSystemOverlays(immersive ? .hidden : .automatic)
        .onChange(of: nav.shellCovered, initial: true) { _, covered in Orientation.allowLandscape(covered) }
        .onChange(of: nav.tab) { old, _ in if old == .browse { model.browseState.leftTab() } }
        // Your picture follows who is signed in, a cosmetics change, and the theme's accent (the
        // selected ring's color).
        .task(id: "\(model.signIn.user?.id ?? "")|\(model.signIn.user?.profileImageURL ?? "")|\(model.cosmetics.registryVersion)|\(model.look.themeID)") {
            await youAvatar.refresh(model: model)
        }
        .sheet(item: $cosmetics.profile) { target in
            ProfileSheet(model: ProfileSheetModel(target: target, core: model.core))
        }
        .sheet(item: $model.clip) { link in
            ClipSheet(url: link.url, model: model)
        }
        #if DEBUG
        .sheet(item: $nav.menuSample) { sample in
            MenuSampleSheet(sample: sample)
        }
        #endif
    }

    private func stack<Content: View>(_ tab: MobileTab, @ViewBuilder content: () -> Content) -> some View {
        NavigationStack(path: nav.path(tab)) {
            content()
                .navigationDestination(for: Route.self) { route in
                    destination(route)
                }
        }
        .overlay {
            if showsTabBar(tab) { TabBarFloor() }
        }
    }

    /// Whether the tab's screen keeps the tab bar: a category's streams and Edit profile hide it.
    private func showsTabBar(_ tab: MobileTab) -> Bool {
        switch nav.paths[tab]?.last {
        case .category, .cosmetics: false
        default: true
        }
    }

    @ViewBuilder
    private func destination(_ route: Route) -> some View {
        switch route {
        case .category(let category):
            CategoryScreen(model: model, category: category)
        case .settings(let section):
            SettingsPage(model: model, section: section)
        case .cosmetics:
            CosmeticsScreen(model: model)
        }
    }
}

extension EnvironmentValues {
    /// The full player covers the tabs: looping decorations in them stop.
    @Entry var shellCovered = false
}

/// The floor under the floating tab bar: the page's own background rising from clear on an eased
/// curve, so content scrolling down fades out under the bar instead of showing through it, with no
/// hard edge and no bare strip below it. It never takes a touch.
private struct TabBarFloor: View {
    /// How far above the bar's top edge the fade starts. Inside a tab the bottom safe area already
    /// spans the bar and the home indicator, so the floor covers that inset plus this lead-in and
    /// nothing higher: content above the bar stays at full strength.
    static let lead: CGFloat = 28

    var body: some View {
        GeometryReader { proxy in
            LinearGradient(stops: Self.easedStops(SN.background), startPoint: .top, endPoint: .bottom)
                .frame(height: proxy.safeAreaInsets.bottom + Self.lead)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottom)
        }
        .ignoresSafeArea(edges: .bottom)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }

    /// An ease-in curve sampled as gradient stops (opacity grows as t squared, smoothed), so the
    /// floor has no visible start line.
    static func easedStops(_ color: Color) -> [Gradient.Stop] {
        [0, 0.15, 0.3, 0.45, 0.6, 0.75, 0.9, 1].map { t in
            let eased = t * t * (3 - 2 * t)
            return Gradient.Stop(color: color.opacity(0.92 * eased), location: t)
        }
    }
}

#if DEBUG
/// A system menu's items listed in a sheet, for a drive step's screenshot.
private struct MenuSampleSheet: View {
    let sample: MenuSample

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: sample.title)
            VStack(spacing: 0) {
                ForEach(Array(sample.items.enumerated()), id: \.offset) { index, item in
                    HStack {
                        Text(item.label).font(.snRowTitle)
                        Spacer()
                        Image(systemName: item.symbol).foregroundStyle(SN.textSecondary)
                    }
                    .padding(.horizontal, 16)
                    .frame(height: 48)
                    if index < sample.items.count - 1 { Divider().overlay(SN.borderSubtle) }
                }
            }
            .snPanel()
            .padding(16)
        }
        .fittedSheet()
    }
}
#endif
