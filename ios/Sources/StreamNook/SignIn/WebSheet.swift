import SNKit
import SwiftUI
import WebKit

/// The in-app web sheet the core opens with `open_sheet` (Kick's sign-in and consent, a
/// checkout): the page on the persistent default store, the same browser the Twitch sign-in
/// uses. A load whose URL starts with one of the request's `stop_at` prefixes is canceled and
/// handed to the core; the sheet stays up until the core closes it or the person does.
struct WebSheet: View {
    let host: HostRequests
    let request: WebSheetRequest
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            SheetWebView(request: request, load: host.sheetLoad, cookies: nil) { url in
                host.redirected(url)
            }
            .ignoresSafeArea(edges: .bottom)
            .navigationTitle(request.title ?? URL(string: request.url)?.host ?? "")
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
    let cookies: CookieSync?
    let stopped: @MainActor (URL) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(request: request, stopped: stopped) }

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView(frame: .zero, configuration: CookieSync.configuration())
        view.navigationDelegate = context.coordinator
        view.allowsBackForwardNavigationGestures = true
        context.coordinator.loaded = load
        if let url = URL(string: request.url) { view.load(URLRequest(url: url)) }
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {
        context.coordinator.request = request
        guard context.coordinator.loaded != load else { return }
        context.coordinator.loaded = load
        if let url = URL(string: request.url) { view.load(URLRequest(url: url)) }
    }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        var request: WebSheetRequest
        var loaded = 0
        private let stopped: @MainActor (URL) -> Void

        init(request: WebSheetRequest, stopped: @escaping @MainActor (URL) -> Void) {
            self.request = request
            self.stopped = stopped
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            guard action.targetFrame?.isMainFrame ?? true, let url = action.request.url else { return .allow }
            if request.stops(at: url) {
                stopped(url)
                return .cancel
            }
            return SignInNavigator.allows(url) ? .allow : .cancel
        }

        /// Nothing listens at a stop URL (a localhost callback), so a load of it that slipped
        /// past the policy check fails; its URL still carries the result.
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error) {
            guard let url = (error as NSError).userInfo[NSURLErrorFailingURLErrorKey] as? URL, request.stops(at: url) else { return }
            stopped(url)
        }
    }
}
