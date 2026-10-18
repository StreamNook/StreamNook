import SNKit
import SwiftUI

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
                empty
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
        // Any touch keeps the screen on for another idle span.
        .simultaneousGesture(TapGesture().onEnded { awake.touched() })
        .simultaneousGesture(DragGesture(minimumDistance: 4).onChanged { _ in awake.touched() })
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

    private var empty: some View {
        VStack(spacing: 0) {
            Image(systemName: Symbol.chats)
                .font(.system(size: 20))
                .foregroundStyle(SN.textSecondary)
                .frame(width: 48, height: 48)
                .snGlass(Circle())
                .padding(.bottom, 12)
            Text("No chats open")
                .font(.sn(16, 600))
                .foregroundStyle(SN.textPrimary)
            Text("Open a channel's chat to read it here without watching. Your chats stay open between launches.")
                .font(.sn(13))
                .foregroundStyle(SN.textMuted)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 280)
                .padding(.top, 4)
                .padding(.bottom, 16)
            Button {
                rooms.addOpen = true
            } label: {
                Label("Add a chat", systemImage: "plus")
                    .font(.sn(14, 500))
                    .foregroundStyle(SN.textPrimary)
                    .padding(.horizontal, 16)
                    .frame(height: 40)
                    .snGlass(Capsule())
            }
            .buttonStyle(.plain)
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    /// The room on screen as the player's start reads it; the core fills what a tab does not know.
    static func stream(_ tab: ChatTabInfo) -> TwitchStream {
        TwitchStream(id: tab.channel, userLogin: tab.login, userName: tab.label, title: "",
                     viewerCount: tab.viewerCount ?? 0, gameName: "", userID: tab.channelID,
                     profileImageURL: tab.avatar, isLive: true, provider: tab.provider, key: tab.channel)
    }
}

/// Holds the screen on until `idle` seconds pass with no touch.
@Observable @MainActor
final class AwakeTimer {
    @ObservationIgnored private var timer: Task<Void, Never>?
    @ObservationIgnored private var on = false
    @ObservationIgnored private var idle = 1800

    func start(on: Bool, idle: Int) {
        self.on = on
        self.idle = max(60, idle)
        touched()
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
        timer?.cancel()
        ScreenAwake.hold("chats", false)
    }
}

extension AppModel {
    /// Opens a channel's chat on its own: it joins your chats and the Chats tab shows it.
    func openChat(_ stream: TwitchStream) {
        chat.add(ChatRoomRow(followed: stream))
        nav.tab = .chats
    }
}
