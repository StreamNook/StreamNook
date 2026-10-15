import SNKit
import SwiftUI
import UIKit

/// A chat row of yours as others will see it, live as you pick: your badges (7TV, then
/// StreamNook last), your name in your paint or your Twitch colour, a line of chat, and your
/// Atmosphere's wash behind it (with its frost when it asks for one) while it shows in chat.
struct CosmeticsPreviewRow: View {
    let name: String
    let paint: ChatPaint?
    let colorHex: String?
    let badges: [String]
    let look: AtmosphereLook?

    private var nameColor: Color {
        Color(cgColor: FillPainter.color(colorHex ?? "#9147ff") ?? CGColor(red: 0.57, green: 0.28, blue: 1, alpha: 1))
    }

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(badges.enumerated()), id: \.offset) { _, url in
                CosmeticImage(url, animate: true).frame(width: 18, height: 18)
            }
            HStack(spacing: 0) {
                PaintedText(text: name, font: .sn(14, 700), paint: paint, fallback: nameColor, shadows: .all)
                Text(": that play was clean").font(.sn(14, 300)).foregroundStyle(SN.textPrimary).lineLimit(1)
            }
            .padding(.leading, badges.isEmpty ? 0 : 2)
        }
        .padding(.horizontal, look?.chatFrost == true ? 6 : 0)
        .padding(.vertical, look?.chatFrost == true ? 2 : 0)
        .background {
            if look?.chatFrost == true {
                RoundedRectangle(cornerRadius: 6, style: .continuous).fill(Color(red: 5 / 255, green: 6 / 255, blue: 13 / 255).opacity(0.66))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .background {
            if let look {
                WashView(look: look)
            }
        }
        .clipShape(RoundedRectangle(cornerRadius: SNRadius.panel, style: .continuous))
        .snPanel()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Preview: \(name): that play was clean")
    }
}

/// The chat row's Atmosphere wash in SwiftUI, through the same layer the chat list draws with.
struct WashView: UIViewRepresentable {
    let look: AtmosphereLook

    func makeUIView(context: Context) -> WashHost { WashHost() }

    func updateUIView(_ view: WashHost, context: Context) {
        view.look = look
        view.setNeedsLayout()
    }

    final class WashHost: UIView {
        var look: AtmosphereLook?
        private let wash = AtmosphereWashLayer()

        override init(frame: CGRect) {
            super.init(frame: frame)
            isUserInteractionEnabled = false
            layer.addSublayer(wash)
        }

        required init?(coder: NSCoder) { fatalError() }

        override func layoutSubviews() {
            super.layoutSubviews()
            guard let look, bounds.width > 0, bounds.height > 0 else { return }
            wash.show(look, cologne: nil, size: bounds.size, scale: traitCollection.displayScale,
                      animate: !UIAccessibility.isReduceMotionEnabled && Look.shared.motion == .full)
        }
    }
}
