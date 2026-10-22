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
    /// The glint over the ring or left bar, and the row and look it was built for.
    private var glintLayer: CALayer?
    private var glintFor: (id: String, glint: RowGlint, size: CGSize)?
    /// Rows whose one-shot glint has played, so a relayout or a scroll back never replays it.
    private static var glinted = Set<String>()
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
        showGlint(layout)
    }

    /// The striped-rows background: the layer's own color, under the bitmap and every decor
    /// layer, so flipping it never redraws the row.
    func setStripe(_ color: CGColor?) {
        if layer.backgroundColor != color { layer.backgroundColor = color }
    }

    /// The text bitmap straight on the view's layer, or on a layer of its own when something
    /// draws under it: the author's Atmosphere, or the arrival flash (a layer's own contents draw
    /// under its sublayers, so the text moves up a layer).
    private func showBitmap(_ bitmap: CGImage?, layout: ChatRowLayout, scale: CGFloat) {
        let look = layout.decor.atmosphere
        guard look != nil || layout.decor.flash != nil else {
            wash?.removeFromSuperlayer()
            textLayer?.removeFromSuperlayer()
            wash = nil
            textLayer = nil
            layer.contents = bitmap
            return
        }
        let size = CGSize(width: layout.width, height: layout.height)
        let text = textLayer ?? CALayer()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        text.actions = ["contents": NSNull(), "position": NSNull(), "bounds": NSNull()]
        text.contentsGravity = .topLeft
        text.contentsScale = scale
        text.frame = CGRect(origin: .zero, size: size)
        text.contents = bitmap
        layer.contents = nil
        if look != nil {
            let wash = wash ?? AtmosphereWashLayer()
            self.wash = wash
            showWash(layout, scale: scale)
            // Over the flash, as a desktop row's own background sits under its Atmosphere.
            if wash.superlayer == nil {
                if let flashLayer, flashLayer.superlayer === layer {
                    layer.insertSublayer(wash, above: flashLayer)
                } else {
                    layer.insertSublayer(wash, at: 0)
                }
            }
        } else {
            wash?.removeFromSuperlayer()
            wash = nil
        }
        if text.superlayer == nil {
            if let wash {
                layer.insertSublayer(text, above: wash)
            } else if let flashLayer, flashLayer.superlayer === layer {
                layer.insertSublayer(text, above: flashLayer)
            } else {
                layer.insertSublayer(text, at: 0)
            }
        }
        CATransaction.commit()
        textLayer = text
    }

    /// The author's Atmosphere on the wash layer, moving unless the user chose still.
    private func showWash(_ layout: ChatRowLayout, scale: CGFloat) {
        guard let wash, let look = layout.decor.atmosphere else { return }
        wash.show(look, cologne: layout.decor.cologne, size: CGSize(width: layout.width, height: layout.height), scale: scale,
                  animate: !UIAccessibility.isReduceMotionEnabled && Look.shared.motion == .full)
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
                // A paint image fills the name's box as the renderer stretches it.
                if attachment.kind == .paint { image.contentsGravity = .resize }
                group.addSublayer(image)
                imageLayers.append(image)
                load(url, into: image, attachment: attachment, isBase: i == 0)
            }
            let moving = effectsMove(attachment)
            // Before the flips and turns: a layer's frame cannot be set once it has a transform.
            let outer = Self.travel(group, effects: attachment.effects, moving: moving)
            applyEffects(attachment.effects, to: group, moving: moving)
            if let outline = attachment.mask {
                // Only the name's letters show the moving paint.
                let mask = CAShapeLayer()
                mask.frame = group.bounds
                mask.path = outline
                group.mask = mask
            }
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
            layer.addSublayer(outer)
            groupLayers.append(outer)
        }
        CATransaction.commit()
    }

    /// Whether an emote's modifier effects move (spin, shake, slide, rainbow and the rest): with
    /// its picture (the chat's animate setting), and only at the app's full motion without
    /// Reduce Motion, as desktop stops them under reduced motion.
    private func effectsMove(_ attachment: RowAttachment) -> Bool {
        moves(attachment) && Look.shared.motion == .full && !UIAccessibility.isReduceMotionEnabled
    }

    /// FFZ's Slide (across the emote's box and round again, clipped to it), Arrive and Leave (in
    /// from the left, out to the right, once) on a layer of their own outside the flips and turns,
    /// as desktop nests them. Leave ends gone, as desktop's fill holds it. Returns the outermost
    /// layer, `group` itself when nothing travels.
    private static func travel(_ group: CALayer, effects: UInt32, moving: Bool) -> CALayer {
        guard moving, effects & (Modifier.slide | Modifier.appear | Modifier.leave) != 0 else { return group }
        let box = group.frame
        let w = box.width
        let mover = CALayer()
        mover.actions = ["position": NSNull(), "bounds": NSNull(), "opacity": NSNull(), "transform": NSNull()]
        mover.frame = box
        group.frame = mover.bounds
        mover.addSublayer(group)
        if effects & Modifier.slide != 0 {
            let slide = CABasicAnimation(keyPath: "transform.translation.x")
            slide.fromValue = -w
            slide.toValue = w
            slide.duration = 2
            slide.timingFunction = CAMediaTimingFunction(name: .linear)
            slide.repeatCount = .infinity
            slide.isAdditive = true
            mover.add(slide, forKey: "slide")
        }
        func pass(from: CGFloat, to: CGFloat, fade: (Float, Float), curve: CAMediaTimingFunctionName) -> CAAnimation {
            let move = CABasicAnimation(keyPath: "transform.translation.x")
            move.fromValue = from
            move.toValue = to
            move.isAdditive = true
            let opacity = CABasicAnimation(keyPath: "opacity")
            opacity.fromValue = fade.0
            opacity.toValue = fade.1
            let both = CAAnimationGroup()
            both.animations = [move, opacity]
            both.duration = 0.5
            both.timingFunction = CAMediaTimingFunction(name: curve)
            return both
        }
        if effects & Modifier.appear != 0 {
            mover.add(pass(from: -w, to: 0, fade: (0, 1), curve: .easeOut), forKey: "appear")
        }
        if effects & Modifier.leave != 0 {
            mover.add(pass(from: 0, to: w, fade: (1, 0), curve: .easeIn), forKey: "leave")
            mover.opacity = 0
        }
        guard effects & Modifier.slide != 0 else { return mover }
        let clip = CALayer()
        clip.actions = ["position": NSNull(), "bounds": NSNull()]
        clip.frame = box
        clip.masksToBounds = true
        mover.frame = clip.bounds
        clip.addSublayer(mover)
        return clip
    }

    /// Whether an attachment moves: emotes follow the chat's animate setting, badges always do
    /// (as on desktop), and a paint follows the app's Motion setting, like a row's Atmosphere.
    /// The shared clock stills them all under Reduce Motion.
    private func moves(_ attachment: RowAttachment) -> Bool {
        guard attachment.animate else { return false }
        switch attachment.kind {
        case .badge: return true
        case .paint: return Look.shared.motion == .full
        default: return animateImages
        }
    }

    private func load(_ url: String, into image: AnimatedImageLayer, attachment: RowAttachment, isBase: Bool) {
        let animate = moves(attachment)
        let revision = layout?.revision
        let id = layout?.id
        let unsized = attachment.kind == .emote || attachment.kind == .gif
        let effects = attachment.effects
        let moving = effectsMove(attachment)
        let tinted = EmoteFilter.applies(effects, moving: moving)
        let tintKey = EmoteFilter.key(url, effects: effects, moving: moving, pixelHeight: attachment.pixelHeight, animated: animate)
        if let hit = ImagePipeline.shared.cached(url, pixelHeight: attachment.pixelHeight, animated: animate) {
            if tinted {
                tint(hit, key: tintKey, effects: effects, moving: moving, into: image, animate: animate)
            } else {
                image.show(hit, animate: animate)
            }
            return
        }
        ImagePipeline.shared.load(url, pixelHeight: attachment.pixelHeight, animated: animate) { [weak self, weak image] decoded in
            guard let self, let image, self.layout?.id == id, self.layout?.revision == revision else { return }
            if unsized, isBase, let decoded { EmoteAspects.shared.note(url, aspect: Double(decoded.aspect)) }
            if tinted, let decoded {
                self.tint(decoded, key: tintKey, effects: effects, moving: moving, into: image, animate: animate)
            } else {
                image.show(decoded, animate: animate)
            }
        }
    }

    /// Shows an emote (or one of its overlays) with its colour modifiers drawn in, drawing them
    /// first when no row has yet.
    private func tint(_ decoded: DecodedImage, key: String, effects: UInt32, moving: Bool, into image: AnimatedImageLayer,
                      animate: Bool) {
        if let done = EmoteFilter.cached(key) {
            image.show(done, animate: animate)
            return
        }
        let revision = layout?.revision
        let id = layout?.id
        EmoteFilter.filter(decoded, key: key, effects: effects, moving: moving) { [weak self, weak image] drawn in
            guard let self, let image, self.layout?.id == id, self.layout?.revision == revision else { return }
            image.show(drawn, animate: animate)
        }
    }

    /// Flips and turns as transforms on the emote's group, and the moving effects as layer
    /// animations when `moving`. Colour modifiers are drawn into the frames (`EmoteFilter`);
    /// Slide, Arrive and Leave travel on a layer outside the group (`travel`).
    private func applyEffects(_ effects: UInt32, to group: CALayer, moving: Bool) {
        guard effects != 0 else { return }
        var transform = CATransform3DIdentity
        if effects & Modifier.flipX != 0 { transform = CATransform3DScale(transform, -1, 1, 1) }
        if effects & Modifier.flipY != 0 { transform = CATransform3DScale(transform, 1, -1, 1) }
        if effects & (Modifier.rotate90 | Modifier.rotateRight) != 0 { transform = CATransform3DRotate(transform, .pi / 2, 0, 0, 1) }
        if effects & Modifier.rotateLeft != 0 { transform = CATransform3DRotate(transform, -.pi / 2, 0, 0, 1) }
        group.transform = transform
        guard moving else { return }
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

    /// The live highlight flash: the row's color under its text, tint and bar, stepping down to
    /// clear over two seconds as desktop's keyframes do. Only the layer's opacity animates, on
    /// the render server, so the row never redraws and the list keeps its frame rate. Motion off
    /// (as desktop's) and Reduce Motion leave the row still.
    func flash() {
        guard let flash = layout?.decor.flash, flash.alphas.count > 1, Look.shared.motion != .off,
              !UIAccessibility.isReduceMotionEnabled else { return }
        let overlay = flashLayer ?? CALayer()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        overlay.frame = bounds
        overlay.backgroundColor = flash.color.cgColor
        overlay.opacity = 0
        // At the bottom: under the Atmosphere and the text layer that holds the bar and tint.
        if overlay.superlayer !== layer { layer.insertSublayer(overlay, at: 0) }
        flashLayer = overlay
        CATransaction.commit()
        let steps = flash.alphas.count - 1
        let fade = CAKeyframeAnimation(keyPath: "opacity")
        fade.values = flash.alphas.map { Float($0) }
        fade.keyTimes = (0...steps).map { NSNumber(value: Double($0) / Double(steps)) }
        // CSS `ease-out`, applied to each step as a keyframe animation does.
        fade.timingFunctions = Array(repeating: CAMediaTimingFunction(controlPoints: 0, 0, 0.58, 1), count: steps)
        fade.duration = RowFlash.duration
        overlay.add(fade, forKey: "flash")
    }

    /// The row's glint over its ring or left bar: once when a live row is first shown, or
    /// cycling while it is on screen when the viewer asked for a loop. Layers and Core Animation
    /// only, so the row never redraws. Reduce Motion (or the app's motion off) keeps it still,
    /// and reduced motion plays a loop once.
    private func showGlint(_ layout: ChatRowLayout) {
        let size = CGSize(width: layout.width, height: layout.height)
        if let now = glintFor, glintLayer != nil, now.id == layout.id, now.glint == layout.decor.glint, now.size == size {
            return
        }
        stopGlint()
        let motion = Look.shared.motion
        guard let glint = layout.decor.glint, layout.live, motion != .off, !UIAccessibility.isReduceMotionEnabled,
              size.width > 0, size.height > 0 else { return }
        let loop = glint.loop && !motion.stopsLoops
        if !loop {
            guard !Self.glinted.contains(layout.id) else { return }
            if Self.glinted.count > 2000 { Self.glinted.removeAll() }
            Self.glinted.insert(layout.id)
        }
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let holder = Self.glintHolder(glint, size: size)
        layer.addSublayer(holder)
        Self.animateGlint(glint, holder: holder, loop: loop)
        CATransaction.commit()
        glintLayer = holder
        glintFor = (layout.id, glint, size)
    }

    private func stopGlint() {
        glintLayer?.removeAllAnimations()
        glintLayer?.removeFromSuperlayer()
        glintLayer = nil
        glintFor = nil
    }

    /// The glint's light and where it shows: the ring's 1-point band (a stroked mask) or the bar.
    private static func glintHolder(_ glint: RowGlint, size: CGSize) -> CALayer {
        let holder = CALayer()
        if glint.onRing {
            holder.frame = CGRect(origin: .zero, size: size)
            let band = CAShapeLayer()
            band.frame = holder.bounds
            band.path = UIBezierPath(roundedRect: holder.bounds.insetBy(dx: 0.5, dy: 0.5),
                                     cornerRadius: RowGlint.ringRadius - 0.5).cgPath
            band.fillColor = nil
            band.strokeColor = UIColor.black.cgColor
            band.lineWidth = 1
            holder.mask = band
        } else {
            holder.frame = CGRect(x: 0, y: 0, width: RowGlint.barWidth, height: size.height)
            holder.masksToBounds = true
        }
        let w = holder.bounds.width
        let h = holder.bounds.height
        let light: CALayer
        switch glint.kind {
        case .pulse:
            light = CALayer()
            light.frame = holder.bounds
            light.backgroundColor = UIColor(white: 1, alpha: 0.55).cgColor
            light.opacity = 0
        case .chase where glint.onRing:
            // A wedge of light orbiting the center, on a square that covers the row at any angle.
            let cone = CAGradientLayer()
            cone.type = .conic
            let side = hypot(w, h)
            cone.frame = CGRect(x: (w - side) / 2, y: (h - side) / 2, width: side, height: side)
            cone.startPoint = CGPoint(x: 0.5, y: 0.5)
            cone.endPoint = CGPoint(x: 0.5, y: 0)
            cone.colors = [UIColor.clear, UIColor(white: 1, alpha: 0.9), UIColor.clear, UIColor.clear].map(\.cgColor)
            cone.locations = [0, 24.0 / 360, 56.0 / 360, 1].map { NSNumber(value: $0) }
            light = cone
        case .sheen where glint.onRing:
            // A slanted band two and a half rows wide, swept across from left to right.
            let band = CAGradientLayer()
            let bw = 2.5 * w
            band.frame = CGRect(x: 0, y: 0, width: bw, height: h)
            // The 105 degree slant: the gradient line through the center, as CSS lays it out.
            let angle = 105 * CGFloat.pi / 180
            let dir = CGPoint(x: sin(angle), y: -cos(angle))
            let length = abs(bw * dir.x) + abs(h * dir.y)
            band.startPoint = CGPoint(x: 0.5 - dir.x * length / (2 * bw), y: 0.5 - dir.y * length / (2 * h))
            band.endPoint = CGPoint(x: 0.5 + dir.x * length / (2 * bw), y: 0.5 + dir.y * length / (2 * h))
            band.colors = [UIColor.clear, UIColor(white: 1, alpha: 0.9), UIColor.clear].map(\.cgColor)
            band.locations = [0.38, 0.5, 0.62]
            light = band
        case .sheen, .chase:
            // Down the bar both read as a band of light rising up it.
            let band = CAGradientLayer()
            band.frame = CGRect(x: 0, y: 0, width: w, height: 3 * h)
            band.startPoint = CGPoint(x: 0.5, y: 0)
            band.endPoint = CGPoint(x: 0.5, y: 1)
            band.colors = [UIColor.clear, UIColor(white: 1, alpha: 0.7), UIColor.clear].map(\.cgColor)
            band.locations = [0.38, 0.5, 0.62]
            light = band
        }
        holder.addSublayer(light)
        return holder
    }

    /// The desktop's timings: a one-shot starts 0.15 s after arrival; a loop sweeps or pulses
    /// at the start of every 5 s (the ring's chase orbits without a pause).
    private static func animateGlint(_ glint: RowGlint, holder: CALayer, loop: Bool) {
        guard let light = holder.sublayers?.first else { return }
        let delay: CFTimeInterval = 0.15
        let start = CACurrentMediaTime() + delay
        let easeOut = CAMediaTimingFunction(name: .easeOut)
        let easeInOut = CAMediaTimingFunction(name: .easeInEaseOut)
        let w = holder.bounds.width
        let h = holder.bounds.height

        // A travel from `from` to `to`: once, or in the first 22% of each 5 s cycle.
        func sweep(_ keyPath: String, from: CGFloat, to: CGFloat, once: CFTimeInterval) -> CAAnimation {
            if loop {
                let move = CAKeyframeAnimation(keyPath: keyPath)
                move.values = [from, to, to]
                move.keyTimes = [0, 0.22, 1]
                move.timingFunctions = [easeOut, easeOut]
                move.duration = 5
                move.repeatCount = .infinity
                return move
            }
            let move = CABasicAnimation(keyPath: keyPath)
            move.fromValue = from
            move.toValue = to
            move.duration = once
            move.timingFunction = easeOut
            return move
        }

        let animation: CAAnimation
        switch glint.kind {
        case .pulse:
            let pulse = CAKeyframeAnimation(keyPath: "opacity")
            if loop {
                pulse.values = [0, 1, 0, 0]
                pulse.keyTimes = [0, 0.12, 0.24, 1]
                pulse.timingFunctions = [easeInOut, easeInOut, easeInOut]
                pulse.duration = 5
                pulse.repeatCount = .infinity
            } else {
                pulse.values = [0, 1, 0]
                pulse.keyTimes = [0, 0.45, 1]
                pulse.timingFunctions = [easeInOut, easeInOut]
                pulse.duration = 1.2
            }
            animation = pulse
        case .chase where glint.onRing:
            let spin = CABasicAnimation(keyPath: "transform.rotation.z")
            spin.fromValue = 0
            spin.toValue = 2 * CGFloat.pi
            spin.duration = loop ? 1.8 : 1.4
            spin.timingFunction = CAMediaTimingFunction(name: .linear)
            if loop { spin.repeatCount = .infinity }
            animation = spin
        case .sheen where glint.onRing:
            // The band's left edge from -1.95 to +0.9 row widths (CSS 130% to -60% of a 250% band).
            let half = 1.25 * w
            animation = sweep("position.x", from: -1.95 * w + half, to: 0.9 * w + half, once: 1.1)
            light.position.x = 0.9 * w + half
        case .sheen, .chase:
            // The band's top from 1.2 to -2.6 bar heights (CSS -60% to 130% of a 300% band).
            let half = 1.5 * h
            animation = sweep("position.y", from: 1.2 * h + half, to: -2.6 * h + half, once: 1)
            light.position.y = -2.6 * h + half
        }
        animation.beginTime = start
        animation.fillMode = .backwards
        light.add(animation, forKey: "glint")

        guard !loop else { return }
        // Once played, the holder goes clear and stays so (the chase fades its last stretch
        // rather than stopping on a lit arc), and a clear layer costs the screen nothing.
        let life = CAKeyframeAnimation(keyPath: "opacity")
        life.values = [1, 1, 0]
        life.keyTimes = glint.kind == .chase && glint.onRing ? [0, 0.8, 1] : [0, 0.99, 1]
        life.duration = delay + animation.duration
        holder.add(life, forKey: "life")
        holder.opacity = 0
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
        stopGlint()
        wash?.stop()
    }

    /// Hands the images back to the clock when the row returns to the screen, and starts the
    /// glint and the Atmosphere's motion again.
    func resume() {
        guard let layout else { return }
        showGlint(layout)
        showWash(layout, scale: layer.contentsScale)
        var i = 0
        for attachment in layout.attachments {
            let count = 1 + attachment.overlays.count
            for _ in 0..<count where i < imageLayers.count {
                imageLayers[i].show(imageLayers[i].image, animate: moves(attachment))
                i += 1
            }
        }
    }

    /// What a tap at `point` (in the row) lands on.
    func tapAction(at point: CGPoint) -> RowTapAction? {
        layout?.taps.first { $0.rect.contains(point) }?.action
    }
}
