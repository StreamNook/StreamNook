import SNKit
import SwiftUI
import UIKit

/// The chat column of the watch page: the list, then the composer. The overlay variant is the
/// same list read-only, for immersive landscape over the video.
struct ChatPanel: View {
    let session: ChatSession
    var overlay = false
    /// Opens a chatter's profile (the profile sheet belongs to the profile screens).
    var onProfile: (String) -> Void = { _ in }

    @State private var toast: String?

    var body: some View {
        VStack(spacing: 0) {
            ChatListHost(session: session, readOnly: overlay, onProfile: onProfile, onToast: show)
                .overlay(alignment: .top) {
                    if let toast {
                        Text(toast)
                            .font(.footnote)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 7)
                            .glassEffect()
                            .padding(.top, 8)
                            .transition(.move(edge: .top).combined(with: .opacity))
                    }
                }
            if !overlay {
                ChatComposer(session: session)
            }
        }
        .onAppear { session.start() }
    }

    private func show(_ text: String) {
        withAnimation(.snappy) { toast = text }
        Task {
            try? await Task.sleep(for: .seconds(2.5))
            withAnimation(.snappy) { if toast == text { toast = nil } }
        }
    }
}

/// The UIKit chat list inside SwiftUI.
struct ChatListHost: UIViewRepresentable {
    let session: ChatSession
    var readOnly: Bool
    var onProfile: (String) -> Void
    var onToast: (String) -> Void

    func makeUIView(context: Context) -> ChatListView {
        let list = ChatListView()
        list.source = session.pipeline
        list.readOnly = readOnly
        let pipeline = session.pipeline
        list.onWidthChange = { [weak list] width in
            pipeline.setWidth(width, scale: list?.traitCollection.displayScale ?? 3)
        }
        list.onPausedChange = { paused in session.setPaused(paused) }
        list.onReply = { id in
            session.replyTo = session.pipeline.message(id)
        }
        list.onTap = { [weak list] action, layout in
            switch action {
            case .name:
                if let message = session.pipeline.message(layout.id), message.provider == "twitch" {
                    onProfile(message.username)
                }
            case .mention(let login):
                onProfile(login)
            case .link(let url):
                UIApplication.shared.open(url)
            case .replyParent(let id):
                if list?.jump(to: id) != true {
                    list?.pauseForJump()
                    onToast("Original message is no longer in chat history")
                }
            }
        }
        context.coordinator.list = list
        ChatRegistry.shared.lists[ObjectIdentifier(session)] = WeakList(list)
        return list
    }

    func updateUIView(_ list: ChatListView, context: Context) {
        list.readOnly = readOnly
        list.setStyle(session.style)
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        weak var list: ChatListView?
    }
}

/// Lists on screen by session, so drive steps and the composer can reach them.
@MainActor
final class ChatRegistry {
    static let shared = ChatRegistry()
    var lists: [ObjectIdentifier: WeakList] = [:]

    func list(for session: ChatSession) -> ChatListView? {
        lists[ObjectIdentifier(session)]?.list
    }
}

final class WeakList {
    weak var list: ChatListView?
    init(_ list: ChatListView) { self.list = list }
}
