//! Notices a main-window page that stops answering.
//!
//! `ui_hang_watchdog` watches the native UI thread. A different freeze looks
//! the same to a viewer: the window still moves and its buttons in the title
//! bar work, but nothing inside the page answers a click, because the page's
//! JavaScript is stuck or its web process hung. The page beats every two
//! seconds (`src/utils/pageHeartbeat.ts`) and says whether the viewer could see
//! it; when a page that was on screen and focused goes quiet, this logs it,
//! and logs again when it comes back.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Silence from an on-screen page this long is a hang.
const QUIET: Duration = Duration::from_secs(10);
const CHECK: Duration = Duration::from_secs(1);

#[derive(Default)]
struct State {
    /// Last beat, and whether the page was on screen and focused then.
    last: Option<(Instant, bool)>,
    /// When the current hang was declared.
    hung_since: Option<Instant>,
}

static STATE: Mutex<State> = Mutex::new(State { last: None, hung_since: None });

/// A beat from the main window's page.
pub fn beat(focused: bool) {
    let now = Instant::now();
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(since) = s.hung_since.take() {
        let quiet = s.last.map_or(Duration::ZERO, |(at, _)| now.duration_since(at));
        log::warn!(
            "[PageHang] main window page answered again after ~{}s of silence (declared hung {}s ago)",
            quiet.as_secs(),
            now.duration_since(since).as_secs()
        );
    }
    if s.last.is_none() {
        log::info!("[PageHang] main window heartbeat started");
    }
    s.last = Some((now, focused));
}

/// What one check decides, pure so it can be tested.
fn is_hung(last: Option<(Instant, bool)>, now: Instant) -> bool {
    matches!(last, Some((at, true)) if now.duration_since(at) >= QUIET)
}

pub fn start(app: crate::rt::AppHandle) {
    use tauri::Manager;
    if !cfg!(desktop) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("page-watchdog".into())
        .spawn(move || {
            let mut tick = Instant::now();
            loop {
                std::thread::sleep(CHECK);
                let now = Instant::now();
                let slept = now.duration_since(tick);
                tick = now;
                // Closed to the tray: the window, and its page, are gone.
                let open = app.get_webview_window("main").is_some();
                let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
                if !open {
                    s.last = None;
                    s.hung_since = None;
                    continue;
                }
                // This thread itself was paused (the Mac slept, App Nap): the
                // page's silence over that time means nothing. Start over.
                if slept > CHECK * 3 {
                    if let Some((_, focused)) = s.last {
                        s.last = Some((now, focused));
                    }
                    continue;
                }
                if s.hung_since.is_none() && is_hung(s.last, now) {
                    s.hung_since = Some(now);
                    log::error!(
                        "[PageHang] main window page has not answered for {}s while on screen and focused: its JavaScript is stuck or its web process hung (the native UI thread is watched separately by [UiHang])",
                        QUIET.as_secs()
                    );
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_on_screen_page_can_hang() {
        let t = Instant::now();
        let later = t + QUIET + Duration::from_millis(1);
        assert!(is_hung(Some((t, true)), later));
        assert!(!is_hung(Some((t, false)), later), "a page behind other windows is throttled, not hung");
        assert!(!is_hung(Some((t, true)), t + Duration::from_secs(3)));
        assert!(!is_hung(None, later), "no page yet");
    }
}
