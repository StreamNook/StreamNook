import AVFoundation
import SwiftUI
import UIKit

/// An AVPlayerLayer as a SwiftUI view.
///
/// AVFoundation pauses a player that is still attached to a layer when the app goes to the
/// background, so the layer lets go of the player there and takes it back on return; the
/// player itself keeps playing audio in between.
struct PlayerView: UIViewRepresentable {
    let player: AVPlayer

    func makeUIView(context: Context) -> LayerView {
        let view = LayerView()
        view.attach(player)
        view.playerLayer.videoGravity = .resizeAspect
        view.backgroundColor = .black
        return view
    }

    func updateUIView(_ view: LayerView, context: Context) {
        view.attach(player)
    }

    final class LayerView: UIView {
        override static var layerClass: AnyClass { AVPlayerLayer.self }
        var playerLayer: AVPlayerLayer { layer as! AVPlayerLayer }

        private weak var player: AVPlayer?
        nonisolated(unsafe) private var observers: [NSObjectProtocol] = []

        func attach(_ player: AVPlayer) {
            self.player = player
            if UIApplication.shared.applicationState != .background, playerLayer.player !== player {
                playerLayer.player = player
            }
            guard observers.isEmpty else { return }
            let center = NotificationCenter.default
            observers = [
                center.addObserver(forName: UIApplication.didEnterBackgroundNotification, object: nil, queue: .main) {
                    [weak self] _ in MainActor.assumeIsolated { self?.playerLayer.player = nil }
                },
                center.addObserver(forName: UIApplication.willEnterForegroundNotification, object: nil, queue: .main) {
                    [weak self] _ in
                    MainActor.assumeIsolated {
                        guard let self else { return }
                        self.playerLayer.player = self.player
                    }
                },
            ]
        }

        deinit {
            observers.forEach(NotificationCenter.default.removeObserver)
        }
    }
}
