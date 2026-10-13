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
}

/// One stream being watched: the player on the URL the core serves, how far behind the live
/// edge it sits, and the chat socket's last frames.
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
    private(set) var frames = FrameLog(limit: 30)
    private(set) var chatState = "Not connected"
    private var chatLines = 0

    /// Set by drive steps before a stream opens.
    var route = PlayRoute.plain
    var offsetFromLive: Double?
    var preservesOffset = false
    /// The distance behind the edge a gentle catch-up rate holds, in milliseconds.
    var catchUpTargetMS: Int?

    private var tasks: [Task<Void, Never>] = []
    private var opened: ContinuousClock.Instant?
    private var observers: [NSObjectProtocol] = []
    private var stalls = 0
    private var loader: LLPlaylistLoader?

    func open(_ stream: TwitchStream, core: CoreBridge) {
        guard self.stream?.id != stream.id else { return }
        close(core: core)
        self.stream = stream
        opened = .now
        status = "Resolving"
        tasks.append(Task { await self.play(stream, core: core) })
        tasks.append(Task { await self.chat(stream, core: core) })
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
        frames = FrameLog(limit: 30)
        chatState = "Not connected"
        chatLines = 0
        Task.detached {
            _ = try? await core.invoke("stop_chat", "{}")
            _ = try? await core.invoke("stop_stream", "{}")
        }
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    // MARK: Playback

    private func play(_ stream: TwitchStream, core: CoreBridge) async {
        do {
            _ = try? await core.invoke("set_experimental_low_latency", args: ["enabled": true])
            let start = try await core.invoke(
                "start_stream", args: ["url": "https://twitch.tv/\(stream.userLogin)", "quality": "best"],
                as: StreamStart.self
            )
            guard !Task.isCancelled else { return }
            let lowLatency = (try? await core.invoke("get_stream_low_latency", args: [:], as: Bool.self)) ?? false
            quality = start.quality
            log.notice("start_stream \(stream.userLogin, privacy: .public) -> \(start.url, privacy: .public) (\(start.quality, privacy: .public)) origin \(lowLatency ? "on" : "off", privacy: .public) route \(self.route.rawValue, privacy: .public)")
            guard let item = makeItem(start.url) else { return }
            if let offsetFromLive {
                item.configuredTimeOffsetFromLive = CMTime(seconds: offsetFromLive, preferredTimescale: 1000)
            }
            item.automaticallyPreservesTimeOffsetFromLive = preservesOffset
            observe(item)
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.playback, mode: .moviePlayback)
            try session.setActive(true)
            player.replaceCurrentItem(with: item)
            player.play()
            status = "Buffering"
            await measure(core: core)
        } catch {
            guard !Task.isCancelled else { return }
            self.error = "\(error)"
            status = "Failed"
            log.error("start_stream failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The player item for the URL the core answered, on the chosen route.
    private func makeItem(_ url: String) -> AVPlayerItem? {
        switch route {
        case .plain, .promote:
            let plain = url.replacingOccurrences(of: "/stream.m3u8", with: "/plain.m3u8")
            return URL(string: route == .promote ? plain + "&promote=1" : plain).map(AVPlayerItem.init(url:))
        case .llhttp:
            return URL(string: url + "&apple=1").map(AVPlayerItem.init(url:))
        case .ll:
            guard var parts = URLComponents(string: url + "&apple=1") else { return nil }
            parts.scheme = LLPlaylistLoader.scheme
            guard let custom = parts.url else { return nil }
            let loader = LLPlaylistLoader()
            let asset = AVURLAsset(url: custom)
            asset.resourceLoader.setDelegate(loader, queue: loader.queue)
            self.loader = loader
            return AVPlayerItem(asset: asset)
        }
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
            if let failure = player.currentItem?.error {
                error = failure.localizedDescription
                status = "Failed"
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
        log.notice("behind \(behind) newest \(newest) edge \(edge, privacy: .public) age \(age) buf \(ahead) rate \(rate) rec \(recommended) cfg \(configured) pdt \(dated, privacy: .public)")
        for chunk in stride(from: 0, to: distance.fetches.count, by: 14) {
            let slice = distance.fetches[chunk..<min(chunk + 14, distance.fetches.count)]
            log.notice("fetch \(slice.joined(separator: " | "), privacy: .public)")
        }
    }

    private func logSummary() {
        guard let item = player.currentItem else { return }
        let errors = item.errorLog()?.events.count ?? 0
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

    // MARK: Chat

    private func chat(_ stream: TwitchStream, core: CoreBridge) async {
        do {
            chatState = "Connecting"
            let port = try await core.invoke(
                "start_chat", args: ["channel": stream.userLogin, "reattach": false], as: Int.self
            )
            guard !Task.isCancelled else { return }
            if port == 0, let fake = core as? FakeCore {
                chatState = "FakeCore ticker"
                for await raw in fake.chatTicker(channel: stream.userLogin) {
                    frames.append(ChatFrame(raw: raw))
                }
            } else {
                chatState = "ws://127.0.0.1:\(port)"
                log.notice("chat socket port \(port)")
                try await readSocket(port: port)
            }
        } catch {
            guard !Task.isCancelled else { return }
            chatState = "Chat failed: \(error)"
            log.error("chat failed: \(String(describing: error), privacy: .public)")
        }
    }

    private func readSocket(port: Int) async throws {
        guard let url = URL(string: "ws://127.0.0.1:\(port)") else { return }
        let socket = URLSession.shared.webSocketTask(with: url)
        socket.resume()
        defer { socket.cancel(with: .goingAway, reason: nil) }
        try await withTaskCancellationHandler {
            while !Task.isCancelled {
                let message = try await socket.receive()
                switch message {
                case .string(let text): received(text)
                case .data(let data): received(String(decoding: data, as: UTF8.self))
                @unknown default: break
                }
            }
        } onCancel: {
            socket.cancel(with: .goingAway, reason: nil)
        }
    }

    /// Keeps the frame and logs the first few chat lines, which is how a drive run sees chat
    /// arrive.
    private func received(_ raw: String) {
        let frame = ChatFrame(raw: raw)
        frames.append(frame)
        guard frame.kind == .message else { return }
        chatLines += 1
        if chatLines <= 5 {
            log.notice("chat line \(self.chatLines): \(String(frame.summary.prefix(80)), privacy: .public)")
        }
    }
}
