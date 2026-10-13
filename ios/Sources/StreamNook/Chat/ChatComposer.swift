import SNKit
import SwiftUI

/// The composer under a chat list: the reply chip, then one row with the field (emote button,
/// text, room-mode indicator, command button) and one round button that is Send while there is
/// text and the chat menu otherwise. Suggestions and the emote picker open upward over the list.
struct ChatComposer: View {
    let session: ChatSession
    @State private var model: ChatComposerModel?

    var body: some View {
        Group {
            if let model {
                ComposerBar(model: model, session: session)
            }
        }
        .onAppear {
            if model == nil { model = ChatComposerModel(session: session) }
            ComposerRegistry.shared.models[ObjectIdentifier(session)] = model
        }
    }
}

/// Composers on screen by session, for drive steps.
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
            inputRow
        }
        .padding(.bottom, 10)
        .overlay(alignment: .top) {
            VStack(spacing: 8) {
                if let toast = model.toast {
                    Text(toast)
                        .font(.footnote)
                        .foregroundStyle(.primary)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 7)
                        .glassEffect()
                        .transition(.opacity)
                }
                if model.panel == .emotes {
                    ChatEmotePicker(model: model)
                        .transition(.asymmetric(insertion: .opacity.combined(with: .offset(y: 10)).combined(with: .scale(scale: 0.98, anchor: .bottom)),
                                                removal: .opacity))
                } else if !model.suggestions.isEmpty {
                    SuggestionStrip(model: model) { focused = true }
                        .transition(.opacity.combined(with: .offset(y: 4)))
                }
            }
            .padding(.horizontal, 10)
            .alignmentGuide(.top) { $0[.bottom] + 8 }
            .animation(.spring(response: 0.3, dampingFraction: 0.85), value: model.panel)
            .animation(.easeOut(duration: 0.12), value: model.suggestions.isEmpty)
        }
        .sheet(isPresented: $model.menuOpen) {
            ChatMenuSheet(model: model, session: session)
                .presentationDetents([.medium])
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

    private func replyChip(_ reply: ChatMessage) -> some View {
        HStack(spacing: 6) {
            Text("Replying to \(Text("@\(reply.shownName)").fontWeight(.semibold))")
                .font(.system(size: 12.5))
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

    private var inputRow: some View {
        HStack(alignment: .bottom, spacing: 6) {
            HStack(spacing: 4) {
                if !session.style.hideEmoteButton {
                    emoteButton
                }
                TextField("", text: $model.draft, prompt: Text(model.placeholder).foregroundStyle(Color(ChatTheme.textMuted)))
                    .font(.system(size: 15))
                    .foregroundStyle(Color(ChatTheme.textPrimary))
                    .padding(.vertical, 8)
                    .padding(.leading, session.style.hideEmoteButton ? 8 : 0)
                    .submitLabel(.send)
                    .onSubmit { Task { await model.send() } }
                    .focused($focused)
                    .autocorrectionDisabled(!session.style.spellcheck)
                    .textInputAutocapitalization(.sentences)
                if let modes = session.modes, !modes.modes.isEmpty {
                    RoomModeIndicator(modes: modes) { model.menuOpen = true }
                }
                if !session.style.hideCommandButton {
                    Button {
                        model.commandsOpen = true
                    } label: {
                        Image(systemName: "slash.circle")
                            .font(.system(size: 17))
                            .foregroundStyle(Color(ChatTheme.textSecondary))
                            .frame(width: 32, height: 36)
                    }
                    .accessibilityLabel("Chat commands")
                }
            }
            .padding(.leading, 4)
            .padding(.trailing, 8)
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
                Image(systemName: model.canSend || model.sending ? "paperplane.fill" : "ellipsis")
                    .font(.system(size: 18, weight: .bold))
                    .foregroundStyle(model.canSend ? Color.white : Color(ChatTheme.textSecondary))
                    .frame(width: 40, height: 40)
                    .contentTransition(.symbolEffect(.replace))
            }
            .glassEffect(.regular.interactive(), in: .circle)
            .disabled(model.sending)
            .opacity(model.sending ? 0.5 : 1)
            .accessibilityLabel(model.canSend ? "Send" : "Chat options")
        }
        .padding(.horizontal, 10)
        .padding(.top, 8)
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
                        .font(.system(size: 20))
                        .opacity(smileyOut ? 0 : 1)
                        .scaleEffect(smileyOut ? 0.8 : 1)
                }
            }
            .frame(width: 36, height: 36)
        }
        .accessibilityLabel(model.panel == .emotes ? "Close emotes" : "Emotes and emoji")
    }
}

/// The room's modes inside the field: the first mode's symbol, and "+N" when more are on.
private struct RoomModeIndicator: View {
    let modes: ChatRoomModes
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            HStack(spacing: 4) {
                Image(systemName: Self.symbol(modes.modes[0].kind))
                    .font(.system(size: 13, weight: .semibold))
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

    static func symbol(_ kind: String) -> String {
        switch kind {
        case "subs_only": return "star"
        case "emote_only": return "face.smiling"
        case "followers_only": return "heart"
        case "slow": return "timer"
        default: return "touchid"
        }
    }
}

/// Emote and chatter suggestions for the last word, one swipe-snapped row.
private struct SuggestionStrip: View {
    let model: ChatComposerModel
    let focus: () -> Void
    @State private var active: ComposerSuggestion.ID?

    var body: some View {
        VStack(spacing: 0) {
            ScrollView(.horizontal, showsIndicators: false) {
                LazyHStack(spacing: 6) {
                    ForEach(model.suggestions) { suggestion in
                        Button {
                            model.complete(suggestion)
                            focus()
                        } label: {
                            tile(suggestion)
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(suggestion.name)
                        .id(suggestion.id)
                    }
                }
                .scrollTargetLayout()
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
            }
            .scrollTargetBehavior(.viewAligned)
            .scrollPosition(id: $active, anchor: .leading)
            HStack(spacing: 6) {
                let current = model.suggestions.first { $0.id == active } ?? model.suggestions.first
                Text(current?.name ?? "")
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(Color(ChatTheme.textPrimary))
                Text((current?.kind ?? "").uppercased())
                    .font(.system(size: 9))
                    .tracking(0.8)
                    .foregroundStyle(Color(ChatTheme.textMuted))
                Spacer()
                Text("Swipe, tap to use")
                    .font(.system(size: 10))
                    .foregroundStyle(Color(ChatTheme.textMuted))
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .overlay(alignment: .top) { Rectangle().fill(.white.opacity(0.05)).frame(height: 1) }
        }
        .background(Color(ChatTheme.background), in: .rect(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color(ChatTheme.borderSubtle)))
        .onChange(of: model.suggestions.first?.id) { _, first in active = first }
    }

    private func tile(_ suggestion: ComposerSuggestion) -> some View {
        let isActive = suggestion.id == (active ?? model.suggestions.first?.id)
        return Group {
            switch suggestion {
            case .emote(let emote):
                ChatImageView(url: ChatImageURL.thumbnail(emote.url), size: 36)
            case .chatter(let display, _):
                Text(display)
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(Color(ChatTheme.textSecondary))
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
                    .padding(2)
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
