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
    private var visible = true
    private var scale: CGFloat
    private var layouts: [String: ChatRowLayout] = [:]
    private let onModes: @Sendable (ChatRoomModes) -> Void
    private let onRowCount: @Sendable (Int) -> Void

    /// Rows decoded so far, for the performance log.
    private let counters = Mutex<(frames: Int, rows: Int, layoutNanos: UInt64)>((0, 0, 0))

    @MainActor weak var list: ChatListView?
    private var failureObserver: NSObjectProtocol?
    private var themeObserver: NSObjectProtocol?

    init(style: ChatStyle, scale: CGFloat, onModes: @escaping @Sendable (ChatRoomModes) -> Void,
         onRowCount: @escaping @Sendable (Int) -> Void = { _ in }) {
        self.style = style
        self.scale = scale
        self.onModes = onModes
        self.onRowCount = onRowCount
        buffer = ChatRoomBuffer(cap: style.bufferCap)
        buffer.setHiddenEvents(style.hiddenEvents)
        failureObserver = NotificationCenter.default.addObserver(forName: ImagePipeline.failedNotification,
                                                                 object: nil, queue: nil) { [weak self] note in
            guard let url = note.userInfo?["url"] as? String else { return }
            self?.dropBadge(url)
        }
        // A new theme redoes every row in the new colours.
        themeObserver = NotificationCenter.default.addObserver(forName: ChatTheme.changed, object: nil, queue: nil) { [weak self] _ in
            self?.themeChanged()
        }
    }

    deinit {
        if let failureObserver { NotificationCenter.default.removeObserver(failureObserver) }
        if let themeObserver { NotificationCenter.default.removeObserver(themeObserver) }
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
        queue.async { [self] in
            pending.append(frame)
            counters.withLock { $0.frames += 1 }
            scheduleFlush()
        }
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
            let built = kept.map { layout($0, bitmap: false) }
            ready.withLock { $0.append(Batch(prepended: built)) }
        }
    }

    /// Rows missed while the connection was down, put in time order among the rows held.
    func backfill(_ messages: [ChatMessage]) {
        queue.async { [self] in
            guard buffer.insertInTimeOrder(messages), visible else { return }
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
            relayoutAll()
            if width > 0 { ready.withLock { $0[$0.count - 1].forceLive = true } }
        }
    }

    /// A room behind another tab keeps its rows but lays none out until it is on screen again.
    func setVisible(_ visible: Bool) {
        queue.async { [self] in self.visible = visible }
    }

    /// Every row the room holds, as one batch, for a list that just started showing it: live,
    /// at the bottom, never a pause left from before. Rows laid out earlier are reused.
    func replay() {
        queue.async { [self] in
            visible = true
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
            ready.withLock { $0.append(Batch(reset: all, forceLive: true)) }
        }
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
            for (id, _) in layouts where buffer.row(id) == nil { layouts[id] = nil }
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
        ready.withLock { $0.append(batch) }
    }

    private func layout(_ row: ChatRowModel, bitmap: Bool) -> ChatRowLayout {
        let start = DispatchTime.now().uptimeNanoseconds
        let built = ChatRowLayoutBuilder(style: style, width: width, scale: scale).build(row)
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
            ready.withLock { $0.append(batch) }
        }
    }

    private func relayoutAll() {
        guard width > 0 else { return }
        let rows = buffer.rows
        let drawFrom = max(0, rows.count - 40)
        let all = rows.enumerated().map { layout($0.element, bitmap: $0.offset >= drawFrom) }
        ready.withLock { $0.append(Batch(reset: all)) }
    }
}
