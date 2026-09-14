//! VOD watch-position store: where the viewer left off in each video, so a
//! VOD opened again resumes there and its card shows how far they got.
//!
//! Rust owns this state end to end. The WebView only samples the media
//! element's playhead (the one thing it alone can see) and reports it here at
//! a low cadence; everything else (keying, the resume policy, retention,
//! persistence) lives on this side, so the MultiChat popout's VOD modal and a
//! recreated main window read the same canonical copy.
//!
//! Shape: one JSON file per signed-in account under
//! `<app_data>/vod_progress/<twitch_user_id>.json` (`anon` when nobody is
//! signed in), lazily seeded on first touch, mutated in memory, flushed by a
//! 5 s debounced task and again on exit (the whisper-storage pattern). Writes
//! go through a temp file + rename so a crash mid-write can never truncate the
//! store. Bounded: `CAP` entries per account (least recently updated evicted)
//! and `RETENTION` days (stale entries dropped at seed time).
//!
//! Keys are composite (`twitch:<video_id>`) from day one. There is a single
//! writer and no legacy data, so this is free now and lets Kick VODs share the
//! store later without a key-space migration.

use crate::models::stream::TwitchVideo;
use crate::services::account_store::AccountStore;
use crate::services::cache_service::get_app_data_dir;
use log::debug;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const STORE_DIR: &str = "vod_progress";
const FILE_VERSION: u32 = 1;
/// Entries kept per account. Past this the least recently updated go first.
const CAP: usize = 500;
/// Entries untouched for this long are dropped when the file is next seeded.
const RETENTION_SECS: i64 = 90 * 24 * 60 * 60;
/// Below this many seconds in, a VOD restarts from the top rather than
/// resuming: nobody wants to be dropped 12 s into a broadcast.
pub const RESUME_MIN_SECS: f64 = 30.0;
/// Within this many seconds of the end the VOD counts as finished and the
/// next open starts over.
pub const RESUME_TAIL_SECS: f64 = 60.0;
/// How long a resolved owner id is trusted before `accounts.json` is re-read.
const OWNER_CACHE_TTL: Duration = Duration::from_secs(60);
const ANON_OWNER: &str = "anon";

/// One video's watch state. `position_secs` is the last playhead reported;
/// `duration_secs` is the video length known at that time (a recording VOD
/// keeps growing, so it is only a hint for the card's progress bar).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VodProgress {
    pub video_id: String,
    pub position_secs: f64,
    pub duration_secs: f64,
    /// Unix seconds of the last report.
    pub updated_at: i64,
    pub completed: bool,
    #[serde(default)]
    pub channel_login: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub thumbnail_url: String,
}

/// The slice of a `VodProgress` a video card needs, joined onto `TwitchVideo`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VodProgressSummary {
    pub position_secs: f64,
    pub duration_secs: f64,
    pub completed: bool,
}

/// Optional descriptive fields a report may carry so a future "continue
/// watching" surface needs no Twitch call to render the row.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VodMeta {
    pub channel_login: Option<String>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProgressFile {
    version: u32,
    entries: HashMap<String, VodProgress>,
}

impl Default for ProgressFile {
    fn default() -> Self {
        Self {
            version: FILE_VERSION,
            entries: HashMap::new(),
        }
    }
}

struct OwnerStore {
    path: PathBuf,
    file: ProgressFile,
    dirty: bool,
}

static STORE: OnceLock<Mutex<HashMap<String, OwnerStore>>> = OnceLock::new();
static ANY_DIRTY: AtomicBool = AtomicBool::new(false);
static FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);
static OWNER_CACHE: Mutex<Option<(String, Instant)>> = Mutex::new(None);

fn store() -> &'static Mutex<HashMap<String, OwnerStore>> {
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Composite store key for a Twitch VOD.
pub fn vod_key(video_id: &str) -> String {
    format!("twitch:{}", video_id.trim())
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The resume decision for a stored position, pure so it can be tested.
/// Returns the position to start at when it is worth resuming, and whether
/// the video counts as finished.
pub fn resume_decision(position_secs: f64, duration_secs: f64) -> (Option<f64>, bool) {
    if !position_secs.is_finite() || position_secs < RESUME_MIN_SECS {
        return (None, false);
    }
    let completed = duration_secs.is_finite()
        && duration_secs > 0.0
        && position_secs >= duration_secs - RESUME_TAIL_SECS;
    if completed {
        (None, true)
    } else {
        (Some(position_secs), false)
    }
}

/// Which account the store belongs to right now. The primary linked account
/// scopes the file; a signed-out session shares an `anon` file. Cached for a
/// minute so a 5 s report cadence does not re-read `accounts.json` each time.
fn current_owner() -> String {
    if let Ok(guard) = OWNER_CACHE.lock() {
        if let Some((owner, at)) = guard.as_ref() {
            if at.elapsed() < OWNER_CACHE_TTL {
                return owner.clone();
            }
        }
    }
    let owner = AccountStore::primary()
        .map(|a| a.user_id)
        .filter(|id| !id.trim().is_empty())
        .unwrap_or_else(|| ANON_OWNER.to_string());
    let owner = sanitize_owner(&owner);
    if let Ok(mut guard) = OWNER_CACHE.lock() {
        *guard = Some((owner.clone(), Instant::now()));
    }
    owner
}

/// Forget the cached owner (account switch / sign-out), so the next report
/// lands in the right file.
pub fn invalidate_owner() {
    if let Ok(mut guard) = OWNER_CACHE.lock() {
        *guard = None;
    }
}

fn sanitize_owner(owner_id: &str) -> String {
    let s: String = owner_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if s.is_empty() {
        ANON_OWNER.to_string()
    } else {
        s
    }
}

fn storage_path(owner: &str) -> Result<PathBuf, String> {
    let dir = get_app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {}", e))?
        .join(STORE_DIR);
    if !dir.exists() {
        fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create vod_progress directory: {}", e))?;
    }
    Ok(dir.join(format!("{}.json", owner)))
}

fn mark_dirty() {
    ANY_DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
}

fn ensure_flush_task() {
    if FLUSH_TASK_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        if tokio::runtime::Handle::try_current().is_err() {
            FLUSH_TASK_STARTED.store(false, Ordering::SeqCst);
            return;
        }
        tokio::spawn(async {
            loop {
                tokio::time::sleep(Duration::from_secs(5)).await;
                if !ANY_DIRTY.load(Ordering::Acquire) {
                    continue;
                }
                match tokio::task::spawn_blocking(flush_now).await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => debug!("[VodProgress] debounced flush failed (will retry): {}", e),
                    Err(_) => ANY_DIRTY.store(true, Ordering::Release),
                }
            }
        });
    }
}

/// Drop entries past retention. Runs once per seed, so a long-lived session
/// never pays for it again.
fn prune_stale(file: &mut ProgressFile) -> usize {
    let cutoff = now_secs() - RETENTION_SECS;
    let before = file.entries.len();
    file.entries.retain(|_, p| p.updated_at >= cutoff);
    before - file.entries.len()
}

/// Keep the store bounded: evict the least recently updated past `CAP`.
fn enforce_cap(file: &mut ProgressFile) {
    if file.entries.len() <= CAP {
        return;
    }
    let mut by_age: Vec<(String, i64)> = file
        .entries
        .iter()
        .map(|(k, p)| (k.clone(), p.updated_at))
        .collect();
    by_age.sort_by_key(|(_, at)| *at);
    let excess = file.entries.len() - CAP;
    for (key, _) in by_age.into_iter().take(excess) {
        file.entries.remove(&key);
    }
}

fn with_owner<R>(f: impl FnOnce(&mut OwnerStore) -> R) -> Result<R, String> {
    let owner = current_owner();
    let mut guard = store().lock().map_err(|e| e.to_string())?;
    if !guard.contains_key(&owner) {
        let path = storage_path(&owner)?;
        let mut file = if path.exists() {
            let contents = fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read vod_progress file: {}", e))?;
            serde_json::from_str::<ProgressFile>(&contents).unwrap_or_else(|e| {
                debug!("[VodProgress] unreadable store, starting empty: {}", e);
                ProgressFile::default()
            })
        } else {
            ProgressFile::default()
        };
        let dropped = prune_stale(&mut file);
        let dirty = dropped > 0;
        if dropped > 0 {
            debug!("[VodProgress] pruned {} stale entries for {}", dropped, owner);
        }
        guard.insert(owner.clone(), OwnerStore { path, file, dirty });
        if dirty {
            ANY_DIRTY.store(true, Ordering::Release);
        }
    }
    Ok(f(guard.get_mut(&owner).expect("seeded above")))
}

/// Record the playhead for a VOD. Called by the player at most every few
/// seconds plus on pause / seek / end / unmount.
pub fn record(
    video_id: &str,
    position_secs: f64,
    duration_secs: f64,
    meta: VodMeta,
) -> Result<VodProgressSummary, String> {
    let video_id = video_id.trim();
    if video_id.is_empty() || !position_secs.is_finite() || position_secs < 0.0 {
        return Err("invalid VOD position report".to_string());
    }
    let duration = if duration_secs.is_finite() && duration_secs > 0.0 {
        duration_secs
    } else {
        0.0
    };
    let (_, completed) = resume_decision(position_secs, duration);
    let key = vod_key(video_id);
    let summary = with_owner(|owner| {
        let entry = owner
            .file
            .entries
            .entry(key)
            .or_insert_with(|| VodProgress {
                video_id: video_id.to_string(),
                position_secs: 0.0,
                duration_secs: 0.0,
                updated_at: 0,
                completed: false,
                channel_login: String::new(),
                title: String::new(),
                thumbnail_url: String::new(),
            });
        entry.position_secs = position_secs;
        // A recording VOD grows; never let a shorter stale hint replace a
        // longer known length.
        if duration > entry.duration_secs {
            entry.duration_secs = duration;
        }
        entry.completed = completed;
        entry.updated_at = now_secs();
        if let Some(login) = meta.channel_login.filter(|s| !s.is_empty()) {
            entry.channel_login = login.to_lowercase();
        }
        if let Some(title) = meta.title.filter(|s| !s.is_empty()) {
            entry.title = title;
        }
        if let Some(thumb) = meta.thumbnail_url.filter(|s| !s.is_empty()) {
            entry.thumbnail_url = thumb;
        }
        let summary = VodProgressSummary {
            position_secs: entry.position_secs,
            duration_secs: entry.duration_secs,
            completed: entry.completed,
        };
        enforce_cap(&mut owner.file);
        owner.dirty = true;
        summary
    })?;
    mark_dirty();
    Ok(summary)
}

/// Stored state for one VOD, if any.
pub fn lookup(video_id: &str) -> Option<VodProgress> {
    let key = vod_key(video_id);
    with_owner(|owner| owner.file.entries.get(&key).cloned())
        .ok()
        .flatten()
}

/// Stored state for several VODs at once (one lock, one seed).
pub fn lookup_many(video_ids: &[String]) -> Vec<VodProgress> {
    with_owner(|owner| {
        video_ids
            .iter()
            .filter_map(|id| owner.file.entries.get(&vod_key(id)).cloned())
            .collect()
    })
    .unwrap_or_default()
}

/// The position a VOD should open at, or None to start from the top. Applies
/// the resume policy to the stored entry.
pub fn resume_position(video_id: &str) -> Option<f64> {
    let entry = lookup(video_id)?;
    if entry.completed {
        return None;
    }
    resume_decision(entry.position_secs, entry.duration_secs).0
}

/// Join stored progress onto a list of videos so a card can draw its bar
/// without a second round trip.
pub fn attach(videos: &mut [TwitchVideo]) {
    if videos.is_empty() {
        return;
    }
    let ids: Vec<String> = videos.iter().map(|v| v.id.clone()).collect();
    let found = lookup_many(&ids);
    if found.is_empty() {
        return;
    }
    let by_id: HashMap<&str, &VodProgress> =
        found.iter().map(|p| (p.video_id.as_str(), p)).collect();
    for v in videos.iter_mut() {
        if let Some(p) = by_id.get(v.id.as_str()) {
            v.progress = Some(VodProgressSummary {
                position_secs: p.position_secs,
                duration_secs: p.duration_secs,
                completed: p.completed,
            });
        }
    }
}

/// Forget one VOD's position (the card's "start over").
pub fn clear(video_id: &str) -> Result<(), String> {
    let key = vod_key(video_id);
    let removed = with_owner(|owner| {
        let removed = owner.file.entries.remove(&key).is_some();
        if removed {
            owner.dirty = true;
        }
        removed
    })?;
    if removed {
        mark_dirty();
    }
    Ok(())
}

fn write_atomic(path: &PathBuf, json: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| format!("Failed to write vod_progress temp file: {}", e))?;
    fs::rename(&tmp, path).map_err(|e| format!("Failed to replace vod_progress file: {}", e))
}

/// Synchronous write of every dirty account file, for the debounced task and
/// the exit path. No-op when nothing changed.
pub fn flush_now() -> Result<(), String> {
    if !ANY_DIRTY.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let mut guard = match store().lock() {
        Ok(g) => g,
        Err(e) => {
            ANY_DIRTY.store(true, Ordering::Release);
            return Err(e.to_string());
        }
    };
    let mut first_err: Option<String> = None;
    for entry in guard.values_mut() {
        if !entry.dirty {
            continue;
        }
        let json = match serde_json::to_string(&entry.file) {
            Ok(j) => j,
            Err(e) => {
                first_err.get_or_insert(format!("Failed to serialize vod_progress: {}", e));
                continue;
            }
        };
        match write_atomic(&entry.path, &json) {
            Ok(()) => entry.dirty = false,
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    if guard.values().any(|s| s.dirty) {
        ANY_DIRTY.store(true, Ordering::Release);
    }
    match first_err {
        None => Ok(()),
        Some(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_positions_do_not_resume() {
        assert_eq!(resume_decision(12.0, 3600.0), (None, false));
        assert_eq!(resume_decision(0.0, 3600.0), (None, false));
        assert_eq!(resume_decision(f64::NAN, 3600.0), (None, false));
    }

    #[test]
    fn mid_video_resumes() {
        assert_eq!(resume_decision(1234.5, 3600.0), (Some(1234.5), false));
    }

    #[test]
    fn near_the_end_counts_as_finished() {
        assert_eq!(resume_decision(3550.0, 3600.0), (None, true));
        assert_eq!(resume_decision(3600.0, 3600.0), (None, true));
    }

    #[test]
    fn unknown_duration_still_resumes() {
        // A recording VOD may report 0 length; resume anyway, never "complete".
        assert_eq!(resume_decision(900.0, 0.0), (Some(900.0), false));
    }

    #[test]
    fn cap_evicts_least_recently_updated() {
        let mut file = ProgressFile::default();
        for i in 0..(CAP + 10) {
            file.entries.insert(
                vod_key(&i.to_string()),
                VodProgress {
                    video_id: i.to_string(),
                    position_secs: 100.0,
                    duration_secs: 1000.0,
                    updated_at: i as i64,
                    completed: false,
                    channel_login: String::new(),
                    title: String::new(),
                    thumbnail_url: String::new(),
                },
            );
        }
        enforce_cap(&mut file);
        assert_eq!(file.entries.len(), CAP);
        // The ten oldest (updated_at 0..9) are gone, the newest survive.
        assert!(!file.entries.contains_key(&vod_key("0")));
        assert!(!file.entries.contains_key(&vod_key("9")));
        assert!(file.entries.contains_key(&vod_key("10")));
        assert!(file.entries.contains_key(&vod_key(&(CAP + 9).to_string())));
    }

    #[test]
    fn retention_drops_stale_entries() {
        let mut file = ProgressFile::default();
        let now = now_secs();
        file.entries.insert(
            vod_key("old"),
            VodProgress {
                video_id: "old".into(),
                position_secs: 1.0,
                duration_secs: 2.0,
                updated_at: now - RETENTION_SECS - 1,
                completed: false,
                channel_login: String::new(),
                title: String::new(),
                thumbnail_url: String::new(),
            },
        );
        file.entries.insert(
            vod_key("fresh"),
            VodProgress {
                video_id: "fresh".into(),
                position_secs: 1.0,
                duration_secs: 2.0,
                updated_at: now,
                completed: false,
                channel_login: String::new(),
                title: String::new(),
                thumbnail_url: String::new(),
            },
        );
        assert_eq!(prune_stale(&mut file), 1);
        assert!(file.entries.contains_key(&vod_key("fresh")));
    }

    #[test]
    fn owner_ids_stay_filesystem_safe() {
        assert_eq!(sanitize_owner("12345"), "12345");
        assert_eq!(sanitize_owner("../evil"), "evil");
        assert_eq!(sanitize_owner(""), ANON_OWNER);
    }
}
