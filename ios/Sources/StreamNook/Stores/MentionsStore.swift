import Foundation
import Observation
import SNKit

/// The mentions inbox as the app shows it: the pages the core answers (`get_mention_inbox`),
/// the unread count its `mention-inbox-changed` push carries, and the inbox commands. The core
/// files, matches, dedupes and decides what to announce; this only presents and asks.
@Observable @MainActor
final class MentionsStore {
    private(set) var list = MentionList()
    private(set) var loadingMore = false
    /// The first page failed to load.
    private(set) var failed = false
    /// "Clear all" is asking for its confirmation.
    var confirmingClear = false
    /// Bumped on every change the core announces (and when senders' pictures land), so an open
    /// inbox reads its view again.
    private(set) var revision = 0
    /// The chat whose inbox sheet is open (the tray's tap), if any.
    var sheetIn: InboxSurface?
    /// The latest arrival waiting for, or showing as, the card over the message bar.
    private(set) var arrival: ArrivalCard?
    /// The composer's own risers (the suggestion strip, the emote picker) hold the room the card
    /// would rise into: it waits while one is up.
    var composerBusy = false
    /// The chat on screen (under the full player, or the Chats tab): a card rises only there.
    var onScreen: InboxSurface?

    /// One card: the core's arrival and a number that tells two cards apart.
    struct ArrivalCard: Equatable {
        var arrival: MentionArrival
        var seq: Int
    }

    var unreadTotal: Int { list.unreadTotal }

    @ObservationIgnored private(set) var core: CoreBridge
    @ObservationIgnored private var listeners: [Task<Void, Never>] = []
    /// Screens showing the rows; a change reloads the first page only while one is up.
    @ObservationIgnored private var viewers = 0
    /// Called with each entry the core asks to announce now (`mention-notify`).
    @ObservationIgnored var onNotify: ((MentionEntry) -> Void)?
    /// Opens a row's message: its chat, then the message brought into view (AppModel's route).
    @ObservationIgnored var onOpen: ((String) -> Void)?
    /// The settings, for a row's Mute this chat.
    @ObservationIgnored weak var settings: SettingsStore?
    @ObservationIgnored private var arrivals = 0

    init(core: CoreBridge) {
        self.core = core
    }

    /// Starts following the core's pushes and reads the unread count.
    func start() {
        listen()
        Task { await refresh() }
    }

    #if DEBUG
    /// Points the store at another core (a drive step's fixture inbox), so its rows and writes
    /// never touch the signed-in account's inbox.
    func use(_ core: CoreBridge) {
        self.core = core
        list = MentionList()
        listen()
        Task { await refresh() }
    }
    #endif

    private func listen() {
        listeners.forEach { $0.cancel() }
        let changed = core.events.stream("mention-inbox-changed")
        let notify = core.events.stream("mention-notify")
        let arrived = core.events.stream("mention-arrived")
        let pictures = core.events.stream("channel-avatars")
        let roster = core.events.stream("account-roster-changed")
        listeners = [
            Task { [weak self] in
                for await event in changed {
                    guard let self else { return }
                    if let push = MentionInboxChanged.parse(event.payloadJSON) { self.list.setUnread(push.unreadTotal) }
                    self.revision += 1
                    if self.viewers > 0 { await self.refresh() }
                }
            },
            // A new entry for the card over the bar. The core already left out replay, a chat on
            // screen and streamer mode; with the sheet open the sheet shows it instead.
            Task { [weak self] in
                for await event in arrived {
                    guard let self,
                          let arrival = try? JSONDecoder().decode(MentionArrival.self, from: Data(event.payloadJSON.utf8))
                    else { continue }
                    log.notice("inbox arrival \(arrival.entry.id, privacy: .public)")
                    guard self.sheetIn == nil else { continue }
                    self.arrivals += 1
                    self.arrival = ArrivalCard(arrival: arrival, seq: self.arrivals)
                }
            },
            // Senders' pictures landing: an open inbox reads again, a card still up takes its own.
            Task { [weak self] in
                for await event in pictures {
                    guard let self else { return }
                    if var card = self.arrival, card.arrival.avatar == nil, let key = InboxText.avatarKey(card.arrival.entry),
                       let found = try? JSONDecoder().decode([String: String].self, from: Data(event.payloadJSON.utf8)),
                       let url = found[key] {
                        card.arrival.avatar = url
                        self.arrival = card
                    }
                    if self.sheetIn != nil || self.viewers > 0 { self.revision += 1 }
                }
            },
            Task { [weak self] in
                for await event in notify {
                    guard let entry = try? JSONDecoder().decode(MentionEntry.self, from: Data(event.payloadJSON.utf8)) else { continue }
                    self?.onNotify?(entry)
                }
            },
            // Another account's inbox replaces this one's.
            Task { [weak self] in
                for await _ in roster {
                    self?.list = MentionList()
                    await self?.refresh()
                }
            },
        ]
    }

    /// One page of the inbox timeline for a chip, as the core builds it (filtered, identical
    /// messages folded, pictures it has attached).
    func view(_ filter: String, before: Int64? = nil, limit: Int = MentionList.pageSize) async throws -> InboxView {
        var args: [String: Any] = ["filter": filter, "limit": limit]
        if let before { args["beforeMs"] = before }
        return try await core.invoke("get_inbox_view", args: args, as: InboxView.self)
    }

    /// The rows an inbox showed, read as it closes.
    func markSeen(_ ids: [String]) {
        guard !ids.isEmpty else { return }
        log.notice("inbox closing, marking \(ids.count) seen")
        Task { _ = try? await core.invoke("mark_mentions_read", args: ["ids": ids]) }
    }

    /// The tray's tap: this chat's sheet opens, or closes when open. A card folds away.
    func toggleSheet(_ surface: InboxSurface) {
        sheetIn = sheetIn == surface ? nil : surface
        arrival = nil
    }

    func dismissArrival(_ seq: Int) {
        if arrival?.seq == seq { arrival = nil }
    }

    /// A row's tap: the message (its whole fold reads as read), or the whisper.
    func open(row: InboxRowView) {
        sheetIn = nil
        let unreadFolded = row.folded.filter { !$0.read }.map(\.id)
        if !unreadFolded.isEmpty {
            Task { _ = try? await core.invoke("mark_mentions_read", args: ["ids": unreadFolded]) }
        }
        onOpen?(row.entry.id)
    }

    #if DEBUG
    /// A drive step's card, as the core's `mention-arrived` would bring it.
    func showArrival(_ arrival: MentionArrival) {
        arrivals += 1
        self.arrival = ArrivalCard(arrival: arrival, seq: arrivals)
    }
    #endif

    /// The Mentions screen came up or went away.
    func appeared() {
        viewers += 1
        Task { await refresh() }
    }

    func disappeared() { viewers = max(0, viewers - 1) }

    /// The first page again; older pages already loaded stay under it.
    func refresh() async {
        do {
            let page = try await core.invoke("get_mention_inbox", args: ["limit": MentionList.pageSize], as: MentionPage.self)
            list.takeFirst(page)
            failed = false
        } catch {
            failed = !list.loaded
            log.error("get_mention_inbox failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The next page, once, when the last row comes into view.
    func loadMore() async {
        guard let before = list.nextBefore, !loadingMore else { return }
        loadingMore = true
        defer { loadingMore = false }
        do {
            let page = try await core.invoke("get_mention_inbox", args: ["beforeMs": before, "limit": MentionList.pageSize],
                                             as: MentionPage.self)
            list.takeMore(page)
        } catch {
            log.error("get_mention_inbox (more) failed: \(String(describing: error), privacy: .public)")
        }
    }

    func markRead(_ ids: [String]?) async {
        list.markRead(ids)
        _ = try? await core.invoke("mark_mentions_read", args: ["ids": ids])
    }

    func remove(_ id: String) async {
        list.remove(id)
        _ = try? await core.invoke("remove_mention", args: ["id": id])
    }

    func clearAll() async {
        list.clear()
        _ = try? await core.invoke("clear_mention_inbox", args: [:])
    }

    /// Opening an entry: the core marks it read and hands it back (nil when it is gone).
    func open(_ id: String) async -> MentionEntry? {
        list.markRead([id])
        return try? await core.invoke("open_mention", args: ["id": id], as: MentionEntry?.self)
    }

    /// Stops a chat's mentions reaching the inbox and notifications (`mentions.muted_channels`).
    /// The core's list editor flips the one room (a login set: trimmed, lowercased, once), so a
    /// mute never rewrites the rest of the list.
    func mute(_ room: String, store: SettingsStore) async {
        await setMuted(room, true, store: store)
    }

    func unmute(_ room: String, store: SettingsStore) async {
        await setMuted(room, false, store: store)
    }

    private func setMuted(_ room: String, _ muted: Bool, store: SettingsStore) async {
        if !store.loaded { await store.refresh() }
        let key = room.trimmingCharacters(in: .whitespaces).lowercased()
        let now = store.strings(Self.mutedPath).contains { $0.lowercased() == key }
        // A toggle flips, so it is sent only when the room is not already where it should be.
        guard now != muted else { return }
        do {
            try await store.edit(Self.mutedPath, .toggle(key))
        } catch {
            log.error("mute chat failed: \(String(describing: error), privacy: .public)")
        }
    }

    private static let mutedPath = "mentions.muted_channels"
}

/// Where an inbox sheet opens: the chat under the player, or the Chats tab.
enum InboxSurface: Equatable {
    case watch, chats
}
