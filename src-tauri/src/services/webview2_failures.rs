//! Records WebView2 process failures, the crashes that take the page (or every
//! page) down without the Rust process noticing.
//!
//! A renderer that dies leaves the window showing a blank or "page crashed"
//! surface, and a browser process that dies leaves every webview on it dead:
//! from then on each call into them fails with 0x8007139F (ERROR_INVALID_STATE),
//! which tauri-runtime-wry logs as a bare "WebView2 error" line. To the viewer
//! either one reads as "the app crashed", and before this nothing recorded
//! which process went, why, or with what exit code.
//!
//! The record is written synchronously (`file_log::record_now`) because the
//! next thing that often happens is the viewer closing the dead window.
//!
//! A browser process that died cannot be brought back in place, so the app
//! relaunches itself instead, once. The commonest way to get there was a
//! launch that attached to the previous run's browser process while it was
//! still shutting down (the updater relaunches the new exe half a second after
//! the old one quits), so boot first waits for that process to be gone.

/// The tauri.conf.json `identifier`. WebView2's default data folder for the
/// app windows is `%LOCALAPPDATA%\<identifier>\EBWebView`.
const IDENTIFIER: &str = "com.streamnook.dev";

/// Set on the relaunched process: the unix second the relaunch was asked for.
const RELAUNCH_ENV: &str = "SN_WEBVIEW2_RELAUNCHED_AT";

/// `RELAUNCH_ENV` as this process found it at boot.
static RELAUNCHED_AT: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();

/// A relaunched process whose browser dies again within this long does not
/// relaunch a second time, so a fault that kills every start cannot loop.
const RELAUNCH_LOOP_GUARD_SECS: u64 = 120;

/// How long boot waits for the previous run's browser process to exit. It
/// usually takes a few seconds; past this the launch goes ahead as before.
#[cfg(windows)]
const PREVIOUS_BROWSER_WAIT: std::time::Duration = std::time::Duration::from_secs(15);

/// Before any webview exists: note a recovery relaunch, then wait for a
/// browser process the previous run left on the app's WebView2 data folder.
///
/// WebView2 hands a new environment the browser process already running on
/// the same data folder with the same arguments, even one whose host has quit
/// and that is tearing itself down. When that process then exits (or faults
/// on the way out, as the logs show with 0xc0000005) it takes the new window
/// with it, and the app sits black from its first seconds. Chromium holds
/// `<data folder>\lockfile` open without sharing for as long as its browser
/// process lives, so the lock says when it is gone.
pub fn before_first_webview() {
    // Taken out of the environment so nothing this run spawns (the updater's
    // relaunch, a later restart) inherits it.
    if let Ok(at) = std::env::var(RELAUNCH_ENV) {
        std::env::remove_var(RELAUNCH_ENV);
        log::info!("[WebView2] relaunched after the browser process failed (asked at unix {at})");
        let _ = RELAUNCHED_AT.set(at.trim().parse::<u64>().ok());
    }
    #[cfg(windows)]
    wait_for_previous_browser();
}

#[cfg(windows)]
fn wait_for_previous_browser() {
    use std::time::{Duration, Instant};

    let Some(lock) = dirs::data_local_dir().map(|d| d.join(IDENTIFIER).join("EBWebView").join("lockfile"))
    else {
        return;
    };
    if !profile_in_use(&lock) {
        return;
    }
    // A running StreamNook owns that browser: this launch is a second one,
    // which the single-instance plugin hands over to it. Waiting would only
    // delay the hand-over.
    if another_instance_running() {
        log::info!("[WebView2] data folder in use by a running StreamNook; leaving it to the single-instance hand-off");
        return;
    }
    let started = Instant::now();
    while profile_in_use(&lock) {
        if started.elapsed() >= PREVIOUS_BROWSER_WAIT {
            log::warn!(
                "[WebView2] the previous run's browser process still holds the data folder after {} s; starting anyway",
                PREVIOUS_BROWSER_WAIT.as_secs()
            );
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    log::info!(
        "[WebView2] waited {} ms for the previous run's browser process to exit",
        started.elapsed().as_millis()
    );
}

/// Whether a live process holds the WebView2 data folder's lockfile.
#[cfg(windows)]
fn profile_in_use(lock: &std::path::Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ_WRITE_DELETE: u32 = 0x7;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ_WRITE_DELETE)
        .open(lock)
    {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

/// A StreamNook instance is running: the single-instance plugin's mutex
/// (`<identifier>-sim`) exists. Only a process past the plugin's setup holds
/// it, so the portable launcher that hands over to the installed copy, which
/// exits before any of that, never counts.
#[cfg(windows)]
fn another_instance_running() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenMutexW, SYNCHRONIZATION_SYNCHRONIZE};

    let name = HSTRING::from(format!("{IDENTIFIER}-sim"));
    // SAFETY: the name is a valid wide string for the call; the handle is
    // closed right away.
    match unsafe { OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, &name) } {
        Ok(handle) => {
            unsafe {
                let _ = CloseHandle(handle);
            }
            true
        }
        Err(_) => false,
    }
}

/// Whether a browser-process failure should relaunch the app, given when
/// this process was itself relaunched for one (unix seconds) and the time now.
fn relaunch_allowed(relaunched_at: Option<u64>, now: u64) -> bool {
    match relaunched_at {
        Some(at) => now.saturating_sub(at) >= RELAUNCH_LOOP_GUARD_SECS,
        None => true,
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The browser process is gone and every app window on it is dead. Relaunch
/// once; the first window to report it wins, the rest see the flag.
#[cfg(windows)]
fn relaunch_after_browser_failure(app: &crate::rt::AppHandle) -> &'static str {
    use std::sync::atomic::{AtomicBool, Ordering};
    static RELAUNCHING: AtomicBool = AtomicBool::new(false);

    if cfg!(debug_assertions) {
        // A dev build relaunches the bare exe, which would end `tauri dev`.
        return "none in place (dev build, not relaunching)";
    }
    let previous = RELAUNCHED_AT.get().copied().flatten();
    let now = unix_now();
    if !relaunch_allowed(previous, now) {
        return "none in place (this run is itself a relaunch for the same failure)";
    }
    if RELAUNCHING.swap(true, Ordering::AcqRel) {
        return "relaunch already under way";
    }
    // The relaunched process inherits this environment.
    std::env::set_var(RELAUNCH_ENV, now.to_string());
    let app = app.clone();
    // Out of the COM callback first: the restart runs the exit events, which
    // tear down the webview that is calling us.
    let _ = app.clone().run_on_main_thread(move || {
        crate::services::app_exit::request_restart(&app, "WebView2 browser process failed");
    });
    "relaunching the app"
}

/// Watch `win`'s WebView2 for process failures. A main-frame renderer that
/// exited is reloaded in place; a browser process that died relaunches the
/// app (see `relaunch_after_browser_failure`).
#[cfg(windows)]
pub fn watch(win: &crate::rt::WebviewWindow) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2, ICoreWebView2ProcessFailedEventArgs,
        ICoreWebView2ProcessFailedEventArgs2, COREWEBVIEW2_PROCESS_FAILED_KIND,
        COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        COREWEBVIEW2_PROCESS_FAILED_REASON,
    };
    use webview2_com::ProcessFailedEventHandler;
    use windows::core::Interface;
    use tauri::Manager;

    let label = win.label().to_string();
    let app = win.app_handle().clone();
    let dispatched = win.with_webview(move |platform_webview| unsafe {
        let core = match platform_webview.controller().CoreWebView2() {
            Ok(c) => c,
            Err(e) => {
                log::warn!("[WebView2] no CoreWebView2 to watch for process failures on '{label}': {e}");
                return;
            }
        };
        let window = label.clone();
        let handler = ProcessFailedEventHandler::create(Box::new(
            move |sender: Option<ICoreWebView2>,
                  args: Option<ICoreWebView2ProcessFailedEventArgs>| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                let _ = args.ProcessFailedKind(&mut kind);
                let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON::default();
                let mut exit_code: i32 = 0;
                if let Ok(args2) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
                    let _ = args2.Reason(&mut reason);
                    let _ = args2.ExitCode(&mut exit_code);
                }
                let summary = format!(
                    "WebView2 {} failed in window '{}' (reason {}, exit code {:#010x})",
                    kind_name(kind.0),
                    window,
                    reason_name(reason.0),
                    exit_code as u32
                );
                let recovering = kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED;
                let recovery = if recovering {
                    "reloading the page"
                } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
                    relaunch_after_browser_failure(&app)
                } else {
                    "none in place"
                };
                crate::services::file_log::record_now("WebView2", &summary, "Recovery", recovery);
                if recovering {
                    if let Some(sender) = sender {
                        let _ = sender.Reload();
                    }
                }
                Ok(())
            },
        ));
        // webview2-com 0.38 types the registration token as a plain i64.
        let mut token: i64 = 0;
        if let Err(e) = core.add_ProcessFailed(&handler, &mut token) {
            log::warn!("[WebView2] add_ProcessFailed failed on '{label}': {e}");
        }
    });
    if let Err(e) = dispatched {
        log::warn!("[WebView2] couldn't reach the webview to watch for process failures: {e}");
    }
}

/// `COREWEBVIEW2_PROCESS_FAILED_KIND` by name.
fn kind_name(kind: i32) -> String {
    match kind {
        0 => "browser process".into(),
        1 => "renderer process".into(),
        2 => "renderer process (unresponsive)".into(),
        3 => "frame renderer process".into(),
        4 => "utility process".into(),
        5 => "sandbox helper process".into(),
        6 => "GPU process".into(),
        7 => "PPAPI plugin process".into(),
        8 => "PPAPI broker process".into(),
        9 => "unknown process".into(),
        other => format!("process kind {other}"),
    }
}

/// `COREWEBVIEW2_PROCESS_FAILED_REASON` by name.
fn reason_name(reason: i32) -> String {
    match reason {
        0 => "unexpected".into(),
        1 => "unresponsive".into(),
        2 => "terminated".into(),
        3 => "crashed".into(),
        4 => "launch failed".into(),
        5 => "out of memory".into(),
        6 => "profile deleted".into(),
        other => format!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_kinds_and_reasons_read_as_words() {
        assert_eq!(kind_name(0), "browser process");
        assert_eq!(kind_name(6), "GPU process");
        assert_eq!(kind_name(42), "process kind 42");
        assert_eq!(reason_name(3), "crashed");
        assert_eq!(reason_name(9), "9");
    }

    #[test]
    fn a_relaunch_happens_once_per_failure_not_in_a_loop() {
        assert!(relaunch_allowed(None, 1_000));
        assert!(!relaunch_allowed(Some(1_000), 1_000));
        assert!(!relaunch_allowed(Some(1_000), 1_000 + RELAUNCH_LOOP_GUARD_SECS - 1));
        assert!(relaunch_allowed(Some(1_000), 1_000 + RELAUNCH_LOOP_GUARD_SECS));
        // A clock that went backwards reads as just relaunched.
        assert!(!relaunch_allowed(Some(1_000), 10));
    }

    #[cfg(windows)]
    #[test]
    fn a_lockfile_reads_in_use_only_while_a_process_holds_it() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = std::env::temp_dir().join(format!("sn-wv2-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lock = dir.join("lockfile");
        assert!(!profile_in_use(&lock), "a missing lockfile is free");
        // Held the way Chromium holds it: open, sharing nothing.
        let held = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .share_mode(0)
            .open(&lock)
            .unwrap();
        assert!(profile_in_use(&lock));
        drop(held);
        assert!(!profile_in_use(&lock), "a lockfile a dead process left behind is free");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_data_folder_follows_the_config_identifier() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).expect("tauri.conf.json parses");
        assert_eq!(config["identifier"].as_str(), Some(IDENTIFIER));
    }
}
