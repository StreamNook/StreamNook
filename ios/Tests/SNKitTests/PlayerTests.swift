import Foundation
import Testing
@testable import SNKit

@Suite struct PlayerTextTests {
    @Test func uptimeReadsHoursAndMinutes() {
        let start = "2026-10-14T10:00:00Z"
        let now = ISO8601DateFormatter().date(from: "2026-10-14T12:14:59Z")!
        #expect(PlayerText.uptime(startedAt: start, now: now) == "2h 14m")
        let soon = ISO8601DateFormatter().date(from: "2026-10-14T10:09:00Z")!
        #expect(PlayerText.uptime(startedAt: start, now: soon) == "9m")
    }

    @Test func uptimeTakesFractionalSecondsAndNeverGoesNegative() {
        let now = ISO8601DateFormatter().date(from: "2026-10-14T10:00:00Z")!
        #expect(PlayerText.uptime(startedAt: "2026-10-14T09:30:00.123Z", now: now) == "29m")
        #expect(PlayerText.uptime(startedAt: "2026-10-14T10:05:00Z", now: now) == "0m")
    }

    @Test func anUnknownOrUnreadableStartShowsNothing() {
        #expect(PlayerText.uptime(startedAt: nil) == "")
        #expect(PlayerText.uptime(startedAt: "") == "")
        #expect(PlayerText.uptime(startedAt: "yesterday") == "")
    }

    @Test func qualityLabelsMatchTheSheet() {
        #expect(PlayerText.qualityLabel("best") == "Auto")
        #expect(PlayerText.qualityLabel("1080p60") == "1080p60")
        #expect(PlayerText.qualityLabel("audio_only") == "Audio only")
        #expect(PlayerText.qualityLabel("worst") == "Lowest")
    }

    @Test func autoLeadsTheSheet() {
        #expect(PlayerText.qualityRows(["1080p60", "720p60", "160p", "best", "audio_only", "worst"])
            == ["best", "1080p60", "720p60", "160p", "audio_only", "worst"])
        #expect(PlayerText.qualityRows(["best", "720p"]) == ["best", "720p"])
    }

    @Test func theTickIsThePickThenTheSavedOneThenAuto() {
        #expect(PlayerText.tickedQuality(picked: "720p60", saved: "480p") == "720p60")
        #expect(PlayerText.tickedQuality(picked: nil, saved: "480p") == "480p")
        #expect(PlayerText.tickedQuality(picked: nil, saved: "") == "best")
        #expect(PlayerText.tickedQuality(picked: nil, saved: nil) == "best")
    }
}

@Suite struct ScreenCapTests {
    @Test func snapsDownToARung() {
        #expect(ScreenCap.height(shortEdgePixels: 1290) == 1080)
        #expect(ScreenCap.height(shortEdgePixels: 1179) == 1080)
        #expect(ScreenCap.height(shortEdgePixels: 1080) == 1080)
        #expect(ScreenCap.height(shortEdgePixels: 1079) == 720)
        #expect(ScreenCap.height(shortEdgePixels: 2400) == 2160)
    }

    @Test func belowTheLadderStaysAtTheBottomAndUnknownIsUncapped() {
        #expect(ScreenCap.height(shortEdgePixels: 200) == 360)
        #expect(ScreenCap.height(shortEdgePixels: 0) == 0)
    }
}

@Suite struct PlayerDecodingTests {
    @Test func startAnswersCarryTheQualityList() throws {
        let json = #"{"url":"http://localhost:1/stream.m3u8","quality":"1080p60","available":["best","1080p60","720p60","audio_only","worst"],"mode":"auth-only"}"#
        let start = try JSONDecoder().decode(StreamStart.self, from: Data(json.utf8))
        #expect(start.available.first == "best" && start.available.count == 5)
        let bare = try JSONDecoder().decode(StreamStart.self, from: Data(#"{"url":"u","quality":"best"}"#.utf8))
        #expect(bare.available.isEmpty)
    }

    @Test func onlyTheViewersSectionIsRead() {
        let viewers = #"{"section":"viewers","login":"jynxzi","viewer_count":41234,"started_at":"2026-10-14T10:00:00Z","title":"t","game_name":"Rainbow Six Siege","at":1}"#
        let parsed = ChannelViewers.parse(viewers)
        #expect(parsed?.viewerCount == 41234 && parsed?.gameName == "Rainbow Six Siege")
        #expect(ChannelViewers.parse(#"{"section":"points","login":"jynxzi","points":null,"at":1}"#) == nil)
    }

    @Test func prefsReadTheQualityAndBackgroundMode() throws {
        let json = #"{"quality":"720p60","video_player":{"background_mode":"audio","muted":false}}"#
        let prefs = try JSONDecoder().decode(PlayerPrefs.self, from: Data(json.utf8))
        #expect(prefs.quality == "720p60" && !prefs.floatsVideo)
        let bare = try JSONDecoder().decode(PlayerPrefs.self, from: Data(#"{"quality":"best"}"#.utf8))
        #expect(bare.floatsVideo)
    }
}
