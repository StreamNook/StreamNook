import SNKit
import SwiftUI
import UIKit

/// The watch page's chat: the room tabs (from two rooms up), then the room on screen. Each room
/// keeps its own list state; switching shows the other room live, never a pause left from before.
struct ChatRoomsPanel: View {
    let rooms: ChatRooms
    var overlay = false
    /// The stream's own card over the chat (the drop being earned), above the pinned banner.
    var streamBanner: AnyView?
    var onProfile: (ProfileTarget) -> Void = { _ in }

    var body: some View {
        VStack(spacing: 0) {
            if rooms.showsStrip {
                ChatTabStrip(rooms: rooms)
            }
            if let session = rooms.activeSession {
                ChatPanel(session: session, overlay: overlay, rooms: rooms, streamBanner: streamBanner, onProfile: onProfile)
                    .id(ObjectIdentifier(session))
            } else {
                Color.clear
            }
        }
        .sheet(isPresented: Binding(get: { rooms.addOpen }, set: { rooms.addOpen = $0 })) {
            AddChatSheet(rooms: rooms)
                .snSheetLight()
        }
    }
}

/// One room's chat column: the list with its pinned messages floating over the top, the streak
/// banners, then the composer. The overlay variant is the same list read-only, for immersive
/// landscape over the video, without pins or banners.
struct ChatPanel: View {
    let session: ChatSession
    var overlay = false
    /// The open rooms this one belongs to (the watch page); nil on the chat stage.
    var rooms: ChatRooms?
    /// The stream's own card, above the pins. It stays mounted in every mode (it decides itself
    /// whether to show), so its reading never stops.
    var streamBanner: AnyView?
    /// Opens a chatter's profile (the profile sheet belongs to the profile screens).
    var onProfile: (ProfileTarget) -> Void = { _ in }

    @State private var toast: String?

    var body: some View {
        VStack(spacing: 0) {
            ChatListHost(session: session, readOnly: overlay, onProfile: onProfile, onToast: show, onOutcome: show)
                // Rows fade out under a pinned banner rather than showing a cut sliver above it.
                // Under the tab strip, a short fade instead of a hard cut too.
                .mask {
                    let pinned = !overlay && !(rooms?.activePins.isEmpty ?? true)
                    let cards = votes.map { !$0.slots.isEmpty } ?? false
                    if pinned || cards || (!overlay && rooms?.showsStrip == true) {
                        VStack(spacing: 0) {
                            LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom)
                                .frame(height: pinned || cards ? 30 : 12)
                            Color.black
                        }
                    } else {
                        Color.black
                    }
                }
                .overlay(alignment: .top) {
                    VStack(spacing: 4) {
                        if let streamBanner { streamBanner }
                        if let rooms, !overlay, !rooms.activePins.isEmpty {
                            PinnedBanner(pins: rooms.activePins,
                                         expanded: Binding(get: { rooms.pinsOpen }, set: { rooms.pinsOpen = $0 }))
                                .padding(.horizontal, 10)
                                .padding(.top, 6)
                        }
                        // The live poll and prediction, under the pins; never over landscape video.
                        if let votes {
                            LiveVotesStack(model: votes)
                                .padding(.horizontal, 10)
                                .padding(.top, rooms?.activePins.isEmpty ?? true ? 6 : 0)
                        }
                        if let toast {
                            Text(toast)
                                .font(.snCaption)
                                .padding(.horizontal, 12)
                                .padding(.vertical, 7)
                                .glassEffect()
                                .padding(.top, 8)
                                .transition(.move(edge: .top).combined(with: .opacity))
                        }
                    }
                }
            if !overlay {
                if let rooms {
                    StreakBanners(rooms: rooms, session: session)
                }
                ChatComposer(session: session, onProfile: onProfile)
            }
        }
        .onAppear {
            session.start()
            if let votes {
                votes.onToast = { [rooms] text, success in
                    // The app's toasts show only failures; a success speaks over the list.
                    if !success, let toasts = rooms?.toasts {
                        toasts.show(text, .error)
                    } else {
                        show(text)
                    }
                }
                votes.start()
            }
        }
    }

    /// The room's poll and prediction cards, never in the read-only landscape overlay.
    private var votes: LiveVotesModel? {
        overlay ? nil : LiveVotesRegistry.shared.model(for: session)
    }

    private func show(_ text: String) {
        withAnimation(.snappy) { toast = text }
        Task {
            try? await Task.sleep(for: .seconds(2.5))
            withAnimation(.snappy) { if toast == text { toast = nil } }
        }
    }

    /// A moderator action's answer: the app's toast (a failure, or Undo after a timeout or ban);
    /// the chat stage has no app toasts, so it shows the words over its list.
    private func show(_ outcome: ChatModActions.Outcome) {
        guard let toasts = rooms?.toasts else {
            show(outcome.text)
            return
        }
        toasts.show(outcome.text, outcome.success ? .success : .error, action: outcome.undo == nil ? nil : "Undo",
                    perform: outcome.undo)
    }
}

/// The UIKit chat list inside SwiftUI.
struct ChatListHost: UIViewRepresentable {
    let session: ChatSession
    var readOnly: Bool
    var onProfile: (ProfileTarget) -> Void
    var onToast: (String) -> Void
    var onOutcome: (ChatModActions.Outcome) -> Void

    func makeUIView(context: Context) -> ChatListView {
        let list = ChatListView()
        list.source = session.pipeline
        list.readOnly = readOnly
        let pipeline = session.pipeline
        // A room coming back on screen hands the new list every row it kept meanwhile.
        pipeline.replay()
        list.onWidthChange = { [weak list] width in
            pipeline.setWidth(width, scale: list?.traitCollection.displayScale ?? 3)
        }
        list.onPausedChange = { paused in session.setPaused(paused) }
        list.onReply = { id in
            session.replyTo = session.pipeline.message(id)
        }
        list.onTap = { [weak list] action, layout in
            switch action {
            case .name:
                if let message = session.pipeline.message(layout.id), message.provider == "twitch" {
                    onProfile(ProfileTarget(userID: message.userID, login: message.username, displayName: message.displayName,
                                            color: message.metadata.view?.nameColor ?? message.color,
                                            badgeKeys: message.badges.map { "\($0.name)/\($0.version)" },
                                            roomID: session.roomID, roomLogin: session.channel,
                                            subBadgeArt: message.badges.first { $0.name == "subscriber" || $0.name == "founder" }
                                                .flatMap { $0.imageURL4x ?? $0.imageURL2x ?? $0.imageURL1x }))
                }
            case .mention(let login):
                // The core resolves the login to an id (Helix, cached); mentions carry no badges.
                if session.pipeline.recentMessages(1).first?.provider ?? "twitch" == "twitch" {
                    onProfile(ProfileTarget(userID: "", login: login, displayName: login, roomID: session.roomID,
                                            roomLogin: session.channel))
                }
            case .link(let url):
                LinkRouter.open(url)
            case .replyParent(let id):
                if list?.jump(to: id) != true {
                    list?.pauseForJump()
                    onToast("Original message is no longer in chat history")
                }
            case .held(let messageID, let allow):
                // Twitch's answer folds the row (the core's AUTOMOD_HELD update); only a refusal
                // needs words here.
                Task { @MainActor in
                    do {
                        try await session.core.resolveHeld(messageID: messageID, allow: allow)
                    } catch {
                        onToast(allow ? "Could not allow that message: \(error)" : "Could not deny that message: \(error)")
                    }
                }
            }
        }
        list.fanContext = { layout in
            // A held message has its own Allow and Deny; there is nothing to reply to yet.
            guard let message = session.pipeline.message(layout.id), !message.isSystem,
                  !layout.id.hasPrefix(ChatRoomBuffer.heldRowID("")) else { return nil }
            let twitch = message.provider == "twitch"
            var buckets: [FanBucket] = []
            if !readOnly { buckets.append(.reply) }
            if twitch { buckets.append(.profile) }
            buckets.append(.copy)
            if session.moderatorTools, twitch, session.roomID != nil {
                buckets += [.delete, .timeout, .ban, .pin]
            }
            let color = ChatTheme.color(message.metadata.view?.nameColor ?? message.color) ?? ChatTheme.fallbackName
            return (buckets, message.shownName, color)
        }
        list.accessibilityActions = { [weak list] layout in
            guard let context = list?.fanContext?(layout) else { return [] }
            return context.buckets.map { bucket in
                UIAccessibilityCustomAction(name: bucket.title) { _ in
                    list?.onFanCommit?(bucket, layout, bucket == .timeout ? 600 : nil)
                    return true
                }
            }
        }
        list.onFanCommit = { bucket, layout, seconds in
            guard let message = session.pipeline.message(layout.id) else { return }
            Task { @MainActor in
                if let outcome = await ChatModActions.run(bucket, message: message, seconds: seconds, session: session,
                                                           onProfile: onProfile) {
                    onOutcome(outcome)
                }
            }
        }
        context.coordinator.list = list
        ChatRegistry.shared.lists[ObjectIdentifier(session)] = WeakList(list)
        return list
    }

    func updateUIView(_ list: ChatListView, context: Context) {
        list.readOnly = readOnly
        list.setStyle(session.style)
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        weak var list: ChatListView?
    }
}

/// Lists on screen by session, so drive steps and the composer can reach them.
@MainActor
final class ChatRegistry {
    static let shared = ChatRegistry()
    var lists: [ObjectIdentifier: WeakList] = [:]

    func list(for session: ChatSession) -> ChatListView? {
        lists[ObjectIdentifier(session)]?.list
    }
}

final class WeakList {
    weak var list: ChatListView?
    init(_ list: ChatListView) { self.list = list }
}
