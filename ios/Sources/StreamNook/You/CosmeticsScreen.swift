import Observation
import SNKit
import SwiftUI
import UIKit

/// Edit profile: how you look, pinned at the top and redrawn with every pick (your Atmosphere as
/// the band, your picture, your painted name and badges, and a line of your chat), then every
/// choice laid open: Atmosphere, your StreamNook badge, your 7TV paint and badge, your Twitch
/// badge, your name color, and your bio. Pushed from the You tab, the tab bar hidden.
struct CosmeticsScreen: View {
    let model: AppModel
    @State private var screen: CosmeticsScreenModel
    @State private var identity: OwnIdentityData?
    /// A picture just picked and the name as typed, shown in the pinned preview before Twitch has them.
    @State private var localPicture: UIImage?
    @State private var draftName = ""

    init(model: AppModel) {
        self.model = model
        _screen = State(initialValue: CosmeticsScreenModel(app: model))
    }

    private var store: CosmeticsStore { model.cosmetics }
    private var user: TwitchUser? { model.signIn.user }

    /// Paints and badges past this many wait behind "Show all".
    private static let shortList = 8

    var body: some View {
        ScrollViewReader { reader in
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    profileSection.id("profile")
                    atmosphereSection.id("atmosphere")
                    badgeSection.id("badge")
                    sevenTvSection.id("seventv")
                    twitchSection.id("twitch")
                    colorSection.id("color")
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 32)
            }
            .onChange(of: store.scrollTarget) { _, target in
                guard let target else { return }
                withAnimation { reader.scrollTo(target, anchor: .top) }
                store.scrollTarget = nil
            }
        }
        .safeAreaInset(edge: .top, spacing: 0) { pinned }
        .background(SN.background)
        .environment(\.themeLightOff, true)
        .navigationTitle("Edit profile")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .task {
            store.activeScreen = screen
            if let user, identity == nil {
                identity = try? await model.core.invoke("own_identity_header_cached", args: ["userId": user.id],
                                                        as: OwnIdentityData?.self)
            }
            await screen.load()
            // Fresh after the cache: the 7TV picture and the banner as they are now.
            if let user, let fresh = try? await model.core.invoke("own_identity_header", args: [
                "userId": user.id, "login": user.login, "displayName": user.displayName,
            ], as: OwnIdentityData.self) {
                identity = fresh
            }
        }
        .sheet(isPresented: $screen.editingBio) {
            BioEditor(initial: screen.bio ?? "") { text in await screen.saveBio(text) }
                .tapsThroughKeyboard()
        }
        .sheet(isPresented: $screen.showSevenTvSignIn) {
            SevenTvSignInSheet(core: model.core, twitchID: model.signIn.user?.id ?? "") { stored in
                Task { await screen.sevenTvSignInFinished(stored) }
            }
        }
    }

    // MARK: Pinned preview

    /// You as others see you, held at the top while the choices scroll under it.
    @ViewBuilder
    private var pinned: some View {
        if let user {
            let look = (screen.sn?.theme).flatMap { $0 == "tier" ? nil : AtmosphereCatalog.shared.look($0) }
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .center, spacing: 12) {
                    CutoutAvatar(platformURL: user.profileImageURL, picture: identity?.picture, size: 56, local: localPicture)
                    HStack(spacing: 6) {
                        PaintedText(text: draftName.isEmpty ? user.displayName : draftName, font: .sn(19, 700), paint: screen.ext?.paints.first(where: \.worn)?.paint ?? identity?.paint,
                                    fallback: nameColor(screen.ext?.chatColor), shadows: .all)
                            .lineLimit(1)
                            .fixedSize()
                            .layoutPriority(1)
                        ForEach(wornBadgeURLs, id: \.self) { url in
                            PreviewBadge(url: url)
                        }
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, 12)
                .padding(.top, 12)
                if screen.sn != nil || screen.ext != nil {
                    ProfilePreviewRow(name: draftName.isEmpty ? user.displayName : draftName, picks: screen,
                                      fallbackPaint: identity?.paint, framed: false)
                        .padding(.horizontal, 12)
                        .padding(.top, 12)
                }
            }
            .padding(.bottom, 12)
            .background(alignment: .top) {
                AtmosphereGlow(look: look, tint: nameColor(screen.ext?.chatColor))
                    .frame(height: 120)
                    .animation(.easeOut(duration: 0.25), value: screen.sn?.theme)
            }
            .clipShape(RoundedRectangle(cornerRadius: 18, style: .continuous))
            .snPanel(radius: 18)
            // Pinned over moving content, so it is solid at any Glassiness: choices pass behind it,
            // never through it.
            .background { RoundedRectangle(cornerRadius: 18, style: .continuous).fill(SN.background) }
            .shadow(color: .black.opacity(0.55), radius: 12, y: 8)
            .padding(.horizontal, 16)
            .padding(.top, 4)
            .padding(.bottom, 8)
            .background {
                // The page fades in under the card, so choices scrolling up pass behind it.
                LinearGradient(stops: [.init(color: SN.background, location: 0.7), .init(color: SN.background.opacity(0), location: 1)],
                               startPoint: .top, endPoint: .bottom)
                    .ignoresSafeArea(edges: .top)
            }
        }
    }

    /// The badges you wear, in chat's order: Twitch, 7TV, StreamNook.
    private var wornBadgeURLs: [String] {
        [screen.ext?.twitchSelectable.first(where: \.isSelected)?.imageURL,
         screen.ext?.sevenTvBadges.first(where: \.worn)?.badge.url,
         screen.sn?.badges.first(where: \.worn)?.imageURL].compactMap { $0 }
    }

    private func header(_ title: String, platform: PlatformMark.Platform? = nil, streamNook: Bool = false,
                        detail: String? = nil) -> some View {
        ProfileSectionHeader(title, platform: platform, streamNook: streamNook, detail: detail)
            .padding(.top, 26)
            .padding(.bottom, 12)
    }

    // MARK: Atmosphere

    @ViewBuilder
    private var atmosphereSection: some View {
        header("Atmosphere", streamNook: true)
        if let state = screen.sn {
            ScrollView(.horizontal) {
                HStack(alignment: .top, spacing: 12) {
                    ForEach(state.atmospheres) { atmosphere in
                        Button { Task { await screen.equipTheme(atmosphere) } } label: {
                            AtmosphereTile(atmosphere: atmosphere,
                                           chosen: atmosphere.worn || (atmosphere.id == "tier" && !state.atmospheres.contains(where: \.worn)))
                        }
                        .buttonStyle(BadgePress())
                        .disabled(!atmosphere.allowed)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 6)
            }
            .scrollIndicators(.hidden)
            .padding(.horizontal, -16)
            .padding(.vertical, -6)
            // Always here, so picking None never moves what is below; off and dimmed while nothing
            // can show in chat.
            Toggle(isOn: Binding(get: { state.showChatToggle && state.chatAtmosphere },
                                 set: { show in Task { await screen.setChatAtmosphere(show) } })) {
                Text("Show in my chat messages").font(.snRowTitle).foregroundStyle(SN.textPrimary)
            }
            .tint(SN.accent)
            .disabled(!state.showChatToggle)
            .opacity(state.showChatToggle ? 1 : 0.4)
            .padding(.horizontal, 14)
            .frame(minHeight: 50)
            .snPanel()
            .padding(.top, 12)
        }
    }

    // MARK: StreamNook badge

    @ViewBuilder
    private var badgeSection: some View {
        let badges = screen.sn?.badges ?? []
        header("Badge", streamNook: true, detail: badges.first(where: \.worn)?.name)
        if screen.sn != nil {
            if badges.isEmpty {
                note("No badges yet. Earned and event badges land here.")
            } else {
                tileGrid(badges, key: "snBadges", worn: \.worn) { badge in
                    Button { Task { await screen.equipBadge(badge) } } label: {
                        BadgeTile(url: badge.imageURL, worn: badge.worn)
                    }
                    .buttonStyle(BadgePress())
                    .accessibilityLabel(badge.name)
                    .accessibilityAddTraits(badge.worn ? .isSelected : [])
                }
            }
        }
    }

    // MARK: 7TV

    @ViewBuilder
    private var sevenTvSection: some View {
        if let ext = screen.ext, !ext.paints.isEmpty || !ext.sevenTvBadges.isEmpty || !ext.sevenTvSignedIn {
            header("Paint", platform: .sevenTV, detail: ext.paints.first(where: \.worn)?.name)
            if !ext.sevenTvSignedIn {
                connectSevenTv(paints: ext.paints.count)
                    .padding(.bottom, ext.paints.isEmpty ? 0 : 10)
            }
            if !ext.paints.isEmpty {
                let all = store.openSections.contains("paints")
                LazyVGrid(columns: [GridItem(.flexible(), spacing: 8), GridItem(.flexible(), spacing: 8)], spacing: 8) {
                    ForEach(all ? ext.paints : Self.short(ext.paints, worn: \.worn)) { paint in
                        Button { Task { await screen.equipPaint(paint) } } label: {
                            PaintTile(paint: paint)
                        }
                        .buttonStyle(BadgePress())
                        .accessibilityAddTraits(paint.worn ? .isSelected : [])
                    }
                }
                showAll(count: ext.paints.count, key: "paints")
            }
            if !ext.sevenTvBadges.isEmpty {
                header("Badge", platform: .sevenTV, detail: ext.sevenTvBadges.first(where: \.worn)?.badge.title)
                tileGrid(ext.sevenTvBadges, key: "seventvBadges", worn: \.worn) { entry in
                    Button { Task { await screen.equipSevenTvBadge(entry) } } label: {
                        BadgeTile(url: entry.badge.url, fallbacks: entry.badge.fallbacks, worn: entry.worn)
                    }
                    .buttonStyle(BadgePress())
                    .accessibilityLabel(entry.badge.name ?? entry.badge.title)
                    .accessibilityAddTraits(entry.worn ? .isSelected : [])
                }
            }
        }
    }

    /// Signed out of 7TV: what connecting does, and the connect, in 7TV's own blue.
    private func connectSevenTv(paints: Int) -> some View {
        HStack(spacing: 12) {
            BrandTile(platform: .sevenTV, size: 40)
            Text(paints > 0 ? "Connect 7TV to switch paints. You own \(paints)."
                            : "Connect 7TV to wear your paints and badges.")
                .font(.sn(13.5))
                .foregroundStyle(SN.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button { screen.signInToSevenTv() } label: {
                Text(screen.signingIn ? "Connecting" : "Connect")
                    .font(.sn(13, 700))
                    .foregroundStyle(Color(red: 0.02, green: 0.13, blue: 0.18))
                    .padding(.horizontal, 14)
                    .frame(height: 32)
                    .background(Capsule().fill(FixedColor.sevenTV.color))
                    .frame(minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(screen.signingIn)
            .opacity(screen.signingIn ? 0.6 : 1)
        }
        .padding(12)
        .snPanel()
    }

    // MARK: Twitch

    @ViewBuilder
    private var twitchSection: some View {
        if let ext = screen.ext {
            if !ext.twitchSelectable.isEmpty {
                header("Badge", platform: .twitch, detail: ext.twitchSelectable.first(where: \.isSelected)?.title)
                tileGrid(ext.twitchSelectable, key: "twitch", worn: \.isSelected) { badge in
                    Button { Task { await screen.equipTwitchBadge(badge) } } label: {
                        BadgeTile(url: badge.imageURL, worn: badge.isSelected)
                    }
                    .buttonStyle(BadgePress())
                    .accessibilityLabel(badge.title)
                    .accessibilityAddTraits(badge.isSelected ? .isSelected : [])
                }
            } else if !ext.twitchEarned.isEmpty {
                header("Badges", platform: .twitch)
                VStack(alignment: .leading, spacing: 10) {
                    FlowLayout(spacing: 10) {
                        ForEach(ext.twitchEarned) { badge in
                            CosmeticImage(badge.url, fallbacks: badge.fallbacks, animate: true)
                                .frame(width: 28, height: 28)
                                .accessibilityLabel(badge.label)
                        }
                    }
                    if !ext.dropsSignedIn {
                        Text("Sign in to Drops to choose which badge you wear.")
                            .font(.snCaption)
                            .foregroundStyle(SN.textMuted)
                    }
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                .snPanel()
            }
        }
    }

    // MARK: Name color

    @ViewBuilder
    private var colorSection: some View {
        if let ext = screen.ext, !ext.namedColors.isEmpty {
            header("Name color", platform: .twitch,
                   detail: ext.namedColors.first(where: { $0.hex.lowercased() == ext.chatColor?.lowercased() })?.label)
            LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 6), count: 8), spacing: 12) {
                ForEach(ext.namedColors) { color in
                    let on = ext.chatColor?.lowercased() == color.hex.lowercased()
                    Button { Task { await screen.setChatColor(color) } } label: {
                        Circle()
                            .fill(nameColor(color.hex))
                            .frame(width: 30, height: 30)
                            .overlay { if on { Circle().strokeBorder(SN.background, lineWidth: 2.5) } }
                            .overlay { if on { Circle().strokeBorder(SN.accent, lineWidth: 2).padding(-3) } }
                            .frame(maxWidth: .infinity, minHeight: 36)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(BadgePress())
                    .accessibilityLabel(color.label)
                    .accessibilityAddTraits(on ? .isSelected : [])
                }
            }
            .padding(12)
            .snPanel()
        }
    }

    // MARK: Twitch profile

    @ViewBuilder
    private var profileSection: some View {
        if let user {
            header("Profile", platform: .twitch)
            TwitchProfileSection(model: model, screen: screen, user: user, identity: identity,
                                 picture: $localPicture, draftName: $draftName)
        }
    }

    // MARK: Pieces

    private func note(_ text: String) -> some View {
        Text(text)
            .font(.sn(13.5))
            .foregroundStyle(SN.textSecondary)
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .snPanel()
    }

    /// Badge tiles five across, the first eight and then "Show all".
    @ViewBuilder
    private func tileGrid<Item: Identifiable, Tile: View>(_ items: [Item], key: String, worn: @escaping (Item) -> Bool,
                                                         @ViewBuilder tile: @escaping (Item) -> Tile) -> some View {
        let all = store.openSections.contains(key)
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 12), count: 5), spacing: 12) {
            ForEach(all || items.count <= Self.shortList + 2 ? items : Self.short(items, worn: worn)) { item in
                tile(item)
            }
        }
        if items.count > Self.shortList + 2 { showAll(count: items.count, key: key) }
    }

    /// The first eight, with the one you wear swapped into the last place when it sits further down,
    /// so what you have on is always in view.
    private static func short<Item>(_ items: [Item], worn: (Item) -> Bool) -> [Item] {
        var shown = Array(items.prefix(shortList))
        if !shown.contains(where: worn), let wearing = items.first(where: worn), !shown.isEmpty {
            shown[shown.count - 1] = wearing
        }
        return shown
    }

    @ViewBuilder
    private func showAll(count: Int, key: String) -> some View {
        if count > Self.shortList + 2 {
            let open = store.openSections.contains(key)
            Button {
                withAnimation(.timingCurve(0.16, 1, 0.3, 1, duration: 0.26)) {
                    if open { store.openSections.remove(key) } else { store.openSections.insert(key) }
                }
            } label: {
                Text(open ? "Show fewer" : "Show all \(count)")
                    .font(.sn(14, 600))
                    .foregroundStyle(SN.accent)
                    .frame(maxWidth: .infinity, minHeight: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.top, 2)
            .padding(.bottom, -12)
        }
    }
}

/// A badge tile: the art at 36 points on a panel, the worn one ringed and checked. The name is the
/// section header's detail and VoiceOver's label, so the tile carries no words.
private struct BadgeTile: View {
    let url: String?
    var fallbacks: [String] = []
    let worn: Bool

    var body: some View {
        Group {
            if let url {
                CosmeticImage(url, fallbacks: fallbacks, animate: true).frame(width: 36, height: 36)
            } else {
                RoundedRectangle(cornerRadius: 6).fill(SN.surface).frame(width: 36, height: 36)
            }
        }
        .frame(maxWidth: .infinity)
        .aspectRatio(1, contentMode: .fit)
        // A tile you can see on the page, so the grid's edge is the tiles', not the icons'.
        .background(RoundedRectangle(cornerRadius: 12, style: .continuous).fill(SN.backgroundTertiary))
        .overlay { RoundedRectangle(cornerRadius: 12, style: .continuous).stroke(SN.borderSubtle, lineWidth: 0.5) }
        .wornMarks(worn, radius: 12)
    }
}

/// An Atmosphere tile: its swatch with nothing on it, its name in a chin under it, the worn ring
/// and check, or the lock beside the name.
private struct AtmosphereTile: View {
    let atmosphere: PickerAtmosphere
    /// Worn, or None while nothing is.
    let chosen: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Group {
                if atmosphere.swatch.isEmpty {
                    SN.surface
                } else {
                    FillView(fill: atmosphere.swatch)
                }
            }
            .frame(width: 116, height: 66)
            .clipShape(RoundedRectangle(cornerRadius: SNRadius.thumb, style: .continuous))
            .wornMarks(chosen, radius: SNRadius.thumb)
            HStack(spacing: 4) {
                if !atmosphere.allowed {
                    Image(systemName: "lock.fill").font(.system(size: 10, weight: .semibold)).foregroundStyle(SN.textMuted)
                }
                Text(atmosphere.id == "tier" ? "None" : atmosphere.name)
                    .font(.sn(12.5, chosen ? 600 : 500))
                    .foregroundStyle(chosen ? SN.textPrimary : SN.textSecondary)
                    .lineLimit(1)
            }
            .frame(width: 116, alignment: .leading)
        }
        .opacity(atmosphere.allowed ? 1 : 0.45)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }
}

/// A 7TV paint as its own name drawn in it, on a panel; the worn one ringed and checked.
private struct PaintTile: View {
    let paint: PickerPaint

    var body: some View {
        PaintedText(text: paint.name, font: .sn(15, 700), paint: paint.paint, fallback: SN.textPrimary)
            .lineLimit(1)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 12)
            .frame(height: 46)
            .snPanel()
            .wornMarks(paint.worn)
    }
}

/// Writes your Twitch bio: Twitch keeps 300 characters, so the count shows as you type and Save
/// stays off past it.
private struct BioEditor: View {
    let initial: String
    let save: (String) async -> Bool
    @State private var text = ""
    @State private var saving = false
    @Environment(\.dismiss) private var dismiss
    private let limit = 300

    var body: some View {
        NavigationStack {
            VStack(alignment: .trailing, spacing: 8) {
                TextEditor(text: $text)
                    .keyboardHideButton()
                    .font(.sn(15))
                    .scrollContentBackground(.hidden)
                    .padding(10)
                    .frame(minHeight: 140)
                    .snPanel(light: nil)
                Text("\(text.count)/\(limit)")
                    .font(.sn(12).monospacedDigit())
                    .foregroundStyle(text.count > limit ? SN.error : SN.textMuted)
            }
            .padding(16)
            .frame(maxHeight: .infinity, alignment: .top)
            .background(SN.background)
            .navigationTitle("Bio")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button(saving ? "Saving" : "Save") {
                        saving = true
                        Task {
                            if await save(text) { dismiss() }
                            saving = false
                        }
                    }
                    .disabled(saving || text.count > limit || text == initial)
                }
            }
        }
        .presentationDetents([.medium, .large])
        .onAppear { text = initial }
    }
}
