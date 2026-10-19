import SNKit
import UIKit

/// Swipe right on a message to moderate it: the desktop mod slider with a finger. The core lays
/// out the ruler for this press, from where the thumb landed, and says every word with the
/// chatter's name in it (`SwipeSlider`); this follows the finger, draws the chosen style over the
/// row, buzzes on each step, and on release runs the armed step.
///
/// One move, one direction: the pull only goes right. Short of the cancel zone, or with the finger
/// strayed `leash` points above or below the message, the chip says "Let go to cancel" and a
/// release does nothing. Ban sits at the end of the pull and arms only after the finger rests
/// there; a release before it arms cancels, so a flick past the end does nothing at all.
@MainActor
final class SwipeModerator {
    let slider: SwipeSlider
    let cell: ChatRowCell
    let rowFrame: CGRect
    private let overlay: SwipeModOverlay
    private let commit: (SwipeStep) -> Void
    private(set) var step: Int?
    private var offRow = false
    private var banArmed = false
    private var banTimer: Timer?
    private var banAnchor: CGFloat = 0
    private let tick = UISelectionFeedbackGenerator()
    private let thud = UIImpactFeedbackGenerator(style: .rigid)

    /// `visibleTop` is the list's scroll offset: the chip keeps inside what is on screen.
    init(slider: SwipeSlider, cell: ChatRowCell, in host: UIView, visibleTop: CGFloat, commit: @escaping (SwipeStep) -> Void) {
        self.slider = slider
        self.cell = cell
        self.commit = commit
        rowFrame = cell.frame
        overlay = SwipeModOverlay(slider: slider, row: cell.frame, visibleTop: visibleTop)
        host.addSubview(overlay)
        tick.prepare()
        thud.prepare()
    }

    /// Ban's rest is running: a release now cancels.
    private var resting: Bool { banTimer != nil && !banArmed }

    /// The step a release would apply, nil when it would cancel.
    var armed: SwipeStep? {
        guard !offRow, !resting, let step else { return nil }
        return slider.steps[step]
    }

    /// The finger moved: `pull` points right of where it went down, `y` in the row's space.
    func track(pull: CGFloat, y: CGFloat) {
        let slide = CGFloat(slider.clamp(Double(pull)))
        let wasOff = offRow
        offRow = y < rowFrame.minY - slider.leash || y > rowFrame.maxY + slider.leash
        var next = slider.step(at: Double(slide), keeping: step)
        // Ban arms only after a still rest in its zone; until then the step before it shows.
        if let n = next, slider.steps[n].isBan {
            if banTimer == nil || abs(slide - banAnchor) > slider.banStill {
                startBanRest(at: slide)
            }
            if !banArmed { next = n > 0 ? n - 1 : nil }
        } else {
            stopBanRest()
        }
        if next != step || wasOff != offRow {
            if next != step, next != nil, !offRow { tick.selectionChanged(); tick.prepare() }
            step = next
        }
        draw(slide: slide)
    }

    private func startBanRest(at slide: CGFloat) {
        banTimer?.invalidate()
        banArmed = false
        banAnchor = slide
        banTimer = Timer.scheduledTimer(withTimeInterval: Double(slider.banDwellMs) / 1000, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self else { return }
                self.banArmed = true
                self.step = self.slider.steps.count - 1
                if !self.offRow { self.thud.impactOccurred() }
                self.draw(slide: self.banAnchor)
            }
        }
        // The fill bar starts once the chip has its new width (drawn right after this).
        DispatchQueue.main.async { [weak self] in
            guard let self, self.resting else { return }
            self.overlay.dwell(seconds: Double(self.slider.banDwellMs) / 1000)
        }
    }

    private func stopBanRest() {
        banTimer?.invalidate()
        banTimer = nil
        banArmed = false
        overlay.dwell(seconds: nil)
    }

    private func draw(slide: CGFloat) {
        let words: String
        if offRow || step == nil {
            words = slider.cancelWords
        } else if resting {
            words = slider.holdBanWords
        } else {
            words = slider.steps[step!].chip
        }
        let live = !offRow && step != nil
        let tint = live ? SwipeModOverlay.tint(slider.steps[step!]) : nil
        overlay.show(slide: slide, step: live ? step : nil, words: words, tint: tint, quiet: resting)
        switch slider.style {
        case "capsule":
            cell.transform = CGAffineTransform(translationX: slide, y: 0)
        case "preview":
            cell.alpha = live && !resting ? 0.45 : 1
        default:
            break
        }
    }

    /// The finger lifted: runs the armed step and answers whether anything ran.
    func release() -> Bool {
        guard let step = armed else { return false }
        if step.isBan || step.action == "timeout" {
            UINotificationFeedbackGenerator().notificationOccurred(.warning)
        } else {
            UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
        }
        commit(step)
        return true
    }

    /// Springs the row back and takes the overlay away.
    func finish() {
        stopBanRest()
        overlay.dismiss()
        UIView.animate(withDuration: 0.22, delay: 0, options: [.curveEaseOut, .allowUserInteraction]) { [cell] in
            cell.transform = .identity
            cell.alpha = 1
        }
    }
}

/// What a swipe draws over its row, from where the thumb landed: the step's wash up to the finger
/// and a track of every step along the bottom (Preview), a bar naming every step (Rail), or the
/// row sliding aside (Capsule); plus the chip that says what a release does.
final class SwipeModOverlay: UIView {
    private let slider: SwipeSlider
    private let row: CGRect
    private let visibleTop: CGFloat
    /// The room above the row this overlay keeps for its chip.
    private static let headroom: CGFloat = 60
    private let wash = CALayer()
    private let track = CALayer()
    private var segments: [CALayer] = []
    private let rail = UIView()
    private var railLabels: [UILabel] = []
    private let railPill = UIView()
    private let chip = UILabel()
    private let chipBox = UIView()
    private let dwellBar = CALayer()

    static func tint(_ step: SwipeStep) -> UIColor {
        guard let tone = step.tone else { return ChatTheme.textSecondary.withAlphaComponent(0.85) }
        if step.isBan { return ChatTheme.error }
        return ChatTheme.warning.mixed(with: ChatTheme.error, amount: CGFloat(tone))
    }

    /// Where a pull of `x` points sits, from the row's left edge.
    private func at(_ x: Double) -> CGFloat { CGFloat(slider.start + x) }

    init(slider: SwipeSlider, row: CGRect, visibleTop: CGFloat) {
        self.slider = slider
        self.row = row
        self.visibleTop = visibleTop
        super.init(frame: row.insetBy(dx: 0, dy: -Self.headroom))
        isUserInteractionEnabled = false
        clipsToBounds = false
        let local = CGRect(x: 0, y: Self.headroom, width: row.width, height: row.height)
        if slider.style == "rail" {
            rail.frame = local.insetBy(dx: 4, dy: 2)
            rail.backgroundColor = ChatTheme.surface
            rail.layer.cornerRadius = 10
            rail.layer.cornerCurve = .continuous
            rail.layer.borderWidth = 0.5
            rail.layer.borderColor = UIColor.white.withAlphaComponent(0.08).cgColor
            addSubview(rail)
            railPill.layer.cornerRadius = 9
            railPill.layer.cornerCurve = .continuous
            rail.addSubview(railPill)
            for (i, step) in slider.steps.enumerated() {
                let label = UILabel()
                label.text = step.label
                label.font = .systemFont(ofSize: 12, weight: .semibold)
                label.textColor = Self.tint(step)
                label.textAlignment = .center
                let zone = (i + 1 < slider.starts.count ? slider.starts[i + 1] : slider.end) - slider.starts[i]
                let center = at(slider.starts[i] + zone / 2) - rail.frame.minX
                label.frame = CGRect(x: center - 30, y: 0, width: 60, height: rail.bounds.height)
                rail.addSubview(label)
                railLabels.append(label)
            }
        } else {
            wash.frame = CGRect(x: at(0), y: local.minY, width: 0, height: local.height)
            layer.addSublayer(wash)
            if slider.style == "preview" {
                track.frame = CGRect(x: 0, y: local.maxY - 3, width: local.width, height: 3)
                layer.addSublayer(track)
                for (i, step) in slider.steps.enumerated() {
                    let start = at(slider.starts[i])
                    let end = at(i + 1 < slider.starts.count ? slider.starts[i + 1] : slider.end)
                    let segment = CALayer()
                    segment.frame = CGRect(x: start + 1, y: 0, width: max(2, end - start - 2), height: 3)
                    segment.cornerRadius = 1.5
                    segment.backgroundColor = Self.tint(step).cgColor
                    segment.opacity = 0.32
                    track.addSublayer(segment)
                    segments.append(segment)
                }
            }
        }
        chip.font = .systemFont(ofSize: 13, weight: .semibold)
        chip.textColor = ChatTheme.textPrimary
        chip.textAlignment = .center
        chipBox.backgroundColor = ChatTheme.surface
        chipBox.layer.cornerRadius = 12
        chipBox.layer.cornerCurve = .continuous
        chipBox.layer.borderWidth = 1
        chipBox.addSubview(chip)
        addSubview(chipBox)
        dwellBar.backgroundColor = ChatTheme.error.cgColor
        dwellBar.cornerRadius = 1
        chipBox.layer.addSublayer(dwellBar)
        alpha = 0
        UIView.animate(withDuration: 0.12) { self.alpha = 1 }
    }

    required init?(coder: NSCoder) { fatalError() }

    /// `quiet`: Ban's rest is running, so the chip reads as a warning, not an armed step.
    func show(slide: CGFloat, step: Int?, words: String, tint: UIColor?, quiet: Bool) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let ink = tint ?? ChatTheme.textSecondary
        wash.frame.size.width = slide
        wash.backgroundColor = ink.withAlphaComponent(0.18).cgColor
        for (i, segment) in segments.enumerated() {
            segment.opacity = slider.starts[i] <= Double(slide) ? 1 : 0.32
        }
        CATransaction.commit()
        if let step, step < railLabels.count, !quiet {
            railPill.isHidden = false
            railPill.backgroundColor = ink.withAlphaComponent(0.22)
            railPill.frame = railLabels[step].frame.insetBy(dx: 2, dy: 5)
        } else {
            railPill.isHidden = true
        }
        chip.text = words
        chip.sizeToFit()
        let width = min(chip.bounds.width + 22, bounds.width - 8)
        let finger = at(Double(slide))
        let x = min(max(4, finger - width / 2), bounds.width - width - 4)
        // Above the row, kept inside what the list shows: near the top it comes down over the
        // row instead of going under the finger, where the thumb would cover it. The list fades
        // its top 12 to 30 points, so the chip stays clear of that.
        let listTop = visibleTop - (row.minY - Self.headroom)
        let y = max(Self.headroom - 34, listTop + 30)
        chipBox.frame = CGRect(x: x, y: y, width: width, height: 28)
        chip.frame = chipBox.bounds
        chipBox.layer.borderColor = (quiet ? ChatTheme.error.withAlphaComponent(0.55) : ink.withAlphaComponent(0.55)).cgColor
        chip.textColor = tint == nil ? ChatTheme.textSecondary : ChatTheme.textPrimary
    }

    /// Fills a thin red bar under the chip while Ban's rest runs; nil clears it.
    func dwell(seconds: Double?) {
        dwellBar.removeAllAnimations()
        guard let seconds else {
            dwellBar.frame = .zero
            return
        }
        let full = max(0, chipBox.bounds.width - 12)
        dwellBar.anchorPoint = CGPoint(x: 0, y: 0.5)
        dwellBar.bounds = CGRect(x: 0, y: 0, width: full, height: 2)
        dwellBar.position = CGPoint(x: 6, y: chipBox.bounds.height - 3)
        let grow = CABasicAnimation(keyPath: "bounds.size.width")
        grow.fromValue = 0
        grow.toValue = full
        grow.duration = seconds
        dwellBar.add(grow, forKey: "dwell")
    }

    func dismiss() {
        UIView.animate(withDuration: 0.16, animations: { self.alpha = 0 }) { _ in self.removeFromSuperview() }
    }
}

extension UIColor {
    /// This color moved `amount` (0 to 1) of the way toward `other`.
    func mixed(with other: UIColor, amount: CGFloat) -> UIColor {
        var (r1, g1, b1, a1): (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0)
        var (r2, g2, b2, a2): (CGFloat, CGFloat, CGFloat, CGFloat) = (0, 0, 0, 0)
        getRed(&r1, green: &g1, blue: &b1, alpha: &a1)
        other.getRed(&r2, green: &g2, blue: &b2, alpha: &a2)
        let t = min(max(amount, 0), 1)
        return UIColor(red: r1 + (r2 - r1) * t, green: g1 + (g2 - g1) * t, blue: b1 + (b2 - b1) * t, alpha: a1 + (a2 - a1) * t)
    }
}
