import AVKit
import UIKit

/// System Picture in Picture for the player layer. Nothing in the app enters it on purpose:
/// leaving the app while a stream plays floats the video by itself, whatever Background play
/// says. Expanding the window returns to the full watch layer. Closing it (the window's X) ends
/// the stream, unless Background play is on: then the layer lets go of the player and the audio
/// plays on with its Lock Screen controls.
@MainActor
final class PictureInPicture: NSObject {
    /// Background play is on (`video_player.background_mode` "audio"): closing the window keeps
    /// the audio going instead of ending the stream.
    var keepsAudio = false
    /// A stream with a picture is playing here, so there is something to float.
    var eligible = false { didSet { update() } }
    /// The window is up.
    private(set) var active = false

    var onRestore: (() -> Void)?
    /// The window's X while Background play is off: the stream ends.
    var onClose: (() -> Void)?
    /// The window's X while Background play is on: the stream plays on as audio.
    var onCloseToAudio: (() -> Void)?

    private var controller: AVPictureInPictureController?
    private weak var layer: AVPlayerLayer?
    private var starting = false
    private var restoring = false
    private var returning: NSObjectProtocol?

    override init() {
        super.init()
        // Coming back to the app brings the video back into it: the window closes into the
        // player (a restore, so the stream plays on).
        returning = NotificationCenter.default.addObserver(forName: UIApplication.didBecomeActiveNotification,
                                                           object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.active, let controller = self.controller else { return }
                self.restoring = true
                controller.stopPictureInPicture()
            }
        }
    }

    /// Takes the layer the player draws into; called once per player view.
    func attach(_ layer: AVPlayerLayer) {
        guard layer !== self.layer, AVPictureInPictureController.isPictureInPictureSupported() else { return }
        self.layer = layer
        let controller = AVPictureInPictureController(playerLayer: layer)
        controller?.delegate = self
        // Live has nowhere to skip to, so the window shows no skip buttons.
        controller?.requiresLinearPlayback = true
        self.controller = controller
        update()
    }

    /// Whether leaving the app now may float the video, so the layer must stay on the player.
    var mayFloat: Bool {
        active || starting || (eligible && controller?.isPictureInPicturePossible == true)
    }

    private func update() {
        controller?.canStartPictureInPictureAutomaticallyFromInline = eligible
    }
}

extension PictureInPicture: @preconcurrency AVPictureInPictureControllerDelegate {
    func pictureInPictureControllerWillStartPictureInPicture(_ controller: AVPictureInPictureController) {
        starting = true
    }

    func pictureInPictureControllerDidStartPictureInPicture(_ controller: AVPictureInPictureController) {
        starting = false
        active = true
        log.notice("pip started")
    }

    func pictureInPictureController(_ controller: AVPictureInPictureController,
                                    failedToStartPictureInPictureWithError error: any Error) {
        starting = false
        log.error("pip failed to start: \(String(describing: error), privacy: .public)")
    }

    func pictureInPictureController(_ controller: AVPictureInPictureController,
                                    restoreUserInterfaceForPictureInPictureStopWithCompletionHandler completionHandler: @escaping (Bool) -> Void) {
        restoring = true
        onRestore?()
        completionHandler(true)
    }

    /// A stop with no restore while the app is away is the window's X: the stream ends, or with
    /// Background play on, carries on as audio. AVPlayer pauses a backgrounded player that is
    /// still on a layer, so the layer lets go first; it takes the player back on return.
    func pictureInPictureControllerDidStopPictureInPicture(_ controller: AVPictureInPictureController) {
        active = false
        let closed = !restoring && UIApplication.shared.applicationState != .active
        restoring = false
        log.notice("pip stopped (\(closed ? (keepsAudio ? "closed to audio" : "closed") : "restored", privacy: .public))")
        guard closed else { return }
        if keepsAudio {
            layer?.player = nil
            onCloseToAudio?()
        } else {
            onClose?()
        }
    }
}
