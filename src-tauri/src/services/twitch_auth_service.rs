// Centralized owner of the Twitch web session token.
//
// Reads the twitch.tv `auth-token` (and `unique_id` device id) cookie through
// WebView2's `ICoreWebView2CookieManager`, the only supported surface that
// doesn't fight Chromium for the locked SQLite file or care about encryption
// schema changes.
//
// Account-tied source. The signed-in web session lives in the ACTIVE account's
// per-account WebView2 profile (`twitch_web_profiles/<id>`), the same profile
// the login and subscribe windows bind to. The app's main window uses a
// separate default profile, so reading the token there would return whichever
// account first owned the default store — not the account you switched to. To
// stay tied to the active account we read its profile directly through a
// short-lived hidden webview that is destroyed the instant the cookie is in
// hand, then cache the value. A persistent second webview would mean a whole
// extra WebView2 host process for a different profile; the transient one only
// spins up on a cold cache (app start, account switch, logout), so steady-state
// watching keeps no extra window alive.
//
// Fallback. A login made before per-account profiles existed has its session in
// the default store, so when the active profile has no session we read the main
// window's store instead. That keeps single-account and legacy users working.
//
// Architecture:
//   - One instance per app, owned by `AppState`.
//   - Callers (auth_proxy, resolver, quality probe) request the token via
//     `get_token()`; they never touch cookies directly.
//   - A single harvest fills both the `auth-token` and `unique_id` cache, so a
//     stream start that needs the device id doesn't open a second window.
//   - Account transitions call `on_account_changed()` / `on_logged_out()` so
//     the next read re-harvests for the now-active account (or returns
//     not-logged-in) instead of serving the previous account's cached token.
//
// Android reads the same two cookies out of the app-global `CookieManager`,
// which is where the native Kotlin login overlay writes them. There is no
// hidden window and no per-account profile involved, so that path is a single
// plugin call (see the bottom of this file). Desktop platforms other than
// Windows have no cookie source and stay stubbed out.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::AppHandle;
use tokio::sync::{oneshot, Mutex, RwLock};

/// How long a SUCCESSFUL harvest (one that found an auth-token) stays hot. Web
/// tokens last for weeks, and account transitions invalidate the cache
/// explicitly, so the only thing this TTL bounds is noticing a logout made
/// directly inside an embedded twitch.tv window (rare). A long TTL keeps the
/// transient harvest window from reopening during a normal viewing session.
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// How long an EMPTY harvest (no auth-token found) stays cached. Kept short on
/// purpose: a login via a persisted token fires no account-change event to
/// invalidate the cache, so a long negative TTL would pin the app to "logged
/// out" for the web session for the better part of an hour after the user is
/// actually signed in. A few seconds is enough to absorb a burst of callers
/// (resolver, chat widget retries) without opening a harvest window per call,
/// while still re-checking almost immediately once a session appears.
const EMPTY_CACHE_TTL: Duration = Duration::from_secs(5);

/// Label of the hidden, short-lived webview used to read the active account's
/// profile cookies. Built bound to that profile and destroyed right after.
#[cfg(windows)]
const SESSION_WINDOW_LABEL: &str = "twitch-session";

/// Cookies a single harvest collects, so one window read serves both the token
/// and the device-id callers.
#[cfg(any(windows, target_os = "android"))]
const COOKIE_NAMES: &[&str] = &["auth-token", "unique_id"];

#[derive(Debug, Clone)]
pub enum AuthError {
    /// The user isn't logged in to twitch.tv in the active profile, or the
    /// auth-token cookie is empty.
    NotLoggedIn,
    /// WebView2 wasn't available to query (main window not yet created, or
    /// non-Windows platform).
    WebViewUnavailable,
    /// COM / WebView2 call failure with a diagnostic string. These are always
    /// loggable, never user-facing.
    Internal(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::NotLoggedIn => f.write_str("not logged in to twitch.tv in WebView"),
            AuthError::WebViewUnavailable => f.write_str("WebView2 not available"),
            AuthError::Internal(s) => write!(f, "internal: {}", s),
        }
    }
}

impl std::error::Error for AuthError {}

#[derive(Clone, Default)]
struct CachedSession {
    auth_token: Option<String>,
    device_id: Option<String>,
}

struct CacheEntry {
    session: CachedSession,
    cached_at: Instant,
}

struct Inner {
    app: AppHandle,
    cache: RwLock<Option<CacheEntry>>,
    /// Single-flight gate: at most one in-flight harvest at a time. Callers that
    /// race past a cache miss queue here and the second one finds the freshly
    /// cached value once the lock releases.
    fetch_gate: Mutex<()>,
    /// Set on logout so reads return not-logged-in without falling back to a
    /// stale default-store cookie. Cleared on the next login / account switch.
    logged_out: AtomicBool,
}

#[derive(Clone)]
pub struct TwitchAuthService {
    inner: Arc<Inner>,
}

impl TwitchAuthService {
    pub fn new(app: AppHandle) -> Self {
        Self {
            inner: Arc::new(Inner {
                app,
                cache: RwLock::new(None),
                fetch_gate: Mutex::new(()),
                logged_out: AtomicBool::new(false),
            }),
        }
    }

    /// Returns the active account's twitch.tv `auth-token` cookie value. Cached
    /// for `CACHE_TTL`; on miss harvests it from the active profile.
    pub async fn get_token(&self) -> Result<String, AuthError> {
        if self.inner.logged_out.load(Ordering::SeqCst) {
            log::warn!(
                "[Auth] get_token: logged_out flag is SET — returning NotLoggedIn without harvesting (login never cleared it)"
            );
            return Err(AuthError::NotLoggedIn);
        }
        match self.session().await.auth_token {
            Some(t) => Ok(t),
            None => {
                log::warn!("[Auth] get_token: harvest returned NO auth-token cookie");
                Err(AuthError::NotLoggedIn)
            }
        }
    }

    /// Returns the twitch.tv `unique_id` (Device-ID) cookie if present. The real
    /// Twitch web player sends this on the playback-token request; Twitch uses it
    /// together with the OAuth token to reflect account ad entitlements (Turbo,
    /// per-channel sub). Served from the same cached harvest as the token, so it
    /// never opens its own window.
    pub async fn get_device_id(&self) -> Option<String> {
        if self.inner.logged_out.load(Ordering::SeqCst) {
            return None;
        }
        self.session().await.device_id
    }

    /// Drop the cached session. Call this when a Twitch API returns 401/403 with
    /// this token so the next read re-harvests.
    pub async fn invalidate(&self) {
        *self.inner.cache.write().await = None;
    }

    /// A login or account switch happened: clear the logged-out flag and drop
    /// the cache so the next read harvests the now-active account's session.
    pub async fn on_account_changed(&self) {
        self.inner.logged_out.store(false, Ordering::SeqCst);
        self.invalidate().await;
    }

    /// A full logout happened: reads return not-logged-in (no fallback to the
    /// default store's lingering cookie) until the next login.
    pub async fn on_logged_out(&self) {
        self.inner.logged_out.store(true, Ordering::SeqCst);
        self.invalidate().await;
    }

    /// Cached session if fresh, otherwise a single harvest behind the gate.
    async fn session(&self) -> CachedSession {
        if let Some(s) = self.cached().await {
            log::warn!(
                "[Auth] session: serving CACHED session (auth-token present = {}); not re-harvesting",
                s.auth_token.is_some()
            );
            return s;
        }
        let _gate = self.inner.fetch_gate.lock().await;
        if let Some(s) = self.cached().await {
            log::warn!(
                "[Auth] session: serving CACHED session after gate (auth-token present = {})",
                s.auth_token.is_some()
            );
            return s;
        }
        let cookies = harvest(&self.inner.app).await;
        let session = CachedSession {
            auth_token: cookies.get("auth-token").cloned(),
            device_id: cookies.get("unique_id").cloned(),
        };
        *self.inner.cache.write().await = Some(CacheEntry {
            session: session.clone(),
            cached_at: Instant::now(),
        });
        session
    }

    async fn cached(&self) -> Option<CachedSession> {
        let guard = self.inner.cache.read().await;
        guard
            .as_ref()
            .filter(|c| {
                // A found token stays hot for the long TTL; an empty result
                // expires fast so a subsequent login is picked up promptly.
                let ttl = if c.session.auth_token.is_some() {
                    CACHE_TTL
                } else {
                    EMPTY_CACHE_TTL
                };
                c.cached_at.elapsed() < ttl
            })
            .map(|c| c.session.clone())
    }
}

// ---------------------------------------------------------------------------
// WebView2 bridge — Windows-only.
//
// `with_webview` gives us the live `ICoreWebView2Controller` on the UI thread.
// We cast to `ICoreWebView2_2` (for `CookieManager`), call `GetCookies`, and the
// handler fires back on the UI thread when WebView2 has the result, signalling a
// `oneshot::Sender` the async caller awaits. COM objects are never marshalled
// across threads — every COM call happens on the UI thread.
// ---------------------------------------------------------------------------

/// Harvest the active account's twitch.tv cookies. Reads the active profile via
/// a short-lived hidden webview; on a profile with no session, falls back to the
/// main window's default store (a pre-multi-account login lives there). Returns
/// whatever was found (possibly empty); callers treat empty as not-logged-in.
#[cfg(windows)]
async fn harvest(app: &AppHandle) -> HashMap<String, String> {
    use crate::services::account_store::AccountStore;

    let mut found: HashMap<String, String> = HashMap::new();

    let primary = AccountStore::primary();
    log::debug!(
        "[Auth] harvest start: primary account = {:?}",
        primary.as_ref().map(|p| p.user_id.clone())
    );

    // The active profile is where the login and subscribe windows write the web
    // session: the account's own profile when one is registered, otherwise the
    // `_pending` staging profile that a device-code login lands in before any
    // AccountStore primary exists. Read it in BOTH cases — a legacy device-code
    // login registers no primary, yet its twitch.tv session still lives in
    // `_pending`, so gating this on `primary.is_some()` (as before) missed it
    // entirely and fell through to the default store, which never holds that
    // login's session.
    let _ = &primary; // retained for the log line above; no longer gates the read
    match harvest_from_active_profile(app).await {
        Some(map) => {
            log::warn!(
                "[Auth] active-profile harvest: {} cookie(s), auth-token present = {}",
                map.len(),
                map.contains_key("auth-token")
            );
            found = map;
        }
        None => log::warn!(
            "[Auth] active-profile harvest returned None (session window could not be built)"
        ),
    }

    // Fallback: a login made before per-account profiles existed wrote its
    // session to the main window's default store.
    if !found.contains_key("auth-token") {
        log::warn!(
            "[Auth] no auth-token from active profile; trying the main window's default store"
        );
        match fetch_cookies_from_window(app, "main", COOKIE_NAMES).await {
            Ok(map) => {
                log::warn!(
                    "[Auth] main-store harvest: {} cookie(s), auth-token present = {}",
                    map.len(),
                    map.contains_key("auth-token")
                );
                for (k, v) in map {
                    found.entry(k).or_insert(v);
                }
            }
            Err(e) => log::warn!("[Auth] main-store harvest failed: {}", e),
        }
    }

    log::warn!(
        "[Auth] harvest done: auth-token present = {}",
        found.contains_key("auth-token")
    );
    found
}

/// Build a hidden webview bound to the active account's profile, read its
/// cookies, and destroy it. Retries the read while the freshly-built webview's
/// controller initializes. `None` if the window couldn't be built at all.
#[cfg(windows)]
async fn harvest_from_active_profile(app: &AppHandle) -> Option<HashMap<String, String>> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

    // Clear any leftover session window from a prior interrupted harvest so the
    // label is free and bound to the CURRENT active profile.
    if let Some(existing) = app.get_webview_window(SESSION_WINDOW_LABEL) {
        let _ = existing.destroy();
    }

    let profile = crate::services::twitch_service::active_twitch_web_profile_dir().ok()?;
    let url = WebviewUrl::External(tauri::Url::parse("about:blank").ok()?);

    let build = WebviewWindowBuilder::new(app, SESSION_WINDOW_LABEL, url)
        .title("")
        .inner_size(1.0, 1.0)
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .data_directory(profile)
        .build();
    if let Err(e) = build {
        log::warn!("[Auth] session window build failed: {}", e);
        return None;
    }

    // The webview controller isn't ready the instant the window builds; retry
    // the read briefly. Break as soon as the auth-token shows up.
    let mut result: HashMap<String, String> = HashMap::new();
    for attempt in 0..12 {
        match fetch_cookies_from_window(app, SESSION_WINDOW_LABEL, COOKIE_NAMES).await {
            Ok(map) => {
                let has_token = map.contains_key("auth-token");
                result = map;
                // A non-empty token read is final. An empty read early on may
                // just be a controller that isn't serving the cookie store yet,
                // so keep trying a few times before accepting "no session".
                if has_token || attempt >= 3 {
                    break;
                }
            }
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    if let Some(window) = app.get_webview_window(SESSION_WINDOW_LABEL) {
        let _ = window.destroy();
    }
    Some(result)
}

#[cfg(windows)]
async fn fetch_cookies_from_window(
    app: &AppHandle,
    window_label: &str,
    names: &[&str],
) -> Result<HashMap<String, String>, AuthError> {
    use tauri::Manager;

    let webview = app
        .get_webview_window(window_label)
        .ok_or(AuthError::WebViewUnavailable)?;

    let (tx, rx) = oneshot::channel::<Result<HashMap<String, String>, AuthError>>();
    // The handler closure runs on the UI thread, so it doesn't need Send, but
    // `with_webview`'s closure does — wrap in a Send-safe slot.
    let tx_slot: Arc<std::sync::Mutex<Option<oneshot::Sender<_>>>> =
        Arc::new(std::sync::Mutex::new(Some(tx)));

    let tx_for_closure = tx_slot.clone();
    let targets: Vec<String> = names.iter().map(|s| s.to_string()).collect();
    let with_webview_result = webview.with_webview(move |platform_webview| {
        let setup_result =
            unsafe { request_cookies(platform_webview, tx_for_closure.clone(), targets.clone()) };
        if let Err(e) = setup_result {
            if let Some(sender) = tx_for_closure.lock().unwrap().take() {
                let _ = sender.send(Err(AuthError::Internal(format!(
                    "WebView2 GetCookies setup failed: {}",
                    e
                ))));
            }
        }
    });

    if let Err(e) = with_webview_result {
        // `with_webview` failed to dispatch — sender was never consumed, so
        // synthesize the error directly.
        return Err(AuthError::Internal(format!("with_webview: {}", e)));
    }

    rx.await
        .map_err(|_| AuthError::Internal("WebView2 cookie callback was dropped".into()))?
}

#[cfg(windows)]
unsafe fn request_cookies(
    platform_webview: tauri::webview::PlatformWebview,
    tx_slot: Arc<
        std::sync::Mutex<Option<oneshot::Sender<Result<HashMap<String, String>, AuthError>>>>,
    >,
    targets: Vec<String>,
) -> windows::core::Result<()> {
    use webview2_com::GetCookiesCompletedHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_2;
    use windows::core::{Interface, HSTRING};

    let controller = platform_webview.controller();
    let core = controller.CoreWebView2()?;
    let core2: ICoreWebView2_2 = core.cast()?;
    let manager = core2.CookieManager()?;

    let uri = HSTRING::from("https://twitch.tv");

    let handler = GetCookiesCompletedHandler::create(Box::new(move |error_code, cookie_list| {
        let result = extract_cookies(error_code, cookie_list, &targets);
        if let Some(sender) = tx_slot.lock().unwrap().take() {
            let _ = sender.send(result);
        }
        Ok(())
    }));

    manager.GetCookies(&uri, &handler)?;
    Ok(())
}

#[cfg(windows)]
fn extract_cookies(
    completion: windows::core::Result<()>,
    cookie_list: Option<webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2CookieList>,
    targets: &[String],
) -> Result<HashMap<String, String>, AuthError> {
    use webview2_com::take_pwstr;
    use windows::core::PWSTR;

    completion.map_err(|e| AuthError::Internal(format!("GetCookies: {}", e)))?;

    let list = cookie_list
        .ok_or_else(|| AuthError::Internal("WebView2 returned null cookie list".into()))?;

    let mut count: u32 = 0;
    unsafe { list.Count(&mut count as *mut u32) }
        .map_err(|e| AuthError::Internal(format!("CookieList::Count: {}", e)))?;

    let mut found: HashMap<String, String> = HashMap::new();
    for i in 0..count {
        let cookie = unsafe { list.GetValueAtIndex(i) }
            .map_err(|e| AuthError::Internal(format!("CookieList[{}]: {}", i, e)))?;

        let mut name_ptr = PWSTR::null();
        unsafe { cookie.Name(&mut name_ptr as *mut PWSTR) }
            .map_err(|e| AuthError::Internal(format!("cookie.Name: {}", e)))?;
        let name = take_pwstr(name_ptr);

        if targets.iter().any(|t| t == &name) {
            let mut value_ptr = PWSTR::null();
            unsafe { cookie.Value(&mut value_ptr as *mut PWSTR) }
                .map_err(|e| AuthError::Internal(format!("cookie.Value: {}", e)))?;
            let value = take_pwstr(value_ptr);
            if !value.is_empty() {
                found.insert(name, value);
            }
        }
    }
    Ok(found)
}

// ---------------------------------------------------------------------------
// Android bridge.
//
// There is no second webview to build here (Tauri 2's child-webview API is
// desktop-only), and no per-account profile either: the login overlay is a
// native Kotlin WebView writing into the app-global `CookieManager`, which is
// the same jar the harvest reads back. So the whole hidden-window dance above
// collapses to one plugin call.
// ---------------------------------------------------------------------------

/// Read the twitch.tv cookies the Kotlin login overlay left in the app-global
/// `CookieManager`, and keep the two the rest of the app asks for.
#[cfg(target_os = "android")]
async fn harvest(app: &AppHandle) -> HashMap<String, String> {
    use tauri::Manager;

    let mut found = HashMap::new();
    let Some(state) = app.try_state::<crate::twitch_login_plugin::TwitchLoginState<tauri::Wry>>()
    else {
        log::warn!("[Auth] android harvest: twitch-login plugin is not registered");
        return found;
    };

    // `getCookies` resolves on the calling thread (unlike the overlay's
    // open/close commands, which hop to the UI thread), so this cannot park on a
    // suspended activity.
    let header = match state
        .0
        .run_mobile_plugin::<CookiesResp>("getCookies", ())
    {
        Ok(r) => r.cookies,
        Err(e) => {
            log::warn!("[Auth] android harvest: getCookies failed: {}", e);
            return found;
        }
    };

    for pair in header.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        // Split on the FIRST `=` only: a token value can contain more of them.
        if COOKIE_NAMES.contains(&name) && !value.is_empty() {
            found.insert(name.to_string(), value.to_string());
        }
    }

    log::info!(
        "[Auth] android harvest: {} twitch.tv cookie(s) in the jar, auth-token present = {}",
        header.split(';').filter(|p| !p.trim().is_empty()).count(),
        found.contains_key("auth-token")
    );
    found
}

#[cfg(target_os = "android")]
#[derive(serde::Deserialize)]
struct CookiesResp {
    cookies: String,
}

/// Desktop platforms other than Windows do not ship, and have no cookie source.
#[cfg(all(not(windows), not(target_os = "android")))]
async fn harvest(_app: &AppHandle) -> HashMap<String, String> {
    HashMap::new()
}
