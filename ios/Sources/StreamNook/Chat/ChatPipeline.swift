import Foundation
import os
import SNKit
import Synchronization
import UIKit

/// Turns a room's socket frames into laid-out rows off the main thread. Frames are decoded,
/// applied to the room's buffer and laid out (with their text bitmaps drawn) on one serial
/// queue; the list picks up the finished batches at its next display frame, so the main thread
/// only places views.
final class ChatPipeline: @unchecked Sendable {
    struct Batch: @unchecked Sendable {
        var appended: [ChatRowLayout] = []
        var prepended: [ChatRowLayout] = []
        var trimmed = 0
        var updated: [ChatRowLayout] = []
        /// Every row laid out again (a new width or style); replaces the list's rows.
        var reset: [ChatRowLayout]?
        var forceLive = false
    }

    private let queue = DispatchQueue(label: "app.streamnook.chat.pipeline", qos: .userInitiated)
    private let ready = Mutex<[Batch]>([])
    private var pending: [ChatSocketFrame] = []
    private var flushScheduled = false
    private var buffer: ChatRoomBuffer
    private var style: ChatStyle
    private var width: CGFloat = 0
    /// The rooms' active tab (`setVisible`); a list showing the room makes it active (`replay`).
    private var active = true
    /// No list can be seen (the app is away, or no chat is on screen): rows stay text in the buffer.
    private var suspended = false
    /// Rows changed while suspended: seen again, the list is handed every row.
    private var missed = false
    private var visible: Bool { active && !suspended }
    /// A main-thread wake is already on its way for the batches queued.
    private let wakePosted = Mutex(false)
    /// Posted on the main thread, object the pipeline, when a batch is ready.
    static let readyNotification = Notification.Name("app.streamnook.chat.ready")
    /// Batches nobody took past this collapse into one reset (a dormant list, the app away).
    private static let readyCap = 120
    private var scale: CGFloat
    private var layouts: [String: ChatRowLayout] = [:]
    private let onModes: @Sendable (ChatRoomModes) -> Void
    private let onRowCount: @Sendable (Int) -> Void
    /// The viewer's own chat color for this room, each time Twitch's USERSTATE reports it.
    private let onOwnColor: @Sendable (String) -> Void
    /// Rows a chatter's late 7TV personal set names (their author's id, the rows held), to have
    /// the set applied by the core.
    private let onPersonalEmotes: @Sendable (String, [ChatMessage]) -> Void

    /// Rows decoded so far, for the performance log.
    private let counters = Mutex<(frames: Int, rows: Int, layoutNanos: UInt64)>((0, 0, 0))

    @MainActor weak var list: ChatListView?
    /// Paint images asked for (an image paint's art), each once: a row's bitmap draws one only
    /// when it is loaded, so the rows wearing it are drawn again when it lands.
    private var paintImages: Set<String> = []
    private var failureObserver: NSObjectProtocol?
    private var themeObserver: NSObjectProtocol?
    private var atmosphereObserver: NSObjectProtocol?

    init(style: ChatStyle, scale: CGFloat, onModes: @escaping @Sendable (ChatRoomModes) -> Void,
         onRowCount: @escaping @Sendable (Int) -> Void = { _ in },
         onOwnColor: @escaping @Sendable (String) -> Void = { _ in },
         onPersonalEmotes: @escaping @Sendable (String, [ChatMessage]) -> Void = { _, _ in }) {
        self.style = style
        self.scale = scale
        self.onModes = onModes
        self.onRowCount = onRowCount
        self.onOwnColor = onOwnColor
        self.onPersonalEmotes = onPersonalEmotes
        buffer = ChatRoomBuffer(cap: style.bufferCap)
        buffer.setHiddenEvents(style.hiddenEvents)
        buffer.ignoreClearChat = style.ignoreClearChat
        buffer.showModMessages = style.showModMessages
        failureObserver = NotificationCenter.default.addObserver(forName: ImagePipeline.failedNotification,
                                                                 object: nil, queue: nil) { [weak self] note in
            guard let url = note.userInfo?["url"] as? String else { return }
            self?.dropBadge(url)
        }
        // A new theme redoes every row in the new colours.
        themeObserver = NotificationCenter.default.addObserver(forName: ChatTheme.changed, object: nil, queue: nil) { [weak self] _ in
            self?.themeChanged()
        }
        // The Atmosphere catalog arriving (or changing) puts the washes on rows laid out without them.
        atmosphereObserver = NotificationCenter.default.addObserver(forName: AtmosphereCatalog.changed, object: nil,
                                                                    queue: nil) { [weak self] _ in
            self?.themeChanged()
        }
    }

    deinit {
        if let failureObserver { NotificationCenter.default.removeObserver(failureObserver) }
        if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
        if let atmosphereObserver { NotificationCenter.default.removeObserver(atmosphereObserver) }
    }

    private func themeChanged() {
        queue.async { [self] in relayoutAll() }
    }

    // MARK: Input

    /// A frame from the socket, from any thread.
    func ingest(_ raw: String) {
        ingest(frame: ChatSocketFrame.decode(raw))
    }

    /// A frame the rooms' shared reader already decoded and routed here, from any thread.
    func ingest(frame: ChatSocketFrame) {
        switch frame {
        case .ownColor(_, let color):
            onOwnColor(color)
            return
        case .personalEmotes(_, let userID, let ids):
            queue.async { [self] in
                // Rows still waiting for this batch are held first, so a row named here is found.
                flush()
                let wanted = Set(ids)
                let named = buffer.rows.map(\.message).filter {
                    wanted.contains($0.id) && $0.userID == userID && !$0.segments.isEmpty
                }
                if !userID.isEmpty, !named.isEmpty { onPersonalEmotes(userID, named) }
            }
            return
        case .pinned:
            // The open rooms keep a platform's pins beside the rows (ChatRooms).
            return
        default:
            break
        }
        queue.async { [self] in
            pending.append(frame)
            counters.withLock { $0.frames += 1 }
            scheduleFlush()
        }
    }

    /// Rows whose segments the core worked again (a late personal set applied), swapped in place
    /// as a re-stamp is.
    func applySegments(_ byID: [String: [ChatSegment]]) {
        guard !byID.isEmpty else { return }
        ingest(frame: .patch(channel: nil, rows: byID.map { ChatRowPatch(id: $0.key, segments: $0.value) }))
    }

    /// History rows that go above everything live (the join backfill).
    func prepend(_ messages: [ChatMessage]) {
        queue.async { [self] in
            let fresh = messages.filter { buffer.row($0.id) == nil }
            guard !fresh.isEmpty else { return }
            // History lands at the top in time order; the buffer holds rows in arrival order.
            let ids = Set(fresh.map(\.id))
            let rows = fresh + buffer.rows.map(\.message)
            buffer.removeAll()
            _ = buffer.apply(rows.map { .row($0) })
            // The cap trims the oldest history first; only what survived goes on the list.
            let kept = buffer.rows.prefix { ids.contains($0.id) }
            guard width > 0, !kept.isEmpty else { return }
            guard visible else {
                missed = true
                return
            }
            let built = kept.map { layout($0, bitmap: false) }
            publish(Batch(prepended: built))
        }
    }

    /// Rows missed while the connection was down, put in time order among the rows held.
    func backfill(_ messages: [ChatMessage]) {
        queue.async { [self] in
            guard buffer.insertInTimeOrder(messages) else { return }
            relayoutAll()
        }
    }

    func setPaused(_ paused: Bool) {
        queue.async { [self] in buffer.paused = paused }
    }

    /// A VOD replay step from the core: a reset replaces every row (a seek, the first rows), else
    /// the rows join the bottom like live ones.
    func applyReplay(_ step: ReplayStep) {
        guard step.reset else {
            for message in step.rows { ingest(frame: .row(message)) }
            return
        }
        queue.async { [self] in
            pending.removeAll()
            buffer.removeAll()
            layouts.removeAll()
            _ = buffer.apply(step.rows.map { .row($0) })
            relayoutAll(forceLive: true)
        }
    }

    /// `/clearmessages`: the rows shown now go, for this viewer only; new rows still arrive.
    func clearShown() {
        queue.async { [self] in
            pending.removeAll()
            buffer.removeAll()
            layouts.removeAll()
            relayoutAll()
        }
    }

    /// A room behind another tab keeps its rows but lays none out until it is on screen again.
    func setVisible(_ visible: Bool) {
        queue.async { [self] in active = visible }
    }

    /// No chat list can be seen: rows keep arriving into the buffer as text and none is laid out
    /// or drawn. Seen again, a room that missed rows hands its list every row at once.
    func setSuspended(_ suspended: Bool) {
        queue.async { [self] in
            guard suspended != self.suspended else { return }
            self.suspended = suspended
            if !suspended, missed, active { replayNow() }
        }
    }

    /// Every row the room holds, as one batch, for a list that just started showing it: live,
    /// at the bottom, never a pause left from before. Rows laid out earlier are reused.
    func replay() {
        queue.async { [self] in
            active = true
            replayNow()
        }
    }

    /// `replay`'s work, on the queue.
    private func replayNow() {
        missed = false
        buffer.paused = false
        guard width > 0 else { return }
        let rows = buffer.rows
        let drawFrom = max(0, rows.count - 40)
        let all = rows.enumerated().map { i, row -> ChatRowLayout in
            guard let cached = layouts[row.id], cached.revision == row.revision else {
                return layout(row, bitmap: i >= drawFrom)
            }
            if i >= drawFrom, !cached.hasBitmap { cached.renderBitmap(scale: scale) }
            return cached
        }
        publish(Batch(reset: all, forceLive: true))
    }

    func setWidth(_ width: CGFloat, scale: CGFloat) {
        queue.async { [self] in
            guard abs(width - self.width) > 0.5 || scale != self.scale else { return }
            self.width = width
            self.scale = scale
            relayoutAll()
        }
    }

    func setStyle(_ style: ChatStyle) {
        queue.async { [self] in
            guard style != self.style else { return }
            self.style = style
            buffer.cap = style.bufferCap
            buffer.setHiddenEvents(style.hiddenEvents)
            buffer.ignoreClearChat = style.ignoreClearChat
            buffer.showModMessages = style.showModMessages
            relayoutAll()
        }
    }

    /// The batches finished since the last call (main thread, every display frame).
    func takeReady() -> [Batch] {
        ready.withLock { batches in
            defer { batches.removeAll(keepingCapacity: true) }
            return batches
        }
    }

    /// A row's message, for taps and replies (rare, so a short wait on the queue is fine).
    func message(_ id: String) -> ChatMessage? {
        queue.sync { buffer.row(id)?.message }
    }

    /// The newest rows' messages, for composer suggestions (chatters to mention).
    func recentMessages(_ count: Int) -> [ChatMessage] {
        queue.sync { buffer.rows.suffix(count).map(\.message) }
    }

    func stats() -> (frames: Int, rows: Int, layoutMS: Double) {
        counters.withLock { ($0.frames, $0.rows, Double($0.layoutNanos) / 1_000_000) }
    }

    // MARK: Work on the queue

    private func scheduleFlush() {
        guard !flushScheduled else { return }
        flushScheduled = true
        // About one display frame of frames per batch.
        queue.asyncAfter(deadline: .now() + .milliseconds(8)) { [self] in
            flushScheduled = false
            flush()
        }
    }

    private func flush() {
        guard !pending.isEmpty else { return }
        let frames = pending
        pending.removeAll(keepingCapacity: true)
        let change = buffer.apply(frames)
        if let modes = buffer.modes { onModes(modes) }
        onRowCount(buffer.rows.count)
        guard width > 0, visible, !change.isEmpty else {
            if suspended, !change.isEmpty { missed = true }
            for (id, _) in layouts where buffer.row(id) == nil { layouts[id] = nil }
            return
        }
        if change.reset {
            // Rows went from among the others (hidden users): the list takes every row again.
            for (id, _) in layouts where buffer.row(id) == nil { layouts[id] = nil }
            relayoutAll()
            return
        }
        var batch = Batch(trimmed: change.trimmed)
        // Rows pushed straight off by a burst never need a bitmap.
        let drawFrom = max(0, change.appended.count - 40)
        for (i, row) in change.appended.enumerated() {
            batch.appended.append(layout(row, bitmap: i >= drawFrom))
        }
        for row in change.updated {
            batch.updated.append(layout(row, bitmap: true))
        }
        for (id, _) in layouts where buffer.row(id) == nil { layouts[id] = nil }
        publish(batch)
    }

    private func layout(_ row: ChatRowModel, bitmap: Bool) -> ChatRowLayout {
        let start = DispatchTime.now().uptimeNanoseconds
        loadPaintImages(row)
        let built = ChatRowLayoutBuilder(style: style, width: width, scale: scale,
                                         mentionPaints: buffer.chatterPaints).build(row)
        if bitmap { built.renderBitmap(scale: scale) }
        layouts[row.id] = built
        let spent = DispatchTime.now().uptimeNanoseconds - start
        counters.withLock {
            $0.rows += 1
            $0.layoutNanos += spent
        }
        return built
    }

    /// A badge image that could not load: the rows wearing it are laid out again without it, so
    /// it leaves no empty slot. Rows of a room not on screen just forget their layout.
    private func dropBadge(_ url: String) {
        queue.async { [self] in
            let wearing = buffer.rows.filter { $0.message.metadata.view?.badges.contains { $0.url == url } == true }
            guard !wearing.isEmpty else { return }
            guard width > 0, visible else {
                for row in wearing { layouts[row.id] = nil }
                return
            }
            let batch = Batch(updated: wearing.map { layout($0, bitmap: true) })
            publish(batch)
        }
    }

    /// Starts loading the art of the author's image paint when nothing has it yet. Without it the
    /// name draws in its base color (the account's Twitch color) until another screen happens
    /// to load the same image.
    private func loadPaintImages(_ row: ChatRowModel) {
        guard let paint = row.message.metadata.view?.paint else { return }
        for url in PaintPainter.imageURLs(paint) where !paintImages.contains(url) {
            guard ImagePipeline.shared.cachedAnySize(url) == nil, !ImagePipeline.shared.hasFailed(url) else { continue }
            paintImages.insert(url)
            // About the name's height on screen; the painter scales whatever size is loaded.
            let pixels = Int((CGFloat(style.fontSize) * 2 * scale).rounded(.up))
            // Asked again after it lands, so art the image cache later drops is loaded again.
            ImagePipeline.shared.load(url, pixelHeight: pixels, animated: false) { [weak self] image in
                guard let self else { return }
                queue.async { self.paintImages.remove(url) }
                if image != nil { redrawPaint(url) }
            }
        }
    }

    /// A paint image that just loaded: the rows wearing it are drawn again with it.
    private func redrawPaint(_ url: String) {
        queue.async { [self] in
            let wearing = buffer.rows.filter { row in
                row.message.metadata.view?.paint.map { PaintPainter.imageURLs($0).contains(url) } == true
            }
            guard !wearing.isEmpty else { return }
            guard width > 0, visible else {
                for row in wearing { layouts[row.id] = nil }
                return
            }
            let batch = Batch(updated: wearing.map { layout($0, bitmap: true) })
            publish(batch)
        }
    }

    private func relayoutAll(forceLive: Bool = false) {
        guard width > 0 else { return }
        // A room nobody sees forgets its layouts; the list that next shows it lays them out (`replay`).
        guard visible else {
            layouts.removeAll()
            missed = true
            return
        }
        let rows = buffer.rows
        let drawFrom = max(0, rows.count - 40)
        let all = rows.enumerated().map { layout($0.element, bitmap: $0.offset >= drawFrom) }
        publish(Batch(reset: all, forceLive: forceLive))
    }

    /// Queues a batch and wakes the lists on this room (one main-thread hop per burst). A reset
    /// carries every row, so it replaces whatever still waits (keeping a waiting force-live);
    /// past `readyCap` untaken batches collapse into one reset.
    private func publish(_ batch: Batch) {
        let overflow = ready.withLock { batches -> Bool in
            var batch = batch
            if batch.reset != nil {
                if batches.contains(where: \.forceLive) { batch.forceLive = true }
                batches.removeAll(keepingCapacity: true)
            }
            batches.append(batch)
            return batches.count > Self.readyCap
        }
        if overflow {
            ready.withLock { $0.removeAll() }
            replayNow()
            return
        }
        let post = wakePosted.withLock { posted -> Bool in
            defer { posted = true }
            return !posted
        }
        guard post else { return }
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            wakePosted.withLock { $0 = false }
            NotificationCenter.default.post(name: Self.readyNotification, object: self)
        }
    }
}
