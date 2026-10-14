import AVFoundation
import Foundation
import Observation
import SNKit

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

/// One stream being watched: the player on the URL the core serves, how far behind the live
/// edge it sits, and the room's chat.
@Observable @MainActor
final class Watch {
    let player = AVPlayer()

    private(set) var stream: TwitchStream?
    private(set) var status = "Idle"
    private(set) var quality: String?
    private(set) var error: String?
    /// How far the player sits behind the relay's live edge, in milliseconds.
    private(set) var behindMS: Int?
    /// From the tap to the player first playing, in milliseconds.
    private(set) var firstPlayMS: Int?
    /// The open stream's chat room; the chat panel starts it when it shows.
    private(set) var chat: ChatSession?
    /// The route the player is on now (after any fallback).
    private(set) var playing: PlayRoute?

    /// Set by drive steps before a stream opens.
    var route = PlayRoute.auto
    /// The quality asked of `start_stream`: `best`, a variant name, or `audio_only`.
    var requestedQuality = "best"
    var offsetFromLive: Double?
    var preservesOffset = false
    /// The distance behind the edge a gentle catch-up rate holds, in milliseconds.
    var catchUpTargetMS: Int?
    /// Part targets of hold-back the low-latency playlist asks AVPlayer for (the relay takes 2 or 3).
    /// Nil keeps the relay's default of three, Apple's recommendation (about 1.5 s behind live);
    /// the `hold` drive step can still ask for two.
    var holdParts: Int?

    private var tasks: [Task<Void, Never>] = []
    private var opened: ContinuousClock.Instant?
    private var observers: [NSObjectProtocol] = []
    private var stalls = 0
    private var loader: LLPlaylistLoader?
    /// The relay URL `start_stream` answered, kept for a fallback to the whole-segment route.
    private var startURL: String?
    private var fellBack = false

    func open(_ stream: TwitchStream, core: CoreBridge) {
        guard self.stream?.id != stream.id else { return }
        close(core: core)
        self.stream = stream
        opened = .now
        status = "Resolving"
        chat = ChatSession(channel: stream.userLogin, core: core, source: .core)
        tasks.append(Task { await self.play(stream, core: core) })
    }

    /// Back from the background: the core's chat bridge may listen on a new port.
    func foregrounded(chatPort: Int) {
        chat?.reconnect(port: chatPort)
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
        player.pause()
        status = "Paused"
        log.notice("drive pause")
    }

    func close(core: CoreBridge) {
        guard stream != nil else { return }
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
        chat?.stop()
        chat = nil
        Task.detached {
            _ = try? await core.invoke("stop_stream", "{}")
        }
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    // MARK: Playback

    private func play(_ stream: TwitchStream, core: CoreBridge) async {
        do {
            _ = try? await core.invoke("set_experimental_low_latency", args: ["enabled": true])
            let start = try await core.invoke(
                "start_stream", args: ["url": "https://twitch.tv/\(stream.userLogin)", "quality": requestedQuality],
                as: StreamStart.self
            )
            guard !Task.isCancelled else { return }
            quality = start.quality
            startURL = start.url
            let chosen = await Self.route(for: start, asked: route, core: core)
            log.notice("start_stream \(stream.userLogin, privacy: .public) -> \(start.url, privacy: .public) (\(start.quality, privacy: .public)) route \(chosen.rawValue, privacy: .public)")
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.playback, mode: .moviePlayback)
            try session.setActive(true)
            load(start.url, route: chosen)
            await measure(core: core)
        } catch {
            guard !Task.isCancelled else { return }
            self.error = "\(error)"
            status = "Failed"
            log.error("start_stream failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// Puts a new item for `url` on `route` in the player and plays it.
    private func load(_ url: String, route: PlayRoute) {
        guard let item = makeItem(url, route: route) else { return }
        if let offsetFromLive {
            item.configuredTimeOffsetFromLive = CMTime(seconds: offsetFromLive, preferredTimescale: 1000)
        }
        item.automaticallyPreservesTimeOffsetFromLive = preservesOffset
        observe(item)
        playing = route
        error = nil
        player.replaceCurrentItem(with: item)
        player.play()
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
                log.notice("first play \(self.firstPlayMS ?? 0) ms")
            }
            if firstPlayMS != nil {
                let nowWaiting = player.timeControlStatus == .waitingToPlayAtSpecifiedRate
                if nowWaiting != waiting {
                    waiting = nowWaiting
                    log.notice("player \(nowWaiting ? "waiting" : "resumed", privacy: .public) \(self.player.reasonForWaitingToPlay?.rawValue ?? "", privacy: .public)")
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
