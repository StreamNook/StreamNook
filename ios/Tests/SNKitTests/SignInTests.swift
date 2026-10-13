import Foundation
import Testing
@testable import SNKit

@Suite struct WebCookieTests {
    @Test func aSiteOwnsItsCookiesAndItsSubdomainsOnly() {
        #expect(WebCookie(name: "a", value: "1", domain: ".twitch.tv").belongs(to: "twitch.tv"))
        #expect(WebCookie(name: "a", value: "1", domain: "www.twitch.tv").belongs(to: "twitch.tv"))
        #expect(WebCookie(name: "a", value: "1", domain: "Twitch.TV").belongs(to: "twitch.tv"))
        #expect(!WebCookie(name: "a", value: "1", domain: "nottwitch.tv").belongs(to: "twitch.tv"))
        #expect(!WebCookie(name: "a", value: "1", domain: "twitch.tv.evil.example").belongs(to: "twitch.tv"))
        #expect(!WebCookie(name: "a", value: "1", domain: "").belongs(to: "twitch.tv"))
    }

    @Test func aSnapshotCarriesOnlyTheSiteInTheCoresShape() throws {
        let cookies = [
            WebCookie(name: "auth-token", value: "t", domain: ".twitch.tv", expires: 1_900_000_000, secure: true),
            WebCookie(name: "session", value: "k", domain: "kick.com", httpOnly: true),
        ]
        let json = WebCookie.snapshotJSON(cookies, site: "twitch.tv")
        let list = try #require(try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[String: Any]])
        #expect(list.count == 1)
        let token = try #require(list.first)
        #expect(token["name"] as? String == "auth-token")
        #expect(token["domain"] as? String == ".twitch.tv")
        #expect(token["path"] as? String == "/")
        #expect(token["expires"] as? Double == 1_900_000_000)
        #expect(token["secure"] as? Bool == true)
        #expect(token["httpOnly"] as? Bool == false)
    }

    @Test func aSiteWithNoCookiesSendsAnEmptyList() {
        #expect(WebCookie.snapshotJSON([], site: "twitch.tv") == "[]")
    }
}

@Suite struct TwitchSignInTests {
    @Test func theDeviceLoginPairDecodes() throws {
        let json = #"["https://www.twitch.tv/activate?device-code=ABCD","ABCD"]"#
        let login = try JSONDecoder().decode(DeviceLogin.self, from: Data(json.utf8))
        #expect(login == DeviceLogin(verificationURI: "https://www.twitch.tv/activate?device-code=ABCD", userCode: "ABCD"))
    }

    @Test func theUserDecodesWithItsOptionalFields() throws {
        let json = #"{"id":"1","login":"nook","display_name":"Nook","email":null,"profile_image_url":"p","broadcaster_type":""}"#
        let user = try JSONDecoder().decode(TwitchUser.self, from: Data(json.utf8))
        #expect(user == TwitchUser(id: "1", login: "nook", displayName: "Nook", profileImageURL: "p"))
    }

    private let activate = URL(string: "https://www.twitch.tv/activate?device-code=ABCD")!

    @Test func landingHomeMidSignInReturnsToTheActivatePageOnce() {
        var nav = ActivateNavigator(activate: activate)
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/login?client_id=x")!) == .stay)
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/")!) == .returnToActivate(activate))
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/")!) == .stay)
    }

    @Test func approvalIsWhereTheAuthorizeRedirectSaid() {
        var nav = ActivateNavigator(activate: activate)
        let authorize = URL(string: "https://auth.twitch.tv/authorize?client_id=x&redirect_uri=https%3A%2F%2Fwww.twitch.tv%2Factivate%2Fdone%3Fa%3D1")!
        #expect(nav.arrived(at: authorize) == .stay)
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/settings/connections")!) == .stay)
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/activate/done/?a=1")!) == .approved)
    }

    @Test func approvalFallsBackToTheConnectionsPage() {
        var nav = ActivateNavigator(activate: activate)
        #expect(nav.arrived(at: activate) == .stay)
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/settings/connections")!) == .approved)
    }

    @Test func onlyWebLinksLoadInTheSheet() {
        #expect(ActivateNavigator.allows(URL(string: "https://www.twitch.tv/login")!))
        #expect(ActivateNavigator.allows(URL(string: "about:blank")!))
        #expect(!ActivateNavigator.allows(URL(string: "itms-apps://apps.apple.com/app/id460177396")!))
        #expect(!ActivateNavigator.allows(URL(string: "twitch://open")!))
    }
}
