import Foundation

/// Chat frames the core made from real emote, badge and paint data and raw IRC lines
/// (`chat_fixture_tests` in `irc_service.rs` writes them), for replaying chat without a Twitch
/// sign-in.
public struct ChatFixture: Sendable {
    public var channel: String
    public var roomID: String
    public var ownLogin: String
    /// The phone's first-run `chat_design` block, as JSON.
    public var chatDesignJSON: Data
    /// The room's emote set (`twitch`, `bttv`, `7tv`, `ffz`), as JSON.
    public var emotesJSON: Data
    /// Socket frames in the order the core sent them.
    public var frames: [String]

    public init(data: Data) throws {
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw CocoaError(.coderReadCorrupt)
        }
        channel = root["channel"] as? String ?? "nookroom"
        roomID = root["room_id"] as? String ?? ""
        ownLogin = (root["own"] as? [String: Any])?["login"] as? String ?? ""
        chatDesignJSON = try JSONSerialization.data(withJSONObject: root["chat_design"] ?? [String: Any]())
        emotesJSON = try JSONSerialization.data(withJSONObject: root["emotes"] ?? [String: Any]())
        frames = root["frames"] as? [String] ?? []
    }

    /// The fixture bundled with SNKit.
    public static func bundled() -> ChatFixture? {
        guard let url = Bundle.module.url(forResource: "chat-frames", withExtension: "json"),
              let data = try? Data(contentsOf: url) else { return nil }
        return try? ChatFixture(data: data)
    }

    /// The chat rows among the frames.
    public var rowFrames: [String] { frames.filter { $0.hasPrefix("{") && !$0.contains("\"type\":") } }

    /// A frame of loop `n` with every fixture id made unique for that loop, so replays keep adding
    /// rows instead of repeating ids (row ids, deletion targets and reply parents alike).
    public static func frame(_ frame: String, loop: Int) -> String {
        loop == 0 ? frame : frame.replacingOccurrences(of: "\"fx-", with: "\"fx\(loop)-")
    }

    /// Replays the frames: the connection frames first, then the rest at `rate` frames a second,
    /// looping with fresh ids while `loop` is set, until the reader stops reading.
    public func replay(rate: Double, loop: Bool = true) -> AsyncStream<String> {
        let frames = frames
        return AsyncStream(bufferingPolicy: .bufferingNewest(4096)) { continuation in
            let task = Task.detached(priority: .userInitiated) {
                let lead = frames.prefix { !$0.hasPrefix("{") || $0.contains("\"type\":\"ROOMSTATE\"") }
                for frame in lead { continuation.yield(frame) }
                let body = Array(frames.dropFirst(lead.count)).filter { $0 != "HEARTBEAT" }
                guard !body.isEmpty else { continuation.finish(); return }
                let clock = ContinuousClock()
                let start = clock.now
                var sent = 0
                var heartbeat = clock.now
                while !Task.isCancelled {
                    let due = Int((clock.now - start) / .seconds(1) * rate)
                    while sent < due, !Task.isCancelled {
                        let n = sent / body.count
                        if n > 0, !loop { continuation.finish(); return }
                        continuation.yield(Self.frame(body[sent % body.count], loop: n))
                        sent += 1
                    }
                    if clock.now - heartbeat > .seconds(30) {
                        continuation.yield("HEARTBEAT")
                        heartbeat = clock.now
                    }
                    try? await Task.sleep(for: .milliseconds(rate > 50 ? 4 : 20))
                }
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }
}
