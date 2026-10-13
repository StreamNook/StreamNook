import Foundation
import Observation
import SNKit

/// The Twitch sign-in on Twitch's own login and Authorize pages. The core starts it
/// (`begin_twitch_web_login`: the authorize page, with its CSRF state kept in Rust), the sheet
/// shows those pages, and the redirect's result goes back to the core
/// (`finish_twitch_web_login`), which stores the token and emits `twitch-login-complete` exactly
/// as the Android device-code login does. The sheet's cookies reach the core as they change,
/// which is the web session playback and the site's own requests ride on.
@Observable @MainActor
final class SignIn {
    /// The sign-in the sheet shows while it is up.
    struct Page: Identifiable {
        let authorize: URL
        let redirect: URL
        var id: String { authorize.absoluteString }
    }

    private(set) var user: TwitchUser?
    /// A sign-in is under way: the sheet is up, or the core is finishing it.
    private(set) var busy = false
    /// The redirect reached the core, which is exchanging its code.
    private(set) var finishing = false
    private(set) var error: String?
    var page: Page?

    /// The redirect's result, as `finish_twitch_web_login` takes it.
    private struct Redirect: Encodable {
        let code: String?
        let state: String?
        let error: String?
    }

    private(set) var cookies: CookieSync?
    private var core: CoreBridge?
    private var waiting: Task<Void, Never>?

    /// Pushes the cookies WebKit kept from earlier launches, then asks the core who is signed in.
    func boot(core: CoreBridge) async {
        self.core = core
        let sync = CookieSync(core: core)
        cookies = sync
        await sync.start()
        await refreshUser(core: core)
    }

    func begin(core: CoreBridge) async {
        guard page == nil, !busy else { return }
        busy = true
        finishing = false
        error = nil
        listen(core: core)
        do {
            let login = try await core.invoke("begin_twitch_web_login", args: [:], as: WebLogin.self)
            guard let authorize = URL(string: login.url), let redirect = URL(string: login.redirect) else {
                throw CoreFailure.command(json: #""Twitch's sign-in page is not a link""#)
            }
            page = Page(authorize: authorize, redirect: redirect)
            log.notice("sign-in sheet opened")
        } catch {
            waiting?.cancel()
            busy = false
            self.error = "\(error)"
            log.error("begin_twitch_web_login failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The sheet reached the redirect. The core checks the state, exchanges the code and answers
    /// with `twitch-login-complete` or `twitch-login-error`, which close the sheet.
    func redirected(code: String?, state: String?, error: String?) {
        guard let core, !finishing else { return }
        finishing = true
        log.notice("sign-in redirect reached (code \(code != nil ? "present" : "absent", privacy: .public), error \(error ?? "none", privacy: .public))")
        let args = (try? JSONEncoder().encode(Redirect(code: code, state: state, error: error)))
            .map { String(decoding: $0, as: UTF8.self) } ?? "{}"
        Task.detached {
            // The outcome arrives as an event; the call's own answer only matters for the log.
            do {
                _ = try await core.invoke("finish_twitch_web_login", args)
            } catch {
                log.error("finish_twitch_web_login: \(String(describing: error), privacy: .public)")
            }
        }
    }

    /// The sheet went away. Closed by hand before the redirect, nothing is waiting any more.
    func sheetClosed() {
        guard busy, !finishing else { return }
        waiting?.cancel()
        busy = false
        log.notice("sign-in sheet closed before the redirect")
    }

    private func listen(core: CoreBridge) {
        waiting?.cancel()
        let events = core.events.all()
        waiting = Task { [weak self] in
            for await event in events {
                guard let self, !Task.isCancelled else { return }
                switch event.name {
                case "twitch-login-complete":
                    await self.completed(core: core)
                    return
                case "twitch-login-error":
                    self.failed(CoreFailure.command(json: event.payloadJSON).description)
                    return
                default:
                    continue
                }
            }
        }
    }

    private func completed(core: CoreBridge) async {
        finishing = true
        page = nil
        await cookies?.push()
        await refreshUser(core: core)
        busy = false
        finishing = false
    }

    private func failed(_ message: String) {
        page = nil
        busy = false
        finishing = false
        error = message
        log.error("sign-in failed: \(message, privacy: .public)")
    }

    private func refreshUser(core: CoreBridge) async {
        do {
            user = try await core.invoke("get_user_info", args: [:], as: TwitchUser.self)
            log.notice("signed in as \(self.user?.login ?? "?", privacy: .public)")
        } catch {
            user = nil
            log.notice("not signed in: \(String(describing: error), privacy: .public)")
        }
    }
}
