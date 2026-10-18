import Observation
import SNKit
import SwiftUI

/// A page pushed inside a tab.
enum Route: Hashable {
    /// A Browse category's live streams.
    case category(TwitchCategory)
    case settings(SettingsSection)
    case cosmetics
    /// The mentions inbox.
    case mentions
    /// You › Your clips.
    case myClips
}

/// The settings sections, in the You tab's order. One table drives the rows and the pages they
/// open, so a row and its page cannot drift apart.
enum SettingsSection: String, CaseIterable, Hashable, Identifiable {
    case player, adFree, chat, theme, notifications, cache, support, changed, backup, whatsNew

    var id: String { rawValue }

    /// The pages the You tab lists.
    static var listed: [SettingsSection] { allCases }

    var title: String {
        switch self {
        case .player: "Player"
        case .adFree: "Ad-free"
        case .chat: "Chat"
        case .theme: "Theme"
        case .notifications: "Notifications"
        case .cache: "Cache"
        case .support: "Support"
        case .changed: "Changed settings"
        case .backup: "Backup"
        case .whatsNew: "What's New"
        }
    }

    var detail: String {
        switch self {
        case .player: "Video quality, latency, and behavior"
        case .adFree: "How streams are served and which relays are used"
        case .chat: "Chat design and behavior"
        case .theme: "Color theme, glass, and fonts"
        case .notifications: "Mentions and replies while StreamNook is open"
        case .cache: "Emote, badge, and metadata caches"
        case .support: "Logs, diagnostics, and feedback"
        case .changed: "Everything you set differently from a fresh install"
        case .backup: "Daily copies, restore, and a file you can keep"
        case .whatsNew: "Recent releases"
        }
    }

    var symbol: String {
        switch self {
        case .player: Symbol.player
        case .adFree: Symbol.adFree
        case .chat: Symbol.chat
        case .theme: Symbol.theme
        case .notifications: Symbol.notifications
        case .cache: Symbol.cache
        case .support: Symbol.support
        case .changed: "slider.horizontal.below.square.and.square.filled"
        case .backup: "clock.arrow.circlepath"
        case .whatsNew: Symbol.whatsNew
        }
    }
}

/// Where the app is: the tab in front, each tab's pushed pages, and the stream being watched with
/// its layer's mode. The tab stacks stay alive under the watch layer, so scroll positions and
/// loaded pages survive watching.
@Observable @MainActor
final class Navigator {
    enum PlayerMode: Equatable { case full, mini }

    var tab: MobileTab
    var paths: [MobileTab: [Route]] = [:]
    /// The tab Chats was opened from: Chats is a full-screen surface with no tab bar, and leaving
    /// it (the grabber, a slide down) returns here.
    var chatsReturnTab = MobileTab.following

    /// Leaves Chats for the tab it was opened from.
    func exitChats() {
        guard tab == .chats else { return }
        tab = chatsReturnTab == .chats ? .following : chatsReturnTab
    }

    /// The stream in the watch layer, nil when nothing is open.
    private(set) var playing: TwitchStream?
    var playerMode = PlayerMode.full
    /// Landscape chat over the video: off each time a stream opens, kept across rotations.
    var landscapeChat = false
    /// The mini box's top-left corner as the person left it; clamped to the screen when drawn.
    var miniOrigin: CGPoint?

    // Drive steps' hooks. Kept outside `#if DEBUG` so Observation tracks them; release builds
    // never set them.
    /// A stand-in for a system menu, which only a finger can open.
    var menuSample: MenuSample?
    /// Bumped to show the player's controls, as a tap does.
    var controlsPing = 0
    /// Bumped to open the quality sheet, as the gear does.
    var qualityPing = 0
    /// The first-run wizard page to show.
    var setupPage: SetupWizard.Page?

    init() {
        tab = MobileTab.stored(UserDefaults.standard.string(forKey: "sn-start-tab"))
    }

    /// The full player covers the tabs: their decorations pause and the tab bar is out of sight.
    var shellCovered: Bool { playing != nil && playerMode == .full }

    func open(_ stream: TwitchStream) {
        if playing?.userLogin.lowercased() != stream.userLogin.lowercased() { landscapeChat = false }
        playing = stream
        playerMode = .full
    }

    func close() {
        playing = nil
        playerMode = .full
    }

    /// The core completed the playing row; the layer keeps its mode.
    func refresh(_ row: TwitchStream) {
        guard playing?.userLogin.lowercased() == row.userLogin.lowercased() else { return }
        playing = row
    }

    func push(_ route: Route) {
        paths[tab, default: []].append(route)
    }

    func path(_ tab: MobileTab) -> Binding<[Route]> {
        Binding(get: { self.paths[tab] ?? [] }, set: { self.paths[tab] = $0 })
    }
}

/// The items of a system menu, listed in a sheet so a drive step can screenshot them.
struct MenuSample: Identifiable {
    let id = UUID()
    let title: String
    let items: [(label: String, symbol: String)]
}
