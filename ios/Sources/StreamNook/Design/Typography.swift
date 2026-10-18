import CoreText
import SwiftUI
import UIKit

/// The interface font: the face the Font setting names. Satoshi is StreamNook's default; the
/// desktop's other faces ship too, cut to Latin and the 300 to 700 weights the app draws (all
/// SIL Open Font License, see Fonts/OFL.txt); "system" is SF Pro. An unknown id draws in Satoshi.
@MainActor
enum Typeface {
    /// Each bundled face: the Font setting's id and its file in Fonts/.
    private static let files = [
        "satoshi": "Satoshi-Variable", "twitch": "Inter-Variable", "geist": "Geist-Variable", "manrope": "Manrope-Variable",
        "outfit": "Outfit-Variable", "space-grotesk": "SpaceGrotesk-Variable", "serif": "Fraunces-Variable",
    ]
    /// The `wght` axis tag, as CoreText names variation axes.
    private static let weightAxis = 0x7767_6874
    /// Each registered face's PostScript and family names, read from its file.
    private static var faces: [String: (name: String, family: String)] = [:]
    private static var cache: [String: Font] = [:]
    private static var registered = false

    /// Registers the bundled fonts once, before the first view draws.
    static func register() {
        guard !registered else { return }
        registered = true
        for (id, file) in files {
            guard let url = Bundle.module.url(forResource: file, withExtension: "ttf", subdirectory: "Fonts") else {
                log.error("\(file, privacy: .public) is missing from the bundle")
                continue
            }
            CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
            guard let descriptor = (CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor])?.first,
                  let name = CTFontDescriptorCopyAttribute(descriptor, kCTFontNameAttribute) as? String,
                  let family = CTFontDescriptorCopyAttribute(descriptor, kCTFontFamilyNameAttribute) as? String else { continue }
            faces[id] = (name, family)
        }
    }

    /// The interface font at `size` points and a CSS weight (400 regular to 700 bold), in the
    /// chosen face or in `face` when given (a font tile drawn in its own face).
    static func font(_ size: CGFloat, _ weight: Int, face: String? = nil) -> Font {
        let id = resolved(face ?? Look.shared.fontID)
        let key = "\(id)|\(size)|\(weight)"
        if let known = cache[key] { return known }
        let made: Font = id == "system" ? .system(size: size, weight: Self.systemWeight(weight)) : Font(faceFont(id, size, weight))
        cache[key] = made
        return made
    }

    static func uiFont(_ size: CGFloat, _ weight: Int) -> UIFont {
        let id = resolved(Look.shared.fontID)
        return id == "system" ? .systemFont(ofSize: size, weight: uiWeight(weight)) : faceFont(id, size, weight)
    }

    private static func resolved(_ id: String?) -> String {
        guard let id else { return "satoshi" }
        return id == "system" || faces[id] != nil ? id : "satoshi"
    }

    private static func faceFont(_ id: String, _ size: CGFloat, _ weight: Int) -> UIFont {
        guard let face = faces[id] else { return .systemFont(ofSize: size, weight: uiWeight(weight)) }
        let base = UIFontDescriptor(fontAttributes: [.name: face.name])
        let variation = UIFontDescriptor.AttributeName(rawValue: kCTFontVariationAttribute as String)
        let descriptor = base.addingAttributes([variation: [weightAxis: max(300, weight)]])
        let font = UIFont(descriptor: descriptor, size: size)
        return font.familyName == face.family ? font : .systemFont(ofSize: size, weight: uiWeight(weight))
    }

    private static func systemWeight(_ css: Int) -> Font.Weight {
        switch css {
        case ..<350: .light
        case ..<450: .regular
        case ..<550: .medium
        case ..<650: .semibold
        case ..<750: .bold
        default: .heavy
        }
    }

    private static func uiWeight(_ css: Int) -> UIFont.Weight {
        switch css {
        case ..<350: .light
        case ..<450: .regular
        case ..<550: .medium
        case ..<650: .semibold
        case ..<750: .bold
        default: .heavy
        }
    }
}

/// Navigation titles in the interface font.
@MainActor
enum Appearance {
    static func apply() {
        let bar = UINavigationBar.appearance()
        bar.largeTitleTextAttributes = [.font: Typeface.uiFont(32, 700), .foregroundColor: UIColor.white]
        bar.titleTextAttributes = [.font: Typeface.uiFont(17, 700), .foregroundColor: UIColor.white]
    }
}

/// The phone's type scale, in points (CSS px map one to one).
@MainActor
extension Font {
    /// 14 regular: the base text size.
    static var snBody: Font { Typeface.font(14, 400) }
    /// 20 bold: a tab screen's title.
    static var snTabTitle: Font { Typeface.font(20, 700) }
    /// 18 bold: a drill-in screen's title.
    static var snDrillTitle: Font { Typeface.font(18, 700) }
    /// 24 bold: the signed-out landing's title.
    static var snHero: Font { Typeface.font(24, 700) }
    /// 16 semibold: a sheet's title, a step above the 15 point rows it heads.
    static var snSheetTitle: Font { Typeface.font(16, 600) }
    /// 12 semibold, set uppercase with wide tracking: a section label.
    static var snSection: Font { Typeface.font(12, 600) }
    /// 15 medium: a row's title.
    static var snRowTitle: Font { Typeface.font(15, 500) }
    /// 15 semibold: an account row's title and the primary buttons.
    static var snRowStrong: Font { Typeface.font(15, 600) }
    /// 12.5 regular: a row's description.
    static var snRowDetail: Font { Typeface.font(12.5, 400) }
    /// 14 semibold: a stream card's title, the card's anchor under the picture.
    static var snCardTitle: Font { Typeface.font(14, 600) }
    /// 13 regular: a card's channel line.
    static var snCardLine: Font { Typeface.font(13, 400) }
    /// 12.5 regular: a card's quietest line (the category, the viewers in a row).
    static var snCardMeta: Font { Typeface.font(12.5, 400) }
    /// 12 regular.
    static var snCaption: Font { Typeface.font(12, 400) }
    /// 11 semibold: the card chips.
    static var snChip: Font { Typeface.font(11, 600) }
    /// 14 medium: segment labels and toasts.
    static var snLabel: Font { Typeface.font(14, 500) }
    /// 14 semibold: the chosen segment.
    static var snLabelStrong: Font { Typeface.font(14, 600) }

    static func sn(_ size: CGFloat, _ weight: Int = 400) -> Font { Typeface.font(size, weight) }
}

extension View {
    /// A section label: 12 semibold, uppercase, wide tracking, muted.
    @MainActor
    func snSectionStyle() -> some View {
        font(.snSection).textCase(.uppercase).tracking(0.6).foregroundStyle(SN.textMuted)
    }
}
