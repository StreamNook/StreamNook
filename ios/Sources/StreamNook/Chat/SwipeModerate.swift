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
    /// The step zone under the finger, before Ban's rest is applied: a buzz is a change of zone.
    private var zone: Int?
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
        offRow = y < rowFrame.minY - slider.leash || y > rowFrame.maxY + slider.leash
        // While Ban's rest runs, Ban is the step its edge rule keeps, so a finger trembling on
        // the edge does not start the rest over.
        let banIndex = slider.steps.count - 1
        let keeping = banTimer != nil && banIndex >= 0 ? banIndex : step
        let here = slider.step(at: Double(slide), keeping: keeping)
        var next = here
        // Ban arms only after a still rest in its zone, on the row; until then the step before it shows.
        if let n = next, slider.steps[n].isBan, !offRow {
            if banTimer == nil || abs(slide - banAnchor) > slider.banStill {
                startBanRest(at: slide)
            }
            if !banArmed { next = n > 0 ? n - 1 : nil }
        } else {
            stopBanRest()
            if let n = next, slider.steps[n].isBan { next = n > 0 ? n - 1 : nil }
        }
        // A buzz marks a new zone under the finger, never Ban arming or disarming (that has its own).
        if here != zone {
            if here != nil, !offRow { tick.selectionChanged(); tick.prepare() }
            zone = here
        }
        step = next
        draw(slide: slide)
    }

    private func startBanRest(at slide: CGFloat) {
        banTimer?.invalidate()
        banArmed = false
        banAnchor = slide
        let timer = Timer(timeInterval: Double(slider.banDwellMs) / 1000, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, !self.offRow else { return }
                self.banArmed = true
                self.step = self.slider.steps.count - 1
                self.thud.impactOccurred()
                self.draw(slide: self.banAnchor)
            }
        }
        // Common modes: a finger on the screen keeps the run loop out of the default mode.
        RunLoop.main.add(timer, forMode: .common)
        banTimer = timer
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
            let fade = (cell.layer.mask as? CAGradientLayer) ?? CAGradientLayer()
            fade.frame = cell.bounds
            fade.startPoint = CGPoint(x: 0, y: 0.5)
            fade.endPoint = CGPoint(x: 1, y: 0.5)
            fade.colors = [UIColor.black.cgColor, UIColor.black.cgColor, UIColor.clear.cgColor, UIColor.clear.cgColor]
            let w = max(cell.bounds.width, 1)
            let edge = max(0, w - slide)
            fade.locations = [0, NSNumber(value: Double(max(0, edge - 24) / w)), NSNumber(value: Double(edge / w)), 1]
            cell.layer.mask = fade
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
        cell.layer.mask = nil
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
    /// The pull's wash from the row's left edge to the finger, its leading edge faded.
    private let wash = CAGradientLayer()
    private let track = CALayer()
    private var segments: [CALayer] = []
    private let rail = UIView()
    private var railLabels: [UILabel] = []
    private let railPill = UIView()
    private let chip = UILabel()
    private let chipBox = UIView()
    /// The chip's lift off the chat: a solid backing with a soft shadow under it (the chip itself
    /// clips its dwell fill, so it cannot carry the shadow).
    private let chipLift = UIView()
    /// Ban's rest filling the chip behind its words.
    private let dwellBar = CALayer()
    private static let chipHeight: CGFloat = 28
    /// The chip keeps to the text column's inset, never the screen's edge.
    private static let chipInset: CGFloat = 12

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
            // Solid: the rail covers the message, and a see-through one let its text clash with
            // the step names.
            rail.backgroundColor = ChatTheme.raisedSolid
            rail.layer.cornerRadius = 10
            rail.layer.cornerCurve = .continuous
            rail.layer.borderWidth = 0.5
            rail.layer.borderColor = ChatTheme.borderSubtle.cgColor
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
                // A legend spread evenly along the rail, so short zones never crowd their labels
                // together; the lit pill marks the step under the finger.
                let slot = rail.bounds.width / CGFloat(max(slider.steps.count, 1))
                label.frame = CGRect(x: slot * CGFloat(i), y: 0, width: slot, height: rail.bounds.height)
                rail.addSubview(label)
                railLabels.append(label)
            }
        } else {
            wash.frame = CGRect(x: 0, y: local.minY, width: 0, height: local.height)
            wash.startPoint = CGPoint(x: 0, y: 0.5)
            wash.endPoint = CGPoint(x: 1, y: 0.5)
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
        // Solid, so the words read over any message text above the row, on every theme.
        chipBox.backgroundColor = ChatTheme.raisedSolid
        chipBox.layer.cornerRadius = 12
        chipBox.layer.cornerCurve = .continuous
        chipBox.layer.borderWidth = 1
        chipBox.clipsToBounds = true
        dwellBar.backgroundColor = ChatTheme.error.withAlphaComponent(0.2).cgColor
        chipBox.layer.addSublayer(dwellBar)
        chipBox.addSubview(chip)
        chipLift.backgroundColor = ChatTheme.raisedSolid
        chipLift.layer.cornerRadius = 12
        chipLift.layer.cornerCurve = .continuous
        chipLift.layer.shadowColor = UIColor.black.cgColor
        chipLift.layer.shadowOpacity = 0.28
        chipLift.layer.shadowRadius = 6
        chipLift.layer.shadowOffset = CGSize(width: 0, height: 2)
        addSubview(chipLift)
        addSubview(chipBox)
        alpha = 0
        UIView.animate(withDuration: 0.12) { self.alpha = 1 }
    }

    required init?(coder: NSCoder) { fatalError() }

    /// `quiet`: Ban's rest is running, so the chip reads as a warning, not an armed step.
    func show(slide: CGFloat, step: Int?, words: String, tint: UIColor?, quiet: Bool) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let ink = tint ?? ChatTheme.textSecondary
        let washWidth = at(Double(slide))
        wash.frame.size.width = washWidth
        let lead = NSNumber(value: Double(min(1, 16 / max(washWidth, 1))))
        wash.colors = [ink.withAlphaComponent(0).cgColor, ink.withAlphaComponent(0.18).cgColor, ink.withAlphaComponent(0.18).cgColor]
        wash.locations = [0, lead, 1]
        for (i, segment) in segments.enumerated() {
            segment.opacity = slider.starts[i] <= Double(slide) ? 1 : 0.32
        }
        CATransaction.commit()
        if let step, step < railLabels.count, !quiet {
            railPill.isHidden = false
            railPill.backgroundColor = ink.withAlphaComponent(0.22)
            let label = railLabels[step]
            let pill = label.intrinsicContentSize.width + 16
            railPill.frame = CGRect(x: label.frame.midX - pill / 2, y: label.frame.minY + 5, width: pill, height: label.frame.height - 10)
        } else {
            railPill.isHidden = true
        }
        chip.text = words
        chip.sizeToFit()
        let inset = Self.chipInset
        let width = min(chip.bounds.width + 22, bounds.width - inset * 2)
        let finger = at(Double(slide))
        let x = min(max(inset, finger - width / 2), bounds.width - width - inset)
        // Above the row, kept inside what the list shows: near the top it comes down over the
        // row instead of going under the finger, where the thumb would cover it. The list fades
        // its top 12 to 30 points, so the chip stays clear of that.
        let listTop = visibleTop - (row.minY - Self.headroom)
        // Never over the track along the row's bottom either.
        let y = min(max(Self.headroom - 34, listTop + 30), Self.headroom + row.height - 3 - Self.chipHeight - 4)
        chipBox.frame = CGRect(x: x, y: y, width: width, height: Self.chipHeight)
        chip.frame = chipBox.bounds
        chipLift.frame = chipBox.frame
        chipLift.layer.shadowPath = UIBezierPath(roundedRect: chipLift.bounds, cornerRadius: 12).cgPath
        chipBox.layer.borderColor = (quiet ? ChatTheme.error.withAlphaComponent(0.55) : ink.withAlphaComponent(0.55)).cgColor
        chip.textColor = tint == nil ? ChatTheme.textSecondary : ChatTheme.textPrimary
    }

    /// Fills the chip from the left behind its words while Ban's rest runs; nil clears it.
    func dwell(seconds: Double?) {
        dwellBar.removeAllAnimations()
        guard let seconds else {
            dwellBar.frame = .zero
            return
        }
        let full = chipBox.bounds.width
        dwellBar.anchorPoint = CGPoint(x: 0, y: 0.5)
        dwellBar.bounds = CGRect(x: 0, y: 0, width: full, height: chipBox.bounds.height)
        dwellBar.position = CGPoint(x: 0, y: chipBox.bounds.height / 2)
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
