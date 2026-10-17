import SwiftUI
import UIKit

/// Where a screen puts its platform choice. `row` is the original segmented row; the others spend
/// no row of their own: `toolbar` is a logo button in the title row, `search` is the logo at the
/// search field's leading edge (Browse), `capsule` is the logo as the first slot of the mode
/// capsule (Browse). Following has no search field or mode capsule, so every style but `row` puts
/// its filter in the title row.
enum PlatformChooserStyle: String, CaseIterable {
    case row, toolbar, search, capsule

    static let key = "sn-platform-chooser"
}

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

/// The platform choice as a menu whose label is the chosen platform's logo, with a small chevron
/// saying it opens. The menu lists every option with its logo and a check on the chosen one.
struct PlatformMenu: View {
    let options: [PlatformOption]
    @Binding var selection: String
    var size: CGFloat = 18
    /// The chevron after the logo; off where the surrounding control already reads as a menu.
    var chevron = true

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
                if chevron {
                    Image(systemName: "chevron.down")
                        .font(.system(size: 9, weight: .bold))
                        .foregroundStyle(SN.textSecondary)
                }
            }
            .frame(minWidth: 44, minHeight: 36)
            .contentShape(Rectangle())
        }
        .accessibilityLabel("Platform: \(chosen.name)")
        .accessibilityHint("Choose which platform to show")
    }
}

/// Option B: a search field whose leading edge is the platform menu. It stands where the system
/// search bar stood, so the choice costs no row.
struct PlatformSearchField: View {
    let options: [PlatformOption]
    @Binding var platform: String
    @Binding var query: String
    let prompt: String
    @FocusState private var focused: Bool

    var body: some View {
        HStack(spacing: 0) {
            PlatformMenu(options: options, selection: $platform, size: 17)
                .padding(.leading, 6)
            Rectangle()
                .fill(SN.textMuted.opacity(0.35))
                .frame(width: 1, height: 20)
                .padding(.trailing, 10)
            Image(systemName: "magnifyingglass")
                .font(.system(size: 15, weight: .medium))
                .foregroundStyle(SN.textSecondary)
                .padding(.trailing, 6)
            TextField("", text: $query, prompt: Text(prompt).foregroundStyle(SN.textSecondary))
                .font(.snRowTitle)
                .foregroundStyle(SN.textPrimary)
                .focused($focused)
                .submitLabel(.search)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
            if !query.isEmpty {
                Button { query = "" } label: {
                    Image(systemName: "xmark.circle.fill")
                        .font(.system(size: 16))
                        .foregroundStyle(SN.textSecondary)
                        .frame(width: 36, height: 36)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Clear search")
            }
        }
        .padding(.trailing, 6)
        .frame(height: 44)
        .snGlass(Capsule())
    }
}
