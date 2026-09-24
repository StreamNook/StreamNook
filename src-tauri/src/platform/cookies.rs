//! Read a webview's HTTP cookie jar without stalling the UI thread.
//!
//! # Why `WebviewWindow::cookies_for_url` is not used on macOS
//!
//! On macOS that call lands in wry's `cookies()`, which asks
//! `WKHTTPCookieStore` for the jar and then PUMPS A NESTED RUN LOOP on the
//! main thread until WebKit's completion block fires. Every caller in this
//! app is a background task, so the call executes inside tao's user-event
//! dispatch, and tao holds its event-callback `Mutex` for the whole
//! dispatch. Three AppKit entry points call straight back into that callback
//! without tao's re-entrancy guard: a content view `drawRect:`, a Dock-icon
//! reopen and a deep link. Any of them firing during the nested pump relocks
//! a mutex the main thread already holds. The main thread never returns, the
//! traffic lights stop answering, and the only way out is Force Quit. That
//! was the first macOS build's freeze (8.6.2, reported 2026-09-15).
//!
//! The fix has the same shape as the Windows COM path in
//! `twitch_auth_service`: from the main thread (`with_webview` runs there
//! and returns at once) hand WebKit a completion block; the block delivers
//! the jar through a channel on a later, ordinary run-loop turn; the async
//! caller awaits it with a deadline. Nothing pumps, so nothing re-enters.
//!
//! Other Unix targets keep `Webview::cookies()`: WebKitGTK's implementation
//! does not pump. Windows keeps its hand-rolled `ICoreWebView2CookieManager`
//! path inside the auth services and never calls into here.

/// The slice of a cookie the auth services consume.
#[derive(Debug, Clone)]
#[cfg_attr(windows, allow(dead_code))]
pub struct CookieRow {
    pub name: String,
    pub value: String,
    /// As the engine reports it: a domain cookie keeps its leading dot.
    pub domain: String,
    pub secure: bool,
}

/// How long one jar read may take. WebKit answers in milliseconds; the
/// deadline only stops a wedged store from wedging a harvest.
#[cfg(not(windows))]
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Every cookie in `window_label`'s jar that applies to `origin`.
///
/// `origin` is a full URL; only its host and scheme matter. A `Secure`
/// cookie is returned only for an `https` origin, matching what the engine
/// itself would send.
#[cfg(not(windows))]
pub async fn cookies_for_origin(
    app: &tauri::AppHandle,
    window_label: &str,
    origin: &str,
) -> anyhow::Result<Vec<CookieRow>> {
    use anyhow::anyhow;

    let url = url::Url::parse(origin)
        .map_err(|e| anyhow!("cookie origin '{origin}' is not a valid URL: {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("cookie origin '{origin}' has no host"))?
        .to_ascii_lowercase();
    let https = url.scheme() == "https";
    let jar = all_cookies(app, window_label).await?;
    Ok(jar
        .into_iter()
        .filter(|c| domain_matches(&c.domain, &host) && (https || !c.secure))
        .collect())
}

/// RFC 6265 domain matching: a host-only cookie matches its host exactly, a
/// domain cookie (with or without the leading dot) matches the host and
/// every subdomain of it.
#[cfg_attr(windows, allow(dead_code))]
fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let d = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    if d.is_empty() {
        return false;
    }
    host == d || host.ends_with(&format!(".{d}"))
}

#[cfg(target_os = "macos")]
#[allow(unused_unsafe)]
async fn all_cookies(
    app: &tauri::AppHandle,
    window_label: &str,
) -> anyhow::Result<Vec<CookieRow>> {
    use anyhow::anyhow;
    use block2::RcBlock;
    use objc2_foundation::{NSArray, NSHTTPCookie};
    use objc2_web_kit::WKWebView;
    use std::ptr::NonNull;
    use tauri::Manager;

    /// Name, value, domain and the Secure flag, straight off WebKit's object.
    unsafe fn row(c: &NSHTTPCookie) -> CookieRow {
        CookieRow {
            name: c.name().to_string(),
            value: c.value().to_string(),
            domain: c.domain().to_string(),
            secure: c.isSecure(),
        }
    }

    let webview = app
        .get_webview_window(window_label)
        .ok_or_else(|| anyhow!("webview window '{window_label}' unavailable"))?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<CookieRow>>();

    // Runs on the main thread and returns immediately. The completion block
    // runs later, on an ordinary run-loop turn, with no event dispatch on the
    // stack, which is the whole point.
    webview
        .with_webview(move |platform_webview| {
            // SAFETY: on macOS `inner()` is the live WKWebView, and this
            // closure runs on the main thread, the only thread WebKit's UI
            // objects may be touched from. The block's array is valid for the
            // duration of the callback.
            unsafe {
                let wk: &WKWebView = &*(platform_webview.inner() as *const WKWebView);
                let store = wk.configuration().websiteDataStore().httpCookieStore();
                let block = RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
                    let rows = cookies
                        .as_ref()
                        .to_vec()
                        .into_iter()
                        .map(|c| row(&c))
                        .collect();
                    let _ = tx.send(rows);
                });
                store.getAllCookies(&block);
            }
        })
        .map_err(|e| anyhow!("with_webview: {e}"))?;

    match tokio::time::timeout(READ_TIMEOUT, rx.recv()).await {
        Ok(Some(rows)) => Ok(rows),
        Ok(None) => Err(anyhow!("the cookie store closed without answering")),
        Err(_) => Err(anyhow!(
            "the cookie store did not answer within {READ_TIMEOUT:?}"
        )),
    }
}

/// Whether a cookie set for `cookie_domain` belongs to `site`: set for the
/// site itself or for any subdomain of it. `domain_matches` with the roles
/// turned round.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn belongs_to(cookie_domain: &str, site: &str) -> bool {
    domain_matches(site, &cookie_domain.trim_start_matches('.').to_ascii_lowercase())
}

/// Delete every cookie in `window_label`'s store that belongs to one of
/// `sites` (see `belongs_to`), and say how many were asked to go.
///
/// For signing a platform out on macOS, where its sign-in pages share one store
/// with the main window and every other account, and clearing all of it would
/// sign the app out of everything (see `services::sign_in_profile`). WebKit
/// deletes asynchronously, so a caller that must know the cookies are gone
/// reads the jar again.
#[cfg(target_os = "macos")]
#[allow(unused_unsafe)]
pub async fn delete_site_cookies(
    app: &tauri::AppHandle,
    window_label: &str,
    sites: &[&str],
) -> anyhow::Result<usize> {
    use anyhow::anyhow;
    use block2::RcBlock;
    use objc2_foundation::{NSArray, NSHTTPCookie};
    use objc2_web_kit::WKWebView;
    use std::ptr::NonNull;
    use tauri::Manager;

    let webview = app
        .get_webview_window(window_label)
        .ok_or_else(|| anyhow!("webview window '{window_label}' unavailable"))?;
    let sites: Vec<String> = sites.iter().map(|s| s.to_ascii_lowercase()).collect();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<usize>();

    // The same shape as `all_cookies` and for the same reason: blocks handed
    // to WebKit from the main thread, answered on a later run-loop turn, so
    // nothing pumps a nested run loop.
    webview
        .with_webview(move |platform_webview| {
            // SAFETY: as in `all_cookies`. The block that deletes holds its
            // own reference to the store.
            unsafe {
                let wk: &WKWebView = &*(platform_webview.inner() as *const WKWebView);
                let store = wk.configuration().websiteDataStore().httpCookieStore();
                let deleter = store.clone();
                let block = RcBlock::new(move |cookies: NonNull<NSArray<NSHTTPCookie>>| {
                    let mut asked = 0;
                    for c in cookies.as_ref().to_vec() {
                        let domain = c.domain().to_string();
                        if sites.iter().any(|s| belongs_to(&domain, s)) {
                            deleter.deleteCookie_completionHandler(&c, None);
                            asked += 1;
                        }
                    }
                    let _ = tx.send(asked);
                });
                store.getAllCookies(&block);
            }
        })
        .map_err(|e| anyhow!("with_webview: {e}"))?;

    match tokio::time::timeout(READ_TIMEOUT, rx.recv()).await {
        Ok(Some(asked)) => Ok(asked),
        Ok(None) => Err(anyhow!("the cookie store closed without answering")),
        Err(_) => Err(anyhow!(
            "the cookie store did not answer within {READ_TIMEOUT:?}"
        )),
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
async fn all_cookies(
    app: &tauri::AppHandle,
    window_label: &str,
) -> anyhow::Result<Vec<CookieRow>> {
    use anyhow::anyhow;
    use tauri::Manager;

    let webview = app
        .get_webview_window(window_label)
        .ok_or_else(|| anyhow!("webview window '{window_label}' unavailable"))?;
    let jar = webview
        .cookies()
        .map_err(|e| anyhow!("cookies() failed: {e}"))?;
    Ok(jar
        .into_iter()
        .map(|c| CookieRow {
            name: c.name().to_string(),
            value: c.value().to_string(),
            domain: c.domain().unwrap_or("").to_string(),
            secure: c.secure().unwrap_or(false),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{belongs_to, domain_matches};

    #[test]
    fn a_sites_cookies_are_its_own_and_its_subdomains_and_nothing_else() {
        assert!(belongs_to(".google.com", "google.com"));
        assert!(belongs_to("accounts.google.com", "google.com"));
        assert!(belongs_to(".id.kick.com", "kick.com"));
        assert!(belongs_to("WWW.TikTok.com", "tiktok.com"), "case is not identity");
        assert!(!belongs_to(".twitch.tv", "kick.com"));
        assert!(!belongs_to("notgoogle.com", "google.com"));
        assert!(!belongs_to("google.com.evil.example", "google.com"));
        assert!(!belongs_to("", "google.com"));
    }

    #[test]
    fn a_domain_cookie_matches_the_host_and_its_subdomains() {
        assert!(domain_matches(".twitch.tv", "twitch.tv"));
        assert!(domain_matches(".twitch.tv", "www.twitch.tv"));
        assert!(domain_matches("youtube.com", "www.youtube.com"));
    }

    #[test]
    fn a_host_only_cookie_matches_only_its_host() {
        assert!(domain_matches("www.twitch.tv", "www.twitch.tv"));
        assert!(!domain_matches("www.twitch.tv", "twitch.tv"));
    }

    #[test]
    fn unrelated_and_lookalike_domains_do_not_match() {
        assert!(!domain_matches(".twitch.tv", "nottwitch.tv"));
        assert!(!domain_matches(".twitch.tv", "twitch.tv.evil.example"));
        assert!(!domain_matches("", "twitch.tv"));
    }
}
