import SNKit
import SwiftUI
import UIKit

/// Chats: your chats with no stream playing. The same rooms as the strip under the player (the
/// core keeps one list), with the strip as the header and its controls beside the rooms: the play
/// pill starts the room on screen when it is live, and + adds a chat. The composer, emotes,
/// mentions and the points wallet are the ordinary chat panel's.
///
/// Opening this tab is the first look that joins the saved chats (the core restores them at
/// launch without joining). While it is in front it keeps the phone awake until the core's idle
/// span passes with no touch, so a phone left on a desk still sleeps.
struct ChatsScreen: View {
    let model: AppModel
    @State private var awake = AwakeTimer()
    /// How far the screen follows a slide down that started on the header.
    @State private var drop: CGFloat = 0
    @State private var groupShown = false

    private var rooms: ChatRooms { model.chat }

    var body: some View {
        VStack(spacing: 0) {
            header
            if !model.signedIn {
                VStack(spacing: 14) {
                    Text("Sign in with Twitch to read and send chat.")
                        .font(.snBody)
                        .foregroundStyle(SN.textMuted)
                        .multilineTextAlignment(.center)
                        .frame(maxWidth: 280)
                    SignInButton(model: model).frame(maxWidth: 280)
                }
                .padding(24)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else if rooms.tabs.isEmpty {
                // No chat open: the tab is the picker itself, on the large glass the "Add a chat"
                // sheet wears (solid at 0 percent glassiness).
                ChatPicker(rooms: rooms, onClose: { model.nav.exitChats() })
                    .padding(.top, 14)
                    .snGlass(RoundedRectangle(cornerRadius: 28, style: .continuous), light: .dialog)
                    .padding(.horizontal, 8)
                    .padding(.top, 4)
                    .padding(.bottom, 8)
                    .onAppear { model.nav.chatsFloor = nil }
            } else if let session = rooms.activeSession {
                ChatPanel(session: session, rooms: rooms, onProfile: { model.cosmetics.open($0) },
                          pinsInline: true, onFloor: { top in
                              // A slide down moves the whole screen; the mini box keeps its rest.
                              guard drop == 0, model.nav.chatsFloor != top else { return }
                              withAnimation(SNMotion.run(.snappy(duration: 0.25))) { model.nav.chatsFloor = top }
                          },
                          // Under the full player the player's chat draws the room.
                          covered: model.nav.shellCovered)
                    .id(ObjectIdentifier(session))
            } else {
                Color.clear
            }
        }
        // The composer sits on the home indicator's inset the way the watch chat's does: the
        // bottom container inset (which still held the hidden tab bar's room) is ignored, and
        // only what the watch layer keeps below the chat is kept (the inset less 24 pt).
        .padding(.bottom, Self.bottomKeep)
        .ignoresSafeArea(.container, edges: .bottom)
        // The page every tab sits on: the theme's background with the room's light on it.
        .snShellBackground()
        .offset(y: drop)
        .toolbar(.hidden, for: .navigationBar)
        // A full-screen surface: no tab bar here.
        .toolbar(.hidden, for: .tabBar)
        .sheet(isPresented: Binding(get: { rooms.addOpen }, set: { rooms.addOpen = $0 })) {
            AddChatSheet(rooms: rooms)
        }
        .sheet(isPresented: $groupShown) {
            if let group = rooms.activeGroup {
                TogetherSheet(group: group, playing: model.nav.playing?.userLogin ?? "") { member in
                    model.startWatching(TwitchStream(id: "", userLogin: member.login, userName: member.displayName, title: "",
                                                     viewerCount: member.viewerCount ?? 0, gameName: "",
                                                     userID: member.userID, profileImageURL: member.avatarURL))
                }
                .snSheetLight()
            }
        }
        // The first look joins the saved chats.
        .task(id: model.signedIn) {
            if model.signedIn, rooms.state.connection != "connected" { rooms.connect() }
        }
        .onAppear { awake.start(on: keepsAwake, idle: rooms.state.keepAwakeIdleSecs) }
        .onDisappear {
            awake.stop()
            model.nav.chatsFloor = nil
        }
        .onChange(of: keepsAwake) { _, on in awake.start(on: on, idle: rooms.state.keepAwakeIdleSecs) }
    }

    /// The handle: the grabber band (a tap closes), then the rooms. A slide down that starts here
    /// leaves Chats; the chat list is never a handle, so a drag there stays a scroll.
    private var header: some View {
        VStack(spacing: 0) {
            Button {
                model.nav.exitChats()
            } label: {
                Color.clear
            }
            .buttonStyle(GrabberBand(held: drop > 0, reach: Self.topInset))
            .accessibilityLabel("Close chats")
            if model.signedIn, !rooms.tabs.isEmpty {
                // 40 pt capsules with 7 pt of air above and below, clear of the separator.
                ChatTabStrip(rooms: rooms, trailing: AnyView(controls), height: 54, leading: groupChip,
                             edge: AnyView(ChatsMinimizeButton { model.nav.exitChats() }))
            }
        }
        .contentShape(Rectangle())
        .gesture(
            DragGesture(minimumDistance: 8, coordinateSpace: .global)
                .onChanged { value in
                    let dy = value.translation.height
                    guard dy > abs(value.translation.width) || drop > 0 else { return }
                    drop = max(0, dy)
                }
                .onEnded { value in
                    let leave = drop > 120 || value.predictedEndTranslation.height - value.translation.height > 180
                    if leave {
                        withAnimation(.easeIn(duration: 0.2)) { drop = 900 } completion: {
                            model.nav.exitChats()
                            drop = 0
                        }
                    } else {
                        withAnimation(.snappy(duration: 0.24)) { drop = 0 }
                    }
                }
        )
    }

    /// The status bar's inset, which the grabber band reaches up through.
    @MainActor private static var topInset: CGFloat {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow)?.safeAreaInsets.top ?? 0
    }

    /// What stays below the chat: the window's home-indicator inset less the 24 pt the watch
    /// chat reaches into it, so the field ends just clear of the indicator on both screens.
    @MainActor private static var bottomKeep: CGFloat {
        let inset = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow)?.safeAreaInsets.bottom ?? 0
        return max(0, inset - 24)
    }

    /// Keep-awake is on in settings, and there is chat to read.
    private var keepsAwake: Bool {
        model.settingsStore.bool("chats_keep_screen_on", true) && !rooms.tabs.isEmpty && model.signedIn
    }

    /// The room on screen streams together or shares its chat, said the way the watch header says
    /// it (the partners' faces and the group's word, from the core's channel state), in the
    /// strip's empty left side. Tapping it lists the partners; a partner starts their stream.
    private var groupChip: AnyView? {
        guard let group = rooms.activeGroup else { return nil }
        return AnyView(
            Button { groupShown = true } label: {
                // The word gives way first when the side is narrow; the faces stay.
                ViewThatFits(in: .horizontal) {
                    GroupChipFace(group: group, word: true)
                    GroupChipFace(group: group, word: false)
                }
                .frame(height: 40)
                .snGlass(Capsule())
                .frame(height: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(group.label)
        )
    }

    /// The strip's own parts: the same 40 pt glass as the rooms capsule, quieter than the room it
    /// serves (secondary text, no red; the room's face already says live). Each keeps a 44 pt touch
    /// area around its 40 pt shape, so the row shares one top and bottom line.
    @ViewBuilder
    private var controls: some View {
        HStack(spacing: 6) {
            if let tab = rooms.active, tab.live == true, !tab.pinnedToStream {
                Button {
                    model.startWatching(Self.stream(tab))
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: "play.fill").font(.system(size: 11, weight: .semibold))
                            .offset(x: 0.5)
                        if let viewers = tab.viewerCount {
                            Text(viewers.formatted(.number.notation(.compactName)))
                                .font(.sn(12, 500))
                                .monospacedDigit()
                        }
                    }
                    .foregroundStyle(SN.textSecondary)
                    .padding(.horizontal, 12)
                    // At least the circles' width, so the row keeps its spacing between a live
                    // room and an offline one; a viewer count still widens it.
                    .frame(minWidth: 40, minHeight: 40, maxHeight: 40)
                    .snGlass(Capsule())
                    .frame(minWidth: 44, minHeight: 44, maxHeight: 44)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Watch \(tab.label)")
            } else if let tab = rooms.active, tab.live != true, tab.provider == "twitch", !tab.pinnedToStream {
                // An offline room's way in: its channel page (latest broadcast, clips, schedule),
                // the same room a search opens.
                Button {
                    model.startWatching(Self.stream(tab))
                } label: {
                    Image(systemName: "play.rectangle")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(SN.textSecondary)
                        .frame(width: 40, height: 40)
                        .snGlass(Circle())
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Open \(tab.label)'s channel")
            }
            Button {
                rooms.addOpen = true
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(SN.textSecondary)
                    .frame(width: 40, height: 40)
                    .snGlass(Circle())
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Add a chat")
        }
    }

    /// The room on screen as the player's start reads it; the core fills what a tab does not know.
    /// A room not known live opens as the offline room (the core starts the stream if it is live
    /// after all).
    static func stream(_ tab: ChatTabInfo) -> TwitchStream {
        TwitchStream(id: tab.channel, userLogin: tab.login, userName: tab.label, title: "",
                     viewerCount: tab.viewerCount ?? 0, gameName: "", userID: tab.channelID,
                     profileImageURL: tab.avatar, isLive: tab.live == true, provider: tab.provider, key: tab.channel)
    }
}

/// Chats' way out: a back arrow at the left edge of the row, where people look for "back"
/// (Brandon's call). The room pills keep their own X for closing a room, so the row has one X.
/// Leaving Chats keeps its chats; the grabber and a slide down still work.
struct ChatsMinimizeButton: View {
    let action: () -> Void

    var body: some View {
        // The strip's own 40 pt glass circle, as + and play beside the rooms.
        Button(action: action) {
            Image(systemName: "chevron.left")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(SN.textSecondary)
                // A chevron's weight sits on its point side; nudged so it reads centered.
                .offset(x: -1)
                .frame(width: 40, height: 40)
                .snGlass(Circle())
                .frame(width: 44, height: 44)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        // Pressed while typing it still leaves Chats on that tap; the keyboard goes with it.
        .tapsThroughKeyboard()
        .accessibilityLabel("Back")
    }
}

/// The grabber: a 40 pt pill in a full-width band that reaches from the top of the screen
/// (through the status bar's inset) down to the rooms, so it is easy to catch. It takes only its
/// 16 pt row in the layout. Held (pressed, or while the screen follows a slide), the pill widens
/// and brightens.
private struct GrabberBand: ButtonStyle {
    var held: Bool
    /// How far above its row the band reaches: the status bar's inset.
    var reach: CGFloat

    func makeBody(configuration: Configuration) -> some View {
        let on = held || configuration.isPressed
        Capsule()
            .fill(on ? SN.textSecondary : SN.textMuted.opacity(0.6))
            .frame(width: on ? 48 : 40, height: 5)
            .padding(.top, 6)
            .frame(maxWidth: .infinity, minHeight: 16, maxHeight: 16, alignment: .top)
            // At least 44 pt to catch, wherever the inset leaves it.
            .padding(.top, max(reach, 28))
            .contentShape(Rectangle())
            .padding(.top, -max(reach, 28))
            .animation(.easeOut(duration: 0.15), value: on)
    }
}

/// Holds the screen on until `idle` seconds pass with no touch. Touches are watched at the window,
/// by a recognizer that never claims one, so scrolling and taps behave exactly as without it.
@Observable @MainActor
final class AwakeTimer {
    @ObservationIgnored private var timer: Task<Void, Never>?
    @ObservationIgnored private var on = false
    @ObservationIgnored private var idle = 1800
    @ObservationIgnored private var watcher: TouchWatcher?

    func start(on: Bool, idle: Int) {
        self.on = on
        self.idle = max(60, idle)
        watch(on)
        touched()
    }

    private func watch(_ on: Bool) {
        if !on {
            watcher.map { $0.view?.removeGestureRecognizer($0) }
            watcher = nil
            return
        }
        guard watcher == nil else { return }
        let window = UIApplication.shared.connectedScenes
            .compactMap { ($0 as? UIWindowScene)?.keyWindow }
            .first
        guard let window else { return }
        let w = TouchWatcher { [weak self] in self?.touched() }
        window.addGestureRecognizer(w)
        watcher = w
    }

    func touched() {
        timer?.cancel()
        guard on else {
            ScreenAwake.hold("chats", false)
            return
        }
        ScreenAwake.hold("chats", true)
        let idle = idle
        timer = Task {
            try? await Task.sleep(for: .seconds(idle))
            guard !Task.isCancelled else { return }
            ScreenAwake.hold("chats", false)
        }
    }

    func stop() {
        on = false
        watch(false)
        timer?.cancel()
        ScreenAwake.hold("chats", false)
    }
}

/// Reports every touch that begins anywhere in its window, then steps aside at once: it fails
/// immediately, cancels nothing and delays nothing.
private final class TouchWatcher: UIGestureRecognizer {
    private let onTouch: () -> Void

    init(onTouch: @escaping () -> Void) {
        self.onTouch = onTouch
        super.init(target: nil, action: nil)
        cancelsTouchesInView = false
        delaysTouchesBegan = false
        delaysTouchesEnded = false
    }

    override func touchesBegan(_ touches: Set<UITouch>, with event: UIEvent) {
        onTouch()
        state = .failed
    }
}

extension AppModel {
    /// Opens a channel's chat on its own: it joins your chats and the Chats tab shows it.
    func openChat(_ stream: TwitchStream) {
        chat.add(ChatRoomRow(followed: stream))
        nav.tab = .chats
    }
}

/// The Chats strip's group chip inside its glass: a chat mark for a Shared Chat, the partners'
/// faces, and the group's word in secondary text, the strip's 40 pt scale.
private struct GroupChipFace: View {
    let group: ChannelGroup
    let word: Bool

    var body: some View {
        HStack(spacing: 6) {
            if !group.together {
                Image(systemName: "bubble.left.and.bubble.right.fill").font(.system(size: 12))
            }
            HStack(spacing: -5) {
                ForEach(group.others.prefix(3), id: \.userID) { member in
                    AsyncImage(url: member.avatarURL.flatMap(URL.init(string:))) { image in
                        image.resizable().scaledToFill()
                    } placeholder: {
                        SN.surface
                    }
                    .frame(width: 18, height: 18)
                    .clipShape(Circle())
                    .overlay(Circle().stroke(SN.background, lineWidth: 1.5))
                }
            }
            if word {
                Text(group.together ? "Together" : "Shared chat")
                    .font(.sn(12, 500))
                    .lineLimit(1)
            }
        }
        .foregroundStyle(SN.textSecondary)
        .padding(.leading, 10)
        .padding(.trailing, word ? 12 : 10)
        .fixedSize()
        .accessibilityHidden(true)
    }
}
