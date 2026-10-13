import AVFoundation
import Foundation
import Observation
import SNKit

/// One stream being watched: the player on the URL the core serves, how far behind the live
/// edge it sits, and the chat socket's last frames.
@Observable @MainActor
final class Watch {
    let player = AVPlayer()

    private(set) var stream: TwitchStream?
    private(set) var status = "Idle"
    private(set) var quality: String?
    private(set) var error: String?
    /// Seekable end minus the playhead, in milliseconds.
    private(set) var edgeMS: Int?
    /// From the tap to the player first playing, in milliseconds.
    private(set) var firstPlayMS: Int?
    private(set) var frames = FrameLog(limit: 30)
    private(set) var chatState = "Not connected"
    private var chatLines = 0

    private var tasks: [Task<Void, Never>] = []
    private var opened: ContinuousClock.Instant?

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
        player.pause()
        status = "Paused"
        log.notice("drive pause")
    }

    func close(core: CoreBridge) {
        guard stream != nil else { return }
        tasks.forEach { $0.cancel() }
        tasks = []
        player.pause()
        player.replaceCurrentItem(with: nil)
        stream = nil
        edgeMS = nil
        firstPlayMS = nil
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
            let start = try await core.invoke(
                "start_stream", args: ["url": "https://twitch.tv/\(stream.userLogin)", "quality": "best"],
                as: StreamStart.self
            )
            guard !Task.isCancelled, let url = URL(string: start.url) else { return }
            quality = start.quality
            log.notice("start_stream \(stream.userLogin, privacy: .public) -> \(start.url, privacy: .public) (\(start.quality, privacy: .public))")
            let session = AVAudioSession.sharedInstance()
            try session.setCategory(.playback, mode: .moviePlayback)
            try session.setActive(true)
            player.replaceCurrentItem(with: AVPlayerItem(url: url))
            player.play()
            status = "Buffering"
            await watchEdge()
        } catch {
            guard !Task.isCancelled else { return }
            self.error = "\(error)"
            status = "Failed"
            log.error("start_stream failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// Checks four times a second for the first play, and once a second logs the distance to
    /// the live edge.
    private func watchEdge() async {
        var tick = 0
        while !Task.isCancelled {
            if firstPlayMS == nil, player.timeControlStatus == .playing, let opened {
                firstPlayMS = Int((ContinuousClock.now - opened) / .milliseconds(1))
                status = "Playing"
                log.notice("first play \(self.firstPlayMS ?? 0) ms")
            }
            if let failure = player.currentItem?.error {
                error = failure.localizedDescription
                status = "Failed"
            }
            if tick % 4 == 0, let item = player.currentItem,
               let range = item.seekableTimeRanges.last?.timeRangeValue, range.end.isNumeric {
                let edge = (range.end.seconds - item.currentTime().seconds) * 1000
                if edge.isFinite {
                    edgeMS = Int(edge.rounded())
                    log.notice("edge \(self.edgeMS ?? 0)")
                }
            }
            tick += 1
            try? await Task.sleep(for: .milliseconds(250))
        }
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
