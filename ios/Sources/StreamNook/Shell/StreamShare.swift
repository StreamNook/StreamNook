import SNKit
import SwiftUI

/// The link a share button hands out for a stream, built by the core as on desktop and Android
/// (`share_link`): a streamnook.app link that opens StreamNook where it is installed and the
/// platform's own page where it is not.
enum StreamShare {
    @MainActor
    static func link(for stream: TwitchStream, core: CoreBridge) async -> URL? {
        let args: [String: Any?] = [
            "provider": stream.platform, "channel": stream.userLogin, "streamId": stream.id.isEmpty ? nil : stream.id,
        ]
        guard let text = try? await core.invoke("share_link", args: args, as: String.self) else { return nil }
        return URL(string: text)
    }

    /// The channel's own page, handed out until the core's link is known.
    static func fallback(_ stream: TwitchStream) -> URL {
        stream.channelLink ?? URL(string: "https://www.twitch.tv/\(stream.userLogin)")!
    }
}

/// A stream's share button, carrying the core's link once it is read (`streamShareLink`).
struct StreamShareLink<Label: View>: View {
    let stream: TwitchStream
    var subject: Text?
    var message: Text?
    @ViewBuilder var label: () -> Label
    @Environment(\.streamShareLink) private var shareLink
    @State private var link: URL?

    var body: some View {
        ShareLink(item: link ?? StreamShare.fallback(stream), subject: subject, message: message, label: label)
            .task(id: stream.rowKey) { link = await shareLink?(stream) }
    }
}
