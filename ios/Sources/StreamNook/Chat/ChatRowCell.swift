import QuartzCore
import SNKit
import UIKit

/// One drawn row: its text bitmap on the view's layer and a layer per image over it. Recycled
/// by the list; nothing here measures or lays out text.
@MainActor
final class ChatRowCell: UIView {
    private(set) var layout: ChatRowLayout?
    private var imageLayers: [AnimatedImageLayer] = []
    private var groupLayers: [CALayer] = []
    private var flashLayer: CALayer?
    private var animateImages = true
    /// A member's Atmosphere behind the row, with the text bitmap on its own layer above it.
    private var wash: AtmosphereWashLayer?
    private var textLayer: CALayer?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        layer.contentsGravity = .topLeft
        layer.actions = ["contents": NSNull(), "position": NSNull(), "bounds": NSNull(), "opacity": NSNull()]
        isAccessibilityElement = true
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Shows `layout`. The bitmap must already be drawn (the list draws it before showing).
    func configure(_ layout: ChatRowLayout, scale: CGFloat, animateImages: Bool) {
        let same = self.layout === layout
        self.layout = layout
        self.animateImages = animateImages
        alpha = layout.decor.alpha
        accessibilityLabel = layout.accessibilityText
        guard !same else { return }
        layer.contentsScale = scale
        showBitmap(layout.renderBitmap(scale: scale), layout: layout, scale: scale)
        rebuildImages(layout, scale: scale)
    }

    /// The text bitmap straight on the view's layer, or over the author's Atmosphere when they
    /// wear one (a layer's own contents draw under its sublayers, so the text moves up a layer).
    private func showBitmap(_ bitmap: CGImage?, layout: ChatRowLayout, scale: CGFloat) {
        guard let look = layout.decor.atmosphere else {
            wash?.removeFromSuperlayer()
            textLayer?.removeFromSuperlayer()
            wash = nil
            textLayer = nil
            layer.contents = bitmap
            return
        }
        let size = CGSize(width: layout.width, height: layout.height)
        let wash = wash ?? AtmosphereWashLayer()
        let text = textLayer ?? CALayer()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        text.actions = ["contents": NSNull(), "position": NSNull(), "bounds": NSNull()]
        text.contentsGravity = .topLeft
        text.contentsScale = scale
        text.frame = CGRect(origin: .zero, size: size)
        text.contents = bitmap
        layer.contents = nil
        wash.show(look, cologne: layout.decor.cologne, size: size, scale: scale,
                  animate: !UIAccessibility.isReduceMotionEnabled && Look.shared.motion == .full)
        if wash.superlayer == nil { layer.insertSublayer(wash, at: 0) }
        if text.superlayer == nil { layer.insertSublayer(text, above: wash) }
        CATransaction.commit()
        self.wash = wash
        textLayer = text
    }

    private func rebuildImages(_ layout: ChatRowLayout, scale: CGFloat) {
        for image in imageLayers { image.stopAnimating() }
        groupLayers.forEach { $0.removeFromSuperlayer() }
        groupLayers.removeAll()
        imageLayers.removeAll()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for attachment in layout.attachments {
            let group = CALayer()
            group.frame = attachment.rect
            group.actions = ["position": NSNull(), "bounds": NSNull(), "transform": NSNull()]
            let urls = [attachment.url] + attachment.overlays
            for (i, url) in urls.enumerated() {
                let image = AnimatedImageLayer()
                image.contentsScale = scale
                image.frame = group.bounds
                // FFZ GrowX and BetterTTV wide stretch the target to their box.
                if i == 0, attachment.effects & (Modifier.growX | Modifier.wide) != 0 { image.contentsGravity = .resize }
                if attachment.kind == .preview { image.contentsGravity = .resizeAspectFill }
                group.addSublayer(image)
                imageLayers.append(image)
                load(url, into: image, attachment: attachment, isBase: i == 0)
            }
            applyEffects(attachment.effects, to: group)
            if attachment.kind == .preview {
                group.cornerRadius = attachment.radius
                group.cornerCurve = .continuous
                group.masksToBounds = true
            }
            if attachment.kind == .avatar {
                // A face: round, ringed in the row's channel color.
                group.cornerRadius = attachment.rect.height / 2
                group.masksToBounds = true
                if let ring = layout.decor.leadingBar?.color {
                    group.borderWidth = 1.5
                    group.borderColor = ring.cgColor
                }
            }
            layer.addSublayer(group)
            groupLayers.append(group)
        }
        CATransaction.commit()
    }

    private func load(_ url: String, into image: AnimatedImageLayer, attachment: RowAttachment, isBase: Bool) {
        let animate = attachment.animate && animateImages
        let revision = layout?.revision
        let id = layout?.id
        let unsized = attachment.kind == .emote || attachment.kind == .gif
        if let hit = ImagePipeline.shared.cached(url, pixelHeight: attachment.pixelHeight, animated: animate) {
            image.show(hit, animate: animate)
            return
        }
        ImagePipeline.shared.load(url, pixelHeight: attachment.pixelHeight, animated: animate) { [weak self, weak image] decoded in
            guard let self, let image, self.layout?.id == id, self.layout?.revision == revision else { return }
            if unsized, isBase, let decoded { EmoteAspects.shared.note(url, aspect: Double(decoded.aspect)) }
            image.show(decoded, animate: animate)
        }
    }

    /// Flips and turns as transforms on the emote's group, and the moving effects as layer
    /// animations. Colour filters have no Core Animation equivalent on iOS and draw unfiltered.
    private func applyEffects(_ effects: UInt32, to group: CALayer) {
        guard effects != 0 else { return }
        var transform = CATransform3DIdentity
        if effects & Modifier.flipX != 0 { transform = CATransform3DScale(transform, -1, 1, 1) }
        if effects & Modifier.flipY != 0 { transform = CATransform3DScale(transform, 1, -1, 1) }
        if effects & (Modifier.rotate90 | Modifier.rotateRight) != 0 { transform = CATransform3DRotate(transform, .pi / 2, 0, 0, 1) }
        if effects & Modifier.rotateLeft != 0 { transform = CATransform3DRotate(transform, -.pi / 2, 0, 0, 1) }
        group.transform = transform
        guard !UIAccessibility.isReduceMotionEnabled else { return }
        if effects & Modifier.rotate != 0 {
            let spin = CABasicAnimation(keyPath: "transform.rotation.z")
            spin.fromValue = 0
            spin.toValue = 2 * Double.pi
            spin.duration = 1.5
            spin.repeatCount = .infinity
            spin.isAdditive = true
            group.add(spin, forKey: "spin")
        }
        if effects & (Modifier.shake | Modifier.bttvShake) != 0 {
            let shake = CAKeyframeAnimation(keyPath: "transform.translation.x")
            shake.values = [0, -2, 2, -1, 1, 0]
            shake.duration = 0.5
            shake.repeatCount = .infinity
            shake.isAdditive = true
            group.add(shake, forKey: "shake")
        }
        if effects & Modifier.bounce != 0 {
            let bounce = CAKeyframeAnimation(keyPath: "transform.translation.y")
            bounce.values = [0, -4, 0]
            bounce.duration = 0.6
            bounce.repeatCount = .infinity
            bounce.isAdditive = true
            group.add(bounce, forKey: "bounce")
        }
        if effects & Modifier.jam != 0 {
            let jam = CAKeyframeAnimation(keyPath: "transform.rotation.z")
            jam.values = [0, 0.12, 0, -0.12, 0]
            jam.duration = 0.6
            jam.repeatCount = .infinity
            jam.isAdditive = true
            group.add(jam, forKey: "jam")
        }
    }

    /// The live highlight flash: from `alpha` of the colour to clear over two seconds.
    func flash() {
        guard let flash = layout?.decor.flash, !UIAccessibility.isReduceMotionEnabled else { return }
        let overlay = flashLayer ?? CALayer()
        overlay.frame = bounds
        overlay.backgroundColor = flash.color.cgColor
        overlay.opacity = 0
        if flashLayer == nil {
            layer.insertSublayer(overlay, at: 0)
            flashLayer = overlay
        }
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = Float(flash.alpha)
        fade.toValue = 0
        fade.duration = 2
        fade.timingFunction = CAMediaTimingFunction(name: .easeOut)
        overlay.add(fade, forKey: "flash")
    }

    /// The arrival animation for a new live row.
    func arrive(_ entrance: String) {
        let curve: CAMediaTimingFunction
        let duration: CFTimeInterval
        var transform = CATransform3DIdentity
        switch entrance {
        case "fade":
            curve = CAMediaTimingFunction(name: .easeOut); duration = 0.22
        case "slide":
            curve = CAMediaTimingFunction(controlPoints: 0.16, 1, 0.3, 1); duration = 0.26
            transform = CATransform3DMakeTranslation(-8, 0, 0)
        case "rise":
            curve = CAMediaTimingFunction(controlPoints: 0.22, 1, 0.36, 1); duration = 0.24
            transform = CATransform3DMakeTranslation(0, 6, 0)
        default:
            return
        }
        let group = CAAnimationGroup()
        let opacity = CABasicAnimation(keyPath: "opacity")
        opacity.fromValue = 0
        opacity.toValue = layer.opacity
        let move = CABasicAnimation(keyPath: "transform")
        move.fromValue = NSValue(caTransform3D: transform)
        move.toValue = NSValue(caTransform3D: CATransform3DIdentity)
        group.animations = [opacity, move]
        group.duration = duration
        group.timingFunction = curve
        layer.add(group, forKey: "arrive")
    }

    /// Stops the row's images following the clock; called when it leaves the screen.
    func recycle() {
        for image in imageLayers { image.stopAnimating() }
        layer.removeAllAnimations()
        flashLayer?.removeAllAnimations()
    }

    /// Hands the images back to the clock when the row returns to the screen.
    func resume() {
        guard let layout else { return }
        var i = 0
        for attachment in layout.attachments {
            let count = 1 + attachment.overlays.count
            for _ in 0..<count where i < imageLayers.count {
                imageLayers[i].show(imageLayers[i].image, animate: attachment.animate && animateImages)
                i += 1
            }
        }
    }

    /// What a tap at `point` (in the row) lands on.
    func tapAction(at point: CGPoint) -> RowTapAction? {
        layout?.taps.first { $0.rect.contains(point) }?.action
    }
}
