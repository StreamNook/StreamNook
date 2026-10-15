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

/// Starts the uptime clock the exit lines carry. Called first thing in `run`.
pub fn note_started() {
    Lazy::force(&STARTED);
}

/// Main is on screen. Also ends any tray stay.
pub fn note_main_shown() {
    MAIN_REVEALED.store(true, Ordering::Release);
    MAIN_IN_TRAY.store(false, Ordering::Release);
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
}
