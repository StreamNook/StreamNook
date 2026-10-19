import Foundation
import Testing
@testable import SNKit

@Suite struct SleepTimerTests {
    @Test func aRunningTimerReadsAsTheCoreWordedIt() throws {
        let state = try #require(SleepTimerState.parse("""
        {"phase":"running","endsAtMs":1792276889810,"remainingSecs":2520,"rowLabel":"Stops in 42 min","fadeSecs":10,
         "presets":[{"minutes":15,"label":"15 min"},{"minutes":30,"label":"30 min"},{"minutes":60,"label":"1 h"},{"minutes":120,"label":"2 h"}]}
        """))
        #expect(state.running)
        #expect(state.rowLabel == "Stops in 42 min")
        #expect(state.presets.map(\.label) == ["15 min", "30 min", "1 h", "2 h"])
    }

    @Test func offAndFiredAreNotRunning() throws {
        let off = try #require(SleepTimerState.parse(#"{"phase":"off","fadeSecs":10,"presets":[]}"#))
        #expect(!off.running)
        let fired = try #require(SleepTimerState.parse(#"{"phase":"fired","fadeSecs":10,"presets":[]}"#))
        #expect(!fired.running)
        let fading = try #require(SleepTimerState.parse(#"{"phase":"fading","remainingSecs":9,"fadeSecs":10,"presets":[]}"#))
        #expect(fading.running)
    }

    @Test func customPicksComeToMinutes() {
        #expect(SleepTimerState.customMinutes(hours: 1, minutes: 30) == 90)
        #expect(SleepTimerState.customMinutes(hours: 0, minutes: 0) == nil)
        #expect(SleepTimerState.customMinutes(hours: 30, minutes: 0) == 1440)
    }
}
