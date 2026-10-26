//! Tells each window whether it is minimized, because the page cannot tell.
//!
//! `CalculateNativeWinOcclusion` is disabled in the WebView2 browser args (it
//! wedged the UI thread in a blocking COM call whenever the window was covered
//! or the shell queried it), so Chromium never learns the window is minimized:
//! `document.visibilityState` stays `visible`, rAF keeps running at the
//! display rate, CSS animations continue and a minimized playing stream costs a
//! full renderer plus the GPU process (2026-09-01 audit, P1-3). Every
//! visibility gate on the page was therefore inert exactly when it mattered.
//!
//! Rust does know: `WebviewWindow::is_minimized` is the Win32 truth. This polls
//! it once a second for every window (a dispatch to the UI thread, microseconds)
//! and emits `window-visibility { hidden }` to that window on change. The page
//! folds it into the same `isWindowHidden()` its gates already read, pauses
//! CSS animations and takes the video element out of the render tree while
//! hidden (audio keeps playing; only compositing stops).
//!
//! The same pass records whether any window is on screen at all, so Rust
//! pollers that only feed the UI can sleep while everything is minimized or
//! in the tray (`all_hidden`), and the ones that skipped a turn catch up the
//! moment something is back on screen.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use log::debug;
use serde::Serialize;
use crate::rt::AppHandle;
use tauri::{Emitter, Manager};

pub const EVENT: &str = "window-visibility";
const PERIOD: Duration = Duration::from_secs(1);

/// True until the first pass proves otherwise, so a poller that starts
/// before it never skips its first run.
static ANY_ON_SCREEN: AtomicBool = AtomicBool::new(true);

/// Every window is minimized or hidden to the tray: nobody can see the UI.
pub fn all_hidden() -> bool {
    !ANY_ON_SCREEN.load(Ordering::Relaxed)
}

/// Record whether anything is on screen. Coming back (the reveal after a
/// boot's first paint, a restore from the tray or a minimize) catches up the
/// reads that sleep while nobody can see them, since their next tick can be a
/// minute away.
fn set_on_screen(on_screen: bool) {
    let was = ANY_ON_SCREEN.swap(on_screen, Ordering::Relaxed);
    if on_screen && !was {
        debug!("[WindowVisibility] back on screen");
        crate::services::home_snapshot::note_ui_on_screen();
        crate::services::channel_state::note_ui_on_screen();
    }
}

/// Some surface is on screen that can show what the UI pollers fetch: a
/// webview window on desktop; on the phones, the app in front (the iPhone app
/// draws natively and has no webview window at all).
pub fn ui_exists(app: &AppHandle) -> bool {
    #[cfg(target_os = "ios")]
    {
        let _ = app;
        !all_hidden()
    }
    #[cfg(target_os = "android")]
    {
        !all_hidden() && !app.webview_windows().is_empty()
    }
    #[cfg(desktop)]
    {
        !app.webview_windows().is_empty()
    }
}

/// The window labelled `label` is still open, for state a command keeps per
/// calling window. The iPhone shell has no webview windows: every command it
/// calls gets a placeholder window labelled `main`, which stands for the shell
/// and lasts as long as the core does.
pub fn window_open(app: &AppHandle, label: &str) -> bool {
    #[cfg(target_os = "ios")]
    {
        let _ = app;
        label == "main"
    }
    #[cfg(not(target_os = "ios"))]
    {
        app.get_webview_window(label).is_some()
    }
}

static SHOWN: once_cell::sync::Lazy<tokio::sync::watch::Sender<bool>> =
    once_cell::sync::Lazy::new(|| tokio::sync::watch::channel(true).0);

/// A phone app moved to the front (`true`) or the background (`false`;
/// picture-in-picture is the front). Neither phone has a window the poller
/// below can read, so each shell says so itself (app_presence::set_foreground).
#[cfg(mobile)]
pub fn set_app_foreground(shown: bool) {
    set_on_screen(shown);
    SHOWN.send_replace(shown);
}

/// The phone app is in the background: pollers that only feed the UI rest.
/// Always false on desktop, whose gates keep reading `all_hidden` (and whose
/// followed poll also feeds live notifications).
pub fn phone_idle() -> bool {
    idle_when(cfg!(mobile), all_hidden())
}

fn idle_when(mobile: bool, hidden: bool) -> bool {
    mobile && hidden
}

/// Returns once the phone app is in front (at once when it is; always at once on desktop).
pub async fn until_shown() {
    if !phone_idle() {
        return;
    }
    let mut rx = SHOWN.subscribe();
    let _ = rx.wait_for(|shown| *shown).await;
}

#[derive(Serialize, Clone)]
struct Visibility {
    hidden: bool,
}

/// A window that only serves another one (the popped-out emote menu).
fn serves_another_window(label: &str) -> bool {
    #[cfg(desktop)]
    {
        label == crate::commands::popout_window::EMOTE_PALETTE_LABEL
    }
    #[cfg(not(desktop))]
    {
        let _ = label;
        false
    }
}

/// Start the poller. Called once from the setup hook.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last: HashMap<String, bool> = HashMap::new();
        loop {
            tokio::time::sleep(PERIOD).await;
            let windows = app.webview_windows();
            last.retain(|label, _| windows.contains_key(label));
            let mut on_screen = false;
            for (label, window) in windows {
                let hidden = window.is_minimized().unwrap_or(false);
                // The emote menu only serves a chat window; on its own it is
                // nobody watching anything, so it never keeps the UI pollers up.
                on_screen |= !hidden && !serves_another_window(&label) && window.is_visible().unwrap_or(true);
                let changed = last.get(&label) != Some(&hidden);
                if changed {
                    last.insert(label.clone(), hidden);
                    debug!("[WindowVisibility] {label}: hidden={hidden}");
                    if let Err(e) = app.emit_to(&label, EVENT, Visibility { hidden }) {
                        debug!("[WindowVisibility] emit to {label} failed: {e}");
                    }
                }
            }
            set_on_screen(on_screen);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_phone_in_the_background_is_idle() {
        assert!(idle_when(true, true));
        assert!(!idle_when(true, false));
        assert!(!idle_when(false, true));
    }
}
