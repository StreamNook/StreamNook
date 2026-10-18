import AVFoundation
import Foundation
import Observation
import SNKit
import UIKit

/// How the player reaches the relay. The core's low-latency origin runs on every route, so the
/// relay measures each one against the same live edge.
enum PlayRoute: String, Sendable {
    /// The whole-segment playlist (`plain.m3u8`): Twitch's segments with the target duration
    /// lowered to the real segment length.
    case plain
    /// The same with the in-progress segment promoted into the playlist.
    case promote
    /// The low-latency playlist through a resource loader, so AVPlayer can run Apple's
    /// low-latency mode without an HTTP/2 server; parts stay plain http to the relay.
    case ll
    /// The low-latency playlist over plain http, to see whether AVPlayer takes it there.
    case llhttp
    /// `ll` when the core's origin took the stream, `plain` when it did not.
    case auto
    /// The URL exactly as the core answered it (an audio-only master, a VOD).
    case direct
}

/// One stream being watched: the player on the URL the core serves and how far behind the live
/// edge it sits. Its chat is one of the open rooms (`ChatRooms`).
@Observable @MainActor
final class Watch {
    let player = AVPlayer()

    private(set) var stream: TwitchStream?
    private(set) var status = "Idle"
    private(set) var quality: String?
    /// The playing stream's start answer: how it is served (Settings > Ad-free reads it).
    private(set) var lastStart: StreamStart?
    private(set) var error: String?
    /// How far the player sits behind the relay's live edge, in milliseconds.
    private(set) var behindMS: Int?
    /// From the tap to the player first playing, in milliseconds.
    private(set) var firstPlayMS: Int?
    /// The route the player is on now (after any fallback).
    private(set) var playing: PlayRoute?
    /// Every quality the stream offers, in the core's order (`best` first); empty until it resolves.
    private(set) var available: [String] = []
    /// The quality picked in the sheet this session; nil until one is picked.
    private(set) var pickedQuality: String?
    /// The player is not playing (paused here, from the lock screen, or by a call). The center
    /// button follows the player, never only its own taps.
    private(set) var paused = false
    private(set) var muted = false
    /// A restart is cycling the relay and the player; its icon spins until the stream plays again.
    private(set) var restarting = false
    /// The watched channel's numbers as they move (viewers, title, category, start).
    private(set) var live: ChannelViewers?
    /// The Shared Viewership group the channel streams in, from the core's collab section.
    private(set) var group: ChannelGroup?
    /// The core is confirming the stream went offline (restart waits for its answer).
    var resolvingOffline = false
    /// The channel's offline room, while it is open (the band shows it instead of video).
    private(set) var room: OfflineRoom?
    /// The room's latest broadcast plays in the band.
    private(set) var roomVod = false
    /// A stale card: the room's channel is live after all, and the stream starts instead.
    var onLive: ((TwitchStream) -> Void)?

    /// The lock screen card and its remote commands.
    let nowPlaying = NowPlaying()
    let pip = PictureInPicture()
    /// The core completed the row (Following's own, or the seed with its gaps filled).
    var onRow: ((TwitchStream) -> Void)?
    /// A message for the toast line.
    var onToast: ((String) -> Void)?

    /// Set by drive steps before a stream opens; wins over every saved choice.
    var route = PlayRoute.auto
    /// A drive step's quality for `start_stream`: `best`, a variant name, or `audio_only`.
    var requestedQuality: String?
    /// The quality saved in settings (the intent last picked, not what was served).
    var savedQuality: String?
    var offsetFromLive: Double?
    var preservesOffset = false
    /// The distance behind the edge a gentle catch-up rate holds, in milliseconds.
    var catchUpTargetMS: Int?
    /// Part targets of hold-back the low-latency playlist asks AVPlayer for (the relay takes 2 or 3).
    /// Nil keeps the relay's default of three, Apple's recommendation (about 1.5 s behind live);
    /// the `hold` drive step can still ask for two.
    var holdParts: Int?

    private var tasks: [Task<Void, Never>] = []
    private var core: CoreBridge?
    private var opened: ContinuousClock.Instant?
    private var observers: [NSObjectProtocol] = []
    private var stalls = 0
    private var loader: LLPlaylistLoader?
    /// The relay URL `start_stream` answered, kept for a fallback to the whole-segment route.
    private var startURL: String?
    private var fellBack = false
    /// Reloads tried since the last first frame; a failure past `maxRecoveries` shows the error.
    private var recoveries = 0
    private static let maxRecoveries = 2
    private var watchers: [NSKeyValueObservation] = []
    private var sessionObservers: [NSObjectProtocol] = []
    /// What the core's link governor hears from this player (stalls, and downloads on the
    /// whole-segment route); it answers with `playback-adapt`.
    private var link = LinkReporter()
    private let linkEpoch = ContinuousClock.now

    init() {
        // KVO reports on whichever thread changed the value, so these are not main-actor code.
        watchers = [
            player.observe(\.timeControlStatus, options: [.initial, .new]) { @Sendable [weak self] player, _ in
                let status = player.timeControlStatus
                Task { @MainActor in self?.playerState(status) }
            },
            player.observe(\.isMuted, options: [.initial, .new]) { @Sendable [weak self] player, _ in
                let muted = player.isMuted
                Task { @MainActor in self?.muted = muted }
            },
        ]
        // A call or an alarm pauses the player; it plays on afterwards only when the system says
        // to resume.
        sessionObservers = [
            NotificationCenter.default.addObserver(forName: AVAudioSession.interruptionNotification, object: nil,
                                                   queue: .main) { [weak self] note in
                let type = (note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt)
                    .flatMap(AVAudioSession.InterruptionType.init(rawValue:))
                let options = (note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt)
                    .map(AVAudioSession.InterruptionOptions.init(rawValue:)) ?? []
                MainActor.assumeIsolated {
                    guard let self, self.stream != nil, type == .ended, options.contains(.shouldResume) else { return }
                    self.player.play()
                }
            },
            // iOS restarted its media services: every player item is dead. The stream comes back on
            // a fresh item from the same relay, with the audio session set up again.
            NotificationCenter.default.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: nil,
                                                   queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self, self.stream != nil, let url = self.startURL, let route = self.playing else { return }
                    log.notice("media services reset; reloading the stream")
                    try? self.activateAudio()
                    self.reloading()
                    self.load(url, route: route)
                }
            },
            // Leaving the app flushes a pause still waiting out its grace (the earning gate).
            NotificationCenter.default.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil,
                                                   queue: .main) { [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self, self.stream != nil else { return }
                    self.reportState("hidden")
                }
            },
        ]
    }

    /// The quality `start_stream` is asked for: a drive step's, else this session's pick, else
    /// the saved one, else `best` (capped to the screen by the core).
    private var askedQuality: String { requestedQuality ?? pickedQuality ?? savedQuality ?? "best" }

    /// Starts `stream`, replacing whatever played. The same channel already open stays as it is.
    /// The player is never rebuilt: only its item changes.
    func open(_ stream: TwitchStream, core: CoreBridge) {
        // The same channel already playing stays as it is; its room or a failure gives way.
        // Twitch `xqc` and Kick `xqc` are two channels.
        if self.stream?.rowKey == stream.rowKey, room == nil, error == nil { return }
        close(core: core, stopping: false)
        self.core = core
        self.stream = stream
        opened = .now
        status = "Resolving"
        showCard()
        tasks.append(Task { await self.play(stream, core: core) })
        tasks.append(Task { await self.followLink(core: core) })
    }

    // MARK: Controls

    func togglePlay() {
        if paused { player.play() } else { player.pause() }
    }

    func toggleMute() {
        player.isMuted.toggle()
    }

    /// The core switched the quality: the player reloads on the answer's URL, on the route it
    /// would pick for a fresh start, through the loading mark.
    func switched(to start: StreamStart, picked: String, core: CoreBridge) async {
        pickedQuality = picked
        savedQuality = picked
        apply(start)
        let chosen = await Self.route(for: start, asked: route, core: core)
        reloading()
        load(start.url, route: chosen)
    }

    /// Cycles the relay and the player for the same stream, quality and chat. Nil when the stream
    /// came back; the error otherwise (the caller decides between offline and a fresh start).
    func restart(core: CoreBridge) async -> Error? {
        guard let stream, !restarting else { return nil }
        restarting = true
        player.pause()
        _ = try? await core.invoke("stop_stream", "{}")
        try? await Task.sleep(for: .milliseconds(300))
        do {
            let start = try await core.invoke(
                "start_stream", args: ["url": stream.playbackURL, "quality": askedQuality],
                as: StreamStart.self
            )
            guard self.stream?.id == stream.id else { return nil }
            apply(start)
            let chosen = await Self.route(for: start, asked: route, core: core)
            log.notice("restart \(stream.userLogin, privacy: .public) -> \(start.url, privacy: .public) (\(start.quality, privacy: .public))")
            reloading()
            load(start.url, route: chosen)
            // The spin stops at the first frame; this only keeps it from spinning forever.
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(15))
                self?.restarting = false
            }
            return nil
        } catch {
            restarting = false
            return error
        }
    }

    /// The channel is offline: the player stops and says so.
    /// Opens the channel's offline room: the core answers its facts (or the live row for a stale
    /// card), stops whatever played and waits for the channel to go live. Chat stays.
    func openRoom(_ stream: TwitchStream, core: CoreBridge) {
        // Offline rooms are Twitch's; a Kick channel opens as a stream.
        guard stream.isTwitch else {
            open(stream, core: core)
            return
        }
        close(core: core, stopping: false)
        self.core = core
        self.stream = stream
        status = "Offline"
        room = OfflineRoom(login: stream.userLogin, displayName: stream.userName, avatarURL: stream.profileImageURL,
                           lastTitle: stream.title.isEmpty ? nil : stream.title,
                           lastCategory: stream.gameName.isEmpty ? nil : stream.gameName)
        tasks.append(Task { await self.loadRoom(stream, core: core) })
    }

    private func loadRoom(_ stream: TwitchStream, core: CoreBridge) async {
        guard let opened = try? await core.invoke("watch_open_room", args: ["login": stream.userLogin, "seed": stream.seed],
                                                  as: RoomOpen.self),
              !Task.isCancelled, !opened.superseded, self.stream?.userLogin == stream.userLogin else { return }
        if let live = opened.live {
            log.notice("room \(stream.userLogin, privacy: .public) is live after all")
            onLive?(live)
            return
        }
        if let found = opened.room { room = found }
        if let row = opened.row {
            self.stream = row
            onRow?(row)
        }
        log.notice("room \(stream.userLogin, privacy: .public) open, vod \(self.room?.latestVod != nil), next \(self.room?.nextStream != nil), waiting \(self.room?.chatters ?? -1)")
    }

    /// A Kick stream ended (the watch session's two missed checks, or the relay's re-sign
    /// failing): the player stops and the band says so; chat keeps running.
    func providerEnded(provider: String, channel: String) {
        guard let stream, !stream.isTwitch, stream.platform == provider.lowercased(),
              stream.userLogin.lowercased() == channel.lowercased() else { return }
        reportState("teardown")
        player.pause()
        player.replaceCurrentItem(with: nil)
        pip.eligible = false
        nowPlaying.clear()
        error = "\(stream.userName) went offline. Chat keeps running."
        status = "Offline"
        if let core { Task.detached { _ = try? await core.invoke("watch_stop", args: [:]) } }
    }

    /// The core's waiting count for the open room.
    func roomWaiting(_ update: RoomWaiting) {
        guard var room, room.login.lowercased() == update.login.lowercased() else { return }
        room.chatters = update.count
        self.room = room
    }

    /// The room's latest broadcast plays in the band; the room stays open behind it.
    func playRoomVod() {
        guard let vod = room?.latestVod, let core, !roomVod else { return }
        roomVod = true
        reloading()
        let asked = askedQuality
        tasks.append(Task {
            do {
                let start = try await core.invoke(
                    "start_stream", args: ["url": "https://twitch.tv/videos/\(vod.id)", "quality": asked], as: StreamStart.self
                )
                guard !Task.isCancelled, self.roomVod else { return }
                self.apply(start)
                try self.activateAudio()
                self.load(start.url, route: .direct)
                self.nowPlaying.show(title: vod.title, channel: self.stream?.userName ?? "",
                                     avatar: self.stream?.profileImageURL.flatMap(URL.init(string:)))
                await self.measure(core: core)
            } catch {
                guard !Task.isCancelled else { return }
                self.roomVod = false
                self.onToast?("Could not play the latest broadcast")
                log.error("room vod failed: \(String(describing: error), privacy: .public)")
            }
        })
    }

    /// Back from the latest broadcast to the room.
    func leaveRoomVod() {
        guard roomVod, let core else { return }
        roomVod = false
        reportState("teardown")
        player.pause()
        player.replaceCurrentItem(with: nil)
        pip.eligible = false
        nowPlaying.clear()
        available = []
        Task.detached { _ = try? await core.invoke("stop_stream", "{}") }
    }

    private func activateAudio() throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playback, mode: .moviePlayback)
        try session.setActive(true)
    }

    private func apply(_ start: StreamStart) {
        lastStart = start
        quality = start.quality
        startURL = start.url
        available = start.available
        fellBack = false
    }

    /// The next item plays through the loading mark again, as a first start does.
    private func reloading() {
        firstPlayMS = nil
        opened = .now
        status = "Buffering"
    }

    /// The player's own state drives the center button, the lock screen card, the earning gate
    /// (a rebuffer is not a pause) and keeping the screen awake while a stream plays.
    private func playerState(_ status: AVPlayer.TimeControlStatus) {
        // With no item yet (a start resolving) nothing is paused: the button keeps saying Pause.
        let paused = status == .paused && player.currentItem != nil
        self.paused = paused
        guard stream != nil else {
            ScreenAwake.hold("player", false)
            return
        }
        nowPlaying.playing(!paused)
        ScreenAwake.hold("player", !paused)
        switch status {
        case .playing: reportState("playing")
        case .paused: reportState("paused")
        default: break
        }
    }

    /// What the video does, for the core's earning gate (drops, channel points, watch time). The
    /// core waits out a pause's grace, so a reload's brief pause costs nothing.
    private func reportState(_ state: String) {
        guard let core else { return }
        Task.detached { _ = try? await core.invoke("report_player_state", args: ["state": state]) }
    }

    /// Publishes the lock screen card with what is known of the stream now.
    private func showCard() {
        guard let stream else { return }
        nowPlaying.show(title: live?.title ?? stream.title, channel: stream.userName,
                        avatar: stream.profileImageURL.flatMap(URL.init(string:)))
    }

    /// Keeps the header's numbers moving: the core polls the channel and pushes `channel-state`.
    /// A push never blanks a field it did not carry.
    private func follow(_ stream: TwitchStream, core: CoreBridge) async {
        guard let id = stream.userID, !id.isEmpty else { return }
        let pushes = core.events.stream("channel-state")
        if let first = try? await core.invoke("watch_channel_state", args: ["login": stream.userLogin, "channelId": id],
                                              as: ChannelViewers.self) {
            take(first)
        }
        for await event in pushes {
            guard !Task.isCancelled else { return }
            if let collab = ChannelGroup.collabPush(event.payloadJSON, login: stream.userLogin) {
                group = collab
                continue
            }
            guard let update = ChannelViewers.parse(event.payloadJSON),
                  update.login.lowercased() == stream.userLogin.lowercased() else { continue }
            take(update)
        }
    }

    /// Takes a push for the watched channel, never blanking a field it did not carry.
    func take(_ update: ChannelViewers) {
        let old = live
        live = ChannelViewers(login: update.login.isEmpty ? old?.login ?? "" : update.login, viewerCount: update.viewerCount ?? old?.viewerCount,
                              startedAt: update.startedAt ?? old?.startedAt, title: update.title ?? old?.title,
                              gameName: update.gameName ?? old?.gameName)
        if live?.title != old?.title { showCard() }
    }

    /// The core's link governor moved the stream on a poor link. A whole-segment switch is
    /// spliced into the playlist the player already polls, so only a URL (the relay restarted on
    /// another rendition, the low-latency route) needs a reload, on the route a fresh start would
    /// take.
    private func followLink(core: CoreBridge) async {
        for await event in core.events.stream("playback-adapt") {
            guard !Task.isCancelled else { return }
            guard let adapt = PlaybackAdapt.parse(event.payloadJSON), stream != nil else { continue }
            log.notice("link: \(adapt.quality, privacy: .public) (\(adapt.reason, privacy: .public)), cushion \(adapt.extraCushion)")
            guard let url = adapt.url else { continue }
            startURL = url
            let lowLatency = (try? await core.invoke("get_stream_low_latency", args: [:], as: Bool.self)) ?? false
            let chosen: PlayRoute = route == .auto ? (lowLatency ? .ll : .plain) : (playing ?? .plain)
            reloading()
            load(url, route: chosen)
        }
    }

    private func report(_ report: LinkReport, core: CoreBridge) {
        Task.detached { _ = try? await core.invoke("report_playback_health", args: report.args) }
    }

    /// The core moved playback to another upstream mid-stream (an ad-free pivot): the player
    /// reloads on the new URL, on the route it was on.
    func pivot(url: String, channel: String?) {
        guard let stream, channel == nil || channel?.lowercased() == stream.userLogin.lowercased() else { return }
        log.notice("ad pivot to \(url, privacy: .public)")
        startURL = url
        load(url, route: playing ?? .plain)
    }

    func pause() {
        logSummary()
        link.reset()
        player.pause()
        status = "Paused"
        log.notice("drive pause")
    }

    /// Ends the stream: the player lets go of its item before the core stops the relay. A switch to
    /// another stream (`stopping` false) leaves the stop to the next start, which supersedes it.
    func close(core: CoreBridge, stopping: Bool = true) {
        guard let login = stream?.userLogin else { return }
        // A pause still waiting out its grace must not gate the next stream off.
        reportState("teardown")
        ScreenAwake.hold("player", false)
        // A brightness the landscape dial borrowed goes back with the stream.
        ScreenBrightness.restore()
        lastStart = nil
        tasks.forEach { $0.cancel() }
        tasks = []
        observers.forEach(NotificationCenter.default.removeObserver)
        observers = []
        player.pause()
        player.replaceCurrentItem(with: nil)
        loader = nil
        stream = nil
        behindMS = nil
        firstPlayMS = nil
        stalls = 0
        quality = nil
        error = nil
        status = "Idle"
        startURL = nil
        playing = nil
        fellBack = false
        recoveries = 0
        available = []
        restarting = false
        live = nil
        group = nil
        resolvingOffline = false
        room = nil
        roomVod = false
        pip.eligible = false
        nowPlaying.clear()
        Task.detached {
            // The relay, the watch session, drops and channel points (services/watch_start.rs).
            if stopping { _ = try? await core.invoke("watch_stop", args: [:]) }
            _ = try? await core.invoke("unwatch_channel_state", args: ["login": login])
        }
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    // MARK: Playback

    /// One core call starts the stream: the relay, the completed row, drops and channel points
    /// aimed at the channel, and the watch session. A newer start or a stop supersedes it, and a
    /// superseded start touches nothing.
    private func play(_ stream: TwitchStream, core: CoreBridge) async {
        do {
            // A Kick row starts on Kick's path in the same Rust call (services/watch_start.rs).
            let started = try await core.invoke(
                "watch_start", args: ["login": stream.userLogin, "provider": stream.isTwitch ? nil : stream.platform,
                                      "seed": stream.seed, "quality": askedQuality],
                as: WatchStart.self
            )
            guard !Task.isCancelled, !started.superseded, let start = started.stream else {
                log.notice("watch_start \(stream.userLogin, privacy: .public) superseded")
                return
            }
            if let row = started.row { adopt(row, core: core) }
            apply(start)
            let chosen = await Self.route(for: start, asked: route, core: core)
            log.notice("watch_start \(stream.userLogin, privacy: .public) -> \(start.url, privacy: .public) (\(start.quality, privacy: .public)) route \(chosen.rawValue, privacy: .public)")
            try activateAudio()
            load(start.url, route: chosen)
            await measure(core: core)
        } catch {
            guard !Task.isCancelled else { return }
            log.error("watch_start failed: \(String(describing: error), privacy: .public)")
            await failed(stream, core: core)
        }
    }

    /// The row the core completed replaces the card's: the header, the lock screen card and the
    /// viewer count follow it. The channel's numbers can be followed once its id is known.
    private func adopt(_ row: TwitchStream, core: CoreBridge) {
        guard stream?.userLogin.lowercased() == row.userLogin.lowercased() else { return }
        stream = row
        onRow?(row)
        showCard()
        // The channel-state poll is Twitch's; a Kick stream's numbers move with the watch
        // session's provider check (`watch-session://stream-update`).
        if row.isTwitch { tasks.append(Task { await self.follow(row, core: core) }) }
    }

    /// Playback could not start, but chat runs on: an offline channel says so; a live one keeps
    /// the band's failure line and a toast saying chat still works.
    private func failed(_ stream: TwitchStream, core: CoreBridge) async {
        guard stream.isTwitch else {
            // Kick has no offline room: the band says why and chat keeps running. A channel Kick
            // already called offline (a search hit) is offline, not a playback failure.
            pip.eligible = false
            if stream.isOffline {
                error = "\(stream.userName) is offline. Chat keeps running."
                status = "Offline"
            } else {
                error = "Playback unavailable. Chat keeps running."
                status = "Failed"
                onToast?("Playback unavailable - showing chat only")
            }
            return
        }
        // An unanswerable check counts as live: the honest message is then about playback.
        var offline = false
        do {
            offline = try await core.invoke("check_stream_online", args: ["userLogin": stream.userLogin],
                                            as: TwitchStream?.self) == nil
        } catch {
            log.error("check_stream_online failed: \(String(describing: error), privacy: .public)")
        }
        guard !Task.isCancelled, self.stream?.userLogin == stream.userLogin else { return }
        if offline {
            openRoom(stream, core: core)
            return
        }
        error = "Playback unavailable. Chat keeps running."
        status = "Failed"
        pip.eligible = false
        onToast?("Playback unavailable - showing chat only")
    }

    /// Puts a new item for `url` on `route` in the player and plays it.
    private func load(_ url: String, route: PlayRoute) {
        guard let item = makeItem(url, route: route) else { return }
        if let offsetFromLive {
            item.configuredTimeOffsetFromLive = CMTime(seconds: offsetFromLive, preferredTimescale: 1000)
        }
        item.automaticallyPreservesTimeOffsetFromLive = preservesOffset
        observe(item)
        link.reset()
        playing = route
        error = nil
        player.replaceCurrentItem(with: item)
        player.play()
        pip.eligible = true
        if firstPlayMS == nil { status = "Buffering" }
    }

    /// The route for what `start_stream` answered: an audio-only master plays as it came; `auto`
    /// rides the low-latency playlist only while the core's origin serves the stream (it stands
    /// down under ad-free playback), the whole-segment one otherwise.
    static func route(for start: StreamStart, asked: PlayRoute, core: CoreBridge) async -> PlayRoute {
        if start.url.contains("/audio.m3u8") || !start.url.contains("/stream.m3u8") { return .direct }
        guard asked == .auto else { return asked }
        let lowLatency = (try? await core.invoke("get_stream_low_latency", args: [:], as: Bool.self)) ?? false
        return lowLatency ? .ll : .plain
    }

    /// The low-latency playlist failed to play (AVPlayer refuses some part layouts outright):
    /// the same relay serves the whole-segment playlist, so the player moves there once.
    private func fallBack(from failure: String) -> Bool {
        guard !fellBack, let startURL, let route = playing, route != .plain, route != .direct else { return false }
        fellBack = true
        log.notice("route \(route.rawValue, privacy: .public) failed (\(failure, privacy: .public)); falling back to plain")
        load(startURL, route: .plain)
        return true
    }

    /// The player item for the URL the core answered, on the chosen route.
    private func makeItem(_ url: String, route: PlayRoute) -> AVPlayerItem? {
        switch route {
        case .direct:
            return URL(string: url).map(AVPlayerItem.init(url:))
        case .plain, .promote:
            let plain = url.replacingOccurrences(of: "/stream.m3u8", with: "/plain.m3u8")
            return URL(string: route == .promote ? plain + "&promote=1" : plain).map(AVPlayerItem.init(url:))
        case .llhttp:
            return URL(string: url + appleQuery).map(AVPlayerItem.init(url:))
        case .ll, .auto:
            guard var parts = URLComponents(string: url + appleQuery) else { return nil }
            parts.scheme = LLPlaylistLoader.scheme
            guard let custom = parts.url else { return nil }
            let loader = LLPlaylistLoader()
            let asset = AVURLAsset(url: custom)
            asset.resourceLoader.setDelegate(loader, queue: loader.queue)
            self.loader = loader
            return AVPlayerItem(asset: asset)
        }
    }

    /// What marks a playlist request as AVPlayer's low-latency one for the relay.
    private var appleQuery: String {
        "&apple=1" + (holdParts.map { "&apple_hold=\($0)" } ?? "")
    }

    private func observe(_ item: AVPlayerItem) {
        let center = NotificationCenter.default
        observers.forEach(center.removeObserver)
        observers = [
            center.addObserver(forName: AVPlayerItem.newErrorLogEntryNotification, object: item, queue: .main) {
                note in
                guard let event = (note.object as? AVPlayerItem)?.errorLog()?.events.last else { return }
                log.notice("player error \(event.errorDomain, privacy: .public) \(event.errorStatusCode) \(event.errorComment ?? "", privacy: .public) uri \(event.uri ?? "", privacy: .public)")
            },
            center.addObserver(forName: AVPlayerItem.playbackStalledNotification, object: item, queue: .main) {
                [weak self] _ in
                MainActor.assumeIsolated {
                    guard let self else { return }
                    self.stalls += 1
                    log.notice("player stalled (\(self.stalls))")
                }
            },
        ]
    }

    /// Checks four times a second for the first play, and once a second asks the core how far
    /// behind its edge the player sits, logs it, and applies the catch-up rate.
    private func measure(core: CoreBridge) async {
        var tick = 0
        var waiting = false
        while !Task.isCancelled {
            if firstPlayMS == nil, player.timeControlStatus == .playing, let opened {
                firstPlayMS = Int((ContinuousClock.now - opened) / .milliseconds(1))
                status = "Playing"
                restarting = false
                recoveries = 0
                log.notice("first play \(self.firstPlayMS ?? 0) ms")
            }
            if firstPlayMS != nil {
                let nowWaiting = player.timeControlStatus == .waitingToPlayAtSpecifiedRate
                if nowWaiting != waiting {
                    waiting = nowWaiting
                    log.notice("player \(nowWaiting ? "waiting" : "resumed", privacy: .public) \(self.player.reasonForWaitingToPlay?.rawValue ?? "", privacy: .public)")
                }
                let now = Double((ContinuousClock.now - linkEpoch) / .milliseconds(1)) / 1000
                if let stall = link.waiting(nowWaiting, at: now) {
                    report(stall, core: core)
                }
            }
            if let failure = player.currentItem?.error, error == nil {
                let ns = failure as NSError
                let reason = player.currentItem?.errorLog()?.events.last?.errorComment ?? "\(ns.domain) \(ns.code)"
                for event in player.currentItem?.errorLog()?.events ?? [] {
                    log.error("error log: \(event.errorDomain, privacy: .public) \(event.errorStatusCode) \(event.errorComment ?? "", privacy: .public) uri \(event.uri ?? "", privacy: .public)")
                }
                if fallBack(from: reason) {
                    try? await Task.sleep(for: .milliseconds(250))
                    continue
                }
                // A failed item is reloaded from the same relay, twice at most, before the error
                // shows. Never in Picture in Picture: replacing the item closes the window.
                if recoveries < Self.maxRecoveries, !pip.active, let startURL, let route = playing {
                    recoveries += 1
                    log.notice("item failed (\(reason, privacy: .public)); reload \(self.recoveries) of \(Self.maxRecoveries)")
                    try? await Task.sleep(for: .seconds(Double(recoveries)))
                    guard !Task.isCancelled, self.stream != nil else { return }
                    reloading()
                    load(startURL, route: route)
                    continue
                }
                error = "Playback failed. Try again in a moment."
                status = "Failed"
                let underlying = (ns.userInfo[NSUnderlyingErrorKey] as? NSError).map { "\($0.domain) \($0.code)" } ?? ""
                log.error("player item failed: \(ns.domain, privacy: .public) \(ns.code) \(ns.localizedDescription, privacy: .public) \(ns.localizedFailureReason ?? "", privacy: .public) underlying \(underlying, privacy: .public) info \(String(describing: ns.userInfo), privacy: .public)")
            }
            if tick % 4 == 0, let item = player.currentItem {
                await sample(item, core: core)
            }
            tick += 1
            try? await Task.sleep(for: .milliseconds(250))
        }
    }

    private func sample(_ item: AVPlayerItem, core: CoreBridge) async {
        // On the whole-segment route AVPlayer downloads Twitch's segments itself (the relay
        // redirects it to the CDN), so its access log measures the link. On the low-latency route
        // it reads parts over loopback and the core measures the link instead.
        if playing == .plain || playing == .promote, let events = item.accessLog()?.events, let last = events.last,
           let transfer = link.accessLog(event: events.count - 1, bytes: last.numberOfBytesTransferred,
                                         transferDuration: last.transferDuration) {
            report(transfer, core: core)
        }
        let pdt = item.currentDate().map { $0.timeIntervalSince1970 * 1000 }
        guard let distance = try? await core.invoke(
            "get_relay_live_distance", args: ["playerPdtMs": pdt, "targetMs": catchUpTargetMS],
            as: LiveDistance.self
        ) else { return }
        behindMS = distance.behindMs
        if catchUpTargetMS != nil, player.timeControlStatus == .playing, player.rate != Float(distance.rate) {
            player.rate = Float(distance.rate)
        }
        let ahead = Self.bufferedAhead(item)
        let behind = distance.behindMs ?? -1
        let newest = distance.behindNewestMs ?? -1
        let edge = "\(distance.edgeSn ?? -1)/\(distance.edgePart ?? -1)"
        let age = distance.edgeAgeMs ?? -1
        let rate = player.rate
        let recommended = Self.ms(item.recommendedTimeOffsetFromLive)
        let configured = Self.ms(item.configuredTimeOffsetFromLive)
        let dated = pdt == nil ? "none" : "ok"
        let state = player.timeControlStatus.rawValue
        let fast = item.canPlayFastForward
        log.notice("behind \(behind) newest \(newest) edge \(edge, privacy: .public) age \(age) buf \(ahead) rate \(rate) rec \(recommended) cfg \(configured) pdt \(dated, privacy: .public) state \(state) ff \(fast)")
        for chunk in stride(from: 0, to: distance.fetches.count, by: 14) {
            let slice = distance.fetches[chunk..<min(chunk + 14, distance.fetches.count)]
            log.notice("fetch \(slice.joined(separator: " | "), privacy: .public)")
        }
    }

    private func logSummary() {
        guard let item = player.currentItem else { return }
        let errors = item.errorLog()?.events.count ?? 0
        for event in item.errorLog()?.events ?? [] {
            log.notice("error log: \(event.errorDomain, privacy: .public) \(event.errorStatusCode) \(event.errorComment ?? "", privacy: .public)")
        }
        if let access = item.accessLog()?.events.last {
            log.notice("summary route \(self.route.rawValue, privacy: .public) first \(self.firstPlayMS ?? -1) stalls \(self.stalls) access-stalls \(access.numberOfStalls) errors \(errors) startup \(access.startupTime) indicated \(Int(access.indicatedBitrate)) observed \(Int(access.observedBitrate)) dropped \(access.numberOfDroppedVideoFrames)")
        } else {
            log.notice("summary route \(self.route.rawValue, privacy: .public) first \(self.firstPlayMS ?? -1) stalls \(self.stalls) errors \(errors)")
        }
    }

    /// Milliseconds buffered past the playhead.
    private static func bufferedAhead(_ item: AVPlayerItem) -> Int {
        let now = item.currentTime().seconds
        for value in item.loadedTimeRanges {
            let range = value.timeRangeValue
            let start = range.start.seconds
            let end = range.end.seconds
            if start <= now + 0.1, end >= now {
                return Int(((end - now) * 1000).rounded())
            }
        }
        return 0
    }

    private static func ms(_ time: CMTime) -> Int {
        time.isNumeric ? Int((time.seconds * 1000).rounded()) : -1
    }
}
