import SwiftUI
import UIKit

/// A chat image (an emote or badge) in SwiftUI, through the shared image pipeline: its first
/// frame, decoded off the main thread at the size it is drawn.
struct ChatImageView: View {
    let url: String
    var size: CGFloat = 32
    @Environment(\.displayScale) private var scale
    @State private var image: CGImage?

    var body: some View {
        Group {
            if let image {
                Image(decorative: image, scale: scale)
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: .fit)
            } else {
                Color.clear
            }
        }
        .frame(width: size, height: size)
        .task(id: url) {
            let pixels = Int(size * scale)
            if let hit = ImagePipeline.shared.cached(url, pixelHeight: pixels, animated: false) {
                image = hit.first
                return
            }
            image = nil
            let decoded: DecodedImage? = await withCheckedContinuation { continuation in
                ImagePipeline.shared.load(url, pixelHeight: pixels, animated: false) { continuation.resume(returning: $0) }
            }
            image = decoded?.first
        }
    }
}
