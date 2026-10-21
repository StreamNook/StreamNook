import SwiftUI
import UIKit

/// A cosmetic image (a badge, an avatar, a paint layer) through the shared image pipeline: it
/// tries each URL in turn, animates when the file moves and `animate` allows, and draws nothing
/// at all when every URL fails. Its box never changes size; only the image in it does.
struct CosmeticImage: UIViewRepresentable {
    let urls: [String]
    var animate = true
    /// Animation follows the shared clock, which stills under Reduce Motion; `alwaysMove` keeps
    /// moving regardless (7TV profile pictures follow their own setting, not the system's).
    var alwaysMove = false
    var fill = false
    /// Stretched to the box, as CSS `background-size: 100% 100%` (a paint's image layer).
    var stretch = false
    /// The decode height in points; the view's own height when nil.
    var pointHeight: CGFloat?
    var onFailure: (() -> Void)?

    init(_ url: String?, fallbacks: [String] = [], animate: Bool = true, alwaysMove: Bool = false, fill: Bool = false,
         stretch: Bool = false, pointHeight: CGFloat? = nil, onFailure: (() -> Void)? = nil) {
        urls = ([url].compactMap { $0 } + fallbacks).filter { !$0.isEmpty }
        self.animate = animate
        self.alwaysMove = alwaysMove
        self.fill = fill
        self.stretch = stretch
        self.pointHeight = pointHeight
        self.onFailure = onFailure
    }

    func makeUIView(context: Context) -> CosmeticImageView {
        let view = CosmeticImageView()
        view.isUserInteractionEnabled = false
        return view
    }

    func updateUIView(_ view: CosmeticImageView, context: Context) {
        view.onFailure = onFailure
        // Under the full player nobody sees the shell's pictures: they hold their frame.
        view.covered = context.environment.shellCovered
        view.configure(urls: urls, animate: animate, alwaysMove: alwaysMove, fill: fill, stretch: stretch, pointHeight: pointHeight)
    }
}

@MainActor
final class CosmeticImageView: UIView {
    private let imageLayer = AnimatedImageLayer()
    private var urls: [String] = []
    private var animate = true
    private var alwaysMove = false
    private var pointHeight: CGFloat?
    private var generation = 0
    var covered = false {
        didSet { if covered != oldValue, let image = imageLayer.image { show(image) } }
    }
    var onFailure: (() -> Void)?

    override init(frame: CGRect) {
        super.init(frame: frame)
        isOpaque = false
        layer.addSublayer(imageLayer)
    }

    required init?(coder: NSCoder) { fatalError() }

    override func layoutSubviews() {
        super.layoutSubviews()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        imageLayer.frame = bounds
        CATransaction.commit()
        if imageLayer.image == nil, !urls.isEmpty, bounds.height > 0 { load(from: 0) }
    }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window == nil {
            imageLayer.stopAnimating()
        } else if let image = imageLayer.image {
            show(image)
        }
    }

    func configure(urls: [String], animate: Bool, alwaysMove: Bool, fill: Bool, stretch: Bool, pointHeight: CGFloat?) {
        imageLayer.contentsGravity = stretch ? .resize : fill ? .resizeAspectFill : .resizeAspect
        imageLayer.masksToBounds = fill
        imageLayer.contentsScale = traitCollection.displayScale
        let changed = urls != self.urls || animate != self.animate || alwaysMove != self.alwaysMove
        self.animate = animate
        self.alwaysMove = alwaysMove
        self.pointHeight = pointHeight
        guard changed else { return }
        self.urls = urls
        generation += 1
        // The old picture stays until the new one has decoded, so the box never blanks.
        if bounds.height > 0 || pointHeight != nil { load(from: 0) }
    }

    private func load(from index: Int) {
        guard index < urls.count else {
            imageLayer.show(nil, animate: false)
            onFailure?()
            return
        }
        let url = urls[index]
        let pixels = Int(((pointHeight ?? bounds.height) * traitCollection.displayScale).rounded(.up))
        guard pixels > 0 else { return }
        let token = generation
        if let hit = ImagePipeline.shared.cached(url, pixelHeight: pixels, animated: animate) {
            show(hit)
            return
        }
        ImagePipeline.shared.load(url, pixelHeight: pixels, animated: animate) { [weak self] decoded in
            guard let self, self.generation == token else { return }
            if let decoded { self.show(decoded) } else { self.load(from: index + 1) }
        }
    }

    private func show(_ image: DecodedImage) {
        imageLayer.show(image, animate: animate && window != nil && !covered, always: alwaysMove)
    }
}
