import Foundation
import Testing
@testable import SNKit

@Suite struct LinkAdaptTests {
    @Test func thePushDecodesWithAndWithoutAReloadUrl() {
        let spliced = PlaybackAdapt.parse(
            #"{"quality":"720p60","auto":true,"extraCushion":1.0,"url":null,"reason":"repeated stalls"}"#
        )
        #expect(spliced == PlaybackAdapt(quality: "720p60", auto: true, extraCushion: 1, reason: "repeated stalls"))
        let restarted = PlaybackAdapt.parse(
            #"{"quality":"480p30","auto":true,"extraCushion":2.5,"url":"http://localhost:1/stream.m3u8?t=1","reason":"a long stall"}"#
        )
        #expect(restarted?.url == "http://localhost:1/stream.m3u8?t=1")
        #expect(PlaybackAdapt.parse("{}") == nil)
    }

    @Test func aStallIsReportedEachSecondAndOnceWhenItEnds() {
        var r = LinkReporter()
        #expect(r.waiting(false, at: 0) == nil)
        #expect(r.waiting(true, at: 10) == nil)
        #expect(r.waiting(true, at: 10.5) == nil)
        #expect(r.waiting(true, at: 11) == .stall(ms: 1000))
        #expect(r.waiting(true, at: 11.75) == nil)
        #expect(r.waiting(true, at: 12.25) == .stall(ms: 2250))
        #expect(r.waiting(false, at: 12.5) == .stallEnd(ms: 2500))
        #expect(r.waiting(false, at: 13) == nil)
    }

    @Test func aResetForgetsAStallInProgress() {
        var r = LinkReporter()
        _ = r.waiting(true, at: 1)
        r.reset()
        #expect(r.waiting(false, at: 5) == nil)
    }

    @Test func transfersReportTheDeltaOnceItIsBigEnough() {
        var r = LinkReporter()
        #expect(r.accessLog(event: 0, bytes: 10_000, transferDuration: 0.1) == nil, "too small to judge")
        #expect(r.accessLog(event: 0, bytes: 2_000_000, transferDuration: 4.0) == .transfer(bytes: 2_000_000, ms: 4000))
        #expect(r.accessLog(event: 0, bytes: 2_000_000, transferDuration: 4.0) == nil, "nothing new")
        #expect(r.accessLog(event: 0, bytes: 3_000_000, transferDuration: 6.0) == .transfer(bytes: 1_000_000, ms: 2000))
        // A new access log event (a variant switch, a reload) starts its own totals.
        #expect(r.accessLog(event: 1, bytes: 500_000, transferDuration: 1.0) == .transfer(bytes: 500_000, ms: 1000))
    }

    @Test func reportArgumentsNameTheCoreEvents() {
        #expect(LinkReport.transfer(bytes: 5, ms: 2).args["event"].flatMap { $0 } as? String == "frag")
        #expect(LinkReport.stall(ms: 2).args["event"].flatMap { $0 } as? String == "stall")
        #expect(LinkReport.stallEnd(ms: 2).args["event"].flatMap { $0 } as? String == "stall_end")
    }
}
