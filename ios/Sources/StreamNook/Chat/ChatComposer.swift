import SNKit
import SwiftUI

/// The composer under a chat list: the reply chip, then one row with the field (emote button,
/// text, room-mode indicator, command button) and one round button that is Send while there is
/// text and the chat menu otherwise. Suggestions and the emote picker open upward over the list.
struct ChatComposer: View {
    let session: ChatSession
    /// Opens a chatter's profile (`/usercard`).
    var onProfile: (ProfileTarget) -> Void = { _ in }
    @State private var model: ChatComposerModel

    init(session: ChatSession, onProfile: @escaping (ProfileTarget) -> Void = { _ in }) {
        self.session = session
        self.onProfile = onProfile
        _model = State(initialValue: ChatComposerModel(session: session))
    }

    var body: some View {
        ComposerBar(model: model, session: session)
            .onAppear {
                model.onProfile = onProfile
                ComposerRegistry.shared.models[ObjectIdentifier(session)] = model
            }
    }
}

/// Composers on screen by session, for drive steps and the streak banners.
@MainActor
final class ComposerRegistry {
    static let shared = ComposerRegistry()
    var models: [ObjectIdentifier: ChatComposerModel] = [:]
}

private struct ComposerBar: View {
    @Bindable var model: ChatComposerModel
    let session: ChatSession
    @FocusState private var focused: Bool
    @Environment(\.inboxSurface) private var inboxSurface
    /// The right end's frame among the trays', while it is the tray.
    @State private var smiley = "😀"
    @State private var smileyOut = false

    private static let smileys = ["😀", "😄", "😁", "😆", "🤣", "😂", "😊", "😇", "🙂", "😉", "😌", "😍", "🥰", "😜", "🤪", "😎",
                                  "🤩", "🥳", "😏", "😋", "🤗", "🫠", "🫡", "😺"]

    var body: some View {
        VStack(spacing: 0) {
            if let reply = session.replyTo {
                replyChip(reply)
            }
            if let failure = model.failure {
                failureChip(failure)
            }
            if model.cheer.shown != nil {
                CheerBar(model: model)
            }
            if let warning = model.warning {
                warningBox(warning)
            }
            inputRow(slowWait: model.slowWait)
            // The emote panel in the keyboard's place, at the keyboard's height (the system emoji
            // keyboard's pattern): the emote button swaps the two and the field stays put, so it
            // can be watched filling while several emotes are tapped. Kept mounted while closed,
            // so its tab, search and scroll survive.
            ChatEmotePicker(model: model) { model.panel = .none }
                .frame(height: model.panel == .emotes ? KeyboardMetrics.panelHeight : 0, alignment: .top)
                .clipped()
                .padding(.top, model.panel == .emotes ? 8 : 0)
                .opacity(model.panel == .emotes ? 1 : 0)
                .allowsHitTesting(model.panel == .emotes)
                .accessibilityHidden(model.panel != .emotes)
                .closesOnOutsideTap(Binding(get: { model.panel == .emotes },
                                            set: { if !$0 { model.panel = .none } }), id: "emotes")
        }
        // The keyboard's own pace, so the panel and the keyboard trade places without a jump.
        .animation(.easeInOut(duration: 0.25), value: model.panel)
        .onAppear { KeyboardMetrics.start() }
        // The home indicator's inset plus 8.
        .padding(.bottom, 8)
        // The composer is the field's own: its buttons act and the keyboard stays, and its emote
        // picker and suggestions stay open while it is used (typing, sending, the emote button).
        .keepsKeyboard()
        .outsideTapAnchor("emotes")
        .outsideTapAnchor("suggestions")
        .onChange(of: model.focusRequest.0) { _, _ in focused = model.focusRequest.1 }
        // The bar is for writing now: an open inbox goes first.
        .onChange(of: focused) { _, now in
            if now, model.panel == .emotes { model.panel = .none }
            guard now, let mentions = session.rooms?.mentions, mentions.sheetIn != nil else { return }
            withAnimation(SNMotion.run(InboxMotion.spring)) { mentions.sheetIn = nil }
        }
        // A new mention's card rises into the room just above the bar, which the bar's own risers
        // use too: the card waits while one is up. A taller draft lifts the card with the bar.
        .onChange(of: model.panel != .none || showsStrip, initial: true) { _, busy in
            session.rooms?.mentions?.composerBusy = busy
        }
        // The wallet reads Rust's channel state for this room and follows its pushes.
        .task { await model.wallet?.start() }
        .onDisappear { model.wallet?.stop() }
        .overlay(alignment: .top) { above }
        .sheet(isPresented: $model.menuOpen) {
            ChatMenuSheet(model: model, session: session)
                .snSheetLight()
                .presentationDetents([.medium, .large])
        }
        .sheet(isPresented: $model.manageOpen) {
            if let roomID = session.roomID {
                ManageChannelSheet(core: session.core, channelID: roomID, channelLogin: session.channel,
                                   toasts: session.rooms?.toasts)
                    .tapsThroughKeyboard()
                    .presentationDetents([.large])
            }
        }
        .sheet(isPresented: $model.moderationOpen) {
            if let roomID = session.roomID {
                ModerationSheet(model: ModerationSheetModel(channel: session.channel, roomID: roomID, core: session.core)) {
                    // After this sheet has gone, so the two never present at once.
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.45) { model.manageOpen = true }
                }
                .tapsThroughKeyboard()
                .presentationDetents([.medium, .large])
            }
        }
        .sheet(isPresented: $model.commandsOpen) {
            ChatCommandSheet(session: session) { name in
                model.replaceWithCommand(name)
                model.commandsOpen = false
                focused = true
            }
            .tapsThroughKeyboard()
            .snSheetLight()
            .presentationDetents([.fraction(0.72), .large])
        }
    }

    /// What opens upward from the input row: a toast, then the picker or the suggestion strip.
    /// The picker stays mounted while closed, so its tab, search and scroll survive.
    private var above: some View {
        VStack(spacing: 8) {
            if let toast = model.toast {
                Text(toast)
                    .font(.snCaption)
                    .foregroundStyle(.primary)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 7)
                    .glassEffect()
                    .transition(.opacity)
            }
            ZStack(alignment: .bottom) {
                if model.panel == .none, showsStrip {
                    SuggestionStrip(model: model) {
                        if !model.fieldDisabled { focused = true }
                    }
                    .closesOnOutsideTap(Binding(get: { showsStrip }, set: { if !$0 { model.suggestionsDismissed = true } }),
                                        id: "suggestions")
                    .floatingSurface()
                    .transition(.opacity.combined(with: .offset(y: 4)))
                }
            }
        }
        .padding(.horizontal, 10)
        .alignmentGuide(.top) { $0[.bottom] + 8 }
        .animation(.easeOut(duration: 0.12), value: showsStrip)
    }

    private var showsStrip: Bool {
        !model.suggestions.isEmpty && session.style.suggestionsEnabled && !model.suggestionsDismissed
    }

    private func replyChip(_ reply: ChatMessage) -> some View {
        HStack(spacing: 6) {
            Text("Replying to \(Text("@\(reply.shownName)").fontWeight(.semibold))")
                .font(.sn(12.5))
                .foregroundStyle(SN.textSecondary)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            Button {
                session.replyTo = nil
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(SN.textMuted)
                    .padding(6)
            }
            .accessibilityLabel("Cancel reply")
        }
        .padding(.horizontal, 12)
        .padding(.top, 6)
    }

    /// A message Twitch did not take: why, and Retry or Discard. The text stays in the field.
    private func failureChip(_ failure: ChatComposerModel.Failure) -> some View {
        HStack(spacing: 4) {
            Image(systemName: "exclamationmark.circle.fill")
                .font(.system(size: 13))
                .foregroundStyle(ComposerInk.warning)
                .frame(width: 36)
            Text("Not sent: \(failure.why)")
                .font(.sn(12.5))
                .foregroundStyle(SN.textSecondary)
                .lineLimit(2)
            Spacer(minLength: 8)
            Button("Retry") { Task { await model.send() } }
                .font(.sn(12.5, 600))
                .disabled(model.sending)
            Button("Discard") { model.discardFailure() }
                .font(.sn(12.5))
                .foregroundStyle(ComposerInk.quiet)
                .padding(.leading, 8)
        }
        .padding(.leading, 14)
        .padding(.trailing, 16)
        .padding(.top, 6)
        .accessibilityElement(children: .contain)
    }

    /// A moderator's warning: what they said, and Acknowledge, which is the one way to chat
    /// again. It sits where the room's words already live and cannot scroll away.
    private func warningBox(_ warning: ChatWarning) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Label(warning.title, systemImage: "exclamationmark.bubble.fill")
                .font(.sn(14, 600))
                .foregroundStyle(Color(ChatTheme.textPrimary))
                .labelStyle(WarningLabelStyle())
            Text(warning.detail)
                .font(.sn(13))
                .foregroundStyle(Color(ChatTheme.textSecondary))
                .fixedSize(horizontal: false, vertical: true)
            Button {
                Task { await model.acknowledge() }
            } label: {
                ZStack {
                    Text(warning.action).opacity(model.acknowledging ? 0 : 1)
                    if model.acknowledging { ProgressView().tint(Color(ChatTheme.background)) }
                }
                .font(.sn(15, 600))
                .frame(maxWidth: .infinity, minHeight: 40)
                .foregroundStyle(Color(ChatTheme.background))
                .background(Color(ChatTheme.info), in: .rect(cornerRadius: 12))
            }
            .buttonStyle(.plain)
            .disabled(model.acknowledging)
            .padding(.top, 6)
        }
        .padding(12)
        .background(Color(ChatTheme.info).opacity(0.08), in: .rect(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(Color(ChatTheme.info).opacity(0.25)))
        .padding(.horizontal, 10)
        .padding(.top, 8)
        .accessibilityElement(children: .contain)
    }

    private func prompt(slowWait: Int) -> Text {
        if model.fieldDisabled {
            // Why the viewer cannot type, in the one warning color.
            return Text(model.placeholder).foregroundStyle(ComposerInk.warning).fontWeight(.medium)
        }
        if slowWait > 0 {
            return Text("Slow mode: you can send again in \(slowWait)s").foregroundStyle(SN.textMuted)
        }
        return Text(model.placeholder).foregroundStyle(SN.textMuted)
    }

    private func inputRow(slowWait: Int) -> some View {
        // Send is always the right end's job. The swap setting decides which of the inbox tray and
        // the chat menu sits left; a bar too narrow for the tray folds it away and the menu takes
        // the right end.
        let hasInbox = session.rooms?.mentions != nil
        let folded = model.inboxFolded || !hasInbox
        let trayRight = session.style.inboxRight && !folded
        return HStack(alignment: .center, spacing: 6) {
            if !folded {
                leftEnd(tray: !session.style.inboxRight)
            }
            fieldBox(slowWait: slowWait)
            endButton(slowWait: slowWait, tray: trayRight)
        }
        .padding(.horizontal, 10)
        .padding(.top, 8)
    }

    /// The bar's left end: the inbox tray, or the chat menu when Settings swaps the pair. While you
    /// type it slides out its own side and the field grows into its room, in one spring; it comes
    /// back the same way.
    private func leftEnd(tray: Bool) -> some View {
        Group {
            if tray, let mentions = session.rooms?.mentions {
                InboxTray(store: mentions, surface: inboxSurface)
            } else {
                Button { openMenu() } label: {
                    Image(systemName: "ellipsis")
                        .font(.system(size: 18, weight: .bold))
                        .foregroundStyle(SN.textSecondary)
                        .frame(width: 44, height: 44)
                }
                .glassEffect(.regular.interactive(), in: .circle)
                .accessibilityLabel("Chat options")
            }
        }
        .opacity(focused ? 0 : 1)
        .offset(x: focused ? -52 : 0)
        .frame(width: focused ? 0 : 44)
        .padding(.trailing, focused ? -6 : 0)
        .allowsHitTesting(!focused)
        .accessibilityHidden(focused)
        .animation(SNMotion.run(InboxMotion.spring), value: focused)
    }

    private func openMenu() {
        model.panel = .none
        model.menuOpen = true
    }

    /// About 12 characters of the placeholder at 15 pt: the least the field keeps before the
    /// tray leaves a narrow bar. Read from the field's real width, so any narrow bar behaves the
    /// same; the gap between the two thresholds keeps it from flickering at the edge. Typing hides
    /// the tray anyway, so the call waits for the keyboard to go.
    private static let fieldMin: CGFloat = 92
    private static let foldHysteresis: CGFloat = 12
    private static let trayRoom: CGFloat = 50

    private func foldTray(_ width: CGFloat) {
        model.inboxSurface = inboxSurface
        guard !focused, session.rooms?.mentions != nil else { return }
        let folded = model.inboxFolded
            ? width - Self.trayRoom < Self.fieldMin + Self.foldHysteresis
            : width < Self.fieldMin
        if folded != model.inboxFolded { model.inboxFolded = folded }
    }

    private func fieldBox(slowWait: Int) -> some View {
        HStack(spacing: 4) {
            if !session.style.hideEmoteButton {
                emoteButton
                    // It inserts nothing that could be sent while a mode other than
                    // emote-only blocks the room, or before a Kick sign-in.
                    .opacity(model.needsKickSignIn || (model.fieldDisabled && session.modes?.blocked != "emote_only") ? 0.4 : 1)
            }
            if model.needsKickSignIn {
                kickSignIn
            } else {
                TextField("", text: $model.draft, prompt: prompt(slowWait: slowWait))
                    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width in foldTray(width) }
                    .font(.sn(15))
                    .foregroundStyle(SN.textPrimary)
                    .padding(.vertical, 8)
                    .padding(.leading, session.style.hideEmoteButton ? 8 : 0)
                    .submitLabel(.send)
                    .onSubmit { Task { await model.send() } }
                    .focused($focused)
                    .disabled(model.fieldDisabled)
                    .autocorrectionDisabled(!session.style.spellcheck)
                    .textInputAutocapitalization(.sentences)
            }
            if model.needsKickSignIn {
                EmptyView()
            } else if model.draft.count >= ChatComposerModel.counterFrom {
                counter
            } else if let modes = model.shownModes, !model.fieldDisabled {
                RoomModeIndicator(modes: modes) {
                    model.panel = .none
                    model.menuOpen = true
                }
            }
            if !session.isKick {
                Button {
                    // The sheet covers the picker: close it rather than leave it under there.
                    model.panel = .none
                    model.commandsOpen = true
                } label: {
                    Image(systemName: "slash.circle")
                        .font(ComposerSymbol.font)
                        .foregroundStyle(SN.textSecondary)
                        .frame(width: 32, height: 36)
                }
                .accessibilityLabel("Chat commands")
            }
            // The channel wallet (points and bits): Twitch rooms only, and only with something to spend.
            if let wallet = model.wallet { WalletButton(wallet: wallet, session: session) { model.panel = .none } }
        }
        .padding(.leading, 4)
        .padding(.trailing, 5)
        .frame(minHeight: 40)
        .glassEffect(.regular, in: .rect(cornerRadius: 12))
    }

    /// What the right end of the bar is right now.
    private enum EndMode {
        case send, keyboard, tray, menu
    }

    /// The bar's right end, one control with every job it has: Send the moment the field holds a
    /// message, whatever sits here otherwise (the chat menu, or the inbox tray when Settings swaps
    /// the pair), and the way to put the keyboard away while you type with nothing written (as a
    /// tap anywhere outside the composer does). One button in one spot: the marks crossfade and the
    /// accent fill grows in under Send, so nothing slides in or moves. Send is always on the right.
    private func endButton(slowWait: Int, tray: Bool) -> some View {
        let mentions = session.rooms?.mentions
        let mode: EndMode = model.canSend || model.sending ? .send : focused ? .keyboard : tray ? .tray : .menu
        let sheetOpen = tray && mentions?.sheetIn == inboxSurface
        let unread = mentions?.unreadTotal ?? 0
        let showsCount = unread > 0 && (mode == .tray ? !sheetOpen : mode == .menu && model.inboxFolded)
        return Button {
            switch mode {
            case .send:
                Task { await model.send() }
            case .keyboard:
                focused = false
            case .tray:
                if let mentions {
                    withAnimation(SNMotion.run(InboxMotion.spring)) { mentions.toggleSheet(inboxSurface) }
                }
            case .menu:
                openMenu()
            }
        } label: {
            ZStack {
                // Send's accent fill, grown in under its mark.
                Circle()
                    .fill(SN.accent)
                    .scaleEffect(mode == .send ? 1 : 0.55)
                    .opacity(mode == .send ? 1 : 0)
                Image(systemName: "ellipsis")
                    .font(.system(size: 18, weight: .bold))
                    .foregroundStyle(SN.textSecondary)
                    .opacity(mode == .menu ? 1 : 0)
                if tray {
                    Image(systemName: sheetOpen ? "xmark" : "tray")
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(sheetOpen ? SN.textPrimary : SN.textSecondary)
                        .opacity(mode == .tray ? 1 : 0)
                }
                Image(systemName: "keyboard.chevron.compact.down")
                    .font(.system(size: 18, weight: .bold))
                    .foregroundStyle(SN.textSecondary)
                    .opacity(mode == .keyboard ? 1 : 0)
                sendMark(slowWait: slowWait)
                    .opacity(mode == .send ? 1 : 0)
            }
            // The marks swap over 0.2 s, as Android crossfades them.
            .animation(SNMotion.run(.easeOut(duration: 0.2)), value: mode)
            // A circle reads smaller than the 40 pt field beside it; 44 matches it.
            .frame(width: 44, height: 44)
            .contentShape(Circle())
        }
        .glassEffect(.regular.interactive(), in: .circle)
        .overlay(alignment: .topLeading) {
            // The inbox's count rides the corner beside the field: on the tray when it sits here,
            // or on the chat menu when a narrow bar folded the tray into it.
            if showsCount {
                InboxCount(count: unread)
                    .offset(x: -4, y: -4)
                    .allowsHitTesting(false)
            }
        }
        // A tap on the tray here is the tray's own, never a tap outside its sheet.
        .background {
            if tray { Color.clear.outsideTapAnchor("inbox") }
        }
        .disabled(mode == .send && (model.sending || slowWait > 0 || model.cheerBlocked))
        .opacity(mode == .send && (model.sending || model.cheerBlocked) ? 0.5 : 1)
        .accessibilityLabel(mode == .send
            ? (slowWait > 0 ? "Send again in \(slowWait) seconds"
                : model.cheer.draft.map { "Cheer \($0.total.formatted()) bits" } ?? "Send")
            : mode == .keyboard ? "Hide keyboard"
            : mode == .tray ? (sheetOpen ? "Close the inbox" : unread > 0 ? "Inbox, \(unread) new" : "Inbox")
            : (showsCount ? "Chat options, \(unread) new in the inbox" : "Chat options"))
    }

    /// Send's mark: the slow-mode seconds, the cheer's gem when the message cheers, else the plane.
    @ViewBuilder
    private func sendMark(slowWait: Int) -> some View {
        if slowWait > 0 {
            // Slow mode: the seconds until the next message can go.
            Text("\(slowWait)")
                .font(.system(size: 14, weight: .semibold).monospacedDigit())
                .foregroundStyle(sendInk)
        } else if let gem = model.cheer.draft?.iconURL {
            // The message cheers: Send spends, so it wears the gem.
            RewardArt(url: gem, width: 22, height: 22, radius: 0, fit: true, placeholder: nil)
        } else {
            Image(systemName: "paperplane.fill")
                .font(.system(size: 18, weight: .bold))
                .foregroundStyle(sendInk)
        }
    }

    /// Send's mark on the accent fill: black on a light accent, white on a dark one.
    private var sendInk: Color {
        Look.shared.palette.accent.luminance > 0.5 ? Color.black.opacity(0.88) : .white
    }

    /// A Kick room without a Kick sign-in: one accent button where the text would go, which runs
    /// the same Kick sign-in as the You tab.
    private var kickSignIn: some View {
        Button { model.signInToKick() } label: {
            HStack(spacing: 6) {
                Text(model.kickSigningIn ? "Waiting for you to sign in" : "Sign in to Kick to chat")
                    .font(.sn(15, 600))
                    .lineLimit(1)
                Spacer(minLength: 0)
                if !model.kickSigningIn {
                    Image(systemName: "arrow.right.circle.fill")
                        .font(ComposerSymbol.font)
                        .frame(width: 32, height: 36)
                }
            }
            .foregroundStyle(SN.accent)
            .padding(.vertical, 2)
            .padding(.leading, session.style.hideEmoteButton ? 8 : 0)
            .frame(minHeight: 36)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(model.kickSigningIn)
        .opacity(model.kickSigningIn ? 0.6 : 1)
    }

    /// "N/500" once a message nears Twitch's limit: quiet, orange in the last 20, red past it.
    private var counter: some View {
        let count = model.draft.count
        let limit = ChatComposerModel.messageLimit
        let ink = count > limit ? Color.red : (count >= limit - 20 ? ComposerInk.warning : ComposerInk.quiet)
        return Text("\(count)/\(limit)")
            .font(.system(size: 13, weight: .medium).monospacedDigit())
            .foregroundStyle(ink)
            .padding(.horizontal, 4)
            .frame(height: 36)
            .accessibilityLabel("\(count) of \(limit) characters")
    }

    private var emoteButton: some View {
        Button {
            withAnimation(.easeInOut(duration: 0.11)) { smileyOut = true }
            Task {
                try? await Task.sleep(for: .milliseconds(110))
                smiley = Self.smileys.filter { $0 != smiley }.randomElement() ?? "😀"
                withAnimation(.easeInOut(duration: 0.11)) { smileyOut = false }
            }
            if model.panel == .emotes {
                // Back to the keyboard: it comes up where the panel was.
                model.panel = .none
                focused = true
            } else {
                // The keyboard goes and the panel takes its place.
                focused = false
                model.panel = .emotes
            }
        } label: {
            Group {
                if model.panel == .emotes {
                    Image(systemName: "keyboard")
                        .font(.system(size: 17, weight: .regular))
                        .foregroundStyle(SN.textSecondary)
                } else {
                    Text(smiley)
                        .font(.sn(20))
                        .opacity(smileyOut ? 0 : 1)
                        .scaleEffect(smileyOut ? 0.8 : 1)
                }
            }
            .frame(width: 36, height: 36)
        }
        .accessibilityLabel(model.panel == .emotes ? "Keyboard" : "Emotes and emoji")
    }
}

/// The warning's title: its symbol in the info ink, centered on the title's line.
private struct WarningLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            configuration.icon.foregroundStyle(Color(ChatTheme.info))
            configuration.title
        }
    }
}

/// The symbols at the field's end (room modes, commands) share one size, weight and ink, so the
/// emoji at its start is the only color in the field.
enum ComposerSymbol {
    static let font = Font.system(size: 17, weight: .medium)
}

/// The composer's two inks beyond the theme: one quiet gray that stays readable on every chat
/// background (placeholders, hints, counts), and one warning color for anything blocked or not sent.
enum ComposerInk {
    static let quiet = Color(red: 0x86 / 255, green: 0x98 / 255, blue: 0x9E / 255)
    static let warning = Color(red: 1, green: 0x94 / 255, blue: 0x30 / 255)
}

extension View {
    /// A surface that floats over the chat above the composer: an opaque apron from its bottom
    /// edge down to the composer's hairline (so no chat shows in the gap or its rounded corners)
    /// and a short fade above it, so the chat it covers never ends in a hard cut.
    func floatingSurface() -> some View {
        let ground = SN.background
        return background(alignment: .bottom) {
            ground
                .frame(height: 28)
                .offset(y: 8)
                .padding(.horizontal, -10)
        }
        .background(alignment: .top) {
            LinearGradient(colors: [ground.opacity(0), ground], startPoint: .top, endPoint: .bottom)
                .frame(height: 12)
                .offset(y: -12)
                .padding(.horizontal, -10)
                .allowsHitTesting(false)
        }
    }
}

/// The room's modes inside the field: the first mode's symbol, and "+N" when more are on.
private struct RoomModeIndicator: View {
    let modes: ChatRoomModes
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 4) {
                Image(systemName: RoomModeSymbol.name(modes.modes[0].kind))
                    .font(ComposerSymbol.font)
                if modes.modes.count > 1 {
                    Text("+\(modes.modes.count - 1)")
                        .font(.system(size: 10, weight: .semibold).monospacedDigit())
                        .frame(minWidth: 13)
                }
            }
            .foregroundStyle(Color(spares ? ChatTheme.textMuted : ChatTheme.textSecondary))
            .padding(.horizontal, 4)
            .frame(height: 36)
        }
        .accessibilityLabel(label)
    }

    /// Every mode spares the viewer.
    private var spares: Bool { modes.prompt == nil && modes.standing != nil }

    private var label: String {
        let labels = modes.modes.map(\.label).joined(separator: ", ")
        guard let standing = modes.standing else { return labels }
        return "\(labels). \(standing)"
    }
}

/// Emote and chatter suggestions for the last word, one swipe-snapped row.
private struct SuggestionStrip: View {
    let model: ChatComposerModel
    let focus: () -> Void
    @State private var active: String?

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { reader in
                ScrollView(.horizontal, showsIndicators: false) {
                    LazyHStack(spacing: 6) {
                        ForEach(model.suggestions, id: \.key) { suggestion in
                            Button {
                                model.complete(suggestion)
                                focus()
                            } label: {
                                tile(suggestion)
                            }
                            .buttonStyle(.plain)
                            .accessibilityLabel(suggestion.name)
                            .id(suggestion.key)
                        }
                    }
                    .scrollTargetLayout()
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                }
                .scrollTargetBehavior(.viewAligned)
                .scrollPosition(id: $active, anchor: .leading)
                // 52 pt tiles and 10 pt above and below: the strip never takes the picker's height.
                .frame(height: 72)
                // A new list starts again at its first tile.
                .onChange(of: model.suggestions.first?.key) { _, first in
                    active = first
                    if let first { reader.scrollTo(first, anchor: .leading) }
                }
            }
            HStack(spacing: 6) {
                let current = model.suggestions.first { $0.key == active } ?? model.suggestions.first
                Text(current?.name ?? "")
                    .font(.sn(12, 600))
                    .foregroundStyle(SN.textPrimary)
                    .lineLimit(1)
                // Where it comes from: the provider's logo, or a person for a chatter.
                if let current {
                    if current.isChatter {
                        Image(systemName: "person.fill")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(SN.textMuted)
                            .accessibilityLabel("Chatter")
                    } else {
                        ProviderLogo(provider: current.provider, size: 10, tinted: true)
                    }
                }
                Spacer()
                Text(model.suggestions.count > 1 ? "Swipe for more, tap to use" : "Tap to use")
                    .font(.sn(10))
                    .foregroundStyle(ComposerInk.quiet)
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 6)
            .overlay(alignment: .top) { Rectangle().fill(.white.opacity(0.05)).frame(height: 1) }
        }
        .background(SN.background, in: .rect(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(SN.borderSubtle))
    }

    private func tile(_ suggestion: EmoteMatch) -> some View {
        let isActive = suggestion.key == (active ?? model.suggestions.first?.key)
        return Group {
            if suggestion.isChatter {
                Text(suggestion.name)
                    .font(.sn(10, 600))
                    .foregroundStyle(SN.textSecondary)
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
                    .padding(2)
            } else {
                ChatImageView(url: ChatImageURL.thumbnail(suggestion.url), size: 36)
            }
        }
        .frame(width: 52, height: 52)
        .background(isActive ? Color.white.opacity(0.10) : .clear, in: .rect(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).stroke(isActive ? Color.white.opacity(0.15) : .clear))
    }
}

/// The CDN size an emote thumbnail needs.
enum ChatImageURL {
    static func thumbnail(_ url: String, points: CGFloat = 32) -> String {
        RowText.emoteURL(url, pixelHeight: Double(points * UITraitCollection.current.displayScale))
    }
}
