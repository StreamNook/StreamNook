//! When each Twitch global badge was added, and the one order every surface
//! lists badges in.
//!
//! Twitch publishes no "date added" for a badge. Penrose watches Twitch's list
//! around the clock, records the moment it first sees each badge, and publishes
//! the lot as `index.json` on modroom.streamnook.app. This module keeps a
//! verified copy of that index on disk, a second small file of the dates the
//! badge feed itself carried (so a badge the index does not list yet still has
//! one), and answers two questions for every surface: when was this badge
//! added, and where does it sit when badges are listed newest first.
//!
//! Rules each surface relies on:
//! - A date is the index's, else the feed's, else none. Nothing here reads
//!   cached metadata or scraped text, so no surface shows a date this module
//!   does not stand behind.
//! - Dates are written in UTC. A local zone would put a badge added late in the
//!   evening on the next day for one viewer and not for another.
//! - The order is the same on the site, the desktop gallery and the phone wall:
//!   undated first, then newest first, then finer precision, then the index's
//!   list position, then the lowercased `<set>-v<version>` key compared by code
//!   unit (`a < b`; a locale compare orders `_` and `-` differently in each
//!   language). `badgeOrder.fixture.json` pins it for both.
//! - A file that cannot be read for any reason but "not there" is never treated
//!   as empty, because the next write would erase it. A file that does not parse
//!   is moved aside to `<name>.corrupt` and read as absent.
//!
//! Both files sit directly in the app data folder, never in the cache folder
//! that Clear Cache wipes: clearing a cache must not reorder the gallery. One
//! static lock covers every read and save of both. A write goes to a temp file
//! whose name no other writer shares and is renamed over the target, so a
//! reader never sees half a file.

use crate::services::badge_polling_service::BadgeNotification;
use log::{debug, warn};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering::{AcqRel, Acquire, Relaxed, Release};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

const INDEX_URL: &str = "https://modroom.streamnook.app/badges/index.json";
const INDEX_FILE: &str = "badge_index.json";
const FEED_FILE: &str = "badge_first_seen_feed.json";
/// How long a copy is trusted before a read or the timer renews it.
const REFRESH_EVERY_MS: i64 = 6 * 60 * 60 * 1000;
/// After a failed fetch nothing asks again for this long.
const RETRY_AFTER_FAILURE_MS: i64 = 15 * 60 * 1000;
/// Let the app shell come up before the first network call.
const START_DELAY: Duration = Duration::from_secs(3);
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// The Worker refuses a published index over 2 MiB; twice that is plenty here.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 5000;
/// Every instant in a badge date lies between 2000 and 2100.
const MS_MIN: i64 = 946_684_800_000;
const MS_MAX: i64 = 4_102_444_800_000;

// ---- Dates ------------------------------------------------------------------

/// How exactly a date is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    Minute,
    Day,
    Month,
}

impl Precision {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "minute" => Some(Self::Minute),
            "day" => Some(Self::Day),
            "month" => Some(Self::Month),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Day => "day",
            Self::Month => "month",
        }
    }

    /// Finer first: at one instant a minute outranks a day outranks a month.
    fn rank(self) -> u8 {
        match self {
            Self::Minute => 0,
            Self::Day => 1,
            Self::Month => 2,
        }
    }
}

/// A date and how exactly it is known. `ms` is the instant: exact for a minute,
/// midnight UTC for a day, the first of the month for a month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dated {
    pub ms: i64,
    pub precision: Precision,
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn in_range(ms: i64) -> bool {
    (MS_MIN..MS_MAX).contains(&ms)
}

impl Dated {
    pub fn new(ms: i64, precision: Precision) -> Option<Self> {
        in_range(ms).then_some(Self { ms, precision })
    }

    /// "13 November 2026", or "March 2017" for a month-only date. Always UTC.
    pub fn label(&self) -> String {
        use chrono::Datelike;
        let Some(at) = chrono::DateTime::from_timestamp_millis(self.ms) else {
            return String::new();
        };
        let month = MONTHS[at.month0() as usize];
        match self.precision {
            Precision::Month => format!("{month} {}", at.year()),
            _ => format!("{} {month} {}", at.day(), at.year()),
        }
    }
}

/// The key every date and order here is looked up by: the lowercased
/// `<set>-v<version>`. Set ids are mixed case on Twitch (`H1Z1_1`), and the
/// index, the site and the feed all compare the lowercased form.
pub fn index_key(set_id: &str, version: &str) -> String {
    format!("{set_id}-v{version}").to_lowercase()
}

// ---- The index copy -----------------------------------------------------------

/// What the app reads of one index entry. The entry's name, art, earn windows
/// and removal time have no reader here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Entry {
    date: Option<Dated>,
    hr: Option<u32>,
}

#[derive(Debug, Clone, Default)]
struct IndexCopy {
    at: i64,
    entries: HashMap<String, Entry>,
}

fn entry_of(value: &serde_json::Value) -> Entry {
    // A date needs both halves; one without the other is no date.
    let date = match (
        value.get("t").and_then(|v| v.as_i64()),
        value.get("p").and_then(|v| v.as_str()).and_then(Precision::parse),
    ) {
        (Some(t), Some(p)) => Dated::new(t, p),
        _ => None,
    };
    let hr = value
        .get("hr")
        .and_then(|v| v.as_u64())
        .and_then(|n| u32::try_from(n).ok());
    Entry { date, hr }
}

/// A copy is valid when its envelope is: version 1, an in-range `at`, a
/// `badges` object of a sane size. A bad entry loses its own date only.
fn parse_index(bytes: &[u8]) -> Result<IndexCopy, String> {
    use serde_json::Value;
    let value: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    if value.get("v").and_then(Value::as_i64) != Some(1) {
        return Err("not index version 1".into());
    }
    let at = value
        .get("at")
        .and_then(Value::as_i64)
        .filter(|at| in_range(*at))
        .ok_or("no valid `at`")?;
    let badges = value
        .get("badges")
        .and_then(Value::as_object)
        .ok_or("`badges` is not an object")?;
    if badges.len() > MAX_ENTRIES {
        return Err(format!("{} entries", badges.len()));
    }
    let entries = badges
        .iter()
        .map(|(key, entry)| (key.to_lowercase(), entry_of(entry)))
        .collect();
    Ok(IndexCopy { at, entries })
}

// ---- Feed dates file ------------------------------------------------------------

/// `{ "<key>": { "ms": <int>, "p": "<precision>" } }`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FeedRecord {
    ms: i64,
    p: String,
}

fn record_date(record: &FeedRecord) -> Option<Dated> {
    Precision::parse(&record.p).and_then(|p| Dated::new(record.ms, p))
}

/// The dates a batch of feed drops carries, keyed by `index_key`. A drop with
/// no date, no precision, or a date outside 2000 to 2100 carries none.
pub fn feed_dates_of(badges: &[BadgeNotification]) -> Vec<(String, Dated)> {
    badges
        .iter()
        .filter_map(|badge| {
            let ms = badge.first_seen_ms?;
            let precision = Precision::parse(badge.first_seen_p.as_deref()?)?;
            Some((
                index_key(&badge.badge_set_id, &badge.badge_version),
                Dated::new(ms, precision)?,
            ))
        })
        .collect()
}

// ---- Files and the one lock -------------------------------------------------------

struct Files {
    index: PathBuf,
    feed: PathBuf,
}

fn files_in(dir: &Path) -> Files {
    Files {
        index: dir.join(INDEX_FILE),
        feed: dir.join(FEED_FILE),
    }
}

/// Both files live directly in the app data folder. `cache_service`'s own
/// folder, not `twitch_service`'s (Roaming, shared by debug and release builds).
fn app_files() -> Option<Files> {
    match crate::services::cache_service::get_app_data_dir() {
        Ok(dir) => Some(files_in(&dir)),
        Err(e) => {
            debug!("[BadgeIndex] no app data folder: {e}");
            None
        }
    }
}

#[derive(Default)]
struct State {
    /// The lazy disk load ran without an IO error.
    loaded: bool,
    /// The valid index copy this process holds, if any.
    index: Option<Arc<IndexCopy>>,
    /// The `at` of the copy known to be saved on disk (read at the lazy load or
    /// written since); None when there is none.
    disk_at: Option<i64>,
    /// The bytes of the held copy when saving it failed or the file could not be
    /// read at the time. Saved as soon as the file can be read and written, so a
    /// held copy never stays unsaved for good.
    unsaved: Option<Vec<u8>>,
    /// The temp sweep ran (once per process, even while the file stays unreadable).
    swept: bool,
    /// The unreadable-file warning was logged; cleared by the next good read, so
    /// a file that stays unreadable is reported once, not on every date read.
    warned_unreadable: bool,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));

/// A panic while the lock was held must not take every later date read down
/// with it: the data under it is a copy that is replaced whole.
fn relock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// `Ok(None)` only for a file that is not there. Anything else (a permission
/// error, a sharing violation, a folder in its place) is an error, never empty.
fn read_file(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Moves a file that does not parse to ONE fixed `<name>.corrupt` (the newest
/// bad copy is kept), so it is never read again but can still be looked at.
fn move_aside(path: &Path) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let corrupt = path.with_file_name(format!("{name}.corrupt"));
    // Rename does not replace an existing file on every platform.
    let _ = std::fs::remove_file(&corrupt);
    if let Err(e) = std::fs::rename(path, &corrupt) {
        warn!("[BadgeIndex] Could not move {} aside: {e}", path.display());
        // Left in place it would be read and reported again on every call.
        let _ = std::fs::remove_file(path);
    }
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Temp file with a name no other writer shares (process and a counter), then
/// rename over the target.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("badge");
    let tmp = path.with_file_name(format!(
        "{name}.{}.{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Relaxed)
    ));
    // Flushed before the rename, so a power loss cannot leave an empty file
    // under the real name (it would be set aside as corrupt and its dates lost).
    let written = (|| {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Deletes this module's temp files another process id left behind (a write that
/// died between the temp file and the rename). Runs once, with the lazy load,
/// before this process writes any of its own; its own temp names are never
/// touched.
fn sweep_temps(files: &Files) {
    let Some(dir) = files.index.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mine = std::process::id().to_string();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        for base in [INDEX_FILE, FEED_FILE] {
            // `<base>.<pid>.<n>.tmp`
            let Some(rest) = name
                .strip_prefix(base)
                .and_then(|r| r.strip_prefix('.'))
                .and_then(|r| r.strip_suffix(".tmp"))
            else {
                continue;
            };
            if rest.split('.').next() != Some(mine.as_str()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// The lazy disk load of the index copy. A valid copy is held; an invalid one is
/// moved aside and compares as absent; an IO error leaves it unloaded so the
/// next call tries again.
fn load_index(state: &mut State, files: &Files) {
    if state.loaded {
        return;
    }
    if !state.swept {
        sweep_temps(files);
        state.swept = true;
    }
    match read_file(&files.index) {
        Ok(None) => state.disk_at = None,
        Ok(Some(bytes)) => match parse_index(&bytes) {
            Ok(copy) => {
                state.disk_at = Some(copy.at);
                // A copy already held (a fetch landed while the file was
                // unreadable) stays if it is newer than the one on disk.
                if state.index.as_ref().map_or(true, |held| held.at < copy.at) {
                    state.index = Some(Arc::new(copy));
                }
            }
            Err(why) => {
                warn!("[BadgeIndex] The saved badge index is unusable ({why}); set aside");
                move_aside(&files.index);
                state.disk_at = None;
            }
        },
        Err(e) => {
            if !state.warned_unreadable {
                warn!("[BadgeIndex] Could not read the saved badge index: {e}");
                state.warned_unreadable = true;
            }
            return;
        }
    }
    state.warned_unreadable = false;
    state.loaded = true;
    save_unsaved(state, files);
}

/// Saves a held copy that never reached disk, when it is newer than the copy on
/// disk (or there is none). A failed save keeps it for the next call; a copy the
/// disk has since overtaken is dropped.
fn save_unsaved(state: &mut State, files: &Files) {
    let Some(bytes) = state.unsaved.take() else {
        return;
    };
    let Some(held) = state.index.as_ref().map(|copy| copy.at) else {
        return;
    };
    if state.disk_at.is_some_and(|on_disk| on_disk >= held) {
        return;
    }
    match write_atomic(&files.index, &bytes) {
        Ok(()) => state.disk_at = Some(held),
        Err(e) => {
            warn!("[BadgeIndex] Could not save the badge index: {e}");
            state.unsaved = Some(bytes);
        }
    }
}

/// Holds `copy` only when its `at` is newer than the copy held, and saves its
/// bytes. True when it replaced the held copy. A copy that cannot be saved now is
/// kept unsaved and written by a later call.
fn install_in(state: &mut State, files: &Files, copy: IndexCopy, bytes: &[u8]) -> bool {
    load_index(state, files);
    let newer = state.index.as_ref().map_or(true, |held| held.at < copy.at);
    if !state.loaded {
        // The saved copy could not be read, so it may be newer than this one:
        // hold this copy only if it beats the held one, and leave the file alone
        // until it can be read.
        if !newer {
            return false;
        }
        state.unsaved = Some(bytes.to_vec());
        state.index = Some(Arc::new(copy));
        return true;
    }
    // A held copy whose earlier save failed is saved now, even when this call
    // brings nothing newer.
    save_unsaved(state, files);
    if !newer {
        return false;
    }
    let at = copy.at;
    match write_atomic(&files.index, bytes) {
        Ok(()) => {
            state.disk_at = Some(at);
            state.unsaved = None;
        }
        Err(e) => {
            warn!("[BadgeIndex] Could not save the badge index: {e}");
            state.unsaved = Some(bytes.to_vec());
        }
    }
    state.index = Some(Arc::new(copy));
    true
}

/// The feed file as stored. The held `State` is required only to prove the one
/// lock is taken.
fn read_feed_file(_held: &mut State, files: &Files) -> Result<BTreeMap<String, FeedRecord>, String> {
    match read_file(&files.feed) {
        Ok(None) => Ok(BTreeMap::new()),
        Ok(Some(bytes)) => match serde_json::from_slice(&bytes) {
            Ok(map) => Ok(map),
            Err(e) => {
                warn!("[BadgeIndex] The feed dates file is unusable ({e}); set aside");
                move_aside(&files.feed);
                Ok(BTreeMap::new())
            }
        },
        Err(e) => Err(e.to_string()),
    }
}

/// Adds the keys the file has no date for (the first date a key gets stays) and
/// writes only when one was added. An unreadable file is an error and is left
/// exactly as it is.
fn merge_feed(held: &mut State, files: &Files, batch: &[(String, Dated)]) -> Result<bool, String> {
    let mut map = read_feed_file(held, files)?;
    let mut gained = false;
    for (key, dated) in batch {
        if map.get(key).and_then(record_date).is_some() {
            continue;
        }
        map.insert(
            key.clone(),
            FeedRecord {
                ms: dated.ms,
                p: dated.precision.as_str().to_string(),
            },
        );
        gained = true;
    }
    if !gained {
        return Ok(false);
    }
    let bytes = serde_json::to_vec(&map).map_err(|e| e.to_string())?;
    write_atomic(&files.feed, &bytes).map_err(|e| e.to_string())?;
    Ok(true)
}

fn record_feed_dates_in(held: &mut State, files: &Files, batch: &[(String, Dated)]) -> bool {
    match merge_feed(held, files, batch) {
        Ok(gained) => gained,
        Err(why) => {
            warn!("[BadgeIndex] Could not save feed dates: {why}");
            false
        }
    }
}

// ---- Reading ------------------------------------------------------------------------

/// What one build of the dates reads: the index copy this process holds, when
/// it holds a valid one, and the dates the badge feed carried.
#[derive(Debug, Clone)]
pub struct Snapshot {
    index: Option<Arc<IndexCopy>>,
    feed: HashMap<String, Dated>,
}

impl Snapshot {
    /// True when this process holds a valid index copy, decided after the lazy
    /// disk load. While false, the order falls back to `fallback_order`.
    pub fn index_loaded(&self) -> bool {
        self.index.is_some()
    }

    /// The index's date, else the feed's, else none. `key` is `index_key`.
    pub fn resolve(&self, key: &str) -> Option<Dated> {
        self.index
            .as_ref()
            .and_then(|copy| copy.entries.get(key))
            .and_then(|entry| entry.date)
            .or_else(|| self.feed.get(key).copied())
    }

    /// The badge's position in Twitch's list, the index's tie-break.
    pub fn hr(&self, key: &str) -> Option<u32> {
        self.index
            .as_ref()
            .and_then(|copy| copy.entries.get(key))
            .and_then(|entry| entry.hr)
    }

    pub fn feed_dates(&self) -> &HashMap<String, Dated> {
        &self.feed
    }
}

#[cfg(test)]
impl Snapshot {
    pub(crate) fn for_test(index: Option<&str>, feed: &[(&str, i64, Precision)]) -> Snapshot {
        Snapshot {
            index: index.map(|text| Arc::new(parse_index(text.as_bytes()).expect("a valid index"))),
            feed: feed
                .iter()
                .map(|(key, ms, p)| (key.to_string(), Dated::new(*ms, *p).expect("in range")))
                .collect(),
        }
    }
}

fn snapshot_in(state: &mut State, files: &Files) -> Snapshot {
    load_index(state, files);
    // Read from disk every time: another process (the Android notification
    // worker) may have added dates since this one last looked.
    let feed = match read_feed_file(state, files) {
        Ok(map) => map
            .iter()
            .filter_map(|(key, record)| Some((key.clone(), record_date(record)?)))
            .collect(),
        Err(why) => {
            warn!("[BadgeIndex] Could not read feed dates: {why}");
            HashMap::new()
        }
    };
    Snapshot {
        index: state.index.clone(),
        feed,
    }
}

/// The dates as they stand now. Does disk IO under the one lock: call it from
/// the blocking pool.
pub fn snapshot() -> Snapshot {
    let Some(files) = app_files() else {
        return Snapshot {
            index: None,
            feed: HashMap::new(),
        };
    };
    let mut state = relock(&STATE);
    snapshot_in(&mut state, &files)
}

/// Saves the dates a batch of feed drops carries, once for the whole batch, and
/// writes the file only when some key gained a date. True when one did. Does
/// disk IO under the one lock.
pub fn record_feed_dates(badges: &[BadgeNotification]) -> bool {
    let batch = feed_dates_of(badges);
    if batch.is_empty() {
        return false;
    }
    let Some(files) = app_files() else {
        return false;
    };
    let mut state = relock(&STATE);
    record_feed_dates_in(&mut state, &files, &batch)
}

fn install(copy: IndexCopy, bytes: &[u8]) -> bool {
    let Some(files) = app_files() else {
        return false;
    };
    let mut state = relock(&STATE);
    install_in(&mut state, &files, copy, bytes)
}

/// Tells open surfaces to read the standing again. With no handle (the Android
/// notification worker) it uses the running app's, when this process has one.
pub fn notify(app: Option<&crate::rt::AppHandle>) {
    use tauri::Emitter;
    let Some(app) = app.cloned().or_else(crate::services::providers::app_handle) else {
        return;
    };
    let _ = app.emit("badge-standing-changed", ());
}

// ---- Order ----------------------------------------------------------------------------

/// One badge to place: its `index_key`, its date and its list position.
#[derive(Debug, Clone)]
pub struct OrderInput {
    pub key: String,
    pub date: Option<Dated>,
    pub hr: Option<u32>,
}

fn newest_first(a: &OrderInput, b: &OrderInput) -> Ordering {
    let by_date = match (a.date, b.date) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => y
            .ms
            .cmp(&x.ms)
            .then_with(|| x.precision.rank().cmp(&y.precision.rank())),
    };
    by_date
        .then_with(|| a.hr.unwrap_or(u32::MAX).cmp(&b.hr.unwrap_or(u32::MAX)))
        .then_with(|| a.key.cmp(&b.key))
}

/// The keys in Newest First order: undated first, then `t` descending, then
/// precision, then list position (missing last), then the key by code unit.
pub fn order_keys(mut inputs: Vec<OrderInput>) -> Vec<String> {
    inputs.sort_by(newest_first);
    inputs.into_iter().map(|input| input.key).collect()
}

/// The order while this process holds no index copy (a first run, or an invalid
/// copy moved aside): keys with a feed date first (newest, then precision, then
/// key), then keys with a universal-cache position ascending, then the rest by
/// key. Positions only exist for entries the daily job knows, so a badge the
/// feed just announced leads instead of sinking.
pub fn fallback_order(
    keys: &[String],
    feed_dates: &HashMap<String, Dated>,
    positions: &HashMap<String, u32>,
) -> Vec<String> {
    let mut list: Vec<&String> = keys.iter().collect();
    list.sort_by(|a, b| match (feed_dates.get(*a), feed_dates.get(*b)) {
        (Some(x), Some(y)) => y
            .ms
            .cmp(&x.ms)
            .then_with(|| x.precision.rank().cmp(&y.precision.rank()))
            .then_with(|| a.cmp(b)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => match (positions.get(*a), positions.get(*b)) {
            (Some(x), Some(y)) => x.cmp(y).then_with(|| a.cmp(b)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.cmp(b),
        },
    });
    list.into_iter().cloned().collect()
}

// ---- Keeping the copy fresh --------------------------------------------------------------

static STARTED: AtomicBool = AtomicBool::new(false);
static REFRESHING: AtomicBool = AtomicBool::new(false);
/// When the last fetch succeeded, in ms; 0 until one has this run.
static LAST_OK_MS: AtomicI64 = AtomicI64::new(0);
/// No fetch is attempted before this instant (set after a failure).
static NEXT_ATTEMPT_MS: AtomicI64 = AtomicI64::new(0);

/// Clears the in-flight flag however the fetch ends, a panic included.
struct RefreshGuard;
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        REFRESHING.store(false, Release);
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A fetch is due once the last good one is six hours old and no failure
/// backoff is running. Nothing fetched yet this run counts as old.
pub fn refresh_due(now: i64, last_ok: i64, next_attempt: i64) -> bool {
    now >= next_attempt && now - last_ok >= REFRESH_EVERY_MS
}

/// How long the timer sleeps: until the later of the six hours and the backoff,
/// never under 30 s (a due fetch another task is already running) or over six hours.
pub fn wait_ms(now: i64, last_ok: i64, next_attempt: i64) -> i64 {
    (next_attempt - now)
        .max(last_ok + REFRESH_EVERY_MS - now)
        .clamp(30_000, REFRESH_EVERY_MS)
}

async fn fetch_index() -> Result<Vec<u8>, String> {
    let client = crate::services::http::client();
    let request = client.get(INDEX_URL).timeout(FETCH_TIMEOUT);
    let mut response = crate::services::sn_transport::send(request)
        .await
        .map_err(|e| crate::services::sn_transport::reason(&e))?;
    if !response.status().is_success() {
        return Err(format!("answered {}", response.status()));
    }
    // An early refusal for a body that announces itself too large. A compressed
    // body announces nothing, so the cap is also enforced while reading, the same
    // way the link preview reads a capped body.
    if response.content_length() > Some(MAX_BODY_BYTES as u64) {
        return Err("body too large".into());
    }
    let mut body: Vec<u8> = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_BODY_BYTES {
                    return Err("body too large".into());
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(format!("read body failed: {e}")),
        }
    }
    Ok(body)
}

/// One fetch. The body is parsed and saved on the blocking pool (a 150 KB parse
/// is not work for a runtime thread). A copy newer than the held one is saved and
/// open surfaces are told, which is the one re-sort an install gets when its
/// first index lands.
async fn refresh_once(app: Option<&crate::rt::AppHandle>) -> bool {
    let now = now_ms();
    let outcome = match fetch_index().await {
        Ok(bytes) => tokio::task::spawn_blocking(move || {
            let copy = parse_index(&bytes)?;
            Ok::<bool, String>(install(copy, &bytes))
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|installed| installed),
        Err(why) => Err(why),
    };
    match outcome {
        Ok(replaced) => {
            LAST_OK_MS.store(now, Release);
            NEXT_ATTEMPT_MS.store(0, Release);
            if replaced {
                notify(app);
            }
            true
        }
        Err(why) => {
            debug!("[BadgeIndex] Could not read the badge index: {why}");
            NEXT_ATTEMPT_MS.store(now + RETRY_AFTER_FAILURE_MS, Release);
            false
        }
    }
}

/// Runs a fetch unless one is already running.
async fn run_single_flight(app: Option<&crate::rt::AppHandle>) {
    if REFRESHING.swap(true, AcqRel) {
        return;
    }
    let _guard = RefreshGuard;
    refresh_once(app).await;
}

/// Called when the standing is read: renews a copy older than six hours in the
/// background, honoring the failure backoff. Never waits on the network.
pub fn refresh_if_due(app: &crate::rt::AppHandle) {
    if REFRESHING.load(Acquire)
        || !refresh_due(
            now_ms(),
            LAST_OK_MS.load(Acquire),
            NEXT_ATTEMPT_MS.load(Acquire),
        )
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        run_single_flight(Some(&app)).await;
    });
}

/// Fetches the index shortly after the app starts and every six hours after
/// (sooner after a failure). The Android notification worker never calls this:
/// it only records feed dates.
pub fn start(app: crate::rt::AppHandle) {
    if STARTED.swap(true, AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(START_DELAY).await;
        loop {
            if refresh_due(
                now_ms(),
                LAST_OK_MS.load(Acquire),
                NEXT_ATTEMPT_MS.load(Acquire),
            ) {
                run_single_flight(Some(&app)).await;
            }
            let wait = wait_ms(
                now_ms(),
                LAST_OK_MS.load(Acquire),
                NEXT_ATTEMPT_MS.load(Acquire),
            );
            tokio::time::sleep(Duration::from_millis(wait as u64)).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::badge_polling_service::BadgeNotificationStatus;
    use chrono::Datelike;
    use serde_json::json;

    const ORDER_FIXTURE: &str = include_str!("../../tests/fixtures/badgeOrder.fixture.json");
    const LABEL_FIXTURE: &str = include_str!("../../tests/fixtures/badgeLabels.fixture.json");

    const AT: i64 = 1_794_600_000_000;
    /// 13 November 2026, 02:48:09 UTC.
    const T_NEW: i64 = 1_794_538_089_000;
    /// 13 November 2026, 00:00 UTC.
    const T_DAY: i64 = 1_794_528_000_000;
    /// 1 March 2017.
    const T_OLD: i64 = 1_488_326_400_000;

    fn dated(ms: i64, p: Precision) -> Dated {
        Dated::new(ms, p).unwrap()
    }

    /// A scratch folder holding both files; removed when it goes out of scope.
    struct Scratch {
        dir: PathBuf,
        files: Files,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("sn-badge-index-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let files = files_in(&dir);
            Self { dir, files }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn index_text(at: i64, badges: serde_json::Value) -> String {
        json!({ "v": 1, "at": at, "badges": badges }).to_string()
    }

    fn sample_index(at: i64) -> String {
        index_text(
            at,
            json!({
                "new-v1": { "s": "new", "ver": "1", "t": T_NEW, "p": "minute", "hr": 3 },
                "old-v1": { "s": "old", "ver": "1", "t": T_OLD, "p": "month", "hr": 9 },
                "bare-v1": { "s": "bare", "ver": "1", "hr": 4 }
            }),
        )
    }

    fn install_text(state: &mut State, files: &Files, text: &str) -> bool {
        install_in(state, files, parse_index(text.as_bytes()).unwrap(), text.as_bytes())
    }

    // ---- the shared fixtures ----

    #[derive(Deserialize)]
    struct FixtureEntry {
        key: String,
        t: Option<i64>,
        p: Option<String>,
        hr: Option<u32>,
    }

    #[derive(Deserialize)]
    struct OrderCase {
        name: String,
        entries: Vec<FixtureEntry>,
        expected: Vec<String>,
    }

    #[derive(Deserialize)]
    struct OrderFixture {
        order: Vec<OrderCase>,
    }

    fn inputs(entries: &[FixtureEntry]) -> Vec<OrderInput> {
        entries
            .iter()
            .map(|e| OrderInput {
                key: e.key.clone(),
                date: match (e.t, e.p.as_deref().and_then(Precision::parse)) {
                    (Some(t), Some(p)) => Dated::new(t, p),
                    _ => None,
                },
                hr: e.hr,
            })
            .collect()
    }

    #[test]
    fn the_shared_order_fixture_holds_in_any_input_order() {
        let fixture: OrderFixture = serde_json::from_str(ORDER_FIXTURE).unwrap();
        assert!(fixture.order.len() >= 7, "the fixture lost rows");
        for case in &fixture.order {
            let mut forward = inputs(&case.entries);
            assert_eq!(order_keys(forward.clone()), case.expected, "{}", case.name);
            forward.reverse();
            assert_eq!(order_keys(forward), case.expected, "{} (reversed)", case.name);
        }
    }

    #[derive(Deserialize)]
    struct LabelCase {
        ms: i64,
        p: String,
        long: String,
    }

    #[derive(Deserialize)]
    struct LabelFixture {
        cases: Vec<LabelCase>,
    }

    #[test]
    fn dates_are_labelled_in_utc_whatever_the_local_zone() {
        let fixture: LabelFixture = serde_json::from_str(LABEL_FIXTURE).unwrap();
        assert!(fixture.cases.len() >= 5);
        for case in &fixture.cases {
            let precision = Precision::parse(&case.p).unwrap();
            assert_eq!(dated(case.ms, precision).label(), case.long, "{} {}", case.ms, case.p);
        }
        // 02:48 UTC on the 13th is still the 12th in a zone west of UTC, which is
        // where a label written in local time would go wrong.
        let west = chrono::FixedOffset::west_opt(8 * 3600).unwrap();
        let local = chrono::DateTime::from_timestamp_millis(T_NEW)
            .unwrap()
            .with_timezone(&west);
        assert_eq!(local.day(), 12);
        assert_eq!(dated(T_NEW, Precision::Minute).label(), "13 November 2026");
    }

    /// The site's checkout: `SN_SITE_DIR`, else `C:/streamnook.app`, else
    /// `/mnt/c/streamnook.app`.
    fn site_dir() -> Option<PathBuf> {
        if let Ok(dir) = std::env::var("SN_SITE_DIR") {
            let path = PathBuf::from(dir);
            if path.exists() {
                return Some(path);
            }
        }
        ["C:/streamnook.app", "/mnt/c/streamnook.app"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
    }

    #[test]
    fn the_committed_fixture_copies_equal_the_sites() {
        let Some(dir) = site_dir() else {
            assert!(
                std::env::var_os("CI").is_some(),
                "the site checkout was not found: set SN_SITE_DIR to it (or CI to skip this check)"
            );
            return;
        };
        for (name, committed) in [
            ("badgeOrder.fixture.json", ORDER_FIXTURE),
            ("badgeLabels.fixture.json", LABEL_FIXTURE),
        ] {
            let site = std::fs::read_to_string(dir.join("functions/utils").join(name))
                .unwrap_or_else(|e| panic!("the site's functions/utils/{name}: {e}"))
                .replace("\r\n", "\n");
            assert_eq!(
                committed.replace("\r\n", "\n"),
                site,
                "src-tauri/tests/fixtures/{name} must equal the site's copy"
            );
        }
    }

    // ---- the index copy ----

    #[test]
    fn an_index_copy_needs_version_one_a_valid_at_and_a_badges_object() {
        assert!(parse_index(sample_index(AT).as_bytes()).is_ok());
        assert!(parse_index(b"not json").is_err());
        let refused = [
            json!({ "v": 2, "at": AT, "badges": {} }),
            json!({ "v": 1, "at": 5, "badges": {} }),
            json!({ "v": 1, "at": AT, "badges": [] }),
            json!({ "v": 1, "badges": {} }),
        ];
        for body in refused {
            assert!(parse_index(body.to_string().as_bytes()).is_err(), "{body}");
        }
        let many: serde_json::Map<String, serde_json::Value> = (0..=MAX_ENTRIES)
            .map(|i| (format!("k{i}-v1"), json!({})))
            .collect();
        assert!(parse_index(index_text(AT, serde_json::Value::Object(many)).as_bytes()).is_err());
    }

    #[test]
    fn a_bad_entry_loses_its_own_date_and_never_the_copy() {
        let copy = parse_index(
            index_text(
                AT,
                json!({
                    "ok-v1": { "t": T_DAY, "p": "day", "hr": 1 },
                    "no-p-v1": { "t": T_DAY },
                    "bad-p-v1": { "t": T_DAY, "p": "hour" },
                    "early-v1": { "t": 5, "p": "day" },
                    "neg-hr-v1": { "t": T_DAY, "p": "day", "hr": -1 },
                    "Mixed-V1": { "t": T_DAY, "p": "minute" }
                }),
            )
            .as_bytes(),
        )
        .unwrap();
        let date = |key: &str| copy.entries.get(key).and_then(|e| e.date);
        assert_eq!(date("ok-v1"), Dated::new(T_DAY, Precision::Day));
        assert_eq!(date("no-p-v1"), None);
        assert_eq!(date("bad-p-v1"), None);
        assert_eq!(date("early-v1"), None);
        assert_eq!(copy.entries["neg-hr-v1"].hr, None);
        assert_eq!(date("neg-hr-v1"), Dated::new(T_DAY, Precision::Day));
        // Keys are compared lowercased.
        assert!(copy.entries.contains_key("mixed-v1"));
    }

    #[test]
    fn resolve_reads_the_index_then_the_feed_then_nothing() {
        let snapshot = Snapshot::for_test(
            Some(&sample_index(AT)),
            &[
                ("new-v1", T_DAY, Precision::Day),
                ("bare-v1", T_DAY, Precision::Day),
                ("fed-v1", T_NEW, Precision::Minute),
            ],
        );
        assert!(snapshot.index_loaded());
        // The index wins over the feed for the same key.
        assert_eq!(snapshot.resolve("new-v1"), Dated::new(T_NEW, Precision::Minute));
        // Listed but undated: the feed's date fills it.
        assert_eq!(snapshot.resolve("bare-v1"), Dated::new(T_DAY, Precision::Day));
        // Not listed at all: the feed's date.
        assert_eq!(snapshot.resolve("fed-v1"), Dated::new(T_NEW, Precision::Minute));
        // Nothing anywhere: undated.
        assert_eq!(snapshot.resolve("ghost-v1"), None);
        assert_eq!(snapshot.hr("old-v1"), Some(9));
        assert_eq!(snapshot.hr("ghost-v1"), None);
        // No index held: the feed alone.
        let no_index = Snapshot::for_test(None, &[("fed-v1", T_NEW, Precision::Minute)]);
        assert!(!no_index.index_loaded());
        assert_eq!(no_index.resolve("fed-v1"), Dated::new(T_NEW, Precision::Minute));
        assert_eq!(no_index.resolve("new-v1"), None);
    }

    #[test]
    fn no_cached_metadata_date_is_ever_read() {
        let src = include_str!("badge_index.rs");
        assert!(!src.contains(concat!("date_", "added")));
        assert!(!src.contains(concat!("universal_cache", "_service")));
    }

    #[test]
    fn the_disk_copy_is_replaced_only_by_a_newer_at() {
        let scratch = Scratch::new("newer-at");
        let mut state = State::default();
        let first = sample_index(AT);
        assert!(install_text(&mut state, &scratch.files, &first));
        let same_at = index_text(AT, json!({ "other-v1": { "t": T_DAY, "p": "day" } }));
        assert!(!install_text(&mut state, &scratch.files, &same_at));
        let older = index_text(AT - 1, json!({}));
        assert!(!install_text(&mut state, &scratch.files, &older));
        assert_eq!(std::fs::read(&scratch.files.index).unwrap(), first.as_bytes());
        let newer = index_text(AT + 1, json!({ "other-v1": { "t": T_DAY, "p": "day" } }));
        assert!(install_text(&mut state, &scratch.files, &newer));
        assert_eq!(std::fs::read(&scratch.files.index).unwrap(), newer.as_bytes());
        assert_eq!(state.index.as_ref().unwrap().at, AT + 1);
        // A fresh process reads the newest copy back.
        let mut restarted = State::default();
        let snapshot = snapshot_in(&mut restarted, &scratch.files);
        assert_eq!(snapshot.resolve("other-v1"), Dated::new(T_DAY, Precision::Day));
    }

    #[test]
    fn a_valid_disk_copy_is_loaded_on_the_first_snapshot() {
        let scratch = Scratch::new("valid-copy");
        std::fs::write(&scratch.files.index, sample_index(AT)).unwrap();
        let mut state = State::default();
        let snapshot = snapshot_in(&mut state, &scratch.files);
        assert!(snapshot.index_loaded());
        assert_eq!(snapshot.resolve("new-v1"), Dated::new(T_NEW, Precision::Minute));
    }

    #[test]
    fn a_missing_copy_is_absent_and_stays_loadable() {
        let scratch = Scratch::new("no-copy");
        let mut state = State::default();
        let snapshot = snapshot_in(&mut state, &scratch.files);
        assert!(!snapshot.index_loaded());
        assert!(state.loaded);
        assert!(install_text(&mut state, &scratch.files, &sample_index(AT)));
        assert!(snapshot_in(&mut state, &scratch.files).index_loaded());
    }

    #[test]
    fn an_unreadable_copy_is_not_read_as_absent() {
        let scratch = Scratch::new("unreadable-copy");
        // A folder where the file belongs: reading it fails with something other
        // than NotFound.
        std::fs::create_dir(&scratch.files.index).unwrap();
        let mut state = State::default();
        let snapshot = snapshot_in(&mut state, &scratch.files);
        assert!(!snapshot.index_loaded());
        assert!(!state.loaded, "an IO error leaves the load to be retried");
        assert!(scratch.files.index.is_dir());
    }

    #[test]
    fn an_invalid_copy_is_moved_aside_to_one_corrupt_name_and_a_fetch_then_replaces_it() {
        let scratch = Scratch::new("invalid-copy");
        let corrupt = scratch.dir.join("badge_index.json.corrupt");
        let bad_copies = [
            "not json".to_string(),
            json!({ "v": 2, "at": AT, "badges": {} }).to_string(),
            json!({ "v": 1, "at": AT, "badges": [] }).to_string(),
        ];
        let mut state = State::default();
        for bad in &bad_copies {
            std::fs::write(&scratch.files.index, bad).unwrap();
            state = State::default();
            let snapshot = snapshot_in(&mut state, &scratch.files);
            assert!(!snapshot.index_loaded(), "{bad}");
            assert!(!scratch.files.index.exists(), "{bad}");
            // One fixed name, the newest bad copy kept.
            assert_eq!(&std::fs::read_to_string(&corrupt).unwrap(), bad);
        }
        // With no valid copy left, the next fetch lands and is written fresh.
        assert!(install_text(&mut state, &scratch.files, &sample_index(AT)));
        assert!(snapshot_in(&mut state, &scratch.files).index_loaded());
        assert_eq!(
            std::fs::read_to_string(&scratch.files.index).unwrap(),
            sample_index(AT)
        );
    }

    // ---- the feed dates file ----

    #[test]
    fn a_feed_date_write_keeps_the_index_copy_and_the_reader_unions_both() {
        let scratch = Scratch::new("union");
        let mut state = State::default();
        install_text(&mut state, &scratch.files, &sample_index(AT));
        let before = std::fs::read(&scratch.files.index).unwrap();
        // A cold process: its own empty state, the same files.
        let mut cold = State::default();
        let batch = vec![("fed-v1".to_string(), dated(T_DAY, Precision::Day))];
        assert!(record_feed_dates_in(&mut cold, &scratch.files, &batch));
        assert_eq!(std::fs::read(&scratch.files.index).unwrap(), before);
        // The running process reads both files.
        let snapshot = snapshot_in(&mut state, &scratch.files);
        assert_eq!(snapshot.resolve("new-v1"), Dated::new(T_NEW, Precision::Minute));
        assert_eq!(snapshot.resolve("fed-v1"), Dated::new(T_DAY, Precision::Day));
    }

    #[test]
    fn the_first_feed_date_for_a_key_wins_and_a_batch_without_news_writes_nothing() {
        let scratch = Scratch::new("first-wins");
        let mut state = State::default();
        let day = dated(T_DAY, Precision::Day);
        let minute = dated(T_NEW, Precision::Minute);
        assert!(record_feed_dates_in(&mut state, &scratch.files, &[("a-v1".to_string(), day)]));
        let stamp = std::fs::metadata(&scratch.files.feed).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        // Known key, later and finer: nothing gained, nothing written.
        let again = [("a-v1".to_string(), minute), ("a-v1".to_string(), day)];
        assert!(!record_feed_dates_in(&mut state, &scratch.files, &again));
        let after = std::fs::metadata(&scratch.files.feed).unwrap().modified().unwrap();
        assert_eq!(stamp, after, "the file was rewritten for nothing");
        assert_eq!(snapshot_in(&mut state, &scratch.files).resolve("a-v1"), Some(day));
        // A new key repeated in one batch stores the first.
        let twice = [("b-v1".to_string(), minute), ("b-v1".to_string(), day)];
        assert!(record_feed_dates_in(&mut state, &scratch.files, &twice));
        assert_eq!(snapshot_in(&mut state, &scratch.files).resolve("b-v1"), Some(minute));
    }

    #[test]
    fn a_feed_read_failing_for_any_reason_but_not_found_aborts_the_write() {
        let scratch = Scratch::new("unreadable-feed");
        // A folder where the file belongs: not NotFound, so never read as empty.
        std::fs::create_dir(&scratch.files.feed).unwrap();
        let mut state = State::default();
        let batch = vec![("a-v1".to_string(), dated(T_DAY, Precision::Day))];
        // An unreadable file must never read as empty.
        assert!(read_feed_file(&mut state, &scratch.files).is_err());
        assert!(merge_feed(&mut state, &scratch.files, &batch).is_err());
        assert!(!record_feed_dates_in(&mut state, &scratch.files, &batch));
        assert!(scratch.files.feed.is_dir(), "the unreadable file was replaced");
    }

    #[test]
    fn a_corrupt_feed_file_is_moved_aside_and_writing_resumes() {
        let scratch = Scratch::new("corrupt-feed");
        std::fs::write(&scratch.files.feed, "{not json").unwrap();
        let mut state = State::default();
        let batch = vec![("a-v1".to_string(), dated(T_DAY, Precision::Day))];
        assert!(record_feed_dates_in(&mut state, &scratch.files, &batch));
        let corrupt = scratch.dir.join("badge_first_seen_feed.json.corrupt");
        assert_eq!(std::fs::read_to_string(corrupt).unwrap(), "{not json");
        assert_eq!(
            snapshot_in(&mut state, &scratch.files).resolve("a-v1"),
            Some(dated(T_DAY, Precision::Day))
        );
    }

    fn drop_with(set: &str, version: &str, ms: Option<i64>, p: Option<&str>) -> BadgeNotification {
        BadgeNotification {
            badge_name: "Badge".into(),
            badge_set_id: set.into(),
            badge_version: version.into(),
            badge_image_url: String::new(),
            badge_description: None,
            status: BadgeNotificationStatus::New,
            date_info: None,
            enrichment: None,
            window: None,
            first_seen_ms: ms,
            first_seen_p: p.map(String::from),
        }
    }

    #[test]
    fn a_drop_carries_a_feed_date_only_when_it_has_a_whole_one() {
        let batch = [
            drop_with("H1Z1_1", "1", Some(T_NEW), Some("minute")),
            drop_with("nodate", "1", None, Some("minute")),
            drop_with("noprecision", "1", Some(T_NEW), None),
            drop_with("badprecision", "1", Some(T_NEW), Some("hour")),
            drop_with("early", "1", Some(5), Some("day")),
        ];
        let dates = feed_dates_of(&batch);
        assert_eq!(dates, vec![("h1z1_1-v1".to_string(), dated(T_NEW, Precision::Minute))]);
    }

    #[test]
    fn an_unchanged_repush_with_a_date_still_writes_the_feed_file_once() {
        let scratch = Scratch::new("repush");
        let mut state = State::default();
        let first = [drop_with("fresh", "1", Some(T_NEW), Some("minute"))];
        // The same drop again, byte for byte: the classifier would skip it, which
        // is why the dates are taken before it. The second batch gains nothing.
        let again = [drop_with("fresh", "1", Some(T_NEW), Some("minute"))];
        assert_eq!(first[0].content_hash(), again[0].content_hash());
        assert!(record_feed_dates_in(&mut state, &scratch.files, &feed_dates_of(&first)));
        assert!(!record_feed_dates_in(&mut state, &scratch.files, &feed_dates_of(&again)));
        assert_eq!(
            snapshot_in(&mut state, &scratch.files).resolve("fresh-v1"),
            Some(dated(T_NEW, Precision::Minute))
        );
    }

    #[test]
    fn writers_never_share_a_temp_name_or_leave_one_behind() {
        let scratch = Scratch::new("temp-names");
        let path = scratch.files.feed.clone();
        let writers: Vec<_> = (0..8)
            .map(|n| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for round in 0..20 {
                        // Another writer's rename can win; the result is not the point.
                        let _ = write_atomic(&path, format!("{{\"w{n}\":{round}}}").as_bytes());
                    }
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
        let left: Vec<_> = std::fs::read_dir(&scratch.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(left, vec!["badge_first_seen_feed.json".to_string()], "{left:?}");
        assert!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&path).unwrap()).is_ok());
    }

    // ---- where the files live, and the one lock ----

    #[test]
    fn both_files_sit_directly_in_the_app_data_dir_outside_the_cache() {
        let data = crate::services::cache_service::get_app_data_dir().unwrap();
        let cache = crate::services::cache_service::get_cache_dir().unwrap();
        let files = app_files().unwrap();
        for path in [&files.index, &files.feed] {
            assert_eq!(path.parent(), Some(data.as_path()), "{}", path.display());
            assert!(!path.starts_with(&cache), "{} is under the cache", path.display());
        }
        assert_eq!(files.index.file_name().unwrap(), "badge_index.json");
        assert_eq!(files.feed.file_name().unwrap(), "badge_first_seen_feed.json");
    }

    #[test]
    fn the_module_never_redirects_the_process_wide_paths() {
        let src = include_str!("badge_index.rs");
        // Process-wide cells that would redirect every other test in this binary.
        assert!(!src.contains(concat!("set_", "base(")));
        assert!(!src.contains(concat!("set_cache_", "base(")));
    }

    #[test]
    fn every_read_and_save_of_both_files_goes_through_the_one_static_lock() {
        let src = include_str!("badge_index.rs").replace("\r\n", "\n");
        for head in [
            "pub fn snapshot() -> Snapshot",
            "pub fn record_feed_dates(",
            "fn install(copy: IndexCopy",
        ] {
            let start = src.find(head).unwrap_or_else(|| panic!("{head}"));
            let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
            assert!(body.contains("relock(&STATE)"), "{head}");
        }
        // The helpers that touch the files take the held state, so none can be
        // called without the lock.
        for head in ["fn load_index(", "fn install_in(", "fn read_feed_file(", "fn merge_feed("] {
            let start = src.find(head).unwrap_or_else(|| panic!("{head}"));
            assert!(src[start..start + 120].contains("&mut State"), "{head}");
        }
    }

    #[test]
    fn a_poisoned_lock_recovers() {
        let shared = Arc::new(Mutex::new(State::default()));
        let poisoner = Arc::clone(&shared);
        let _ = std::thread::spawn(move || {
            let _held = poisoner.lock().unwrap();
            panic!("poisoned on purpose");
        })
        .join();
        assert!(shared.is_poisoned());
        let state = relock(&shared);
        assert!(state.index.is_none());
    }

    // ---- the timer ----

    #[test]
    fn a_read_renews_the_index_only_when_stale_and_not_backing_off() {
        let now = 1_794_600_000_000;
        let hours = |h: i64| h * 60 * 60 * 1000;
        // Nothing fetched yet this run counts as stale.
        assert!(refresh_due(now, 0, 0));
        assert!(!refresh_due(now, now - hours(5), 0));
        assert!(refresh_due(now, now - hours(6), 0));
        // A failure 5 minutes ago backs off for 10 more, stale or not.
        let backoff_until = now + 10 * 60 * 1000;
        assert!(!refresh_due(now, 0, backoff_until));
        assert!(refresh_due(backoff_until, 0, backoff_until));
    }

    #[test]
    fn the_timer_sleeps_six_hours_after_a_success_and_fifteen_minutes_after_a_failure() {
        let now = 1_794_600_000_000;
        assert_eq!(wait_ms(now, now, 0), REFRESH_EVERY_MS);
        assert_eq!(wait_ms(now, 0, now + RETRY_AFTER_FAILURE_MS), RETRY_AFTER_FAILURE_MS);
        // Due, with another task already fetching: look again in 30 s.
        assert_eq!(wait_ms(now, 0, 0), 30_000);
    }

    /// Both single-flight tests flip the one static flag; they take turns.
    static REFRESH_TEST: Mutex<()> = Mutex::new(());

    #[test]
    fn refreshes_are_single_flight() {
        let _turn = relock(&REFRESH_TEST);
        REFRESHING.store(false, Release);
        assert!(!REFRESHING.swap(true, AcqRel));
        assert!(REFRESHING.swap(true, AcqRel), "a second fetch must not start");
        drop(RefreshGuard);
        assert!(!REFRESHING.load(Acquire));
    }

    #[tokio::test]
    async fn a_second_caller_while_one_runs_fetches_nothing_and_changes_no_clock() {
        let _turn = relock(&REFRESH_TEST);
        REFRESHING.store(true, Release);
        let (ok, next) = (LAST_OK_MS.load(Acquire), NEXT_ATTEMPT_MS.load(Acquire));
        run_single_flight(None).await;
        assert_eq!((LAST_OK_MS.load(Acquire), NEXT_ATTEMPT_MS.load(Acquire)), (ok, next));
        assert!(REFRESHING.load(Acquire), "the running fetch still owns the flag");
        REFRESHING.store(false, Release);
    }

    #[test]
    fn telling_surfaces_with_no_handle_and_no_app_does_nothing() {
        notify(None);
    }

    #[test]
    fn an_unreadable_saved_copy_is_never_overwritten_by_a_fetch() {
        let scratch = Scratch::new("install-unreadable");
        // A folder where the saved copy belongs: it cannot be read, so it may be
        // newer than anything fetched.
        std::fs::create_dir(&scratch.files.index).unwrap();
        let mut state = State::default();
        assert!(install_text(&mut state, &scratch.files, &sample_index(AT)));
        assert!(scratch.files.index.is_dir(), "the unreadable file was replaced");
        assert!(state.index.is_some(), "held in memory for the run");
    }

    #[test]
    fn a_lazy_load_never_replaces_a_newer_held_copy_with_an_older_disk_copy() {
        let scratch = Scratch::new("lazy-keeps-newer");
        // The file is unreadable when the fetch lands, so the fetch is held in memory.
        std::fs::create_dir(&scratch.files.index).unwrap();
        let mut state = State::default();
        assert!(install_text(&mut state, &scratch.files, &sample_index(AT + 1)));
        // Then the file becomes readable and holds an older copy.
        std::fs::remove_dir(&scratch.files.index).unwrap();
        std::fs::write(&scratch.files.index, sample_index(AT)).unwrap();
        snapshot_in(&mut state, &scratch.files);
        assert!(state.loaded);
        assert_eq!(state.index.as_ref().unwrap().at, AT + 1);
        // The held copy that never reached disk is saved now that the file can be read.
        assert_eq!(
            std::fs::read_to_string(&scratch.files.index).unwrap(),
            sample_index(AT + 1)
        );
    }

    #[test]
    fn a_held_copy_is_saved_over_a_corrupt_file_once_it_can_be_read() {
        let scratch = Scratch::new("unsaved-over-corrupt");
        std::fs::create_dir(&scratch.files.index).unwrap();
        let mut state = State::default();
        assert!(install_text(&mut state, &scratch.files, &sample_index(AT + 1)));
        std::fs::remove_dir(&scratch.files.index).unwrap();
        std::fs::write(&scratch.files.index, "garbage").unwrap();
        snapshot_in(&mut state, &scratch.files);
        assert_eq!(
            std::fs::read_to_string(&scratch.files.index).unwrap(),
            sample_index(AT + 1)
        );
        let corrupt = scratch.dir.join("badge_index.json.corrupt");
        assert_eq!(std::fs::read_to_string(corrupt).unwrap(), "garbage");
        assert!(state.unsaved.is_none());
    }

    #[test]
    fn an_unsaved_held_copy_never_overwrites_a_newer_disk_copy() {
        let scratch = Scratch::new("unsaved-vs-newer-disk");
        std::fs::create_dir(&scratch.files.index).unwrap();
        let mut state = State::default();
        assert!(install_text(&mut state, &scratch.files, &sample_index(AT)));
        assert!(state.unsaved.is_some());
        // The file becomes readable and holds a NEWER copy than the one held.
        std::fs::remove_dir(&scratch.files.index).unwrap();
        std::fs::write(&scratch.files.index, sample_index(AT + 1)).unwrap();
        snapshot_in(&mut state, &scratch.files);
        assert_eq!(
            std::fs::read_to_string(&scratch.files.index).unwrap(),
            sample_index(AT + 1)
        );
        assert_eq!(state.index.as_ref().unwrap().at, AT + 1);
        assert!(state.unsaved.is_none());
    }

    #[test]
    fn a_failed_save_is_retried_by_the_next_call_even_with_nothing_newer() {
        let scratch = Scratch::new("save-retry");
        let mut state = State::default();
        // Loaded with no file; then a folder blocks the rename of the first save.
        snapshot_in(&mut state, &scratch.files);
        std::fs::create_dir(&scratch.files.index).unwrap();
        assert!(install_text(&mut state, &scratch.files, &sample_index(AT)));
        assert!(state.unsaved.is_some());
        std::fs::remove_dir(&scratch.files.index).unwrap();
        // The same `at` again brings nothing newer, but the held copy is saved.
        assert!(!install_text(&mut state, &scratch.files, &sample_index(AT)));
        assert_eq!(std::fs::read_to_string(&scratch.files.index).unwrap(), sample_index(AT));
        assert!(state.unsaved.is_none());
    }

    #[test]
    fn an_older_fetch_never_replaces_the_held_copy_while_the_file_is_unreadable() {
        let scratch = Scratch::new("unreadable-older");
        std::fs::create_dir(&scratch.files.index).unwrap();
        let mut state = State::default();
        assert!(install_text(&mut state, &scratch.files, &sample_index(AT + 1)));
        assert!(!install_text(&mut state, &scratch.files, &sample_index(AT)));
        assert_eq!(state.index.as_ref().unwrap().at, AT + 1);
    }

    #[test]
    fn temp_files_another_process_left_are_swept_and_this_ones_are_not() {
        let scratch = Scratch::new("sweep");
        let mine = std::process::id();
        let stale_index = scratch.dir.join("badge_index.json.1.0.tmp");
        let stale_feed = scratch.dir.join("badge_first_seen_feed.json.1.7.tmp");
        let own = scratch.dir.join(format!("badge_index.json.{mine}.9.tmp"));
        let other = scratch.dir.join("something_else.json.1.0.tmp");
        for path in [&stale_index, &stale_feed, &own, &other] {
            std::fs::write(path, "x").unwrap();
        }
        let mut state = State::default();
        snapshot_in(&mut state, &scratch.files);
        assert!(!stale_index.exists());
        assert!(!stale_feed.exists());
        assert!(own.exists());
        assert!(other.exists());
    }

    #[test]
    fn the_label_never_asks_for_the_local_zone() {
        let src = include_str!("badge_index.rs").replace("\r\n", "\n");
        let start = src.find("pub fn label(").unwrap();
        let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
        assert!(!body.contains("Local"));
        assert!(!body.contains("localtime"));
    }

    #[test]
    fn the_android_app_runs_in_one_process_so_the_one_lock_covers_the_worker() {
        // The notification worker shares the app's process only while the manifest
        // declares no separate one for it. A separate process would make the
        // static lock per process and the feed file a two-writer file.
        let manifest = include_str!("../../gen/android/app/src/main/AndroidManifest.xml");
        assert!(!manifest.contains("android:process"));
    }

    #[test]
    fn the_start_hook_and_the_fetch_outcome_are_in_place() {
        let lib = include_str!("../lib.rs").replace("\r\n", "\n");
        assert!(lib.contains("services::badge_index::start(app_handle.clone());"));
        let src = include_str!("badge_index.rs").replace("\r\n", "\n");
        let start = src.find("async fn refresh_once(").unwrap();
        let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
        let replaced = body.find("if replaced {").unwrap();
        assert!(body[replaced..].contains("notify(app)"));
        // The parse is done with the install, on the blocking pool.
        let spawn = body.find("spawn_blocking(").unwrap();
        assert!(body[spawn..].contains("parse_index(&bytes)"));
    }

    // ---- the order while no index is held ----

    #[test]
    fn the_never_loaded_order_leads_with_feed_dates_then_positions_then_the_key() {
        let keys: Vec<String> = [
            "rest-b-v1",
            "old-v1",
            "new-v1",
            "mid-v1",
            "pos-v1",
            "rest-a-v1",
            "tie-v1",
        ]
        .iter()
        .map(|k| k.to_string())
        .collect();
        let feed = HashMap::from([
            ("new-v1".to_string(), dated(T_NEW, Precision::Minute)),
            ("mid-v1".to_string(), dated(T_DAY, Precision::Day)),
            ("tie-v1".to_string(), dated(T_DAY, Precision::Minute)),
        ]);
        let positions = HashMap::from([("old-v1".to_string(), 4u32), ("pos-v1".to_string(), 1u32)]);
        // Feed dates lead (newest, then finer precision at one instant), then
        // positions ascending, then the rest by key.
        assert_eq!(
            fallback_order(&keys, &feed, &positions),
            ["new-v1", "tie-v1", "mid-v1", "pos-v1", "old-v1", "rest-a-v1", "rest-b-v1"]
        );
        // A fresh Android install has no positions: feed-dated badges still lead.
        assert_eq!(
            fallback_order(&keys, &feed, &HashMap::new()),
            ["new-v1", "tie-v1", "mid-v1", "old-v1", "pos-v1", "rest-a-v1", "rest-b-v1"]
        );
        // Nothing known at all: by key.
        assert_eq!(
            fallback_order(&keys, &HashMap::new(), &HashMap::new()),
            ["mid-v1", "new-v1", "old-v1", "pos-v1", "rest-a-v1", "rest-b-v1", "tie-v1"]
        );
    }
}
