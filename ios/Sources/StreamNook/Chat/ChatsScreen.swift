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

    private var rooms: ChatRooms { model.chat }

    var body: some View {
        Group {
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
                // No chat open: the tab is the picker itself.
                ChatPicker(rooms: rooms)
                    .padding(.top, 8)
            } else {
                VStack(spacing: 0) {
                    ChatTabStrip(rooms: rooms, trailing: AnyView(controls))
                    if let session = rooms.activeSession {
                        ChatPanel(session: session, rooms: rooms, onProfile: { model.cosmetics.open($0) })
                            .id(ObjectIdentifier(session))
                    } else {
                        Color.clear
                    }
                }
            }
        }
        .background(SN.background)
        .toolbar(.hidden, for: .navigationBar)
        .sheet(isPresented: Binding(get: { rooms.addOpen }, set: { rooms.addOpen = $0 })) {
            AddChatSheet(rooms: rooms)
        }
        // The first look joins the saved chats.
        .task(id: model.signedIn) {
            if model.signedIn, rooms.state.connection != "connected" { rooms.connect() }
        }
        .onAppear { awake.start(on: keepsAwake, idle: rooms.state.keepAwakeIdleSecs) }
        .onDisappear { awake.stop() }
        .onChange(of: keepsAwake) { _, on in awake.start(on: on, idle: rooms.state.keepAwakeIdleSecs) }
    }

    /// Keep-awake is on in settings, and there is chat to read.
    private var keepsAwake: Bool {
        model.settingsStore.bool("chats_keep_screen_on", true) && !rooms.tabs.isEmpty && model.signedIn
    }

    @ViewBuilder
    private var controls: some View {
        HStack(spacing: 6) {
            if let tab = rooms.active, tab.live == true, !tab.pinnedToStream {
                Button {
                    model.startWatching(Self.stream(tab))
                } label: {
                    HStack(spacing: 5) {
                        Circle().fill(SN.live).frame(width: 6, height: 6)
                        if let viewers = tab.viewerCount {
                            Text(viewers.formatted(.number.notation(.compactName)))
                                .font(.sn(12, 600))
                                .monospacedDigit()
                        }
                        Image(systemName: "play.fill").font(.system(size: 11, weight: .semibold))
                    }
                    .foregroundStyle(SN.textPrimary)
                    .padding(.leading, 10)
                    .padding(.trailing, 12)
                    .frame(height: 32)
                    .background(Capsule().fill(SN.live.opacity(0.15)))
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Watch \(tab.label)")
            }
            Button {
                rooms.addOpen = true
            } label: {
                Image(systemName: "plus")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(SN.textSecondary)
                    .frame(width: 32, height: 32)
                    .snGlass(Circle())
                    .contentShape(Circle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Add a chat")
        }
    }

    /// The room on screen as the player's start reads it; the core fills what a tab does not know.
    static func stream(_ tab: ChatTabInfo) -> TwitchStream {
        TwitchStream(id: tab.channel, userLogin: tab.login, userName: tab.label, title: "",
                     viewerCount: tab.viewerCount ?? 0, gameName: "", userID: tab.channelID,
                     profileImageURL: tab.avatar, isLive: true, provider: tab.provider, key: tab.channel)
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
