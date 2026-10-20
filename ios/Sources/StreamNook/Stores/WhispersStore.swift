import Foundation
import Observation
import SNKit

/// The whisper inbox as the app shows it: the core's list (`whisper_list`), a conversation
/// (`whisper_thread`), and the writes (`whisper_mark_read`, `whisper_set_active`, `whisper_send`).
/// The core files every incoming whisper from its socket, keeps the unread counts, and decides
/// which whisper is worth a notification (`whisper-notify`); this presents and asks, and holds
/// only what was typed here and is still on its way or was refused.
@Observable @MainActor
final class WhispersStore {
    private(set) var list = WhisperList(ownerID: "", conversations: [], unreadTotal: 0)
    /// A list answer arrived at least once.
    private(set) var loaded = false
    /// The list failed to load and there is nothing to show.
    private(set) var failed = false
    /// Typed here and not confirmed (or refused) by Twitch, oldest first.
    private(set) var pending: [PendingWhisper] = []
    /// Bumped on every change the core reports, so an open conversation re-reads.
    private(set) var revision = 0
    /// The newest change the core reported, for the open conversation to show at once.
    private(set) var lastUpdate: WhisperUpdate?

    var unreadTotal: Int { list.unreadTotal }

    @ObservationIgnored private(set) var core: CoreBridge
    @ObservationIgnored private var listeners: [Task<Void, Never>] = []
    /// The conversation on screen, while the app is in front.
    @ObservationIgnored private var openKey: String?
    @ObservationIgnored private var inFront = true
    /// Called with each whisper the core asks to announce now (`whisper-notify`).
    @ObservationIgnored var onNotify: ((WhisperUpdate) -> Void)?

    init(core: CoreBridge) {
        self.core = core
    }

    /// Starts following the core's pushes and reads the list.
    func start() {
        listen()
        Task { await refresh() }
    }

    #if DEBUG
    /// Points the store at another core (a drive step's fixture inbox), so its rows and writes
    /// never touch the signed-in account's inbox.
    func use(_ core: CoreBridge) {
        self.core = core
        list = WhisperList(ownerID: "", conversations: [], unreadTotal: 0)
        loaded = false
        pending = []
        lastUpdate = nil
        listen()
        Task { await refresh() }
    }
    #endif

    private func listen() {
        listeners.forEach { $0.cancel() }
        let updated = core.events.stream("whisper-conversation-updated")
        let archive = core.events.stream("whisper-archive-changed")
        let notify = core.events.stream("whisper-notify")
        let roster = core.events.stream("account-roster-changed")
        listeners = [
            Task { [weak self] in
                for await event in updated {
                    guard let self, let update = WhisperUpdate.parse(event.payloadJSON) else { continue }
                    self.take(update)
                }
            },
            Task { [weak self] in
                for await _ in archive {
                    guard let self else { return }
                    self.lastUpdate = nil
                    self.revision += 1
                    await self.refresh()
                }
            },
            Task { [weak self] in
                for await event in notify {
                    guard let update = WhisperUpdate.parse(event.payloadJSON) else { continue }
                    self?.onNotify?(update)
                }
            },
            // Another account's inbox replaces this one's.
            Task { [weak self] in
                for await _ in roster {
                    guard let self else { return }
                    self.pending = []
                    self.lastUpdate = nil
                    await self.refresh()
                }
            },
        ]
    }

    private func take(_ update: WhisperUpdate) {
        if !list.ownerID.isEmpty, update.ownerID != list.ownerID { return }
        lastUpdate = update
        revision += 1
        Task { await refresh() }
    }

    /// The list again.
    func refresh() async {
        do {
            list = try await core.invoke("whisper_list", args: [:], as: WhisperList.self)
            loaded = true
            failed = false
        } catch {
            failed = !loaded
            log.error("whisper_list failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The row for `key`, if the list has it.
    func row(_ key: String) -> WhisperRow? { list.conversations.first { $0.key == key } }

    /// One conversation, nil when it is gone.
    func thread(_ key: String) async throws -> WhisperThread? {
        guard !list.ownerID.isEmpty else { return nil }
        return try await core.invoke("whisper_thread", args: ["ownerId": list.ownerID, "key": key], as: WhisperThread?.self)
    }

    // MARK: The conversation on screen

    /// The conversation view came up (a key) or went away (nil). The core reads a whisper landing
    /// in it as read and does not notify for it, but only while the app is in front.
    func setOpen(_ key: String?) {
        openKey = key
        report()
    }

    /// The app came to the front or left it.
    func setInFront(_ front: Bool) {
        guard front != inFront else { return }
        inFront = front
        report()
    }

    private func report() {
        let key = inFront ? openKey : nil
        let core = core
        Task { _ = try? await core.invoke("whisper_set_active", args: ["key": key]) }
        if let key { Task { await markRead(key) } }
    }

    /// The conversation is on screen: the core zeroes its unread count.
    func markRead(_ key: String) async {
        guard !list.ownerID.isEmpty else { return }
        if let i = list.conversations.firstIndex(where: { $0.key == key }), list.conversations[i].unreadCount > 0 {
            list.unreadTotal = max(0, list.unreadTotal - list.conversations[i].unreadCount)
            list.conversations[i].unreadCount = 0
        }
        do {
            _ = try await core.invoke("whisper_mark_read", args: ["ownerId": list.ownerID, "key": key])
        } catch {
            log.error("whisper_mark_read failed: \(String(describing: error), privacy: .public)")
        }
    }

    // MARK: Sending

    @ObservationIgnored private var seq = 0

    /// Sends `text` to the conversation: shown at once as sending, then as sent (the core's own
    /// copy) or as not sent with Twitch's reason.
    func send(_ text: String, to row: WhisperRow) {
        let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return }
        seq += 1
        let item = PendingWhisper(id: "local-\(seq)", key: row.key, text: clean, atMs: Self.nowMs)
        pending.append(item)
        Task { await deliver(item.id, text: clean, to: row) }
    }

    func retry(_ id: String, to row: WhisperRow) {
        guard let i = pending.firstIndex(where: { $0.id == id }), case .failed = pending[i].status else { return }
        pending[i].status = .sending
        pending[i].atMs = Self.nowMs
        let text = pending[i].text
        Task { await deliver(id, text: text, to: row) }
    }

    func discard(_ id: String) {
        pending.removeAll { $0.id == id }
    }

    #if DEBUG
    /// A message shown as on its way and never sent (the drive's picture of the sending state).
    func holdForDrive(_ text: String, to row: WhisperRow) {
        seq += 1
        pending.append(PendingWhisper(id: "local-\(seq)", key: row.key, text: text, atMs: Self.nowMs))
    }
    #endif

    private func deliver(_ id: String, text: String, to row: WhisperRow) async {
        do {
            let update = try await sendThroughCore(text, to: row)
            pending.removeAll { $0.id == id }
            take(update)
        } catch {
            log.error("whisper_send failed: \(String(describing: error), privacy: .public)")
            if let i = pending.firstIndex(where: { $0.id == id }) { pending[i].status = .failed(WhisperText.reason(error)) }
        }
    }

    /// The one send: `whisper_send`, which records the message in the conversation.
    private func sendThroughCore(_ text: String, to row: WhisperRow) async throws -> WhisperUpdate {
        if list.ownerID.isEmpty { await refresh() }
        guard !list.ownerID.isEmpty else { throw CoreFailure.command(json: #""You are not signed in to Twitch.""#) }
        let target: [String: Any?] = [
            "key": row.key, "user_id": row.userID, "user_login": row.userLogin, "user_name": row.userName,
            "profile_image_url": row.profileImageURL,
        ]
        return try await core.invoke("whisper_send", args: [
            "ownerId": list.ownerID, "target": target.mapValues { $0 ?? NSNull() }, "text": text,
        ], as: WhisperUpdate.self)
    }

    /// A reply typed into a whisper notification: the same send. Returns nil when Twitch took
    /// it, else the reason it was not sent.
    func reply(_ text: String, to update: WhisperUpdate) async -> String? {
        let clean = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !clean.isEmpty else { return nil }
        let meta = update.meta
        let target = row(update.key) ?? WhisperRow(key: update.key, userID: meta.userID, userLogin: meta.userLogin,
                                                  userName: meta.userName, profileImageURL: meta.profileImageURL,
                                                  lastMessageTimestamp: meta.lastMessageTimestamp)
        do {
            take(try await sendThroughCore(clean, to: target))
            return nil
        } catch {
            log.error("notification reply failed: \(String(describing: error), privacy: .public)")
            return WhisperText.reason(error)
        }
    }

    private static var nowMs: Int64 { Int64(Date().timeIntervalSince1970 * 1000) }
}
