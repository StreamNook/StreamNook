import Foundation
import Testing
@testable import SNKit

@Suite struct SplitLayoutTests {
    @Test func readsTheCoreLayout() throws {
        let json = #"{"grid":24.0,"mark":11.5,"art_at":0.44,"gem_at":12.5,"x1":19.74,"y1":8.24,"x2":8.24,"y2":19.74,"shift_x":0.5,"shift_y":-0.12}"#
        let layout = try #require(SplitLayout.parse(json))
        #expect(layout.artAt == 0.44)
        #expect(layout.gemAt == 12.5)
        #expect(layout.shiftX == 0.5)
        #expect(layout.x1 == 19.74 && layout.y2 == 19.74)
    }

    @Test func aBadAnswerIsNotALayout() {
        #expect(SplitLayout.parse("null") == nil)
        #expect(SplitLayout.parse(#"{"grid":24}"#) == nil)
    }

    @Test func theFallbackIsADiagonalInsideItsSquare() {
        let f = SplitLayout.fallback
        #expect(abs((f.x1 + f.y1) - (f.x2 + f.y2)) < 0.02)
        for v in [f.x1, f.y1, f.x2, f.y2] { #expect((0...f.grid).contains(v)) }
    }
}
