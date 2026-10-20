import Foundation
import Testing
@testable import SNPush

@Suite struct LiveAvatarTests {
    private let picture = "https://static-cdn.jtvnw.net/jtv_user_pictures/alpha-profile_image-300x300.png"
    private let png = Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])

    private func push(_ avatar: Any?, type: String = "live") -> [AnyHashable: Any] {
        var sn: [String: Any] = ["type": type, "channel_id": "123", "login": "alpha"]
        sn["avatar"] = avatar
        return ["aps": ["alert": ["title": "Alpha is live", "body": "hello"], "mutable-content": 1], "sn": sn]
    }

    @Test func theServersPictureIsFetched() throws {
        let url = try #require(LiveAvatar.url(in: push(picture)))
        #expect(url.absoluteString == picture)
        #expect(LiveAvatar.url(in: push("HTTPS://STATIC-CDN.JTVNW.NET/a.png")) != nil, "scheme and host compare caseless")
        #expect(LiveAvatar.url(in: push("https://static-cdn.jtvnw.net:443/a.png")) != nil)
    }

    @Test func noPictureMeansTheBannerAsSent() {
        #expect(LiveAvatar.url(in: push(nil)) == nil)
        #expect(LiveAvatar.url(in: push("")) == nil, "Twitch gave the server no picture")
        #expect(LiveAvatar.url(in: push(42)) == nil)
        #expect(LiveAvatar.url(in: push(picture, type: "mention")) == nil, "only go-live pushes")
        #expect(LiveAvatar.url(in: ["aps": ["alert": "hi"]]) == nil)
    }

    @Test func onlyHttpsFromTwitchsPictureHost() {
        for raw in [
            "http://static-cdn.jtvnw.net/a.png",
            "https://evil.example/a.png",
            "https://static-cdn.jtvnw.net.evil.example/a.png",
            "https://evilstatic-cdn.jtvnw.net/a.png",
            "https://user:pw@static-cdn.jtvnw.net/a.png",
            "https://static-cdn.jtvnw.net:8443/a.png",
            "file:///etc/passwd",
            "data:image/png;base64,iVBORw0KGgo=",
            "//static-cdn.jtvnw.net/a.png",
            "not a url",
            "https://static-cdn.jtvnw.net/" + String(repeating: "a", count: 2100),
        ] {
            #expect(LiveAvatar.allowed(raw) == nil, "\(raw.prefix(60))")
        }
        #expect(LiveAvatar.allowed(URL(string: "https://evil.example/x.png")) == nil, "a redirect elsewhere is refused")
        #expect(LiveAvatar.allowed(URL(string: picture)) != nil)
        #expect(LiveAvatar.allowed(nil as URL?) == nil)
    }

    @Test func theBytesDecideTheKind() {
        #expect(LiveAvatar.kind(of: png) == .png)
        #expect(LiveAvatar.kind(of: Data([0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10])) == .jpg)
        #expect(LiveAvatar.kind(of: Data("GIF89a..".utf8)) == .gif)
        #expect(LiveAvatar.kind(of: Data("GIF87a".utf8)) == .gif)
        #expect(LiveAvatar.kind(of: Data("<html>".utf8)) == nil, "an error page is not a picture")
        #expect(LiveAvatar.kind(of: Data([0x89, 0x50])) == nil, "too short to be sure")
        #expect(LiveAvatar.kind(of: Data()) == nil)
    }

    @Test func aDownloadIsAttachedOnlyWhenWholeAndSmall() {
        let cap = LiveAvatar.maxBytes
        #expect(LiveAvatar.accept(status: 200, mimeType: "image/png", expectedLength: 40_000, received: 40_000, head: png) == .png)
        #expect(LiveAvatar.accept(status: 200, mimeType: "IMAGE/PNG", expectedLength: -1, received: 40_000, head: png) == .png,
                "no announced length is fine; the received size still counts")
        #expect(LiveAvatar.accept(status: 200, mimeType: "image/jpeg", expectedLength: Int64(cap), received: cap, head: png) == .png,
                "the bytes decide, at exactly the cap")
        #expect(LiveAvatar.accept(status: 404, mimeType: "image/png", expectedLength: 10, received: 10, head: png) == nil)
        #expect(LiveAvatar.accept(status: 301, mimeType: "image/png", expectedLength: 10, received: 10, head: png) == nil,
                "a refused redirect")
        #expect(LiveAvatar.accept(status: 200, mimeType: "text/html", expectedLength: 10, received: 10, head: png) == nil)
        #expect(LiveAvatar.accept(status: 200, mimeType: nil, expectedLength: 10, received: 10, head: png) == nil)
        #expect(LiveAvatar.accept(status: 200, mimeType: "image/png", expectedLength: Int64(cap) + 1, received: 10, head: png) == nil)
        #expect(LiveAvatar.accept(status: 200, mimeType: "image/png", expectedLength: -1, received: cap + 1, head: png) == nil)
        #expect(LiveAvatar.accept(status: 200, mimeType: "image/png", expectedLength: 0, received: 0, head: Data()) == nil)
        #expect(LiveAvatar.accept(status: 200, mimeType: "image/png", expectedLength: 6, received: 6, head: Data("<html>".utf8)) == nil)
    }

    @Test func theRoundPictureOnlyWhenReleaseSigningSaysSo() {
        #expect(LiveAvatar.style(nil) == .attachment)
        #expect(LiveAvatar.style([:]) == .attachment)
        #expect(LiveAvatar.style(["SNCommunicationStyle": false]) == .attachment, "the value the repo ships")
        #expect(LiveAvatar.style(["SNCommunicationStyle": "YES"]) == .attachment, "only a real boolean counts")
        #expect(LiveAvatar.style(["SNCommunicationStyle": true]) == .communication)
        #expect(LiveAvatar.communicationKey == "SNCommunicationStyle")
    }

    @Test func theSenderIsTheChannel() throws {
        let sender = try #require(LiveAvatar.sender(in: push(picture)))
        #expect(sender == LiveAvatar.Sender(channelID: "123", login: "alpha"))
        #expect(sender.senderID == "twitch:123")
        #expect(sender.conversationID == "live-123", "the push's thread-id, so grouping matches")
        let noID = LiveAvatar.Sender(channelID: "", login: "Alpha")
        #expect(noID.senderID == "twitch:alpha")
        #expect(noID.conversationID == "live-alpha")
        #expect(LiveAvatar.sender(in: push(picture, type: "mention")) == nil)
        #expect(LiveAvatar.sender(in: ["sn": ["type": "live", "login": ""]]) == nil)
        #expect(LiveAvatar.sender(in: push(nil)) != nil, "a go-live without a picture still has a sender")
    }

    @Test func theFetchIsBounded() {
        #expect(LiveAvatar.timeout == 5)
        #expect(LiveAvatar.maxBytes == 1_048_576)
        #expect(LiveAvatar.hosts == ["static-cdn.jtvnw.net"])
    }
}
