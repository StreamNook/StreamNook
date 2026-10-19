import Foundation
import Observation
import os
import SNKit
import SwiftUI

let log = Logger(subsystem: "app.streamnook", category: "spike")

/// A clip link shown in the clip sheet; each tap is a fresh sheet.
struct ClipLink: Identifiable {
    let id = UUID()
    let url: URL
}

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
    /// Your Kick account: its sign-in and your Kick follows.
    let kick = KickAccount()
    /// The mentions inbox: its rows, the unread count, and the notifications the core asks for.
    let mentions: MentionsStore

    private(set) var phase = Phase.booting
    /// A clip link from chat, open in the clip sheet.
    var clip: ClipLink?
    /// The Create clip sheet, from the player's Clip button or `/clip`.
    var clipTarget: ClipTarget?
    /// The sleep timer as the core runs it (the player's menu and the moon in the header).
    private(set) var sleepTimer: SleepTimerState?
    /// A playing VOD's recorded chat, fed by the core in step with the player.
    private(set) var replay: ChatSession?
    /// The chat pane shows the replay (the default for a VOD) rather than the channel's live chat.
    var replayShown = true
    /// The Channel sheet (a channel's videos and clips).
    var channelSheet: TwitchStream?
    private(set) var started = false
    private(set) var startError: String?
    private(set) var startMS: Int?
    private(set) var eventCount = 0
    private(set) var lastEvent: String?
    /// The wizard was closed without finishing; it stays away for the rest of this session.
    private var setupClosed = false
    /// The alerts this iPhone sends, as the core lists them: the Notifications page's "Notify me
    /// about" rows and the wizard's Notifications page.
    private(set) var alertTypes: [AlertType] = []

    let version = CoreHost.appVersion

    init() {
        home = HomeStore(core: core)
        settingsStore = SettingsStore(core: core)
        chat = ChatRooms(core: core)
        rewards = RewardsStore(core: core)
        mentions = MentionsStore(core: core)
        let watch = watch
        watch.nowPlaying.onPlay = { watch.player.play() }
        watch.nowPlaying.onPause = { watch.player.pause() }
        watch.nowPlaying.onStop = { [weak self] in self?.stopWatching() }
        watch.nowPlaying.onSeek = { watch.seekVod(to: $0) }
        watch.nowPlaying.onSkip = { watch.skipVod(by: $0) }
        watch.pip.onRestore = { [weak self] in
            guard let self else { return }
            withAnimation(SNMotion.run(SNMotion.settleBack)) { self.nav.playerMode = .full }
        }
        watch.pip.onClose = { [weak self] in self?.stopWatching() }
        watch.pip.onCloseToAudio = { watch.player.play() }
        watch.onRow = { [weak self] row in
            guard let self else { return }
            self.nav.refresh(row)
            // The stream's room learns the id, name and avatar a bare login lacked. Kick chat
            // reads without any sign-in.
            if self.signedIn || !row.isTwitch { self.chat.syncStream(row) }
        }
        let toasts = toasts
        watch.onToast = { toasts.show($0) }
        watch.onVodChange = { [weak self] vod in self?.vodChanged(vod) }
        watch.onReplay = { [weak self] step in self?.replay?.pipeline.applyReplay(step) }
        watch.onLive = { [weak self] live in self?.startWatching(live) }
        LinkRouter.clip = { [weak self] url in
            guard let self, LinkRouter.isClip(url) else { return false }
            self.clip = ClipLink(url: url)
            return true
        }
        ClipRouter.create = { [weak self] login, channelID, title in
            self?.createClip(login: login, channelID: channelID, title: title)
        }
        chat.toasts = toasts
        chat.kick = kick
        chat.mentions = mentions
        let home = home
        chat.followedLive = { home.lists.followingLive }
        chat.groupFor = { id in home.lists.collaborations[id] ?? home.lists.sharedChats[id] }
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
            let (dirs, key, codecs) = try await CoreHost.startInputs()
            try await Task.detached(priority: .userInitiated) {
                try core.start(dirs: dirs, version: CoreHost.appVersion, vaultKey: key, codecs: codecs)
                CoreHost.excludeSealedFromBackup()
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
        alertTypes = await AlertTypes.load(core: core)
        // The watch layer reads its landscape chat and player rows from the settings view.
        Task { await settingsStore.refresh() }
        capVideoHeight()
        await signIn.boot(core: core)
        host.start(core: core, cookies: signIn.cookies)
        home.start()
        cosmetics.start(core: core)
        kick.start(core: core, toasts: toasts)
        chat.start()
        let mentionCategory = alertTypes.first { $0.id == "mentions" }?.channel
        mentions.onNotify = { entry in Task { await MentionAlerts.post(entry, category: mentionCategory) } }
        mentions.start()
        #if DEBUG
        if Drive.skipsSetup { setupClosed = true }
        #endif
        let boot = try? await core.invoke("get_boot_state", args: [:], as: BootState.self)
        let needsSetup = boot?.needsSetup ?? (settings?.setupComplete == false)
        phase = needsSetup && !setupClosed ? .setup : .main
        // A mention tapped while the app started opens once the tabs are up.
        NotificationRouter.handle { [weak self] id in
            guard let self else { return }
            Task { await self.openMention(id) }
        }
        // Go-lives: the channel playing gets no banner, and a tap plays the stream.
        NotificationRouter.isPlaying = { [weak self] alert in self?.isPlaying(alert) ?? false }
        NotificationRouter.handleLive { [weak self] alert in self?.openLiveAlert(alert) }
        Task { await settingsStore.refresh(); await LiveAlerts.sync(self) }
    }

    /// Reads the settings, applies the look and hands the player its saved choices. A failed read
    /// keeps what it has.
    @discardableResult
    func loadSettings() async -> SettingsLook? {
        do {
            let json = try await core.invoke("load_settings", args: [:])
            let settings = try JSONDecoder().decode(SettingsLook.self, from: Data(json.utf8))
            look.apply(settings)
            let core = core
            Task { await Typeface.loadCustom(fontID: settings.font, family: settings.fontCustom, core: core) }
            await loadTheme()
            if let prefs = try? JSONDecoder().decode(PlayerPrefs.self, from: Data(json.utf8)) {
                watch.savedQuality = prefs.quality
                watch.pip.keepsAudio = prefs.keepsAudio
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
            look.apply(palette: Palette(tokens: tokens), themeID: theme["id"] as? String ?? "winters-glass",
                       light: (theme["light"] as? [String: Any]).flatMap(ThemeLight.init(json:)))
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

    /// Opens the watch layer on `stream`; another stream already open switches in place. Chat
    /// opens first, so a playback failure never leaves the viewer with nothing.
    func startWatching(_ stream: TwitchStream) {
        // Twitch chat needs the account; signing in later points the stream tab then (see
        // `ChatArea`). Kick chat reads without one.
        if signedIn || !stream.isTwitch { chat.syncStream(stream) }
        nav.open(stream)
        // An offline Twitch channel opens its room; the core starts the stream if it is live
        // after all. Kick has no room: its exact search can return an offline channel, which
        // starts like any other and says so if it cannot play.
        if stream.isOffline && stream.isTwitch {
            watch.openRoom(stream, core: core)
        } else {
            watch.open(stream, core: core)
        }
    }

    /// Plays one of `channel`'s videos (from its Channel sheet): the channel's chat opens with it,
    /// showing the recorded chat in step with the video.
    func openVod(videoID: String, title: String, thumbnail: String?, channel: TwitchStream, at: Double? = nil) {
        channelSheet = nil
        if signedIn { chat.syncStream(channel) }
        nav.open(channel)
        watch.openVod(videoID: videoID, title: title, thumbnail: thumbnail, channel: channel, at: at, core: core)
    }

    /// A VOD began or ended: its replay room opens with it, and goes with it.
    private func vodChanged(_ vod: VodInfo?) {
        replay?.stop()
        guard let vod else {
            replay = nil
            return
        }
        let login = vod.channelLogin ?? watch.stream?.userLogin ?? ""
        replay = ChatSession(channel: login, core: core, source: .replay, roomID: watch.stream?.userID)
        replayShown = true
    }

    /// Switches the playing stream's quality (the relay restarts on its port) and saves the pick
    /// as the intent. A VOD switches at its playhead. A failure leaves playback as it was.
    func changeQuality(_ quality: String) async {
        guard let stream = watch.stream else { return }
        let vod = watch.playingVod
        let resumeAt = watch.vodPosition
        let url = vod.map { "https://www.twitch.tv/videos/\($0.videoID)" } ?? stream.playbackURL
        do {
            let start = try await core.invoke(
                "change_stream_quality", args: ["url": url, "quality": quality],
                as: StreamStart.self
            )
            guard watch.stream?.id == stream.id else { return }
            log.notice("quality \(quality, privacy: .public) -> \(start.quality, privacy: .public)")
            await watch.switched(to: start, picked: quality, core: core)
            if vod != nil { watch.seekVod(to: resumeAt) }
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
        guard let stream = watch.stream, !watch.resolvingOffline else { return }
        guard let failure = await watch.restart(core: core) else {
            toasts.show("Stream restarted with new settings", .success)
            return
        }
        log.error("restart failed: \(String(describing: failure), privacy: .public)")
        guard stream.isTwitch else {
            toasts.show("Failed to restart stream")
            return
        }
        do {
            let online = try await core.invoke("check_stream_online", args: ["userLogin": stream.userLogin],
                                               as: TwitchStream?.self)
            if online == nil {
                watch.openRoom(stream, core: core)
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
    /// Opens Create clip for `login`'s room. The watched channel clips what plays (a VOD at its
    /// playhead); any other room clips its live stream.
    func createClip(login: String, channelID: String, title: String = "") {
        guard !login.hasPrefix("kick:") else {
            toasts.show("Clips work on Twitch channels")
            return
        }
        let bare = login.lowercased()
        if let stream = watch.stream, stream.isTwitch, stream.userLogin.lowercased() == bare {
            var target = ClipTarget(channelID: stream.userID ?? channelID, login: stream.userLogin, name: stream.userName,
                                    title: title.isEmpty ? (watch.live?.title ?? stream.title) : title)
            if let vod = watch.playingVod {
                target.vodID = vod.videoID
                target.vodOffset = watch.player.currentTime().seconds
            }
            clipTarget = target
        } else {
            clipTarget = ClipTarget(channelID: channelID, login: bare, name: login, title: title)
        }
    }

    func startSleepTimer(minutes: Int) async {
        if let state = try? await core.invoke("sleep_timer_start", args: ["minutes": minutes], as: SleepTimerState.self) {
            sleepTimer = state
        }
    }

    func cancelSleepTimer() async {
        if let state = try? await core.invoke("sleep_timer_cancel", args: [:], as: SleepTimerState.self) {
            sleepTimer = state
        }
        watch.cancelFade()
    }

    func refreshSleepTimer() async {
        if let state = try? await core.invoke("sleep_timer_state", args: [:], as: SleepTimerState.self) {
            sleepTimer = state
        }
    }

    /// The core's timer moved: the audio fades for its last seconds, then playback stops as if
    /// the viewer had closed it (the core already stopped the relay; this lets go of the player,
    /// the lock screen card and background audio).
    private func sleepTimerChanged(_ state: SleepTimerState) {
        sleepTimer = state
        switch state.phase {
        case "fading": watch.fadeOut(seconds: state.fadeSecs)
        case "fired":
            log.notice("sleep timer fired; stopping playback")
            if watch.stream != nil { stopWatching() }
        default: watch.cancelFade()
        }
    }

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
            // Notifications may have been allowed in iPhone Settings meanwhile.
            Task { await LiveAlerts.sync(self) }
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
                case "sleep-timer://state":
                    if let state = SleepTimerState.parse(event.payloadJSON) { self.sleepTimerChanged(state) }
                case "app-foregrounded":
                    let payload = Self.object(event.payloadJSON)
                    log.notice("foregrounded: chat port \(payload["chat_port"] as? Int ?? 0) alive \(payload["chat_alive"] as? Bool ?? false) relay rebound \(payload["relay_rebound"] as? Bool ?? false)")
                    self.chat.foregrounded(port: payload["chat_port"] as? Int)
                    if let port = payload["chat_port"] as? Int {
                        #if DEBUG
                        ChatLab.session?.reconnect(port: port)
                        #endif
                    }
                case "watch-session://redirect":
                    let json = event.payloadJSON
                    Task { await self.followRaid(json) }
                case "watch-session://offline":
                    let payload = Self.object(event.payloadJSON)
                    if let provider = payload["provider"] as? String, provider != "twitch" {
                        // Two missed provider checks: the stream ended.
                        self.watch.providerEnded(provider: provider, channel: payload["login"] as? String ?? "")
                    } else {
                        // The confirmation polls Helix for up to 40 s; other events keep flowing.
                        Task { await self.streamWentOffline() }
                    }
                case "provider-stream-offline":
                    // The relay could not re-sign a Kick stream and Kick confirmed it ended.
                    let payload = Self.object(event.payloadJSON)
                    self.watch.providerEnded(provider: payload["provider"] as? String ?? "",
                                             channel: payload["channel"] as? String ?? "")
                case "watch-session://went-live":
                    // The room's channel went live: the stream starts in place, chat kept, unless
                    // its latest broadcast is playing, which is never cut off.
                    if let wentLive = try? JSONDecoder().decode(WentLive.self, from: Data(event.payloadJSON.utf8)),
                       self.watch.room?.login.lowercased() == wentLive.stream.userLogin.lowercased() {
                        if self.watch.roomVod {
                            let live = wentLive.stream
                            self.toasts.show("\(live.userName) is live", .info, action: "Watch") { [weak self] in
                                self?.startWatching(live)
                            }
                        } else {
                            self.startWatching(wentLive.stream)
                        }
                    }
                case "watch-session://waiting":
                    if let waiting = try? JSONDecoder().decode(RoomWaiting.self, from: Data(event.payloadJSON.utf8)) {
                        self.watch.roomWaiting(waiting)
                    }
                case "watch-session://resolving":
                    // The core is confirming an offline stream; a restart now would fight it.
                    self.watch.resolvingOffline = event.payloadJSON.trimmingCharacters(in: .whitespaces) == "true"
                case "watch-session://stream-update":
                    if let update = ChannelViewers.streamUpdate(event.payloadJSON) { self.watch.take(update) }
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

    /// A raid the core follows: a notice, a beat to read it, then the target starts from the
    /// raid's seed (ids, names); the start fills in the rest.
    private func followRaid(_ json: String) async {
        guard watch.stream != nil,
              let payload = try? JSONDecoder().decode(RaidRedirect.self, from: Data(json.utf8)) else { return }
        let target = payload.target
        log.notice("raid: joining \(target.userLogin, privacy: .public)")
        toasts.show("Raid starting! Joining \(target.userLogin)...", .info)
        try? await Task.sleep(for: .milliseconds(1500))
        guard watch.stream != nil else { return }
        startWatching(target)
    }

    /// The watched stream looks offline: the core confirms it and says what to do.
    private func streamWentOffline() async {
        guard let ended = watch.stream else { return }
        guard let decision = try? await core.invoke("watch_session_resolve_offline", args: [:], as: OfflineDecision.self)
        else {
            toasts.show("Auto-switch failed. Please select a new stream manually.")
            return
        }
        guard watch.stream?.userLogin == ended.userLogin else { return }
        log.notice("offline decision \(decision.action, privacy: .public) \(decision.why ?? "", privacy: .public)")
        let notice = { (n: OfflineDecision.Notice?) in
            if let n { self.toasts.show(n.message, n.error ? .error : .info) }
        }
        switch decision.action {
        case "notify":
            notice(decision.notice)
        case "offline_chat":
            // The video stops; chat stays. The phone has no offline room screen yet.
            _ = try? await core.invoke("stop_stream", args: [:])
            watch.openRoom(ended, core: core)
        case "stop":
            notice(decision.notice)
            stopWatching()
        case "switch_to":
            guard let next = decision.stream else { return }
            notice(decision.notice)
            try? await Task.sleep(for: .milliseconds(500))
            startWatching(next)
        default:
            break
        }
    }

    private static func object(_ json: String) -> [String: Any] {
        (try? JSONSerialization.jsonObject(with: Data(json.utf8))) as? [String: Any] ?? [:]
    }
}
