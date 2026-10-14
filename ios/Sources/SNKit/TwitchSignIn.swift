import Foundation

/// What `begin_twitch_web_login` answers: Twitch's authorize page, and the redirect it ends on
/// with `code` and `state` (or `error`), which the sheet hands to `finish_twitch_web_login`.
public struct WebLogin: Decodable, Sendable, Equatable {
    public var url: String
    public var redirect: String

    public init(url: String, redirect: String) {
        self.url = url
        self.redirect = redirect
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

/// Follows the sign-in sheet's page loads:
///
/// - The authorize page's redirect never loads. Reaching it carries the result: `code` and
///   `state` on approval, `error` when the person declined.
/// - Twitch's mobile site can finish a fresh sign-in on its home page instead of returning to
///   the authorize page. The first arrival home goes back to the authorize page, where Authorize
///   now waits.
public struct SignInNavigator: Sendable {
    public enum Step: Equatable, Sendable {
        case stay
        /// Load the authorize page again.
        case returnToAuthorize(URL)
        /// The redirect: stop there and hand its result to the core.
        case redirect(code: String?, state: String?, error: String?)
    }

    public let authorize: URL
    public let redirect: URL
    private var returnedHome = false

    public init(authorize: URL, redirect: URL) {
        self.authorize = authorize
        self.redirect = redirect
    }

    /// What to do on arriving at `url`.
    public mutating func arrived(at url: URL) -> Step {
        if Self.sameEndpoint(url, redirect) {
            let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems ?? []
            func value(_ name: String) -> String? {
                items.first(where: { $0.name == name })?.value.flatMap { $0.isEmpty ? nil : $0 }
            }
            // Twitch writes spaces in the description as "+", which URLComponents keeps.
            let error = value("error").map { code in
                (value("error_description") ?? code).replacingOccurrences(of: "+", with: " ")
            }
            return .redirect(code: value("code"), state: value("state"), error: error)
        }
        guard let here = Self.hostPath(url) else { return .stay }
        if !returnedHome, here == "www.twitch.tv" || here == "m.twitch.tv" {
            returnedHome = true
            return .returnToAuthorize(authorize)
        }
        return .stay
    }

    /// Same scheme, host, port and path, whatever the query.
    static func sameEndpoint(_ a: URL, _ b: URL) -> Bool {
        a.scheme?.lowercased() == b.scheme?.lowercased() && a.port == b.port && hostPath(a) == hostPath(b)
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

/// What `get_boot_state` answers: whether the first-run setup is done, whether a Twitch sign-in
/// is stored, and, when StreamNook ended the last one, why.
public struct BootState: Decodable, Sendable, Equatable {
    /// The sentence for the signed-out landing, and the short form for a row that already says
    /// Twitch.
    public struct SignInText: Decodable, Sendable, Equatable {
        public var sentence: String
        public var short: String

        public init(sentence: String, short: String) {
            self.sentence = sentence
            self.short = short
        }
    }

    public var needsSetup: Bool
    public var signedIn: Bool
    public var signInReason: String?
    public var signInText: SignInText?

    enum CodingKeys: String, CodingKey {
        case needsSetup = "needs_setup"
        case signedIn = "signed_in"
        case signInReason = "sign_in_reason"
        case signInText = "sign_in_text"
    }

    public init(needsSetup: Bool, signedIn: Bool, signInReason: String? = nil, signInText: SignInText? = nil) {
        self.needsSetup = needsSetup
        self.signedIn = signedIn
        self.signInReason = signInReason
        self.signInText = signInText
    }
}

/// The in-app web sheet the core asks for with `open_sheet`: a page, its title, the URL
/// prefixes that end it (sent to `sheet_redirected` instead of loading), and the sites whose
/// cookies the core reads while it is up.
public struct WebSheetRequest: Decodable, Sendable, Equatable, Identifiable {
    public var url: String
    public var title: String?
    public var stopAt: [String]
    public var cookieOrigins: [String]

    public var id: String { url }

    enum CodingKeys: String, CodingKey {
        case url, title
        case stopAt = "stop_at"
        case cookieOrigins = "cookie_origins"
    }

    public init(url: String, title: String? = nil, stopAt: [String] = [], cookieOrigins: [String] = []) {
        self.url = url
        self.title = title
        self.stopAt = stopAt
        self.cookieOrigins = cookieOrigins
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        url = try c.decode(String.self, forKey: .url)
        title = try c.decodeIfPresent(String.self, forKey: .title)
        stopAt = try c.decodeIfPresent([String].self, forKey: .stopAt) ?? []
        cookieOrigins = try c.decodeIfPresent([String].self, forKey: .cookieOrigins) ?? []
    }

    /// Whether loading `url` ends the sheet: it starts with one of the `stop_at` prefixes.
    public func stops(at url: URL) -> Bool {
        let text = url.absoluteString
        return stopAt.contains { !$0.isEmpty && text.hasPrefix($0) }
    }
}

extension WebCookie {
    /// Whether the cookie is sent to `host`, or was set for it or a subdomain: what clearing a
    /// site's cookies removes.
    public func matches(host: String) -> Bool {
        let host = host.lowercased()
        let domain = domain.lowercased().drop(while: { $0 == "." })
        guard !domain.isEmpty else { return false }
        return host == domain || host.hasSuffix("." + domain) || belongs(to: host)
    }

    /// The site a cookie origin names, as `belongs(to:)` takes it: its host without `www.`.
    public static func site(ofOrigin origin: String) -> String? {
        guard let host = URL(string: origin)?.host?.lowercased(), !host.isEmpty else { return nil }
        return host.hasPrefix("www.") ? String(host.dropFirst(4)) : host
    }
}
