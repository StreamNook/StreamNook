import SNKit
import SwiftUI
import UIKit

/// The watch page's chat: the room tabs (from two rooms up), then the room on screen. Each room
/// keeps its own list state; switching shows the other room live, never a pause left from before.
struct ChatRoomsPanel: View {
    let rooms: ChatRooms
    var overlay = false
    /// A playing VOD's recorded chat; shown instead of the rooms while `replayShown` is on.
    var replay: ChatSession?
    var replayShown: Binding<Bool>?
    /// The VOD's playhead, for the line beside the switch.
    var replayAt: String?
    /// The drop being earned on the watched stream, a card of the stack at the top of chat.
    var drop: WatchDropModel?
    var onProfile: (ProfileTarget) -> Void = { _ in }

    var body: some View {
        VStack(spacing: 0) {
            if let replay, let replayShown {
                ReplaySwitch(shown: replayShown, at: replayAt)
                if replayShown.wrappedValue {
                    ChatPanel(session: replay, overlay: true, onProfile: onProfile)
                        .id(ObjectIdentifier(replay))
                } else {
                    roomsBody
                }
            } else {
                roomsBody
            }
        }
        .sheet(isPresented: Binding(get: { rooms.addOpen }, set: { rooms.addOpen = $0 })) {
            AddChatSheet(rooms: rooms)
                .snSheetLight()
        }
    }

    @ViewBuilder
    private var roomsBody: some View {
        VStack(spacing: 0) {
            if rooms.showsStrip {
                ChatTabStrip(rooms: rooms)
            }
            if let session = rooms.activeSession {
                ChatPanel(session: session, overlay: overlay, rooms: rooms, drop: drop, onProfile: onProfile)
                    .id(ObjectIdentifier(session))
            } else {
                Color.clear
            }
        }
    }
}

/// Over a VOD's chat: Replay (the recorded chat, in step with the video) or the channel's live
/// chat, and where the replay is.
struct ReplaySwitch: View {
    @Binding var shown: Bool
    let at: String?

    var body: some View {
        HStack(spacing: 10) {
            GlazeSegments([Segment(value: true, label: "Replay"), Segment(value: false, label: "Live chat")],
                          selection: Binding(get: { shown }, set: { if let on = $0 { shown = on } }))
                .fixedSize()
            Spacer(minLength: 8)
            if shown, let at {
                Text("chat at \(at)")
                    .font(.sn(12.5).monospacedDigit())
                    .foregroundStyle(SN.textMuted)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 14)
        .padding(.top, 8)
        .padding(.bottom, 4)
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
    /// The watched stream's drop, which joins the poll and prediction in the stack at the top
    /// (it shows only while its model says so: the stream's own room, below the video).
    var drop: WatchDropModel?
    /// Opens a chatter's profile (the profile sheet belongs to the profile screens).
    var onProfile: (ProfileTarget) -> Void = { _ in }
    /// The pins and live votes take their own room above the list instead of floating over its
    /// top rows (the Chats screen, which has the height), a grabber's gap under the header.
    var pinsInline = false
    /// Where the column's bottom (the composer and any banner over it) starts, in global points,
    /// so the mini player can rest clear of it.
    var onFloor: ((CGFloat) -> Void)?

    @State private var toast: String?
    @AppStorage(SwipeHint.key) private var hintLearned = false
    /// The pinned banner's and the vote cards' heights, for the inline room.
    @State private var pinsBox: CGFloat = 0
    @State private var votesBox: CGFloat = 0

    var body: some View {
        VStack(spacing: 0) {
            ChatListHost(session: session, readOnly: overlay, onProfile: onProfile, onToast: show, onOutcome: show)
                .padding(.top, inlineTop)
                // Rows fade out under a pinned banner rather than showing a cut sliver above it.
                // Under the tab strip (or under inline pins), a short fade instead of a hard cut.
                .mask {
                    let pinned = !overlay && (rooms?.pinsShown ?? false)
                    let mentionCard = !overlay && rooms?.mentionCards[session.channel] != nil
                    let cards = mentionCard || (votes.map { !$0.slots.isEmpty } ?? false)
                    if pinned || cards || (!overlay && rooms?.showsStrip == true) {
                        VStack(spacing: 0) {
                            Color.clear.frame(height: inlineTop)
                            LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom)
                                .frame(height: (pinned || cards) && inlineTop == 0 ? 30 : 12)
                            Color.black
                        }
                    } else {
                        Color.black
                    }
                }
                // Rows under the inbox's sheet or card fade out above it instead of showing as
                // slivers beside it.
                .mask { InboxClearMask(sheet: inboxSheetHere, card: inboxCardHere) }
                // The stack's height cap and its short-column rule are Rust's, from this height.
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
                    votes?.setChatHeight(height)
                }
                // The inbox: a new mention's card risen from the tray, or the sheet grown out of
                // the bar. Over the rows, flush on the composer below them.
                .overlay {
                    if let mentions = rooms?.mentions {
                        ZStack(alignment: .bottom) {
                            if !overlay, mentions.onScreen == surface {
                                InboxArrivalCard(store: mentions, leading: !session.style.inboxRight)
                            }
                            // In landscape the tray sits at the strip's leading end.
                            InboxSheetOverlay(store: mentions, surface: surface,
                                              leading: overlay || !session.style.inboxRight)
                        }
                    }
                }
                .overlay(alignment: .top) {
                    VStack(spacing: 4) {
                        if let rooms, !overlay, rooms.pinsShown {
                            let unrolled = Binding(get: { rooms.pinsOpen }, set: { rooms.pinsOpen = $0 })
                            PinnedBanner(pins: rooms.activePins, expanded: unrolled)
                                // Unrolled, it floats over the rows: a tap outside rolls it up.
                                .closesOnOutsideTap(unrolled, id: "pins")
                                .padding(.horizontal, 10)
                                .padding(.top, topGap)
                                // Opened, the banner floats over the rows; the list keeps the
                                // closed banner's room, so nothing below it moves.
                                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height in
                                    if !rooms.pinsOpen { pinsBox = height }
                                }
                        }
                        if let rooms, !overlay, let card = rooms.mentionCards[session.channel] {
                            MentionContextCard(entry: card) { rooms.dismissMentionCard(session.channel) }
                                .closesOnOutsideTap(Binding(get: { true }, set: { if !$0 { rooms.dismissMentionCard(session.channel) } }),
                                                    id: "mention-card")
                                .padding(.horizontal, 10)
                                .padding(.top, 6)
                                .transition(.opacity.combined(with: .move(edge: .top)))
                        }
                        // The live poll, prediction and drop as one stack, under the pins; never
                        // over landscape video.
                        if let votes {
                            LiveVotesStack(model: votes, drop: drop)
                                .padding(.horizontal, 10)
                                .padding(.top, rooms?.pinsShown ?? false ? 0 : topGap)
                                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { votesBox = $0 }
                        }
                        if !overlay, session.moderatorTools, session.style.swipeModerate, session.roomID != nil, !hintLearned {
                            SwipeHint { hintLearned = true }
                                .padding(.top, 8)
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
                // The line between the rows and what belongs to the composer: the streak banners
                // sit under it, directly over the field, since sharing sends what is typed there.
                VStack(spacing: 0) {
                    Rectangle()
                        .fill(SN.borderSubtle)
                        .frame(height: 1)
                    if let rooms {
                        StreakBanners(rooms: rooms, session: session)
                    }
                    ChatComposer(session: session, onProfile: onProfile)
                        .environment(\.inboxSurface, surface)
                }
                .onGeometryChange(for: CGFloat.self) { $0.frame(in: .global).minY } action: { onFloor?($0) }
            }
        }
        // The drop reaches the stack as one of its cards (the model asks Rust where it goes).
        .onChange(of: overlay ? nil : drop?.shown?.dropID, initial: true) { _, id in
            votes?.setDrop(id)
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

    /// The pins' gap under the header: the grabber's gap on Chats, a tight 6 under the player.
    private var topGap: CGFloat { pinsInline ? 12 : 6 }

    /// Which chat this is, for the inbox: the Chats tab's, or the one under the player.
    private var surface: InboxSurface { pinsInline ? .chats : .watch }

    private var inboxSheetHere: Bool { rooms?.mentions?.sheetIn == surface }

    private var inboxCardHere: Bool {
        guard !overlay, let mentions = rooms?.mentions else { return false }
        return mentions.arrival != nil && !mentions.composerBusy && mentions.sheetIn == nil && mentions.onScreen == surface
    }

    /// The room the inline pins and votes take above the list, and 4 points of air under them.
    private var inlineTop: CGFloat {
        guard pinsInline, !overlay else { return 0 }
        let pins = rooms?.pinsShown ?? false ? pinsBox : 0
        let cards = (votes.map { !$0.slots.isEmpty } ?? false) ? votesBox : 0
        let total = pins + cards + (pins > 0 && cards > 0 ? 4 : 0)
        return total > 0 ? total + 4 : 0
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
                // Delete, Timeout and Ban only while Hold menu buttons is on (Settings > Moderation),
                // and only those Twitch allows on this message: Delete alone on the viewer's own.
                if session.style.holdModButtons {
                    let reach = session.reach(for: message)
                    if reach.canDelete { buckets.append(.delete) }
                    if reach.canTimeout { buckets += [.timeout, .ban] }
                }
                buckets.append(.pin)
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
        // Swipe right on a message to moderate it (the core lays out the slider and words it).
        list.swipeAllowed = { layout in
            guard session.moderatorTools, session.roomID != nil, !layout.id.hasPrefix(ChatRoomBuffer.heldRowID("")),
                  let message = session.pipeline.message(layout.id), message.provider == "twitch", !message.isSystem,
                  message.tags["system-msg"] == nil
            else { return false }
            // Only where Twitch allows something on this message: the viewer's own in a room they
            // moderate (Delete), never the broadcaster's (the core decides the steps).
            return session.reach(for: message) != .nothing
        }
        list.swipeRefusal = { layout in
            if layout.id.hasPrefix(ChatRoomBuffer.heldRowID("")) { return "a held message" }
            return session.swipeRefusal(session.pipeline.message(layout.id))
        }
        // A press on a moderatable row: the core's slider for where the thumb landed, and the
        // chatter captured now, so a release acts on the person pressed even if their message
        // has since left the buffer.
        list.prepareSwipe = { layout, width, startX in
            guard let message = session.pipeline.message(layout.id), let room = session.roomID else { return nil }
            let core = session.core
            let slider: SwipeSlider
            do {
                slider = try await core.swipeSlider(width: Double(width), startX: Double(startX), name: message.shownName,
                                                    targetID: message.userID,
                                                    badges: message.badges.map { "\($0.name)/\($0.version)" },
                                                    channel: session.channel)
            } catch {
                chatLog.notice("[swipe] slider failed: \(String(describing: error), privacy: .public)")
                return nil
            }
            let (messageID, userID, name) = (message.id, message.userID, message.shownName)
            return ChatListView.PreparedSwipe(slider: slider) { step in
                Task { @MainActor in
                    let outcome = await ChatModActions.runSwipe(step, core: core, room: room, messageID: messageID,
                                                                userID: userID, name: name)
                    onOutcome(outcome)
                }
            }
        }
        list.onSwipeDone = { SwipeHint.learned = true }
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
        // Swiping is on where moderator tools are; the core lays the slider out per press.
        list.swipeEnabled = session.moderatorTools && session.roomID != nil && !readOnly
        if list.swipeEnabled {
            let core = session.core
            Task { @MainActor [weak list] in
                if let quick = try? await core.swipeSlider(width: 390).quickSeconds { list?.quickSeconds = quick }
            }
        }
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


/// The one-time hint that swiping a message moderates it, shown over the list until a swipe has
/// run once (or it is dismissed). Android shows the same words.
struct SwipeHint: View {
    static let key = "sn-swipe-mod-learned"
    static var learned: Bool {
        get { UserDefaults.standard.bool(forKey: key) }
        set { UserDefaults.standard.set(newValue, forKey: key) }
    }
    var onDismiss: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "hand.draw")
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(SN.accent)
            Text("Swipe right on a message to moderate it")
                .font(.snCaption)
                .foregroundStyle(.primary)
            // The X's place; the X itself sits over the pill so it alone takes a touch.
            Color.clear.frame(width: 18, height: 18)
        }
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .padding(.vertical, 5)
        .glassEffect()
        // Only the X takes a touch: a swipe on the row under the pill still starts.
        .allowsHitTesting(false)
        .overlay(alignment: .trailing) {
            Button(action: onDismiss) {
                Image(systemName: "xmark")
                    .font(.system(size: 10, weight: .bold))
                    .foregroundStyle(SN.textMuted)
                    .padding(4)
            }
            .accessibilityLabel("Dismiss the swipe hint")
            .padding(.trailing, 6)
        }
        .transition(.opacity)
    }
}
