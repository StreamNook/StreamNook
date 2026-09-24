//! Badge-drop feed state, and the `BadgeNotification` payload shape shared by
//! the relay, the Tauri commands and the UI.
//!
//! The relay re-pushes a badge under a stable id (`{set_id}-v{version}`) when
//! its writeup is corrected or its earn window opens. So "already notified" and
//! "already stored" are tracked separately: two notify legs (first sight,
//! became available) plus a per-badge content hash. On disk rather than in
//! `localStorage`, which a WebView data-folder reset would clear.

use crate::services::cache_service;
use log::debug;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BadgeNotificationStatus {
    New,
    Available,
    ComingSoon,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadgeNotification {
    pub badge_name: String,
    pub badge_set_id: String,
    pub badge_version: String,
    pub badge_image_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge_description: Option<String>,
    pub status: BadgeNotificationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_info: Option<String>,
    /// Rich, campaign-grounded writeup composed by the relay ("Penrose bot") and
    /// delivered in the drop payload. Passed through so the desktop can render it
    /// in the badge More Info panel. Absent on locally-detected notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrichment: Option<serde_json::Value>,
    /// Earn window resolved by `badge_window`, set on the copy that is emitted
    /// to the page and never on the relay's own payload, so it stays out of
    /// `content_hash` (which serializes this struct).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Vec<crate::services::badge_window::WindowRun>>,
}

impl BadgeNotification {
    /// Stable id for this badge, matching the relay's drop id.
    pub fn feed_id(&self) -> String {
        format!("{}-v{}", self.badge_set_id, self.badge_version)
    }

    /// Content fingerprint, for telling a corrected re-push from a redundant one.
    /// Deterministic: `serde_json` maps are `BTreeMap` here, so keys are sorted.
    pub fn content_hash(&self) -> String {
        let text = serde_json::to_string(self).unwrap_or_default();
        // FNV-1a. Detects change; collision resistance is not needed here.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }
}

/// What the feed should do with an incoming drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedAction {
    /// First sight: merge into the gallery, store enrichment, toast.
    NotifyNew,
    /// Earn window has opened: store, then toast availability.
    NotifyAvailable,
    /// Writeup changed: store and amend open panels, no toast.
    SilentStore,
    /// Unchanged, and availability already announced.
    Skip,
}

impl FeedAction {
    /// Whether the gallery merge and enrichment store need to run.
    pub fn stores(&self) -> bool {
        !matches!(self, FeedAction::Skip)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct BadgeFeedStateFile {
    /// Badges surfaced at least once. Name kept for compatibility with state
    /// files written by the retired poller.
    #[serde(default)]
    known_badges: Vec<String>,
    #[serde(default)]
    notified_available_badges: Vec<String>,
    /// Feed id to content hash. Absent in older state files, hence `default`.
    #[serde(default)]
    content_hashes: HashMap<String, String>,
    #[serde(default)]
    last_poll_timestamp_ms: u64,
}

#[derive(Debug, Clone, Default)]
struct BadgeFeedState {
    known_badges: HashSet<String>,
    notified_available_badges: HashSet<String>,
    content_hashes: HashMap<String, String>,
    last_poll_timestamp_ms: u64,
}

impl BadgeFeedState {
    fn from_file(file: BadgeFeedStateFile) -> Self {
        Self {
            known_badges: file.known_badges.into_iter().collect(),
            notified_available_badges: file.notified_available_badges.into_iter().collect(),
            content_hashes: file.content_hashes,
            last_poll_timestamp_ms: file.last_poll_timestamp_ms,
        }
    }

    fn to_file(&self) -> BadgeFeedStateFile {
        BadgeFeedStateFile {
            known_badges: self.known_badges.iter().cloned().collect(),
            notified_available_badges: self
                .notified_available_badges
                .iter()
                .cloned()
                .collect(),
            content_hashes: self.content_hashes.clone(),
            last_poll_timestamp_ms: self.last_poll_timestamp_ms,
        }
    }
}

/// `None` until first load; guarded so the file is read once.
static FEED_STATE: Lazy<RwLock<Option<BadgeFeedState>>> = Lazy::new(|| RwLock::new(None));

fn state_file_path() -> anyhow::Result<PathBuf> {
    Ok(cache_service::get_app_data_dir()?.join("badge_polling_state.json"))
}

fn load_state_from_disk() -> anyhow::Result<BadgeFeedState> {
    let path = state_file_path()?;
    if !path.exists() {
        return Ok(BadgeFeedState::default());
    }
    let text = std::fs::read_to_string(&path)?;
    let file: BadgeFeedStateFile = serde_json::from_str(&text)?;
    Ok(BadgeFeedState::from_file(file))
}

fn save_state_to_disk(state: &BadgeFeedState) -> anyhow::Result<()> {
    let path = state_file_path()?;
    let text = serde_json::to_string_pretty(&state.to_file())?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Loads once, on the blocking pool. Sync file I/O on a runtime worker stalls
/// every async task in the process (Defender-scanned AppData reads run to
/// hundreds of ms), so load and save both use `spawn_blocking`.
async fn ensure_loaded() {
    if FEED_STATE.read().await.is_some() {
        return;
    }
    let loaded = tokio::task::spawn_blocking(load_state_from_disk)
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default();

    let mut guard = FEED_STATE.write().await;
    if guard.is_none() {
        *guard = Some(loaded);
    }
}

/// True when this feed logic has never recorded anything: a fresh install, a
/// lost state file, or a file left behind by the retired poller. The caller
/// seeds that first batch silently instead of toasting the relay's history.
///
/// Keyed on `content_hashes` rather than `known_badges` because the retired
/// poller wrote the same file with the same key format. A legacy file is
/// populated but stops at whenever that poller last ran, so testing
/// `known_badges` reports "not a first run" while every recent badge still
/// looks unseen.
pub async fn is_empty() -> bool {
    ensure_loaded().await;
    FEED_STATE
        .read()
        .await
        .as_ref()
        .map(|s| s.content_hashes.is_empty())
        .unwrap_or(true)
}

/// Decide what to do with a drop, recording nothing. `is_available` comes from
/// the earn window, not the payload's `status` (a snapshot of when it was sent).
pub async fn classify(feed_id: &str, content_hash: &str, is_available: bool) -> FeedAction {
    ensure_loaded().await;
    let guard = FEED_STATE.read().await;
    let Some(state) = guard.as_ref() else {
        return FeedAction::NotifyNew;
    };

    if !state.known_badges.contains(feed_id) {
        return FeedAction::NotifyNew;
    }
    if is_available && !state.notified_available_badges.contains(feed_id) {
        return FeedAction::NotifyAvailable;
    }
    if state.content_hashes.get(feed_id).map(String::as_str) != Some(content_hash) {
        return FeedAction::SilentStore;
    }
    FeedAction::Skip
}

/// Record an outcome. Called only after the store succeeded, so a failed store
/// retries on the next frame. Does not touch disk; `persist` once per batch.
pub async fn record(feed_id: &str, content_hash: &str, action: FeedAction, is_available: bool) {
    ensure_loaded().await;
    let mut guard = FEED_STATE.write().await;
    let Some(state) = guard.as_mut() else {
        return;
    };

    state.known_badges.insert(feed_id.to_string());
    state
        .content_hashes
        .insert(feed_id.to_string(), content_hash.to_string());
    // Latched on first sight of an open window too, so a badge that arrives
    // already available never announces itself twice.
    if is_available || action == FeedAction::NotifyAvailable {
        state
            .notified_available_badges
            .insert(feed_id.to_string());
    }
}

/// Flush to disk. One write per ingest call, not one per drop.
pub async fn persist() {
    let snapshot = {
        let mut guard = FEED_STATE.write().await;
        let Some(state) = guard.as_mut() else {
            return;
        };
        state.last_poll_timestamp_ms = current_time_ms();
        state.clone()
    };
    let _ = tokio::task::spawn_blocking(move || {
        if let Err(e) = save_state_to_disk(&snapshot) {
            debug!("[BadgeFeed] Failed to persist feed state: {e}");
        }
    })
    .await;
}

fn current_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn badge(name: &str) -> BadgeNotification {
        BadgeNotification {
            badge_name: name.to_string(),
            badge_set_id: "spiderman".to_string(),
            badge_version: "1".to_string(),
            badge_image_url: "https://static-cdn.jtvnw.net/badges/v1/abc/3".to_string(),
            badge_description: None,
            status: BadgeNotificationStatus::New,
            date_info: None,
            enrichment: None,
            window: None,
        }
    }

    #[test]
    fn feed_id_matches_the_relay_drop_id() {
        assert_eq!(badge("Spider-Man").feed_id(), "spiderman-v1");
    }

    #[test]
    fn content_hash_is_stable_for_identical_payloads() {
        assert_eq!(badge("Spider-Man").content_hash(), badge("Spider-Man").content_hash());
    }

    #[test]
    fn content_hash_changes_when_the_writeup_is_corrected() {
        let thin = badge("Spider-Man");
        let mut enriched = badge("Spider-Man");
        enriched.enrichment = Some(serde_json::json!({ "how_to_earn": "Watch 2 hours." }));
        assert_ne!(thin.content_hash(), enriched.content_hash());
    }

    // A legacy `badge_polling_state.json` from the retired poller uses the same
    // key format, so it looks populated while stopping at whenever that poller
    // last ran. Seeding must key on content_hashes or every badge that dropped
    // afterwards notifies on the first launch of a new build.
    #[test]
    fn legacy_poller_state_still_counts_as_a_first_run() {
        let legacy = BadgeFeedStateFile {
            known_badges: vec!["spiderman-v1".into(), "budz-v1".into()],
            notified_available_badges: vec![],
            content_hashes: HashMap::new(),
            last_poll_timestamp_ms: 0,
        };
        let state = BadgeFeedState::from_file(legacy);
        assert!(!state.known_badges.is_empty(), "legacy field is populated");
        assert!(
            state.content_hashes.is_empty(),
            "but the new logic has never recorded, so this is a first run"
        );
    }

    #[test]
    fn skip_only_applies_when_nothing_changed() {
        assert!(FeedAction::NotifyNew.stores());
        assert!(FeedAction::NotifyAvailable.stores());
        assert!(FeedAction::SilentStore.stores());
        assert!(!FeedAction::Skip.stores());
    }
}
