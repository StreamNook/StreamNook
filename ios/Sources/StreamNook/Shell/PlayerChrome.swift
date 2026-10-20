import AVKit
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
                // The loading mark holds the center until the first frame; the button takes it then.
                if watch.error == nil, watch.firstPlayMS != nil {
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
        .sheet(isPresented: $qualityShown) { QualitySheet(model: model).snSheetLight() }
        .sheet(isPresented: $togetherShown) {
            if let stream, let group = group(stream) {
                TogetherSheet(group: group, playing: stream.userLogin) { member in
                    model.startWatching(TwitchStream(id: "", userLogin: member.login, userName: member.displayName, title: "",
                                                     viewerCount: member.viewerCount ?? 0, gameName: "",
                                                     userID: member.userID, profileImageURL: member.avatarURL))
                }
                .snSheetLight()
            }
        }
        .onChange(of: model.nav.qualityPing) { _, _ in qualityShown = true }
    }

    // MARK: Header

    private func header(_ stream: TwitchStream) -> some View {
        let live = watch.live
        // The room's latest broadcast names itself; a live stream its current title.
        let title = watch.vodTitle ?? (watch.roomVod ? (watch.room?.latestVod?.title ?? stream.title)
                                                      : (live?.title ?? stream.title))
        let category = live?.gameName ?? stream.gameName
        // The avatar anchors a column: the name, the title and the category all start at the
        // name's edge, so the three read as one block about one stream. The live red is the
        // avatar's ring alone; the counts stay white so they never out-pull the name.
        return HStack(alignment: .top, spacing: 10) {
            Button {
                onTouch()
                model.channelSheet = stream
            } label: {
                ChannelAvatar(url: stream.profileImageURL.flatMap(URL.init(string:)), name: stream.userName)
            }
            .buttonStyle(.plain)
            .disabled(!stream.isTwitch)
            .accessibilityLabel("\(stream.userName)'s videos and clips")
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    HStack(spacing: 4) {
                        Text(stream.userName)
                            .font(.sn(14, 600))
                            .foregroundStyle(.white)
                            .lineLimit(1)
                        if stream.isPartner { PartnerMark(size: 12) }
                        // Live alerts are Twitch's (the notify worker watches Twitch logins).
                        if stream.isTwitch {
                            AlertBell(store: model.settingsStore, login: stream.userLogin, name: stream.userName)
                        }
                    }
                    .layoutPriority(1)
                    // Twitch and Kick only (every row here is one of the two); it drops out
                    // whole before the name gives way.
                    if !watch.roomVod {
                        SubscribePill(model: model, stream: stream)
                    }
                    Spacer(minLength: 8)
                    // When room runs out the Together chip keeps only its faces, then the uptime
                    // goes; the name never gives way to the counts.
                    ViewThatFits(in: .horizontal) {
                        metrics(stream, compactGroup: false, uptime: true)
                        metrics(stream, compactGroup: true, uptime: true)
                        metrics(stream, compactGroup: true, uptime: false)
                    }
                }
                Text(title)
                    .font(.sn(13))
                    .foregroundStyle(.white.opacity(0.85))
                    .lineLimit(1)
                if !category.isEmpty {
                    Text(category)
                        .font(.sn(12))
                        .foregroundStyle(.white.opacity(0.7))
                        .lineLimit(1)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The header's right side: the group, viewers, and uptime (only when the start is known).
    private func metrics(_ stream: TwitchStream, compactGroup: Bool, uptime: Bool) -> some View {
        let live = watch.live
        let viewers = live?.viewerCount ?? stream.viewerCount
        let started = live?.startedAt ?? stream.startedAt
        return HStack(spacing: 10) {
            if let group = group(stream) {
                Button { togetherShown = true } label: {
                    TogetherTag(group: group, compact: compactGroup, ink: .white.opacity(0.85))
                }
                .buttonStyle(.plain)
            }
            // A quiet moon while the sleep timer runs; the menu says when it stops.
            if model.sleepTimer?.running == true {
                Image(systemName: "moon.zzz.fill")
                    .font(.system(size: 11, weight: .medium))
                    .accessibilityLabel(model.sleepTimer?.rowLabel ?? "Sleep timer on")
            }
            if let recorded = recordedLine {
                Text(recorded)
            } else if watch.room == nil {
                HStack(spacing: 4) {
                    Image(systemName: Symbol.viewers).font(.system(size: 10, weight: .medium))
                    Text(viewers.formatted()).monospacedDigit()
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("\(viewers.formatted()) viewers")
            }
            if uptime, watch.playingVod == nil, !PlayerText.uptime(startedAt: started).isEmpty {
                TimelineView(.periodic(from: .now, by: 30)) { context in
                    Text(PlayerText.uptime(startedAt: started, now: context.date))
                        .monospacedDigit()
                }
            }
        }
        .font(.sn(12, 500))
        .foregroundStyle(.white.opacity(0.8))
        .fixedSize()
    }

    /// "Recorded Sep 28" while a VOD plays (with the year when it is not this one).
    private var recordedLine: String? {
        guard let vod = watch.playingVod else { return nil }
        guard let raw = vod.recordedAt, let date = PlayerText.parseDate(raw) else { return "Recorded" }
        let sameYear = Calendar.current.isDate(date, equalTo: .now, toGranularity: .year)
        return "Recorded " + date.formatted(.dateTime.month(.abbreviated).day().year(sameYear ? .omitted : .defaultDigits))
    }

    /// The channel's group: Shared Viewership from the core's live collab section, else what the
    /// home lists know (a Shared Chat session included).
    private func group(_ stream: TwitchStream) -> ChannelGroup? {
        if let group = watch.group { return group }
        guard let id = stream.twitchID, !id.isEmpty else { return nil }
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

    /// A VOD's transport: back and forward ten seconds and the speed. Play and pause stay the
    /// center button's, as on a live stream.
    private var vodTransport: some View {
        HStack(spacing: 0) {
            IconButton(symbol: "gobackward.10", label: "Back 10 seconds", size: Self.barIcon, weight: .medium,
                       tint: .white) {
                watch.skipVod(by: -10)
                onTouch()
            }
            IconButton(symbol: "goforward.10", label: "Forward 10 seconds", size: Self.barIcon, weight: .medium,
                       tint: .white) {
                watch.skipVod(by: 10)
                onTouch()
            }
            Menu {
                ForEach([Float(1), 1.25, 1.5, 2], id: \.self) { rate in
                    Button {
                        watch.setVodRate(rate)
                        onTouch()
                    } label: {
                        if rate == watch.vodRate {
                            Label(PlayerText.rate(rate), systemImage: "checkmark")
                        } else {
                            Text(PlayerText.rate(rate))
                        }
                    }
                }
            } label: {
                Text(PlayerText.rate(watch.vodRate))
                    .font(.sn(12, 700).monospacedDigit())
                    .foregroundStyle(.white)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 1)
                    .overlay(RoundedRectangle(cornerRadius: 5, style: .continuous).strokeBorder(.white, lineWidth: 1.5))
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("Speed \(PlayerText.rate(watch.vodRate))")
        }
    }

    private func liveStatus(behind: String?) -> some View {
        HStack(spacing: 5) {
            LiveDot(size: 7)
            Text("LIVE").font(.sn(12, 700)).tracking(0.4).foregroundStyle(.white)
            if let behind {
                ZStack(alignment: .leading) {
                    Text("· 59s+").hidden()
                    Text("· \(behind)")
                }
                .font(.sn(12, 500).monospacedDigit())
                .foregroundStyle(.white.opacity(0.7))
            }
        }
        .fixedSize()
    }

    private var bottomBar: some View {
        VStack(spacing: 0) {
            if watch.playingVod != nil {
                VodSeekBar(watch: watch, onTouch: onTouch)
                    .padding(.trailing, 0)
            }
            barRow
        }
    }

    /// The bar's buttons, left to right: the most used at the right, under the thumb, Minimize
    /// last. One list, so a change of order is one line. A button a stream has no use for (Mute
    /// and Reload on a VOD, Clip signed out, Chat in portrait) leaves its place out.
    private enum BarSlot: Hashable { case cast, share, clip, mute, reload, settings, chat, rotate, minimize }
    private static let barOrder: [BarSlot] = [.cast, .share, .clip, .mute, .reload, .settings, .chat, .rotate, .minimize]

    @ViewBuilder
    private func barButton(_ slot: BarSlot) -> some View {
        switch slot {
        case .cast:
            // The system's AirPlay picker: an Apple TV, a HomePod, AirPods. UIKit draws its glyph
            // a point high in its box, so it is set down to the row's bottom edge.
            RoutePicker()
                .barSlot()
                .offset(y: 1.5)
                .accessibilityLabel("Watch on a TV")
        case .share:
            if let stream {
                let title = watch.vodTitle ?? watch.live?.title ?? stream.title
                let link = watch.playingVod.flatMap { URL(string: "https://www.twitch.tv/videos/\($0.videoID)") }
                    ?? stream.channelLink ?? URL(string: "https://www.twitch.tv/\(stream.userLogin)")!
                ShareLink(item: link,
                          subject: Text(title.isEmpty ? stream.userName : title),
                          message: Text(watch.playingVod == nil ? "\(stream.userName) is live" : "\(stream.userName)'s broadcast")) {
                    Image(systemName: Symbol.share)
                        .font(.system(size: Self.barIcon, weight: .medium))
                        .foregroundStyle(.white)
                        .barSlot()
                        .contentShape(Rectangle())
                }
                .accessibilityLabel(watch.playingVod == nil ? "Share stream" : "Share video")
            }
        case .clip:
            if let stream, stream.isTwitch, model.signedIn {
                BarButton(symbol: "scissors", label: "Create clip", size: Self.barIcon, weight: .medium, tint: .white) {
                    onTouch()
                    model.createClip(login: stream.userLogin, channelID: stream.userID ?? "")
                }
            }
        case .mute:
            // The speaker is short and wide, so centered in its box it ends higher than the
            // taller symbols beside it; set down to share their bottom edge.
            if watch.playingVod == nil {
                BarButton(symbol: watch.muted ? Symbol.unmute : Symbol.mute, label: watch.muted ? "Unmute" : "Mute",
                          size: Self.barIcon, weight: .medium, tint: .white) {
                    watch.toggleMute()
                    onTouch()
                }
                .offset(y: 2)
            }
        case .reload:
            if watch.playingVod == nil {
                Button {
                    onTouch()
                    Task { await model.restartStream() }
                } label: {
                    Image(systemName: Symbol.refresh)
                        .font(.system(size: Self.barIcon, weight: .medium))
                        .foregroundStyle(.white)
                        .symbolEffect(.rotate, options: .repeat(.continuous), isActive: watch.restarting)
                        .barSlot()
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Restart stream")
            }
        case .settings:
            BarButton(symbol: Symbol.quality, label: "Quality", size: Self.barIcon, weight: .medium, tint: .white) {
                qualityShown = true
            }
        case .chat:
            if landscape {
                let on = model.nav.landscapeChat
                BarButton(symbol: Symbol.chatShow, label: on ? "Hide chat" : "Show chat",
                          size: 18, weight: .medium, tint: on ? SN.accent : .white) {
                    onTouch()
                    withAnimation(SNMotion.run(.easeOut(duration: 0.18))) { model.nav.landscapeChat.toggle() }
                }
            }
        case .rotate:
            // Turns the screen whatever the phone's rotation lock says; it holds until the player
            // closes or shrinks, which hands the screen back to the sensor.
            BarButton(symbol: landscape ? "arrow.down.right.and.arrow.up.left" : "arrow.up.left.and.arrow.down.right",
                      label: landscape ? "Rotate to portrait" : "Rotate to landscape",
                      size: Self.barIcon, weight: .medium, tint: .white) {
                onTouch()
                Orientation.hold(landscape: !landscape)
            }
        case .minimize:
            BarButton(symbol: Symbol.minimize, label: "Minimize player", size: 22, weight: .medium, tint: .white,
                      action: onMinimize)
        }
    }

    private var barRow: some View {
        HStack(spacing: 0) {
            if watch.roomVod {
                // The room's latest broadcast plays: the way back is where LIVE sits.
                Button { watch.leaveRoomVod() } label: {
                    Label("Room", systemImage: Symbol.back)
                        .font(.sn(12, 700))
                        .foregroundStyle(.white)
                        .frame(height: 44)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Back to the room")
            }
            if watch.playingVod != nil {
                vodTransport
            }
            // A status, not a control: a dot and the word, no filled badge.
            if !watch.roomVod, watch.playingVod == nil {
                // How far behind live sits beside the word, in a slot sized for the widest
                // reading so the bar never shifts. It is sized first: the buttons give up a few
                // points each to make room, and only a bar too narrow even then drops the number.
                ViewThatFits(in: .horizontal) {
                    liveStatus(behind: watch.behindLabel)
                    liveStatus(behind: nil)
                }
                .opacity(watch.error == nil ? 1 : 0)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(watch.behindSpoken.map { "Live, \($0)" } ?? "Live")
                .accessibilityHidden(watch.error != nil)
                .layoutPriority(1)
            }
            Spacer(minLength: 8)
            ForEach(Self.barOrder, id: \.self) { slot in barButton(slot) }
        }
        // The last icon's ink, not its 44 pt target, ends on the header's right edge.
        .padding(.trailing, -9)
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
                                .padding(.vertical, name == "best" ? 4 : 0)
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
            SleepTimerSection(model: model)
        }
        .fittedSheet()
    }
}

/// The scrim under the controls: dark at the top and the bottom, clear through the middle. The top
/// reaches past the header's category line, so the three lines stay readable over bright video.
private struct Scrim: View {
    var body: some View {
        LinearGradient(stops: [.init(color: .black.opacity(0.66), location: 0), .init(color: .black.opacity(0.45), location: 0.30),
                               .init(color: .clear, location: 0.46),
                               .init(color: .clear, location: 0.60), .init(color: .black.opacity(0.35), location: 0.78),
                               .init(color: .black.opacity(0.7), location: 1)],
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
                .foregroundStyle(.white.opacity(on ? 0.7 : 0.45))
                .contentTransition(.symbolEffect(.replace))
                .frame(width: 14, height: 20)
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

/// The system route picker in the bar's style: white, accent while a route is active, video
/// devices first.
private struct RoutePicker: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let picker = AVRoutePickerView()
        picker.prioritizesVideoDevices = true
        picker.tintColor = .white
        picker.activeTintColor = UIColor(SN.accent)
        return picker
    }

    func updateUIView(_ picker: AVRoutePickerView, context: Context) {}
}

/// A bottom-bar button: 44 points wide when the bar has room, down to 34 when it is full, so the
/// LIVE status and its behind-live number always fit beside a full row of buttons.
private struct BarButton: View {
    let symbol: String
    let label: String
    var size: CGFloat = 20
    var weight: Font.Weight = .regular
    var tint: Color?
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
                .font(.system(size: size, weight: weight))
                .foregroundStyle(tint ?? SN.textSecondary)
                .barSlot()
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

private extension View {
    /// A bar button's box: 44 points, giving way to 34 when the row is short of room.
    func barSlot() -> some View {
        frame(minWidth: 34, idealWidth: 44, maxWidth: 44, minHeight: 44, maxHeight: 44)
    }
}
