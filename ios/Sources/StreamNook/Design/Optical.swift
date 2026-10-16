import SwiftUI

/// Optical corrections: where a shape has to sit to look right, which is not where its box puts
/// it. The rules and the reasons are in ios/docs/design-eye.md.
enum Optical {
    /// How far right a play triangle moves inside a round or square button: its mass sits at the
    /// flat left side (the centroid is a third of the way in), so box-centered it looks pushed
    /// left. Seven percent of the symbol's point size moves it most of the way.
    static func playNudge(_ pointSize: CGFloat) -> CGFloat { (pointSize * 0.07).rounded(toNearestOrAwayFromZero: 0.5) }
}

private extension CGFloat {
    func rounded(toNearestOrAwayFromZero step: CGFloat) -> CGFloat { (self / step).rounded() * step }
}
