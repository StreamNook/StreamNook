import ImageIO
import SNKit
import UIKit
import UniformTypeIdentifiers

/// A decoded chat image: one still frame, or every frame of an animation decoded once at the
/// size it is drawn, with its timeline. Shared by every layer that shows it.
final class DecodedImage: @unchecked Sendable {
    let frames: [CGImage]
    let timeline: FrameTimeline
    /// Width over height of the drawn image.
    let aspect: CGFloat
    /// Decoded bytes, what the memory cache counts.
    let cost: Int

    var isAnimated: Bool { frames.count > 1 }
    var first: CGImage? { frames.first }

    init(frames: [CGImage], durations: [Double]) {
        self.frames = frames
        timeline = FrameTimeline(durations: durations.isEmpty ? [0.1] : durations)
        let first = frames.first
        aspect = first.map { CGFloat($0.width) / CGFloat(max($0.height, 1)) } ?? 1
        cost = frames.reduce(0) { $0 + $1.bytesPerRow * $1.height }
    }
}

/// Loads chat images (emotes, badges, paints, GIFs): a memory cache counted in decoded bytes, a
/// disk cache of the original files in Library/Caches, and decoding off the main thread at the
/// drawn pixel size. One request per image at a time, however many rows ask.
final class ImagePipeline: @unchecked Sendable {
    static let shared = ImagePipeline()

    /// What one image takes at most in memory is bounded by its frames; the whole cache by this.
    private let memory = CostCache<String, DecodedImage>(limit: 96 * 1024 * 1024)
    private let disk: URL
    private let session: URLSession
    private let decodeQueue = OperationQueue()
    private let lock = NSLock()
    private var inFlight: [String: [@Sendable (DecodedImage?) -> Void]] = [:]
    /// Images that failed, with the time before which they are not fetched again: a missing
    /// file (HTTP 404 or 410) never, anything else (offline, a server error, a bad decode)
    /// after `retryAfter`. An entry stays until the image loads, so whatever draws a stand-in
    /// keeps it rather than an empty box while a retry runs.
    private var failed: [String: Failure] = [:]
    /// Failed images something drew a stand-in for since their last try, so worth trying again.
    private var asked: Set<String> = []
    /// Failed images with a retry waiting or running.
    private var retrying: Set<String> = []
    private let retryAfter: TimeInterval = 30
    /// The newest decode of each URL, whatever its size, for surfaces that draw one where
    /// another surface already decoded it.
    private var newest: [String: String] = [:]
    /// Most frames decoded per animation; longer ones are sampled evenly.
    private let frameCap = 240

    private init() {
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask)[0]
        disk = caches.appending(path: "app.streamnook/images", directoryHint: .isDirectory)
        try? FileManager.default.createDirectory(at: disk, withIntermediateDirectories: true)
        let config = URLSessionConfiguration.default
        config.urlCache = nil
        config.requestCachePolicy = .reloadIgnoringLocalCacheData
        config.httpMaximumConnectionsPerHost = 6
        session = URLSession(configuration: config)
        decodeQueue.maxConcurrentOperationCount = 3
        decodeQueue.qualityOfService = .userInitiated
        NotificationCenter.default.addObserver(forName: UIApplication.didReceiveMemoryWarningNotification,
                                               object: nil, queue: nil) { [weak self] _ in
            self?.memory.trim(toFraction: 0.25)
        }
    }

    /// Posted (from any thread) when an image starts failing, with its URL as `userInfo["url"]`,
    /// so surfaces that kept room for it can give that room back or draw a stand-in.
    static let failedNotification = Notification.Name("app.streamnook.image.failed")
    /// Posted (from any thread) when an image that had failed loads after all, with its URL as
    /// `userInfo["url"]`, so surfaces drawing a stand-in can draw the image again.
    static let recoveredNotification = Notification.Name("app.streamnook.image.recovered")

    private struct Failure {
        var until: Date
        /// The load that failed, for the retry.
        let pixelHeight: Int
        let animated: Bool
    }

    /// What fetching an image's file came to.
    private enum Fetched {
        case data(Data)
        /// The server says the file does not exist (HTTP 404 or 410), or the URL cannot work.
        case gone
        /// Anything else: offline, a timeout, a server error. Worth trying again later.
        case failed
    }

    var memoryCost: Int { memory.totalCost }
    var memoryCount: Int { memory.count }

    /// True while `url` is failing: from its failure until it loads again. Draw a stand-in (an
    /// emote's name, no badge) meanwhile. A missing file is never tried again this run; any
    /// other failure is retried 30 s on, or as soon as it is asked about after that, while
    /// something still draws a stand-in for it, and `recoveredNotification` says when it loads.
    func hasFailed(_ url: String) -> Bool {
        let (failing, retry) = lock.withLock { () -> (Bool, Failure?) in
            guard let entry = failed[url] else { return (false, nil) }
            asked.insert(url)
            // A window that ran out with no retry waiting: one starts now.
            if entry.until <= Date(), !retrying.contains(url) {
                retrying.insert(url)
                return (true, entry)
            }
            return (true, nil)
        }
        if let retry { load(url, pixelHeight: retry.pixelHeight, animated: retry.animated) { _ in } }
        return failing
    }

    /// The end of a failed image's window: tried again when something drew a stand-in for it
    /// meanwhile, else left until something asks (`hasFailed`).
    private func windowEnded(_ url: String) {
        let retry: Failure? = lock.withLock {
            guard asked.remove(url) != nil, let entry = failed[url], entry.until <= Date() else {
                retrying.remove(url)
                return nil
            }
            return entry
        }
        if let retry { load(url, pixelHeight: retry.pixelHeight, animated: retry.animated) { _ in } }
    }

    /// Whether `url` is inside a failure window now (the lock held).
    private func skipping(_ url: String) -> Bool {
        guard let entry = failed[url] else { return false }
        return entry.until > Date()
    }

    private static func key(_ url: String, _ pixelHeight: Int, _ animated: Bool) -> String {
        "\(pixelHeight)|\(animated ? "a" : "s")|\(url)"
    }

    /// The image when it is already decoded at this size.
    func cached(_ url: String, pixelHeight: Int, animated: Bool) -> DecodedImage? {
        memory.value(Self.key(url, pixelHeight, animated))
    }

    /// The image at whatever size it was last decoded, if it still is.
    func cachedAnySize(_ url: String) -> DecodedImage? {
        guard let key = lock.withLock({ newest[url] }) else { return nil }
        return memory.value(key)
    }

    /// Loads `url` decoded to `pixelHeight` pixels tall (animations keep every frame when
    /// `animated`, else only the first). `done` runs on the main thread, with nil on failure.
    func load(_ url: String, pixelHeight: Int, animated: Bool, done: @escaping @MainActor @Sendable (DecodedImage?) -> Void) {
        let key = Self.key(url, pixelHeight, animated)
        if let hit = memory.value(key) {
            MainActor.assumeIsolatedIfMain { done(hit) }
            return
        }
        let start: Bool = lock.withLock {
            if skipping(url) { return false }
            if inFlight[key] != nil {
                inFlight[key]?.append { image in Task { @MainActor in done(image) } }
                return false
            }
            inFlight[key] = [{ image in Task { @MainActor in done(image) } }]
            return true
        }
        guard start else {
            if lock.withLock({ skipping(url) }) { Task { @MainActor in done(nil) } }
            return
        }
        Task.detached(priority: .userInitiated) { [self] in
            let fetched = await self.bytes(url)
            let data: Data?
            if case .data(let bytes) = fetched { data = bytes } else { data = nil }
            let gone: Bool
            if case .gone = fetched { gone = true } else { gone = false }
            let image: DecodedImage? = await withCheckedContinuation { continuation in
                self.decodeQueue.addOperation {
                    continuation.resume(returning: data.flatMap { self.decode($0, pixelHeight: pixelHeight, animated: animated) })
                }
            }
            if let image { self.memory.insert(image, for: key, cost: image.cost) }
            // A file that will not decode is fetched afresh next time rather than read again.
            if data != nil, image == nil { self.forget(url) }
            let (waiters, didFail, recovered) = self.lock.withLock {
                () -> ([@Sendable (DecodedImage?) -> Void], Bool, Bool) in
                self.retrying.remove(url)
                var didFail = false
                var recovered = false
                if image == nil {
                    // A failure of a load that started before another one's window: that window stands.
                    if !self.skipping(url) {
                        let until = gone ? Date.distantFuture : Date().addingTimeInterval(self.retryAfter)
                        self.failed[url] = Failure(until: until, pixelHeight: pixelHeight, animated: animated)
                        if !gone { self.retrying.insert(url) }
                        didFail = true
                    }
                } else {
                    self.newest[url] = key
                    recovered = self.failed.removeValue(forKey: url) != nil
                    self.asked.remove(url)
                }
                return (self.inFlight.removeValue(forKey: key) ?? [], didFail, recovered)
            }
            waiters.forEach { $0(image) }
            if didFail {
                // Every failure, a retry's too, so the surfaces drawing a stand-in say they still
                // want the image (they ask `hasFailed` as they lay out again).
                NotificationCenter.default.post(name: Self.failedNotification, object: nil, userInfo: ["url": url])
                if !gone {
                    Task.detached(priority: .utility) { [self] in
                        try? await Task.sleep(for: .seconds(self.retryAfter))
                        self.windowEnded(url)
                    }
                }
            }
            if recovered {
                NotificationCenter.default.post(name: Self.recoveredNotification, object: nil, userInfo: ["url": url])
            }
        }
    }

    /// Drops `url`'s file from the disk cache.
    private func forget(_ url: String) {
        try? FileManager.default.removeItem(at: disk.appending(path: ImageCacheKey.fileName(for: url)))
    }

    /// The original file: from disk when cached there, else fetched once and kept. A FrankerFaceZ
    /// size the CDN does not have falls back to the next smaller one, kept under the URL asked for.
    private func bytes(_ url: String) async -> Fetched {
        if let bundled = HouseArt.data(url) { return .data(bundled) }
        if url.hasPrefix(HouseArt.scheme) { return .gone }
        // A picture the core finished itself (a badge painted onto its colour) carries its bytes.
        if url.hasPrefix("data:") {
            guard let comma = url.firstIndex(of: ","), url[..<comma].hasSuffix(";base64"),
                  let data = Data(base64Encoded: String(url[url.index(after: comma)...])) else { return .gone }
            return .data(data)
        }
        let file = disk.appending(path: ImageCacheKey.fileName(for: url))
        if let data = try? Data(contentsOf: file), !data.isEmpty { return .data(data) }
        guard let remote = URL(string: url) else { return .gone }
        var outcome = await fetch(remote)
        if case .httpError(let status) = outcome {
            // FFZ serves only the sizes an emote was uploaded with: try the smaller ones.
            var smallerFailed = false
            for smaller in ImageTier.ffzFallbacks(url) {
                guard let next = URL(string: smaller) else { continue }
                let tried = await fetch(next)
                if case .data = tried {
                    outcome = tried
                    break
                }
                if case .httpError(let code) = tried, Self.missing(code) { continue }
                smallerFailed = true
            }
            if case .httpError = outcome {
                return Self.missing(status) && !smallerFailed ? .gone : .failed
            }
        }
        switch outcome {
        case .data(let data):
            try? data.write(to: file, options: .atomic)
            return .data(data)
        case .httpError(let status):
            return Self.missing(status) ? .gone : .failed
        case .unreachable:
            return .failed
        }
    }

    private enum Download {
        case data(Data)
        /// The server answered without the file (any status outside 2xx, or an empty body as 0).
        case httpError(Int)
        /// No answer: offline, a timeout, a dropped connection.
        case unreachable
    }

    /// One file over the network, a dropped connection tried once more.
    private func fetch(_ remote: URL) async -> Download {
        for attempt in 0..<2 {
            do {
                let (data, response) = try await session.data(from: remote)
                guard let http = response as? HTTPURLResponse else { return .httpError(0) }
                guard (200..<300).contains(http.statusCode), !data.isEmpty else { return .httpError(http.statusCode) }
                return .data(data)
            } catch {
                if attempt == 0 { try? await Task.sleep(for: .milliseconds(400)) }
            }
        }
        return .unreachable
    }

    /// The statuses that say a file does not exist, rather than that it could not be had now.
    private static func missing(_ status: Int) -> Bool {
        status == 404 || status == 410
    }

    /// Decodes every frame (or the first) downsampled to `pixelHeight`, forcing the decode here so
    /// the main thread only hands finished bitmaps to layers.
    private func decode(_ data: Data, pixelHeight: Int, animated: Bool) -> DecodedImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary)
        else { return nil }
        let count = CGImageSourceGetCount(source)
        guard count > 0 else { return nil }
        // Downsample by the longer side so a wide emote keeps its height.
        var maxSide = pixelHeight
        if let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
           let w = props[kCGImagePropertyPixelWidth] as? Int, let h = props[kCGImagePropertyPixelHeight] as? Int, h > 0 {
            maxSide = max(pixelHeight, Int((Double(pixelHeight) * Double(w) / Double(h)).rounded(.up)))
            // Never upscale past the file.
            if h <= pixelHeight { maxSide = max(w, h) }
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceThumbnailMaxPixelSize: maxSide,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
        ]
        let indices: [Int]
        if animated, count > 1 {
            indices = count <= frameCap ? Array(0..<count) : (0..<frameCap).map { $0 * count / frameCap }
        } else {
            indices = [0]
        }
        var frames: [CGImage] = []
        var durations: [Double] = []
        frames.reserveCapacity(indices.count)
        for (n, i) in indices.enumerated() {
            guard let image = CGImageSourceCreateThumbnailAtIndex(source, i, options as CFDictionary) else { continue }
            frames.append(Self.native(image) ?? image)
            if indices.count > 1 {
                let next = n + 1 < indices.count ? indices[n + 1] : count
                var span = 0.0
                for j in i..<next { span += Self.delay(source, j) }
                durations.append(span)
            }
        }
        guard !frames.isEmpty else { return nil }
        // An animation whose thumbnails all came back as one picture (a lossless WebP's frames
        // can) is decoded again frame by frame at full size and scaled here, as WebKit decodes
        // animations; the timing stays the same.
        if indices.count > 1, frames.count == indices.count, Self.looksStill(frames) {
            let redone = indices.compactMap { i in
                CGImageSourceCreateImageAtIndex(source, i, nil).flatMap { Self.scaled($0, maxSide: maxSide) }
            }
            if redone.count == frames.count, !Self.looksStill(redone) { frames = redone }
        }
        return DecodedImage(frames: frames, durations: durations)
    }

    /// True when the first, quarter and middle frames hold the same pixels.
    private static func looksStill(_ frames: [CGImage]) -> Bool {
        let picks = [0, frames.count / 4, frames.count / 2].map { frames[$0].dataProvider?.data as Data? }
        guard let first = picks[0] else { return false }
        return picks.allSatisfy { $0 == first }
    }

    /// The frame drawn at most `maxSide` pixels on its longer side, as premultiplied BGRA.
    private static func scaled(_ image: CGImage, maxSide: Int) -> CGImage? {
        let longer = max(image.width, image.height)
        let factor = longer > maxSide ? Double(maxSide) / Double(longer) : 1
        let width = max(1, Int((Double(image.width) * factor).rounded()))
        let height = max(1, Int((Double(image.height) * factor).rounded()))
        let info = CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: colorSpace, bitmapInfo: info) else { return nil }
        context.interpolationQuality = .high
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        return context.makeImage()
    }

    private static let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()

    /// The frame redrawn as premultiplied BGRA, the layout Core Animation shows without converting
    /// it on the main thread every time a layer's frame changes.
    private static func native(_ image: CGImage) -> CGImage? {
        let info = CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        if image.bitmapInfo.rawValue & ~CGBitmapInfo.floatComponents.rawValue == info, image.bitsPerPixel == 32 { return image }
        guard let context = CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: colorSpace, bitmapInfo: info) else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
        return context.makeImage()
    }

    private static func delay(_ source: CGImageSource, _ index: Int) -> Double {
        guard let props = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any] else { return 0.1 }
        let dictionaries: [(CFString, CFString, CFString)] = [
            (kCGImagePropertyWebPDictionary, kCGImagePropertyWebPUnclampedDelayTime, kCGImagePropertyWebPDelayTime),
            (kCGImagePropertyGIFDictionary, kCGImagePropertyGIFUnclampedDelayTime, kCGImagePropertyGIFDelayTime),
            (kCGImagePropertyPNGDictionary, kCGImagePropertyAPNGUnclampedDelayTime, kCGImagePropertyAPNGDelayTime),
        ]
        for (dict, unclamped, clamped) in dictionaries {
            if let d = props[dict] as? [CFString: Any] {
                if let v = d[unclamped] as? Double, v > 0 { return v }
                if let v = d[clamped] as? Double, v > 0 { return v }
            }
        }
        return 0.1
    }
}

extension MainActor {
    /// Runs `body` now when already on the main thread, else on it soon.
    static func assumeIsolatedIfMain(_ body: @escaping @MainActor () -> Void) {
        if Thread.isMainThread {
            MainActor.assumeIsolated { body() }
        } else {
            Task { @MainActor in body() }
        }
    }
}

/// StreamNook's house badges ship in the app rather than on the CDN; the core names them
/// `streamnook-house:<slug>` (with `@chat` for the chat-size cut).
enum HouseArt {
    static let scheme = "streamnook-house:"

    static func data(_ url: String) -> Data? {
        guard url.hasPrefix(scheme) else { return nil }
        let name = String(url.dropFirst(scheme.count))
        guard let file = Bundle.module.url(forResource: name, withExtension: "webp", subdirectory: "HouseBadges")
            ?? Bundle.module.url(forResource: "streamnook-default", withExtension: "webp", subdirectory: "HouseBadges")
        else { return nil }
        return try? Data(contentsOf: file)
    }
}
