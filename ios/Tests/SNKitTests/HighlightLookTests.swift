import Foundation
import Testing
@testable import SNKit

/// The Highlight Appearance and the first-time chatter look, read out of `chat_highlights`.
@Suite struct HighlightLookTests {
    private func style(_ json: String) -> ChatStyle {
        ChatStyle(settingsJSON: Data(json.utf8))
    }

    @Test func absentSettingsKeepTheDesktopDefaults() {
        let s = style("{}")
        #expect(s.highlightDisplay == "standard")
        #expect(s.highlightOpacity == 20)
        #expect(s.highlightAlpha == 0.2)
        #expect(s.highlightShowsTint)
        #expect(s.highlightShowsBar)
        #expect(s.firstTimeStyle == "wash")
        #expect(s.firstTimeFill == false)
        #expect(s.firstTimeAnimation == "none")
        #expect(s.firstTimeAnimateRepeat == false)
    }

    @Test func aGroupMissingItsFieldsKeepsTheDefaults() {
        let s = style(#"{"chat_highlights":{"appearance":{},"built_in":{"first_time_chatter":{"enabled":true}}}}"#)
        #expect(s.highlightDisplay == "standard")
        #expect(s.highlightOpacity == 20)
        #expect(s.firstTimeStyle == "wash")
        #expect(s.firstTimeAnimation == "none")
    }

    @Test func theAppearanceIsRead() {
        let minimal = style(#"{"chat_highlights":{"appearance":{"display_style":"minimal","opacity":45}}}"#)
        #expect(minimal.highlightDisplay == "minimal")
        #expect(minimal.highlightOpacity == 45)
        #expect(minimal.highlightAlpha == 0.45)
        #expect(minimal.highlightShowsTint == false)
        #expect(minimal.highlightShowsBar)

        let none = style(#"{"chat_highlights":{"appearance":{"display_style":"none"}}}"#)
        #expect(none.highlightShowsTint == false)
        #expect(none.highlightShowsBar == false)
    }

    @Test func theOpacityIsClampedToItsRange() {
        #expect(style(#"{"chat_highlights":{"appearance":{"opacity":250}}}"#).highlightOpacity == 100)
        #expect(style(#"{"chat_highlights":{"appearance":{"opacity":-5}}}"#).highlightOpacity == 0)
        #expect(style(#"{"chat_highlights":{"appearance":{"opacity":0}}}"#).highlightAlpha == 0)
    }

    @Test func theFirstTimeLookIsRead() {
        let json = ##"{"chat_highlights":{"built_in":{"first_time_chatter":{"enabled":true,"color":"#a855f7","style":"ring","fill":true,"animation":"chase","animate_repeat":true}}}}"##
        let s = style(json)
        #expect(s.firstTimeStyle == "ring")
        #expect(s.firstTimeFill)
        #expect(s.firstTimeAnimation == "chase")
        #expect(s.firstTimeAnimateRepeat)
    }

    @Test func otherBuiltInRulesDoNotSetTheFirstTimeLook() {
        let s = style(#"{"chat_highlights":{"built_in":{"raider":{"style":"ring","animation":"pulse"}}}}"#)
        #expect(s.firstTimeStyle == "wash")
        #expect(s.firstTimeAnimation == "none")
    }

    @Test func aChangedLookMakesADifferentStyle() {
        // Open rooms restyle only when the style read differs from the last one.
        let before = style("{}")
        let after = style(#"{"chat_highlights":{"built_in":{"first_time_chatter":{"animation":"sheen"}}}}"#)
        #expect(before != after)
    }
}
