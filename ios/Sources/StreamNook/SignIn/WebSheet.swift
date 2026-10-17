import SNKit
import SwiftUI
import WebKit

/// The in-app web sheet the core opens with `open_sheet` (Kick's sign-in and consent, a
/// checkout): the page on the persistent default store, the same browser the Twitch sign-in
/// uses. A load whose URL starts with one of the request's `stop_at` prefixes is canceled and
/// handed to the core; the sheet stays up until the core closes it or the person does.
struct WebSheet: View {
    let host: HostRequests
    /// The request the sheet opened with; the core can point the open sheet somewhere new
    /// (Kick's Authorize page after its login), and that newer request is the one loaded.
    let request: WebSheetRequest
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        // Read from the host every time: a request captured when the sheet appeared would
        // reload the first page when the core points the sheet somewhere new.
        let current = host.sheet ?? request
        NavigationStack {
            SheetWebView(request: current, load: host.sheetLoad, stopped: { url in
                host.redirected(url)
            }, finished: { url in
                host.pageFinished(url)
            })
            .ignoresSafeArea(edges: .bottom)
            .navigationTitle(current.title ?? URL(string: current.url)?.host ?? "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close", systemImage: Symbol.close) { dismiss() }
                }
            }
        }
    }
}

private struct SheetWebView: UIViewRepresentable {
    let request: WebSheetRequest
    /// Changes when the core points the open sheet somewhere new.
    let load: Int
    let stopped: @MainActor (URL) -> Void
    /// A page finished loading.
    let finished: @MainActor (URL) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(request: request, stopped: stopped, finished: finished) }

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView(frame: .zero, configuration: CookieSync.configuration())
        view.navigationDelegate = context.coordinator
        view.allowsBackForwardNavigationGestures = true
        context.coordinator.loaded = load
        Self.open(request, in: view)
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        context.coordinator.request = request
        guard context.coordinator.loaded != load else { return }
        context.coordinator.loaded = load
        Self.open(request, in: view)
    }

    private static func open(_ request: WebSheetRequest, in view: WKWebView) {
        guard let url = URL(string: request.url) else { return }
        // Host and path only: the query carries a sign-in's state and challenge.
        log.notice("web sheet loads \(url.host ?? "", privacy: .public)\(url.path, privacy: .public)")
        view.load(URLRequest(url: url))
    }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        var request: WebSheetRequest
        var loaded = 0
        private let stopped: @MainActor (URL) -> Void
        private let finished: @MainActor (URL) -> Void

        init(request: WebSheetRequest, stopped: @escaping @MainActor (URL) -> Void,
             finished: @escaping @MainActor (URL) -> Void) {
            self.request = request
            self.stopped = stopped
            self.finished = finished
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            guard action.targetFrame?.isMainFrame ?? true, let url = action.request.url else { return .allow }
            if request.stops(at: url) {
                stopped(url)
                return .cancel
            }
            return SignInNavigator.allows(url) ? .allow : .cancel
        }

        /// Kick's mobile site ignores `/login` and shows its home page with the form closed, so on
        /// that page the sheet presses Kick's own Log In button (twice a second, up to 20 times,
        /// stopping once a password field shows).
        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            guard let url = webView.url else { return }
            finished(url)
            guard KickLogin.opensForm(url) else { return }
            webView.evaluateJavaScript(KickLogin.script) { _, error in
                if let error { log.notice("kick login press failed: \(String(describing: error), privacy: .public)") }
            }
        }

        /// Nothing listens at a stop URL (a localhost callback), so a load of it that slipped
        /// past the policy check fails; its URL still carries the result.
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error) {
            guard let url = (error as NSError).userInfo[NSURLErrorFailingURLErrorKey] as? URL, request.stops(at: url) else { return }
            stopped(url)
        }
    }
}
