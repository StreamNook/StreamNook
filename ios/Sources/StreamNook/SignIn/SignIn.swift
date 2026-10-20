import Foundation
import Observation
import SNKit
import WebKit

/// The Twitch sign-in on Twitch's own login and Authorize pages, and who is signed in. The core
/// starts a sign-in (`begin_twitch_web_login`: the authorize page, with its CSRF state kept in
/// Rust), the sheet shows those pages, and the redirect's result goes back to the core
/// (`finish_twitch_web_login`), which stores the token and emits `twitch-login-complete` exactly
/// as the Android device-code login does. The sheet's cookies reach the core as they change,
/// which is the web session playback and the site's own requests ride on.
///
/// Whether someone is signed in comes from `get_boot_state`, read at boot, on every sign-in,
/// roster change and return to the foreground, so every screen follows the core whichever way
/// the account came or went. The profile (`get_user_info`) only dresses it.
@Observable @MainActor
final class SignIn {
    /// The sign-in the sheet shows while it is up.
    struct Page: Identifiable {
        let authorize: URL
        let redirect: URL
        var id: String { authorize.absoluteString }
    }

    /// A Twitch sign-in is stored in the core.
    private(set) var signedIn = false
    private(set) var user: TwitchUser?
    /// Why StreamNook ended the last sign-in, for the signed-out landing; nil when it did not.
    private(set) var reason: String?
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
    private var watching: Task<Void, Never>?
    private var refreshing: Task<Void, Never>?

    /// Pushes the cookies WebKit kept from earlier launches, asks the core who is signed in, and
    /// from then on follows the core's account events.
    func boot(core: CoreBridge) async {
        self.core = core
        let sync = CookieSync(core: core)
        cookies = sync
        await sync.start()
        observe(core: core)
        await refresh(force: true)
    }

    func begin(core: CoreBridge) async {
        guard page == nil, !busy else { return }
        busy = true
        finishing = false
        error = nil
        do {
            let login = try await core.invoke("begin_twitch_web_login", args: [:], as: WebLogin.self)
            guard let authorize = URL(string: login.url), let redirect = URL(string: login.redirect) else {
                throw CoreFailure.command(json: #""Twitch's sign-in page is not a link""#)
            }
            page = Page(authorize: authorize, redirect: redirect)
            log.notice("sign-in sheet opened")
        } catch {
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
        busy = false
        log.notice("sign-in sheet closed before the redirect")
    }

    /// The core's account events for as long as the app runs: a finished sign-in wherever it
    /// started, a failed one while the sheet waits, and roster changes (sign-outs, a token the
    /// core dropped).
    private func observe(core: CoreBridge) {
        guard watching == nil else { return }
        let events = core.events.all()
        watching = Task { [weak self] in
            for await event in events {
                guard let self else { return }
                switch event.name {
                case "twitch-login-complete":
                    await self.completed()
                case "twitch-login-error":
                    if self.busy { self.failed(CoreFailure.command(json: event.payloadJSON).description) }
                case "account-roster-changed":
                    await self.refresh(force: true)
                default:
                    continue
                }
            }
        }
    }

    private func completed() async {
        let fromSheet = busy
        finishing = true
        page = nil
        await refresh(force: true)
        busy = false
        finishing = false
        log.notice("sign-in complete\(fromSheet ? "" : " (started elsewhere)", privacy: .public)")
        // After the state, never before it: the cookie store answers slowly once the sheet's web
        // view is gone, and the screens must not wait on it.
        Task { await cookies?.push() }
    }

    private func failed(_ message: String) {
        page = nil
        busy = false
        finishing = false
        error = message
        log.error("sign-in failed: \(message, privacy: .public)")
    }

    /// The core signed the account out: forget it, and empty the sign-in browser so the next
    /// sign-in starts at Twitch's login page instead of continuing as the person who left.
    func signedOut(core: CoreBridge) async {
        user = nil
        signedIn = false
        await WKWebsiteDataStore.default().removeData(ofTypes: WKWebsiteDataStore.allWebsiteDataTypes(), modifiedSince: .distantPast)
        await refresh(force: true)
        await cookies?.push()
        log.notice("signed out")
    }

    /// Reads who is signed in. `force` reads the profile again even when one is shown (a sign-in
    /// or account switch); otherwise only a change of state reads it. Overlapping calls share
    /// one read.
    func refresh(force: Bool = false) async {
        if let refreshing {
            await refreshing.value
            if !force { return }
        }
        let task = Task { await self.read(force: force) }
        refreshing = task
        await task.value
        if refreshing == task { refreshing = nil }
    }

    private func read(force: Bool) async {
        guard let core else { return }
        guard let boot = try? await core.invoke("get_boot_state", args: [:], as: BootState.self) else {
            // An older core without the boot answer: the profile alone says it.
            await readUser(core: core, signedInHint: nil)
            return
        }
        reason = boot.signedIn ? nil : boot.signInText?.sentence
        if !boot.signedIn {
            if signedIn || user != nil { log.notice("not signed in") }
            signedIn = false
            user = nil
            return
        }
        signedIn = true
        if force || user == nil { await readUser(core: core, signedInHint: true) }
    }

    /// The profile, tried three times a second apart: right after a sign-in Twitch can take a
    /// moment to answer for the new token. When it still fails, the sign-in is gone unless the
    /// network is what failed, as desktop and Android read it (`checkAuthStatus`). A read also
    /// asks for the viewer's FFZ subscriber status, which the core keeps for the emote picker.
    private func readUser(core: CoreBridge, signedInHint: Bool?) async {
        for attempt in 1...3 {
            do {
                let found = try await core.invoke("get_user_info", args: [:], as: TwitchUser.self)
                user = found
                signedIn = true
                log.notice("signed in as \(found.login, privacy: .public)")
                Task { _ = try? await core.invoke("ffz_local_user_status", args: [:]) }
                return
            } catch {
                if signedInHint == nil || (attempt == 3 && !Self.isNetwork(error)) {
                    user = nil
                    signedIn = false
                    log.notice("not signed in: \(String(describing: error), privacy: .public)")
                    return
                }
                log.notice("profile read \(attempt) failed: \(String(describing: error), privacy: .public)")
                if attempt < 3 { try? await Task.sleep(for: .seconds(1)) }
            }
        }
    }

    /// A failure the network caused rather than the sign-in, by the words desktop and Android
    /// read in `checkAuthStatus`.
    static func isNetwork(_ error: Error) -> Bool {
        let text = String(describing: error).lowercased()
        return ["error sending request", "timeout", "network", "dns error", "proxy", "failed to fetch",
                "500", "502", "503", "504"].contains { text.contains($0) }
    }
}
