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
    @State private var smiley = "😀"
    @State private var smileyOut = false

    private static let smileys = ["😀", "😄", "😁", "😆", "🤣", "😂", "😊", "😇", "🙂", "😉", "😌", "😍", "🥰", "😜", "🤪", "😎",
                                  "🤩", "🥳", "😏", "😋", "🤗", "🫠", "🫡", "😺"]

    var body: some View {
        VStack(spacing: 0) {
            Rectangle()
                .fill(Color(ChatTheme.borderSubtle))
                .frame(height: 1)
            if let reply = session.replyTo {
                replyChip(reply)
            }
            if let failure = model.failure {
                failureChip(failure)
            }
            inputRow(slowWait: model.slowWait)
        }
        // The home indicator's inset plus 8.
        .padding(.bottom, 8)
        .onChange(of: model.focusRequest.0) { _, _ in focused = model.focusRequest.1 }
        .overlay(alignment: .top) { above }
        .sheet(isPresented: $model.menuOpen) {
            ChatMenuSheet(model: model, session: session)
                .presentationDetents([.medium, .large])
        }
        .sheet(isPresented: $model.commandsOpen) {
            ChatCommandSheet(session: session) { name in
                model.replaceWithCommand(name)
                model.commandsOpen = false
                focused = true
            }
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
                ChatEmotePicker(model: model)
                    .floatingSurface()
                    .opacity(model.panel == .emotes ? 1 : 0)
                    .offset(y: model.panel == .emotes ? 0 : 10)
                    .scaleEffect(model.panel == .emotes ? 1 : 0.98, anchor: .bottom)
                    .allowsHitTesting(model.panel == .emotes)
                    .accessibilityHidden(model.panel != .emotes)
                if model.panel == .none, showsStrip {
                    SuggestionStrip(model: model) {
                        if !model.fieldDisabled { focused = true }
                    }
                    .floatingSurface()
                    .transition(.opacity.combined(with: .offset(y: 4)))
                }
            }
        }
        .padding(.horizontal, 10)
        .alignmentGuide(.top) { $0[.bottom] + 8 }
        .animation(.interpolatingSpring(stiffness: 400, damping: 30), value: model.panel)
        .animation(.easeOut(duration: 0.12), value: showsStrip)
    }

    private var showsStrip: Bool { !model.suggestions.isEmpty && session.style.suggestionsEnabled }

    private func replyChip(_ reply: ChatMessage) -> some View {
        HStack(spacing: 6) {
            Text("Replying to \(Text("@\(reply.shownName)").fontWeight(.semibold))")
                .font(.sn(12.5))
                .foregroundStyle(Color(ChatTheme.textSecondary))
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
            Button {
                session.replyTo = nil
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Color(ChatTheme.textMuted))
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
                .foregroundStyle(Color(ChatTheme.textSecondary))
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

    private func prompt(slowWait: Int) -> Text {
        if model.fieldDisabled {
            // Why the viewer cannot type, in the one warning color.
            return Text(model.placeholder).foregroundStyle(ComposerInk.warning).fontWeight(.medium)
        }
        if slowWait > 0 {
            return Text("Slow mode: you can send again in \(slowWait)s").foregroundStyle(Color(ChatTheme.textMuted))
        }
        return Text(model.placeholder).foregroundStyle(Color(ChatTheme.textMuted))
    }

    private func inputRow(slowWait: Int) -> some View {
        HStack(alignment: .center, spacing: 6) {
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
                        .font(.sn(15))
                        .foregroundStyle(Color(ChatTheme.textPrimary))
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
                if !session.style.hideCommandButton, !session.isKick {
                    Button {
                        model.commandsOpen = true
                    } label: {
                        Image(systemName: "slash.circle")
                            .font(ComposerSymbol.font)
                            .foregroundStyle(Color(ChatTheme.textSecondary))
                            .frame(width: 32, height: 36)
                    }
                    .accessibilityLabel("Chat commands")
                }
            }
            .padding(.leading, 4)
            .padding(.trailing, 5)
            .frame(minHeight: 40)
            .glassEffect(.regular, in: .rect(cornerRadius: 12))

            Button {
                if model.canSend {
                    Task { await model.send() }
                } else {
                    model.panel = .none
                    model.menuOpen = true
                }
            } label: {
                Group {
                    if model.canSend, slowWait > 0 {
                        // Slow mode: the seconds until the next message can go.
                        Text("\(slowWait)")
                            .font(.system(size: 14, weight: .semibold).monospacedDigit())
                            .foregroundStyle(Color(ChatTheme.textPrimary))
                    } else {
                        Image(systemName: model.canSend || model.sending ? "paperplane.fill" : "ellipsis")
                            .font(.system(size: 18, weight: .bold))
                            .foregroundStyle(model.canSend ? Color.white : Color(ChatTheme.textSecondary))
                            .contentTransition(.symbolEffect(.replace))
                    }
                }
                // A circle reads smaller than the 40 pt field beside it; 44 matches it.
                .frame(width: 44, height: 44)
            }
            .glassEffect(.regular.interactive(), in: .circle)
            .disabled(model.sending || (model.canSend && slowWait > 0))
            .opacity(model.sending ? 0.5 : 1)
            .accessibilityLabel(model.canSend ? (slowWait > 0 ? "Send again in \(slowWait) seconds" : "Send") : "Chat options")
        }
        .padding(.horizontal, 10)
        .padding(.top, 8)
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
            .foregroundStyle(Color(ChatTheme.accent))
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
            model.panel = model.panel == .emotes ? .none : .emotes
        } label: {
            Group {
                if model.panel == .emotes {
                    Image(systemName: "xmark")
                        .font(.system(size: 17, weight: .bold))
                        .foregroundStyle(Color(ChatTheme.accent))
                } else {
                    Text(smiley)
                        .font(.sn(20))
                        .opacity(smileyOut ? 0 : 1)
                        .scaleEffect(smileyOut ? 0.8 : 1)
                }
            }
            .frame(width: 36, height: 36)
        }
        .accessibilityLabel(model.panel == .emotes ? "Close emotes" : "Emotes and emoji")
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
        let ground = Color(ChatTheme.background)
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
                    .foregroundStyle(Color(ChatTheme.textPrimary))
                    .lineLimit(1)
                // Where it comes from: the provider's logo, or a person for a chatter.
                if let current {
                    if current.isChatter {
                        Image(systemName: "person.fill")
                            .font(.system(size: 9, weight: .semibold))
                            .foregroundStyle(Color(ChatTheme.textMuted))
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
        .background(Color(ChatTheme.background), in: .rect(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color(ChatTheme.borderSubtle)))
    }

    private func tile(_ suggestion: EmoteMatch) -> some View {
        let isActive = suggestion.key == (active ?? model.suggestions.first?.key)
        return Group {
            if suggestion.isChatter {
                Text(suggestion.name)
                    .font(.sn(10, 600))
                    .foregroundStyle(Color(ChatTheme.textSecondary))
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
