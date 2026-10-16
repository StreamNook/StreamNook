import SNKit
import SwiftUI

/// What sits over the full player: a quiet loader until the first frame and the failure if there
/// is one; on a tap, the controls. The top carries the stream (avatar, name, viewers, uptime,
/// title, category), the center plays and pauses, and the bottom bar holds LIVE and the buttons
/// (mute, share, minimize, restart, quality, and chat on its side). The controls fade after
/// three seconds without a touch; play/pause, mute and restart keep them up.
struct PlayerChrome: View {
    let model: AppModel
    let landscape: Bool
    let insets: EdgeInsets
    /// Room the landscape chat column takes over the picture, so no control sits under it.
    let chatWidth: CGFloat
    /// The column is on the leading side.
    var chatLeading = false
    let shown: Bool
    let onMinimize: () -> Void
    let onTouch: () -> Void
    @State private var qualityShown = false
    @State private var togetherShown = false

    private var watch: Watch { model.watch }
    private var stream: TwitchStream? { model.nav.playing }

    var body: some View {
        ZStack {
            // The loading mark, sized from the band (18 percent of its height, 22 to 60 points),
            // fades out with the first frame and is back at once when loading returns.
            GeometryReader { band in
                if watch.error == nil, watch.firstPlayMS == nil {
                    PenroseMark(width: min(max(band.size.height * 0.18, 22), 60))
                        .frame(width: band.size.width, height: band.size.height)
                        .transition(.asymmetric(insertion: .identity, removal: .opacity))
                }
            }
            .animation(SNMotion.run(.easeOut(duration: 0.3)), value: watch.firstPlayMS == nil)
            .allowsHitTesting(false)
            if let error = watch.error {
                ErrorState(text: error) {
                    Task { await model.restartStream() }
                }
            }
            if shown {
                Scrim().transition(.opacity)
                VStack(spacing: 0) {
                    if let stream { header(stream) }
                    Spacer(minLength: 0)
                    bottomBar
                }
                .padding(.top, landscape ? insets.top + 8 : 8)
                .padding(.bottom, landscape ? max(insets.bottom, 8) : 4)
                .padding(.leading, (landscape ? insets.leading : 0) + 12 + (chatLeading ? chatWidth : 0))
                .padding(.trailing, (landscape ? insets.trailing : 0) + 12 + (chatLeading ? 0 : chatWidth))
                .transition(.opacity)
                if watch.error == nil {
                    playPause
                        .padding(chatLeading ? .leading : .trailing, chatWidth)
                        .transition(.opacity)
                }
            }
            #if DEBUG
            if Drive.showsEdge {
                Text(edgeText)
                    .font(.caption.monospacedDigit())
                    .padding(.horizontal, 10)
                    .padding(.vertical, 5)
                    .glassEffect()
                    .padding(10)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .bottomLeading)
                    .allowsHitTesting(false)
            }
            #endif
        }
        .clipped()
        .sheet(isPresented: $qualityShown) { QualitySheet(model: model) }
        .sheet(isPresented: $togetherShown) {
            if let stream, let group = group(stream) {
                TogetherSheet(group: group, playing: stream.userLogin) { member in
                    model.startWatching(TwitchStream(id: "", userLogin: member.login, userName: member.displayName, title: "",
                                                     viewerCount: member.viewerCount ?? 0, gameName: "",
                                                     userID: member.userID, profileImageURL: member.avatarURL))
                }
            }
        }
        .onChange(of: model.nav.qualityPing) { _, _ in qualityShown = true }
    }

    // MARK: Header

    private func header(_ stream: TwitchStream) -> some View {
        let live = watch.live
        let title = live?.title ?? stream.title
        let category = live?.gameName ?? stream.gameName
        // The avatar anchors a column: the name, the title and the category all start at the
        // name's edge, so the three read as one block about one stream. The live red is the
        // avatar's ring alone; the counts stay white so they never out-pull the name.
        return HStack(alignment: .top, spacing: 10) {
            ChannelAvatar(url: stream.profileImageURL.flatMap(URL.init(string:)), name: stream.userName)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    HStack(spacing: 4) {
                        Text(stream.userName)
                            .font(.sn(14, 600))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                        if stream.isPartner { PartnerMark(size: 12) }
                        AlertBell(store: model.settingsStore, login: stream.userLogin, name: stream.userName)
                    }
                    Spacer(minLength: 8)
                    HStack(spacing: 10) {
                        if let group = group(stream) {
                            Button { togetherShown = true } label: { TogetherTag(group: group, ink: .white) }
                                .buttonStyle(.plain)
                        }
                        let viewers = live?.viewerCount ?? stream.viewerCount
                        HStack(spacing: 4) {
                            Image(systemName: Symbol.viewers).font(.system(size: 10, weight: .medium))
                            Text(viewers.formatted()).monospacedDigit()
                        }
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel("\(viewers.formatted()) viewers")
                        TimelineView(.periodic(from: .now, by: 30)) { context in
                            Text(PlayerText.uptime(startedAt: live?.startedAt ?? stream.startedAt, now: context.date))
                                .monospacedDigit()
                        }
                    }
                    .font(.sn(12, 500))
                    .foregroundStyle(.white.opacity(0.8))
                    .fixedSize()
                }
                Text(title)
                    .font(.sn(13))
                    .foregroundStyle(.white.opacity(0.85))
                    .lineLimit(1)
                if !category.isEmpty {
                    Text(category)
                        .font(.sn(12))
                        .foregroundStyle(.white.opacity(0.6))
                        .lineLimit(1)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The channel's group: Shared Viewership from the core's live collab section, else what the
    /// home lists know (a Shared Chat session included).
    private func group(_ stream: TwitchStream) -> ChannelGroup? {
        if let group = watch.group { return group }
        guard let id = stream.userID, !id.isEmpty else { return nil }
        let lists = model.home.lists
        return lists.collaborations[id] ?? lists.sharedChats[id]
    }

    // MARK: Transport

    private var playPause: some View {
        Button {
            watch.togglePlay()
            onTouch()
        } label: {
            Image(systemName: watch.paused ? Symbol.play : Symbol.pause)
                .font(.system(size: 28))
                .foregroundStyle(.white)
                .contentTransition(.symbolEffect(.replace))
                // The triangle's weight sits at its flat left side; centered by its box it looks
                // pushed left, so it moves right (see design-eye.md, optical nudges).
                .offset(x: watch.paused ? Optical.playNudge(28) : 0)
                .frame(width: 64, height: 64)
                .background(Circle().fill(Color.black.opacity(0.45)))
                .contentShape(Circle())
        }
        .buttonStyle(PressScale())
        .accessibilityLabel(watch.paused ? "Play" : "Pause")
    }

    /// The bar's symbols: one size and one weight, outlines throughout, so no button out-pulls
    /// another.
    private static let barIcon: CGFloat = 20

    private var bottomBar: some View {
        HStack(spacing: 0) {
            // A status, not a control: a dot and the word, no filled badge.
            HStack(spacing: 5) {
                LiveDot(size: 7)
                Text("LIVE").font(.sn(12, 700)).tracking(0.4).foregroundStyle(.white)
            }
            .padding(.leading, 4)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Live")
            Spacer(minLength: 8)
            IconButton(symbol: watch.muted ? Symbol.unmute : Symbol.mute, label: watch.muted ? "Unmute" : "Mute",
                       size: Self.barIcon, weight: .medium, tint: .white) {
                watch.toggleMute()
                onTouch()
            }
            if let stream {
                ShareLink(item: URL(string: "https://www.twitch.tv/\(stream.userLogin)")!) {
                    Image(systemName: Symbol.share)
                        .font(.system(size: Self.barIcon, weight: .medium))
                        .foregroundStyle(.white)
                        .frame(width: 44, height: 44)
                        .contentShape(Rectangle())
                }
                .accessibilityLabel("Share stream")
            }
            IconButton(symbol: Symbol.minimize, label: "Minimize player", size: Self.barIcon, weight: .medium, tint: .white,
                       action: onMinimize)
            Button {
                onTouch()
                Task { await model.restartStream() }
            } label: {
                Image(systemName: Symbol.refresh)
                    .font(.system(size: Self.barIcon, weight: .medium))
                    .foregroundStyle(.white)
                    .symbolEffect(.rotate, options: .repeat(.continuous), isActive: watch.restarting)
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Restart stream")
            IconButton(symbol: Symbol.quality, label: "Quality", size: Self.barIcon, weight: .medium, tint: .white) {
                qualityShown = true
            }
            if landscape {
                let on = model.nav.landscapeChat
                IconButton(symbol: on ? Symbol.chatShown : Symbol.chatShow, label: on ? "Hide chat" : "Show chat",
                           size: Self.barIcon, weight: .medium, tint: .white) {
                    onTouch()
                    withAnimation(SNMotion.run(.easeOut(duration: 0.18))) { model.nav.landscapeChat.toggle() }
                }
            }
        }
    }

    #if DEBUG
    private var edgeText: String {
        guard let behind = watch.behindMS else { return watch.status }
        return String(format: "Behind %.1f s", Double(behind) / 1000)
    }
    #endif
}

/// The quality sheet: "best" as "Auto" (capped to the screen) first, then the qualities the
/// stream offers in the core's order, the pick ticked. Picking closes the sheet, then switches.
struct QualitySheet: View {
    let model: AppModel
    @Environment(\.dismiss) private var dismiss

    private var watch: Watch { model.watch }

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: "Quality")
            if watch.available.isEmpty {
                Text("Quality options appear once the stream loads.")
                    .font(.sn(14))
                    .foregroundStyle(SN.textMuted)
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 24)
                    .padding(.vertical, 24)
            } else {
                let ticked = PlayerText.tickedQuality(picked: watch.pickedQuality, saved: watch.savedQuality)
                VStack(spacing: 0) {
                    ForEach(PlayerText.qualityRows(watch.available), id: \.self) { name in
                        Button {
                            dismiss()
                            Task { await model.changeQuality(name) }
                        } label: {
                            HStack {
                                VStack(alignment: .leading, spacing: 1) {
                                    Text(PlayerText.qualityLabel(name))
                                        .font(.sn(15))
                                        .foregroundStyle(SN.textPrimary)
                                    if name == "best" {
                                        Text("Matches your screen")
                                            .font(.sn(12))
                                            .foregroundStyle(SN.textMuted)
                                    }
                                }
                                Spacer()
                                if name == ticked {
                                    // The tick matches the label's size and sits one weight above
                                    // it, never heavier than the words it marks.
                                    Image(systemName: Symbol.check)
                                        .font(.system(size: 15, weight: .semibold))
                                        .foregroundStyle(SN.accent)
                                }
                            }
                            .padding(.horizontal, 8)
                            .frame(minHeight: 44)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(DimPress())
                        .accessibilityAddTraits(name == ticked ? .isSelected : [])
                    }
                }
                .padding(.horizontal, 16)
                .padding(.bottom, 12)
            }
        }
        .fittedSheet()
    }
}

/// The scrim under the controls: dark at the top and the bottom, clear through the middle. The top
/// reaches past the header's category line, so the three lines stay readable over bright video.
private struct Scrim: View {
    var body: some View {
        LinearGradient(stops: [.init(color: .black.opacity(0.66), location: 0), .init(color: .black.opacity(0.3), location: 0.26),
                               .init(color: .clear, location: 0.44),
                               .init(color: .clear, location: 0.62), .init(color: .black.opacity(0.55), location: 1)],
                       startPoint: .top, endPoint: .bottom)
            .allowsHitTesting(false)
    }
}

/// The channel's face in the player: 32 points (as tall as the name and title lines beside it)
/// with a live-red ring, or its first letter.
private struct ChannelAvatar: View {
    let url: URL?
    let name: String

    var body: some View {
        Group {
            if let url {
                AsyncImage(url: url) { image in
                    image.resizable().scaledToFill()
                } placeholder: {
                    letter
                }
            } else {
                letter
            }
        }
        .frame(width: 32, height: 32)
        .clipShape(Circle())
        .overlay(Circle().stroke(SN.live.opacity(0.8), lineWidth: 2))
        .accessibilityHidden(true)
    }

    private var letter: some View {
        ZStack {
            Circle().fill(Color.white.opacity(0.15))
            Text(name.prefix(1).uppercased())
                .font(.sn(12, 700))
                .foregroundStyle(.white)
        }
    }
}

/// The channel's live alerts beside its name: a bell when they are on, a crossed bell when the
/// channel is muted. Muting writes the lowercased login into the alert exclusion list.
private struct AlertBell: View {
    let store: SettingsStore
    let login: String
    let name: String

    private static let path = "live_notifications.muted_live_channels"

    var body: some View {
        let key = login.lowercased()
        let on = !store.strings(Self.path).contains(key)
        Button {
            Task { _ = try? await store.edit(Self.path, .toggle(key)) }
        } label: {
            Image(systemName: on ? "bell.fill" : "bell.slash")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.white.opacity(on ? 1 : 0.45))
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 22, height: 20)
                .contentShape(Rectangle().inset(by: -10))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(on ? "Turn off live alerts for \(name)" : "Turn on live alerts for \(name)")
        .accessibilityAddTraits(on ? .isSelected : [])
    }
}

/// Playback failed: a dark scrim, what happened, and Retry (a restart). Both take their own taps,
/// so they never toggle the controls.
private struct ErrorState: View {
    let text: String
    let retry: () -> Void

    var body: some View {
        ZStack {
            Color.black.opacity(0.7)
            VStack(spacing: 14) {
                Text(text)
                    .font(.sn(14))
                    .foregroundStyle(.white.opacity(0.8))
                    .multilineTextAlignment(.center)
                Button(action: retry) {
                    Text("Retry")
                        .font(.sn(14, 600))
                        .foregroundStyle(.white)
                        .padding(.horizontal, 20)
                        .padding(.vertical, 10)
                        .snGlass(Capsule(), interactive: true)
                }
                .buttonStyle(.plain)
            }
            .padding(24)
        }
        .contentShape(Rectangle())
        .onTapGesture {}
    }
}

/// Shrinks to 95 percent while pressed.
private struct PressScale: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed ? 0.95 : 1)
            .animation(.easeOut(duration: 0.12), value: configuration.isPressed)
    }
}

/// Dims to 70 percent while pressed, as the sheet's rows do.
private struct DimPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

/// A live status: a solid live-red dot.
struct LiveDot: View {
    var size: CGFloat = 6

    var body: some View {
        Circle()
            .fill(SN.live)
            .frame(width: size, height: size)
            .shadow(color: SN.live.opacity(0.6), radius: 1.5)
            .accessibilityLabel("Live")
    }
}
