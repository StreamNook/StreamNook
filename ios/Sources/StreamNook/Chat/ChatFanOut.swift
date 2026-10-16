import SNKit
import UIKit

/// What a long press on a row can do. Distance is severity: everyday actions sit on the near
/// arc, heavier ones on the far arc, so a ban takes a longer reach than a delete.
enum FanBucket: CaseIterable {
    case reply, profile, delete, copy, timeout, ban, pin

    var title: String {
        switch self {
        case .reply: return "Reply"
        case .profile: return "Profile"
        case .delete: return "Delete"
        case .copy: return "Copy"
        case .timeout: return "Timeout"
        case .ban: return "Ban"
        case .pin: return "Pin"
        }
    }

    var symbol: String {
        switch self {
        case .reply: return "arrowshape.turn.up.left"
        case .profile: return "person"
        case .delete: return "trash"
        case .copy: return "doc.on.doc"
        case .timeout: return "clock"
        case .ban: return "nosign"
        case .pin: return "pin"
        }
    }

    /// The solid fill of the active tile.
    var fill: UIColor {
        switch self {
        case .delete: return UIColor(red: 0.92, green: 0.35, blue: 0.05, alpha: 1)
        case .timeout: return UIColor(red: 0.85, green: 0.47, blue: 0.02, alpha: 1)
        case .ban: return UIColor(red: 0.86, green: 0.15, blue: 0.15, alpha: 1)
        case .pin: return UIColor(red: 0.01, green: 0.52, blue: 0.78, alpha: 1)
        case .reply: return UIColor(red: 0.49, green: 0.23, blue: 0.93, alpha: 1)
        case .copy: return UIColor(red: 0.32, green: 0.32, blue: 0.36, alpha: 1)
        case .profile: return UIColor(red: 0.31, green: 0.27, blue: 0.9, alpha: 1)
        }
    }

    var danger: Bool { self == .delete || self == .timeout || self == .ban }
    var near: Bool { self == .reply || self == .profile || self == .delete }
}

/// The timeout dial: the distance past 30 pt from the Timeout tile, on a tenth-power ramp over
/// 260 pt, up to 14 days, snapped to 5 s under a minute, whole minutes under an hour, half hours
/// under a day and whole days above.
enum TimeoutRamp {
    static let maxSeconds = 14.0 * 86400

    static func seconds(forDistance distance: CGFloat) -> Int? {
        let past = distance - 30
        guard past > 0 else { return nil }
        let ratio = min(1, Double(past) / 260)
        let raw = pow(ratio, 10) * maxSeconds
        let snapped: Double
        switch raw {
        case ..<60: snapped = (raw / 5).rounded() * 5
        case ..<3600: snapped = (raw / 60).rounded() * 60
        case ..<86400: snapped = (raw / 1800).rounded() * 1800
        default: snapped = (raw / 86400).rounded() * 86400
        }
        return Int(min(max(snapped, 1), maxSeconds))
    }

    /// Which unit a duration reads in; a step buzz fires when it changes.
    static func unit(_ seconds: Int) -> Int {
        switch seconds {
        case ..<60: return 0
        case ..<3600: return 1
        case ..<86400: return 2
        default: return 3
        }
    }
}

/// The fan of action tiles a long press opens over a dimmed screen. It follows the same touch:
/// slide to a tile, release to do it, release elsewhere to cancel.
@MainActor
final class ChatFanOverlay: UIView {
    private let buckets: [FanBucket]
    private var tiles: [FanBucket: FanTile] = [:]
    private var centers: [FanBucket: CGPoint] = [:]
    private let chip = UILabel()
    private let chipBox = UIVisualEffectView(effect: UIGlassEffect())
    private(set) var active: FanBucket?
    private var timeoutEngaged = false
    private(set) var dialled: Int?
    private var lastUnit = -1
    private let selection = UISelectionFeedbackGenerator()

    init(frame: CGRect, buckets: [FanBucket], press: CGPoint, name: String, nameColor: UIColor) {
        self.buckets = buckets
        super.init(frame: frame)
        backgroundColor = UIColor.black.withAlphaComponent(0.45)
        alpha = 0
        let center = CGPoint(x: bounds.midX, y: min(max(press.y, 302), bounds.height - 8))
        place(center: center)
        for bucket in buckets {
            let tile = FanTile(bucket: bucket)
            tile.center = press
            tile.transform = CGAffineTransform(scaleX: 0.4, y: 0.4)
            addSubview(tile)
            tiles[bucket] = tile
        }
        chip.text = name
        chip.font = .systemFont(ofSize: 14, weight: .bold)
        chip.textColor = nameColor
        chip.sizeToFit()
        chipBox.contentView.addSubview(chip)
        // At 0 percent glassiness (or with blur off) nothing is see-through: the chip turns solid.
        if !Look.shared.frosted {
            chipBox.effect = nil
            chipBox.backgroundColor = UIColor(SN.backgroundTertiary)
        }
        chipBox.layer.cornerRadius = 12
        chipBox.clipsToBounds = true
        let chipSize = CGSize(width: chip.bounds.width + 24, height: chip.bounds.height + 16)
        chipBox.frame = CGRect(x: center.x - chipSize.width / 2, y: max(28, center.y - 150 - 32 - 60), width: chipSize.width, height: chipSize.height)
        chip.frame = CGRect(x: 12, y: 8, width: chip.bounds.width, height: chip.bounds.height)
        chipBox.transform = CGAffineTransform(scaleX: 0.6, y: 0.6).rotated(by: -4 * .pi / 180)
        addSubview(chipBox)
        isAccessibilityElement = false
        selection.prepare()
    }

    required init?(coder: NSCoder) { fatalError() }

    /// Near arc 92 pt over 120 degrees, far arc 150 pt over 160 degrees, both centred straight up.
    private func place(center: CGPoint) {
        for (arc, radius, span) in [(buckets.filter(\.near), 92.0, 120.0), (buckets.filter { !$0.near }, 150.0, 160.0)] {
            for (i, bucket) in arc.enumerated() {
                let degrees = arc.count == 1 ? 0 : -span / 2 + span * Double(i) / Double(arc.count - 1)
                let angle = degrees * .pi / 180
                centers[bucket] = CGPoint(x: center.x + CGFloat(sin(angle) * radius), y: center.y - CGFloat(cos(angle) * radius))
            }
        }
    }

    func present() {
        UIView.animate(withDuration: 0.1) { self.alpha = 1 }
        for (i, bucket) in buckets.enumerated() {
            guard let tile = tiles[bucket], let target = centers[bucket] else { continue }
            UIView.animate(springDuration: 0.28, bounce: 0.15, initialSpringVelocity: 0, delay: Double(i) * 0.012, options: []) {
                tile.center = target
                tile.transform = .identity
            }
        }
        UIView.animate(springDuration: 0.3, bounce: 0.3) {
            self.chipBox.transform = CGAffineTransform(rotationAngle: -4 * .pi / 180)
        }
    }

    /// The finger moved: the nearest tile within 60 pt is active; Timeout stays engaged once
    /// reached and dials its duration from the distance.
    func track(_ point: CGPoint) {
        var next: FanBucket?
        if timeoutEngaged, let timeout = centers[.timeout] {
            next = .timeout
            let distance = hypot(point.x - timeout.x, point.y - timeout.y)
            let seconds = TimeoutRamp.seconds(forDistance: distance)
            if seconds != dialled {
                dialled = seconds
                if let seconds {
                    let unit = TimeoutRamp.unit(seconds)
                    if unit != lastUnit, lastUnit >= 0 { UIImpactFeedbackGenerator(style: .soft).impactOccurred() }
                    lastUnit = unit
                }
                tiles[.timeout]?.setLabel(seconds.map(ChatRowLayoutBuilder.duration) ?? "Timeout")
            }
        } else {
            var best: (FanBucket, CGFloat)?
            for (bucket, center) in centers {
                let d = hypot(point.x - center.x, point.y - center.y)
                if d <= 60, d < (best?.1 ?? .infinity) { best = (bucket, d) }
            }
            next = best?.0
            if next == .timeout { timeoutEngaged = true }
        }
        guard next != active else { return }
        if let active { tiles[active]?.setActive(false) }
        active = next
        if let next {
            tiles[next]?.setActive(true)
            selection.selectionChanged()
        }
        UIView.animate(springDuration: 0.3, bounce: 0.3) {
            self.chipBox.transform = next == nil
                ? CGAffineTransform(rotationAngle: -4 * .pi / 180)
                : CGAffineTransform(scaleX: 0.86, y: 0.86)
        }
    }

    func center(of bucket: FanBucket) -> CGPoint? {
        centers[bucket]
    }

    func dismiss() {
        UIView.animate(withDuration: 0.12, animations: { self.alpha = 0 }, completion: { _ in self.removeFromSuperview() })
    }
}

/// One 64 pt action tile: solid dark until active, then solid in its action's colour.
@MainActor
final class FanTile: UIView {
    let bucket: FanBucket
    private let icon = UIImageView()
    private let label = UILabel()

    init(bucket: FanBucket) {
        self.bucket = bucket
        super.init(frame: CGRect(x: 0, y: 0, width: 64, height: 64))
        backgroundColor = UIColor(red: 0.094, green: 0.094, blue: 0.106, alpha: 1)
        layer.cornerRadius = 16
        layer.borderWidth = 1
        layer.borderColor = UIColor.white.withAlphaComponent(0.15).cgColor
        layer.shadowColor = UIColor.black.cgColor
        layer.shadowOpacity = 0.5
        layer.shadowRadius = 11
        layer.shadowOffset = CGSize(width: 0, height: 8)
        layer.shadowPath = UIBezierPath(roundedRect: bounds, cornerRadius: 16).cgPath
        icon.image = UIImage(systemName: bucket.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 20, weight: .semibold))
        icon.tintColor = UIColor.white.withAlphaComponent(0.8)
        icon.contentMode = .center
        icon.frame = CGRect(x: 0, y: 10, width: 64, height: 26)
        label.text = bucket.title
        label.font = .systemFont(ofSize: 11, weight: .semibold)
        label.textColor = UIColor.white.withAlphaComponent(0.8)
        label.textAlignment = .center
        label.frame = CGRect(x: 2, y: 38, width: 60, height: 16)
        label.adjustsFontSizeToFitWidth = true
        addSubview(icon)
        addSubview(label)
    }

    required init?(coder: NSCoder) { fatalError() }

    func setLabel(_ text: String) {
        label.text = text
    }

    func setActive(_ on: Bool) {
        backgroundColor = on ? bucket.fill : UIColor(red: 0.094, green: 0.094, blue: 0.106, alpha: 1)
        icon.tintColor = on ? .white : UIColor.white.withAlphaComponent(0.8)
        label.textColor = icon.tintColor
        UIView.animate(springDuration: 0.28, bounce: 0.45) {
            self.transform = on ? CGAffineTransform(scaleX: 1.18, y: 1.18) : .identity
        }
        if on, bucket.danger, !UIAccessibility.isReduceMotionEnabled {
            let wiggle = CAKeyframeAnimation(keyPath: "transform.rotation.z")
            wiggle.values = [0, -10 * Double.pi / 180, 6 * Double.pi / 180, 0]
            wiggle.duration = 0.3
            icon.layer.add(wiggle, forKey: "wiggle")
        }
    }
}
