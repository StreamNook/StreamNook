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
//! This is the canonical copy; `vod_sync` reconciles it with the viewer's
//! Twitch viewing history (newest save wins, `synced_at` marks what Twitch
//! has seen, `dismissed` keeps a pull from reviving a card the viewer closed).
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
/// Live bookmarks kept at most, and evicted before any VOD entry, so a heavy
/// live watcher never pushes out positions in VODs they chose to open.
const LIVE_CAP: usize = 60;
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
/// What wrote an entry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// A VOD the viewer chose to open. The only kind that is a Continue
    /// Watching card or syncs to Twitch.
    #[default]
    Vod,
    /// Where the viewer left a live broadcast (`live_bookmark`). Shows on that
    /// recording only, and becomes `Vod` once the viewer plays it.
    Live,
}

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
    /// Display name, so a card is not forced to show a bare lowercase login.
    /// Falls back to `channel_login` when unknown.
    #[serde(default)]
    pub channel_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub thumbnail_url: String,
    /// Channel avatar, filled by the batched hydrate (the player never
    /// has it at report time). Empty until then.
    #[serde(default)]
    pub profile_image_url: String,
    /// Twitch partner, which is what the verified mark on a card means.
    #[serde(default)]
    pub partner: bool,
    /// Category of the broadcast, from the hydrate. Empty until then.
    #[serde(default)]
    pub game_name: String,
    /// The channel's Twitch user id, from the hydrate, so a card can draw the
    /// channel's 7TV picture. Empty until then.
    #[serde(default)]
    pub channel_id: String,
    /// The `updated_at` this entry last matched on Twitch (pushed or pulled).
    /// Behind `updated_at` means this device has a position Twitch has not
    /// seen yet. 0 for entries written before sync existed.
    #[serde(default)]
    pub synced_at: i64,
    /// Entries from before live bookmarks existed were all deliberate opens.
    #[serde(default)]
    pub source: Source,
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
    pub channel_name: Option<String>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
}

/// A fresh metadata read for one stored VOD, from the batched GQL hydrate that
/// keeps the Continue Watching row honest.
#[derive(Debug, Clone)]
pub struct VodRefresh {
    pub video_id: String,
    pub title: String,
    pub channel_name: String,
    pub thumbnail_url: String,
    pub length_secs: f64,
    /// The broadcast has finished, so `length_secs` is final and completion
    /// can finally be judged against it.
    pub finished: bool,
    pub profile_image_url: String,
    /// None when the owner node was missing, so an unknown never clears
    /// a mark the store already holds.
    pub partner: Option<bool>,
    pub game_name: String,
    /// Empty when the owner node was missing.
    pub channel_id: String,
}

/// A thumbnail URL Twitch serves while a VOD is still being processed. Storing
/// it would freeze a placeholder image into the card (verified 2026-09-12: a
/// RECORDING archive returns `404_processing_440x248.png`).
fn is_placeholder_thumb(url: &str) -> bool {
    url.contains("404_processing")
}

/// A watch position read from Twitch's viewing history, with the card fields
/// the same answer carried.
#[derive(Debug, Clone)]
pub struct RemoteProgress {
    pub position_secs: f64,
    /// Unix seconds Twitch last saved the position.
    pub updated_at: i64,
    pub channel_login: String,
    pub meta: VodRefresh,
}

#[derive(Debug, Serialize, Deserialize)]
struct ProgressFile {
    version: u32,
    entries: HashMap<String, VodProgress>,
    /// Entries the viewer dismissed or started over, keyed like `entries`, with
    /// the Unix second it happened. Twitch has no way to delete a position, so
    /// this is what keeps a pull from bringing the card straight back. A newer
    /// position from another device (watched again there) wins over it.
    #[serde(default)]
    dismissed: HashMap<String, i64>,
    /// How far this account's changes have reached StreamNook's account copy.
    #[serde(default)]
    cloud: CloudCursor,
}

impl Default for ProgressFile {
    fn default() -> Self {
        Self {
            version: FILE_VERSION,
            entries: HashMap::new(),
            dismissed: HashMap::new(),
            cloud: CloudCursor::default(),
        }
    }
}

/// Where the StreamNook account sync stands for one account's file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CloudCursor {
    /// Local Unix second through which this device's changes (positions and
    /// dismissals) have been sent.
    #[serde(default)]
    pushed_through: i64,
    /// StreamNook's clock at the last answer (RFC 3339), sent back as `since`.
    #[serde(default)]
    pulled_through: Option<String>,
}

/// One change bound for StreamNook's account copy: a position, or a dismissal.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudChange {
    pub video_id: String,
    pub position_secs: f64,
    pub duration_secs: f64,
    /// Unix seconds: the report time, or when the card was dismissed.
    pub at: i64,
    pub dismissed: bool,
    pub channel_login: String,
    pub channel_name: String,
    pub title: String,
    pub thumbnail_url: String,
}

/// What a sync sends, and where the cursor moves once it lands.
#[derive(Debug, Clone)]
pub struct CloudOutbox {
    pub changes: Vec<CloudChange>,
    /// `pushed_through` once these are accepted.
    pub cutoff: i64,
    pub since: Option<String>,
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

/// The signed-in account the store currently belongs to, or None when it is the
/// shared signed-out store (which never syncs anywhere).
pub fn owner_id() -> Option<String> {
    Some(current_owner()).filter(|o| o != ANON_OWNER)
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
    // A pull ignores positions older than retention anyway, so an older
    // dismissal guards nothing.
    let marks = file.dismissed.len();
    file.dismissed.retain(|_, at| *at >= cutoff);
    before - file.entries.len() + (marks - file.dismissed.len())
}

/// Keep the store bounded: evict the least recently updated past `CAP`.
fn enforce_cap(file: &mut ProgressFile) {
    if file.dismissed.len() > CAP {
        let mut marks: Vec<(String, i64)> =
            file.dismissed.iter().map(|(k, at)| (k.clone(), *at)).collect();
        marks.sort_by_key(|(_, at)| *at);
        let excess = marks.len() - CAP;
        for (key, _) in marks.into_iter().take(excess) {
            file.dismissed.remove(&key);
        }
    }
    let live = file.entries.values().filter(|p| p.source == Source::Live).count();
    if file.entries.len() <= CAP && live <= LIVE_CAP {
        return;
    }
    // Live bookmarks go first (oldest first), then the oldest of the rest.
    let mut by_age: Vec<(String, bool, i64)> = file
        .entries
        .iter()
        .map(|(k, p)| (k.clone(), p.source == Source::Live, p.updated_at))
        .collect();
    by_age.sort_by_key(|(_, is_live, at)| (!*is_live, *at));
    let mut live_excess = live.saturating_sub(LIVE_CAP);
    let mut excess = file.entries.len().saturating_sub(CAP);
    for (key, is_live, _) in by_age {
        if live_excess == 0 && excess == 0 {
            break;
        }
        if is_live && live_excess > 0 {
            live_excess -= 1;
        } else if excess == 0 {
            continue;
        }
        file.entries.remove(&key);
        excess = excess.saturating_sub(1);
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
                channel_name: String::new(),
                title: String::new(),
                thumbnail_url: String::new(),
                profile_image_url: String::new(),
                partner: false,
                game_name: String::new(),
                channel_id: String::new(),
                synced_at: 0,
                source: Source::Vod,
            });
        entry.position_secs = position_secs;
        // Playing it is deliberate, even if a live session filed it.
        entry.source = Source::Vod;
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
        if let Some(name) = meta.channel_name.filter(|s| !s.is_empty()) {
            entry.channel_name = name;
        }
        if let Some(title) = meta.title.filter(|s| !s.is_empty()) {
            entry.title = title;
        }
        if let Some(thumb) = meta
            .thumbnail_url
            .filter(|s| !s.is_empty() && !is_placeholder_thumb(s))
        {
            entry.thumbnail_url = thumb;
        }
        let summary = VodProgressSummary {
            position_secs: entry.position_secs,
            duration_secs: entry.duration_secs,
            completed: entry.completed,
        };
        // Watching it again undoes a dismissal.
        owner.file.dismissed.remove(&vod_key(video_id));
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

/// Does this entry still have somewhere worth resuming to? The predicate
/// behind Home's Continue Watching row, pulled out so it can be tested
/// without a filesystem.
pub fn is_resumable(entry: &VodProgress) -> bool {
    !entry.completed && entry.position_secs >= RESUME_MIN_SECS
}

/// Is this entry a Continue Watching card? Only a deliberately opened VOD:
/// a live bookmark shows on its recording alone.
pub fn is_card(entry: &VodProgress) -> bool {
    entry.source == Source::Vod && is_resumable(entry)
}

/// Order the row: newest watch first, capped. Ties break on the id so the
/// order is stable, which matters because the Home snapshot decides whether to
/// emit by comparing serialized content and a reshuffle would read as a change.
/// Split out from `recent` so it can be tested without a filesystem.
fn rank(rows: &mut Vec<VodProgress>, limit: usize) {
    rows.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.video_id.cmp(&b.video_id))
    });
    rows.truncate(limit);
}

/// Most recently watched VODs that still have somewhere to resume, newest
/// first. Reads the local store only, so the Continue Watching row paints with
/// no Twitch call and survives a cold start.
pub fn recent(limit: usize) -> Vec<VodProgress> {
    if limit == 0 {
        return Vec::new();
    }
    with_owner(|owner| {
        let mut rows: Vec<VodProgress> = owner
            .file
            .entries
            .values()
            .filter(|p| is_card(p))
            .cloned()
            .collect();
        rank(&mut rows, limit);
        rows
    })
    .unwrap_or_default()
}

/// Fold a batched hydrate back into the store: refresh metadata and length for
/// `updates`, forget `gone` (Twitch deleted those VODs), and recompute
/// `completed` for finished videos now that their length is final. One lock
/// for the whole batch. Returns true when anything changed.
///
/// `updated_at` is deliberately untouched: it is watch recency, and the row
/// sorts on it.
pub fn reconcile(updates: &[VodRefresh], gone: &[String]) -> bool {
    if updates.is_empty() && gone.is_empty() {
        return false;
    }
    let changed = with_owner(|owner| {
        let mut changed = false;
        for id in gone {
            if owner.file.entries.remove(&vod_key(id)).is_some() {
                debug!("[VodProgress] dropping {} (deleted on Twitch)", id);
                changed = true;
            }
        }
        for up in updates {
            let Some(entry) = owner.file.entries.get_mut(&vod_key(&up.video_id)) else {
                continue;
            };
            if !up.title.is_empty() && entry.title != up.title {
                entry.title = up.title.clone();
                changed = true;
            }
            if !up.channel_name.is_empty() && entry.channel_name != up.channel_name {
                entry.channel_name = up.channel_name.clone();
                changed = true;
            }
            if !up.thumbnail_url.is_empty()
                && !is_placeholder_thumb(&up.thumbnail_url)
                && entry.thumbnail_url != up.thumbnail_url
            {
                entry.thumbnail_url = up.thumbnail_url.clone();
                changed = true;
            }
            if !up.profile_image_url.is_empty() && entry.profile_image_url != up.profile_image_url {
                entry.profile_image_url = up.profile_image_url.clone();
                changed = true;
            }
            if !up.channel_id.is_empty() && entry.channel_id != up.channel_id {
                entry.channel_id = up.channel_id.clone();
                changed = true;
            }
            if let Some(partner) = up.partner {
                if entry.partner != partner {
                    entry.partner = partner;
                    changed = true;
                }
            }
            if !up.game_name.is_empty() && entry.game_name != up.game_name {
                entry.game_name = up.game_name.clone();
                changed = true;
            }
            // Same ratchet as `record`: a recording VOD only ever grows.
            if up.length_secs > entry.duration_secs {
                entry.duration_secs = up.length_secs;
                changed = true;
            }
            // Completion is only meaningful against a final length, which is
            // exactly what `finished` promises.
            if up.finished {
                let done = resume_decision(entry.position_secs, entry.duration_secs).1;
                if entry.completed != done {
                    entry.completed = done;
                    changed = true;
                }
            }
        }
        if changed {
            owner.dirty = true;
        }
        changed
    })
    .unwrap_or(false);
    if changed {
        mark_dirty();
    }
    changed
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
            owner.file.dismissed.insert(key, now_secs());
            enforce_cap(&mut owner.file);
            owner.dirty = true;
        }
        removed
    })?;
    if removed {
        mark_dirty();
    }
    Ok(())
}

/// Where a remote position came from, which decides what it may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteOrigin {
    /// Twitch's recently-viewed list. It holds every broadcast the viewer
    /// watched LIVE as well as the VODs they opened, with nothing to tell the
    /// two apart, so a list row may only move a card this device already shows.
    /// Creating from it filled the row with every stream ever watched.
    List,
    /// Deliberate: the viewer just opened this VOD here, or opened it on another
    /// of their StreamNook installs (StreamNook's account copy holds only
    /// deliberate VOD watches). It may create the entry, or turn a live
    /// bookmark into a VOD entry.
    Opened,
}

/// Fold one remote position into a file. Pure over the file so it can be
/// tested without a filesystem. Returns true when the entry changed.
///
/// Last writer wins on `updated_at`. A remote row never revives a dismissed
/// entry unless it was saved after the dismissal, and never creates an entry
/// that would not be worth resuming. See `RemoteOrigin` for what a list row
/// may not do.
fn merge_one(file: &mut ProgressFile, row: &RemoteProgress, now: i64, origin: RemoteOrigin) -> bool {
    let id = row.meta.video_id.trim();
    if id.is_empty() || !row.position_secs.is_finite() || row.position_secs < 0.0 {
        return false;
    }
    if row.updated_at < now - RETENTION_SECS {
        return false;
    }
    let key = vod_key(id);
    if origin == RemoteOrigin::List
        && !file.entries.get(&key).is_some_and(|e| e.source == Source::Vod)
    {
        return false;
    }
    if let Some(&dismissed_at) = file.dismissed.get(&key) {
        if row.updated_at <= dismissed_at {
            return false;
        }
        file.dismissed.remove(&key);
    }
    let length = if row.meta.length_secs.is_finite() {
        row.meta.length_secs.max(0.0)
    } else {
        0.0
    };
    if let Some(entry) = file.entries.get_mut(&key) {
        if entry.updated_at >= row.updated_at {
            // This device's position is the newer one; the push carries it.
            return false;
        }
        if (entry.position_secs - row.position_secs).abs() < 2.0 {
            // Our own push echoed back: same position, nothing to adopt.
            if entry.synced_at < entry.updated_at {
                entry.synced_at = entry.updated_at;
                return true;
            }
            return false;
        }
    } else if row.position_secs < RESUME_MIN_SECS {
        return false;
    }
    let entry = file.entries.entry(key).or_insert_with(|| VodProgress {
        video_id: id.to_string(),
        position_secs: 0.0,
        duration_secs: 0.0,
        updated_at: 0,
        completed: false,
        channel_login: String::new(),
        channel_name: String::new(),
        title: String::new(),
        thumbnail_url: String::new(),
        profile_image_url: String::new(),
        partner: false,
        game_name: String::new(),
        channel_id: String::new(),
        synced_at: 0,
        source: Source::Vod,
    });
    // Watched as a VOD on another device or app.
    entry.source = Source::Vod;
    entry.position_secs = row.position_secs;
    entry.updated_at = row.updated_at;
    entry.synced_at = row.updated_at;
    if length > entry.duration_secs {
        entry.duration_secs = length;
    }
    entry.completed = resume_decision(entry.position_secs, entry.duration_secs).1;
    if !row.channel_login.is_empty() {
        entry.channel_login = row.channel_login.to_lowercase();
    }
    let m = &row.meta;
    if !m.title.is_empty() {
        entry.title = m.title.clone();
    }
    if !m.channel_name.is_empty() {
        entry.channel_name = m.channel_name.clone();
    }
    if !m.thumbnail_url.is_empty() && !is_placeholder_thumb(&m.thumbnail_url) {
        entry.thumbnail_url = m.thumbnail_url.clone();
    }
    if !m.profile_image_url.is_empty() {
        entry.profile_image_url = m.profile_image_url.clone();
    }
    if let Some(partner) = m.partner {
        entry.partner = partner;
    }
    if !m.game_name.is_empty() {
        entry.game_name = m.game_name.clone();
    }
    if !m.channel_id.is_empty() {
        entry.channel_id = m.channel_id.clone();
    }
    true
}

/// Fold positions read from Twitch into the signed-in account's store, one
/// lock for the batch. Returns true when anything changed.
pub fn merge_remote(rows: &[RemoteProgress], origin: RemoteOrigin) -> bool {
    if rows.is_empty() {
        return false;
    }
    let now = now_secs();
    let changed = with_owner(|owner| {
        let mut changed = false;
        for row in rows {
            changed |= merge_one(&mut owner.file, row, now, origin);
        }
        if changed {
            enforce_cap(&mut owner.file);
            owner.dirty = true;
        }
        changed
    })
    .unwrap_or(false);
    if changed {
        mark_dirty();
    }
    changed
}

/// Twitch confirmed it holds the position this device recorded at
/// `updated_at`. A report that landed since then stays unsynced.
pub fn mark_synced(video_id: &str, updated_at: i64) {
    let key = vod_key(video_id);
    let changed = with_owner(|owner| match owner.file.entries.get_mut(&key) {
        Some(entry) if entry.synced_at < updated_at => {
            entry.synced_at = updated_at.min(entry.updated_at);
            owner.dirty = true;
            true
        }
        _ => false,
    })
    .unwrap_or(false);
    if changed {
        mark_dirty();
    }
}

/// Whether an entry's position may leave the device for the viewer's Twitch
/// history. Only a deliberately opened VOD may. A live bookmark must answer
/// false here, or the sync files it as a VOD position on every device the
/// viewer owns.
pub fn may_sync(entry: &VodProgress) -> bool {
    entry.source == Source::Vod
}

/// Fold a live bookmark into a file. Pure so it can be tested. A deliberate
/// entry is left exactly as it is: live watching never changes the rail.
fn put_live(file: &mut ProgressFile, video_id: &str, position_secs: f64, channel_login: &str, now: i64) -> bool {
    let key = vod_key(video_id);
    if file.entries.get(&key).is_some_and(|e| e.source == Source::Vod) {
        return false;
    }
    let entry = file.entries.entry(key).or_insert_with(|| VodProgress {
        video_id: video_id.to_string(),
        position_secs: 0.0,
        duration_secs: 0.0,
        updated_at: 0,
        completed: false,
        channel_login: String::new(),
        channel_name: String::new(),
        title: String::new(),
        thumbnail_url: String::new(),
        profile_image_url: String::new(),
        partner: false,
        game_name: String::new(),
        channel_id: String::new(),
        synced_at: 0,
        source: Source::Live,
    });
    entry.position_secs = position_secs;
    // A recording still growing has no final length, so a live bookmark is
    // never "finished"; the VOD's own length decides that when it is opened.
    entry.completed = false;
    entry.updated_at = now;
    if !channel_login.is_empty() {
        entry.channel_login = channel_login.to_lowercase();
    }
    true
}

/// File where the viewer left a live broadcast, on its recording.
pub fn record_live(video_id: &str, position_secs: f64, channel_login: &str) -> Result<(), String> {
    let video_id = video_id.trim();
    if video_id.is_empty() || !position_secs.is_finite() || position_secs < 0.0 {
        return Err("invalid live bookmark".to_string());
    }
    let changed = with_owner(|owner| {
        let changed = put_live(&mut owner.file, video_id, position_secs, channel_login, now_secs());
        if changed {
            enforce_cap(&mut owner.file);
            owner.dirty = true;
        }
        changed
    })?;
    if changed {
        mark_dirty();
    }
    Ok(())
}

/// Forget a live bookmark (its broadcast was watched to the end). A
/// deliberate entry stays.
pub fn remove_live(video_id: &str) -> Result<(), String> {
    let key = vod_key(video_id);
    let removed = with_owner(|owner| {
        let live = owner.file.entries.get(&key).is_some_and(|e| e.source == Source::Live);
        if live {
            owner.file.entries.remove(&key);
            owner.dirty = true;
        }
        live
    })?;
    if removed {
        mark_dirty();
    }
    Ok(())
}

/// The changes since the last sync, oldest first, at most `limit`. Pure over
/// the file so it can be tested. Only `may_sync` positions go: a live bookmark
/// never leaves the device. A truncated batch moves the cursor only to just
/// before its newest change (a resend of a same-second change is harmless),
/// so nothing older is ever skipped.
fn outbox_of(file: &ProgressFile, limit: usize, now: i64) -> CloudOutbox {
    let since = file.cloud.pushed_through;
    let mut changes: Vec<CloudChange> = file
        .entries
        .values()
        .filter(|e| e.updated_at > since && may_sync(e))
        .map(|e| CloudChange {
            video_id: e.video_id.clone(),
            position_secs: e.position_secs,
            duration_secs: e.duration_secs,
            at: e.updated_at,
            dismissed: false,
            channel_login: e.channel_login.clone(),
            channel_name: e.channel_name.clone(),
            title: e.title.clone(),
            thumbnail_url: e.thumbnail_url.clone(),
        })
        .chain(file.dismissed.iter().filter(|(_, at)| **at > since).filter_map(|(key, at)| {
            Some(CloudChange {
                video_id: key.strip_prefix("twitch:")?.to_string(),
                position_secs: 0.0,
                duration_secs: 0.0,
                at: *at,
                dismissed: true,
                channel_login: String::new(),
                channel_name: String::new(),
                title: String::new(),
                thumbnail_url: String::new(),
            })
        }))
        .collect();
    changes.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.video_id.cmp(&b.video_id)));
    let cutoff = if changes.len() > limit {
        changes.truncate(limit);
        changes.last().map(|c| c.at - 1).unwrap_or(since).max(since)
    } else {
        now - 1
    };
    CloudOutbox {
        changes,
        cutoff,
        since: file.cloud.pulled_through.clone(),
    }
}

/// What the next StreamNook sync should send.
pub fn cloud_outbox(limit: usize) -> Option<CloudOutbox> {
    let now = now_secs();
    with_owner(|owner| outbox_of(&owner.file, limit, now)).ok()
}

/// A sync landed: move both cursors.
pub fn cloud_commit(cutoff: i64, pulled_through: String) {
    let changed = with_owner(|owner| {
        let cursor = &mut owner.file.cloud;
        cursor.pushed_through = cursor.pushed_through.max(cutoff);
        cursor.pulled_through = Some(pulled_through);
        owner.dirty = true;
    })
    .is_ok();
    if changed {
        mark_dirty();
    }
}

/// Apply one card dismissal made on another device. Pure over the file. A
/// position recorded here after the dismissal wins over it.
fn dismiss_one(file: &mut ProgressFile, video_id: &str, at: i64) -> bool {
    let key = vod_key(video_id);
    if let Some(entry) = file.entries.get(&key) {
        if entry.updated_at > at || entry.source != Source::Vod {
            return false;
        }
        file.entries.remove(&key);
    }
    let mark = file.dismissed.entry(key).or_insert(0);
    if *mark >= at {
        return false;
    }
    *mark = at;
    true
}

/// Fold dismissals from another device into the store. Returns true when any
/// card left.
pub fn apply_remote_dismissals(dismissals: &[(String, i64)]) -> bool {
    if dismissals.is_empty() {
        return false;
    }
    let changed = with_owner(|owner| {
        let mut changed = false;
        for (id, at) in dismissals {
            changed |= dismiss_one(&mut owner.file, id, *at);
        }
        if changed {
            enforce_cap(&mut owner.file);
            owner.dirty = true;
        }
        changed
    })
    .unwrap_or(false);
    if changed {
        mark_dirty();
    }
    changed
}

/// The row's entries holding a position Twitch has not seen: watched while
/// sync was off or offline, or the last report before the app quit.
pub fn unsynced(limit: usize) -> Vec<VodProgress> {
    recent(limit)
        .into_iter()
        .filter(|p| p.synced_at < p.updated_at)
        .collect()
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

    /// A bare stored entry. One place to add a field, so a new one does not
    /// break every test that happens to need an entry.
    fn entry(video_id: &str, position_secs: f64, duration_secs: f64, updated_at: i64) -> VodProgress {
        VodProgress {
            video_id: video_id.to_string(),
            position_secs,
            duration_secs,
            updated_at,
            completed: false,
            channel_login: String::new(),
            channel_name: String::new(),
            title: String::new(),
            thumbnail_url: String::new(),
            profile_image_url: String::new(),
            partner: false,
            game_name: String::new(),
            channel_id: String::new(),
            synced_at: 0,
            source: Source::Vod,
        }
    }

    fn live_entry(video_id: &str, updated_at: i64) -> VodProgress {
        VodProgress { source: Source::Live, ..entry(video_id, 900.0, 0.0, updated_at) }
    }

    #[test]
    fn a_live_bookmark_is_never_a_card_and_never_syncs() {
        let live = live_entry("v", 1);
        assert!(is_resumable(&live));
        assert!(!is_card(&live));
        assert!(!may_sync(&live));
        let vod = entry("w", 900.0, 3600.0, 1);
        assert!(is_card(&vod));
        assert!(may_sync(&vod));
    }

    #[test]
    fn a_live_write_leaves_a_deliberate_entry_alone() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("v"), entry("v", 1200.0, 3600.0, 5));
        assert!(!put_live(&mut file, "v", 3000.0, "caedrel", 9));
        let e = &file.entries[&vod_key("v")];
        assert_eq!((e.position_secs, e.updated_at, e.source), (1200.0, 5, Source::Vod));
    }

    #[test]
    fn a_live_write_files_an_unfinished_bookmark() {
        let mut file = ProgressFile::default();
        assert!(put_live(&mut file, "v", 3000.0, "CaEdReL", 9));
        assert!(put_live(&mut file, "v", 4000.0, "caedrel", 10));
        let e = &file.entries[&vod_key("v")];
        assert_eq!((e.position_secs, e.updated_at, e.source), (4000.0, 10, Source::Live));
        assert!(!e.completed);
        assert_eq!(e.channel_login, "caedrel");
    }

    #[test]
    fn a_pulled_position_turns_a_live_bookmark_into_a_vod_entry() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("v"), live_entry("v", 5));
        assert!(merge_one(&mut file, &remote("v", 1500.0, 50), 60, RemoteOrigin::Opened));
        assert_eq!(file.entries[&vod_key("v")].source, Source::Vod);
    }

    #[test]
    fn a_list_row_never_promotes_a_live_bookmark() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("v"), live_entry("v", 5));
        assert!(!merge_one(&mut file, &remote("v", 1500.0, 50), 60, RemoteOrigin::List));
        assert_eq!(file.entries[&vod_key("v")].source, Source::Live);
    }

    fn cloud_ids(o: &CloudOutbox) -> Vec<(&str, bool)> {
        o.changes.iter().map(|c| (c.video_id.as_str(), c.dismissed)).collect()
    }

    #[test]
    fn the_outbox_sends_positions_and_dismissals_since_the_cursor_oldest_first() {
        let mut file = ProgressFile::default();
        file.cloud.pushed_through = NOW - 50;
        file.entries.insert(vod_key("old"), entry("old", 900.0, 7200.0, NOW - 60));
        file.entries.insert(vod_key("new"), entry("new", 900.0, 7200.0, NOW - 10));
        file.dismissed.insert(vod_key("gone"), NOW - 20);
        file.dismissed.insert(vod_key("stale"), NOW - 70);
        let o = outbox_of(&file, 50, NOW);
        assert_eq!(cloud_ids(&o), vec![("gone", true), ("new", false)]);
        assert_eq!(o.cutoff, NOW - 1);
    }

    #[test]
    fn a_live_bookmark_never_reaches_the_outbox() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("v"), live_entry("v", NOW - 5));
        assert!(outbox_of(&file, 50, NOW).changes.is_empty());
    }

    #[test]
    fn a_truncated_outbox_never_skips_an_older_change() {
        let mut file = ProgressFile::default();
        for i in 0..5 {
            let id = format!("{i}");
            file.entries.insert(vod_key(&id), entry(&id, 900.0, 7200.0, NOW - 100 + i));
        }
        let o = outbox_of(&file, 2, NOW);
        assert_eq!(cloud_ids(&o), vec![("0", false), ("1", false)]);
        assert_eq!(o.cutoff, NOW - 100);
        file.cloud.pushed_through = o.cutoff;
        let next = outbox_of(&file, 50, NOW);
        assert_eq!(next.changes.first().map(|c| c.video_id.as_str()), Some("1"));
        assert_eq!(next.changes.len(), 4);
    }

    #[test]
    fn a_dismissal_from_another_device_removes_the_card() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("1"), entry("1", 900.0, 7200.0, NOW - 100));
        assert!(dismiss_one(&mut file, "1", NOW - 10));
        assert!(file.entries.is_empty());
        assert_eq!(file.dismissed[&vod_key("1")], NOW - 10);
        // The same dismissal arriving again changes nothing.
        assert!(!dismiss_one(&mut file, "1", NOW - 10));
    }

    #[test]
    fn watching_here_after_a_remote_dismissal_keeps_the_card() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("1"), entry("1", 900.0, 7200.0, NOW - 5));
        assert!(!dismiss_one(&mut file, "1", NOW - 10));
        assert!(file.entries.contains_key(&vod_key("1")));
    }

    #[test]
    fn a_remote_dismissal_leaves_a_live_bookmark_alone() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("v"), live_entry("v", 5));
        assert!(!dismiss_one(&mut file, "v", 50));
        assert!(file.entries.contains_key(&vod_key("v")));
    }

    #[test]
    fn a_list_row_never_creates_a_card() {
        let mut file = ProgressFile::default();
        assert!(!merge_one(&mut file, &remote("1", 900.0, NOW - 10), NOW, RemoteOrigin::List));
        assert!(file.entries.is_empty());
    }

    #[test]
    fn a_list_row_moves_a_card_this_device_already_shows() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("1"), entry("1", 600.0, 7200.0, NOW - 100));
        assert!(merge_one(&mut file, &remote("1", 3000.0, NOW - 10), NOW, RemoteOrigin::List));
        assert_eq!(file.entries[&vod_key("1")].position_secs, 3000.0);
    }

    #[test]
    fn live_bookmarks_have_their_own_cap_and_go_first() {
        let mut file = ProgressFile::default();
        for i in 0..(LIVE_CAP + 5) {
            file.entries.insert(vod_key(&format!("l{i}")), live_entry(&format!("l{i}"), 1000 + i as i64));
        }
        file.entries.insert(vod_key("old-vod"), entry("old-vod", 900.0, 3600.0, 1));
        enforce_cap(&mut file);
        let live = file.entries.values().filter(|p| p.source == Source::Live).count();
        assert_eq!(live, LIVE_CAP);
        // The oldest live ones went; the older VOD entry stayed.
        assert!(!file.entries.contains_key(&vod_key("l0")));
        assert!(file.entries.contains_key(&vod_key("old-vod")));
    }

    #[test]
    fn over_the_total_cap_live_bookmarks_are_evicted_before_vods() {
        let mut file = ProgressFile::default();
        for i in 0..CAP {
            file.entries.insert(vod_key(&i.to_string()), entry(&i.to_string(), 100.0, 1000.0, i as i64));
        }
        file.entries.insert(vod_key("fresh-live"), live_entry("fresh-live", 1_000_000));
        enforce_cap(&mut file);
        assert_eq!(file.entries.len(), CAP);
        assert!(!file.entries.contains_key(&vod_key("fresh-live")));
        assert!(file.entries.contains_key(&vod_key("0")));
    }

    #[test]
    fn a_store_written_before_live_bookmarks_loads_as_vod_entries() {
        let json = r#"{"version":1,"entries":{"twitch:1":{"video_id":"1","position_secs":900.0,"duration_secs":7200.0,"updated_at":5,"completed":false}}}"#;
        let file: ProgressFile = serde_json::from_str(json).unwrap();
        assert_eq!(file.entries[&vod_key("1")].source, Source::Vod);
    }

    fn remote(video_id: &str, position_secs: f64, updated_at: i64) -> RemoteProgress {
        RemoteProgress {
            position_secs,
            updated_at,
            channel_login: "Caedrel".to_string(),
            meta: VodRefresh {
                video_id: video_id.to_string(),
                title: "Remote title".to_string(),
                channel_name: "Caedrel".to_string(),
                thumbnail_url: String::new(),
                length_secs: 7200.0,
                finished: true,
                profile_image_url: String::new(),
                partner: Some(true),
                game_name: String::new(),
                channel_id: "92038375".to_string(),
            },
        }
    }

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn a_newer_remote_position_is_adopted_with_its_card_fields() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("1"), entry("1", 600.0, 7200.0, NOW - 100));
        assert!(merge_one(&mut file, &remote("1", 3000.0, NOW - 10), NOW, RemoteOrigin::Opened));
        let e = &file.entries[&vod_key("1")];
        assert_eq!(e.position_secs, 3000.0);
        assert_eq!(e.updated_at, NOW - 10);
        assert_eq!(e.synced_at, NOW - 10);
        assert_eq!(e.channel_login, "caedrel");
        assert_eq!(e.title, "Remote title");
        assert!(e.partner);
    }

    #[test]
    fn a_newer_local_position_is_kept() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("1"), entry("1", 600.0, 7200.0, NOW - 5));
        assert!(!merge_one(&mut file, &remote("1", 3000.0, NOW - 50), NOW, RemoteOrigin::Opened));
        assert_eq!(file.entries[&vod_key("1")].position_secs, 600.0);
    }

    #[test]
    fn our_own_push_echoing_back_only_marks_the_entry_synced() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("1"), entry("1", 600.4, 7200.0, NOW - 20));
        assert!(merge_one(&mut file, &remote("1", 600.0, NOW - 10), NOW, RemoteOrigin::Opened));
        let e = &file.entries[&vod_key("1")];
        assert_eq!(e.position_secs, 600.4);
        assert_eq!(e.updated_at, NOW - 20);
        assert_eq!(e.synced_at, NOW - 20);
        // Already synced: a second echo changes nothing.
        assert!(!merge_one(&mut file, &remote("1", 600.0, NOW - 10), NOW, RemoteOrigin::Opened));
    }

    #[test]
    fn a_remote_position_too_short_to_resume_never_creates_an_entry() {
        let mut file = ProgressFile::default();
        assert!(!merge_one(&mut file, &remote("1", 12.0, NOW - 10), NOW, RemoteOrigin::Opened));
        assert!(file.entries.is_empty());
        assert!(merge_one(&mut file, &remote("2", 900.0, NOW - 10), NOW, RemoteOrigin::Opened));
        assert!(is_resumable(&file.entries[&vod_key("2")]));
    }

    #[test]
    fn a_dismissal_holds_until_the_vod_is_watched_again_elsewhere() {
        let mut file = ProgressFile::default();
        file.dismissed.insert(vod_key("1"), NOW - 100);
        assert!(!merge_one(&mut file, &remote("1", 900.0, NOW - 200), NOW, RemoteOrigin::Opened));
        assert!(!merge_one(&mut file, &remote("1", 900.0, NOW - 100), NOW, RemoteOrigin::Opened));
        assert!(file.entries.is_empty());
        assert!(merge_one(&mut file, &remote("1", 900.0, NOW - 50), NOW, RemoteOrigin::Opened));
        assert!(file.dismissed.is_empty());
    }

    #[test]
    fn a_remote_position_at_the_end_arrives_as_watched() {
        let mut file = ProgressFile::default();
        file.entries.insert(vod_key("1"), entry("1", 600.0, 7200.0, NOW - 100));
        assert!(merge_one(&mut file, &remote("1", 7190.0, NOW - 10), NOW, RemoteOrigin::Opened));
        let e = &file.entries[&vod_key("1")];
        assert!(e.completed);
        assert!(!is_resumable(e));
    }

    #[test]
    fn a_remote_position_past_retention_is_ignored() {
        let mut file = ProgressFile::default();
        assert!(!merge_one(&mut file, &remote("1", 900.0, NOW - RETENTION_SECS - 1), NOW, RemoteOrigin::Opened));
        assert!(file.entries.is_empty());
    }

    #[test]
    fn a_store_written_before_sync_still_loads() {
        let json = r#"{"version":1,"entries":{"twitch:1":{"video_id":"1","position_secs":900.0,"duration_secs":7200.0,"updated_at":5,"completed":false}}}"#;
        let file: ProgressFile = serde_json::from_str(json).unwrap();
        assert!(file.dismissed.is_empty());
        assert_eq!(file.entries[&vod_key("1")].synced_at, 0);
    }

    #[test]
    fn only_unfinished_entries_past_the_floor_are_resumable() {
        // Mirrors the 30 s floor the resume policy uses.
        assert!(is_resumable(&entry("a", 900.0, 3600.0, 1)));
        assert!(!is_resumable(&entry("b", 12.0, 3600.0, 1)));
        let mut done = entry("c", 900.0, 3600.0, 1);
        done.completed = true;
        assert!(!is_resumable(&done));
        // A recording VOD reports no duration and must still qualify.
        assert!(is_resumable(&entry("d", 900.0, 0.0, 1)));
    }

    #[test]
    fn the_row_ranks_newest_watch_first_with_a_stable_tie_break() {
        let mut rows = vec![
            entry("b", 100.0, 1000.0, 50),
            entry("a", 100.0, 1000.0, 50),
            entry("c", 100.0, 1000.0, 99),
        ];
        rank(&mut rows, 24);
        // Newest first, and equal timestamps fall back to the id so `same()`
        // never sees a reshuffle of identical content as a change.
        assert_eq!(
            rows.iter().map(|r| r.video_id.as_str()).collect::<Vec<_>>(),
            vec!["c", "a", "b"]
        );
    }

    #[test]
    fn the_row_honours_its_limit() {
        let mut rows: Vec<VodProgress> = (0..10)
            .map(|i| entry(&i.to_string(), 100.0, 1000.0, i as i64))
            .collect();
        rank(&mut rows, 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].video_id, "9");
    }

    #[test]
    fn a_processing_thumbnail_is_never_stored() {
        assert!(is_placeholder_thumb(
            "https://vod-secure.twitch.tv/_404/404_processing_440x248.png"
        ));
        assert!(!is_placeholder_thumb(
            "https://static-cdn.jtvnw.net/cf_vods/abc//thumb/thumb0-440x248.jpg"
        ));
    }

    #[test]
    fn cap_evicts_least_recently_updated() {
        let mut file = ProgressFile::default();
        for i in 0..(CAP + 10) {
            file.entries.insert(
                vod_key(&i.to_string()),
                entry(&i.to_string(), 100.0, 1000.0, i as i64),
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
            entry("old", 1.0, 2.0, now - RETENTION_SECS - 1),
        );
        file.entries
            .insert(vod_key("fresh"), entry("fresh", 1.0, 2.0, now));
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
