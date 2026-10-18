import SNKit
import SwiftUI
import WebKit

/// Twitch's own login and Authorize pages in a web view on the persistent default store.
struct SignInSheet: View {
    let signIn: SignIn
    let page: SignIn.Page
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            TwitchWebView(page: page, cookies: signIn.cookies) { code, state, error in
                signIn.redirected(code: code, state: state, error: error)
            }
            .ignoresSafeArea(edges: .bottom)
            .overlay {
                if signIn.finishing {
                    ProgressView()
                        .controlSize(.large)
                        .padding(24)
                        .glassEffect()
                }
            }
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
    let page: SignIn.Page
    let cookies: CookieSync?
    let redirected: @MainActor (String?, String?, String?) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(page: page, cookies: cookies, redirected: redirected)
    }

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView(frame: .zero, configuration: CookieSync.configuration())
        view.navigationDelegate = context.coordinator
        view.allowsBackForwardNavigationGestures = true
        context.coordinator.watch(view)
        view.load(URLRequest(url: page.authorize))
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        private var navigator: SignInNavigator
        private let cookies: CookieSync?
        private let redirected: @MainActor (String?, String?, String?) -> Void
        private var done = false
        private var urlWatch: NSKeyValueObservation?

        init(page: SignIn.Page, cookies: CookieSync?, redirected: @escaping @MainActor (String?, String?, String?) -> Void) {
            navigator = SignInNavigator(authorize: page.authorize, redirect: page.redirect)
            self.cookies = cookies
            self.redirected = redirected
        }

        /// Twitch's site moves between pages in place, which no navigation callback reports; the
        /// web view's URL changes either way.
        func watch(_ view: WKWebView) {
            urlWatch = view.observe(\.url, options: [.new]) { [weak self] view, _ in
                MainActor.assumeIsolated {
                    guard let self, let url = view.url else { return }
                    if case .returnToAuthorize(let authorize) = self.step(url) {
                        view.load(URLRequest(url: authorize))
                    }
                }
            }
        }

        /// Every main-frame load, redirects included, so the redirect to the callback is caught
        /// before anything tries to load it.
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            guard action.targetFrame?.isMainFrame ?? true, let url = action.request.url else { return .allow }
            guard SignInNavigator.allows(url) else {
                log.notice("sign-in sheet ignored a \(url.scheme ?? "?", privacy: .public): link")
                return .cancel
            }
            // Host and path only: the query carries the code.
            log.notice("sign-in nav \(url.host ?? "", privacy: .public)\(url.path, privacy: .public)")
            switch step(url) {
            case .stay:
                return .allow
            case .returnToAuthorize(let authorize):
                webView.load(URLRequest(url: authorize))
                return .cancel
            case .redirect:
                return .cancel
            }
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            Task { await cookies?.push() }
        }

        /// Nothing listens at the redirect, so a load of it that slipped past the policy check
        /// fails; its URL still carries the result.
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: any Error) {
            guard let url = (error as NSError).userInfo[NSURLErrorFailingURLErrorKey] as? URL else { return }
            _ = step(url)
        }

        private func step(_ url: URL) -> SignInNavigator.Step {
            guard !done else { return .stay }
            let step = navigator.arrived(at: url)
            switch step {
            case .stay: break
            case .returnToAuthorize:
                log.notice("sign-in landed home; back to the authorize page")
            case .redirect(let code, let state, let error):
                done = true
                Task {
                    // The login just set the session cookies; the core gets them before the code.
                    await cookies?.push()
                    redirected(code, state, error)
                }
            }
            return step
        }
    }
}
