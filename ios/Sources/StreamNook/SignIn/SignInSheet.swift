import SNKit
import SwiftUI
import WebKit

/// Twitch's own sign-in and activate pages in a web view on the persistent default store.
struct SignInSheet: View {
    let signIn: SignIn
    let page: SignIn.Page
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            TwitchWebView(url: page.url, cookies: signIn.cookies) {
                signIn.approvalLanded()
            }
            .ignoresSafeArea(edges: .bottom)
            .navigationTitle("Sign in to Twitch")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close", systemImage: "xmark") { dismiss() }
                }
            }
        }
    }
}

private struct TwitchWebView: UIViewRepresentable {
    let url: URL
    let cookies: CookieSync?
    let approved: @MainActor () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(activate: url, cookies: cookies, approved: approved)
    }

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView(frame: .zero, configuration: CookieSync.configuration())
        view.navigationDelegate = context.coordinator
        view.allowsBackForwardNavigationGestures = true
        context.coordinator.watch(view)
        view.load(URLRequest(url: url))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        private var navigator: ActivateNavigator
        private let cookies: CookieSync?
        private let approved: @MainActor () -> Void
        private var done = false
        private var urlWatch: NSKeyValueObservation?

        init(activate: URL, cookies: CookieSync?, approved: @escaping @MainActor () -> Void) {
            navigator = ActivateNavigator(activate: activate)
            self.cookies = cookies
            self.approved = approved
        }

        /// Twitch's site moves between pages in place, which no navigation callback reports; the
        /// web view's URL changes either way.
        func watch(_ view: WKWebView) {
            urlWatch = view.observe(\.url, options: [.new]) { [weak self] view, _ in
                MainActor.assumeIsolated {
                    guard let self, let url = view.url else { return }
                    if case .returnToActivate(let activate) = self.step(url) {
                        view.load(URLRequest(url: activate))
                    }
                }
            }
        }

        /// Every main-frame load, redirects included, so the authorize hop is seen even when it
        /// never draws a page.
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            guard action.targetFrame?.isMainFrame ?? true, let url = action.request.url else { return .allow }
            guard ActivateNavigator.allows(url) else {
                log.notice("sign-in sheet ignored a \(url.scheme ?? "?", privacy: .public): link")
                return .cancel
            }
            // Host and path only: a fragment or query can carry a credential.
            log.notice("sign-in nav \(url.host ?? "", privacy: .public)\(url.path, privacy: .public)")
            if case .returnToActivate(let activate) = step(url) {
                webView.load(URLRequest(url: activate))
                return .cancel
            }
            return .allow
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            Task { await cookies?.push() }
        }

        private func step(_ url: URL) -> ActivateNavigator.Step {
            guard !done else { return .stay }
            let step = navigator.arrived(at: url)
            switch step {
            case .stay: break
            case .returnToActivate:
                log.notice("sign-in landed home; back to the activate page")
            case .approved:
                done = true
                log.notice("sign-in approval landed on \(url.host ?? "", privacy: .public)\(url.path, privacy: .public)")
                Task { await cookies?.push() }
                approved()
            }
            return step
        }
    }
}
