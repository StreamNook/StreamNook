import Foundation
import Observation
import os
import SNKit

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
    let look = Look.shared
    let home: HomeStore
    /// Browse's mode and search, kept here so they survive the tab being rebuilt.
    let browseState = BrowseModel()

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
    }

    var signedIn: Bool { signIn.user != nil }

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
        await signIn.boot(core: core)
        home.start()
        #if DEBUG
        if Drive.skipsSetup { setupClosed = true }
        #endif
        phase = settings?.setupComplete == false && !setupClosed ? .setup : .main
    }

    /// Reads the settings and applies the look. A failed read keeps the look it has.
    @discardableResult
    func loadSettings() async -> SettingsLook? {
        do {
            let settings = try await core.invoke("load_settings", args: [:], as: SettingsLook.self)
            look.apply(settings)
            return settings
        } catch {
            log.error("load_settings failed: \(String(describing: error), privacy: .public)")
            return nil
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
        await patchSettings(["setup_complete": true])
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
    }

    /// Closes the watch layer: the player lets go of its item before the core stops the relay.
    func stopWatching() {
        watch.close(core: core)
        nav.close()
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
    }

    private func listenForEvents() {
        let events = core.events.all()
        Task { [weak self] in
            for await event in events {
                guard let self else { return }
                self.eventCount += 1
                self.lastEvent = event.name
                if event.name == "streamnook-settings-updated" { await self.loadSettings() }
            }
        }
    }
}
