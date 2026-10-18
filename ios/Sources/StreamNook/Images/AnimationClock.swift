import QuartzCore
import UIKit

/// One clock for every animated chat image on screen. Each layer picks its frame from the same
/// time, so every copy of an emote shows the same frame; only layers that are on screen are
/// registered, and the display link runs only while one is, at up to 120 Hz.
@MainActor
final class AnimationClock {
    static let shared = AnimationClock()

    private var layers = NSHashTable<AnimatedImageLayer>.weakObjects()
    private var link: CADisplayLink?
    private let origin = CACurrentMediaTime()
    /// When on, stills are drawn instead (Reduce Motion, or the setting asks for stills).
    private(set) var reduceMotion = UIAccessibility.isReduceMotionEnabled

    private init() {
        NotificationCenter.default.addObserver(forName: UIAccessibility.reduceMotionStatusDidChangeNotification,
                                               object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.reduceMotionChanged() }
        }
    }

    /// Seconds on the shared clock.
    var now: Double { CACurrentMediaTime() - origin }

    var registeredCount: Int { layers.allObjects.count }

    func register(_ layer: AnimatedImageLayer) {
        layers.add(layer)
        layer.advance(to: now)
        startIfNeeded()
    }

    func unregister(_ layer: AnimatedImageLayer) {
        layers.remove(layer)
        if layers.allObjects.isEmpty { stop() }
    }

    private func startIfNeeded() {
        guard link == nil, !reduceMotion else { return }
        let link = CADisplayLink(target: Ticker(self), selector: #selector(Ticker.tick(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 30, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    private func stop() {
        link?.invalidate()
        link = nil
    }

    fileprivate func tick(_ link: CADisplayLink) {
        let t = link.targetTimestamp - origin
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for layer in layers.allObjects {
            layer.advance(to: t)
        }
        CATransaction.commit()
    }

    private func reduceMotionChanged() {
        reduceMotion = UIAccessibility.isReduceMotionEnabled
        if reduceMotion {
            stop()
            for layer in layers.allObjects { layer.showFirstFrame() }
        } else if !layers.allObjects.isEmpty {
            startIfNeeded()
        }
    }

    /// Holds the clock weakly so the display link never keeps it alive.
    @MainActor
    private final class Ticker: NSObject {
        weak var clock: AnimationClock?
        init(_ clock: AnimationClock) { self.clock = clock }
        @objc func tick(_ link: CADisplayLink) {
            clock?.tick(link)
        }
    }
}

/// A layer showing a decoded chat image; an animated one follows the shared clock while it is
/// registered.
final class AnimatedImageLayer: CALayer {
    private(set) var image: DecodedImage?
    private var shown = -1

    override init() {
        super.init()
        contentsGravity = .resizeAspect
        actions = ["contents": NSNull(), "position": NSNull(), "bounds": NSNull(), "frame": NSNull(), "hidden": NSNull()]
    }

    override init(layer: Any) {
        super.init(layer: layer)
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Shows `image`; `animate` asks for the clock (false draws the first frame).
    @MainActor
    func show(_ image: DecodedImage?, animate: Bool) {
        if self.image !== image {
            self.image = image
            shown = -1
        }
        guard let image else {
            contents = nil
            AnimationClock.shared.unregister(self)
            return
        }
        if animate, image.isAnimated, !AnimationClock.shared.reduceMotion {
            AnimationClock.shared.register(self)
        } else {
            AnimationClock.shared.unregister(self)
            showFirstFrame()
        }
    }

    @MainActor
    func stopAnimating() {
        AnimationClock.shared.unregister(self)
    }

    func advance(to time: Double) {
        guard let image, image.isAnimated else { return }
        let frame = image.timeline.frame(at: time)
        guard frame != shown, frame < image.frames.count else { return }
        shown = frame
        contents = image.frames[frame]
    }

    func showFirstFrame() {
        guard let image, shown != 0 else { return }
        shown = 0
        contents = image.first
    }
}
