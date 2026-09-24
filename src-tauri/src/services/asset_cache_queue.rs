//! Filling the on-disk image cache (emotes, 7TV cosmetics, Twitch badges), one
//! queue per kind for every window.
//!
//! Each window used to run its own copy of this per kind: its own queue, its
//! own in-flight set, its own pacing, and one IPC call per file. A channel open
//! in the main window and a MultiChat popout queued the same set twice. Windows
//! now hand over ids in batches; Rust dedupes across them, answers what is
//! already on disk at once, paces the downloads, and announces files as they
//! land (`asset-cache://cached { kind, files: { id: path } }`) so every window
//! can point at them.
//!
//! Pacing, per kind:
//! - emotes trickle one at a time with a real gap, so background caching never
//!   competes with the live video; while an emote picker is open (the one moment
//!   someone is waiting on emotes) they burst five at a time;
//! - cosmetics and badges are few and small: five at a time, no gap.

use crate::services::universal_cache_service::{cache_file, cached_file_paths, CacheType};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const CACHED_EVENT: &str = "asset-cache://cached";
/// The cache folder was wiped: every window drops its id -> path maps.
pub const CLEARED_EVENT: &str = "asset-cache://cleared";

const POLITE_CONCURRENT: usize = 1;
const POLITE_GAP: Duration = Duration::from_millis(250);
const BURST_CONCURRENT: usize = 5;
const BURST_GAP: Duration = Duration::from_millis(15);
const SMALL_ASSET_CONCURRENT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Emote,
    Cosmetic,
    Badge,
}

impl AssetKind {
    fn cache_type(self) -> CacheType {
        match self {
            AssetKind::Emote => CacheType::Emote,
            AssetKind::Cosmetic => CacheType::Cosmetic,
            AssetKind::Badge => CacheType::Badge,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct Cached {
    kind: AssetKind,
    files: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetRequest {
    pub id: String,
    pub url: String,
}

#[derive(Default)]
struct Lane {
    queue: VecDeque<AssetRequest>,
    /// Queued or downloading: the dedupe set.
    known: HashSet<String>,
    active: usize,
}

#[derive(Default)]
struct State {
    lanes: HashMap<AssetKind, Lane>,
    /// Window label -> open emote pickers in it.
    burst_owners: HashMap<String, usize>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// Add requests to a lane, skipping any already queued or downloading.
/// Priority requests go to the front. Returns how many were new.
fn admit(lane: &mut Lane, items: Vec<AssetRequest>, priority: bool) -> usize {
    let mut fresh = Vec::new();
    for item in items {
        if !item.id.is_empty() && !item.url.is_empty() && lane.known.insert(item.id.clone()) {
            fresh.push(item);
        }
    }
    let added = fresh.len();
    if priority {
        for item in fresh.into_iter().rev() {
            lane.queue.push_front(item);
        }
    } else {
        lane.queue.extend(fresh);
    }
    added
}

fn bursting(app: &AppHandle, state: &mut State) -> bool {
    // A window that closed with a picker open never said so; drop it here.
    state.burst_owners.retain(|label, _| app.get_webview_window(label).is_some());
    !state.burst_owners.is_empty()
}

fn pacing(kind: AssetKind, burst: bool) -> (usize, Duration) {
    match kind {
        AssetKind::Emote if burst => (BURST_CONCURRENT, BURST_GAP),
        AssetKind::Emote => (POLITE_CONCURRENT, POLITE_GAP),
        _ => (SMALL_ASSET_CONCURRENT, Duration::ZERO),
    }
}

/// Start as many downloads as the lane's pacing allows.
fn pump(app: &AppHandle, kind: AssetKind) {
    let starts: Vec<(AssetRequest, Duration)> = with_state(|s| {
        let burst = bursting(app, s);
        let (limit, gap) = pacing(kind, burst);
        let lane = s.lanes.entry(kind).or_default();
        let mut starts = Vec::new();
        while lane.active < limit {
            let Some(item) = lane.queue.pop_front() else { break };
            lane.active += 1;
            starts.push((item, gap * starts.len() as u32));
        }
        starts
    });
    for (item, delay) in starts {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if let Some(path) = download(&app, kind, &item).await {
                announce(&app, kind, HashMap::from([(item.id.clone(), path)]));
            }
            let gap = with_state(|s| {
                let burst = bursting(&app, s);
                let lane = s.lanes.entry(kind).or_default();
                lane.active = lane.active.saturating_sub(1);
                lane.known.remove(&item.id);
                pacing(kind, burst).1
            });
            if !gap.is_zero() {
                tokio::time::sleep(gap).await;
            }
            pump(&app, kind);
        });
    }
}

fn cache_settings(app: &AppHandle) -> (bool, u32) {
    app.state::<crate::models::settings::AppState>()
        .settings
        .lock()
        .map(|s| (s.cache.enabled, s.cache.expiry_days))
        .unwrap_or((true, 7))
}

fn announce(app: &AppHandle, kind: AssetKind, files: HashMap<String, String>) {
    if !files.is_empty() {
        let _ = app.emit(CACHED_EVENT, Cached { kind, files });
    }
}

/// Tell every window the files it points at are gone.
pub fn announce_cleared(app: &AppHandle) {
    let _ = app.emit(CLEARED_EVENT, ());
}

/// Announce files written outside this queue (the AFK emote prefetch), so
/// every window can point at them as they land rather than after a relisting.
pub fn announce_entries(app: &AppHandle, kind: AssetKind, entries: &[crate::services::universal_cache_service::UniversalCacheEntry]) {
    let files = entries
        .iter()
        .filter_map(|e| {
            let id = e.id.strip_prefix("file:")?;
            let path = e.data.get("local_path")?.as_str()?;
            Some((id.to_string(), path.to_string()))
        })
        .collect();
    announce(app, kind, files);
}

/// Download one file. None when the cache was turned off meanwhile or the
/// download failed (the page keeps its CDN URL).
async fn download(app: &AppHandle, kind: AssetKind, item: &AssetRequest) -> Option<String> {
    let (enabled, expiry_days) = cache_settings(app);
    if !enabled {
        return None;
    }
    match cache_file(kind.cache_type(), item.id.clone(), item.url.clone(), expiry_days).await {
        Ok(path) => Some(path),
        Err(e) => {
            log::debug!("[AssetCache] could not cache {:?} {}: {e}", kind, item.id);
            None
        }
    }
}

/// Split requests into files already on disk (id -> path) and ones to fetch.
/// Reads the in-memory manifest only: no network, no pacing. Uses the same rule
/// as the per-window listing, so a file a window would show is never re-queued.
fn split_cached(kind: AssetKind, items: Vec<AssetRequest>) -> (HashMap<String, String>, Vec<AssetRequest>) {
    let ids: Vec<String> = items.iter().map(|i| i.id.clone()).collect();
    let hits: HashMap<String, String> = cached_file_paths(kind.cache_type(), &ids)
        .into_iter()
        .filter(|(_, path)| std::path::Path::new(path).exists())
        .collect();
    let misses = items.into_iter().filter(|i| !hits.contains_key(&i.id)).collect();
    (hits, misses)
}

/// Queue files for the disk cache. Ones already there are announced at once.
pub fn enqueue(app: &AppHandle, kind: AssetKind, items: Vec<AssetRequest>, priority: bool) {
    if items.is_empty() || !cache_settings(app).0 {
        return;
    }
    let (hits, misses) = split_cached(kind, items);
    announce(app, kind, hits);
    let added = with_state(|s| admit(s.lanes.entry(kind).or_default(), misses, priority));
    if added > 0 {
        pump(app, kind);
    }
}

/// An emote picker opened (true) or closed (false) in `window`.
pub fn set_burst(app: &AppHandle, window: &str, active: bool) {
    with_state(|s| {
        let count = s.burst_owners.entry(window.to_string()).or_insert(0);
        if active {
            *count += 1;
        } else {
            *count = count.saturating_sub(1);
            if *count == 0 {
                s.burst_owners.remove(window);
            }
        }
    });
    if active {
        pump(app, AssetKind::Emote);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: &str) -> AssetRequest {
        AssetRequest { id: id.into(), url: format!("https://cdn/{id}") }
    }

    #[test]
    fn repeats_across_windows_are_queued_once() {
        let mut lane = Lane::default();
        assert_eq!(admit(&mut lane, vec![req("a"), req("b"), req("a")], false), 2);
        assert_eq!(admit(&mut lane, vec![req("b"), req("c")], false), 1);
        let ids: Vec<_> = lane.queue.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn priority_requests_jump_the_queue_in_order() {
        let mut lane = Lane::default();
        admit(&mut lane, vec![req("a"), req("b")], false);
        admit(&mut lane, vec![req("x"), req("y")], true);
        let ids: Vec<_> = lane.queue.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["x", "y", "a", "b"]);
    }

    #[test]
    fn empty_ids_or_urls_are_ignored() {
        let mut lane = Lane::default();
        assert_eq!(admit(&mut lane, vec![AssetRequest { id: String::new(), url: "u".into() }, AssetRequest { id: "i".into(), url: String::new() }], false), 0);
    }

    #[test]
    fn emotes_trickle_unless_a_picker_is_open() {
        assert_eq!(pacing(AssetKind::Emote, false), (POLITE_CONCURRENT, POLITE_GAP));
        assert_eq!(pacing(AssetKind::Emote, true), (BURST_CONCURRENT, BURST_GAP));
        assert_eq!(pacing(AssetKind::Badge, false).0, SMALL_ASSET_CONCURRENT);
    }
}
