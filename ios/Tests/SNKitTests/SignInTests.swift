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
    @Test func theWebLoginDecodes() throws {
        let json = #"{"url":"https://id.twitch.tv/oauth2/authorize?state=s","redirect":"http://localhost:3000/callback"}"#
        let login = try JSONDecoder().decode(WebLogin.self, from: Data(json.utf8))
        #expect(login == WebLogin(url: "https://id.twitch.tv/oauth2/authorize?state=s", redirect: "http://localhost:3000/callback"))
    }

    @Test func theUserDecodesWithItsOptionalFields() throws {
        let json = #"{"id":"1","login":"nook","display_name":"Nook","email":null,"profile_image_url":"p","broadcaster_type":""}"#
        let user = try JSONDecoder().decode(TwitchUser.self, from: Data(json.utf8))
        #expect(user == TwitchUser(id: "1", login: "nook", displayName: "Nook", profileImageURL: "p"))
    }

    private let authorize = URL(string: "https://id.twitch.tv/oauth2/authorize?client_id=x&state=s")!
    private let redirect = URL(string: "http://localhost:3000/callback")!

    @Test func theRedirectCarriesTheCodeAndState() {
        var nav = SignInNavigator(authorize: authorize, redirect: redirect)
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/login?client_id=x")!) == .stay)
        #expect(nav.arrived(at: URL(string: "http://localhost:3000/callback?code=abc&scope=chat%3Aread&state=s")!)
            == .redirect(code: "abc", state: "s", error: nil))
    }

    @Test func aDeclinedAuthorizeCarriesTheError() {
        var nav = SignInNavigator(authorize: authorize, redirect: redirect)
        let declined = URL(string: "http://localhost:3000/callback?error=access_denied&error_description=The+user+denied+you+access&state=s")!
        #expect(nav.arrived(at: declined) == .redirect(code: nil, state: "s", error: "The user denied you access"))
        let bare = URL(string: "http://localhost:3000/callback/?error=access_denied")!
        #expect(nav.arrived(at: bare) == .redirect(code: nil, state: nil, error: "access_denied"))
    }

    @Test func anotherPortOrPathIsNotTheRedirect() {
        var nav = SignInNavigator(authorize: authorize, redirect: redirect)
        #expect(nav.arrived(at: URL(string: "http://localhost:3001/callback?code=abc")!) == .stay)
        #expect(nav.arrived(at: URL(string: "http://localhost:3000/other?code=abc")!) == .stay)
    }

    @Test func landingHomeMidSignInReturnsToTheAuthorizePageOnce() {
        var nav = SignInNavigator(authorize: authorize, redirect: redirect)
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/")!) == .returnToAuthorize(authorize))
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/")!) == .stay)
        #expect(nav.arrived(at: URL(string: "https://www.twitch.tv/somechannel")!) == .stay)
    }

    @Test func onlyWebLinksLoadInTheSheet() {
        #expect(SignInNavigator.allows(URL(string: "https://www.twitch.tv/login")!))
        #expect(SignInNavigator.allows(URL(string: "about:blank")!))
        #expect(!SignInNavigator.allows(URL(string: "itms-apps://apps.apple.com/app/id460177396")!))
        #expect(!SignInNavigator.allows(URL(string: "twitch://open")!))
    }
}
