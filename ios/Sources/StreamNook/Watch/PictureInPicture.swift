import AVKit
import UIKit

/// System Picture in Picture for the player layer. Nothing in the app enters it on purpose:
/// leaving the app while a stream plays floats the video by itself, unless the viewer chose
/// audio only for the background. Expanding the window returns to the full watch layer; closing
/// it (the window's X) ends the stream, so it does not come back on the next open.
@MainActor
final class PictureInPicture: NSObject {
    /// Leaving the app floats the video (the `video_player.background_mode` setting is not "audio").
    var floats = true { didSet { update() } }
    /// A stream is playing here, so there is something to float.
    var eligible = false { didSet { update() } }
    /// The window is up.
    private(set) var active = false

    var onRestore: (() -> Void)?
    var onClose: (() -> Void)?

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
        active || starting || (floats && eligible && controller?.isPictureInPicturePossible == true)
    }

    private func update() {
        controller?.canStartPictureInPictureAutomaticallyFromInline = floats && eligible
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

    /// A stop with no restore while the app is away is the window's X: the stream ends.
    func pictureInPictureControllerDidStopPictureInPicture(_ controller: AVPictureInPictureController) {
        active = false
        let closed = !restoring && UIApplication.shared.applicationState != .active
        restoring = false
        log.notice("pip stopped (\(closed ? "closed" : "restored", privacy: .public))")
        if closed { onClose?() }
    }
}
