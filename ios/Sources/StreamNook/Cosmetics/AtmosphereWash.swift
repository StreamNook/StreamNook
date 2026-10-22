import CoreImage
import QuartzCore
import SNKit
import UIKit

/// A StreamNook member's Atmosphere behind one chat row, the way desktop's chat wash draws it:
/// the static base (colour and glow at 85%), then either the image (cover, defocused by the
/// look's blur and overhung by twice it) or the two aurora curtains drifting across the row,
/// the 1 pt rim, and the 1 pt edge down the left. The CS2 Major Cologne look draws its glass
/// texture drifting diagonally under a readability veil, with the coin watermark when worn.
/// Every bitmap is made once per look and size and shared by the rows that need it.
final class AtmosphereWashLayer: CALayer {
    private var key = ""
    private let base = CALayer()
    private let curtain1 = CALayer()
    private let curtain2 = CALayer()
    private let image = AnimatedImageLayer()
    private let rim = CALayer()
    private let edge = CALayer()
    private let veil = CALayer()
    private let coin = CALayer()

    /// Desktop's curtain box: wide enough to cover the drift, a little taller than the row.
    private static let overhang = UIEdgeInsets(top: -28, left: -360, bottom: -28, right: -360)

    override init() {
        super.init()
        masksToBounds = true
        actions = ["position": NSNull(), "bounds": NSNull(), "contents": NSNull(), "hidden": NSNull()]
        for layer in [base, curtain1, curtain2, image, rim, edge, veil, coin] {
            layer.actions = ["position": NSNull(), "bounds": NSNull(), "contents": NSNull(), "hidden": NSNull(),
                             "opacity": NSNull()]
            addSublayer(layer)
        }
        image.contentsGravity = .resizeAspectFill
    }

    override init(layer: Any) {
        super.init(layer: layer)
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Shows `look` over `size`; `animate` false stills the curtains and the drift. With `breathe`
    /// the curtains hold still and only their opacity moves, the way an identity's glow lives
    /// behind a card rather than streaming past like a chat row.
    @MainActor
    func show(_ look: AtmosphereLook, cologne: CologneLook?, size: CGSize, scale: CGFloat, animate: Bool,
              breathe: Bool = false) {
        let next = "\(look.hashValue)|\(cologne?.coin == true)|\(Int(size.width))x\(Int(size.height))|\(animate)|\(breathe)"
        guard next != key else { return }
        key = next
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        frame.size = size
        contentsScale = scale
        for layer in [base, curtain1, curtain2, image, rim, edge, veil, coin] {
            layer.removeAllAnimations()
            layer.isHidden = true
            layer.contentsScale = scale
        }
        let bounds = CGRect(origin: .zero, size: size)
        if look.kind == "cologne-chrome" {
            // Its only motion is the texture's drift, so a breathing glow holds it still.
            showCologne(look, coin: cologne?.coin == true, bounds: bounds, scale: scale, animate: animate && !breathe)
        } else {
            showAtmosphere(look, bounds: bounds, scale: scale, animate: animate, breathe: breathe)
        }
        CATransaction.commit()
    }

    /// Stops it and forgets what it showed, so the next `show` builds it again with its motion:
    /// Core Animation drops a layer's animations once it leaves the window.
    @MainActor
    func stop() {
        key = ""
        for layer in [base, curtain1, curtain2, image, rim, edge, veil, coin] { layer.removeAllAnimations() }
        image.stopAnimating()
    }

    @MainActor
    private func showAtmosphere(_ look: AtmosphereLook, bounds: CGRect, scale: CGFloat, animate: Bool, breathe: Bool) {
        base.frame = bounds
        base.opacity = 0.85
        base.contents = WashBitmaps.fill(SNFill(layers: look.baseLayers.layers + [.solid(color: look.baseColor)]),
                                         size: bounds.size, scale: scale)
        base.isHidden = false
        if let url = look.image {
            let blur = CGFloat(look.chatBlur ?? 0)
            image.frame = bounds.insetBy(dx: -blur * 2, dy: -blur * 2)
            image.isHidden = false
            image.contentsScale = scale
            let height = Int(image.frame.height * scale)
            WashBitmaps.blurred(url, blur: blur * scale, pixelHeight: height) { [weak self] decoded in
                guard let self, !self.image.isHidden else { return }
                self.image.show(decoded, animate: animate)
            }
        } else {
            let box = bounds.inset(by: Self.overhang)
            for (layer, fill, n) in [(curtain1, look.layers, 1), (curtain2, look.layers2, 2)] where !fill.isEmpty {
                layer.frame = box
                layer.contents = WashBitmaps.fill(fill, size: box.size, scale: scale)
                layer.isHidden = false
                if animate {
                    layer.add(breathe ? Self.breathing(n) : Self.drift(n), forKey: "drift")
                } else {
                    layer.opacity = n == 1 ? 0.78 : 0.6
                }
            }
        }
        if let fill = look.chatRim {
            rim.frame = bounds
            rim.contents = WashBitmaps.rim(fill, size: bounds.size, scale: scale)
            rim.isHidden = false
        }
        if let fill = look.chatEdge {
            edge.frame = CGRect(x: 0, y: 0, width: 1, height: bounds.height)
            edge.contents = WashBitmaps.fill(fill, size: edge.frame.size, scale: scale)
            edge.isHidden = false
        }
    }

    /// Desktop's `sn-aurora-1` (9 s in chat) and `sn-aurora-2` (12 s): a drift across one curtain
    /// period with a gentle sway and breathing opacity.
    private static func drift(_ n: Int) -> CAAnimation {
        let x = CAKeyframeAnimation(keyPath: "transform.translation.x")
        let y = CAKeyframeAnimation(keyPath: "transform.translation.y")
        let o = CAKeyframeAnimation(keyPath: "opacity")
        if n == 1 {
            x.values = [0, -160, -320]
            y.values = [0, -12, 0]
            o.values = [0.78, 1, 0.78]
        } else {
            x.values = [0, 120, 240]
            y.values = [0, 12, 0]
            o.values = [0.6, 0.92, 0.6]
        }
        let group = CAAnimationGroup()
        group.animations = [x, y, o]
        group.duration = n == 1 ? 9 : 12
        group.repeatCount = .infinity
        group.timingFunction = CAMediaTimingFunction(name: .linear)
        group.preferredFrameRateRange = FrameRate.decor
        return group
    }

    /// The curtains' breathing alone (the chat drift's opacity curve and periods, no travel):
    /// opacity-only, so the compositor runs it at full frame rate with nothing to repaint.
    private static func breathing(_ n: Int) -> CAAnimation {
        let o = CAKeyframeAnimation(keyPath: "opacity")
        o.values = n == 1 ? [0.78, 1, 0.78] : [0.6, 0.92, 0.6]
        o.duration = n == 1 ? 9 : 12
        o.repeatCount = .infinity
        o.timingFunctions = [CAMediaTimingFunction(name: .easeInEaseOut), CAMediaTimingFunction(name: .easeInEaseOut)]
        o.preferredFrameRateRange = FrameRate.decor
        return o
    }

    @MainActor
    private func showCologne(_ look: AtmosphereLook, coin wantsCoin: Bool, bounds: CGRect, scale: CGFloat, animate: Bool) {
        let tile: CGFloat = 230
        base.frame = bounds
        base.opacity = 1
        base.contents = WashBitmaps.fill(SNFill(layers: [.solid(color: look.baseColor)]), size: bounds.size, scale: scale)
        base.isHidden = false
        if let url = look.chromeTexture {
            let box = bounds.insetBy(dx: -tile, dy: -tile)
            curtain1.frame = box
            curtain1.opacity = 1
            curtain1.isHidden = false
            WashBitmaps.tiled(url, tile: tile, size: box.size, scale: scale, brightness: 1.18) { [weak self] cg in
                guard let self, !self.curtain1.isHidden else { return }
                self.curtain1.contents = cg
            }
            if animate {
                let move = CABasicAnimation(keyPath: "transform.translation")
                move.fromValue = NSValue(cgSize: .zero)
                move.toValue = NSValue(cgSize: CGSize(width: -tile, height: -tile))
                move.duration = 32
                move.repeatCount = .infinity
                move.preferredFrameRateRange = FrameRate.slowDecor
                curtain1.add(move, forKey: "drift")
            }
        }
        veil.frame = bounds
        veil.contents = WashBitmaps.fill(SNFill(layers: [.linear(angle: 90, repeating: false, stops: [
            .init(color: "#080c169e", at: 0), .init(color: "#080c164d", at: 0.55), .init(color: "#080c161f", at: 1),
        ])]), size: bounds.size, scale: scale)
        veil.isHidden = false
        if wantsCoin, let url = look.chromeCoin {
            let side = min(44, bounds.height * 0.8)
            coin.frame = CGRect(x: bounds.width - 14 - side, y: bounds.midY - side / 2, width: side, height: side)
            coin.opacity = 0.3
            coin.contentsGravity = .resizeAspect
            coin.isHidden = false
            ImagePipeline.shared.load(url, pixelHeight: Int(side * scale), animated: false) { [weak self] decoded in
                self?.coin.contents = decoded?.first
            }
        }
        edge.frame = CGRect(x: 0, y: 0, width: 2, height: bounds.height)
        edge.contents = WashBitmaps.fill(SNFill(layers: [.linear(angle: 180, repeating: false, stops: [
            .init(color: "#f6e3a0ff", at: 0), .init(color: "#9c7528ff", at: 1),
        ])]), size: edge.frame.size, scale: scale)
        edge.isHidden = false
    }
}

/// The wash's bitmaps, made once per look, size and scale.
enum WashBitmaps {
    nonisolated(unsafe) private static let cache: NSCache<NSString, CGImage> = {
        let c = NSCache<NSString, CGImage>()
        c.totalCostLimit = 24 * 1024 * 1024
        return c
    }()
    nonisolated(unsafe) private static let blurredCache = NSCache<NSString, DecodedImage>()
    private static let context = CIContext(options: [.cacheIntermediates: false])

    static func fill(_ fill: SNFill, size: CGSize, scale: CGFloat) -> CGImage? {
        let key = "f|\(fill.hashValue)|\(Int(size.width))x\(Int(size.height))@\(scale)" as NSString
        if let hit = cache.object(forKey: key) { return hit }
        let image = draw(size: size, scale: scale) { ctx, rect in FillPainter.fill(fill, in: rect, ctx, imageScale: scale) }
        if let image { cache.setObject(image, forKey: key, cost: image.bytesPerRow * image.height) }
        return image
    }

    /// The fill painted across the box with everything but its outer 1 pt masked away.
    static func rim(_ fill: SNFill, size: CGSize, scale: CGFloat) -> CGImage? {
        let key = "r|\(fill.hashValue)|\(Int(size.width))x\(Int(size.height))@\(scale)" as NSString
        if let hit = cache.object(forKey: key) { return hit }
        let image = draw(size: size, scale: scale) { ctx, rect in
            let ring = CGMutablePath()
            ring.addRect(rect)
            ring.addRect(rect.insetBy(dx: 1, dy: 1))
            ctx.addPath(ring)
            ctx.clip(using: .evenOdd)
            FillPainter.fill(fill, in: rect, ctx, imageScale: scale)
        }
        if let image { cache.setObject(image, forKey: key, cost: image.bytesPerRow * image.height) }
        return image
    }

    private static func draw(size: CGSize, scale: CGFloat, _ body: (CGContext, CGRect) -> Void) -> CGImage? {
        guard size.width > 0, size.height > 0 else { return nil }
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        format.opaque = false
        format.preferredRange = .standard
        return UIGraphicsImageRenderer(size: size, format: format).image { context in
            body(context.cgContext, CGRect(origin: .zero, size: size))
        }.cgImage
    }

    /// An atmosphere image defocused by `blur` pixels, every frame of it, once per URL and size.
    @MainActor
    static func blurred(_ url: String, blur: CGFloat, pixelHeight: Int, done: @escaping @MainActor (DecodedImage?) -> Void) {
        let key = "\(url)|\(Int(blur))|\(pixelHeight)"
        if let hit = blurredCache.object(forKey: key as NSString) {
            done(hit)
            return
        }
        // Decoded smaller than drawn: a defocused picture loses nothing, and the blur gets cheap.
        let decodeHeight = max(64, pixelHeight / 2)
        ImagePipeline.shared.load(url, pixelHeight: decodeHeight, animated: true) { decoded in
            guard let decoded else { return done(nil) }
            guard blur > 0 else { return done(decoded) }
            let radius = blur / 2
            Task.detached(priority: .utility) {
                let frames = decoded.frames.compactMap { frame -> CGImage? in
                    let input = CIImage(cgImage: frame).clampedToExtent()
                    let output = input.applyingGaussianBlur(sigma: Double(radius)).cropped(to: CIImage(cgImage: frame).extent)
                    return context.createCGImage(output, from: output.extent)
                }
                let result = DecodedImage(frames: frames, durations: decoded.timeline.durations)
                await MainActor.run {
                    blurredCache.setObject(result, forKey: key as NSString)
                    done(result)
                }
            }
        }
    }

    /// A texture repeated at `tile` points over a box, brightened, made once per size.
    @MainActor
    static func tiled(_ url: String, tile: CGFloat, size: CGSize, scale: CGFloat, brightness: CGFloat,
                      done: @escaping @MainActor (CGImage?) -> Void) {
        let key = "t|\(url)|\(Int(tile))|\(Int(size.width))x\(Int(size.height))@\(scale)" as NSString
        if let hit = cache.object(forKey: key) {
            done(hit)
            return
        }
        ImagePipeline.shared.load(url, pixelHeight: Int(tile * scale), animated: false) { decoded in
            guard let texture = decoded?.first else { return done(nil) }
            let image = draw(size: size, scale: scale) { ctx, rect in
                var y: CGFloat = 0
                while y < rect.height {
                    var x: CGFloat = 0
                    while x < rect.width {
                        FillPainter.draw(texture, cover: true, in: CGRect(x: x, y: y, width: tile, height: tile), ctx)
                        x += tile
                    }
                    y += tile
                }
                // CSS `brightness(1.18)`: lighten by adding a share of white.
                ctx.setBlendMode(.plusLighter)
                ctx.setFillColor(UIColor(white: 1, alpha: max(0, brightness - 1) * 0.5).cgColor)
                ctx.fill(rect)
            }
            if let image { cache.setObject(image, forKey: key, cost: image.bytesPerRow * image.height) }
            done(image)
        }
    }
}
