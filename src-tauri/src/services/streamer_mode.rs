//! Streamer mode: hide what should not be on stream (viewer counts, link
//! previews, restricted users' rows, highlight sounds). Rust owns the state
//! and the detection; surfaces read one flag.
//!
//! Settings group `streamer_mode` (frontend-managed, rides the flattened
//! `extra` map): `{ mode: "off" | "on" | "auto", ... }`. In `auto`, a
//! ToolHelp process snapshot every 30 s looks for known broadcasting
//! software; in `on`/`off` nothing runs at all. The task sleeps on a Notify
//! between settings changes, so a disabled feature costs zero wakeups.

use crate::models::settings::Settings;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};
use tokio::sync::Notify;

const AUTO_POLL_SECS: u64 = 30;

/// Process names (lowercase, without path) that mean a broadcast is likely
/// running. Same list Chatterino uses plus Twitch Studio.
///
/// **The list is per-platform because the NAMES are.** macOS processes carry no
/// `.exe`, and `ps -o comm` reports a bundle's inner executable, so OBS appears
/// as `obs`, not `obs64.exe`. Matching the Windows list on macOS is why
/// auto-detect could never fire there: it was not that detection failed, it was
/// that it was looking for filenames that do not exist on the platform.
#[cfg(windows)]
const BROADCAST_PROCESSES: &[&str] = &[
    "obs64.exe",
    "obs32.exe",
    "obs.exe",
    "streamlabs obs.exe",
    "streamlabsobs.exe",
    "streamlabs.exe",
    "xsplit.core.exe",
    "twitchstudio.exe",
    "vmix64.exe",
    "vmix.exe",
    "prismlivestudio.exe",
];

/// XSplit and vMix are Windows-only products, so they are absent here rather
/// than merely unlisted.
#[cfg(target_os = "macos")]
const BROADCAST_PROCESSES: &[&str] = &[
    "obs",
    "streamlabs desktop",
    "streamlabs obs",
    "streamlabs",
    "twitch studio",
    "prism live studio",
    "prismlivestudio",
    "ecamm live",
];

#[cfg(all(unix, not(target_os = "macos")))]
const BROADCAST_PROCESSES: &[&str] = &["obs", "obs-studio", "streamlabs-desktop"];

const MODE_OFF: u8 = 0;
const MODE_ON: u8 = 1;
const MODE_AUTO: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(MODE_OFF);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static NOTIFY: OnceLock<Notify> = OnceLock::new();
static APP: OnceLock<AppHandle> = OnceLock::new();

fn notify() -> &'static Notify {
    NOTIFY.get_or_init(Notify::new)
}

#[derive(Deserialize, Default)]
struct StreamerModeSettings {
    #[serde(default)]
    mode: Option<String>,
}

#[derive(Serialize, Clone, Copy)]
pub struct StreamerModeState {
    pub active: bool,
    /// "off" | "on" | "auto"
    pub mode: &'static str,
}

pub struct StreamerMode;

impl StreamerMode {
    /// Start the detector task. Idempotent; call once from setup.
    pub fn init(app: AppHandle) {
        if APP.set(app).is_err() {
            return;
        }
        tauri::async_runtime::spawn(async {
            loop {
                match MODE.load(Ordering::Acquire) {
                    MODE_AUTO => {
                        let found = tokio::task::spawn_blocking(broadcast_running)
                            .await
                            .unwrap_or(false);
                        Self::set_active(found);
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_secs(AUTO_POLL_SECS)) => {}
                            _ = notify().notified() => {}
                        }
                    }
                    MODE_ON => {
                        Self::set_active(true);
                        notify().notified().await;
                    }
                    _ => {
                        Self::set_active(false);
                        notify().notified().await;
                    }
                }
            }
        });
    }

    /// Read the mode from settings and wake the task if it changed.
    pub fn refresh(settings: &Settings) {
        let cfg: StreamerModeSettings = settings
            .extra
            .get("streamer_mode")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let mode = match cfg.mode.as_deref() {
            Some("on") => MODE_ON,
            Some("auto") => MODE_AUTO,
            _ => MODE_OFF,
        };
        let prev = MODE.swap(mode, Ordering::AcqRel);
        if prev != mode {
            notify().notify_one();
        }
    }

    pub fn state() -> StreamerModeState {
        StreamerModeState {
            active: ACTIVE.load(Ordering::Acquire),
            mode: match MODE.load(Ordering::Acquire) {
                MODE_ON => "on",
                MODE_AUTO => "auto",
                _ => "off",
            },
        }
    }

    pub fn is_active() -> bool {
        ACTIVE.load(Ordering::Acquire)
    }

    fn set_active(active: bool) {
        let prev = ACTIVE.swap(active, Ordering::AcqRel);
        if prev != active {
            log::info!("[StreamerMode] {}", if active { "active" } else { "inactive" });
            if let Some(app) = APP.get() {
                let _ = app.emit("streamer-mode-changed", Self::state());
            }
        }
    }
}

/// True when any known broadcasting app is running.
///
/// The process walk itself moved to `platform::process::running_names`, which
/// has a real implementation on every platform (ToolHelp on Windows, `ps`
/// elsewhere). This function is now pure policy: compare what is running
/// against the per-platform name list above.
fn broadcast_running() -> bool {
    let running = crate::platform::process::running_names();
    BROADCAST_PROCESSES
        .iter()
        .any(|wanted| running.contains(*wanted))
}


