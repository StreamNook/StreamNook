import Foundation
import SNKit
import WebKit

/// Keeps the core's copy of the web sessions in step with WebKit's store. The store is the
/// default persistent one the sign-in and web sheets use, so a session survives relaunches; the
/// core keeps nothing on disk and gets the whole set again at every launch.
@MainActor
final class CookieSync: NSObject, WKHTTPCookieStoreObserver {
    private let core: CoreBridge
    private let store = WKWebsiteDataStore.default().httpCookieStore
    private var signedInSites: Set<String> = []
    /// A web view kept alive while the store is read: the store answers from WebKit's network
    /// process, which only runs while some web view exists, and a read made with none (after a
    /// sheet closed) can wait indefinitely.
    private var warm: WKWebView?
    private var cooling: Task<Void, Never>?
    /// Sites an open web sheet asked to have pushed besides the usual ones, by origin.
    private var sheetOrigins: [String] = []

    init(core: CoreBridge) {
        self.core = core
    }

    /// Starts watching the store and pushes what it already holds.
    func start() async {
        store.add(self)
        await push()
    }

    /// Also pushes these origins' cookies while a web sheet is up; empty when it closes.
    func setSheetOrigins(_ origins: [String]) {
        sheetOrigins = origins
    }

    /// Hands the core each site's cookies as the store holds them now.
    func push() async {
        let cookies = await allCookies()
        var pushed: Set<String> = []
        for site in WebSessionSite.allCases {
            pushed.insert(site.site)
            let json = WebCookie.snapshotJSON(cookies, site: site.site)
            try? core.setWebCookies(origin: site.origin, cookiesJSON: json)
            let signedIn = cookies.contains { $0.belongs(to: site.site) && $0.name == site.sessionCookie && !$0.value.isEmpty }
            if signedIn != signedInSites.contains(site.site) {
                if signedIn { signedInSites.insert(site.site) } else { signedInSites.remove(site.site) }
                log.notice("web session for \(site.site, privacy: .public): \(signedIn ? "signed in" : "signed out", privacy: .public)")
            }
        }
        for origin in sheetOrigins {
            guard let site = WebCookie.site(ofOrigin: origin), !pushed.contains(site) else { continue }
            pushed.insert(site)
            try? core.setWebCookies(origin: origin, cookiesJSON: WebCookie.snapshotJSON(cookies, site: site))
        }
    }

    /// Deletes every cookie the sites of these origins would see, then pushes the result.
    func clear(origins: [String]) async {
        let hosts = origins.compactMap { URL(string: $0)?.host?.lowercased() }
        guard !hosts.isEmpty else { return }
        let all = await withWarmView { await self.store.allCookies() }
        var removed = 0
        for cookie in all where hosts.contains(where: { WebCookie(cookie).matches(host: $0) }) {
            await store.deleteCookie(cookie)
            removed += 1
        }
        log.notice("cleared \(removed) cookies for \(hosts.joined(separator: ","), privacy: .public)")
        await push()
    }

    nonisolated func cookiesDidChange(in cookieStore: WKHTTPCookieStore) {
        Task { @MainActor in await self.push() }
    }

    private func allCookies() async -> [WebCookie] {
        await withWarmView { await self.store.allCookies().map(WebCookie.init) }
    }

    /// Runs `body` with a web view alive, kept a few seconds after for reads that follow.
    private func withWarmView<T>(_ body: () async -> T) async -> T {
        cooling?.cancel()
        if warm == nil { warm = WKWebView(frame: .zero, configuration: Self.configuration()) }
        let result = await body()
        cooling = Task { [weak self] in
            try? await Task.sleep(for: .seconds(10))
            guard !Task.isCancelled else { return }
            self?.warm = nil
        }
        return result
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
