import SNKit
import SwiftUI

/// Where the watch layer puts chat: below the player in portrait, and as the landscape column
/// (read-only there: no composer). This stand-in lists the chat socket's last lines until the
/// chat list and composer replace its body.
struct ChatSlot: View {
    let model: AppModel
    /// The landscape overlay shows chat without the composer.
    let readOnly: Bool

    private var watch: Watch { model.watch }

    var body: some View {
        ScrollViewReader { reader in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 6) {
                    ForEach(Array(watch.frames.frames.enumerated()), id: \.offset) { index, frame in
                        line(frame).id(index)
                    }
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
            }
            .scrollIndicators(.hidden)
            .defaultScrollAnchor(.bottom)
            .onChange(of: watch.frames.frames.count) { _, count in
                reader.scrollTo(count - 1, anchor: .bottom)
            }
        }
        .overlay {
            if watch.frames.frames.isEmpty {
                Text(watch.chatState)
                    .font(.snCaption)
                    .foregroundStyle(SN.textMuted)
            }
        }
    }

    @ViewBuilder
    private func line(_ frame: ChatFrame) -> some View {
        if frame.kind == .message {
            let author = Text(frame.author ?? "?").font(.sn(13, 700)).foregroundStyle(SN.textSecondary)
            let words = Text(frame.text).font(.sn(13, 400)).foregroundStyle(SN.textPrimary)
            Text("\(author)  \(words)").lineLimit(3)
        } else {
            Text(frame.summary)
                .font(.sn(11, 400))
                .foregroundStyle(SN.textMuted)
                .lineLimit(1)
        }
    }
}
