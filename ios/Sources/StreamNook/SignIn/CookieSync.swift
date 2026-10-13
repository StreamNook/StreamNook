import Foundation
import SNKit
import WebKit

/// Keeps the core's copy of the sign-in cookies in step with WebKit's store. The store is the
/// default persistent one the sign-in sheet uses, so a session survives relaunches; the core
/// keeps nothing on disk and gets the whole set again at every launch.
@MainActor
final class CookieSync: NSObject, WKHTTPCookieStoreObserver {
    private let core: CoreBridge
    private let store = WKWebsiteDataStore.default().httpCookieStore
    private var signedInSites: Set<String> = []
    private var warm: WKWebView?

    init(core: CoreBridge) {
        self.core = core
    }

    /// Starts watching the store and pushes what it already holds.
    func start() async {
        store.add(self)
        // The store answers from WebKit's network process, which a process only has once it
        // has made a web view; this one lives just long enough for the first read.
        warm = WKWebView(frame: .zero, configuration: Self.configuration())
        await push()
        warm = nil
    }

    /// Hands the core each site's cookies as the store holds them now.
    func push() async {
        let cookies = await store.allCookies().map(WebCookie.init)
        for site in WebSessionSite.allCases {
            let json = WebCookie.snapshotJSON(cookies, site: site.site)
            try? core.setWebCookies(origin: site.origin, cookiesJSON: json)
            let signedIn = cookies.contains { $0.belongs(to: site.site) && $0.name == site.sessionCookie && !$0.value.isEmpty }
            if signedIn != signedInSites.contains(site.site) {
                if signedIn { signedInSites.insert(site.site) } else { signedInSites.remove(site.site) }
                log.notice("web session for \(site.site, privacy: .public): \(signedIn ? "signed in" : "signed out", privacy: .public)")
            }
        }
    }

    nonisolated func cookiesDidChange(in cookieStore: WKHTTPCookieStore) {
        Task { @MainActor in await self.push() }
    }

    /// The persistent default store and a Mobile Safari user agent: WebKit's own agent carries
    /// no Safari version, which sites read as an old or unknown browser.
    static func configuration() -> WKWebViewConfiguration {
        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .default()
        let os = ProcessInfo.processInfo.operatingSystemVersion
        configuration.applicationNameForUserAgent = "Version/\(os.majorVersion).\(os.minorVersion) Mobile/15E148 Safari/604.1"
        return configuration
    }
}

extension WebCookie {
    init(_ cookie: HTTPCookie) {
        self.init(
            name: cookie.name, value: cookie.value, domain: cookie.domain, path: cookie.path,
            expires: cookie.expiresDate?.timeIntervalSince1970, secure: cookie.isSecure,
            httpOnly: cookie.isHTTPOnly
        )
    }
}
