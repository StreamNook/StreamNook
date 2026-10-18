//! The iPhone's in-app web sheet, driven from Rust: the iOS twin of the
//! Rust-facing helpers of Android's login overlay (`twitch_login_plugin`), with
//! the same names and shapes so the Kick sign-in and import and Subscribe run
//! one body on both phones.
//!
//! Rust asks the shell through host requests:
//!
//! - `open_sheet` `{url, title, stop_at, cookie_origins}`: show (or
//!   re-navigate) the sheet at `url`. A navigation whose URL starts with any
//!   `stop_at` prefix is not loaded; the shell hands it to `sheet_redirected`
//!   and keeps the sheet up. While the sheet is up the shell pushes the cookies
//!   of every `cookie_origins` site through `set_web_cookies` as they change.
//! - `close_sheet` `{}`: take the sheet down.
//! - `clear_cookies` `{origins}`: delete every cookie of those sites from the
//!   sheet's store, leaving other sites signed in.
//!
//! The shell answers with two commands: `sheet_redirected {url}` and
//! `sheet_closed` (the person dismissed it).

use std::sync::Mutex;

use crate::platform::cookies::domain_matches;
use crate::platform::shell_cookies;
use crate::rt::AppHandle;

/// Kick's registered OAuth redirect. Like the Android overlay, the sheet
/// always stops there, whatever page it was opened on.
const KICK_REDIRECT: &str = "http://localhost:3000/callback";

/// Where the Twitch Drops grant lands, with its credential on the fragment
/// (`DropsAuthService::authorize_url`). The sheet stops there too, so the
/// token is taken before any page could show it.
const DROPS_REDIRECT: &str = "https://www.twitch.tv/#";

/// The sites whose cookies the core reads while a sheet is up.
const COOKIE_ORIGINS: &[&str] = &["https://kick.com", "https://id.kick.com", "https://www.twitch.tv"];

#[derive(Default)]
struct Sheet {
    open: bool,
    kick_redirect: Option<String>,
}

static SHEET: Mutex<Sheet> = Mutex::new(Sheet { open: false, kick_redirect: None });

fn sheet() -> std::sync::MutexGuard<'static, Sheet> {
    SHEET.lock().unwrap_or_else(|p| p.into_inner())
}

/// Show (or re-navigate) the sheet.
pub fn open_overlay(_app: &AppHandle, url: &str, title: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("not a link: {e}"))?;
    if parsed.scheme() != "https" {
        return Err(format!("the sheet opens https pages only, not {}", parsed.scheme()));
    }
    {
        let mut s = sheet();
        s.open = true;
        s.kick_redirect = None;
    }
    tauri::host_request(
        "open_sheet",
        &serde_json::json!({
            "url": url,
            "title": title,
            "stop_at": [KICK_REDIRECT, DROPS_REDIRECT],
            "cookie_origins": COOKIE_ORIGINS,
        }),
    );
    Ok(())
}

pub fn overlay_is_open(_app: &AppHandle) -> bool {
    sheet().open
}

pub fn close_overlay(_app: &AppHandle) {
    sheet().open = false;
    tauri::host_request("close_sheet", &serde_json::json!({}));
}

/// The sheet's cookie header for `url` (`a=b; c=d`), HttpOnly included: the
/// cookies the shell last pushed that apply to that host.
pub fn cookies_for(_app: &AppHandle, url: &str) -> String {
    let Ok(parsed) = url::Url::parse(url) else {
        return String::new();
    };
    let Some(host) = parsed.host_str().map(|h| h.to_ascii_lowercase()) else {
        return String::new();
    };
    let https = parsed.scheme() == "https";
    shell_cookies::rows()
        .into_iter()
        .filter(|c| domain_matches(&c.domain, &host) && (https || !c.secure))
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// The Kick consent redirect the sheet stopped at, once; empty when none.
pub fn take_kick_redirect(_app: &AppHandle) -> String {
    sheet().kick_redirect.take().unwrap_or_default()
}

/// Expire every cookie on these origins, leaving other sites signed in.
pub fn expire_cookies(_app: &AppHandle, urls: &[&str]) {
    shell_cookies::forget(urls);
    tauri::host_request("clear_cookies", &serde_json::json!({ "origins": urls }));
}

/// The Twitch Drops sign-in in the sheet: the Drops client's authorize page,
/// which the twitch.tv session from sign-in already answers, so the person
/// only taps Authorize. The grant comes back through `sheet_redirected`, which
/// stores it and announces `drops-login-complete` or `drops-login-error`, as
/// the desktop overlay and Android do.
#[tauri::command]
pub fn open_drops_sheet(app: AppHandle) -> Result<(), String> {
    let url = crate::services::drops_auth_service::DropsAuthService::authorize_url();
    open_overlay(&app, &url, "Connect drops")
}

/// The sheet stopped at a `stop_at` URL.
#[tauri::command]
pub fn sheet_redirected(app: AppHandle, url: String) {
    if url.starts_with(KICK_REDIRECT) {
        sheet().kick_redirect = Some(url);
    } else if url.starts_with(DROPS_REDIRECT) {
        let token = crate::services::drops_auth_service::DropsAuthService::access_token_from_redirect(&url);
        close_overlay(&app);
        tauri::async_runtime::spawn(async move {
            use tauri::Emitter;
            let outcome = match token {
                Some(token) => {
                    crate::services::drops_auth_service::DropsAuthService::store_access_token(token)
                        .await
                        .map_err(|e| e.to_string())
                }
                None => Err("Twitch did not hand back a drops credential".to_string()),
            };
            let _ = match outcome {
                Ok(()) => app.emit("drops-login-complete", ()),
                Err(e) => app.emit("drops-login-error", e),
            };
        });
    } else {
        log::warn!("[iOS] the sheet stopped at a URL nothing waits for");
    }
}

/// The person dismissed the sheet.
#[tauri::command]
pub fn sheet_closed() {
    sheet().open = false;
}
