import Foundation
import Observation
import SNKit
import UIKit

/// What the core asks the shell to do for it (`HostSink.host_request`): open a link outside the
/// app, copy text, share a file, show or close the in-app web sheet, clear a site's cookies.
@Observable @MainActor
final class HostRequests {
    /// The web sheet while it is up.
    private(set) var sheet: WebSheetRequest?
    /// Bumped when the core points an open sheet somewhere new.
    private(set) var sheetLoad = 0

    private var core: CoreBridge?
    private var cookies: CookieSync?
    private var listening: Task<Void, Never>?
    /// The core took the sheet down itself, so its dismissal is not the person's.
    private var closedByCore = false

    func start(core: CoreBridge, cookies: CookieSync?) {
        self.core = core
        self.cookies = cookies
        guard listening == nil else { return }
        let requests = core.events.requests()
        listening = Task { [weak self] in
            for await request in requests {
                guard let self else { return }
                await self.handle(request)
            }
        }
    }

    func handle(_ request: HostRequest) async {
        let payload = (try? JSONSerialization.jsonObject(with: Data(request.payloadJSON.utf8))) as? [String: Any] ?? [:]
        log.notice("host request \(request.kind, privacy: .public)")
        switch request.kind {
        case "open_url":
            guard let text = payload["url"] as? String, let url = URL(string: text),
                  ["http", "https"].contains(url.scheme?.lowercased() ?? "") else { return }
            await UIApplication.shared.open(url)
        case "copy_text":
            UIPasteboard.general.string = payload["text"] as? String ?? ""
        case "share":
            guard let path = payload["path"] as? String else { return }
            share(file: URL(fileURLWithPath: path), name: payload["name"] as? String)
        case "open_sheet":
            guard let next = try? JSONDecoder().decode(WebSheetRequest.self, from: Data(request.payloadJSON.utf8)) else { return }
            cookies?.setSheetOrigins(next.cookieOrigins)
            let reload = sheet != nil
            closedByCore = false
            // The request first, then the bump: an open sheet reloads on the new page.
            sheet = next
            if reload { sheetLoad += 1 }
            await cookies?.push()
        case "close_sheet":
            guard sheet != nil else { return }
            closedByCore = true
            sheet = nil
        case "clear_cookies":
            await cookies?.clear(origins: payload["origins"] as? [String] ?? [])
        case "exit", "restart":
            // An iPhone app does not quit or relaunch itself.
            break
        default:
            log.error("host request \(request.kind, privacy: .public) not handled")
        }
    }

    /// The sheet stopped at one of its `stop_at` URLs: the core takes it from there.
    func redirected(_ url: URL) {
        guard let core else { return }
        log.notice("web sheet stopped at \(url.host ?? "", privacy: .public)\(url.path, privacy: .public)")
        Task.detached { _ = try? await core.invoke("sheet_redirected", args: ["url": url.absoluteString]) }
        Task { await cookies?.push() }
    }

    /// A page in the sheet finished loading: the core gets the sites' cookies now, not only when
    /// WebKit's store says they changed, so a sign-in it polls for is seen at once.
    func pageFinished(_ url: URL) {
        log.notice("web sheet shows \(url.host ?? "", privacy: .public)\(url.path, privacy: .public)")
        Task { await cookies?.push() }
    }

    /// The sheet went away. Dismissed by the person, the core hears it; closed by the core, not.
    func sheetDismissed() {
        if closedByCore {
            closedByCore = false
            settle()
            return
        }
        // A swipe reports twice (the binding, then the dismissal); the first one counts.
        guard sheet != nil else { return }
        sheet = nil
        settle()
        guard let core else { return }
        log.notice("web sheet closed by the person")
        Task.detached { _ = try? await core.invoke("sheet_closed", "{}") }
    }

    /// The sheet is gone: its extra sites stop being pushed, after one last push.
    private func settle() {
        Task {
            await cookies?.push()
            cookies?.setSheetOrigins([])
        }
    }

    /// A share sheet for a file, offered under `name` when given (Save to Files is one choice).
    private func share(file: URL, name: String?) {
        var item = file
        if let name, !name.isEmpty, name != file.lastPathComponent {
            let named = FileManager.default.temporaryDirectory.appending(path: name)
            try? FileManager.default.removeItem(at: named)
            if (try? FileManager.default.copyItem(at: file, to: named)) != nil { item = named }
        }
        guard let top = Self.topController() else { return }
        let activity = UIActivityViewController(activityItems: [item], applicationActivities: nil)
        activity.popoverPresentationController?.sourceView = top.view
        top.present(activity, animated: true)
    }

    static func topController() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let window = scenes.flatMap(\.windows).first { $0.isKeyWindow } ?? scenes.first?.windows.first
        var top = window?.rootViewController
        while let next = top?.presentedViewController { top = next }
        return top
    }
}
