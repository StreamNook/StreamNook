import SNKit
import SwiftUI

/// The watch layer over the tabs: full (portrait: the player under the status bar with chat below
/// it; landscape: the video edge to edge, immersive, chat floating over it when asked for) or
/// the mini box floating over the tabs while browsing. In portrait a downward drag on the player
/// shrinks the whole layer toward the mini box under the finger. One player view serves every
/// mode, so rotating, shrinking and expanding never rebuild it.
struct WatchLayer: View {
    let model: AppModel
    /// The screen's safe area, read where it is respected (the layer itself draws past it).
    let insets: EdgeInsets
    @State private var shrink: CGFloat = 0
    @State private var dragging = false
    @State private var miniDrag: CGSize = .zero
    @State private var controlsShown = true
    @State private var controlsTouch = 0
    @Environment(\.snStill) private var still

    private var nav: Navigator { model.nav }
    private var watch: Watch { model.watch }

    static let miniSize = CGSize(width: 200, height: 112.5)
    /// How far the portrait chat reaches into the bottom safe area. With the composer's own 10 pt
    /// below it, the field ends 20 pt above the screen edge, just clear of the home indicator.
    static let indicatorOverlap: CGFloat = 24

    var body: some View {
        GeometryReader { box in
            let screen = box.size
            let landscape = screen.width > screen.height
            let mini = nav.playerMode == .mini
            let miniRect = Self.miniRect(screen: screen, insets: insets, origin: nav.miniOrigin, drag: miniDrag)
            let progress: CGFloat = mini ? 1 : (landscape ? 0 : shrink)
            let layer = Self.lerp(CGRect(origin: .zero, size: screen), miniRect, progress)
            let band = Self.band(layer: layer, topInset: landscape ? 0 : insets.top, progress: progress,
                                 landscape: landscape && !mini)
            let showChatBelow = !landscape && !mini
            let overlayWidth = min(max(340, 240), screen.width / 2)

            ZStack(alignment: .topLeading) {
                (landscape && !mini ? Color.black : SN.background)
                    .frame(width: layer.width, height: layer.height)
                    .offset(x: layer.minX, y: layer.minY)

                PlayerView(player: watch.player, pip: watch.pip)
                    .frame(width: band.width, height: band.height)
                    .contentShape(Rectangle())
                    .onTapGesture { mini ? expand() : toggleControls() }
                    .gesture(mini ? nil : shrinkGesture(screen: screen, landscape: landscape))
                    .overlay {
                        if !mini {
                            PlayerChrome(model: model, landscape: landscape, insets: landscape ? insets : EdgeInsets(),
                                         chatWidth: nav.landscapeChat ? overlayWidth + insets.trailing : 0,
                                         shown: controlsShown && !dragging, onMinimize: minimize, onTouch: touchedControls)
                        }
                    }
                    .offset(x: band.minX, y: band.minY)

                if !mini {
                    if !landscape {
                        BandRim()
                            .frame(width: band.width)
                            .offset(x: band.minX, y: band.maxY)
                    }
                }

                // One chat for every mode, so rotating moves the same list instead of building a
                // second one: below the player in portrait, over the video's trailing side in
                // landscape (read-only, when asked for), hidden but still reading while mini.
                // Portrait chat runs down into the home-indicator inset, keeping only the strip
                // the indicator needs: stopping at the safe-area line left a black band under the
                // composer.
                let overlayChat = landscape && !mini
                let chatShown = showChatBelow || (overlayChat && nav.landscapeChat)
                let bottomKeep = max(0, insets.bottom - Self.indicatorOverlap)
                ChatArea(model: model, overlay: overlayChat, outerInset: overlayChat ? insets.trailing : 0)
                    .frame(width: overlayChat ? overlayWidth + insets.trailing : layer.width,
                           height: overlayChat ? screen.height : max(0, layer.maxY - band.maxY - bottomKeep * (1 - progress)))
                    .offset(x: overlayChat ? screen.width - overlayWidth - insets.trailing : layer.minX,
                            y: overlayChat ? 0 : band.maxY)
                    .opacity(chatShown ? 1 : 0)
                    .animation(SNMotion.run(.easeOut(duration: 0.2)), value: nav.landscapeChat)
                    .allowsHitTesting(chatShown && !dragging)

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
            .gesture(mini ? miniGesture(screen: screen, insets: insets, rect: miniRect) : nil)
            .onChange(of: nav.playing?.id) { _, _ in touchedControls() }
            .onChange(of: nav.controlsPing) { _, _ in touchedControls() }
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

    // MARK: Gestures

    /// Down and mostly vertical, past 8 points: the layer shrinks toward the mini box with the
    /// finger. Let go past 45 percent, or flicked, it becomes the mini box; otherwise it springs
    /// back.
    private func shrinkGesture(screen: CGSize, landscape: Bool) -> some Gesture {
        DragGesture(minimumDistance: 8, coordinateSpace: .global)
            .onChanged { value in
                guard !landscape else { return }
                let dy = value.translation.height
                let dx = value.translation.width
                let travel = min(max(screen.height * 0.28, 120), 280)
                dragging = true
                shrink = dy > 0 && dy > abs(dx) * 1.2 ? min(1, dy / travel) : 0
            }
            .onEnded { value in
                guard !landscape else { return }
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
            .onChanged { value in miniDrag = value.translation }
            .onEnded { _ in
                let left = rect.midX < screen.width / 2
                let x = left ? insets.leading + 12 : screen.width - insets.trailing - 12 - Self.miniSize.width
                withAnimation(SNMotion.run(SNMotion.settleBack)) {
                    nav.miniOrigin = CGPoint(x: x, y: rect.minY)
                    miniDrag = .zero
                }
            }
    }

    // MARK: Geometry

    /// The mini box: where the person left it (bottom-right above the tab bar at first), kept 12
    /// points clear of the safe area and the tab bar, worked out against the screen as it is now.
    static func miniRect(screen: CGSize, insets: EdgeInsets, origin: CGPoint?, drag: CGSize) -> CGRect {
        let size = miniSize
        let bottomClear = max(104, insets.bottom + 76)
        let minX = insets.leading + 12
        let maxX = screen.width - insets.trailing - 12 - size.width
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

/// Landscape chat's backdrop over the video: the theme background at 55 percent, a faint sheen at
/// the top and a hairline on the video edge. No blur: video moves under it.
private struct LandscapeBackdrop: View {
    var body: some View {
        ZStack {
            SN.background.opacity(0.55)
            LinearGradient(stops: [.init(color: .white.opacity(0.035), location: 0), .init(color: .clear, location: 0.25)],
                           startPoint: .top, endPoint: .bottom)
        }
        .overlay(alignment: .leading) { Color.white.opacity(0.1).frame(width: 1) }
        .shadow(color: .black.opacity(0.6), radius: 20)
    }
}

/// Chat in the watch layer: the room's chat panel when signed in (the overlay variant in
/// landscape: read-only, over the video); signed out, what signing in gets you.
private struct ChatArea: View {
    let model: AppModel
    let overlay: Bool
    let outerInset: CGFloat

    var body: some View {
        content
            .padding(.trailing, outerInset)
            .background { if overlay { LandscapeBackdrop() } }
            // Signing in while watching opens the stream's room.
            .task(id: model.signedIn) {
                if model.signedIn, let stream = model.watch.stream { model.chat.syncStream(stream) }
            }
    }

    @ViewBuilder
    private var content: some View {
        if model.signedIn {
            ChatRoomsPanel(rooms: model.chat, overlay: overlay, onProfile: { model.cosmetics.open($0) })
        } else {
            let readOnly = overlay
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
