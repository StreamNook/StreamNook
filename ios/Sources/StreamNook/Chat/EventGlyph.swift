import CoreGraphics
import SwiftUI

/// The glyphs the desktop's event cards lead with, drawn from the same SVG paths (a 20-unit box):
/// Twitch's Prime crown for a Prime sub, the group for a raid, Twitch's streak flame for a watch
/// streak.
enum EventGlyph {
    case prime, raid, streak

    private var data: String {
        switch self {
        case .prime:
            "M18 5v8a2 2 0 0 1-2 2H4a2.002 2.002 0 0 1-2-2V5l4 3 4-4 4 4 4-3z"
        case .raid:
            "M7 6a3 3 0 1 1 6 0 3 3 0 0 1-6 0ZM3.5 8.5a2 2 0 1 1 4 0 2 2 0 0 1-4 0ZM12.5 8.5a2 2 0 1 1 4 0 2 2 0 0 1-4 0ZM5 15a5 5 0 0 1 10 0v2H5v-2ZM1 16a3.5 3.5 0 0 1 3.6-3.5A6.5 6.5 0 0 0 3.5 15v2H1v-1ZM19 16a3.5 3.5 0 0 0-3.6-3.5 6.5 6.5 0 0 1 1.1 2.5v2H19v-1Z"
        case .streak:
            "M11 4.5 9 2 4.8 6.9A7.48 7.48 0 0 0 3 11.77C3 15.2 5.8 18 9.23 18h1.65A6.12 6.12 0 0 0 17 11.88c0-1.86-.65-3.66-1.84-5.1L12 3l-1 1.5ZM6.32 8.2 9 5l2 2.5L12 6l1.62 2.07A5.96 5.96 0 0 1 15 11.88c0 2.08-1.55 3.8-3.56 4.08.36-.47.56-1.05.56-1.66 0-.52-.18-1.02-.5-1.43L10 11l-1.5 1.87c-.32.4-.5.91-.5 1.43 0 .6.2 1.18.54 1.64A4.23 4.23 0 0 1 5 11.77c0-1.31.47-2.58 1.32-3.57Z"
        }
    }

    /// Filled even-odd, as the desktop's SVGs say (the crown and the flame have holes).
    var evenOdd: Bool { self != .raid }

    /// The glyph scaled to fill `rect`, centered in it.
    func path(in rect: CGRect) -> CGPath {
        let base = Path(vector: data).cgPath
        let s = min(rect.width, rect.height) / 20
        var t = CGAffineTransform(translationX: rect.midX - 10 * s, y: rect.midY - 10 * s).scaledBy(x: s, y: s)
        return base.copy(using: &t) ?? base
    }
}
