import Foundation
import Observation
import os
import SNKit
import SwiftUI

let log = Logger(subsystem: "app.streamnook", category: "spike")

/// The app's state: the core, where boot stands, the look, navigation, the Twitch account, the
/// home snapshot and the stream being watched.
@Observable @MainActor
final class AppModel {
    /// The root view's state: the boot loader, the first-run wizard, then the tabs.
    enum Phase: Equatable { case booting, setup, main }

    let core: CoreBridge = CoreHost.makeCore()
    let watch = Watch()
    let signIn = SignIn()
    let nav = Navigator()
    let toasts = Toasts()
    let host = HostRequests()
    let look = Look.shared
    let home: HomeStore
    /// What the settings pages read and write, one field at a time.
    let settingsStore: SettingsStore
    /// Browse's mode and search, kept here so they survive the tab being rebuilt.
    let browseState = BrowseModel()
    /// Profiles and cosmetics: the Atmosphere catalog, the profile sheet, the Cosmetics screen.
    let cosmetics = CosmeticsStore()
    /// The open chat rooms: the watched stream's and those opened beside it.
    let chat: ChatRooms
    /// The Rewards tab: drops, the badge wall, and the drop bar's jump target.
    let rewards: RewardsStore

    private(set) var phase = Phase.booting
    private(set) var started = false
    private(set) var startError: String?
    private(set) var startMS: Int?
    private(set) var eventCount = 0
    private(set) var lastEvent: String?
    /// The wizard was closed without finishing; it stays away for the rest of this session.
    private var setupClosed = false

    let version = CoreHost.appVersion

    init() {
        home = HomeStore(core: core)
        settingsStore = SettingsStore(core: core)
        chat = ChatRooms(core: core)
        rewards = RewardsStore(core: core)
        let watch = watch
        watch.nowPlaying.onPlay = { watch.player.play() }
        watch.nowPlaying.onPause = { watch.player.pause() }
        watch.nowPlaying.onStop = { [weak self] in self?.stopWatching() }
        watch.pip.onRestore = { [weak self] in
            guard let self else { return }
            withAnimation(SNMotion.run(SNMotion.settleBack)) { self.nav.playerMode = .full }
        }
        watch.pip.onClose = { [weak self] in self?.stopWatching() }
        chat.toasts = toasts
        let home = home
        chat.followedLive = { home.lists.followedLive }
    }

    var signedIn: Bool { signIn.signedIn }

    /// The Browse list, which drive steps open streams from.
    var streams: [TwitchStream] { home.recommended }

    /// Starts the core once, reads the settings, then who is signed in, then picks the first
    /// screen. The loader always drops, whatever failed.
    func boot() async {
        guard !started, startError == nil else { return }
        defer { if phase == .booting { phase = .main } }
        listenForEvents()
        let core = core
        let clock = ContinuousClock.now
        do {
            let (dirs, key, codecs) = try CoreHost.startInputs()
            try await Task.detached(priority: .userInitiated) {
                try core.start(dirs: dirs, version: CoreHost.appVersion, vaultKey: key, codecs: codecs)
            }.value
            startMS = Int((ContinuousClock.now - clock) / .milliseconds(1))
            started = true
            log.notice("core started (\(core.kind, privacy: .public)) in \(self.startMS ?? 0) ms, codecs \(codecs.joined(separator: ","), privacy: .public)")
        } catch {
            startError = "\(error)"
            log.error("core start failed: \(String(describing: error), privacy: .public)")
            return
        }
        let settings = await loadSettings()
        capVideoHeight()
        await signIn.boot(core: core)
        host.start(core: core, cookies: signIn.cookies)
        home.start()
        cosmetics.start(core: core)
        chat.start()
        #if DEBUG
        if Drive.skipsSetup { setupClosed = true }
        #endif
        let boot = try? await core.invoke("get_boot_state", args: [:], as: BootState.self)
        let needsSetup = boot?.needsSetup ?? (settings?.setupComplete == false)
        phase = needsSetup && !setupClosed ? .setup : .main
    }

    /// Reads the settings, applies the look and hands the player its saved choices. A failed read
    /// keeps what it has.
    @discardableResult
    func loadSettings() async -> SettingsLook? {
        do {
            let json = try await core.invoke("load_settings", args: [:])
            let settings = try JSONDecoder().decode(SettingsLook.self, from: Data(json.utf8))
            look.apply(settings)
            await loadTheme()
            if let prefs = try? JSONDecoder().decode(PlayerPrefs.self, from: Data(json.utf8)) {
                watch.savedQuality = prefs.quality
                watch.pip.floats = prefs.floatsVideo
            }
            return settings
        } catch {
            log.error("load_settings failed: \(String(describing: error), privacy: .public)")
            return nil
        }
    }

    /// Paints the theme the core resolves from the saved choice (custom themes and the OLED
    /// accent included). A failed read keeps the palette in force.
    func loadTheme() async {
        do {
            let json = try await core.invoke("get_active_theme", args: [:])
            guard let theme = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any],
                  let tokens = theme["palette"] as? [String: Any] else { return }
            look.apply(palette: Palette(tokens: tokens), themeID: theme["id"] as? String ?? "winters-glass")
        } catch {
            log.error("get_active_theme failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// Writes top-level settings keys through the core, then reads the look back.
    func patchSettings(_ patch: [String: Any?]) async {
        do {
            _ = try await core.invoke("patch_settings", args: ["patch": patch.mapValues { $0 ?? NSNull() }])
            await loadSettings()
        } catch {
            toasts.show("Failed to save settings")
            log.error("patch_settings failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The wizard closed without finishing: on to the tabs for this session.
    func closeSetup() {
        setupClosed = true
        phase = .main
    }

    /// The wizard finished: the core remembers, and the tabs come up.
    func finishSetup() async {
        do {
            _ = try await core.invoke("complete_setup", "{}")
        } catch {
            log.error("complete_setup failed: \(String(describing: error), privacy: .public)")
        }
        await loadSettings()
        phase = .main
    }

    #if DEBUG
    func showSetup() { phase = .setup }
    #endif

    /// Loads Browse again (the drive step `browse`).
    func browse() async {
        guard started else { return }
        await home.loadRecommended()
    }

    /// Opens the watch layer on `stream`; another stream already open switches in place.
    func startWatching(_ stream: TwitchStream) {
        nav.open(stream)
        watch.open(stream, core: core)
        // Chat needs the account; signing in later points the stream tab then (see `ChatArea`).
        if signedIn { chat.syncStream(stream) }
    }

    /// Switches the playing stream's quality (the relay restarts on its port) and saves the pick
    /// as the intent. A failure leaves playback as it was.
    func changeQuality(_ quality: String) async {
        guard let stream = watch.stream else { return }
        do {
            let start = try await core.invoke(
                "change_stream_quality", args: ["url": "https://twitch.tv/\(stream.userLogin)", "quality": quality],
                as: StreamStart.self
            )
            guard watch.stream?.id == stream.id else { return }
            log.notice("quality \(quality, privacy: .public) -> \(start.quality, privacy: .public)")
            await watch.switched(to: start, picked: quality, core: core)
            await patchSettings(["quality": quality])
            toasts.show(quality == start.quality ? "Quality changed to \(start.quality)"
                                                 : "Quality \"\(quality)\" unavailable, switched to \"\(start.quality)\"",
                        quality == start.quality ? .success : .info)
        } catch {
            log.error("change_stream_quality failed: \(String(describing: error), privacy: .public)")
            toasts.show("Failed to change quality: \(error)")
        }
    }

    /// The manual unstick: the relay and player cycle for the same stream. If it cannot come back,
    /// an offline channel shows as offline; otherwise the stream starts afresh.
    func restartStream() async {
        guard let stream = watch.stream else { return }
        guard let failure = await watch.restart(core: core) else {
            toasts.show("Stream restarted with new settings", .success)
            return
        }
        log.error("restart failed: \(String(describing: failure), privacy: .public)")
        do {
            let online = try await core.invoke("check_stream_online", args: ["userLogin": stream.userLogin],
                                               as: TwitchStream?.self)
            if online == nil {
                watch.showOffline()
                return
            }
        } catch {
            log.error("check_stream_online failed: \(String(describing: error), privacy: .public)")
        }
        toasts.show("Failed to restart stream")
        watch.close(core: core)
        watch.open(stream, core: core)
    }

    /// Tells the core the tallest rendition this display can show, so "Auto" never pulls a source
    /// taller than the screen (an explicit tier ignores the cap).
    private func capVideoHeight() {
        let scene = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first
        let pixels = scene.map { Int(min($0.screen.nativeBounds.width, $0.screen.nativeBounds.height)) } ?? 0
        let height = ScreenCap.height(shortEdgePixels: pixels)
        guard height > 0 else { return }
        Task {
            _ = try? await core.invoke("set_max_video_height", args: ["height": height])
            log.notice("video height cap \(height) (screen short edge \(pixels) px)")
        }
    }

    /// Closes the watch layer: the player lets go of its item before the core stops the relay.
    func stopWatching() {
        watch.close(core: core)
        nav.close()
        // The stream's room closes; rooms opened beside it stay.
        chat.syncStream(nil)
    }

    func signOut() async {
        do {
            _ = try await core.invoke("sign_out_active_twitch_account", args: [:])
            await signIn.signedOut(core: core)
            home.signedOut()
        } catch {
            toasts.show("Failed to sign out. Please try again.")
        }
    }

    func lifecycle(_ state: String) {
        guard started else { return }
        core.lifecycle(state)
        if state == "background" { chat.backgrounded(inPiP: watch.pip.active) }
        if state == "foreground" {
            Task { await signIn.refresh() }
            Task { await home.resumed() }
        }
    }

    private func listenForEvents() {
        let events = core.events.all()
        Task { [weak self] in
            for await event in events {
                guard let self else { return }
                self.eventCount += 1
                self.lastEvent = event.name
                switch event.name {
                case "streamnook-settings-updated":
                    await self.loadSettings()
                    if self.settingsStore.loaded { await self.settingsStore.refresh() }
                case "app-foregrounded":
                    let payload = Self.object(event.payloadJSON)
                    log.notice("foregrounded: chat port \(payload["chat_port"] as? Int ?? 0) alive \(payload["chat_alive"] as? Bool ?? false) relay rebound \(payload["relay_rebound"] as? Bool ?? false)")
                    self.chat.foregrounded(port: payload["chat_port"] as? Int)
                    if let port = payload["chat_port"] as? Int {
                        #if DEBUG
                        ChatLab.session?.reconnect(port: port)
                        #endif
                    }
                case "ad-pivot":
                    let payload = Self.object(event.payloadJSON)
                    if let url = payload["url"] as? String {
                        self.watch.pivot(url: url, channel: payload["channel"] as? String)
                    }
                default:
                    break
                }
            }
        }
    }

    private static func object(_ json: String) -> [String: Any] {
        (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }
}
