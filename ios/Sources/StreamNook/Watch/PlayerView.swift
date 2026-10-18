import AVFoundation
import SwiftUI
import UIKit

/// An AVPlayerLayer as a SwiftUI view, handed to Picture in Picture.
///
/// AVFoundation pauses a player that is still attached to a layer when the app goes to the
/// background, so the layer lets go of the player there and takes it back on return; the
/// player itself keeps playing audio in between. When the video may float in Picture in Picture
/// the layer has to stay on the player for the window to take it, so it waits a second and lets
/// go only if no window came up (a screen lock never floats the video).
struct PlayerView: UIViewRepresentable {
    let player: AVPlayer
    let pip: PictureInPicture

    func makeUIView(context: Context) -> LayerView {
        let view = LayerView()
        view.attach(player, pip: pip)
        view.playerLayer.videoGravity = .resizeAspect
        view.backgroundColor = .black
        return view
    }

    func updateUIView(_ view: LayerView, context: Context) {
        view.attach(player, pip: pip)
    }

    final class LayerView: UIView {
        override static var layerClass: AnyClass { AVPlayerLayer.self }
        var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }

        private weak var player: AVPlayer?
        private weak var pip: PictureInPicture?
        private var letGo: Task<Void, Never>?
        nonisolated(unsafe) private var observers: [NSObjectProtocol] = []

        func attach(_ player: AVPlayer, pip: PictureInPicture) {
            self.player = player
            self.pip = pip
            if UIApplication.shared.applicationState != .background, playerLayer.player !== player {
                playerLayer.player = player
            }
            pip.attach(playerLayer)
            guard observers.isEmpty else { return }
            let center = NotificationCenter.default
            observers = [
                center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) {
                    [weak self] _ in MainActor.assumeIsolated { self?.backgrounded() }
                },
                center.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) {
                    [weak self] _ in
                    MainActor.assumeIsolated {
                        guard let self else { return }
                        self.letGo?.cancel()
                        self.playerLayer.player = self.player
                    }
                },
            ]
        }

        private func backgrounded() {
            guard pip?.mayFloat == true else {
                playerLayer.player = nil
                return
            }
            letGo?.cancel()
            letGo = Task { [weak self] in
                try? await Task.sleep(for: .seconds(1))
                guard let self, !Task.isCancelled, self.pip?.active != true,
                      UIApplication.shared.applicationState == .background else { return }
                self.playerLayer.player = nil
                log.notice("no pip window: layer let go for background audio")
            }
        }

        deinit {
            observers.forEach(NotificationCenter.default.removeObserver)
        }
    }
}
