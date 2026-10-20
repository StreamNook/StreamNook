import Foundation
import Testing
@testable import SNKit

@Suite struct GifPickerTests {
    private func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
        try JSONDecoder().decode(type, from: Data(json.utf8))
    }

    @Test func gifsShowOnlyWhereTheChannelHasThemOn() throws {
        let open = try decode(GifPickerStatus.self, #"{"is_enabled":true,"is_allowlisted":true,"can_use":true,"content_rating":"pg"}"#)
        #expect(open.showsSection && open.tierLine == nil)
        let off = try decode(GifPickerStatus.self, #"{"is_enabled":false,"is_allowlisted":true,"can_use":false,"content_rating":null}"#)
        #expect(!off.showsSection && off.tierLine == nil, "off: nothing shows, nothing is announced")
        let tier = try decode(GifPickerStatus.self, #"{"is_enabled":true,"is_allowlisted":false,"can_use":false,"content_rating":null}"#)
        #expect(tier.showsSection && tier.tierLine == "Tier 2 and Tier 3 subscribers can post GIFs in this channel.")
    }

    @Test func resultsDecodeAsTheCoreSendsThem() throws {
        let items = try decode([GifItem].self, #"[{"id":"g1","title":"wave","preview_url":"https://p/1.gif","url":"https://f/1.gif?x=1","width":200,"height":120}]"#)
        #expect(items == [GifItem(id: "g1", title: "wave", previewURL: "https://p/1.gif", url: "https://f/1.gif?x=1")])
    }

    @Test func aRefusedSendIsSaidInAndroidsWords() throws {
        #expect(try decode(SendGifOutcome.self, #"{"sent":true,"error":null,"seconds_until_can_send":0}"#).notSentLine == "")
        #expect(SendGifOutcome(sent: false, error: "TEMPORARILY_UNAVAILABLE").notSentLine
            == "GIFs didn't go through. Try again in a moment.")
        #expect(SendGifOutcome(sent: false, secondsUntilCanSend: 12).notSentLine
            == "Slow down a moment, you can send another GIF in 12s.")
        #expect(SendGifOutcome(sent: false).notSentLine == "That GIF could not be sent.")
    }
}
