import Foundation
import Testing
@testable import SNKit

@Suite struct OutsideTapTests {
    // A phone: the composer along the bottom, the emote picker above it, the chat above that.
    let composer = CGRect(x: 0, y: 760, width: 390, height: 84)
    let field = CGRect(x: 50, y: 770, width: 230, height: 40)
    let picker = CGRect(x: 10, y: 380, width: 370, height: 372)
    let chatRow = CGPoint(x: 195, y: 200)

    @Test func aTapOnTheChatPutsTheKeyboardAwayAndIsUsedUp() {
        let d = OutsideTapRule.decide(at: chatRow, field: field, keepers: [composer], popouts: [])
        #expect(d == OutsideTapRule.Decision(keyboard: true))
        #expect(d.usedUp, "the row under the finger is not pressed by the same tap")
    }

    @Test func aSearchResultTakesTheTapAsTheKeyboardGoes() {
        let search = CGRect(x: 16, y: 60, width: 358, height: 36)
        let results = CGRect(x: 0, y: 100, width: 390, height: 700)
        let d = OutsideTapRule.decide(at: CGPoint(x: 195, y: 300), field: search, keepers: [], popouts: [], through: [results])
        #expect(d.keyboard && d.passes && !d.usedUp, "the keyboard goes and the result opens")
        // Outside the results (a header) the tap only puts the keyboard away.
        #expect(OutsideTapRule.decide(at: CGPoint(x: 195, y: 20), field: search, keepers: [], popouts: [], through: [results]).usedUp)
        // A pop-out closing always uses the tap up, results or not.
        let open = [OutsideTapRule.Popout(id: "menu", regions: [CGRect(x: 200, y: 400, width: 100, height: 100)])]
        #expect(OutsideTapRule.decide(at: CGPoint(x: 50, y: 300), field: nil, keepers: [], popouts: open, through: [results]).usedUp)
    }

    @Test func theComposersOwnButtonsKeepTheKeyboard() {
        let send = CGPoint(x: 360, y: 790)
        #expect(!OutsideTapRule.decide(at: send, field: field, keepers: [composer], popouts: []).closesAnything)
        #expect(!OutsideTapRule.decide(at: CGPoint(x: 100, y: 790), field: field, keepers: [], popouts: []).closesAnything,
                "a tap in the field itself")
    }

    @Test func withNoKeyboardAndNothingOpenATapIsJustATap() {
        #expect(!OutsideTapRule.decide(at: chatRow, field: nil, keepers: [composer], popouts: []).closesAnything)
    }

    @Test func aTapOutsideAnOpenPopoutClosesIt() {
        let open = [OutsideTapRule.Popout(id: "emotes", regions: [picker, composer])]
        let d = OutsideTapRule.decide(at: CGPoint(x: 195, y: 100), field: nil, keepers: [], popouts: open)
        #expect(d == OutsideTapRule.Decision(popouts: ["emotes"]))
        // Inside it, or on its toggle (the composer), nothing closes: picking an emote, scrolling it.
        #expect(!OutsideTapRule.decide(at: CGPoint(x: 195, y: 500), field: nil, keepers: [], popouts: open).closesAnything)
        #expect(!OutsideTapRule.decide(at: CGPoint(x: 30, y: 790), field: nil, keepers: [], popouts: open).closesAnything)
    }

    @Test func oneTapPutsAwayTheKeyboardAndThePopoutTogether() {
        let search = CGRect(x: 20, y: 390, width: 350, height: 32)
        let open = [OutsideTapRule.Popout(id: "emotes", regions: [picker])]
        let d = OutsideTapRule.decide(at: chatRow, field: search, keepers: [composer], popouts: open)
        #expect(d == OutsideTapRule.Decision(keyboard: true, popouts: ["emotes"]))
        // Picking an emote while typing in the picker's search keeps both.
        #expect(!OutsideTapRule.decide(at: CGPoint(x: 195, y: 600), field: search, keepers: [composer], popouts: open).closesAnything)
    }

    @Test func anInnerPopoutClosesAloneWhenTheTapIsInsideTheOuterOne() {
        let sheet = CGRect(x: 0, y: 300, width: 390, height: 544)
        let menu = CGRect(x: 200, y: 400, width: 170, height: 120)
        let open = [OutsideTapRule.Popout(id: "sheet", regions: [sheet]), OutsideTapRule.Popout(id: "menu", regions: [menu])]
        let d = OutsideTapRule.decide(at: CGPoint(x: 50, y: 600), field: nil, keepers: [], popouts: open)
        #expect(d.popouts == ["menu"])
    }

    @Test func dragsAndHoldsAreNotTaps() {
        #expect(OutsideTapRule.isTap(distance: 4, duration: 0.12))
        #expect(!OutsideTapRule.isTap(distance: 24, duration: 0.12), "a drag scrolls")
        #expect(!OutsideTapRule.isTap(distance: 2, duration: 0.8), "a hold opens a hold menu")
    }
}
