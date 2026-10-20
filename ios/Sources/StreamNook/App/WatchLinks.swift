import SNKit
import SwiftUI

extension AppModel {
    /// A StreamNook link the app is opened with (`streamnook://`, or a streamnook.app share link
    /// once one reaches the app): the core reads it as the desktop and Android deep-link handler
    /// does (`watch_link_key`), and the stream plays full, as a tapped go-live alert's does. A link
    /// that cold-starts the app waits for the core first.
    func openWatchLink(_ url: URL) {
        Task { @MainActor in
            guard await startCore(),
                  let key = try? await core.invoke("watch_link_key", args: ["url": url.absoluteString], as: String?.self),
                  !key.isEmpty else {
                log.notice("watch link: nothing to open in \(url.absoluteString, privacy: .public)")
                return
            }
            // A bare login is Twitch; another platform's key is `provider:channel`.
            let parts = key.split(separator: ":", maxSplits: 1).map(String.init)
            let provider = parts.count == 2 ? parts[0] : "twitch"
            let channel = parts.count == 2 ? parts[1] : key
            let other = provider != "twitch"
            let seed = TwitchStream(id: "", userLogin: channel.lowercased(), userName: channel, title: "", viewerCount: 0,
                                    gameName: "", provider: other ? provider : nil, key: other ? key : nil)
            startWatching(seed)
            if nav.playerMode != .full {
                withAnimation(SNMotion.run(SNMotion.settleBack)) { nav.playerMode = .full }
            }
        }
    }
}
