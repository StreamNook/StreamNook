import Foundation
import Observation
import SNKit

/// The Twitch sign-in, run the way the Android app runs it: the core starts a device login,
/// the sheet shows Twitch's own activate page (which asks for the password first when nobody is
/// signed in), and the core's poll finishes the sign-in and emits `twitch-login-complete`. The
/// sheet's cookies reach the core as they change, which is the web session playback and the
/// site's own requests ride on.
@Observable @MainActor
final class SignIn {
    /// The page the sheet shows while it is up.
    struct Page: Identifiable {
        let url: URL
        var id: String { url.absoluteString }
    }

    private(set) var user: TwitchUser?
    /// A sign-in is under way: the sheet is up, or approval landed and the core is collecting
    /// the token.
    private(set) var busy = false
    private(set) var error: String?
    var page: Page?

    private(set) var cookies: CookieSync?
    private var waiting: Task<Void, Never>?
    private var approved = false

    /// Pushes the cookies WebKit kept from earlier launches, then asks the core who is signed in.
    func boot(core: CoreBridge) async {
        let sync = CookieSync(core: core)
        cookies = sync
        await sync.start()
        await refreshUser(core: core)
    }

    func begin(core: CoreBridge) async {
        guard page == nil, !busy else { return }
        busy = true
        approved = false
        error = nil
        listen(core: core)
        do {
            let login = try await core.invoke("twitch_login", args: [:], as: DeviceLogin.self)
            guard let url = URL(string: login.verificationURI) else {
                throw CoreFailure.command(json: #""Twitch sent an activate page that is not a link""#)
            }
            page = Page(url: url)
            log.notice("sign-in sheet opened")
        } catch {
            waiting?.cancel()
            busy = false
            self.error = "\(error)"
            log.error("twitch_login failed: \(String(describing: error), privacy: .public)")
        }
    }

    /// The sheet saw approval land. It closes now; the core's poll finishes the sign-in.
    func approvalLanded() {
        approved = true
        page = nil
        log.notice("sign-in approved; waiting for the core")
    }

    /// The sheet went away. Closed by hand before approval, nothing is waiting any more; the
    /// core's device-code poll runs out on its own, as on Android.
    func sheetClosed() {
        guard !approved, busy else { return }
        waiting?.cancel()
        busy = false
        log.notice("sign-in sheet closed before approval")
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
        // The sheet closing now is the sign-in finishing, not someone giving up on it.
        approved = true
        page = nil
        await cookies?.push()
        await refreshUser(core: core)
        busy = false
        approved = false
    }

    private func failed(_ message: String) {
        page = nil
        busy = false
        approved = false
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
