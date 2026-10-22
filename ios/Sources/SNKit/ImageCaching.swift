import Foundation
import Synchronization

/// The CDN size steps chat images come in, and which one a drawn size needs.
public enum ImageTier {
    /// The URL of the smallest file at least `pixelHeight` tall, for the CDNs whose URLs name
    /// their size. 7TV serves 1x to 4x at 32 px per step, BetterTTV 1x to 3x at 28, FFZ 1, 2 and 4
    /// at 32, Twitch 1.0 to 3.0 at 28. Anything else comes back as it was.
    public static func url(_ url: String, pixelHeight: Double) -> String {
        func step(_ base: Double, _ steps: [Int]) -> Int {
            steps.first(where: { Double($0) * base >= pixelHeight }) ?? steps.last ?? 1
        }
        if url.contains("cdn.7tv.app/emote/") {
            let n = step(32, [1, 2, 3, 4])
            return replacingLast(url, pattern: #"/[1-4]x(\.[a-z]+)?$"#, with: "/\(n)x", keepGroup: true)
        }
        if url.contains("cdn.betterttv.net/emote/") {
            let n = step(28, [1, 2, 3])
            return replacingLast(url, pattern: #"/[1-3]x(\.[a-z]+)?$"#, with: "/\(n)x", keepGroup: true)
        }
        if url.contains("cdn.frankerfacez.com/") {
            let n = step(32, [1, 2, 4])
            return replacingLast(url, pattern: #"/[124]$"#, with: "/\(n)", keepGroup: false)
        }
        if url.contains("static-cdn.jtvnw.net/emoticons/") {
            let n = step(28, [1, 2, 3])
            return replacingLast(url, pattern: #"/[1-3]\.0$"#, with: "/\(n).0", keepGroup: false)
        }
        return url
    }

    /// The smaller FrankerFaceZ sizes to try, largest first, when the size asked for is missing:
    /// not every FFZ emote has a 4 or a 2. Static (`.../emote/<id>/4`) and animated
    /// (`.../emote/<id>/animated/4`) alike; any other URL has none.
    public static func ffzFallbacks(_ url: String) -> [String] {
        guard url.contains("cdn.frankerfacez.com/") else { return [] }
        let sizes: [Int]
        if url.hasSuffix("/4") {
            sizes = [2, 1]
        } else if url.hasSuffix("/2") {
            sizes = [1]
        } else {
            return []
        }
        return sizes.map { replacingLast(url, pattern: #"/[124]$"#, with: "/\($0)", keepGroup: false) }
    }

    private static func replacingLast(_ url: String, pattern: String, with replacement: String, keepGroup: Bool) -> String {
        guard let regex = try? NSRegularExpression(pattern: pattern) else { return url }
        let range = NSRange(url.startIndex..., in: url)
        guard let match = regex.firstMatch(in: url, range: range),
              let whole = Range(match.range, in: url) else { return url }
        var tail = ""
        if keepGroup, match.numberOfRanges > 1, let group = Range(match.range(at: 1), in: url) {
            tail = String(url[group])
        }
        return url.replacingCharacters(in: whole, with: replacement + tail)
    }
}

/// A least recently used store whose limit is a byte cost (what decoded images take in
/// memory), not a count. Safe from any thread.
public final class CostCache<Key: Hashable & Sendable, Value: Sendable>: Sendable {
    private struct Entry {
        var value: Value
        var cost: Int
        var tick: UInt64
    }

    private struct State {
        var entries: [Key: Entry] = [:]
        var total = 0
        var tick: UInt64 = 0
        var limit: Int
    }

    private let state: Mutex<State>

    public init(limit: Int) {
        state = Mutex(State(limit: limit))
    }

    public var totalCost: Int { state.withLock { $0.total } }
    public var count: Int { state.withLock { $0.entries.count } }
    public var limit: Int {
        get { state.withLock { $0.limit } }
        set { state.withLock { $0.limit = newValue; Self.trim(&$0) } }
    }

    public func value(_ key: Key) -> Value? {
        state.withLock { state in
            guard var entry = state.entries[key] else { return nil }
            state.tick += 1
            entry.tick = state.tick
            state.entries[key] = entry
            return entry.value
        }
    }

    public func insert(_ value: Value, for key: Key, cost: Int) {
        state.withLock { state in
            state.tick += 1
            if let old = state.entries[key] { state.total -= old.cost }
            state.entries[key] = Entry(value: value, cost: max(0, cost), tick: state.tick)
            state.total += max(0, cost)
            Self.trim(&state)
        }
    }

    public func removeAll() {
        state.withLock { state in
            state.entries.removeAll()
            state.total = 0
        }
    }

    /// Drops the least recently used entries until the total is at most `fraction` of the limit.
    public func trim(toFraction fraction: Double) {
        state.withLock { state in
            let keep = Int(Double(state.limit) * fraction)
            Self.evict(&state, to: keep)
        }
    }

    private static func trim(_ state: inout State) {
        evict(&state, to: state.limit)
    }

    private static func evict(_ state: inout State, to limit: Int) {
        guard state.total > limit else { return }
        let oldest = state.entries.sorted { $0.value.tick < $1.value.tick }
        for (key, entry) in oldest {
            guard state.total > limit else { break }
            state.entries[key] = nil
            state.total -= entry.cost
        }
    }
}

/// File names for the image disk cache: a stable 64-bit FNV-1a hash of the URL in hex, with
/// the URL's extension kept so a reader can tell formats apart.
public enum ImageCacheKey {
    public static func fileName(for url: String) -> String {
        var hash: UInt64 = 0xcbf2_9ce4_8422_2325
        for byte in url.utf8 {
            hash ^= UInt64(byte)
            hash = hash &* 0x0000_0100_0000_01b3
        }
        let ext = URL(string: url)?.pathExtension.lowercased() ?? ""
        let known = ["webp", "gif", "png", "jpg", "jpeg", "avif"]
        return String(format: "%016llx", hash) + (known.contains(ext) ? ".\(ext)" : "")
    }
}

/// How animated frames are timed: each copy of an animation on screen picks its frame from one
/// shared clock, so two copies of an emote always show the same frame.
public struct FrameTimeline: Sendable, Equatable {
    public let durations: [Double]
    public let total: Double
    /// The shortest frame, for the rate that shows every frame (`ClockRate`).
    public let shortest: Double
    private let ends: [Double]

    /// Browsers draw frames shorter than 20 ms at 100 ms; so does this.
    public init(durations: [Double]) {
        let clean = durations.map { $0 < 0.02 ? 0.1 : $0 }
        self.durations = clean
        shortest = clean.min() ?? 0.1
        var running = 0.0
        ends = clean.map { running += $0; return running }
        total = running
    }

    /// The frame to show at `time` seconds on the shared clock.
    public func frame(at time: Double) -> Int {
        guard durations.count > 1, total > 0 else { return 0 }
        let t = time.truncatingRemainder(dividingBy: total)
        var low = 0
        var high = ends.count - 1
        while low < high {
            let mid = (low + high) / 2
            if ends[mid] > t { high = mid } else { low = mid + 1 }
        }
        return low
    }
}
