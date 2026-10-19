import QuartzCore
import SNKit
import UIKit

/// The chat list: a scroll view that places pre-laid-out rows by hand and recycles their views.
/// Live, it keeps the newest row at the bottom; a finger dragging older rows into view pauses it
/// until the reader scrolls back down or taps Resume live.
@MainActor
final class ChatListView: UIView, UIScrollViewDelegate, UIGestureRecognizerDelegate {
    struct Row {
        var layout: ChatRowLayout
        var y: CGFloat
        var id: String { layout.id }
        var height: CGFloat { layout.height }
        /// Whether the row wears the striped-rows background; given once and kept (see `restripe`).
        var striped: Bool?
    }

    /// Batches made off the main thread, applied at the next display frame.
    var source: ChatPipeline? {
        didSet { oldValue?.list = nil; source?.list = self }
    }

    var readOnly = false { didSet { updateOverlay() } }
    var onTap: ((RowTapAction, ChatRowLayout) -> Void)?
    var onReply: ((String) -> Void)?
    var onLongPress: ((ChatRowLayout, CGPoint) -> Void)?
    var onPausedChange: ((Bool) -> Void)?
    var onWidthChange: ((CGFloat) -> Void)?

    let scroll = UIScrollView()
    private(set) var rows: [Row] = []
    private var cells: [String: ChatRowCell] = [:]
    private var pool: [ChatRowCell] = []
    private var flashed: Set<String> = []
    private let topInset: CGFloat = 40
    private var displayLink: CADisplayLink?
    private var lastWidth: CGFloat = 0
    private var style = ChatStyle()

    // Follow and pause.
    private(set) var paused = false
    private var lastTransition = CACurrentMediaTime()
    private var lastTranslation: CGFloat = 0
    private var leftBottomBand = false
    private var programmatic = false
    private var glide: (start: CGFloat, began: CFTimeInterval, duration: CFTimeInterval)?

    private let resumeButton = UIButton(configuration: .glass())
    private let hint = UILabel()
    private let hintBox = UIVisualEffectView(effect: UIGlassEffect())
    private let overlayStack = UIStackView()

    let pacing = FramePacing()
    private var ticks = 0

    override init(frame: CGRect) {
        super.init(frame: frame)
        backgroundColor = .clear
        scroll.delegate = self
        scroll.alwaysBounceVertical = true
        scroll.showsVerticalScrollIndicator = false
        scroll.contentInsetAdjustmentBehavior = .never
        scroll.scrollsToTop = false
        scroll.delaysContentTouches = false
        addSubview(scroll)

        let tap = UITapGestureRecognizer(target: self, action: #selector(tapped(_:)))
        scroll.addGestureRecognizer(tap)
        press.addTarget(self, action: #selector(pressed(_:)))
        press.minimumPressDuration = 0.42
        press.allowableMovement = 10
        press.delegate = self
        scroll.addGestureRecognizer(press)
        // Swipe right on a message to moderate it. The list's own scroll waits for this to fail,
        // which it does at once unless the finger heads right over a row the viewer moderates.
        swipePan.addTarget(self, action: #selector(swiped(_:)))
        swipePan.delegate = self
        swipePan.maximumNumberOfTouches = 1
        scroll.addGestureRecognizer(swipePan)
        scroll.panGestureRecognizer.require(toFail: swipePan)
        // The thumb landing on a row the viewer moderates holds the list at once, so the row
        // under it is the row a swipe acts on, however fast the chat runs.
        touchDown.addTarget(self, action: #selector(touched(_:)))
        touchDown.minimumPressDuration = 0
        touchDown.allowableMovement = .greatestFiniteMagnitude
        touchDown.cancelsTouchesInView = false
        touchDown.delaysTouchesEnded = false
        touchDown.delegate = self
        scroll.addGestureRecognizer(touchDown)

        var config = UIButton.Configuration.glass()
        config.title = "Resume live"
        config.image = UIImage(systemName: "chevron.down", withConfiguration: UIImage.SymbolConfiguration(pointSize: 11, weight: .bold))
        config.imagePlacement = .trailing
        config.imagePadding = 4
        config.cornerStyle = .capsule
        config.contentInsets = NSDirectionalEdgeInsets(top: 4, leading: 12, bottom: 4, trailing: 12)
        config.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { attrs in
            var attrs = attrs
            attrs.font = UIFont.systemFont(ofSize: 12, weight: .medium)
            return attrs
        }
        resumeButton.configuration = config
        resumeButton.addAction(UIAction { [weak self] _ in self?.resume(force: true, glide: true) }, for: .touchUpInside)

        hint.text = "Tap a message to reply"
        hint.font = .systemFont(ofSize: 10.5)
        hint.textColor = ChatTheme.textMuted
        hint.translatesAutoresizingMaskIntoConstraints = false
        hintBox.contentView.addSubview(hint)
        hintBox.layer.cornerRadius = 9
        hintBox.clipsToBounds = true
        hintBox.isUserInteractionEnabled = false
        NSLayoutConstraint.activate([
            hint.leadingAnchor.constraint(equalTo: hintBox.contentView.leadingAnchor, constant: 8),
            hint.trailingAnchor.constraint(equalTo: hintBox.contentView.trailingAnchor, constant: -8),
            hint.topAnchor.constraint(equalTo: hintBox.contentView.topAnchor, constant: 2),
            hint.bottomAnchor.constraint(equalTo: hintBox.contentView.bottomAnchor, constant: -2),
        ])
        overlayStack.axis = .vertical
        overlayStack.alignment = .center
        overlayStack.spacing = 4
        overlayStack.addArrangedSubview(hintBox)
        overlayStack.addArrangedSubview(resumeButton)
        overlayStack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(overlayStack)
        NSLayoutConstraint.activate([
            overlayStack.centerXAnchor.constraint(equalTo: centerXAnchor),
            overlayStack.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
        ])
        updateOverlay()
    }

    required init?(coder: NSCoder) { fatalError() }

    override func didMoveToWindow() {
        super.didMoveToWindow()
        if window != nil {
            startLink()
            for cell in cells.values { cell.resume() }
        } else {
            displayLink?.invalidate()
            displayLink = nil
            for cell in cells.values { cell.recycle() }
        }
    }

    private func startLink() {
        guard displayLink == nil else { return }
        let link = CADisplayLink(target: LinkTarget(self), selector: #selector(LinkTarget.tick(_:)))
        link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
        link.add(to: .main, forMode: .common)
        displayLink = link
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        scroll.frame = bounds
        if bounds.width != lastWidth, bounds.width > 0 {
            lastWidth = bounds.width
            onWidthChange?(bounds.width)
        }
        updateInsets()
        if !paused { pinToBottom() }
        layoutVisible()
    }

    // MARK: Applying batches

    fileprivate func tick(_ link: CADisplayLink) {
        pacing.frame(link)
        ticks += 1
        // While a thumb rests on (or swipes) a message the list holds still: new rows wait until
        // it lifts or turns the press into a scroll.
        if let source, swipe == nil, touchPress == nil {
            let batches = source.takeReady()
            if !batches.isEmpty { apply(batches) }
        }
        if glide != nil { stepGlide(link) }
        if ticks % 60 == 0 { releaseFarBitmaps() }
    }

    func setStyle(_ style: ChatStyle) {
        self.style = style
    }

    private func apply(_ batches: [ChatPipeline.Batch]) {
        var newLive: [String] = []
        var animate = true
        var mentioned = false
        for batch in batches {
            if let reset = batch.reset {
                for cell in cells.values { recycle(cell) }
                cells.removeAll()
                rows.removeAll()
                var y = topInset
                for layout in reset {
                    rows.append(Row(layout: layout, y: y))
                    y += layout.height
                }
                animate = false
                if batch.forceLive { paused = false; updateOverlay() }
                continue
            }
            if !batch.prepended.isEmpty {
                var added: [Row] = []
                var y = topInset
                for layout in batch.prepended {
                    added.append(Row(layout: layout, y: y))
                    y += layout.height
                }
                let shift = y - topInset
                for i in rows.indices { rows[i].y += shift }
                rows.insert(contentsOf: added, at: 0)
                if paused { adjustOffset(by: shift) }
            }
            var y = rows.last.map { $0.y + $0.height } ?? topInset
            for layout in batch.appended {
                rows.append(Row(layout: layout, y: y))
                y += layout.height
                if layout.live {
                    newLive.append(layout.id)
                    if layout.mentionsMe { mentioned = true }
                }
            }
            if batch.trimmed > 0 {
                let count = min(batch.trimmed, rows.count)
                let removed = rows.prefix(count)
                let shift = (rows.count > count ? rows[count].y : y) - topInset
                for row in removed {
                    if let cell = cells.removeValue(forKey: row.id) { recycle(cell) }
                    flashed.remove(row.id)
                }
                rows.removeFirst(count)
                for i in rows.indices { rows[i].y -= shift }
                if paused { adjustOffset(by: -shift) }
            }
            if !batch.updated.isEmpty {
                var byID: [String: ChatRowLayout] = [:]
                for layout in batch.updated { byID[layout.id] = layout }
                var delta: CGFloat = 0
                let anchor = scroll.contentOffset.y
                var deltaAbove: CGFloat = 0
                for i in rows.indices {
                    rows[i].y += delta
                    if let layout = byID[rows[i].id] {
                        let change = layout.height - rows[i].height
                        if rows[i].y + rows[i].height < anchor { deltaAbove += change }
                        rows[i].layout = layout
                        delta += change
                    }
                }
                if paused, deltaAbove != 0 { adjustOffset(by: deltaAbove) }
            }
        }
        if style.alternatingBackgrounds { restripe() }
        updateInsets()
        if !paused { pinToBottom() }
        layoutVisible()
        if animate, !paused, style.messageEntrance != "none", !UIAccessibility.isReduceMotionEnabled {
            for id in newLive.suffix(3) { cells[id]?.arrive(style.messageEntrance) }
        }
        if mentioned { MentionBuzz.fire() }
    }

    /// Every other shown row wears the stripe. A row keeps its stripe once given one, so trimming
    /// the oldest rows or loading older history never flips the rows already on screen; rows a
    /// setting hides are skipped, as the desktop list leaves them out.
    private func restripe() {
        guard let anchor = rows.firstIndex(where: { $0.height > 0 && $0.striped != nil })
                ?? rows.firstIndex(where: { $0.height > 0 }) else { return }
        let base = rows[anchor].striped ?? false
        var next = base
        for i in anchor..<rows.count where rows[i].height > 0 {
            rows[i].striped = next
            next.toggle()
        }
        next = !base
        for i in stride(from: anchor - 1, through: 0, by: -1) where rows[i].height > 0 {
            rows[i].striped = next
            next.toggle()
        }
    }

    private func adjustOffset(by dy: CGFloat) {
        programmatic = true
        scroll.contentOffset.y += dy
        programmatic = false
    }

    private var contentHeight: CGFloat { (rows.last.map { $0.y + $0.height } ?? topInset) }

    private func updateInsets() {
        let height = contentHeight
        scroll.contentSize = CGSize(width: bounds.width, height: height)
        // Few rows sit at the bottom, where the newest one always is.
        let top = max(0, bounds.height - height)
        if scroll.contentInset.top != top {
            programmatic = true
            scroll.contentInset.top = top
            programmatic = false
        }
    }

    private var bottomOffset: CGFloat {
        max(-scroll.contentInset.top, scroll.contentSize.height - scroll.bounds.height + scroll.contentInset.bottom)
    }

    private func pinToBottom() {
        let target = bottomOffset
        guard abs(scroll.contentOffset.y - target) > 0.25 else { return }
        programmatic = true
        scroll.contentOffset.y = target
        programmatic = false
    }

    // MARK: Placing cells

    private func layoutVisible() {
        guard !rows.isEmpty else {
            for cell in cells.values { recycle(cell) }
            cells.removeAll()
            return
        }
        let top = scroll.contentOffset.y - 60
        let bottom = scroll.contentOffset.y + scroll.bounds.height + 60
        var low = 0
        var high = rows.count - 1
        while low < high {
            let mid = (low + high) / 2
            if rows[mid].y + rows[mid].height < top { low = mid + 1 } else { high = mid }
        }
        var wanted = Set<String>()
        var i = low
        let scale = window?.screen.scale ?? traitCollection.displayScale
        let animate = style.animatesEmotes
        // The theme's surface behind every other row (`.chat-striped` on desktop).
        let stripe = style.alternatingBackgrounds ? ChatTheme.surface.cgColor : nil
        while i < rows.count, rows[i].y <= bottom {
            let row = rows[i]
            if row.height > 0 {
                wanted.insert(row.id)
                let cell: ChatRowCell
                if let existing = cells[row.id] {
                    cell = existing
                } else {
                    cell = pool.popLast() ?? ChatRowCell()
                    cells[row.id] = cell
                    scroll.addSubview(cell)
                }
                let frame = CGRect(x: 0, y: row.y, width: bounds.width, height: row.height)
                if cell.frame != frame { cell.frame = frame }
                if cell.layout !== row.layout {
                    cell.configure(row.layout, scale: scale, animateImages: animate)
                    // Built only for VoiceOver: it reads the row from the chat queue.
                    cell.accessibilityCustomActions = UIAccessibility.isVoiceOverRunning ? accessibilityActions?(row.layout) : nil
                }
                cell.setStripe(row.striped == true ? stripe : nil)
                if row.layout.decor.flash != nil, row.layout.live, !flashed.contains(row.id) {
                    flashed.insert(row.id)
                    cell.flash()
                }
            }
            i += 1
        }
        for (id, cell) in cells where !wanted.contains(id) {
            cells[id] = nil
            recycle(cell)
        }
    }

    private func recycle(_ cell: ChatRowCell) {
        cell.recycle()
        cell.removeFromSuperview()
        if pool.count < 40 { pool.append(cell) }
    }

    /// Bitmaps of rows far from the screen are dropped; they are drawn again if scrolled back to.
    private func releaseFarBitmaps() {
        let top = scroll.contentOffset.y - 2 * scroll.bounds.height
        let bottom = scroll.contentOffset.y + 3 * scroll.bounds.height
        for row in rows where row.y + row.height < top || row.y > bottom {
            if cells[row.id] == nil, row.layout.hasBitmap { row.layout.dropBitmap() }
        }
    }

    // MARK: Pause and resume

    private func setPaused(_ value: Bool, force: Bool) {
        guard value != paused else { return }
        let now = CACurrentMediaTime()
        guard force || now - lastTransition > 0.12 else { return }
        lastTransition = now
        paused = value
        leftBottomBand = false
        source?.setPaused(value)
        onPausedChange?(value)
        updateOverlay()
    }

    /// Back to live: a glide to the bottom (or a snap for a short gap), the overflow then trimmed.
    func resume(force: Bool, glide wantsGlide: Bool) {
        setPaused(false, force: force)
        let distance = bottomOffset - scroll.contentOffset.y
        if wantsGlide, style.smoothScrollOnResume, distance >= 40 {
            let duration = min(max(distance * 0.4, 200), 420) / 1000
            glide = (scroll.contentOffset.y, CACurrentMediaTime(), duration)
        } else {
            pinToBottom()
            layoutVisible()
        }
    }

    /// Pauses for a reply jump, past the settle window.
    func pauseForJump() {
        setPaused(true, force: true)
    }

    private func stepGlide(_ link: CADisplayLink) {
        guard let g = glide else { return }
        let t = min(1, (link.targetTimestamp - g.began) / g.duration)
        let eased = 1 - pow(1 - t, 3)
        let target = bottomOffset
        programmatic = true
        scroll.contentOffset.y = g.start + (target - g.start) * eased
        programmatic = false
        layoutVisible()
        if t >= 1 { glide = nil }
    }

    private func updateOverlay() {
        overlayStack.isHidden = !paused
        hintBox.isHidden = readOnly
    }

    func scrollViewWillBeginDragging(_ scrollView: UIScrollView) {
        glide = nil
        // The press became a scroll: the list is the finger's again.
        if swipe == nil { touchPress = nil }
        lastTranslation = scrollView.panGestureRecognizer.translation(in: scrollView).y
    }

    func scrollViewDidScroll(_ scrollView: UIScrollView) {
        layoutVisible()
        guard !programmatic else { return }
        if scrollView.isTracking {
            let translation = scrollView.panGestureRecognizer.translation(in: scrollView).y
            let moved = translation - lastTranslation
            lastTranslation = translation
            // A finger travelling down reveals older rows: that intent pauses at once.
            if moved > 0.5, !paused, rows.count > 0, scroll.contentSize.height > scroll.bounds.height - scroll.contentInset.top {
                setPaused(true, force: false)
            }
        }
        if paused {
            let distance = bottomOffset - scrollView.contentOffset.y
            if distance > 80 { leftBottomBand = true }
            if leftBottomBand, distance <= 30 { setPaused(false, force: false) }
        }
    }

    // MARK: Gestures

    private func row(at point: CGPoint) -> (Row, ChatRowCell)? {
        for (id, cell) in cells where cell.frame.contains(point) {
            if let row = rows.first(where: { $0.id == id }) { return (row, cell) }
        }
        return nil
    }

    @objc private func tapped(_ gesture: UITapGestureRecognizer) {
        let point = gesture.location(in: scroll)
        guard let (row, cell) = row(at: point) else { return }
        let local = CGPoint(x: point.x - cell.frame.minX, y: point.y - cell.frame.minY)
        if let action = cell.tapAction(at: local) {
            onTap?(action, row.layout)
            return
        }
        // A row moving under the finger is never a reply target; a paused one is.
        if paused, !readOnly { onReply?(row.id) }
    }

    /// The fan's actions for VoiceOver, which cannot make the long-press gesture.
    var accessibilityActions: ((ChatRowLayout) -> [UIAccessibilityCustomAction])?
    /// The fan's buckets, the chatter's name and colour for a row; nil when the row has none.
    var fanContext: ((ChatRowLayout) -> (buckets: [FanBucket], name: String, color: UIColor)?)?
    /// A released bucket, with the dialled timeout in seconds.
    var onFanCommit: ((FanBucket, ChatRowLayout, Int?) -> Void)?
    /// A release on the fan's Timeout without dialing: the swipe's first timeout, from the core.
    var quickSeconds = 600
    private var fan: ChatFanOverlay?
    private var fanRow: ChatRowLayout?

    @objc private func pressed(_ gesture: UILongPressGestureRecognizer) {
        guard let window else { return }
        switch gesture.state {
        case .began:
            let point = gesture.location(in: scroll)
            guard swipe == nil, let (row, _) = row(at: point), let context = fanContext?(row.layout), !context.buckets.isEmpty
            else { return }
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            onLongPress?(row.layout, gesture.location(in: self))
            // Once armed the list holds still for the rest of the gesture.
            scroll.isScrollEnabled = false
            let overlay = ChatFanOverlay(frame: window.bounds, buckets: context.buckets, press: gesture.location(in: window),
                                         name: context.name, nameColor: context.color)
            window.addSubview(overlay)
            overlay.present()
            fan = overlay
            fanRow = row.layout
        case .changed:
            fan?.track(gesture.location(in: window))
        case .ended:
            if let fan, let row = fanRow, let bucket = fan.active {
                if bucket.danger {
                    UINotificationFeedbackGenerator().notificationOccurred(.warning)
                } else {
                    UIImpactFeedbackGenerator(style: .rigid).impactOccurred()
                }
                onFanCommit?(bucket, row, bucket == .timeout ? (fan.dialled ?? quickSeconds) : nil)
            }
            endFan()
        default:
            endFan()
        }
    }

    private func endFan() {
        fan?.dismiss()
        fan = nil
        fanRow = nil
        scroll.isScrollEnabled = true
    }

    #if DEBUG
    /// Opens the fan over the newest visible row as a press there would, then moves the finger to
    /// `bucket` (and `reach` points past it), for screenshots of the fan.
    func debugFan(select bucket: FanBucket?, reach: CGFloat) {
        guard let window, let row = rows.last(where: { cells[$0.id] != nil }),
              let context = fanContext?(row.layout) else { return }
        endFan()
        let cell = cells[row.id]!
        let press = cell.convert(CGPoint(x: cell.bounds.midX, y: cell.bounds.midY), to: window)
        let overlay = ChatFanOverlay(frame: window.bounds, buckets: context.buckets, press: press, name: context.name,
                                     nameColor: context.color)
        window.addSubview(overlay)
        overlay.present()
        fan = overlay
        fanRow = row.layout
        guard let bucket else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
            guard let target = overlay.center(of: bucket) else { return }
            overlay.track(target)
            if reach > 0 { overlay.track(CGPoint(x: target.x, y: target.y - reach)) }
        }
    }

    func debugEndFan() {
        endFan()
    }
    #endif

    func gestureRecognizer(_ g: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        // The touch-down watcher only watches; everything else runs alongside it.
        g === touchDown || other === touchDown
    }

    // MARK: Swipe to moderate

    /// A swipe ready to start: the core's slider for this press and what a release does with its
    /// step, captured for the chatter under the thumb at touch-down.
    struct PreparedSwipe {
        var slider: SwipeSlider
        var commit: (SwipeStep) -> Void
    }

    /// Swiping is on here (moderator tools on, the room known); the core's slider comes per press.
    var swipeEnabled = false {
        didSet { press.allowableMovement = swipeEnabled ? 6 : 10 }
    }
    /// Whether a row can be moderated with a swipe (a Twitch chatter the viewer may act on).
    var swipeAllowed: ((ChatRowLayout) -> Bool)?
    /// The slider for a press `startX` points into a row, and the release action, for that row.
    var prepareSwipe: ((ChatRowLayout, _ width: CGFloat, _ startX: CGFloat) async -> PreparedSwipe?)?
    /// A swipe started (the hint goes once one has).
    var onSwipeDone: (() -> Void)?
    /// The long press that opens the fan; it gives way sooner to a sideways drift while swiping is on.
    private let press = UILongPressGestureRecognizer()
    private let swipePan = UIPanGestureRecognizer()
    private let touchDown = UILongPressGestureRecognizer()
    private var swipe: SwipeModerator?
    /// The press a swipe may start from: its row, where the thumb landed, and once the core has
    /// answered, the prepared swipe.
    private var touchPress: (row: Row, cell: ChatRowCell, startX: CGFloat, prepared: PreparedSwipe?)?

    @objc private func touched(_ g: UILongPressGestureRecognizer) {
        switch g.state {
        case .began:
            // Whether this room is moderated is read live (`swipeAllowed`), never from a flag the
            // SwiftUI host pushes in, which could be stale when the roles land after the list.
            guard !readOnly, swipe == nil, fan == nil, swipeAllowed != nil else { return }
            let point = g.location(in: scroll)
            guard let (row, cell) = row(at: point) else { return }
            guard swipeAllowed?(row.layout) == true else {
                chatLog.notice("[swipe] down on \(row.id, privacy: .public): not swipeable here")
                return
            }
            let startX = point.x - cell.frame.minX
            touchPress = (row, cell, startX, nil)
            chatLog.notice("[swipe] down on \(row.id, privacy: .public) at x \(Int(startX)) of \(Int(cell.frame.width)); slider requested")
            let id = row.id
            Task { @MainActor [weak self] in
                guard let self, let prepare = self.prepareSwipe else { return }
                let ready = await prepare(row.layout, cell.frame.width, startX)
                guard self.touchPress?.row.id == id else { return }
                if let ready {
                    chatLog.notice("[swipe] slider ready: enabled \(ready.slider.enabled) steps \(ready.slider.steps.count) start \(Int(ready.slider.start))")
                } else {
                    chatLog.notice("[swipe] slider missing (see the error above)")
                }
                self.touchPress?.prepared = ready
            }
        case .ended, .cancelled, .failed:
            if swipe == nil { touchPress = nil }
        default:
            break
        }
    }

    override func gestureRecognizerShouldBegin(_ g: UIGestureRecognizer) -> Bool {
        guard g === swipePan else { return true }
        guard !readOnly, swipe == nil, fan == nil, let held = touchPress else { return false }
        // The core's slider may still be on its way; only a press it turned down is refused here.
        if let ready = held.prepared, !ready.slider.enabled {
            chatLog.notice("[swipe] pan refused: swipe to moderate is off")
            return false
        }
        let t = swipePan.translation(in: scroll)
        // Only a mainly vertical move (the list's scroll) or a leftward one is refused.
        let begin = t.x > 0 && abs(t.x) >= abs(t.y)
        chatLog.notice("[swipe] pan \(begin ? "begins" : "refused (vertical or leftward)", privacy: .public) at \(Int(t.x)),\(Int(t.y)); slider \(held.prepared == nil ? "pending" : "ready", privacy: .public)")
        return begin
    }

    /// How far right of where the thumb went down the finger is now.
    private func pull(_ g: UIGestureRecognizer, _ held: (row: Row, cell: ChatRowCell, startX: CGFloat, prepared: PreparedSwipe?)) -> CGFloat {
        g.location(in: scroll).x - (held.cell.frame.minX + held.startX)
    }

    @objc private func swiped(_ g: UIPanGestureRecognizer) {
        switch g.state {
        case .began, .changed:
            guard let held = touchPress else { return }
            scroll.isScrollEnabled = false
            // The overlay starts once the core's slider is here; until then the finger just pulls.
            if swipe == nil, let ready = held.prepared {
                guard ready.slider.enabled else { return endSwipe() }
                swipe = SwipeModerator(slider: ready.slider, cell: held.cell, in: scroll, visibleTop: scroll.contentOffset.y,
                                       commit: ready.commit)
                chatLog.notice("[swipe] slider drawn")
                // The gesture is found: the one-time hint goes now, before the chip could meet it.
                onSwipeDone?()
            }
            swipe?.track(pull: pull(g, held), y: g.location(in: scroll).y)
        case .ended:
            let ran = swipe?.release() ?? false
            let outcome = ran ? "ran the armed step" : swipe == nil ? "no slider drawn" : "cancelled"
            chatLog.notice("[swipe] released: \(outcome, privacy: .public)")
            endSwipe()
        default:
            endSwipe()
        }
    }

    private func endSwipe() {
        swipe?.finish()
        swipe = nil
        touchPress = nil
        scroll.isScrollEnabled = true
    }

    #if DEBUG
    /// Swipes the newest moderatable row `pull` points (from 16 points into the row) and holds it
    /// there, for screenshots; `release` lets go (and so runs the release action, which only a
    /// FakeCore session wires to anything).
    func debugSwipe(pull: CGFloat, release: Bool) async {
        if release {
            if swipe?.release() == true { onSwipeDone?() }
            endSwipe()
            return
        }
        if swipe == nil {
            guard let row = rows.last(where: { cells[$0.id] != nil && swipeAllowed?($0.layout) == true }),
                  let cell = cells[row.id], let prepare = prepareSwipe,
                  let ready = await prepare(row.layout, cell.frame.width, 16) else { return }
            touchPress = (row, cell, 16, ready)
            swipe = SwipeModerator(slider: ready.slider, cell: cell, in: scroll, visibleTop: scroll.contentOffset.y,
                                   commit: ready.commit)
        }
        swipe?.track(pull: pull, y: touchPress?.cell.frame.midY ?? 0)
    }
    #endif

    /// Scrolls until the row sits in the middle and flashes it; false when it left the buffer.
    func jump(to id: String) -> Bool {
        guard let row = rows.first(where: { $0.id == id }) else { return false }
        pauseForJump()
        let target = min(max(row.y + row.height / 2 - scroll.bounds.height / 2, -scroll.contentInset.top), bottomOffset)
        programmatic = true
        scroll.setContentOffset(CGPoint(x: 0, y: target), animated: true)
        programmatic = false
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { [weak self] in
            guard let cell = self?.cells[id] else { return }
            let overlay = CALayer()
            overlay.frame = cell.bounds
            overlay.backgroundColor = UIColor(red: 1, green: 140 / 255, blue: 0, alpha: 1).cgColor
            overlay.opacity = 0
            cell.layer.insertSublayer(overlay, at: 0)
            let fade = CABasicAnimation(keyPath: "opacity")
            fade.fromValue = 0.5
            fade.toValue = 0.2
            fade.duration = 2
            overlay.add(fade, forKey: "jump")
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { overlay.removeFromSuperlayer() }
        }
        return true
    }

    @MainActor
    private final class LinkTarget: NSObject {
        weak var list: ChatListView?
        init(_ list: ChatListView) { self.list = list }
        @objc func tick(_ link: CADisplayLink) {
            list?.tick(link)
        }
    }
}

/// The one buzz for a new live mention of the viewer, when the phone setting allows it and the
/// app is in front.
@MainActor
enum MentionBuzz {
    static var enabled: Bool {
        UserDefaults.standard.object(forKey: "sn-mention-haptic") as? Bool ?? true
    }

    static func fire() {
        guard enabled, UIApplication.shared.applicationState == .active else { return }
        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
    }
}

/// Frame pacing of the chat list's display link: frames, dropped frames and frame times, for
/// the performance drive step.
@MainActor
final class FramePacing {
    private(set) var frames = 0
    private(set) var dropped = 0
    private var last: CFTimeInterval = 0
    private var intervals: [Double] = []
    private(set) var expected: Double = 1.0 / 120

    func reset() {
        frames = 0
        dropped = 0
        last = 0
        intervals.removeAll(keepingCapacity: true)
    }

    func frame(_ link: CADisplayLink) {
        let now = link.timestamp
        let expected = link.targetTimestamp - link.timestamp
        if expected > 0 { self.expected = expected }
        defer { last = now }
        guard last > 0 else { return }
        let interval = now - last
        frames += 1
        intervals.append(interval)
        let missed = Int((interval / self.expected).rounded()) - 1
        if missed > 0 { dropped += missed }
    }

    /// p50, p99 and worst frame time in milliseconds.
    func percentiles() -> (p50: Double, p99: Double, worst: Double) {
        guard !intervals.isEmpty else { return (0, 0, 0) }
        let sorted = intervals.sorted()
        func at(_ q: Double) -> Double { sorted[min(sorted.count - 1, Int(Double(sorted.count - 1) * q))] * 1000 }
        return (at(0.5), at(0.99), (sorted.last ?? 0) * 1000)
    }
}
