import CoreGraphics
import Foundation
import SNKit

/// The colour modifiers of FrankerFaceZ and BetterTTV emotes, drawn into the decoded frames since
/// Core Animation takes no layer filters on iOS. Greyscale, Sepia, Cursed and Hyper Red always
/// draw; Rainbow and Party cycle their hue as frames on the shared clock while the emote's effects
/// move, and draw nothing when still, as desktop's keyframes do under reduced motion. The steps are
/// the CSS filter functions desktop composes (`emoteModifiers.ts`, `globals.css`), in its order:
/// the still filters innermost, then Rainbow, then Party, each step clamped as a browser's is.
enum EmoteFilter {
    private static let still = Modifier.greyscale | Modifier.sepia | Modifier.cursed | Modifier.hyperRed
    private static let cycling = Modifier.rainbow | Modifier.party
    /// Most frames one cycling emote holds, as the pipeline caps animations.
    private static let frameCap = 240

    /// Whether `effects` change the colours drawn.
    static func applies(_ effects: UInt32, moving: Bool) -> Bool {
        effects & still != 0 || (moving && effects & cycling != 0)
    }

    /// The cache key of one image drawn with `effects`.
    static func key(_ url: String, effects: UInt32, moving: Bool, pixelHeight: Int, animated: Bool) -> String {
        let bits = effects & (still | (moving ? cycling : 0))
        return "\(bits)|\(pixelHeight)|\(animated ? "a" : "s")|\(url)"
    }

    private static let cache = CostCache<String, DecodedImage>(limit: 32 * 1024 * 1024)
    private static let queue = DispatchQueue(label: "app.streamnook.emote.filter", qos: .userInitiated)
    private static let waiting = Waiting()

    private final class Waiting: @unchecked Sendable {
        let lock = NSLock()
        var callbacks: [String: [@MainActor @Sendable (DecodedImage) -> Void]] = [:]
    }

    /// The image drawn with its effects, when it already is.
    static func cached(_ key: String) -> DecodedImage? {
        cache.value(key)
    }

    /// Draws `image` with the colour modifiers in `effects`, once however many rows ask. `done`
    /// runs on the main thread, with the image as it was if it cannot be drawn.
    static func filter(_ image: DecodedImage, key: String, effects: UInt32, moving: Bool,
                       done: @escaping @MainActor @Sendable (DecodedImage) -> Void) {
        let first: Bool = waiting.lock.withLock {
            if waiting.callbacks[key] != nil {
                waiting.callbacks[key]?.append(done)
                return false
            }
            waiting.callbacks[key] = [done]
            return true
        }
        guard first else { return }
        queue.async {
            let made = make(image, effects: effects, moving: moving) ?? image
            if made !== image { cache.insert(made, for: key, cost: made.cost) }
            let callbacks = waiting.lock.withLock { waiting.callbacks.removeValue(forKey: key) ?? [] }
            Task { @MainActor in callbacks.forEach { $0(made) } }
        }
    }

    // MARK: Drawing

    private static func make(_ image: DecodedImage, effects: UInt32, moving: Bool) -> DecodedImage? {
        let fixed = stillSteps(effects)
        let cycle = moving ? effects & cycling : 0
        guard cycle != 0 else {
            guard !fixed.isEmpty else { return nil }
            var frames: [CGImage] = []
            for frame in image.frames {
                guard let drawn = apply(frame, fixed) else { return nil }
                frames.append(drawn)
            }
            return DecodedImage(frames: frames, durations: image.isAnimated ? image.timeline.durations : [])
        }
        // Rainbow turns the hue once every 2 s, Party every 1.5 s; both at once repeat every 6 s.
        let period: Double = cycle == cycling ? 6 : (cycle == Modifier.party ? 1.5 : 2)
        var frames: [CGImage] = []
        var durations: [Double] = []
        if image.isAnimated {
            // The emote's own frames, each at the hue of its time, the animation repeated until it
            // spans a whole turn (within the frame cap).
            let own = image.timeline.durations
            let count = min(own.count, image.frames.count)
            let total = own.prefix(count).reduce(0, +)
            let loops = total >= period || total <= 0 ? 1 : min(Int((period / total).rounded(.up)), max(1, frameCap / max(count, 1)))
            var t = 0.0
            for _ in 0..<loops {
                for i in 0..<count {
                    guard let drawn = apply(image.frames[i], fixed + cyclingSteps(cycle, at: t)) else { return nil }
                    frames.append(drawn)
                    durations.append(own[i])
                    t += own[i]
                }
            }
        } else if let first = image.first {
            // A still emote turns in 15 steps a second.
            let steps = max(2, Int(period * 15))
            for k in 0..<steps {
                let t = Double(k) * period / Double(steps)
                guard let drawn = apply(first, fixed + cyclingSteps(cycle, at: t)) else { return nil }
                frames.append(drawn)
                durations.append(period / Double(steps))
            }
        }
        guard !frames.isEmpty else { return nil }
        return DecodedImage(frames: frames, durations: durations)
    }

    /// Desktop's `MODIFIER_FILTERS`, in table order.
    private static func stillSteps(_ effects: UInt32) -> [Step] {
        var steps: [Step] = []
        if effects & Modifier.greyscale != 0 { steps.append(.grayscale(1)) }
        if effects & Modifier.sepia != 0 { steps.append(.sepia(1)) }
        // grayscale(1) brightness(0.7) contrast(2.5)
        if effects & Modifier.cursed != 0 { steps += [.grayscale(1), .brightness(0.7), .contrast(2.5)] }
        // brightness(0.2) sepia(1) brightness(2.2) contrast(3) saturate(8)
        if effects & Modifier.hyperRed != 0 {
            steps += [.brightness(0.2), .sepia(1), .brightness(2.2), .contrast(3), .saturate(8)]
        }
        return steps
    }

    /// `sn-ffz-rainbow` (hue-rotate over 2 s) then `sn-bttv-party` (sepia(0.5) hue-rotate
    /// saturate(2.5) over 1.5 s) at `t` seconds.
    private static func cyclingSteps(_ cycle: UInt32, at t: Double) -> [Step] {
        var steps: [Step] = []
        if cycle & Modifier.rainbow != 0 { steps.append(.hueRotate(2 * .pi * t / 2)) }
        if cycle & Modifier.party != 0 { steps += [.sepia(0.5), .hueRotate(2 * .pi * t / 1.5), .saturate(2.5)] }
        return steps
    }

    private static let colorSpace = CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB()

    /// `image` with every step applied to its unpremultiplied sRGB colours, as premultiplied BGRA.
    private static func apply(_ image: CGImage, _ steps: [Step]) -> CGImage? {
        let width = image.width
        let height = image.height
        let info = CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue
        guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
                                      space: colorSpace, bitmapInfo: info),
              let base = context.data else { return nil }
        context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
        let rowBytes = context.bytesPerRow
        let bytes = base.assumingMemoryBound(to: UInt8.self)
        for y in 0..<height {
            let row = bytes + y * rowBytes
            for x in 0..<width {
                // Bytes are B, G, R, A.
                let p = row + x * 4
                let alpha = p[3]
                guard alpha > 0 else { continue }
                let a = Float(alpha) / 255
                var r = min(1, Float(p[2]) / 255 / a)
                var g = min(1, Float(p[1]) / 255 / a)
                var b = min(1, Float(p[0]) / 255 / a)
                for step in steps {
                    let m = step.m
                    let nr = m.0 * r + m.1 * g + m.2 * b
                    let ng = m.3 * r + m.4 * g + m.5 * b
                    let nb = m.6 * r + m.7 * g + m.8 * b
                    r = clamp(nr * step.slope + step.offset)
                    g = clamp(ng * step.slope + step.offset)
                    b = clamp(nb * step.slope + step.offset)
                }
                p[2] = UInt8((r * a * 255).rounded())
                p[1] = UInt8((g * a * 255).rounded())
                p[0] = UInt8((b * a * 255).rounded())
            }
        }
        return context.makeImage()
    }

    @inline(__always) private static func clamp(_ v: Float) -> Float {
        min(1, max(0, v))
    }

    /// One CSS filter function: a 3x3 matrix over r, g, b (row major), then each channel times
    /// `slope` plus `offset`, clamped to 0...1.
    private struct Step {
        var m: (Float, Float, Float, Float, Float, Float, Float, Float, Float) = (1, 0, 0, 0, 1, 0, 0, 0, 1)
        var slope: Float = 1
        var offset: Float = 0

        static func brightness(_ amount: Float) -> Step {
            Step(slope: amount)
        }

        static func contrast(_ amount: Float) -> Step {
            Step(slope: amount, offset: 0.5 - 0.5 * amount)
        }

        static func grayscale(_ amount: Float) -> Step {
            let i = 1 - amount
            return Step(m: (0.2126 + 0.7874 * i, 0.7152 - 0.7152 * i, 0.0722 - 0.0722 * i,
                            0.2126 - 0.2126 * i, 0.7152 + 0.2848 * i, 0.0722 - 0.0722 * i,
                            0.2126 - 0.2126 * i, 0.7152 - 0.7152 * i, 0.0722 + 0.9278 * i))
        }

        static func sepia(_ amount: Float) -> Step {
            let i = 1 - amount
            return Step(m: (0.393 + 0.607 * i, 0.769 - 0.769 * i, 0.189 - 0.189 * i,
                            0.349 - 0.349 * i, 0.686 + 0.314 * i, 0.168 - 0.168 * i,
                            0.272 - 0.272 * i, 0.534 - 0.534 * i, 0.131 + 0.869 * i))
        }

        static func saturate(_ s: Float) -> Step {
            Step(m: (0.213 + 0.787 * s, 0.715 - 0.715 * s, 0.072 - 0.072 * s,
                     0.213 - 0.213 * s, 0.715 + 0.285 * s, 0.072 - 0.072 * s,
                     0.213 - 0.213 * s, 0.715 - 0.715 * s, 0.072 + 0.928 * s))
        }

        static func hueRotate(_ radians: Double) -> Step {
            let c = Float(cos(radians))
            let s = Float(sin(radians))
            return Step(m: (0.213 + c * 0.787 - s * 0.213, 0.715 - c * 0.715 - s * 0.715, 0.072 - c * 0.072 + s * 0.928,
                            0.213 - c * 0.213 + s * 0.143, 0.715 + c * 0.285 + s * 0.140, 0.072 - c * 0.072 - s * 0.283,
                            0.213 - c * 0.213 - s * 0.787, 0.715 - c * 0.715 + s * 0.715, 0.072 + c * 0.928 + s * 0.072))
        }
    }
}
