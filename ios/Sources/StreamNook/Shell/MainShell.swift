import SNKit
import SwiftUI

/// The tabs (the system tab bar, one navigation stack each, all kept alive while another is in
/// front) with the watch layer over them. Landscape watching is always immersive: no status bar,
/// no home indicator.
struct MainShell: View {
    @Bindable var model: AppModel
    @State private var landscape = false
    @State private var insets = EdgeInsets()

    private var nav: Navigator { model.nav }

    var body: some View {
        @Bindable var nav = model.nav
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
                Tab(MobileTab.you.title, systemImage: Symbol.you, value: MobileTab.you) {
                    stack(.you) { YouScreen(model: model) }
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
    }

    @ViewBuilder
    private func destination(_ route: Route) -> some View {
        switch route {
        case .category(let category):
            CategoryScreen(model: model, category: category)
        case .settings(let section):
            SettingsPage(model: model, section: section)
        case .cosmetics:
            CosmeticsFrame(model: model)
        }
    }
}

extension EnvironmentValues {
    /// The full player covers the tabs: looping decorations in them stop.
    @Entry var shellCovered = false
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
