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

    var unreadTotal: Int { list.unreadTotal }

    @ObservationIgnored private(set) var core: CoreBridge
    @ObservationIgnored private var listeners: [Task<Void, Never>] = []
    /// Screens showing the rows; a change reloads the first page only while one is up.
    @ObservationIgnored private var viewers = 0
    /// Called with each entry the core asks to announce now (`mention-notify`).
    @ObservationIgnored var onNotify: ((MentionEntry) -> Void)?

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
        let roster = core.events.stream("account-roster-changed")
        listeners = [
            Task { [weak self] in
                for await event in changed {
                    guard let self else { return }
                    if let push = MentionInboxChanged.parse(event.payloadJSON) { self.list.setUnread(push.unreadTotal) }
                    if self.viewers > 0 { await self.refresh() }
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
