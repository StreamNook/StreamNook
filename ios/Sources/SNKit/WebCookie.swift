import Foundation

/// A cookie as the core's `set_web_cookies` takes it: the fields of WebKit's cookie store, with
/// `expires` in seconds since 1970 and nil for a session cookie.
public struct WebCookie: Codable, Sendable, Equatable {
    public var name: String
    public var value: String
    public var domain: String
    public var path: String
    public var expires: Double?
    public var secure: Bool
    public var httpOnly: Bool

    public init(name: String, value: String, domain: String, path: String = "/", expires: Double? = nil,
                secure: Bool = false, httpOnly: Bool = false) {
        self.name = name
        self.value = value
        self.domain = domain
        self.path = path
        self.expires = expires
        self.secure = secure
        self.httpOnly = httpOnly
    }

    /// Whether the cookie was set for `site` itself or any subdomain of it (`.twitch.tv` and
    /// `www.twitch.tv` belong to `twitch.tv`; `nottwitch.tv` does not).
    public func belongs(to site: String) -> Bool {
        let domain = domain.lowercased().drop(while: { $0 == "." })
        let site = site.lowercased()
        guard !domain.isEmpty else { return false }
        return domain == site || domain.hasSuffix("." + site)
    }

    /// The cookies of `site` out of a whole store, as the JSON list the core takes. The list is
    /// the site's entire jar each time, so a cookie the site deleted leaves the core's copy too.
    public static func snapshotJSON(_ cookies: [WebCookie], site: String) -> String {
        let mine = cookies.filter { $0.belongs(to: site) }
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        guard let data = try? encoder.encode(mine) else { return "[]" }
        return String(decoding: data, as: UTF8.self)
    }
}

/// The sites whose web sessions the core reads, with the origin each snapshot is filed under.
public enum WebSessionSite: CaseIterable, Sendable {
    case twitch

    public var origin: String {
        switch self {
        case .twitch: return "https://www.twitch.tv"
        }
    }

    public var site: String {
        switch self {
        case .twitch: return "twitch.tv"
        }
    }

    /// The cookie whose presence means someone is signed in to the site.
    public var sessionCookie: String {
        switch self {
        case .twitch: return "auth-token"
        }
    }
}
