import SNKit
import SwiftUI
import WebKit

/// The 7TV sign-in sheet ("Sign in to 7TV"): 7TV's login page in the app's own browser, so it rides
/// the Twitch session the app signed in with. 7TV finishes on 7tv.app by writing its session token
/// into the page's storage under `7tv-token`; there is no redirect to catch, so the sheet reads that
/// key and hands the token to the core, which works out whose account it is. Gives up after five
/// minutes; closing it resolves "not signed in".
struct SevenTvSignInSheet: View {
    let core: CoreBridge
    /// true when a token was stored.
    let finished: @MainActor (Bool) -> Void
    @State private var url: URL?
    @State private var done = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                if let url {
                    SevenTvWebView(url: url) { token in
                        Task { await store(token) }
                    }
                    .ignoresSafeArea(edges: .bottom)
                } else {
                    Color.clear
                }
            }
            .navigationTitle("Sign in to 7TV")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close", systemImage: Symbol.close) { finish(false) }
                }
            }
        }
        .task {
            let raw = (try? await core.invoke("get_seventv_login_url", args: [:])) ?? ""
            let text = (try? JSONDecoder().decode(String.self, from: Data(raw.utf8))) ?? "https://api.7tv.app/v4/auth/login?platform=twitch"
            url = URL(string: text)
        }
        .task {
            try? await Task.sleep(for: .seconds(300))
            finish(false)
        }
        .onDisappear { finish(false) }
    }

    private func store(_ token: String) async {
        guard !done else { return }
        let stored = (try? await core.invoke("store_seventv_token", args: ["accessToken": token, "userId": "", "twitchId": ""])) != nil
        finish(stored)
    }

    private func finish(_ stored: Bool) {
        guard !done else { return }
        done = true
        finished(stored)
        dismiss()
    }
}

private struct SevenTvWebView: UIViewRepresentable {
    let url: URL
    let captured: @MainActor (String) -> Void

    func makeCoordinator() -> Coordinator { Coordinator(captured: captured) }

    func makeUIView(context: Context) -> WKWebView {
        let view = WKWebView(frame: .zero, configuration: CookieSync.configuration())
        view.navigationDelegate = context.coordinator
        view.allowsBackForwardNavigationGestures = true
        view.load(URLRequest(url: url))
        context.coordinator.watch(view)
        return view
    }

    func updateUIView(_ view: WKWebView, context: Context) {}

    static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
        coordinator.stop()
    }

    @MainActor
    final class Coordinator: NSObject, WKNavigationDelegate {
        private let captured: @MainActor (String) -> Void
        private var timer: Timer?
        private var sent = false

        init(captured: @escaping @MainActor (String) -> Void) {
            self.captured = captured
        }

        /// Reads the page's `7tv-token` twice a second while the sheet is up.
        func watch(_ view: WKWebView) {
            timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self, weak view] _ in
                MainActor.assumeIsolated {
                    guard let self, let view, !self.sent, view.url?.host?.hasSuffix("7tv.app") == true else { return }
                    view.evaluateJavaScript("window.localStorage.getItem('7tv-token')") { value, _ in
                        MainActor.assumeIsolated {
                            guard let token = (value as? String)?.trimmingCharacters(in: CharacterSet(charactersIn: "\" ")),
                                  token.split(separator: ".").count == 3, !self.sent else { return }
                            self.sent = true
                            self.captured(token)
                        }
                    }
                }
            }
        }

        func stop() {
            timer?.invalidate()
            timer = nil
        }

        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
            guard let url = action.request.url else { return .allow }
            return SignInNavigator.allows(url) ? .allow : .cancel
        }
    }
}
