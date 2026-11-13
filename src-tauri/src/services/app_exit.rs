//! Every deliberate way the process ends, and the decision behind the one exit
//! the app takes on its own (the last popout closing while main sits in the
//! tray).
//!
//! Each exit writes one `[Exit] <reason>` line to streamnook.log
//! synchronously (`file_log::record_line_now`) BEFORE the process is told to
//! end. The buffered writer drains every 300 ms, so a line merely logged on
//! the way out was usually lost with the process, and an app that quit itself
//! looked exactly like a crash: a log that stops with no shutdown line.
//!
//! `RunEvent::ExitRequested` and `RunEvent::Exit` are recorded too, naming the
//! last stated reason, or for a runtime-initiated request (`code: None`, the
//! last window went away) the window that was destroyed last.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use once_cell::sync::Lazy;

const TARGET: &str = "streamnook_lib::exit";

static STARTED: Lazy<Instant> = Lazy::new(Instant::now);
/// The main window has been on screen at least once this run: the frontend's
/// first-paint reveal, the reveal failsafe, or a tray or popout show. Before
/// that, a hidden main is the boot gate, not a choice the user made.
static MAIN_REVEALED: AtomicBool = AtomicBool::new(false);
/// The user's close sent main to the tray and nothing has shown it since.
static MAIN_IN_TRAY: AtomicBool = AtomicBool::new(false);
static LAST_REASON: Mutex<Option<String>> = Mutex::new(None);
static LAST_DESTROYED: Mutex<Option<String>> = Mutex::new(None);
/// The process has started ending: set by `exit`, by an `ExitRequested` that
/// is not prevented, and first thing in the `RunEvent::Exit` arm. Background
/// mode's teardown and exit tasks stand down once it is set.
pub static SHUTDOWN: AtomicBool = AtomicBool::new(false);
/// What the last user window's close decided (stay or exit), keyed by the raw
/// label of the window that closed; ended by every show of main.
static CLOSE_DECISION: Mutex<Option<(String, bool)>> = Mutex::new(None);

/// Starts the uptime clock the exit lines carry. Called first thing in `run`.
pub fn note_started() {
    Lazy::force(&STARTED);
}

/// Main is on screen. Also ends any tray stay.
pub fn note_main_shown() {
    MAIN_REVEALED.store(true, Ordering::Release);
    MAIN_IN_TRAY.store(false, Ordering::Release);
    // A show ends any recorded close decision and the "stayed only for
    // keep-alive" flag (every path that puts main on screen comes through here).
    clear_close_decision();
    #[cfg(desktop)]
    crate::services::background_mode::note_shown();
}

/// The user's close hid main to the tray.
pub fn note_main_hidden_to_tray() {
    MAIN_IN_TRAY.store(true, Ordering::Release);
}

pub fn main_revealed() -> bool {
    MAIN_REVEALED.load(Ordering::Acquire)
}

pub fn main_in_tray() -> bool {
    MAIN_IN_TRAY.load(Ordering::Acquire)
}

/// Remembered so a runtime exit request (last window gone) can name it.
pub fn note_window_destroyed(label: &str) {
    if let Ok(mut last) = LAST_DESTROYED.lock() {
        *last = Some(label.to_string());
    }
}

/// Every user-facing window: main, MultiChat and overlay popouts, plugin
/// windows, the 7TV sign-in window, a profile card and the visible Kick
/// sign-in window. A window is a helper only when it is never shown, or is
/// owned by a user window, or is closed by Rust together with its owner (the
/// emote palette); everything else the viewer can see is a user window.
pub fn is_user_window(label: &str) -> bool {
    label == "main"
        || ["multichat-", "overlay-", "plugin-", "seventv-login-", "profile-", "kick-sync-shown-"]
            .iter()
            .any(|prefix| label.starts_with(prefix))
}

/// How many user windows other than main and `closed` are open.
pub fn other_user_windows<'a>(labels: impl IntoIterator<Item = &'a str>, closed: &str) -> usize {
    labels
        .into_iter()
        .filter(|l| is_user_window(l) && *l != "main" && *l != closed)
        .count()
}

/// The process has started ending (an exit, an unprevented exit request, the
/// event loop's exit). Background mode's teardown and exit tasks stand down.
pub fn begin_shutdown() {
    SHUTDOWN.store(true, Ordering::Release);
}

/// The decision the last user window's close made (stay or exit), keyed by the
/// raw label of the window that closed.
pub fn note_close_decision(label: &str, stay: bool) {
    if let Ok(mut decision) = CLOSE_DECISION.lock() {
        *decision = Some((label.to_string(), stay));
    }
}

/// The recorded decision, only when it was made for the window destroyed last
/// (compared against the raw label, never the quoted log form).
pub fn take_close_decision() -> Option<bool> {
    let last = LAST_DESTROYED.lock().ok()?.clone();
    let mut decision = CLOSE_DECISION.lock().ok()?;
    take_if_same(&mut decision, last.as_deref())
}

/// A show ends any recorded decision.
pub fn clear_close_decision() {
    if let Ok(mut decision) = CLOSE_DECISION.lock() {
        *decision = None;
    }
}

fn take_if_same(decision: &mut Option<(String, bool)>, last_destroyed: Option<&str>) -> Option<bool> {
    let same = decision
        .as_ref()
        .is_some_and(|(label, _)| Some(label.as_str()) == last_destroyed);
    if same {
        decision.take().map(|(_, stay)| stay)
    } else {
        None
    }
}

/// The main window as the last popout closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainWindow {
    /// Destroyed: Go Live, or the tray close that frees its memory.
    Gone,
    Visible,
    Hidden,
}

/// Whether closing the last popout should end the app.
///
/// Only when the user put main away and the popouts were what kept the app
/// alive: main hidden to the tray by their close, or destroyed after being on
/// screen. A main that has never been shown is the boot gate (hidden until
/// the frontend paints), and quitting there would end the app before the
/// user ever saw it. Under the "Always" close mode the tray is home, so
/// nothing here ends the app.
pub fn exit_when_last_popout_closes(
    main: MainWindow,
    revealed: bool,
    in_tray: bool,
    always_tray: bool,
) -> bool {
    if always_tray || !revealed {
        return false;
    }
    match main {
        MainWindow::Visible => false,
        MainWindow::Gone => true,
        MainWindow::Hidden => in_tray,
    }
}

/// Write `[Exit] <reason>` to disk now and remember it for the run events.
pub fn record(reason: &str) {
    if let Ok(mut last) = LAST_REASON.lock() {
        *last = Some(reason.to_string());
    }
    write(&format!("{reason} (up {})", uptime()));
}

/// `AppHandle::exit`, recorded first.
pub fn exit(app: &crate::rt::AppHandle, code: i32, reason: &str) {
    begin_shutdown();
    record(&format!("{reason}; exit code {code}"));
    app.exit(code);
}

/// `AppHandle::restart`, recorded first.
pub fn restart(app: &crate::rt::AppHandle, reason: &str) -> ! {
    record(&format!("{reason}; restarting"));
    app.restart()
}

/// `AppHandle::request_restart`, recorded first. Goes through the exit
/// events, so stores flush and the single-instance lock is released before
/// the new process starts.
pub fn request_restart(app: &crate::rt::AppHandle, reason: &str) {
    record(&format!("{reason}; restarting"));
    app.request_restart();
}

/// `std::process::exit`, recorded first. Skips `RunEvent::Exit`, so callers
/// flush their own stores.
pub fn process_exit(code: i32, reason: &str) -> ! {
    record(&format!("{reason}; process exit code {code}"));
    std::process::exit(code)
}

/// `RunEvent::ExitRequested`, after the handler decided whether to keep the
/// process alive.
pub fn on_exit_requested(code: Option<i32>, prevented: bool, close_to_tray: &str) {
    let outcome = if prevented { "prevented, staying in the tray" } else { "proceeding" };
    let line = match code {
        None => format!(
            "exit requested by the runtime: last window closed (last destroyed {}, close_to_tray={close_to_tray}); {outcome}",
            last_destroyed()
        ),
        Some(code) => format!(
            "exit requested with code {} (reason: {}); {outcome}",
            code_name(code),
            last_reason()
        ),
    };
    write(&format!("{line} (up {})", uptime()));
}

/// `RunEvent::Exit`: the event loop is ending; stores flush next.
pub fn on_run_exit() {
    write(&format!(
        "event loop exiting (reason: {}, last destroyed {}); flushing stores (up {})",
        last_reason(),
        last_destroyed(),
        uptime()
    ));
}

/// The `RunEvent::Exit` work finished; the process ends right after.
pub fn on_shutdown_complete(elapsed_ms: u128) {
    write(&format!("shutdown complete in {elapsed_ms} ms"));
}

fn write(line: &str) {
    crate::services::file_log::record_line_now(log::Level::Info, TARGET, &format!("[Exit] {line}"));
}

fn uptime() -> String {
    format!("{:.1} s", STARTED.elapsed().as_secs_f64())
}

fn last_reason() -> String {
    LAST_REASON
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| "none recorded".into())
}

fn last_destroyed() -> String {
    LAST_DESTROYED
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .map(|l| format!("'{l}'"))
        .unwrap_or_else(|| "none".into())
}

fn code_name(code: i32) -> String {
    if code == tauri::RESTART_EXIT_CODE {
        "RESTART".into()
    } else {
        code.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_main_never_shown_keeps_the_app_alive() {
        // The boot gate: main is hidden until first paint, and no close of
        // the user's put it there.
        assert!(!exit_when_last_popout_closes(MainWindow::Hidden, false, false, false));
        assert!(!exit_when_last_popout_closes(MainWindow::Hidden, false, true, false));
        assert!(!exit_when_last_popout_closes(MainWindow::Gone, false, false, false));
    }

    #[test]
    fn main_in_the_tray_ends_with_its_last_popout() {
        assert!(exit_when_last_popout_closes(MainWindow::Hidden, true, true, false));
        assert!(exit_when_last_popout_closes(MainWindow::Gone, true, false, false));
    }

    #[test]
    fn main_hidden_without_the_users_close_stays() {
        // Shown once, hidden by something other than the tray close (a
        // minimize that reads as invisible on some platforms).
        assert!(!exit_when_last_popout_closes(MainWindow::Hidden, true, false, false));
    }

    #[test]
    fn a_visible_main_or_the_always_mode_never_exits() {
        assert!(!exit_when_last_popout_closes(MainWindow::Visible, true, false, false));
        assert!(!exit_when_last_popout_closes(MainWindow::Visible, true, true, false));
        assert!(!exit_when_last_popout_closes(MainWindow::Hidden, true, true, true));
        assert!(!exit_when_last_popout_closes(MainWindow::Gone, true, false, true));
    }

    #[test]
    fn the_restart_code_reads_as_a_word() {
        assert_eq!(code_name(tauri::RESTART_EXIT_CODE), "RESTART");
        assert_eq!(code_name(0), "0");
    }

    // 22m
    #[test]
    fn user_windows_include_every_visible_unowned_window() {
        for label in [
            "main",
            "multichat-1",
            "overlay-2",
            "plugin-drops-farmer-main-view",
            "seventv-login-4",
            "profile-123-1700000000",
            "kick-sync-shown-3",
        ] {
            assert!(is_user_window(label), "{label} is a user window");
        }
        for label in [
            "kick-sync-3",
            "yt-resolve",
            "twitch-integrity-1",
            "tiktok-resolve-1",
            "emote-palette",
            "twitch-login",
        ] {
            assert!(!is_user_window(label), "{label} is a helper");
        }
    }

    #[test]
    fn the_popout_count_leaves_out_main_and_the_closed_window() {
        let open = ["main", "multichat-1", "overlay-2", "yt-resolve", "profile-9-1"];
        assert_eq!(other_user_windows(open, "multichat-1"), 2);
        assert_eq!(other_user_windows(["main", "multichat-1"], "multichat-1"), 0);
        // A helper never counts, so the last user window going is seen.
        assert_eq!(other_user_windows(["yt-resolve", "emote-palette"], "overlay-2"), 0);
        // The main arm closes "main": nothing else is left to exclude.
        assert_eq!(other_user_windows(["plugin-a-b"], "main"), 1);
    }

    // 22j
    #[test]
    fn close_decision_is_taken_only_for_the_same_raw_label() {
        let mut recorded = Some(("multichat-1".to_string(), true));
        assert_eq!(take_if_same(&mut recorded, Some("multichat-1")), Some(true));
        // Taken once.
        assert_eq!(take_if_same(&mut recorded, Some("multichat-1")), None);
        // A helper closed last: the label differs, the decision stays unread.
        let mut recorded = Some(("multichat-1".to_string(), false));
        assert_eq!(take_if_same(&mut recorded, Some("yt-resolve")), None);
        assert!(recorded.is_some());
        // The quoted log form never matches the raw label.
        let mut recorded = Some(("main".to_string(), true));
        assert_eq!(take_if_same(&mut recorded, Some("'main'")), None);
        assert_eq!(take_if_same(&mut recorded, None), None);
        assert_eq!(take_if_same(&mut None, Some("main")), None);
    }
}
