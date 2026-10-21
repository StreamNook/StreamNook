import Foundation
import Testing
@testable import SNKit

@Suite struct BatteryTests {
    @Test func theClockRunsAt30OnlyWhenNoFrameIsShorterThanATick() {
        #expect(ClockRate.hz(shortest: 0.1) == 30)
        #expect(ClockRate.hz(shortest: 0.04) == 30)
        #expect(ClockRate.hz(shortest: 1.0 / 30) == 30)
        #expect(ClockRate.hz(shortest: 0.03) == 60)
        #expect(ClockRate.hz(shortest: 0.02) == 60)
    }

    @Test func aTimelineKnowsItsShortestFrameAfterTheBrowserClamp() {
        #expect(FrameTimeline(durations: [0.1, 0.04, 0.07]).shortest == 0.04)
        // 10 ms frames draw at 100 ms, so the 50 ms frame is the shortest.
        #expect(FrameTimeline(durations: [0.01, 0.05]).shortest == 0.05)
    }

    @Test func theChatLinkPausesOnlyAfterItsLinger() {
        let last = ChatLinkPolicy.linger - 1
        #expect(ChatLinkPolicy.rate(idleTicks: 0, gliding: false, touching: false) == .follow)
        #expect(ChatLinkPolicy.rate(idleTicks: last, gliding: false, touching: false) == .follow)
        #expect(ChatLinkPolicy.rate(idleTicks: ChatLinkPolicy.linger, gliding: false, touching: false) == .paused)
        #expect(ChatLinkPolicy.rate(idleTicks: 100, gliding: false, touching: true) == .touch)
        #expect(ChatLinkPolicy.rate(idleTicks: 100, gliding: true, touching: false) == .touch)
    }

    @Test func onlyAFingerGetsTheFullRateAndLowPowerCapsIt() {
        #expect(ChatLinkPolicy.range(.follow, lowPower: false).maximum == 60)
        #expect(ChatLinkPolicy.range(.touch, lowPower: false).preferred == 120)
        #expect(ChatLinkPolicy.range(.touch, lowPower: true).maximum == 60)
    }

    @Test func aLongPausedLiveStreamJumpsToTheEdge() {
        #expect(!PausePark.jumpsToLive(pausedFor: .seconds(29), live: true))
        #expect(PausePark.jumpsToLive(pausedFor: .seconds(30), live: true))
        #expect(!PausePark.jumpsToLive(pausedFor: .seconds(120), live: false))
        #expect(!PausePark.jumpsToLive(pausedFor: nil, live: true))
    }

    @Test func theWatchLoopSlowsWhenNobodyLooks() {
        #expect(MeasureCadence.tick(loading: true) == .milliseconds(250))
        #expect(MeasureCadence.tick(loading: false) == .seconds(1))
        #expect(MeasureCadence.sampleEvery(paused: true, background: false, pip: false) == nil)
        #expect(MeasureCadence.sampleEvery(paused: false, background: false, pip: false) == .seconds(1))
        #expect(MeasureCadence.sampleEvery(paused: false, background: true, pip: false) == .seconds(5))
        #expect(MeasureCadence.sampleEvery(paused: false, background: true, pip: true) == .seconds(1))
    }

    @Test func latencyLogsChangesAndAHeartbeat() {
        var gate = LatencyLogGate()
        #expect(gate.shouldLog(behind: 2000, state: 1, rate: 1, at: 0))
        #expect(!gate.shouldLog(behind: 2100, state: 1, rate: 1, at: 1))
        #expect(gate.shouldLog(behind: 2250, state: 1, rate: 1, at: 2))
        #expect(gate.shouldLog(behind: 2250, state: 2, rate: 1, at: 3))
        #expect(gate.shouldLog(behind: 2250, state: 2, rate: 1.05, at: 4))
        #expect(!gate.shouldLog(behind: 2250, state: 2, rate: 1.05, at: 33))
        #expect(gate.shouldLog(behind: 2250, state: 2, rate: 1.05, at: 34.1))
    }
}
