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
//! in the tray (`all_hidden`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use log::debug;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT: &str = "window-visibility";
const PERIOD: Duration = Duration::from_secs(1);

/// True until the first pass proves otherwise, so a poller that starts
/// before it never skips its first run.
static ANY_ON_SCREEN: AtomicBool = AtomicBool::new(true);

/// Every window is minimized or hidden to the tray: nobody can see the UI.
pub fn all_hidden() -> bool {
    !ANY_ON_SCREEN.load(Ordering::Relaxed)
}

#[derive(Serialize, Clone)]
struct Visibility {
    hidden: bool,
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
                on_screen |= !hidden && window.is_visible().unwrap_or(true);
                let changed = last.get(&label) != Some(&hidden);
                if changed {
                    last.insert(label.clone(), hidden);
                    debug!("[WindowVisibility] {label}: hidden={hidden}");
                    if let Err(e) = app.emit_to(&label, EVENT, Visibility { hidden }) {
                        debug!("[WindowVisibility] emit to {label} failed: {e}");
                    }
                }
            }
            ANY_ON_SCREEN.store(on_screen, Ordering::Relaxed);
        }
    });
}
