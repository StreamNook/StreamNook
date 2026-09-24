// Android in-app Twitch login. Tauri 2 cannot open a second WebView on Android
// (the child-webview API is desktop-only), so login is presented by a native
// Kotlin WebView overlay registered as an Android plugin. The Kotlin class
// `app.streamnook.TwitchLoginPlugin` exposes openLogin/closeLogin/getCookies.
//
// Plugin commands (`plugin:twitch-login|...`) always require an ACL grant, which
// app-local plugins don't get for free. So instead of invoking the Kotlin plugin
// directly from JS, we forward through regular app commands (always allowed from
// the local origin) that call the stored PluginHandle.
#![cfg(target_os = "android")]

use serde::{Deserialize, Serialize};
use tauri::plugin::{PluginHandle, TauriPlugin};
use tauri::{AppHandle, Manager, Runtime};

pub struct TwitchLoginState<R: Runtime>(pub PluginHandle<R>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OpenLoginArgs {
    url: String,
    /// localStorage key the overlay should watch for. See the Kotlin side.
    #[serde(skip_serializing_if = "Option::is_none")]
    watch_storage_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    /// Run the overlay invisibly: no bar, nothing on screen, a short deadline.
    /// For silent session re-mints (7TV) where the page completes on its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    hidden: Option<bool>,
}

#[derive(Deserialize)]
struct CookiesResp {
    cookies: String,
}

#[derive(Deserialize)]
struct DropsTokenResp {
    token: String,
}

#[derive(Serialize)]
struct CookieUrlArgs<'a> {
    url: &'a str,
}

#[derive(Serialize)]
struct ExpireCookiesArgs<'a> {
    urls: &'a [&'a str],
}

#[derive(Deserialize)]
struct OpenResp {
    open: bool,
}

#[derive(Deserialize)]
struct KickRedirectResp {
    url: String,
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("twitch-login")
        .setup(|app, api| {
            let handle = api.register_android_plugin("app.streamnook", "TwitchLoginPlugin")?;
            app.manage(TwitchLoginState(handle));
            Ok(())
        })
        .build()
}

#[tauri::command]
pub async fn open_mobile_login<R: Runtime>(
    app: AppHandle<R>,
    url: String,
    watch_storage_key: Option<String>,
    title: Option<String>,
    hidden: Option<bool>,
) -> Result<(), String> {
    let state = app.state::<TwitchLoginState<R>>();
    state
        .0
        .run_mobile_plugin::<serde_json::Value>(
            "openLogin",
            OpenLoginArgs {
                url,
                watch_storage_key,
                title,
                hidden,
            },
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn close_mobile_login<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let state = app.state::<TwitchLoginState<R>>();
    state
        .0
        .run_mobile_plugin::<serde_json::Value>("closeLogin", ())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Finish a drops sign-in the overlay reported (`sn:drops-redirect`): collect
/// the credential the Kotlin plugin captured off the redirect and store it the
/// way the device flow used to. The token comes plugin -> Rust -> disk and is
/// never handed to the page.
#[tauri::command]
pub async fn finish_mobile_drops_login<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let token = {
        let state = app.state::<TwitchLoginState<R>>();
        state
            .0
            .run_mobile_plugin::<DropsTokenResp>("takeDropsToken", ())
            .map(|r| r.token)
            .map_err(|e| e.to_string())?
    };
    if token.is_empty() {
        return Err("Twitch did not hand back a drops credential".to_string());
    }
    crate::services::drops_auth_service::DropsAuthService::store_access_token(token)
        .await
        .map_err(|e| e.to_string())
}

/// Sign the embedded browser out: drop every cookie and web-storage entry the
/// login overlay's WebView holds. Called on sign-out so the next sign-in starts
/// from Twitch's login page rather than the previous account's session.
#[tauri::command]
pub async fn clear_mobile_login_cookies<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let state = app.state::<TwitchLoginState<R>>();
    state
        .0
        .run_mobile_plugin::<serde_json::Value>("clearCookies", ())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_mobile_login_cookies<R: Runtime>(app: AppHandle<R>) -> Result<String, String> {
    let state = app.state::<TwitchLoginState<R>>();
    state
        .0
        .run_mobile_plugin::<CookiesResp>("getCookies", ())
        .map(|r| r.cookies)
        .map_err(|e| e.to_string())
}

// ── Driven from Rust ─────────────────────────────────────────────────────────
// The Kick sign-in runs the overlay itself (open, poll, collect, close) rather
// than bouncing through the page, so these are plain functions, not commands,
// and need no ACL grant.

fn handle(app: &AppHandle) -> tauri::State<'_, TwitchLoginState<tauri::Wry>> {
    app.state::<TwitchLoginState<tauri::Wry>>()
}

/// Show (or re-navigate) the login overlay.
pub fn open_overlay(app: &AppHandle, url: &str, title: &str) -> Result<(), String> {
    handle(app)
        .0
        .run_mobile_plugin::<serde_json::Value>(
            "openLogin",
            OpenLoginArgs {
                url: url.to_string(),
                watch_storage_key: None,
                title: Some(title.to_string()),
                hidden: None,
            },
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn overlay_is_open(app: &AppHandle) -> bool {
    handle(app)
        .0
        .run_mobile_plugin::<OpenResp>("isOpen", ())
        .map(|r| r.open)
        .unwrap_or(false)
}

pub fn close_overlay(app: &AppHandle) {
    let _ = handle(app)
        .0
        .run_mobile_plugin::<serde_json::Value>("closeLogin", ());
}

/// The overlay's cookie header for `url` (`a=b; c=d`), HttpOnly included.
pub fn cookies_for(app: &AppHandle, url: &str) -> String {
    handle(app)
        .0
        .run_mobile_plugin::<CookiesResp>("getCookiesFor", CookieUrlArgs { url })
        .map(|r| r.cookies)
        .unwrap_or_default()
}

/// The Kick consent redirect the overlay caught, once; empty when none.
pub fn take_kick_redirect(app: &AppHandle) -> String {
    handle(app)
        .0
        .run_mobile_plugin::<KickRedirectResp>("takeKickRedirect", ())
        .map(|r| r.url)
        .unwrap_or_default()
}

/// Expire every cookie on these origins, leaving other sites signed in.
pub fn expire_cookies(app: &AppHandle, urls: &[&str]) {
    let _ = handle(app)
        .0
        .run_mobile_plugin::<serde_json::Value>("expireCookies", ExpireCookiesArgs { urls });
}

// ── File handoff ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct FileHandoffArgs<'a> {
    path: &'a str,
    name: &'a str,
    mime: &'a str,
}

#[derive(Deserialize)]
struct StatusResp {
    status: String,
}

/// Copy a file into the public Downloads collection as `name`. `Ok(false)`
/// when the OS is too old to do that without a storage permission (below
/// Android 10), so the caller can fall back to sharing.
pub fn save_to_downloads(
    app: &AppHandle,
    path: &std::path::Path,
    name: &str,
    mime: &str,
) -> Result<bool, String> {
    let path = path.to_string_lossy();
    handle(app)
        .0
        .run_mobile_plugin::<StatusResp>(
            "saveToDownloads",
            FileHandoffArgs { path: &path, name, mime },
        )
        .map(|r| r.status == "saved")
        .map_err(|e| e.to_string())
}

/// Open the share sheet with this file.
pub fn share_file(app: &AppHandle, path: &std::path::Path, name: &str, mime: &str) -> Result<(), String> {
    let path = path.to_string_lossy();
    handle(app)
        .0
        .run_mobile_plugin::<serde_json::Value>("shareFile", FileHandoffArgs { path: &path, name, mime })
        .map(|_| ())
        .map_err(|e| e.to_string())
}
