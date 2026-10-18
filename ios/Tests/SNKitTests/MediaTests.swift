import Foundation
import Testing
@testable import SNKit

@Suite struct MediaTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(type, from: Data(json.utf8))
    }

    @Test func aVideoRowReadsAsTheCoreWordedIt() throws {
        let row = try decode(MediaRow.self, """
        {"kind":"video","id":"2886637167","title":"im sorry","thumbnailUrl":"https://x/t.jpg",
         "url":"https://www.twitch.tv/videos/2886637167","lengthLabel":"4h 12m","ageLabel":"3 weeks ago",
         "viewsLabel":"812K views","byline":"Just Chatting","progress":0.42,"watched":false,
         "recording":false,"vodId":null,"vodOffset":null}
        """)
        #expect(!row.isClip)
        #expect(row.metaLine == "3 weeks ago · 4h 12m")
        #expect(row.countLine == "812K views · Just Chatting")
        #expect(row.progress == 0.42)
    }

    @Test func aClipRowNamesWhoClippedIt() throws {
        let row = try decode(MediaRow.self, """
        {"kind":"clip","id":"Slug","title":"t","thumbnailUrl":"","url":"https://clips.twitch.tv/Slug",
         "lengthLabel":"0:28","ageLabel":"3 days ago","viewsLabel":"214K views","byline":"Clipped by m4rkus",
         "progress":null,"watched":false,"recording":false,"vodId":"123","vodOffset":2781}
        """)
        #expect(row.isClip)
        #expect(row.metaLine == "Clipped by m4rkus · 3 days ago")
        #expect(row.countLine == "0:28 · 214K views")
        #expect(row.vodOffset == 2781)
    }

    @Test func shelvesAskTheCoreForTheirList() {
        let videos = MediaShelf.videos(.highlight, .popular).args(channelID: "1", cursor: nil)
        #expect(videos["kind"] as? String == "videos")
        #expect(videos["filter"] as? String == "highlight")
        #expect(videos["sort"] as? String == "popular")
        let clips = MediaShelf.clips(.week).args(channelID: "1", cursor: "c")
        #expect(clips["filter"] as? String == "7d")
        #expect(clips["cursor"] as? String == "c")
    }

    @Test func yourClipsCarryTheirLink() throws {
        let clips = try decode([MyClip].self, """
        [{"slug":"Abc","title":"t","channelId":"1","channelLogin":"jynxzi","channelName":"Jynxzi",
          "madeAt":1792226788948,"durationSeconds":30,"source":"live",
          "url":"https://www.twitch.tv/jynxzi/clip/Abc","ageLabel":"just now","lengthLabel":"0:30"}]
        """)
        #expect(clips.first?.id == "Abc")
        #expect(clips.first?.thumbnailUrl == nil)
        #expect(clips.first?.url == "https://www.twitch.tv/jynxzi/clip/Abc")
    }

    @Test func aVodStartCarriesItsStampAndMarks() throws {
        let start = try decode(StreamStart.self, """
        {"url":"http://127.0.0.1:1/v.m3u8","quality":"1080p60","entitled":false,"available":[],
         "vod":{"video_id":"9","status":"recorded","length_seconds":15120,"start_position_secs":3734.5,
                "rewound_from_live":false,"play_id":42,
                "muted_segments":[{"start_secs":7800,"end_secs":8100}],
                "chapters":[{"start_secs":0,"end_secs":5000,"title":"Just Chatting"}]}}
        """)
        #expect(start.vod?.playID == 42)
        #expect(start.vod?.startPositionSecs == 3734.5)
        #expect(start.vod?.mutedSegments.first?.endSecs == 8100)
        #expect(start.vod?.chapters.first?.title == "Just Chatting")
        let live = try decode(StreamStart.self, #"{"url":"u","quality":"best","entitled":true,"available":[]}"#)
        #expect(live.vod == nil)
    }

    @Test func storyboardFramesFallOnTheirCell() throws {
        let board = try decode(Storyboard.self, """
        {"base_url":"https://x/sb/","variants":[
          {"quality":"low","width":110,"height":62,"rows":10,"cols":10,"count":300,"interval_secs":10,"images":["low-0.jpg","low-1.jpg","low-2.jpg"]},
          {"quality":"high","width":220,"height":124,"rows":5,"cols":5,"count":300,"interval_secs":10,"images":["h-0.jpg"]}]}
        """)
        // Under 160 px the low variant answers: 1234 s is its frame 123, sheet 1, cell 23.
        let frame = board.frame(at: 1234, maxWidth: 160)
        #expect(frame?.url.absoluteString == "https://x/sb/low-1.jpg")
        #expect(frame?.x == 330)
        #expect(frame?.y == 124)
        #expect(frame?.sheetWidth == 1100)
        #expect(board.frame(at: 0, maxWidth: 320)?.url.absoluteString == "https://x/sb/h-0.jpg")
    }

    @Test func speeds() {
        #expect(PlayerText.rate(1) == "1x")
        #expect(PlayerText.rate(1.25) == "1.25x")
        #expect(PlayerText.rate(1.5) == "1.5x")
        #expect(PlayerText.rate(2) == "2x")
    }

    @Test func playheadClocks() {
        #expect(PlayerText.clock(3734.9) == "1:02:14")
        #expect(PlayerText.clock(245) == "4:05")
        #expect(PlayerText.clock(9) == "0:09")
        #expect(PlayerText.clock(9, like: 15_120) == "0:00:09")
        #expect(PlayerText.clock(.nan) == "0:00")
    }

    @Test func aReplayStepWithNoRowsDecodes() throws {
        let step = try decode(ReplayStep.self, #"{"reset":true,"rows":[],"loading":true,"error":null}"#)
        #expect(step.reset && step.loading && step.rows.isEmpty)
    }
}
