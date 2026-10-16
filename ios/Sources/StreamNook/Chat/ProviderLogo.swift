import SNKit
import SwiftUI

/// An emote provider's own logo, drawn from the same artwork the desktop's emote picker uses
/// (src/components/chat/EmoteProviderLogo.tsx), so a provider is never just a word. In its brand
/// color, or in the color around it.
struct ProviderLogo: View {
    let provider: String
    var size: CGFloat = 16
    /// Paints the logo in the provider's brand color; otherwise it takes the foreground style.
    var tinted = false

    var body: some View {
        if let art = ProviderArt.art(provider) {
            let shape = ProviderShape(art: art)
            Group {
                if tinted, let brand = ProviderArt.brand(provider) {
                    shape.fill(brand, style: FillStyle(eoFill: art.evenOdd))
                } else {
                    shape.fill(.foreground, style: FillStyle(eoFill: art.evenOdd))
                }
            }
            .frame(width: size * art.aspect, height: size)
            .accessibilityLabel(PickerTabs.title(provider))
        }
    }
}

/// The artwork behind each logo: SVG path data in its own view box.
struct ProviderArt {
    let width: Double
    let height: Double
    let minX: Double
    let minY: Double
    let paths: [String]
    var evenOdd = false

    var aspect: CGFloat { CGFloat(width / height) }

    static func art(_ provider: String) -> ProviderArt? {
        switch provider {
        case "twitch": return twitch
        case "bttv": return bttv
        case "7tv": return sevenTV
        case "ffz": return ffz
        case "kick": return kick
        default: return nil
        }
    }

    /// Brand colors, matching the picker's active tab tints.
    static func brand(_ provider: String) -> Color? {
        switch provider {
        case "twitch": return Color(red: 0x91 / 255, green: 0x47 / 255, blue: 1)
        case "bttv": return Color(red: 0xD5 / 255, green: 0, blue: 0x14 / 255)
        case "7tv": return Color(red: 0x29 / 255, green: 0xB6 / 255, blue: 0xF6 / 255)
        // FrankerFaceZ's brand is white, which only reads on a dark background: it takes the
        // foreground instead, so it shows in both themes.
        case "kick": return Color(red: 0x53 / 255, green: 0xFC / 255, blue: 0x18 / 255)
        default: return nil
        }
    }

    static let twitch = ProviderArt(width: 24, height: 24, minX: 0, minY: 0, paths: [
        "M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z",
    ], evenOdd: true)

    static let bttv = ProviderArt(width: 300, height: 300, minX: 0, minY: 0, paths: [
        "M150 1.74C68.409 1.74 1.74 68.41 1.74 150S68.41 298.26 150 298.26h148.26V150.17h-.004c0-.057.004-.113.004-.17C298.26 68.409 231.59 1.74 150 1.74zm0 49c55.11 0 99.26 44.15 99.26 99.26 0 55.11-44.15 99.26-99.26 99.26-55.11 0-99.26-44.15-99.26-99.26 0-55.11 44.15-99.26 99.26-99.26z",
        "M161.388 70.076c-10.662 0-19.42 7.866-19.42 17.67 0 9.803 8.758 17.67 19.42 17.67 10.662 0 19.42-7.867 19.42-17.67 0-9.804-8.758-17.67-19.42-17.67zm45.346 24.554-.02.022-.004.002c-5.402 2.771-11.53 6.895-18.224 11.978l-.002.002-.004.002c-25.943 19.766-60.027 54.218-80.344 80.33h-.072l-1.352 1.768c-5.114 6.69-9.267 12.762-12.098 18.006l-.082.082.022.021v.002l.004.002.174.176.052-.053.102.053-.07.072c30.826 30.537 81.213 30.431 111.918-.273 30.783-30.784 30.8-81.352.04-112.152l-.005-.004zM87.837 142.216c-9.803 0-17.67 8.758-17.67 19.42 0 10.662 7.867 19.42 17.67 19.42 9.804 0 17.67-8.758 17.67-19.42 0-10.662-7.866-19.42-17.67-19.42z",
    ])

    static let sevenTV = ProviderArt(width: 28, height: 21, minX: 0, minY: 0, paths: [
        "M20.7465 5.48825L21.9799 3.33745L22.646 2.20024L21.4125 0.0494437V0H14.8259L17.2928 4.3016L17.9836 5.48825H20.7465Z",
        "M7.15395 19.9258L14.5546 7.02104L15.4673 5.43884L13.0004 1.13724L12.3097 0.0247596H1.8995L0.666057 2.17556L0 3.31276L1.23344 5.46356V5.51301H9.12745L2.96025 16.267L2.09685 17.7998L3.33029 19.9506V20H7.15395",
        "M17.4655 19.9257H21.2398L26.1736 11.3225L27.037 9.83924L25.8036 7.68844V7.63899H22.0046L19.5377 11.9406L19.365 12.262L16.8981 7.96038L16.7255 7.63899L14.2586 11.9406L13.5679 13.1272L17.2682 19.5796L17.4655 19.9257Z",
    ])

    static let ffz = ProviderArt(width: 40, height: 30, minX: -0.5, minY: -0.5, paths: [
        "M 15.5,-0.5 C 17.8333,-0.5 20.1667,-0.5 22.5,-0.5C 24.6552,3.13905 26.8218,6.80572 29,10.5C 29.691,7.40943 31.5243,6.24276 34.5,7C 36.585,9.68221 38.2517,12.5155 39.5,15.5C 39.5,17.5 39.5,19.5 39.5,21.5C 34.66,25.2533 29.3267,27.92 23.5,29.5C 20.5,29.5 17.5,29.5 14.5,29.5C 9.11466,27.3005 4.11466,24.3005 -0.5,20.5C -0.5,17.5 -0.5,14.5 -0.5,11.5C 4.17691,4.45967 7.34358,5.12633 9,13.5C 10.6047,10.3522 11.6047,7.01889 12,3.5C 12.6897,1.64977 13.8564,0.316435 15.5,-0.5 Z",
    ])

    static let kick = ProviderArt(width: 24, height: 24, minX: 0, minY: 0, paths: [
        "M1.333 0h8v5.333H12V2.667h2.667V0h8v8H20v2.667h-2.667v2.666H20V16h2.667v8h-8v-2.667H12v-2.666H9.333V24h-8Z",
    ])

    /// The artwork as one path in its view box, read once.
    var path: Path {
        ProviderArt.cache.withLock { cache in
            let key = paths.joined()
            if let hit = cache[key] { return hit }
            var p = Path()
            for data in paths {
                for step in VectorPath.parse(data) {
                    switch step {
                    case .move(let to): p.move(to: CGPoint(x: to.x, y: to.y))
                    case .line(let to): p.addLine(to: CGPoint(x: to.x, y: to.y))
                    case .cubic(let c1, let c2, let to):
                        p.addCurve(to: CGPoint(x: to.x, y: to.y), control1: CGPoint(x: c1.x, y: c1.y),
                                   control2: CGPoint(x: c2.x, y: c2.y))
                    case .quad(let c, let to):
                        p.addQuadCurve(to: CGPoint(x: to.x, y: to.y), control: CGPoint(x: c.x, y: c.y))
                    case .close: p.closeSubpath()
                    }
                }
            }
            cache[key] = p
            return p
        }
    }

    private static let cache = ArtCache<[String: Path]>([:])
}

/// A small lock around a value, for caches read from any thread.
fileprivate final class ArtCache<Value>: @unchecked Sendable {
    private var value: Value
    private let lock = NSLock()

    init(_ value: Value) {
        self.value = value
    }

    func withLock<R>(_ body: (inout Value) -> R) -> R {
        lock.lock()
        defer { lock.unlock() }
        return body(&value)
    }
}

private struct ProviderShape: Shape {
    let art: ProviderArt

    func path(in rect: CGRect) -> Path {
        let scale = min(rect.width / art.width, rect.height / art.height)
        let transform = CGAffineTransform(translationX: rect.minX - art.minX * scale, y: rect.minY - art.minY * scale)
            .scaledBy(x: scale, y: scale)
        return art.path.applying(transform)
    }
}
