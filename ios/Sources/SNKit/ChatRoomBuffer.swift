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

    public init(appended: [ChatRowModel] = [], trimmed: Int = 0, updated: [ChatRowModel] = []) {
        self.appended = appended
        self.trimmed = trimmed
        self.updated = updated
    }

    public var isEmpty: Bool { appended.isEmpty && trimmed == 0 && updated.isEmpty }
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

    static let pausedOverflow = 150
    static let decayPerBatch = 5
    private var pausedOverflowAllowance = 0
    private var index: [String: Int] = [:]
    private var base = 0

    public init(cap: Int = 100) {
        self.cap = cap
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

    public mutating func apply(_ frames: [ChatSocketFrame]) -> ChatBufferChange {
        var change = ChatBufferChange()
        var updated: [String: ChatRowModel] = [:]
        for frame in frames {
            switch frame {
            case .row(let message):
                if let i = index[message.id] {
                    var row = rows[i - base]
                    row.message = message
                    row.revision += 1
                    rows[i - base] = row
                    updated[row.id] = row
                } else {
                    let row = ChatRowModel(message: message, live: !message.metadata.fromBackfill)
                    index[message.id] = base + rows.count
                    rows.append(row)
                    change.appended.append(row)
                }
            case .clearMessage(_, let targetID):
                mark(id: targetID, .deleted, &updated, &change)
            case .clearChat(_, let userID, _, let duration):
                guard let userID else { continue }
                let mark: ChatModeration = duration.map { .timedOut(seconds: $0) } ?? .banned
                for (offset, row) in rows.enumerated() where row.message.userID == userID && row.moderation != mark {
                    self.mark(id: rows[offset].id, mark, &updated, &change)
                }
            case .cosmetics(_, let userID, let paint, let badge):
                for (offset, row) in rows.enumerated() where row.message.userID == userID {
                    guard var view = row.message.metadata.view else { continue }
                    let before = view
                    view.paint = paint ?? view.paint
                    if let badge, !view.badges.contains(badge) { view.badges.append(badge) }
                    guard view != before else { continue }
                    var next = row
                    next.message.metadata.view = view
                    next.revision += 1
                    rows[offset] = next
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
            case .patch(_, let patches):
                for patch in patches {
                    guard let i = index[patch.id] else { continue }
                    var row = rows[i - base]
                    if !patch.segments.isEmpty { row.message.segments = patch.segments }
                    if let view = patch.view { row.message.metadata.view = view }
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
        let fresh = messages.filter { index[$0.id] == nil && seen.insert($0.id).inserted }
        guard !fresh.isEmpty else { return false }
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
