import SNKit
import SwiftUI
import WebKit

/// A captured 7TV session token, stored the way desktop and Android store it (`sevenTvConnect.ts`):
/// with the 7TV user id read out of the token and the signed-in Twitch id beside it.
enum SevenTvToken {
    /// The 7TV user id: the token is a JWT and its subject claim is the id.
    static func userID(_ token: String) -> String {
        let parts = token.split(separator: ".")
        guard parts.count == 3 else { return "" }
        var b64 = String(parts[1]).replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        while b64.count % 4 != 0 { b64 += "=" }
        guard let data = Data(base64Encoded: b64),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return "" }
        if let sub = payload["sub"] { return "\(sub)" }
        if let id = payload["user_id"] { return "\(id)" }
        return ""
    }

    /// True once the core stored it. A token that names no user is not stored.
    @MainActor
    static func store(_ token: String, core: CoreBridge, twitchID: String) async -> Bool {
        let userID = userID(token)
        guard !userID.isEmpty else {
            log.warning("7TV token captured but carried no user id")
            return false
        }
        let args: [String: Any?] = ["accessToken": token, "userId": userID, "twitchId": twitchID]
        return (try? await core.invoke("store_seventv_token", args: args)) != nil
    }
}

/// Keeping the 7TV session alive, as desktop and Android do (`sevenTvSession.ts`). 7TV's tokens last
/// 30 days and an expired one reads as signed out, so a token inside a week of its expiry is
/// re-minted with nothing on screen while the Twitch session is still good, never more than once
/// per twelve hours. No stored token (never connected, or signed out on purpose) is left alone.
@MainActor
enum SevenTvSession {
    static let refreshAhead: TimeInterval = 7 * 24 * 60 * 60
    static let retryWindow: TimeInterval = 12 * 60 * 60
    private static let stampKey = "sn-7tv-refresh-attempt"

    static func shouldRefresh(expiresAt: Double?, now: Date, lastAttempt: Date?) -> Bool {
        guard let expiresAt else { return false }
        if let lastAttempt, now.timeIntervalSince(lastAttempt) < retryWindow { return false }
        return expiresAt - now.timeIntervalSince1970 <= refreshAhead
    }

    private struct Status: Decodable {
        let expiresAt: Double?
        enum CodingKeys: String, CodingKey { case expiresAt = "expires_at" }
    }

    /// Checks the stored token and re-mints it when it is close to expiring.
    static func keepAlive(core: CoreBridge, twitchID: String) async {
        guard let status = try? await core.invoke("get_seventv_auth_status", args: [:], as: Status.self) else { return }
        let defaults = UserDefaults.standard
        let now = Date()
        guard shouldRefresh(expiresAt: status.expiresAt, now: now, lastAttempt: defaults.object(forKey: stampKey) as? Date) else {
            return
        }
        defaults.set(now, forKey: stampKey)
        guard let token = await SilentSevenTvCapture.run(core: core) else {
            log.notice("7TV session refresh: no fresh token (Twitch wanted a click, or 7TV did not answer)")
            return
        }
        let stored = await SevenTvToken.store(token, core: core, twitchID: twitchID)
        log.notice("7TV session refresh: \(stored ? "refreshed" : "failed", privacy: .public)")
    }
}

/// 7TV's sign-in run with nothing on screen: while the Twitch session in the shared cookie store
/// is alive, 7TV's login round-trips through Twitch on its own and writes a fresh token into the
/// page, read the same way the visible sheet reads it. If Twitch wants a click, nothing arrives and
/// it gives up after 35 seconds.
@MainActor
final class SilentSevenTvCapture: NSObject, WKNavigationDelegate {
    private var view: WKWebView?
    private var timer: Timer?
    private var continuation: CheckedContinuation<String?, Never>?

    static func run(core: CoreBridge) async -> String? {
        let raw = (try? await core.invoke("get_seventv_login_url", args: [:], as: String.self))
            ?? "https://api.7tv.app/v4/auth/login?platform=twitch"
        guard let url = URL(string: raw) else { return nil }
        let capture = SilentSevenTvCapture()
        return await capture.capture(url)
    }

    private func capture(_ url: URL) async -> String? {
        await withCheckedContinuation { (continuation: CheckedContinuation<String?, Never>) in
            self.continuation = continuation
            // In the window (a point, see-through, untouchable) so the page runs as a shown one does.
            let view = WKWebView(frame: CGRect(x: 0, y: 0, width: 1, height: 1), configuration: CookieSync.configuration())
            view.alpha = 0.01
            view.isUserInteractionEnabled = false
            view.navigationDelegate = self
            Self.keyWindow?.addSubview(view)
            self.view = view
            view.load(URLRequest(url: url))
            timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
                MainActor.assumeIsolated { self?.poll() }
            }
            Task { [weak self] in
                try? await Task.sleep(for: .seconds(35))
                self?.finish(nil)
            }
        }
    }

    private func poll() {
        guard let view, view.url?.host?.hasSuffix("7tv.app") == true else { return }
        view.evaluateJavaScript("window.localStorage.getItem('7tv-token')") { [weak self] value, _ in
            MainActor.assumeIsolated {
                guard let token = (value as? String)?.trimmingCharacters(in: CharacterSet(charactersIn: "\" ")),
                      token.split(separator: ".").count == 3 else { return }
                self?.finish(token)
            }
        }
    }

    private func finish(_ token: String?) {
        guard let continuation else { return }
        self.continuation = nil
        timer?.invalidate()
        timer = nil
        view?.stopLoading()
        view?.removeFromSuperview()
        view = nil
        continuation.resume(returning: token)
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
        guard let url = action.request.url else { return .allow }
        return SignInNavigator.allows(url) ? .allow : .cancel
    }

    private static var keyWindow: UIWindow? {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows).first(where: \.isKeyWindow)
    }
}
