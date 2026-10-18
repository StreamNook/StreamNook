import MediaPlayer
import UIKit

/// The lock screen and Control Center card for the stream being watched: the title (or the
/// channel when untitled), the channel's name and avatar, marked live so no seek controls show.
/// The player is the source of truth: a card button only sends a command, and the card's
/// play/pause follows what the player then does. Stop ends the stream (headsets, CarPlay, Siri;
/// the lock screen itself has no Stop slot).
@MainActor
final class NowPlaying {
    var onPlay: (() -> Void)?
    var onPause: (() -> Void)?
    var onStop: (() -> Void)?

    private var info: [String: Any] = [:]
    private var avatar: URL?
    private var artwork: Task<Void, Never>?
    private var commandsReady = false

    /// Publishes (or repaints in place) the card for a stream.
    func show(title: String, channel: String, avatar: URL?) {
        setUpCommands()
        info[MPMediaItemPropertyTitle] = title.isEmpty ? channel : title
        info[MPMediaItemPropertyArtist] = channel
        info[MPNowPlayingInfoPropertyIsLiveStream] = true
        info[MPNowPlayingInfoPropertyMediaType] = MPNowPlayingInfoMediaType.video.rawValue
        if info[MPNowPlayingInfoPropertyPlaybackRate] == nil { info[MPNowPlayingInfoPropertyPlaybackRate] = 1.0 }
        publish()
        guard avatar != self.avatar else { return }
        self.avatar = avatar
        info[MPMediaItemPropertyArtwork] = nil
        artwork?.cancel()
        guard let avatar else { return }
        artwork = Task { [weak self] in
            guard let (data, _) = try? await URLSession.shared.data(from: avatar), !Task.isCancelled,
                  let image = UIImage(data: data) else { return }
            self?.setArtwork(image)
        }
    }

    /// The card's play/pause, from what the player is doing.
    func playing(_ playing: Bool) {
        guard !info.isEmpty else { return }
        info[MPNowPlayingInfoPropertyPlaybackRate] = playing ? 1.0 : 0.0
        publish()
    }

    /// The stream ended or closed: the card goes away.
    func clear() {
        artwork?.cancel()
        artwork = nil
        avatar = nil
        info = [:]
        MPNowPlayingInfoCenter.default().nowPlayingInfo = nil
    }

    private func setArtwork(_ image: UIImage) {
        guard !info.isEmpty else { return }
        // MediaPlayer asks for the picture on its own queue: the closure must not be main-actor code.
        info[MPMediaItemPropertyArtwork] = MPMediaItemArtwork(boundsSize: image.size) { @Sendable _ in image }
        publish()
    }

    private func toggle() {
        let rate = info[MPNowPlayingInfoPropertyPlaybackRate] as? Double ?? 0
        if rate > 0 { onPause?() } else { onPlay?() }
    }

    private func publish() {
        MPNowPlayingInfoCenter.default().nowPlayingInfo = info
    }

    /// Play, pause, toggle and stop answer; every seek and skip command is off, since live has
    /// nowhere to seek to. A command may arrive on any queue, so each handler is non-isolated and
    /// hops to the main actor.
    private func setUpCommands() {
        guard !commandsReady else { return }
        commandsReady = true
        let center = MPRemoteCommandCenter.shared()
        center.playCommand.addTarget { @Sendable [weak self] _ in
            Task { @MainActor in self?.onPlay?() }
            return .success
        }
        center.pauseCommand.addTarget { @Sendable [weak self] _ in
            Task { @MainActor in self?.onPause?() }
            return .success
        }
        center.togglePlayPauseCommand.addTarget { @Sendable [weak self] _ in
            Task { @MainActor in self?.toggle() }
            return .success
        }
        center.stopCommand.addTarget { @Sendable [weak self] _ in
            Task { @MainActor in self?.onStop?() }
            return .success
        }
        for command in [center.changePlaybackPositionCommand, center.skipForwardCommand, center.skipBackwardCommand,
                        center.seekForwardCommand, center.seekBackwardCommand, center.nextTrackCommand,
                        center.previousTrackCommand, center.changePlaybackRateCommand] {
            command.isEnabled = false
        }
    }
}
