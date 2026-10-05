//! Read a webview's current URL without crashing on macOS.
//!
//! # Why `WebviewWindow::url()` is not used on macOS
//!
//! On macOS that call lands in wry's `url_from_webview`, which unwraps
//! `WKWebView.URL`. WebKit leaves that value nil until a navigation commits,
//! and keeps it nil when a network filter blocks the page. The unwrap runs on
//! the main thread inside the event loop, so a blocked site quits the whole
//! app instead of failing the one request. Every hidden or sign-in window this
//! app polls by URL (TikTok directory, YouTube PoToken, 7TV sign-in and token
//! refresh) was exposed.
//!
//! On macOS the URL is read from WebKit directly through `with_webview` (the
//! same main-thread hop as `platform::cookies`), and nil comes back as `None`.
//! Callers treat `None` as "not there yet", so their own timeouts report the
//! failure. Every other platform keeps `WebviewWindow::url()`, which does not
//! unwrap.

/// The window's current URL, or `None` while it has none (not yet
/// navigated, or the page was blocked).
pub async fn current_url(win: &crate::rt::WebviewWindow) -> Option<url::Url> {
    #[cfg(target_os = "macos")]
    {
        use objc2_web_kit::WKWebView;
        use std::time::Duration;

        let (tx, rx) = tokio::sync::oneshot::channel();
        win.with_webview(move |platform_webview| {
            // SAFETY: Tauri runs this closure on the main thread with a live
            // WKWebView, as in platform::cookies.
            let url = unsafe {
                let wk: &WKWebView = &*(platform_webview.inner() as *const WKWebView);
                wk.URL()
                    .and_then(|url| url.absoluteString())
                    .map(|url| url.to_string())
            };
            let _ = tx.send(url);
        })
        .ok()?;

        let url = tokio::time::timeout(Duration::from_secs(1), rx)
            .await
            .ok()?
            .ok()?;
        url?.parse().ok()
    }
    #[cfg(not(target_os = "macos"))]
    {
        win.url().ok()
    }
}
