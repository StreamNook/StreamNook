import QuartzCore
import SNKit
import UIKit

/// One clock for every animated image on screen (chat emotes, badges, cosmetics). Each layer picks
/// its frame from the same time, so every copy shows the same frame. Only layers on screen are
/// registered; the link runs only while one may move, at the slowest rate that still shows every
/// frame of the fastest animation among them (`ClockRate`: 30 or 60 Hz). Reduce Motion stills all
/// but `always` layers (7TV profile pictures); Low Power Mode stills those too.
@MainActor
final class AnimationClock {
    static let shared = AnimationClock()

    private var layers = NSHashTable<AnimatedImageLayer>.weakObjects()
    /// Layers that move whatever Reduce Motion says.
    private var always = NSHashTable<AnimatedImageLayer>.weakObjects()
    private var link: CADisplayLink?
    private var hz = 0
    private let origin = CACurrentMediaTime()
    private(set) var reduceMotion = UIAccessibility.isReduceMotionEnabled
    private(set) var lowPower = PowerState.shared.lowPower

    private init() {
        let center = NotificationCenter.default
        center.addObserver(forName: UIAccessibility.reduceMotionStatusDidChangeNotification, object: nil,
                           queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.motionChanged() }
        }
        center.addObserver(forName: PowerState.changed, object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.motionChanged() }
        }
    }

    /// Seconds on the shared clock.
    var now: Double { CACurrentMediaTime() - origin }

    var registeredCount: Int { layers.allObjects.count + always.allObjects.count }

    /// Whether a layer of this kind moves now.
    func moves(always: Bool) -> Bool { !lowPower && (always || !reduceMotion) }

    /// A stilled layer stays registered with its first frame, and moves when motion comes back.
    func register(_ layer: AnimatedImageLayer, always: Bool = false) {
        if always {
            self.always.add(layer)
            layers.remove(layer)
        } else {
            layers.add(layer)
            self.always.remove(layer)
        }
        if moves(always: always) {
            layer.advance(to: now)
            startIfNeeded()
        } else {
            layer.showFirstFrame()
        }
    }

    func unregister(_ layer: AnimatedImageLayer) {
        layers.remove(layer)
        always.remove(layer)
        if moving.isEmpty { stop() }
    }

    private var moving: [AnimatedImageLayer] {
        (moves(always: false) ? layers.allObjects : []) + (moves(always: true) ? always.allObjects : [])
    }

    private func startIfNeeded() {
        guard link == nil else { return }
        let link = CADisplayLink(target: Ticker(self), selector: #selector(Ticker.tick(_:)))
        hz = 60
        link.preferredFrameRateRange = Self.range(hz)
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    private func stop() {
        link?.invalidate()
        link = nil
        hz = 0
    }

    fileprivate func tick(_ link: CADisplayLink) {
        let t = link.targetTimestamp - origin
        var shortest = Double.infinity
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for layer in moving {
            layer.advance(to: t)
            if let image = layer.image, image.isAnimated { shortest = min(shortest, image.timeline.shortest) }
        }
        CATransaction.commit()
        guard shortest.isFinite else { return stop() }
        let wanted = ClockRate.hz(shortest: shortest)
        guard wanted != hz else { return }
        hz = wanted
        link.preferredFrameRateRange = Self.range(wanted)
    }

    private static func range(_ hz: Int) -> CAFrameRateRange {
        CAFrameRateRange(minimum: Float(hz), maximum: Float(hz), preferred: Float(hz))
    }

    private func motionChanged() {
        reduceMotion = UIAccessibility.isReduceMotionEnabled
        lowPower = PowerState.shared.lowPower
        if !moves(always: false) { for layer in layers.allObjects { layer.showFirstFrame() } }
        if !moves(always: true) { for layer in always.allObjects { layer.showFirstFrame() } }
        if moving.isEmpty { stop() } else { startIfNeeded() }
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

    /// Shows `image`; `animate` asks for the clock (false draws the first frame), `always` moves
    /// it whatever Reduce Motion says.
    @MainActor
    func show(_ image: DecodedImage?, animate: Bool, always: Bool = false) {
        if self.image !== image {
            self.image = image
            shown = -1
        }
        guard let image else {
            contents = nil
            AnimationClock.shared.unregister(self)
            return
        }
        if animate, image.isAnimated {
            AnimationClock.shared.register(self, always: always)
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
