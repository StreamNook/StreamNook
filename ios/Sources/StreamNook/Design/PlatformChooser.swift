import SwiftUI
import UIKit

/// One platform choice: a value the screen stores (`""` for every platform) and its marks.
struct PlatformOption: Identifiable, Hashable {
    let value: String
    let name: String
    let marks: [PlatformMark.Platform]

    var id: String { value }

    static let twitch = PlatformOption(value: "twitch", name: "Twitch", marks: [.twitch])
    static let kick = PlatformOption(value: "kick", name: "Kick", marks: [.kick])
    static let all = PlatformOption(value: "", name: "All platforms", marks: [.twitch, .kick])
}

/// The platform marks drawn in their own colors as one image, for menu items (a menu tints a
/// symbol, never an original-rendered image).
@MainActor
enum PlatformLogoImage {
    private static var cache: [String: UIImage] = [:]

    static func image(_ marks: [PlatformMark.Platform], size: CGFloat = 18) -> UIImage {
        let key = "\(marks.map(\.name).joined(separator: "+"))@\(size)"
        if let hit = cache[key] { return hit }
        let renderer = ImageRenderer(content: PlatformMarks(marks: marks, size: size))
        renderer.scale = UITraitCollection.current.displayScale
        let image = (renderer.uiImage ?? UIImage()).withRenderingMode(.alwaysOriginal)
        cache[key] = image
        return image
    }
}

/// One or more marks side by side; two marks overlap a little, as a pair.
struct PlatformMarks: View {
    let marks: [PlatformMark.Platform]
    var size: CGFloat = 18

    var body: some View {
        HStack(spacing: marks.count > 1 ? -size * 0.12 : 0) {
            ForEach(marks.indices, id: \.self) { i in
                PlatformMark(platform: marks[i], size: marks.count > 1 ? size * 0.82 : size)
            }
        }
    }
}

/// The platform choice in a screen's title row, so it spends no row of its own: a menu whose label
/// is the chosen platform's logo with a small chevron saying it opens. The menu lists every option
/// with its logo and a check on the chosen one.
struct PlatformMenu: View {
    let options: [PlatformOption]
    @Binding var selection: String
    var size: CGFloat = 18

    private var chosen: PlatformOption { options.first { $0.value == selection } ?? options[0] }

    var body: some View {
        Menu {
            Picker("Platform", selection: $selection) {
                ForEach(options) { option in
                    Label {
                        Text(option.name)
                    } icon: {
                        Image(uiImage: PlatformLogoImage.image(option.marks))
                    }
                    .tag(option.value)
                }
            }
            .pickerStyle(.inline)
        } label: {
            HStack(spacing: 4) {
                PlatformMarks(marks: chosen.marks, size: size)
                Image(systemName: "chevron.down")
                    .font(.system(size: 9, weight: .bold))
                    .foregroundStyle(SN.textSecondary)
            }
            .frame(minWidth: 44, minHeight: 36)
            .contentShape(Rectangle())
        }
        .accessibilityLabel("Platform: \(chosen.name)")
        .accessibilityHint("Choose which platform to show")
    }
}
