import SwiftUI
import UIKit

/// A platform's or badge service's own logo beside its name, from the desktop's art: Twitch and
/// Kick as vector marks, the rest as the bundled images in `Logos/` (YouTube, TikTok, 7TV, BTTV,
/// FFZ, Chatterino, Homies, Moltorino, Chatsen, Chatty, DankChat, StreamNook).
struct ProviderLogo: View {
    let id: String
    var size: CGFloat = 16

    var body: some View {
        switch id {
        case "twitch": PlatformMark(platform: .twitch, size: size)
        case "kick": PlatformMark(platform: .kick, size: size)
        default:
            if let image = Self.image(id) {
                Image(uiImage: image).resizable().interpolation(.high).scaledToFit()
                    .frame(width: size, height: size)
                    .accessibilityHidden(true)
            }
        }
    }

    @MainActor private static var cache: [String: UIImage] = [:]

    @MainActor static func image(_ id: String) -> UIImage? {
        let file = switch id {
        case "homies-custom": "homies"
        case "seventv": "7tv"
        default: id
        }
        if let known = cache[file] { return known }
        guard let url = Bundle.module.url(forResource: file, withExtension: "png", subdirectory: "Logos"),
              let image = UIImage(contentsOfFile: url.path) else { return nil }
        cache[file] = image
        return image
    }
}
