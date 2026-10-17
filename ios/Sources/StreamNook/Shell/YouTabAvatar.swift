import Observation
import SNKit
import SwiftUI
import UIKit

/// The You tab's icon as your picture: the one your You card shows (your 7TV picture when you have
/// one, else your Twitch picture), as a small circle drawn once into original-color images, since a
/// tab bar would otherwise flatten it into a template. Selected, a thin accent ring sits around it.
/// Signed out, or until the picture has loaded, the tab keeps its person glyph; a new picture
/// replaces the old one only once it is drawn, so the icon never flashes empty.
@Observable @MainActor
final class YouTabAvatar {
    /// Your picture as a plain circle, and inside the selected ring.
    private(set) var plain: UIImage?
    private(set) var ringed: UIImage?
    @ObservationIgnored private var shownURL: String?
    @ObservationIgnored private var accent: UIColor?

    /// The icon's canvas, the size the tab bar's own symbols take.
    static let canvas: CGFloat = 26
    /// A filled circle reads larger than an outline glyph of the same box, so the plain picture is
    /// a touch smaller than the canvas.
    static let plainDiameter: CGFloat = 24
    /// Selected: a 1.5 point ring at the canvas edge, a 1.5 point gap, the picture inside.
    static let ringWidth: CGFloat = 1.5
    static let ringGap: CGFloat = 1.5

    /// Reads your identity (the core's saved one first, then fresh) and draws the picture it names.
    func refresh(model: AppModel) async {
        guard let user = model.signIn.user else {
            clear()
            return
        }
        if shownURL == nil, let cached = try? await model.core.invoke("own_identity_header_cached", args: ["userId": user.id],
                                                                        as: OwnIdentityData?.self) {
            show(Self.pictureURL(cached, user: user))
        } else if shownURL == nil {
            show(user.profileImageURL)
        }
        if let fresh = try? await model.core.invoke("own_identity_header", args: [
            "userId": user.id, "login": user.login, "displayName": user.displayName,
        ], as: OwnIdentityData.self) {
            show(Self.pictureURL(fresh, user: user))
        }
    }

    func clear() {
        plain = nil
        ringed = nil
        shownURL = nil
    }

    /// The picture your You card shows: 7TV's still when you have one, else Twitch's.
    static func pictureURL(_ identity: OwnIdentityData?, user: TwitchUser) -> String? {
        identity?.picture?.still ?? user.profileImageURL
    }

    private func show(_ url: String?) {
        let accent = UIColor(SN.accent)
        guard let url, !url.isEmpty, url != shownURL || accent != self.accent else { return }
        let scale = UIScreen.main.scale
        ImagePipeline.shared.load(url, pixelHeight: Int((Self.canvas * scale).rounded(.up)), animated: false) { [weak self] decoded in
            guard let self, let image = decoded?.first else { return }
            self.plain = Self.render(image, ring: nil, scale: scale)
            self.ringed = Self.render(image, ring: accent, scale: scale)
            self.shownURL = url
            self.accent = accent
        }
    }

    /// Draws the picture center-cropped into a circle on the canvas, with the ring when given.
    private static func render(_ image: CGImage, ring: UIColor?, scale: CGFloat) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = scale
        format.opaque = false
        let size = CGSize(width: canvas, height: canvas)
        let palette = Look.shared.palette
        let floor = palette.backgroundTertiary.uiColor.cgColor
        let wash = palette.surface.uiColor.cgColor
        let drawn = UIGraphicsImageRenderer(size: size, format: format).image { context in
            let cg = context.cgContext
            let diameter = ring == nil ? plainDiameter : canvas - 2 * (ringWidth + ringGap)
            let circle = CGRect(x: (canvas - diameter) / 2, y: (canvas - diameter) / 2, width: diameter, height: diameter)
            cg.saveGState()
            cg.addEllipse(in: circle)
            cg.clip()
            // A solid disc under the picture, as the You card draws it: a picture with transparent
            // parts (many 7TV pictures) otherwise floats as loose marks in the bar.
            cg.setFillColor(floor)
            cg.fill(circle)
            cg.setFillColor(wash)
            cg.fill(circle)
            let w = CGFloat(image.width), h = CGFloat(image.height)
            let fill = max(diameter / w, diameter / h)
            let box = CGRect(x: circle.midX - w * fill / 2, y: circle.midY - h * fill / 2, width: w * fill, height: h * fill)
            UIImage(cgImage: image).draw(in: box)
            cg.restoreGState()
            if let ring {
                cg.setStrokeColor(ring.cgColor)
                cg.setLineWidth(ringWidth)
                cg.strokeEllipse(in: CGRect(origin: .zero, size: size).insetBy(dx: ringWidth / 2, dy: ringWidth / 2))
            }
        }
        return drawn.withRenderingMode(.alwaysOriginal)
    }
}

/// The You tab's label: your picture when it is ready, the person glyph until then.
struct YouTabLabel: View {
    let avatar: YouTabAvatar
    let selected: Bool

    var body: some View {
        Label {
            Text(MobileTab.you.title)
        } icon: {
            if let image = selected ? avatar.ringed : avatar.plain {
                Image(uiImage: image).renderingMode(.original)
            } else {
                Image(systemName: Symbol.you)
            }
        }
    }
}
