//! Signing an account out of a platform's sign-in profile, and starting a
//! sign-in from a clean one.
//!
//! TikTok, YouTube and Kick are signed in to through their own web pages, in the
//! shared login overlay, each inside a persistent WebView2 profile of its own;
//! Rust then reads the session off that profile's cookie jar. Two traps follow,
//! and every platform signed in this way has to handle both.
//!
//! Deleting a profile's folder does not sign it out. The browser behind a
//! profile can outlive its window, holding the session in memory and its files
//! open, so the delete fails quietly and the session survives. Only a webview on
//! the profile reaches that browser, so the clear goes through one, and counts
//! as done only once the session cookie is gone.
//!
//! And a sign-in that polls the jar takes the first session it finds. A profile
//! still holding a signed-out account's session hands that account straight
//! back, on the first poll, before the user has typed anything. So a fresh
//! sign-in mounts its overlay on a blank page, clears the profile through it, and
//! only then opens the login page; and it never takes the session the profile
//! held when it began.
//!
//! On macOS the sign-in pages have no store of their own, so there a clear
//! removes only the platform's own cookies: see `clear`.

use std::collections::HashMap;
use std::path::PathBuf;

/// What a fresh sign-in's overlay is mounted on, so that nothing loads with the
/// old session before the profile is cleared.
const BLANK: &str = "about:blank";

/// One platform's sign-in profile.
pub(crate) struct SignInProfile {
    /// The profile's folder.
    pub dir: PathBuf,
    /// The window label of the platform's sign-in overlay.
    pub overlay_label: &'static str,
    /// The label of the hidden window a sign-out opens on the profile when the
    /// overlay is not open. Named by no capability.
    pub sign_out_label: &'static str,
    /// The site whose cookies carry the session.
    pub origin: &'static str,
    /// Every site the platform's sign-in leaves cookies on, each with its
    /// subdomains. What a clear removes on macOS (see `clear`).
    pub sites: &'static [&'static str],
    /// The session cookie's value in a jar read from `origin`, None when signed
    /// out.
    pub session: fn(&HashMap<String, String>) -> Option<&str>,
    /// The log prefix.
    pub tag: &'static str,
}

/// Whether `current`, the session a sign-in's jar holds now, is one it may take:
/// there is one, and it is not the session the profile held when the sign-in
/// began.
pub(crate) fn is_new_session(current: Option<&str>, stale: Option<&str>) -> bool {
    current.is_some() && current != stale
}

/// Sign the profile out: clear it through a live webview on it, and wait for
/// the session to go, so a sign-in started straight after cannot find it. The
/// sign-in overlay is used if it is open; otherwise a hidden 1x1 window exists
/// for the second this takes.
#[cfg(desktop)]
pub(crate) async fn sign_out(p: &SignInProfile) {
    use tauri::Manager;

    let Some(app) = crate::services::providers::app_handle() else {
        // Before the app is up nothing holds the profile open, so the delete
        // lands. On macOS the folder holds nothing; the next fresh sign-in
        // clears the session instead.
        let _ = std::fs::remove_dir_all(&p.dir);
        return;
    };
    let (win, temporary) = match app.get_webview_window(p.overlay_label) {
        Some(w) => (w, false),
        None => match hidden_window(&app, p) {
            Ok(w) => (w, true),
            Err(e) => {
                // No browser to go through; the folder delete is all that is left.
                log::warn!("[{}] couldn't open the sign-in profile to clear it: {e}", p.tag);
                let _ = std::fs::remove_dir_all(&p.dir);
                return;
            }
        },
    };
    if !clear(&app, &win, p, 40).await {
        log::warn!("[{}] the sign-in profile still held a session after clearing", p.tag);
    }
    if temporary {
        let _ = win.destroy();
    }
}

/// The page a sign-in's overlay opens on: blank for a fresh sign-in, so that
/// nothing loads with the old session before `begin_fresh` clears it. A page
/// that had would already be showing that account, and could set its session
/// again after the clear (Google hands a session to youtube.com through a
/// redirect, for one).
#[cfg(desktop)]
pub(crate) fn opening_url(fresh: bool, login_url: &'static str) -> &'static str {
    if fresh {
        BLANK
    } else {
        login_url
    }
}

/// Start a fresh sign-in in `win`, the platform's sign-in overlay, opened on
/// `opening_url(true, ..)`: clear the profile, wait for its session to go, then
/// open `login_url`.
///
/// Returns the session the profile held when this began, which the sign-in must
/// never take: it belongs to an account that signed out. It is returned even
/// when the clear worked, because on macOS the clear removes cookies only, not
/// the sites' page storage, and nothing may bring that same session back
/// unnoticed.
#[cfg(desktop)]
pub(crate) async fn begin_fresh(
    app: &tauri::AppHandle,
    win: &tauri::WebviewWindow,
    p: &SignInProfile,
    login_url: &str,
) -> Option<String> {
    let before = session_in(app, win.label(), p).await.ok().flatten();
    let stale = if clear(app, win, p, 20).await {
        before
    } else {
        let after = session_in(app, win.label(), p).await.ok().flatten();
        if after.is_some() {
            log::warn!("[{}] the sign-in profile kept a signed-out session; waiting for a new one", p.tag);
        }
        after.or(before)
    };
    if let Ok(url) = login_url.parse() {
        let _ = win.navigate(url);
    }
    stale
}

/// Clear the profile through `win`, then look until its session is gone. False
/// when it was still there, or the jar unreadable, after `attempts` looks.
#[cfg(desktop)]
async fn clear(
    app: &tauri::AppHandle,
    win: &tauri::WebviewWindow,
    p: &SignInProfile,
    attempts: u32,
) -> bool {
    // Windows and Linux give each profile a store of its own (a WebView2 user
    // data folder; a WebKitGTK context per folder), so all of it goes.
    #[cfg(not(target_os = "macos"))]
    if let Err(e) = win.clear_all_browsing_data() {
        log::warn!("[{}] clearing the sign-in profile failed: {e}", p.tag);
    }
    // macOS ignores the folder, and the sign-in pages there stay in WebKit's
    // default store with the main window and every other account
    // (`platform::webview_store` says why). Clearing all of it would sign the
    // app out of everything, so only the platform's own cookies go.
    #[cfg(target_os = "macos")]
    if let Err(e) =
        crate::platform::cookies::delete_site_cookies(app, win.label(), p.sites).await
    {
        log::warn!("[{}] deleting the sign-in cookies failed: {e}", p.tag);
    }
    for _ in 0..attempts {
        match session_in(app, win.label(), p).await {
            Ok(None) => return true,
            _ => tokio::time::sleep(std::time::Duration::from_millis(150)).await,
        }
    }
    false
}

/// The session in the profile's jar right now, read through the webview `label`.
#[cfg(desktop)]
async fn session_in(
    app: &tauri::AppHandle,
    label: &str,
    p: &SignInProfile,
) -> anyhow::Result<Option<String>> {
    let jar =
        crate::services::youtube_auth_service::fetch_cookies_for_origin(app, label, &[], p.origin)
            .await?;
    Ok((p.session)(&jar).map(str::to_string))
}

#[cfg(desktop)]
fn hidden_window(app: &tauri::AppHandle, p: &SignInProfile) -> Result<tauri::WebviewWindow, String> {
    let url: tauri::Url = BLANK.parse().map_err(|e| format!("{e}"))?;
    tauri::WebviewWindowBuilder::new(app, p.sign_out_label, tauri::WebviewUrl::External(url))
        .data_directory(p.dir.clone())
        .visible(false)
        .focused(false)
        .skip_taskbar(true)
        .inner_size(1.0, 1.0)
        .build()
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_from_before_a_sign_in_is_never_taken_for_it() {
        assert!(!is_new_session(Some("old"), Some("old")), "the leftover session");
        assert!(is_new_session(Some("new"), Some("old")), "signing in replaced it");
        assert!(is_new_session(Some("any"), None), "a clean profile takes any session");
        assert!(!is_new_session(None, Some("old")), "signed out is not new");
        assert!(!is_new_session(None, None));
    }
}
