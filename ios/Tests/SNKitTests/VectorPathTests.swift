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

    @Test func arcFlagsMayRunIntoTheNumbersAfterThem() {
        // Flags 0 and 1 written as "01", then the end point: a half circle of radius 1 from
        // (0, 0) to (2, 0), sweeping clockwise on screen, so through (1, -1).
        let steps = VectorPath.parse("M0 0a1 1 0 012 0")
        #expect(steps.count == 3, "a move and two quarter turns")
        guard case .cubic(_, _, let middle) = steps[1], case .cubic(_, _, let end) = steps[2] else {
            Issue.record("no arc curves")
            return
        }
        #expect(abs(middle.x - 1) < 1e-9 && abs(middle.y + 1) < 1e-9)
        #expect(end == .init(2, 0))
    }

    @Test func anArcWithoutRadiiIsALine() {
        #expect(VectorPath.parse("M0 0A0 0 0 0 0 4 4") == [.move(.init(0, 0)), .line(.init(4, 4))])
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
