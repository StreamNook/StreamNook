import Foundation
import Testing
@testable import SNKit

@Suite struct VectorPathTests {
    @Test func numbersRunTogetherAndSplitOnSignsAndSecondDots() {
        #expect(VectorPath.tokens("M1.5.5-2,3e-1z") == ["M", "1.5", ".5", "-2", "3e-1", "z"])
    }

    @Test func relativeLinesAndClosesTrackThePen() {
        let steps = VectorPath.parse("M10 10h5v5H10zm4.715 0")
        #expect(steps == [
            .move(.init(10, 10)), .line(.init(15, 10)), .line(.init(15, 15)), .line(.init(10, 15)), .close,
            .move(.init(14.715, 10)),
        ])
    }

    @Test func smoothCurvesReflectTheLastControlPoint() {
        let steps = VectorPath.parse("M0 0C1 0 2 1 2 2S3 4 4 4")
        #expect(steps.count == 3)
        guard case .cubic(let c1, _, let to) = steps[2] else {
            Issue.record("no smooth curve")
            return
        }
        #expect(c1 == .init(2, 3), "the reflection of (2, 1) through (2, 2)")
        #expect(to == .init(4, 4))
    }

    @Test func everyLogoTheAppDrawsReadsToTheEnd() {
        // The BetterTTV ring: relative curves, smooth forms and numbers written as "-.004".
        let bttv = VectorPath.parse("M150 1.74C68.409 1.74 1.74 68.41 1.74 150S68.41 298.26 150 298.26h148.26V150.17h-.004c0-.057.004-.113.004-.17C298.26 68.409 231.59 1.74 150 1.74z")
        #expect(bttv.first == .move(.init(150, 1.74)))
        #expect(bttv.last == .close)
        // M, C, S, h, V, h, c, C, z.
        #expect(bttv.count == 9)
    }
}
