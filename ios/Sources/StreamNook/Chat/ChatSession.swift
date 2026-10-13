import Foundation
import Observation
import os
import SNKit
import UIKit

let chatLog = Logger(subsystem: "app.streamnook", category: "chat")

/// One open chat room: the core's socket (or a fixture replay) feeding the pipeline, the room's
/// modes, and the reply draft. The list and the composer both read it.
@Observable @MainActor
final class ChatSession {
    enum Source {
        /// The core's chat socket for a channel.
        case core
        /// Recorded frames replayed at a rate (frames a second), for building without a sign-in.
        case fixture(ChatFixture, rate: Double)
    }

    let channel: String
    let core: CoreBridge
    let source: Source
    @ObservationIgnored let pipeline: ChatPipeline

    private(set) var modes: ChatRoomModes?
    private(set) var style = ChatStyle()
    private(set) var connected = false
    private(set) var error: String?
    private(set) var rowCount = 0
    /// The message a reply is drafted to, by room.
    var replyTo: ChatMessage?
    private(set) var paused = false
    /// The room's numeric Twitch id, once known.
    var roomID: String?

    @ObservationIgnored private var reader: Task<Void, Never>?
    @ObservationIgnored private var settingsTask: Task<Void, Never>?

    init(channel: String, core: CoreBridge, source: Source) {
        self.channel = channel.lowercased()
        self.core = core
        self.source = source
        let box = WeakBox<ChatSession>()
        pipeline = ChatPipeline(style: ChatStyle(), scale: UITraitCollection.current.displayScale, onModes: { modes in
            Task { @MainActor in box.value?.modes = modes }
        }, onRowCount: { count in
            Task { @MainActor in box.value?.rowCount = count }
        })
        box.value = self
    }

    func start() {
        guard reader == nil else { return }
        loadSettings()
        switch source {
        case .fixture(let fixture, let rate):
            roomID = fixture.roomID
            let pipeline = pipeline
            connected = true
            reader = Task.detached(priority: .userInitiated) {
                for await frame in fixture.replay(rate: rate) {
                    pipeline.ingest(frame)
                }
            }
        case .core:
            let channel = channel
            let core = core
            let pipeline = pipeline
            reader = Task { [weak self] in
                do {
                    let port = try await core.invoke("start_chat", args: ["channel": channel, "reattach": false], as: Int.self)
                    guard !Task.isCancelled else { return }
                    self?.connected = true
                    chatLog.notice("chat socket \(channel, privacy: .public) port \(port)")
                    Task { await self?.loadHistory() }
                    try await Self.read(port: port, into: pipeline)
                } catch {
                    guard !Task.isCancelled else { return }
                    self?.error = "\(error)"
                    chatLog.error("chat failed: \(String(describing: error), privacy: .public)")
                }
            }
        }
    }

    func stop() {
        reader?.cancel()
        reader = nil
        settingsTask?.cancel()
        if case .core = source {
            let core = core
            Task.detached { _ = try? await core.invoke("stop_chat", "{}") }
        }
    }

    func setPaused(_ paused: Bool) {
        self.paused = paused
    }

    /// The core's chat settings, read once when the room opens.
    private func loadSettings() {
        let core = core
        settingsTask = Task { [weak self] in
            guard let json = try? await core.invoke("get_settings", "{}") else { return }
            let style = ChatStyle(settingsJSON: Data(json.utf8))
            guard let self else { return }
            self.style = style
            self.pipeline.setStyle(style)
        }
    }

    /// The join backfill: recent history above the live rows.
    private func loadHistory() async {
        guard let json = try? await core.invoke("load_channel_history", args: ["channel": channel]),
              let rows = try? JSONDecoder().decode([ChatMessage].self, from: Data(json.utf8)) else { return }
        pipeline.prepend(rows)
        if roomID == nil { roomID = rows.first?.tags["room-id"] }
    }

    private nonisolated static func read(port: Int, into pipeline: ChatPipeline) async throws {
        guard let url = URL(string: "ws://127.0.0.1:\(port)") else { return }
        let socket = URLSession.shared.webSocketTask(with: url)
        socket.maximumMessageSize = 4 * 1024 * 1024
        socket.resume()
        try await withTaskCancellationHandler {
            while !Task.isCancelled {
                switch try await socket.receive() {
                case .string(let text): pipeline.ingest(text)
                case .data(let data): pipeline.ingest(String(decoding: data, as: UTF8.self))
                @unknown default: break
                }
            }
        } onCancel: {
            socket.cancel(with: .goingAway, reason: nil)
        }
    }
}

/// A weak reference a Sendable closure can hold.
final class WeakBox<T: AnyObject>: @unchecked Sendable {
    weak var value: T?
}
