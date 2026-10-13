import Foundation

/// What `twitch_login` answers: the activate page that approves the core's device code, and the
/// code itself. The core polls for that code and emits `twitch-login-complete` once it is
/// approved, as on Android.
public struct DeviceLogin: Decodable, Sendable, Equatable {
    public var verificationURI: String
    public var userCode: String

    public init(verificationURI: String, userCode: String) {
        self.verificationURI = verificationURI
        self.userCode = userCode
    }

    /// The command answers a `[verification_uri, user_code]` pair.
    public init(from decoder: Decoder) throws {
        var pair = try decoder.unkeyedContainer()
        verificationURI = try pair.decode(String.self)
        userCode = try pair.decode(String.self)
    }
}

/// The signed-in account as `get_user_info` returns it (the TypeScript `UserInfo`).
public struct TwitchUser: Decodable, Sendable, Equatable {
    public var id: String
    public var login: String
    public var displayName: String
    public var profileImageURL: String?

    enum CodingKeys: String, CodingKey {
        case id
        case login
        case displayName = "display_name"
        case profileImageURL = "profile_image_url"
    }

    public init(id: String, login: String, displayName: String, profileImageURL: String? = nil) {
        self.id = id
        self.login = login
        self.displayName = displayName
        self.profileImageURL = profileImageURL
    }
}

/// Follows the sign-in sheet's page loads, the way the Android login overlay does:
///
/// - Twitch's mobile site can finish a fresh sign-in on its home page instead of returning to
///   the activate page, with the device code nowhere in sight. The first arrival home goes back
///   to the activate page, where the code now waits to be approved.
/// - Approving leaves through `auth.twitch.tv/authorize`, whose `redirect_uri` says where it
///   lands. Reaching that page means the code is approved; the sheet can close while the core's
///   poll collects the token.
public struct ActivateNavigator: Sendable {
    public enum Step: Equatable, Sendable {
        case stay
        /// Load the activate page again.
        case returnToActivate(URL)
        /// Approval landed; close the sheet and wait for the core.
        case approved
    }

    /// Where approving lands when the authorize step was not seen.
    static let fallbackLanding = "www.twitch.tv/settings/connections"

    public let activate: URL
    private var returnedHome = false
    private var landing: String?

    public init(activate: URL) {
        self.activate = activate
    }

    /// What to do on arriving at `url`.
    public mutating func arrived(at url: URL) -> Step {
        guard let here = Self.hostPath(url) else { return .stay }
        if !returnedHome, activate.path.contains("/activate"), here == "www.twitch.tv" || here == "m.twitch.tv" {
            returnedHome = true
            return .returnToActivate(activate)
        }
        if url.host?.lowercased() == "auth.twitch.tv",
           let redirect = URLComponents(url: url, resolvingAgainstBaseURL: false)?
               .queryItems?.first(where: { $0.name == "redirect_uri" })?.value,
           let target = URL(string: redirect).flatMap(Self.hostPath) {
            landing = target
            return .stay
        }
        return here == (landing ?? Self.fallbackLanding) ? .approved : .stay
    }

    /// Lowercased host and path without a trailing slash, so queries and slashes do not matter.
    static func hostPath(_ url: URL) -> String? {
        guard let host = url.host?.lowercased() else { return nil }
        var path = url.path
        while path.hasSuffix("/") { path.removeLast() }
        return host + path
    }

    /// Whether the sheet loads `url` at all. App-store and custom-scheme hops (a site's "Open in
    /// app") have nothing to show in here.
    public static func allows(_ url: URL) -> Bool {
        let scheme = url.scheme?.lowercased()
        return scheme == "https" || scheme == "http" || scheme == "about"
    }
}
