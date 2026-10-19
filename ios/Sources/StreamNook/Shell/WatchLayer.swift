import SNKit
import SwiftUI

/// The watch layer over the tabs: full (portrait: the player under the status bar with chat below
/// it; landscape: the video edge to edge, immersive, chat over it or beside it when asked for) or
/// the mini box floating over the tabs while browsing. In portrait a downward drag on the player
/// shrinks the whole layer toward the mini box under the finger; in landscape a vertical swipe
/// turns brightness (left half of the picture) or volume (right half). One player view serves
/// every mode, so rotating, shrinking and expanding never rebuild it.
struct WatchLayer: View {
    let model: AppModel
    /// The screen's safe area, read where it is respected (the layer itself draws past it).
    let insets: EdgeInsets
    @State private var shrink: CGFloat = 0
    @State private var dragging = false
    @GestureState private var shrinkHeld = false
    @State private var miniDrag: CGSize = .zero
    @GestureState private var miniHeld = false
    @State private var controlsShown = true
    @State private var controlsTouch = 0
    /// The landscape column's width and background while a drag or the slider leads the saved
    /// setting; nil once the setting has caught up.
    @State private var liveWidth: Double?
    @State private var liveOpacity: Double?
    @State private var stripOpen = false
    @State private var stripTouch = 0
    @State private var dial: DialAxis?
    @State private var dialLevel: Double = 0
    @State private var dialStart: Double?
    @State private var dialShown = false
    @State private var dialRelease = 0
    @GestureState private var dialHeld = false
    @Environment(\.snStill) private var still

    private var nav: Navigator { model.nav }
    private var watch: Watch { model.watch }
    private var store: SettingsStore { model.settingsStore }

    static let miniSize = CGSize(width: 200, height: 112.5)
    /// The mini box keeps the pages' 16 pt gutter, so its edge lines up with the cards under it.
    static let gutter: CGFloat = 16
    /// How far the portrait chat reaches into the bottom safe area. With the composer's own 10 pt
    /// below it, the field ends 20 pt above the screen edge, just clear of the home indicator.
    static let indicatorOverlap: CGFloat = 24

    /// The window's own bottom inset (the home indicator's). The safe area the layer is handed
    /// grows to the keyboard's height while it is up; the window's never does.
    @MainActor private static var homeIndicatorInset: CGFloat {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows).first(where: \.isKeyWindow)?.safeAreaInsets.bottom ?? 0
    }

    private var chat: LandscapeChatLayout {
        LandscapeChatLayout(layout: store.string("fullscreen_chat.phone_layout", "overlay"),
                            side: store.string("fullscreen_chat.side", "auto"),
                            width: store.double("fullscreen_chat.width", 340),
                            opacity: liveOpacity ?? store.double("fullscreen_chat.opacity", 55))
    }

    var body: some View {
        GeometryReader { box in
            let screen = box.size
            let landscape = screen.width > screen.height
            let mini = nav.playerMode == .mini
            let full = !mini
            let miniRect = Self.miniRect(screen: screen, insets: insets, origin: nav.miniOrigin, drag: miniDrag,
                                         floor: nav.tab == .chats ? nav.chatsFloor : nil)
            let progress: CGFloat = mini ? 1 : (landscape ? 0 : shrink)
            let layer = Self.lerp(CGRect(origin: .zero, size: screen), miniRect, progress)
            let chat = chat
            let sideChat = landscape && full && nav.landscapeChat
            // The landscape column: the chat's width plus the safe inset on its outer edge (the
            // Dynamic Island side), so its text never sits under the cutout.
            let outer = chat.leading ? insets.leading : insets.trailing
            let columnWidth = CGFloat(chat.width(screenWidth: screen.width, live: liveWidth)) + outer
            let beside = sideChat && chat.beside
            let band = beside
                ? CGRect(x: chat.leading ? columnWidth : 0, y: 0, width: screen.width - columnWidth, height: screen.height)
                : Self.band(layer: layer, topInset: landscape ? 0 : insets.top, progress: progress,
                            landscape: landscape && full)
            let columnX = chat.leading ? 0 : screen.width - columnWidth

            ZStack(alignment: .topLeading) {
                (landscape && full ? Color.black : SN.background)
                    .frame(width: layer.width, height: layer.height)
                    .offset(x: layer.minX, y: layer.minY)

                PlayerView(player: watch.player, pip: watch.pip)
                    .frame(width: band.width, height: band.height)
                    .contentShape(Rectangle())
                    .onTapGesture { mini ? expand() : toggleControls() }
                    .gesture(mini ? nil : (landscape ? nil : shrinkGesture(screen: screen)))
                    .simultaneousGesture(landscape && full ? dialGesture(band: band) : nil)
                    .overlay {
                        // An offline room takes the band (its latest broadcast gives it back).
                        if watch.room != nil, !watch.roomVod {
                            OfflineRoomView(model: model, compact: mini)
                        } else if full {
                            PlayerChrome(model: model, landscape: landscape,
                                         insets: landscape ? chromeInsets(sideChat: sideChat, leading: chat.leading) : EdgeInsets(),
                                         chatWidth: sideChat && !beside ? columnWidth : 0, chatLeading: chat.leading,
                                         shown: controlsShown && !dragging, onMinimize: minimize, onTouch: touchedControls)
                        }
                    }
                    .overlay {
                        if let dial, dialShown {
                            DialReadout(axis: dial, level: dialLevel)
                                .id(dial)
                                .transition(.opacity.combined(with: .scale(scale: 0.94)))
                        }
                    }
                    .offset(x: band.minX, y: band.minY)

                if full, !landscape {
                    BandRim()
                        .frame(width: band.width)
                        .offset(x: band.minX, y: band.maxY)
                }

                // One chat for every mode, so rotating moves the same list instead of building a
                // second one: below the player in portrait, over or beside the video in landscape
                // when asked for, hidden but still reading while mini. Portrait chat runs down
                // into the home-indicator inset, keeping only the strip the indicator needs:
                // stopping at the safe-area line left a black band under the composer. With the
                // keyboard up the bottom inset is the keyboard's, and the chat stops at its top
                // edge in every layout (reaching past it put the field under the keys).
                let landscapeChat = landscape && full
                let chatShown = (full && !landscape) || sideChat
                let keyboardUp = insets.bottom > Self.homeIndicatorInset + 1
                let bottomKeep = max(0, insets.bottom - (keyboardUp ? 0 : Self.indicatorOverlap))
                ChatArea(model: model, readOnly: landscapeChat && !chat.beside,
                         backdrop: landscapeChat ? (chat.beside ? .solid : .tint(chat.opacity / 100)) : nil,
                         outerInset: landscapeChat ? outer : 0, leading: chat.leading)
                    .frame(width: landscapeChat ? columnWidth : layer.width,
                           height: landscapeChat ? max(0, screen.height - (keyboardUp ? insets.bottom : 0))
                                                 : max(0, layer.maxY - band.maxY - bottomKeep * (1 - progress)))
                    .offset(x: landscapeChat ? columnX : layer.minX, y: landscapeChat ? 0 : band.maxY)
                    .opacity(chatShown ? 1 : 0)
                    .animation(SNMotion.run(.easeOut(duration: 0.2)), value: nav.landscapeChat)
                    .allowsHitTesting(chatShown && !dragging)

                if sideChat {
                    let edge = chat.leading ? columnX + columnWidth : columnX
                    ChatWidthHandle(chat: chat, width: Double(columnWidth - outer), screenWidth: screen.width,
                                    onDrag: { liveWidth = $0 },
                                    onCommit: commitWidth,
                                    onTap: {
                                        guard !chat.beside else { return }
                                        withAnimation(SNMotion.run(.easeOut(duration: 0.16))) { stripOpen.toggle() }
                                        stripTouch += 1
                                    })
                        .offset(x: edge - 14, y: screen.height / 2 - 40)
                    if !chat.beside, stripOpen {
                        ChatBackgroundStrip(opacity: chat.opacity, onChange: { liveOpacity = $0 },
                                            onHide: hideLandscapeChat, onTouch: { stripTouch += 1 })
                            .frame(width: columnWidth - outer - 24)
                            .offset(x: columnX + (chat.leading ? outer : 0) + 12,
                                    y: screen.height - max(insets.bottom, 8) - 44)
                            .transition(.opacity.combined(with: .offset(y: 8)))
                    }
                }

                if mini {
                    MiniClose { model.stopWatching() }
                        .offset(x: miniRect.maxX - 56, y: miniRect.minY)
                }
            }
            .frame(width: screen.width, height: screen.height, alignment: .topLeading)
            .mask {
                RoundedRectangle(cornerRadius: SNRadius.mini * progress, style: .continuous)
                    .frame(width: layer.width, height: layer.height)
                    .offset(x: layer.minX, y: layer.minY)
                    .frame(width: screen.width, height: screen.height, alignment: .topLeading)
            }
            .background {
                if progress > 0 {
                    RoundedRectangle(cornerRadius: SNRadius.mini * progress, style: .continuous)
                        .fill(Color.black)
                        .frame(width: layer.width, height: layer.height)
                        .floatShadow()
                        .offset(x: layer.minX, y: layer.minY)
                        .frame(width: screen.width, height: screen.height, alignment: .topLeading)
                }
            }
            // The edge fades in as the layer shrinks, so a full-screen player shows none.
            .overlay {
                if progress > 0 {
                    RoundedRectangle(cornerRadius: SNRadius.mini * progress, style: .continuous)
                        .strokeBorder(SN.floatEdge.opacity(progress), lineWidth: 1)
                        .frame(width: layer.width, height: layer.height)
                        .offset(x: layer.minX, y: layer.minY)
                        .frame(width: screen.width, height: screen.height, alignment: .topLeading)
                        .allowsHitTesting(false)
                }
            }
            .gesture(mini ? miniGesture(screen: screen, insets: insets, rect: miniRect) : nil)
            .onChange(of: nav.playing?.userLogin) { _, _ in touchedControls() }
            .onChange(of: nav.controlsPing) { _, _ in touchedControls() }
            // The strip is open whenever the overlay appears, so the slider is found.
            .onChange(of: sideChat, initial: true) { _, on in
                stripOpen = on && !chat.beside
                if on { stripTouch += 1 }
            }
            // An OS-stolen gesture (an edge swipe, the notification shade, a call) unwinds with no
            // decision: no tap, no commit.
            .onChange(of: shrinkHeld) { _, held in
                guard !held, dragging else { return }
                dragging = false
                withAnimation(SNMotion.run(SNMotion.settleBack)) { shrink = 0 }
            }
            .onChange(of: miniHeld) { _, held in
                guard !held, miniDrag != .zero else { return }
                withAnimation(SNMotion.run(SNMotion.settleBack)) { miniDrag = .zero }
            }
            .onChange(of: dialHeld) { _, held in if !held, dialStart != nil { endDial() } }
            .onChange(of: store.double("fullscreen_chat.width", 340)) { _, saved in
                if let live = liveWidth, abs(live - saved) < 0.5 { liveWidth = nil }
            }
            .onChange(of: store.double("fullscreen_chat.opacity", 55)) { _, saved in
                if let live = liveOpacity, abs(live - saved) < 0.5 { liveOpacity = nil }
            }
        }
        .ignoresSafeArea()
        .task(id: controlsTouch) {
            guard controlsShown else { return }
            try? await Task.sleep(for: .seconds(3))
            guard !Task.isCancelled else { return }
            #if DEBUG
            if Drive.pinsControls { return }
            #endif
            withAnimation(SNMotion.run(.easeOut(duration: 0.2))) { controlsShown = false }
        }
        // The strip closes itself 4 s after the last touch.
        .task(id: stripTouch) {
            guard stripOpen else { return }
            try? await Task.sleep(for: .seconds(4))
            guard !Task.isCancelled else { return }
            withAnimation(SNMotion.run(.easeOut(duration: 0.16))) { stripOpen = false }
        }
        // The background is saved 350 ms after the slider stops.
        .task(id: liveOpacity) {
            guard let value = liveOpacity else { return }
            try? await Task.sleep(for: .milliseconds(350))
            guard !Task.isCancelled else { return }
            await store.set("fullscreen_chat.opacity", value)
        }
        // The readout lingers 650 ms after the finger lifts.
        .task(id: dialRelease) {
            guard dialRelease > 0 else { return }
            try? await Task.sleep(for: .milliseconds(650))
            guard !Task.isCancelled, dialStart == nil else { return }
            withAnimation(SNMotion.run(.easeOut(duration: 0.16))) { dialShown = false }
        }
    }

    /// The controls' insets in landscape: the safe area, except on the side the chat column
    /// already covers (its width includes that side's inset).
    private func chromeInsets(sideChat: Bool, leading: Bool) -> EdgeInsets {
        guard sideChat else { return insets }
        var adjusted = insets
        if leading { adjusted.leading = 0 } else { adjusted.trailing = 0 }
        return adjusted
    }

    // MARK: Modes

    private func minimize() {
        withAnimation(SNMotion.run(SNMotion.settleBack)) {
            nav.playerMode = .mini
            shrink = 0
        }
    }

    private func expand() {
        withAnimation(SNMotion.run(SNMotion.settleBack)) { nav.playerMode = .full }
        touchedControls()
    }

    private func toggleControls() {
        withAnimation(SNMotion.run(.easeOut(duration: 0.2))) { controlsShown.toggle() }
        if controlsShown { controlsTouch += 1 }
    }

    private func touchedControls() {
        withAnimation(SNMotion.run(.easeOut(duration: 0.2))) { controlsShown = true }
        controlsTouch += 1
    }

    private func hideLandscapeChat() {
        withAnimation(SNMotion.run(.easeOut(duration: 0.18))) { nav.landscapeChat = false }
    }

    private func commitWidth(_ width: Double) {
        liveWidth = width
        Task { await store.set("fullscreen_chat.width", width) }
    }

    // MARK: Gestures

    /// Down and mostly vertical, past 8 points: the layer shrinks toward the mini box with the
    /// finger. Let go past 45 percent, or flicked, it becomes the mini box; otherwise it springs
    /// back.
    private func shrinkGesture(screen: CGSize) -> some Gesture {
        DragGesture(minimumDistance: 8, coordinateSpace: .global)
            .updating($shrinkHeld) { _, held, _ in held = true }
            .onChanged { value in
                let dy = value.translation.height
                let dx = value.translation.width
                let travel = min(max(screen.height * 0.28, 120), 280)
                dragging = true
                shrink = dy > 0 && dy > abs(dx) * 1.2 ? min(1, dy / travel) : 0
            }
            .onEnded { value in
                dragging = false
                if shrink >= 0.45 || value.velocity.height >= 600 {
                    minimize()
                } else {
                    withAnimation(SNMotion.run(SNMotion.settleBack)) { shrink = 0 }
                }
            }
    }

    /// The mini box follows the finger, then snaps to the nearer side, keeping its height.
    private func miniGesture(screen: CGSize, insets: EdgeInsets, rect: CGRect) -> some Gesture {
        DragGesture(minimumDistance: 8, coordinateSpace: .global)
            .updating($miniHeld) { _, held, _ in held = true }
            .onChanged { value in miniDrag = value.translation }
            .onEnded { _ in
                let left = rect.midX < screen.width / 2
                let x = left ? insets.leading + Self.gutter : screen.width - insets.trailing - Self.gutter - Self.miniSize.width
                withAnimation(SNMotion.run(SNMotion.settleBack)) {
                    nav.miniOrigin = CGPoint(x: x, y: rect.minY)
                    miniDrag = .zero
                }
            }
    }

    /// Landscape only: a vertical swipe over the left half of the picture turns the screen's
    /// brightness, over the right half the stream's volume. The swing is relative to the level
    /// when the finger landed; a tenth crossed gives one light step.
    private func dialGesture(band: CGRect) -> some Gesture {
        DragGesture(minimumDistance: 8, coordinateSpace: .local)
            .updating($dialHeld) { _, held, _ in held = true }
            .onChanged { value in
                if dialStart == nil {
                    // Mostly vertical, or it is not a dial.
                    guard abs(value.translation.height) > abs(value.translation.width) else { return }
                    let axis: DialAxis = value.startLocation.x < band.width / 2 ? .brightness : .volume
                    dial = axis
                    dialStart = axis == .brightness ? ScreenBrightness.level : Double(watch.player.volume)
                    dialLevel = dialStart ?? 0
                    withAnimation(SNMotion.run(.spring(response: 0.25, dampingFraction: 0.8))) { dialShown = true }
                }
                guard let start = dialStart, let dial else { return }
                let next = Dial.level(start: start, dy: value.translation.height, height: band.height)
                guard Dial.hundredth(next) != Dial.hundredth(dialLevel) else { return }
                if Dial.tenth(next) != Dial.tenth(dialLevel) { Haptics.step() }
                dialLevel = next
                switch dial {
                case .brightness: ScreenBrightness.set(next)
                case .volume: watch.player.volume = Float(next)
                }
            }
            .onEnded { _ in endDial() }
    }

    private func endDial() {
        guard dialStart != nil else { return }
        dialStart = nil
        if let dial { AccessibilityNotification.Announcement("\(dial.label) \(Dial.hundredth(dialLevel))%").post() }
        dialRelease += 1
    }

    // MARK: Geometry

    /// The resting mini box's gap over what is under it: the default rest leaves this much over
    /// the tab bar's top, and on Chats (no tab bar) it keeps the same gap over the composer.
    static let restGap: CGFloat = 22

    /// The mini box: where the person left it (bottom-right above the tab bar at first), kept 12
    /// points clear of the safe area and the tab bar, worked out against the screen as it is now.
    /// `floor` is where the Chats composer starts, when Chats is in front with a chat open.
    static func miniRect(screen: CGSize, insets: EdgeInsets, origin: CGPoint?, drag: CGSize,
                         floor: CGFloat? = nil) -> CGRect {
        let size = miniSize
        let bottomClear = floor.map { screen.height - $0 + restGap } ?? max(104, insets.bottom + 76)
        let minX = insets.leading + gutter
        let maxX = screen.width - insets.trailing - gutter - size.width
        let minY = insets.top + 12
        let maxY = screen.height - bottomClear - size.height
        let start = origin ?? CGPoint(x: maxX, y: maxY)
        let x = maxX < minX ? minX : min(max(start.x + drag.width, minX), maxX)
        let y = maxY < minY ? minY : min(max(start.y + drag.height, minY), maxY)
        return CGRect(origin: CGPoint(x: x, y: y), size: size)
    }

    /// The player band inside the layer: 16:9 across its width under the status bar, or the
    /// whole screen in landscape.
    static func band(layer: CGRect, topInset: CGFloat, progress: CGFloat, landscape: Bool) -> CGRect {
        if landscape { return layer }
        let top = layer.minY + topInset * (1 - progress)
        return CGRect(x: layer.minX, y: top, width: layer.width, height: layer.width * 9 / 16)
    }

    static func lerp(_ a: CGRect, _ b: CGRect, _ t: CGFloat) -> CGRect {
        CGRect(x: a.minX + (b.minX - a.minX) * t, y: a.minY + (b.minY - a.minY) * t,
               width: a.width + (b.width - a.width) * t, height: a.height + (b.height - a.height) * t)
    }
}

/// The accent rim under the portrait player and its short bloom, dying within about 24 points.
private struct BandRim: View {
    var body: some View {
        Rectangle()
            .fill(SN.accent.opacity(0.14))
            .frame(height: 1)
            .shadow(color: SN.accent.opacity(0.10), radius: 4, y: 3)
            .shadow(color: SN.accent.opacity(0.06), radius: 9, y: 8)
            .allowsHitTesting(false)
    }
}

/// The mini box's close: the whole top-right 56 point corner, marked by an X in a dark circle.
private struct MiniClose: View {
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: Symbol.close)
                .font(.system(size: 14, weight: .bold))
                .foregroundStyle(.white)
                .frame(width: 32, height: 32)
                .background(Circle().fill(Color.black.opacity(0.6)))
                .frame(width: 56, height: 56)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Close stream")
    }
}

/// What sits behind landscape chat: a tint of the theme background over the video (no blur:
/// video moves under it), or the plain background beside it.
enum ChatBackdrop: Equatable {
    case tint(Double)
    case solid
}

/// Landscape chat's backdrop over the video: the theme background at the chosen percent, a faint
/// sheen at the top and a hairline on the video edge.
private struct LandscapeBackdrop: View {
    let opacity: Double
    let leading: Bool

    var body: some View {
        ZStack {
            SN.background.opacity(opacity)
            LinearGradient(stops: [.init(color: .white.opacity(0.035), location: 0), .init(color: .clear, location: 0.25)],
                           startPoint: .top, endPoint: .bottom)
        }
        .overlay(alignment: leading ? .trailing : .leading) { Color.white.opacity(0.1).frame(width: 1) }
        .shadow(color: .black.opacity(0.6), radius: 20)
    }
}

/// Chat in the watch layer: the room's chat panel when signed in (read-only over the video in
/// landscape); signed out, what signing in gets you.
private struct ChatArea: View {
    let model: AppModel
    let readOnly: Bool
    let backdrop: ChatBackdrop?
    let outerInset: CGFloat
    let leading: Bool

    var body: some View {
        content
            .padding(leading ? .leading : .trailing, outerInset)
            .background {
                switch backdrop {
                case .tint(let opacity): LandscapeBackdrop(opacity: opacity, leading: leading)
                case .solid: SN.background
                case nil: EmptyView()
                }
            }
            // Signing in while watching opens the stream's room.
            .task(id: model.signedIn) {
                if model.signedIn, let stream = model.watch.stream { model.chat.syncStream(stream) }
            }
    }

    @ViewBuilder
    private var content: some View {
        if model.signedIn {
            // The drop being earned shows only over the stream's own room, never over the video.
            ChatRoomsPanel(rooms: model.chat, overlay: readOnly,
                           replay: model.replay,
                           replayShown: Binding(get: { model.replayShown }, set: { model.replayShown = $0 }),
                           replayAt: model.watch.playingVod == nil ? nil : PlayerText.clock(model.watch.vodPosition),
                           drop: .watched,
                           onProfile: { model.cosmetics.open($0) })
                // The reader stays mounted whichever room or mode is on screen, so its reading
                // never stops; the chat stack draws what it reads.
                .background {
                    WatchDropReader(model: model, drops: .watched, visible: !readOnly && model.chat.viewingStream)
                }
        } else {
            VStack(spacing: 14) {
                Text("Sign in with Twitch to read and send chat.")
                    .font(.snBody)
                    .foregroundStyle(SN.textMuted)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 280)
                if !readOnly { SignInButton(model: model).frame(maxWidth: 280) }
            }
            .padding(24)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}
