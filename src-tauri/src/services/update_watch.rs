//! The periodic update check: shortly after start, then every half hour.
//!
//! Before this, the title-bar indicator's check ran from the app shell and the
//! notification centre ran its own 5 s + 30 min loop, both on webview timers
//! tied to the main window's life. One Rust loop now checks and sends the
//! result as `update://status { status }`. A user-requested check ("Check for
//! updates") still calls `check_for_bundle_update` directly.
//!
//! An event only reaches the windows listening when it is sent, and the main
//! window often is not: close-to-tray and Go Live destroy it, reopening builds
//! a new one, and a slow cold start can mount it after the first check. Such a
//! window showed no update until the next half-hour tick, so opening the app no
//! longer showed one at all. The last answer is kept here instead, and a window
//! reads it when it mounts (`get_update_status`).
//!
//! The announcement (the notification-centre entry and its sound) is made once
//! per version, across restarts. The window that shows it claims it, so an
//! update found while no window was listening is announced when one opens
//! rather than spent on nobody.

use crate::models::components::BundleUpdateStatus;
use crate::models::settings::AppState;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use crate::rt::AppHandle;
use tauri::{Emitter, Manager};

pub const STATUS_EVENT: &str = "update://status";

const FIRST_CHECK: Duration = Duration::from_secs(5);
const INTERVAL: Duration = Duration::from_secs(30 * 60);
const ANNOUNCED_FILE: &str = "announced_updates.json";

/// The last periodic check's answer. None until the first one finishes.
static LAST: Lazy<Mutex<Option<BundleUpdateStatus>>> = Lazy::new(|| Mutex::new(None));
/// Versions already announced, mirrored to `ANNOUNCED_FILE`.
static ANNOUNCED: Lazy<Mutex<BTreeSet<String>>> = Lazy::new(|| Mutex::new(load_announced()));

#[derive(Debug, Clone, Serialize)]
struct StatusEvent {
    status: BundleUpdateStatus,
}

/// What `get_update_status` hands a window.
#[derive(Debug, Clone, Serialize)]
pub struct StatusUpdate {
    pub status: BundleUpdateStatus,
    /// True for the one caller that claimed this version's announcement, and
    /// only while update notifications are on.
    pub announce: bool,
}

fn announced_path() -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir().ok().map(|d| d.join(ANNOUNCED_FILE))
}

fn load_announced() -> BTreeSet<String> {
    announced_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Record `version` as announced; true when it was not already.
fn first_announcement(announced: &mut BTreeSet<String>, version: &str) -> bool {
    !version.is_empty() && announced.insert(version.to_string())
}

/// Whether this caller announces `status`: only a claiming caller, only for an
/// available version not announced before, and only with update notifications
/// on. The setting is read last, so the claim is spent either way, as it always
/// was: turning notifications on later does not announce an old find.
fn takes_announcement(
    status: &BundleUpdateStatus,
    claim_announcement: bool,
    announced: &mut BTreeSet<String>,
    notifications_on: impl FnOnce() -> bool,
) -> bool {
    claim_announcement
        && status.update_available
        && first_announcement(announced, &status.latest_version)
        && notifications_on()
}

fn update_notifications_on(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let Ok(settings) = state.settings.lock() else { return true };
    settings.live_notifications.enabled && settings.live_notifications.show_update_notifications
}

/// The last answer, for a window that mounted after it was sent. With
/// `claim_announcement`, also takes the announcement if this version has not
/// had one, persisting it so no later window or restart makes it again.
pub fn current(app: &AppHandle, claim_announcement: bool) -> Option<StatusUpdate> {
    let status = LAST.lock().ok()?.clone()?;
    let Ok(mut announced) = ANNOUNCED.lock() else {
        return Some(StatusUpdate { status, announce: false });
    };
    let before = announced.len();
    let announce = takes_announcement(&status, claim_announcement, &mut announced, || {
        update_notifications_on(app)
    });
    if announced.len() != before {
        if let (Some(path), Ok(json)) = (announced_path(), serde_json::to_string(&*announced)) {
            let _ = std::fs::write(path, json);
        }
    }
    Some(StatusUpdate { status, announce })
}

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            match crate::commands::components::check_for_bundle_update(app.clone()).await {
                Ok(status) => {
                    if let Ok(mut last) = LAST.lock() {
                        *last = Some(status.clone());
                    }
                    let _ = app.emit(STATUS_EVENT, StatusEvent { status });
                }
                Err(e) => log::warn!("[UpdateWatch] update check failed: {e}"),
            }
            tokio::time::sleep(INTERVAL).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_version_is_announced_once() {
        let mut seen = BTreeSet::new();
        assert!(first_announcement(&mut seen, "8.7.0"));
        assert!(!first_announcement(&mut seen, "8.7.0"));
        assert!(first_announcement(&mut seen, "8.7.1"));
        assert!(!first_announcement(&mut seen, ""));
    }

    fn status(available: bool, latest: &str) -> BundleUpdateStatus {
        serde_json::from_value(serde_json::json!({
            "update_available": available,
            "current_version": "8.8.11",
            "latest_version": latest,
        }))
        .expect("every other field is optional")
    }

    #[test]
    fn the_title_bar_read_never_spends_the_announcement() {
        let mut seen = BTreeSet::new();
        let s = status(true, "8.8.12");
        assert!(!takes_announcement(&s, false, &mut seen, || true));
        assert!(!takes_announcement(&s, false, &mut seen, || true));
        // The window that opens later, after the check nobody heard, still gets it.
        assert!(takes_announcement(&s, true, &mut seen, || true));
        assert!(!takes_announcement(&s, true, &mut seen, || true));
    }

    #[test]
    fn notifications_off_spends_the_claim_without_announcing() {
        let mut seen = BTreeSet::new();
        let s = status(true, "8.8.12");
        assert!(!takes_announcement(&s, true, &mut seen, || false));
        assert!(!takes_announcement(&s, true, &mut seen, || true));
        assert!(takes_announcement(&status(true, "8.8.13"), true, &mut seen, || true));
    }

    #[test]
    fn nothing_to_announce_without_an_update() {
        let mut seen = BTreeSet::new();
        assert!(!takes_announcement(&status(false, "8.8.11"), true, &mut seen, || true));
        assert!(seen.is_empty());
    }
}
