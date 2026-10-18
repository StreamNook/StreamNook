import SwiftUI

/// A reward, badge or box-art picture through the shared image pipeline, filling its frame. While
/// it loads, or with no picture, the frame shows the surface with `placeholder` in it.
struct RewardArt: View {
    let url: String
    var width: CGFloat
    var height: CGFloat
    var radius: CGFloat = 6
    /// Fit the whole picture (a badge) instead of filling the frame (art).
    var fit = false
    var placeholder: String? = Symbol.giftOutline
    var ring = false
    @Environment(\.displayScale) private var scale
    @State private var image: CGImage?

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        ZStack {
            if let image {
                Image(decorative: image, scale: scale)
                    .resizable()
                    .interpolation(.high)
                    .aspectRatio(contentMode: fit ? .fit : .fill)
                    .frame(width: width, height: height)
            } else if !fit {
                SN.surface
                if let placeholder {
                    Image(systemName: placeholder)
                        .font(.system(size: min(width, height) * 0.36, weight: .regular))
                        .foregroundStyle(SN.textMuted)
                }
            }
        }
        .frame(width: width, height: height)
        .clipShape(shape)
        .overlay { if ring && image != nil { shape.strokeBorder(.white.opacity(0.1), lineWidth: 1) } }
        .accessibilityHidden(true)
        .task(id: url) {
            guard !url.isEmpty else {
                image = nil
                return
            }
            let pixels = Int(max(width, height) * scale)
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
