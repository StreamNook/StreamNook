//! The periodic update check: shortly after start, then every half hour.
//!
//! Before this, the title-bar indicator's check ran from the app shell and the
//! notification centre ran its own 5 s + 30 min loop, both on webview timers
//! tied to the main window's life. One Rust loop now checks, and announces the
//! result as `update://status { status, announce }`. `announce` is true the
//! first time a version is seen as available, so it is notified once, across
//! restarts, while the indicator follows every check. A user-requested check
//! ("Check for updates") still calls `check_for_bundle_update` directly.

use crate::models::components::BundleUpdateStatus;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub const STATUS_EVENT: &str = "update://status";

const FIRST_CHECK: Duration = Duration::from_secs(5);
const INTERVAL: Duration = Duration::from_secs(30 * 60);
const ANNOUNCED_FILE: &str = "announced_updates.json";

#[derive(Debug, Clone, Serialize)]
struct StatusUpdate {
    status: BundleUpdateStatus,
    announce: bool,
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

pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        let mut announced = load_announced();
        loop {
            match crate::commands::components::check_for_bundle_update().await {
                Ok(status) => {
                    let announce = status.update_available && first_announcement(&mut announced, &status.latest_version);
                    if announce {
                        if let (Some(path), Ok(json)) = (announced_path(), serde_json::to_string(&announced)) {
                            let _ = std::fs::write(path, json);
                        }
                    }
                    let _ = app.emit(STATUS_EVENT, StatusUpdate { status, announce });
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
}
