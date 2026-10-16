import AVKit
import SNKit
import SwiftUI

/// Where a link tapped in chat goes first. A Twitch clip opens the clip sheet; anything else (or
/// no taker) opens outside the app.
@MainActor
enum LinkRouter {
    static var clip: ((URL) -> Bool)?

    static func open(_ url: URL) {
        if clip?(url) == true { return }
        UIApplication.shared.open(url)
    }

    /// A Twitch clip link: `clips.twitch.tv/<slug>` or `twitch.tv/<channel>/clip/<slug>`. The core
    /// decides for certain; a link it cannot resolve opens outside.
    static func isClip(_ url: URL) -> Bool {
        guard let host = url.host()?.lowercased() else { return false }
        if host == "clips.twitch.tv" { return url.pathComponents.count > 1 }
        return host.hasSuffix("twitch.tv") && url.pathComponents.contains("clip")
    }
}

/// A clip from chat in its own player, over everything: it resolves to a signed MP4 without
/// touching the live stream or chat, plays inline with native controls, and the live stream is
/// muted (not paused) only while the sheet is open.
struct ClipSheet: View {
    let url: URL
    let model: AppModel
    @State private var player: AVPlayer?
    @State private var wasMuted: Bool?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(spacing: 0) {
            SheetTitle(text: "Clip")
            ZStack {
                Color.black
                if let player {
                    VideoPlayer(player: player)
                } else {
                    PenroseMark(width: 40)
                }
            }
            .aspectRatio(16 / 9, contentMode: .fit)
            .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
            .padding(12)
            Spacer(minLength: 0)
        }
        .presentationDetents([.fraction(0.6)])
        .presentationDragIndicator(.visible)
        .task(id: url) { await load() }
        .onAppear {
            wasMuted = model.watch.player.isMuted
            if model.watch.stream != nil { model.watch.player.isMuted = true }
        }
        .onDisappear {
            player?.pause()
            player?.replaceCurrentItem(with: nil)
            player = nil
            if let wasMuted, model.watch.stream != nil { model.watch.player.isMuted = wasMuted }
        }
    }

    private func load() async {
        do {
            let start = try await model.core.invoke("resolve_clip_media", args: ["url": url.absoluteString, "quality": "best"],
                                                    as: StreamStart.self)
            guard let media = URL(string: start.url) else { throw CancellationError() }
            let player = AVPlayer(url: media)
            self.player = player
            player.play()
        } catch {
            log.notice("clip did not resolve, opening outside: \(String(describing: error), privacy: .public)")
            UIApplication.shared.open(url)
            dismiss()
        }
    }
}
