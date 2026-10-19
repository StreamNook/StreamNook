import SwiftUI
import UIKit

/// A platform's or badge service's own logo beside its name, from the desktop's art: the vector
/// marks `ProviderLogo` draws (Twitch, Kick, 7TV, BTTV, FFZ) in their brand color, the operating
/// systems' one-color marks (`SystemMark`), the rest as the bundled images in `Logos/` (YouTube,
/// TikTok, Chatterino, Homies, Moltorino, Chatsen, Chatty, DankChat, StreamNook).
struct BrandLogo: View {
    let id: String
    var size: CGFloat = 16

    var body: some View {
        switch id {
        case "twitch", "kick", "7tv", "seventv", "bttv", "ffz":
            ProviderLogo(provider: id == "seventv" ? "7tv" : id, size: size, tinted: true)
        // One color, in the title's own.
        case "windows": SystemMark(platform: .windows, size: size).foregroundStyle(SN.textPrimary)
        case "apple", "macos", "ios": SystemMark(platform: .apple, size: size).foregroundStyle(SN.textPrimary)
        case "linux": SystemMark(platform: .linux, size: size).foregroundStyle(SN.textPrimary)
        case "android": SystemMark(platform: .android, size: size).foregroundStyle(SN.textPrimary)
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
