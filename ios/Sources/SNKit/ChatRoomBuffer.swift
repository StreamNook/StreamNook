import Foundation

/// How a moderator touched a row.
public enum ChatModeration: Sendable, Equatable, Hashable {
    case deleted
    case timedOut(seconds: Int)
    case banned
}

/// One row as a room holds it: the core's message, a revision that grows whenever what the row
/// draws changes, and the moderation mark the room applied.
public struct ChatRowModel: Sendable, Equatable, Identifiable {
    public var message: ChatMessage
    public var revision: Int
    public var moderation: ChatModeration?
    /// Arrived live (not from history), so it may animate in and buzz for a mention.
    public var live: Bool
    /// A message AutoMod held, shown to a moderator with Allow and Deny; folds to one line once
    /// decided.
    public var held: HeldRow?

    public var id: String { message.id }

    public init(message: ChatMessage, revision: Int = 0, moderation: ChatModeration? = nil, live: Bool = true,
                held: HeldRow? = nil) {
        self.message = message
        self.revision = revision
        self.moderation = moderation
        self.live = live
        self.held = held
    }
}

/// What one batch did to a room, for the list to apply.
public struct ChatBufferChange: Sendable, Equatable {
    /// Rows added at the bottom, oldest first.
    public var appended: [ChatRowModel] = []
    /// Rows dropped from the top.
    public var trimmed = 0
    /// Rows whose drawing changed, by id.
    public var updated: [ChatRowModel] = []
    /// Rows were taken out from among the others (hidden users): the list takes every row the
    /// room holds again instead of the changes above.
    public var reset = false

    public init(appended: [ChatRowModel] = [], trimmed: Int = 0, updated: [ChatRowModel] = [], reset: Bool = false) {
        self.appended = appended
        self.trimmed = trimmed
        self.updated = updated
        self.reset = reset
    }

    public var isEmpty: Bool { appended.isEmpty && trimmed == 0 && updated.isEmpty && !reset }
}

/// A room's rows: applies the core's frames and keeps the newest `cap` rows, plus up to 150 more
/// while the reader is paused so the scrollback being read is not cut, decaying 5 rows per
/// batch after resume.
public struct ChatRoomBuffer: Sendable {
    public private(set) var rows: [ChatRowModel] = []
    public var cap: Int
    public var paused = false {
        didSet { if paused { pausedOverflowAllowance = Self.pausedOverflow } }
    }
    public private(set) var modes: ChatRoomModes?
    /// Chat event kinds turned off (`provider:category`): a row the core keyed with one is never
    /// held, and setting this drops the held ones.
    public private(set) var hiddenEvents: Set<String> = []
    /// A moderator's delete, timeout, ban or clear marks no row (moderation.ignore_clear_chat).
    public var ignoreClearChat = false
    /// Each delete, timeout, ban and clear posts a line in the room (moderation.show_mod_messages).
    public var showModMessages = false
    /// The 7TV paint each chatter seen here wears, by lowercased login, so an @mention of them
    /// can wear it too (the shared store's chatter list). Learned from their rows and kept after
    /// the rows scroll away.
    public private(set) var chatterPaints: [String: ChatPaint] = [:]

    static let pausedOverflow = 150
    /// Chatters whose paint is remembered before the memory starts over.
    static let paintMemory = 1000
    static let decayPerBatch = 5
    private var pausedOverflowAllowance = 0
    private var index: [String: Int] = [:]
    private var base = 0

    public init(cap: Int = 100) {
        self.cap = cap
    }

    /// Whether a row is a chat event kind in `hidden`.
    public static func hides(_ message: ChatMessage, _ hidden: Set<String>) -> Bool {
        guard !hidden.isEmpty, let event = message.metadata.view?.event else { return false }
        return hidden.contains(event)
    }

    /// Turns event kinds off or back on. Rows already held of a kind now off are dropped; answers
    /// whether any were, so the list can be laid out again. A kind turned back on shows from its
    /// next row.
    @discardableResult
    public mutating func setHiddenEvents(_ hidden: Set<String>) -> Bool {
        guard hidden != hiddenEvents else { return false }
        hiddenEvents = hidden
        guard rows.contains(where: { Self.hides($0.message, hidden) }) else { return false }
        rows.removeAll { Self.hides($0.message, hidden) }
        base = 0
        index = Dictionary(uniqueKeysWithValues: rows.enumerated().map { ($1.id, $0) })
        return true
    }

    public func row(_ id: String) -> ChatRowModel? {
        guard let i = index[id] else { return nil }
        return rows[i - base]
    }

    /// Applies a batch of frames for this room and trims.
    /// Whether a `ROW_MEMBER` frame's id names the author of `message`.
    static func member(_ id: String, wrote message: ChatMessage) -> Bool {
        let twitch = message.provider.isEmpty || message.provider == "twitch"
        return twitch ? message.userID == id : "\(message.provider):\(message.userID)" == id
    }

    /// Puts a late 7TV badge where the core's badge order has it (`surface_rank`): after the
    /// platform's own badges, before the chat apps' and the member's StreamNook badge. It
    /// replaces the 7TV badge the row wore, so a changed badge is never drawn twice.
    static func wear(sevenTv badge: ChatViewBadge, in badges: inout [ChatViewBadge]) {
        badges.removeAll { $0.provider == "7tv" }
        let platform: Set<String> = ["", "twitch", "kick", "youtube", "tiktok"]
        let at = badges.firstIndex { !platform.contains($0.provider) } ?? badges.endIndex
        badges.insert(badge, at: at)
    }

    public mutating func apply(_ frames: [ChatSocketFrame]) -> ChatBufferChange {
        var change = ChatBufferChange()
        var updated: [String: ChatRowModel] = [:]
        for frame in frames {
            switch frame {
            case .row(let message):
                notePaint(message)
                if let i = index[message.id] {
                    var row = rows[i - base]
                    row.message = message
                    row.revision += 1
                    rows[i - base] = row
                    updated[row.id] = row
                } else if !Self.hides(message, hiddenEvents) {
                    let row = ChatRowModel(message: message, live: !message.metadata.fromBackfill)
                    index[message.id] = base + rows.count
                    rows.append(row)
                    change.appended.append(row)
                }
            case .clearMessage(let channel, let targetID, let login, let reason):
                if !ignoreClearChat { mark(id: targetID, .deleted, &updated, &change) }
                if showModMessages, let channel {
                    // The frame may not name the author (Kick's delete gives only the id): the
                    // row is still held, a delete only marks it.
                    let author = row(targetID).map { $0.message.displayName.isEmpty ? $0.message.username : $0.message.displayName }
                    let who = login ?? author.flatMap { $0.isEmpty ? nil : $0 } ?? "a user"
                    let line = reason.map { "\(who)'s message was deleted (\($0))." }
                        ?? "\(who)'s message was deleted by a moderator."
                    appendSystem(line, channel: channel, &change)
                }
            case .clearChat(let channel, let targetUserID, let targetUser, let duration):
                let userID = targetUserID.flatMap { $0.isEmpty ? nil : $0 }
                // A full clear marks nothing.
                if let userID, !ignoreClearChat {
                    let mark: ChatModeration = duration.map { .timedOut(seconds: $0) } ?? .banned
                    for (offset, row) in rows.enumerated() where row.message.userID == userID && row.moderation != mark {
                        self.mark(id: rows[offset].id, mark, &updated, &change)
                    }
                }
                if showModMessages, let channel {
                    let line: String
                    if userID != nil {
                        let who = targetUser ?? "A user"
                        line = duration.map { "\(who) was timed out for \(Self.shortDuration($0))." } ?? "\(who) was banned."
                    } else {
                        line = "Chat was cleared by a moderator."
                    }
                    appendSystem(line, channel: channel, &change)
                }
            case .hideUsers(_, let logins):
                let names = Set(logins.map { $0.lowercased() }.filter { !$0.isEmpty })
                guard !names.isEmpty else { continue }
                let hidden = { (row: ChatRowModel) -> Bool in
                    names.contains(row.message.username.lowercased()) || names.contains(row.message.displayName.lowercased())
                }
                guard rows.contains(where: hidden) else { continue }
                rows.removeAll(where: hidden)
                change.appended.removeAll(where: hidden)
                base = 0
                index = Dictionary(uniqueKeysWithValues: rows.enumerated().map { ($1.id, $0) })
                change.reset = true
            case .cosmetics(_, let userID, let paint, let badge):
                for (offset, row) in rows.enumerated() where Self.member(userID, wrote: row.message) {
                    guard var view = row.message.metadata.view else { continue }
                    let before = view
                    view.paint = paint ?? view.paint
                    if let badge { Self.wear(sevenTv: badge, in: &view.badges) }
                    guard view != before else { continue }
                    var next = row
                    next.message.metadata.view = view
                    next.revision += 1
                    rows[offset] = next
                    notePaint(next.message)
                    replaceAppended(next, &change)
                    updated[next.id] = next
                }
            case .member(let userID, let atmosphere, let cologne, let badge):
                // A member is named by their Twitch id on Twitch rows, and by the claimed account
                // (`kick:<id>`) on another platform's; a Kick chatter with the same bare number
                // as a member is someone else.
                for (offset, row) in rows.enumerated() where Self.member(userID, wrote: row.message) {
                    guard var view = row.message.metadata.view else { continue }
                    let before = view
                    view.atmosphere = atmosphere
                    view.cologne = cologne
                    view.badges.removeAll { $0.provider == "streamnook" }
                    if let badge { view.badges.append(badge) }
                    guard view != before else { continue }
                    var next = row
                    next.message.metadata.view = view
                    next.revision += 1
                    rows[offset] = next
                    replaceAppended(next, &change)
                    updated[next.id] = next
                }
            case .preview(let url, let preview):
                for (offset, row) in rows.enumerated() {
                    guard var view = row.message.metadata.view,
                          let at = view.previews.firstIndex(where: { $0.url == url }) else { continue }
                    let before = view
                    if let preview {
                        view.previews[at].preview = preview
                    } else {
                        view.previews.remove(at: at)
                    }
                    guard view != before else { continue }
                    var next = row
                    next.message.metadata.view = view
                    next.revision += 1
                    rows[offset] = next
                    replaceAppended(next, &change)
                    updated[next.id] = next
                }
            case .patch(_, let patches):
                for patch in patches {
                    guard let i = index[patch.id] else { continue }
                    var row = rows[i - base]
                    if !patch.segments.isEmpty { row.message.segments = patch.segments }
                    if let view = patch.view { row.message.metadata.view = view }
                    notePaint(row.message)
                    row.revision += 1
                    rows[i - base] = row
                    replaceAppended(row, &change)
                    updated[row.id] = row
                }
            case .roomState(_, let modes):
                if let modes { self.modes = modes }
            case .held(_, let held):
                let id = Self.heldRowID(held.held.messageID)
                if let i = index[id] {
                    var row = rows[i - base]
                    guard row.held != held else { continue }
                    row.held = held
                    row.revision += 1
                    rows[i - base] = row
                    replaceAppended(row, &change)
                    updated[id] = row
                } else if held.held.isPending, let message = Self.heldMessage(held.held) {
                    // A decision on a message held before this room opened has nothing to fold.
                    let row = ChatRowModel(message: message, live: true, held: held)
                    index[id] = base + rows.count
                    rows.append(row)
                    change.appended.append(row)
                }
            default:
                break
            }
        }
        change.trimmed = trim()
        let appendedIDs = Set(change.appended.map(\.id))
        // The list appends first and then drops `trimmed` rows from the top, so a batch larger
        // than the cap keeps its appended rows listed and lets the trim take them.
        change.updated = updated.values.filter { index[$0.id] != nil && !appendedIDs.contains($0.id) }
        return change
    }

    /// The row id a held message lives under, apart from the real message Twitch delivers if it
    /// is allowed.
    public static func heldRowID(_ messageID: String) -> String { "held-\(messageID)" }

    /// The row a held message is drawn from: its author and text, as the room shows a message.
    static func heldMessage(_ held: HeldMessage) -> ChatMessage? {
        let sent = held.heldAtMs > 0 ? Date(timeIntervalSince1970: Double(held.heldAtMs) / 1000) : Date()
        let row: [String: Any] = [
            "id": heldRowID(held.messageID),
            "user_id": held.userID,
            "username": held.userLogin,
            "display_name": held.userName,
            "color": NSNull(),
            "badges": [Any](),
            "timestamp": ISO8601DateFormatter().string(from: sent),
            "content": held.text,
            "provider": "twitch",
            "channel": held.channel,
            "tags": [String: String](),
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: row) else { return nil }
        return try? JSONDecoder().decode(ChatMessage.self, from: data)
    }

    /// A moderation line only the viewer sees, at the bottom of the room.
    private mutating func appendSystem(_ line: String, channel: String, _ change: inout ChatBufferChange) {
        guard let message = ChatSocketFrame.systemRow(line, channel: channel) else { return }
        let row = ChatRowModel(message: message, live: true)
        index[message.id] = base + rows.count
        rows.append(row)
        change.appended.append(row)
    }

    /// A timeout's length as the moderation line words it: whole days, hours, minutes or seconds.
    static func shortDuration(_ seconds: Int) -> String {
        let s = Double(seconds)
        if seconds >= 86400 { return "\(Int((s / 86400).rounded()))d" }
        if seconds >= 3600 { return "\(Int((s / 3600).rounded()))h" }
        if seconds >= 60 { return "\(Int((s / 60).rounded()))m" }
        return "\(seconds)s"
    }

    /// Remembers the paint a chatter's row wears, for mentions of them.
    private mutating func notePaint(_ message: ChatMessage) {
        guard let paint = message.metadata.view?.paint, !message.username.isEmpty else { return }
        let login = message.username.lowercased()
        guard chatterPaints[login] != paint else { return }
        if chatterPaints.count >= Self.paintMemory { chatterPaints.removeAll() }
        chatterPaints[login] = paint
    }

    /// Rows a moderation mark touches are drawn again; the row stays unless the style hides it.
    private mutating func mark(id: String, _ mark: ChatModeration, _ updated: inout [String: ChatRowModel],
                               _ change: inout ChatBufferChange) {
        guard let i = index[id] else { return }
        var row = rows[i - base]
        guard row.moderation != mark else { return }
        row.moderation = mark
        row.revision += 1
        rows[i - base] = row
        replaceAppended(row, &change)
        updated[id] = row
    }

    /// A row appended earlier in the same batch carries its newest state.
    private func replaceAppended(_ row: ChatRowModel, _ change: inout ChatBufferChange) {
        if let at = change.appended.firstIndex(where: { $0.id == row.id }) { change.appended[at] = row }
    }

    private mutating func trim() -> Int {
        let limit: Int
        if paused {
            limit = cap + Self.pausedOverflow
        } else {
            pausedOverflowAllowance = max(0, pausedOverflowAllowance - Self.decayPerBatch)
            limit = cap + pausedOverflowAllowance
        }
        let excess = rows.count - limit
        guard excess > 0 else { return 0 }
        for row in rows.prefix(excess) { index[row.id] = nil }
        rows.removeFirst(excess)
        base += excess
        return excess
    }

    /// Rows missed while the connection was down (an outage backfill), placed in time order among
    /// the rows held. Rows already held stay as they are, moderation marks included. A row with
    /// no send time keeps the place of the row before it. Answers whether anything was added.
    public mutating func insertInTimeOrder(_ messages: [ChatMessage]) -> Bool {
        var seen = Set<String>()
        let fresh = messages.filter { index[$0.id] == nil && !Self.hides($0, hiddenEvents) && seen.insert($0.id).inserted }
        guard !fresh.isEmpty else { return false }
        for message in fresh { notePaint(message) }
        var last: Int64 = 0
        var keyed: [(at: Int64, order: Int, row: ChatRowModel)] = []
        for (i, row) in (rows + fresh.map { ChatRowModel(message: $0, live: false) }).enumerated() {
            let at = Self.sentAt(row.message) ?? last
            last = at
            keyed.append((at, i, row))
        }
        keyed.sort { $0.at == $1.at ? $0.order < $1.order : $0.at < $1.at }
        rows = keyed.map(\.row)
        base = 0
        index = Dictionary(uniqueKeysWithValues: rows.enumerated().map { ($1.id, $0) })
        _ = trim()
        return true
    }

    /// When Twitch says the message was sent, in milliseconds.
    static func sentAt(_ message: ChatMessage) -> Int64? {
        message.tags["tmi-sent-ts"].flatMap { Int64($0) }
    }

    /// Rows older than these were dropped; the room starts empty again.
    public mutating func removeAll() {
        rows.removeAll()
        index.removeAll()
        base = 0
    }
}
