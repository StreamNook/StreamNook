import Foundation
import Observation
import SNKit

/// One composer's channel wallet: the points and bits Rust's `channel_state` holds for the room
/// being typed in (the chat rooms already watch every open room), the bonus chest, and the "+N"
/// for each gain Rust records. Every number comes from the core; this keeps no balance math.
@Observable @MainActor
final class WalletModel {
    let login: String
    private let core: CoreBridge
    private(set) var faces = WalletFaces()
    /// The gains still to float, oldest first; one plays at a time.
    private(set) var pops: [Pop] = []
    private(set) var claimingChest = false
    var sheetOpen = false
    /// Show gains is Bonus chest only; the button keeps it in step with the room's chat style.
    var chestOnly = false
    /// A reward whose detail the sheet opens on (drive steps).
    var pendingReward: String?
    /// Wallets on screen, by login (drive steps).
    static var live: [String: WalletModel] = [:]
    /// The last gain seen: nil until the first answer, which is only a baseline, so a gain
    /// recorded before this composer existed never plays.
    private var seenSeq: UInt64?
    private var listening: Task<Void, Never>?

    struct Pop: Identifiable, Equatable {
        let id: UInt64
        let amount: Int64
    }

    init(login: String, core: CoreBridge) {
        self.login = login.lowercased()
        self.core = core
    }

    var points: ChannelPoints? { faces.pointsOn }
    var bits: ChannelBitsFace? { faces.bitsOn }
    /// A chest waiting to be collected (Rust collects it itself on the watched stream when
    /// auto-collect is on, so this mostly shows with it off or on another room).
    var chestWaiting: Bool { points?.availableClaimID != nil }

    /// Reads the faces once and follows the core's pushes for this channel.
    func start() async {
        Self.live[login] = self
        if listening == nil {
            let pushes = core.events.stream("channel-state")
            listening = Task { [weak self] in
                for await event in pushes {
                    guard let self, let push = WalletFaces.parsePush(event.payloadJSON) else { continue }
                    self.take(push)
                }
            }
        }
        if let json = try? await core.invoke("get_channel_state", args: ["login": login]),
           let first = WalletFaces.parseState(json) {
            faces = first
            noteGain(first.points?.lastGain)
        }
    }

    func stop() {
        listening?.cancel()
        listening = nil
        if Self.live[login] === self { Self.live[login] = nil }
    }

    private func take(_ push: WalletFaces.Push) {
        switch push {
        case .points(let who, let points) where who == login:
            faces.points = points
            noteGain(points?.lastGain)
        case .bits(let who, let bits) where who == login:
            faces.bits = bits
        default:
            break
        }
    }

    private func noteGain(_ gain: WalletGain?) {
        let seq = gain?.seq ?? 0
        guard let seen = seenSeq else {
            seenSeq = seq
            return
        }
        guard seq > seen, let gain else { return }
        seenSeq = seq
        guard gain.floats(chestOnly: chestOnly) else { return }
        pops.append(Pop(id: seq, amount: gain.amount))
    }

    /// The float at the head finished.
    func popDone(_ id: UInt64) {
        pops.removeAll { $0.id == id }
    }

    /// Asks Rust for a fresh points read; the answer arrives as a push.
    func refresh() async {
        _ = try? await core.invoke("refresh_channel_state", args: ["login": login, "section": "points"])
    }

    /// Collects the waiting chest by hand through Rust's one chest path.
    func claimChest(channelID: String?) async {
        guard let claim = points?.availableClaimID, let channelID, !claimingChest else { return }
        claimingChest = true
        defer { claimingChest = false }
        _ = try? await core.invoke("claim_channel_points", args: ["channelId": channelID, "channelName": login, "claimId": claim])
        await refresh()
    }
}
