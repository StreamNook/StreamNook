// Per-channel moderation-log persistence.
//
// Mirrors whisper_storage_service: a single JSON file in the app data dir, keyed
// by lowercase channel login -> a capped list of mod-log entries. Entries are
// stored as opaque JSON values so this layer never has to track the frontend's
// ModLogEvent shape. The point is durability + bounded RAM: the live UI keeps
// only the channels you're currently viewing in memory, and reloads a channel's
// recent history from here when you open it again, instead of holding every
// moderation event for the whole session.
//
// Persistence: an in-memory store seeded from disk ONCE per session (the file
// plus the journal of actions recorded since its last write), each recorded
// action appended to the journal before its command returns, and a background
// task that folds the store into the file every 30 s and empties the journal
// (flush_now does the same for shutdown paths). A crash loses nothing, and a
// busy moderated chat costs a line per action instead of a rewrite of every
// channel's log. Actions recorded while a write is in flight queue up and go
// to disk together, so a burst costs one sync per batch, not one per action.

use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use crate::rt::AppHandle;
use tauri::Manager;

const MOD_LOGS_FILE: &str = "mod_logs.json";
/// Actions recorded since the file's last write, one JSON line each.
const MOD_LOGS_JOURNAL: &str = "mod_logs.journal";
/// How often the store is folded into the file (and the journal emptied).
const FOLD_EVERY: std::time::Duration = std::time::Duration::from_secs(30);
// Keep at most this many entries per channel on disk. Mod actions are
// infrequent, so this is plenty of history without unbounded growth.
const MAX_PER_CHANNEL: usize = 500;

// In-memory store, lazily seeded from disk on first touch (the session's one
// full-file read). None until seeded.
static STORE: OnceLock<Mutex<Option<ModLogStorage>>> = OnceLock::new();
// Resolved once at seed time so flush paths never need an AppHandle.
static STORE_PATH: OnceLock<PathBuf> = OnceLock::new();
static DIRTY: AtomicBool = AtomicBool::new(false);
static JOURNAL_PATH: OnceLock<PathBuf> = OnceLock::new();
/// Held by every journal enqueue (with the store change it records) and by
/// every fold, so a fold never empties the journal of a line its snapshot
/// does not hold. It also guards the queue of lines waiting for the disk.
static JOURNAL_LOCK: Mutex<JournalQueue> = Mutex::new(JournalQueue::new());
/// Signaled whenever a write ends, so waiters re-check their line.
static JOURNAL_DONE: Condvar = Condvar::new();
/// The sequence number of the last journal line written; the next line takes
/// one more. Only advanced under `JOURNAL_LOCK`.
static LAST_SEQ: AtomicU64 = AtomicU64::new(0);
static FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);

fn store() -> &'static Mutex<Option<ModLogStorage>> {
    STORE.get_or_init(|| Mutex::new(None))
}

fn mark_dirty() {
    DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
}

fn ensure_flush_task() {
    if FLUSH_TASK_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // No runtime yet: reset the flag so a later write retries; DIRTY stays
        // set, so nothing is lost (the exit flush is the last-resort net).
        if tokio::runtime::Handle::try_current().is_err() {
            FLUSH_TASK_STARTED.store(false, Ordering::SeqCst);
            return;
        }
        tokio::spawn(async {
            loop {
                tokio::time::sleep(FOLD_EVERY).await;
                if !DIRTY.load(Ordering::Acquire) {
                    continue;
                }
                let flushed = tokio::task::spawn_blocking(ModLogStorageService::flush_now).await;
                match flushed {
                    Ok(Ok(())) => {}
                    // flush_now restored DIRTY itself on failure.
                    Ok(Err(e)) => {
                        debug!("[ModLogStorage] debounced flush failed (will retry): {}", e)
                    }
                    Err(_) => DIRTY.store(true, Ordering::Release),
                }
            }
        });
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ModLogStorage {
    /// lowercase channel login -> chronological list of entries (oldest first)
    pub channels: HashMap<String, Vec<serde_json::Value>>,
    pub version: i32,
    /// The sequence number of the last journal line this snapshot holds. A
    /// replay applies only lines past it, so a journal that outlived its fold
    /// (a crash between the rename and the truncate) is never applied twice.
    #[serde(default)]
    pub journal_seq: u64,
}

pub struct ModLogStorageService;

impl ModLogStorageService {
    fn get_storage_path(app_handle: &AppHandle) -> Result<PathBuf, String> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get app data directory: {}", e))?;
        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir)
                .map_err(|e| format!("Failed to create app data directory: {}", e))?;
        }
        Ok(app_data_dir.join(MOD_LOGS_FILE))
    }

    /// Lock the store, seeding it from disk on first touch, and run `f` on it.
    fn with_store<R>(
        app_handle: &AppHandle,
        f: impl FnOnce(&mut ModLogStorage) -> R,
    ) -> Result<R, String> {
        Self::with_store_at(|| Self::get_storage_path(app_handle), f)
    }

    /// `with_store` with the snapshot path supplied by `resolve_path`, which
    /// only runs when the store still has to be seeded.
    fn with_store_at<R>(
        resolve_path: impl FnOnce() -> Result<PathBuf, String>,
        f: impl FnOnce(&mut ModLogStorage) -> R,
    ) -> Result<R, String> {
        let mut guard = store().lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            let path = resolve_path()?;
            let journal = path.with_file_name(MOD_LOGS_JOURNAL);
            let (storage, replayed) = load_storage(&path, &journal);
            let _ = STORE_PATH.set(path);
            let _ = JOURNAL_PATH.set(journal);
            LAST_SEQ.store(storage.journal_seq, Ordering::SeqCst);
            *guard = Some(storage);
            // Actions recorded after the last fold (a crash, or a kill before the
            // exit flush): the next fold writes them into the file.
            if replayed > 0 {
                debug!("[ModLogStorage] replayed {} journal lines", replayed);
                mark_dirty();
            }
        }
        Ok(f(guard.as_mut().expect("seeded above")))
    }

    /// Synchronous write-if-dirty, for shutdown paths.
    pub fn flush_now() -> Result<(), String> {
        // Held to the end: nothing is journaled between the snapshot below and
        // the journal being emptied after the write.
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A write already in flight finishes first; no new one can start while
        // this holds the lock.
        while queue.writing {
            queue = JOURNAL_DONE.wait(queue).unwrap_or_else(|e| e.into_inner());
        }
        if !DIRTY.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        // DIRTY is only ever set after a seed stored the path.
        let Some(path) = STORE_PATH.get() else {
            return Ok(());
        };
        let json = {
            let mut guard = match store().lock() {
                Ok(g) => g,
                Err(e) => {
                    DIRTY.store(true, Ordering::Release);
                    return Err(e.to_string());
                }
            };
            let Some(storage) = guard.as_mut() else {
                return Ok(());
            };
            // Every line journaled so far is in this snapshot.
            storage.journal_seq = LAST_SEQ.load(Ordering::SeqCst);
            match serde_json::to_string(&*storage) {
                Ok(j) => j,
                Err(e) => {
                    DIRTY.store(true, Ordering::Release);
                    return Err(format!("Failed to serialize mod logs: {}", e));
                }
            }
        };
        if let Err(e) = write_snapshot(path, &json) {
            DIRTY.store(true, Ordering::Release);
            return Err(format!("Failed to write mod logs file: {}", e));
        }
        // Everything journaled is in the file now, including lines still
        // waiting for the disk, whose callers are released here.
        queue.pending.clear();
        queue.durable_seq = LAST_SEQ.load(Ordering::SeqCst);
        JOURNAL_DONE.notify_all();
        // A truncate that fails leaves lines the file's journal_seq already
        // covers, which a replay skips.
        if let Some(journal) = JOURNAL_PATH.get() {
            match fs::write(journal, b"") {
                // An empty journal has no torn line to guard against.
                Ok(()) => queue.torn = false,
                Err(e) => warn!("[ModLogStorage] could not empty the journal after a fold: {}", e),
            }
        }
        Ok(())
    }

    /// Load one channel's persisted entries (oldest first). Empty if none.
    pub fn load_channel(app_handle: &AppHandle, channel: &str) -> Vec<serde_json::Value> {
        let key = channel.to_lowercase();
        Self::with_store(app_handle, |storage| {
            storage.channels.get(&key).cloned().unwrap_or_default()
        })
        .unwrap_or_default()
    }

    /// Record one moderation action, merging it with the same action reported
    /// by the other feed, and return the entry every window should show.
    ///
    /// IRC (CLEARCHAT/CLEARMSG/NOTICE) and EventSub `channel.moderate` both
    /// report each action: IRC is universal but anonymous, EventSub names the
    /// moderator. Within DEDUP_WINDOW_MS, the same channel + action + target is
    /// one action. A richer EventSub report upgrades an IRC entry in place
    /// (keeping its id, and the message or reason IRC captured that EventSub
    /// does not carry); anything else is a duplicate and the stored entry wins.
    /// Deciding this here, once, is what keeps two windows showing the same
    /// channel from each persisting their own copy of every action.
    pub fn record(
        app_handle: &AppHandle,
        channel: &str,
        entry: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Self::record_at(|| Self::get_storage_path(app_handle), channel, entry)
    }

    /// `record` with the snapshot path supplied by `resolve_path`.
    fn record_at(
        resolve_path: impl FnOnce() -> Result<PathBuf, String>,
        channel: &str,
        entry: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let key = channel.to_lowercase();
        if key.is_empty() {
            return Ok(entry);
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let channel_key = key.clone();
        let (shown, changed) = Self::with_store_at(resolve_path, move |storage| {
            merge_into(storage.channels.entry(key).or_default(), entry, now_ms)
        })?;
        if changed {
            let seq = journal_append(&mut queue, &JournalOp::Put { channel: channel_key, entry: shown.clone() });
            mark_dirty();
            if let (Some(seq), Some(path)) = (seq, JOURNAL_PATH.get()) {
                // The action is in the store and the store is dirty, so a journal
                // that cannot be written only costs the crash window: the fold
                // saves the action, and the caller is not told it failed.
                wait_or_fold(&JOURNAL_LOCK, &JOURNAL_DONE, path, queue, seq, commit_lines, Self::flush_now);
            }
        }
        Ok(shown)
    }

    /// Clear one channel's persisted entries.
    pub fn clear_channel(app_handle: &AppHandle, channel: &str) -> Result<(), String> {
        Self::clear_channel_at(|| Self::get_storage_path(app_handle), channel)
    }

    /// `clear_channel` with the snapshot path supplied by `resolve_path`.
    fn clear_channel_at(
        resolve_path: impl FnOnce() -> Result<PathBuf, String>,
        channel: &str,
    ) -> Result<(), String> {
        let key = channel.to_lowercase();
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let removed =
            Self::with_store_at(resolve_path, |storage| storage.channels.remove(&key).is_some())?;
        if removed {
            debug!("[ModLogStorage] cleared {}", key);
            let seq = journal_append(&mut queue, &JournalOp::Clear { channel: key });
            mark_dirty();
            if let (Some(seq), Some(path)) = (seq, JOURNAL_PATH.get()) {
                wait_or_fold(&JOURNAL_LOCK, &JOURNAL_DONE, path, queue, seq, commit_lines, Self::flush_now);
            }
        }
        Ok(())
    }
}

/// One journal line.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum JournalOp {
    /// The entry to show for `channel`: a new one, or the upgrade of the entry
    /// with its id.
    Put { channel: String, entry: serde_json::Value },
    /// `channel` was cleared.
    Clear { channel: String },
}

/// A journal line as read: the operation and its sequence number.
#[derive(Debug, Deserialize)]
struct JournalLine {
    #[serde(default)]
    seq: u64,
    #[serde(flatten)]
    op: JournalOp,
}

/// A journal line as written; reads back as a `JournalLine`.
#[derive(Serialize)]
struct JournalLineOut<'a> {
    seq: u64,
    #[serde(flatten)]
    op: &'a JournalOp,
}

/// Write `json` to `path` through a sibling that is flushed to disk and then
/// renamed over the file, so a crash or power loss mid-write never leaves a
/// truncated or empty file for the next seed to read.
fn write_snapshot(path: &Path, json: &str) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(json.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)
}

/// Journal lines waiting for the disk, and the state of the write that carries
/// them. Guarded by `JOURNAL_LOCK`.
struct JournalQueue {
    /// Serialized lines (newline included) with their sequence numbers, oldest
    /// first, not yet on disk.
    pending: Vec<(u64, String)>,
    /// A writer has taken a batch and has not finished with it.
    writing: bool,
    /// Every line up to this sequence number is on disk or in the file.
    durable_seq: u64,
    /// The journal may end in a line with no newline: a write failed, or a
    /// crash left a fragment that startup could not cut. The next write starts
    /// with a newline, so its first line is never glued onto the fragment.
    /// Replay skips the blank line and the fragment. A new queue starts torn,
    /// so a session's first write is safe whatever it finds.
    torn: bool,
}

impl JournalQueue {
    const fn new() -> Self {
        Self { pending: Vec::new(), writing: false, durable_seq: 0, torn: true }
    }
}

/// One journal line as written: JSON and a closing newline.
fn journal_line(seq: u64, op: &JournalOp) -> Result<String, String> {
    let mut line = serde_json::to_string(&JournalLineOut { seq, op }).map_err(|e| e.to_string())?;
    line.push('\n');
    Ok(line)
}

/// Append `lines` to the journal with one write and one sync. A failed write
/// cuts whatever torn line it left, so the next append starts a clean line.
fn commit_lines(path: &Path, lines: &[(u64, String)]) -> Result<(), String> {
    use std::io::Write;
    let bytes: Vec<u8> = lines.iter().flat_map(|(_, line)| line.as_bytes().iter().copied()).collect();
    let written = fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut file| {
        file.write_all(&bytes)?;
        file.sync_data()
    });
    if let Err(e) = written {
        cut_torn_tail(path);
        return Err(e.to_string());
    }
    Ok(())
}

/// Append one line to the journal and flush it to disk.
#[cfg(test)]
fn append_journal(path: &Path, seq: u64, op: &JournalOp) -> Result<(), String> {
    commit_lines(path, &[(seq, journal_line(seq, op)?)])
}

/// Queue a line for this session's journal and return its sequence number.
/// Called with `JOURNAL_LOCK` held (`queue` is what it guards), which is what
/// orders the sequence numbers; `wait_or_fold` then carries the line to disk.
/// `None` when there is no journal or the line cannot be serialized: the store
/// is dirty, so the next fold writes the action anyway.
fn journal_append(queue: &mut JournalQueue, op: &JournalOp) -> Option<u64> {
    JOURNAL_PATH.get()?;
    let seq = LAST_SEQ.load(Ordering::SeqCst) + 1;
    match journal_line(seq, op) {
        Ok(line) => {
            LAST_SEQ.store(seq, Ordering::SeqCst);
            queue.pending.push((seq, line));
            Some(seq)
        }
        Err(e) => {
            debug!("[ModLogStorage] journal line not serialized: {}", e);
            None
        }
    }
}

/// Return once line `seq` is on disk. The first caller that finds no write in
/// flight takes every pending line and writes them with one `commit` (one
/// sync), outside the lock; the others wait for it. A waiter that wakes to find
/// no writer and its line still pending becomes the writer. A failed write puts
/// its lines back at the front of the queue, marks the journal torn so the next
/// write starts on a fresh line, and returns the error to that writer's own
/// caller; the lines are written by the next writer.
fn wait_durable<'a>(
    lock: &'a Mutex<JournalQueue>,
    done: &Condvar,
    path: &Path,
    mut queue: MutexGuard<'a, JournalQueue>,
    seq: u64,
    commit: impl Fn(&Path, &[(u64, String)]) -> Result<(), String>,
) -> Result<(), String> {
    loop {
        if queue.durable_seq >= seq {
            return Ok(());
        }
        if queue.writing {
            queue = done.wait(queue).unwrap_or_else(|e| e.into_inner());
            continue;
        }
        let mut batch = std::mem::take(&mut queue.pending);
        if batch.is_empty() {
            // A fold took the line into the file.
            return Ok(());
        }
        if queue.torn {
            if let Some((_, first)) = batch.first_mut() {
                if !first.starts_with('\n') {
                    first.insert(0, '\n');
                }
            }
        }
        queue.writing = true;
        drop(queue);
        let written = commit(path, &batch);
        queue = lock.lock().unwrap_or_else(|e| e.into_inner());
        queue.writing = false;
        match &written {
            Ok(()) => {
                queue.torn = false;
                if let Some((last, _)) = batch.last() {
                    queue.durable_seq = queue.durable_seq.max(*last);
                }
            }
            Err(_) => {
                queue.torn = true;
                let newer = std::mem::replace(&mut queue.pending, batch);
                queue.pending.extend(newer);
            }
        }
        done.notify_all();
        written?;
    }
}

/// Wait until line `seq` is durable (`wait_durable`, which releases the journal
/// lock). A journal that cannot be written is never an error for the caller:
/// the store is already dirty, so the failure is logged and `fold` saves the
/// action in the snapshot instead. `fold` runs after the journal lock is
/// released; a fold that fails is logged too.
fn wait_or_fold<'a>(
    lock: &'a Mutex<JournalQueue>,
    done: &Condvar,
    path: &Path,
    queue: MutexGuard<'a, JournalQueue>,
    seq: u64,
    commit: impl Fn(&Path, &[(u64, String)]) -> Result<(), String>,
    fold: impl FnOnce() -> Result<(), String>,
) {
    if let Err(e) = wait_durable(lock, done, path, queue, seq, commit) {
        warn!("[ModLogStorage] journal write failed, the next fold saves the action: {}", e);
        if let Err(e) = fold() {
            warn!("[ModLogStorage] flush after a failed journal write failed: {}", e);
        }
    }
}

/// Apply one journal line: a put replaces the entry with the same id (an
/// upgrade) or is appended and capped like `merge_into`; a clear removes the
/// channel.
fn apply_journal_op(storage: &mut ModLogStorage, op: JournalOp) {
    match op {
        JournalOp::Put { channel, entry } => {
            let list = storage.channels.entry(channel).or_default();
            let id = entry.get("id").cloned().filter(|v| !v.is_null());
            match id.and_then(|id| list.iter().position(|e| e.get("id") == Some(&id))) {
                Some(i) => list[i] = entry,
                None => {
                    list.push(entry);
                    if list.len() > MAX_PER_CHANNEL {
                        let overflow = list.len() - MAX_PER_CHANNEL;
                        list.drain(0..overflow);
                    }
                }
            }
        }
        JournalOp::Clear { channel } => {
            storage.channels.remove(&channel);
        }
    }
}

/// Cut a torn last line (no closing newline: a crash mid-append) off the
/// journal, so the next append starts a fresh line instead of merging into it.
fn cut_torn_tail(journal: &Path) {
    let Ok(bytes) = fs::read(journal) else {
        return;
    };
    if bytes.is_empty() || bytes.ends_with(b"\n") {
        return;
    }
    let keep = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let cut = fs::OpenOptions::new().write(true).open(journal).and_then(|file| {
        file.set_len(keep as u64)?;
        file.sync_all()
    });
    if let Err(e) = cut {
        warn!("[ModLogStorage] could not cut a torn journal line: {}", e);
    }
}

/// The store from `snapshot` plus every readable journal line past the
/// snapshot's `journal_seq`, and how many lines it applied. A torn last line
/// from a crash is cut off the journal first.
fn load_storage(snapshot: &Path, journal: &Path) -> (ModLogStorage, usize) {
    let mut storage: ModLogStorage = match fs::read_to_string(snapshot) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => ModLogStorage::default(),
    };
    cut_torn_tail(journal);
    let mut applied = 0;
    // Read as bytes and split on newlines: a fragment that ends inside a
    // multi-byte character is not valid UTF-8 and fails only its own line.
    if let Ok(bytes) = fs::read(journal) {
        for line in bytes.split(|b| *b == b'\n').filter(|l| !l.iter().all(|b| b.is_ascii_whitespace())) {
            if let Ok(JournalLine { seq, op }) = serde_json::from_slice::<JournalLine>(line) {
                if seq > storage.journal_seq {
                    apply_journal_op(&mut storage, op);
                    storage.journal_seq = seq;
                    applied += 1;
                }
            }
        }
    }
    (storage, applied)
}

/// Same action inside this window = one action reported twice.
const DEDUP_WINDOW_MS: i64 = 5000;
/// Duplicates arrive within seconds of each other, so only the newest entries
/// can match.
const DEDUP_SCAN: usize = 64;

fn text<'a>(entry: &'a serde_json::Value, field: &str) -> &'a str {
    entry.get(field).and_then(|v| v.as_str()).unwrap_or_default()
}

fn dedup_key(entry: &serde_json::Value) -> String {
    let action = text(entry, "action").to_lowercase();
    let action = if action == "clear_chat" { "clear".to_string() } else { action };
    format!(
        "{}|{}|{}",
        text(entry, "channel").to_lowercase(),
        action,
        text(entry, "target_user_name").to_lowercase()
    )
}

fn timestamp_ms(entry: &serde_json::Value) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text(entry, "timestamp"))
        .ok()
        .map(|t| t.timestamp_millis())
}

/// Apply the merge rule to one channel's list (oldest first). Returns the entry
/// to show and whether the list changed.
fn merge_into(
    list: &mut Vec<serde_json::Value>,
    mut entry: serde_json::Value,
    now_ms: i64,
) -> (serde_json::Value, bool) {
    // The raw payload is never rendered; keep the stored and shown copy lean.
    if let Some(fields) = entry.as_object_mut() {
        fields.remove("details");
    }
    let key = dedup_key(&entry);
    let duplicate = list.iter().rev().take(DEDUP_SCAN).position(|existing| {
        dedup_key(existing) == key
            && timestamp_ms(existing).is_some_and(|ts| now_ms - ts < DEDUP_WINDOW_MS)
    });
    if let Some(from_end) = duplicate {
        let index = list.len() - 1 - from_end;
        let upgrades = text(&entry, "source") == "eventsub" && text(&list[index], "source") != "eventsub";
        if !upgrades {
            return (list[index].clone(), false);
        }
        let existing = &list[index];
        if let Some(fields) = entry.as_object_mut() {
            fields.insert("id".into(), existing.get("id").cloned().unwrap_or_default());
            for kept in ["message", "reason"] {
                let missing = fields.get(kept).is_none_or(|v| v.is_null());
                if missing {
                    if let Some(v) = existing.get(kept).filter(|v| !v.is_null()) {
                        fields.insert(kept.into(), v.clone());
                    }
                }
            }
        }
        list[index] = entry.clone();
        return (entry, true);
    }
    list.push(entry.clone());
    if list.len() > MAX_PER_CHANNEL {
        let overflow = list.len() - MAX_PER_CHANNEL;
        list.drain(0..overflow);
    }
    (entry, true)
}

/// (channels, entries) held in the in-memory mod-log store; `None` before the
/// first seed or while locked. Diagnostics for the resource line.
pub fn cache_counts() -> Option<(usize, usize)> {
    let store = STORE.get()?.try_lock().ok()?;
    let inner = store.as_ref()?;
    Some((inner.channels.len(), inner.channels.values().map(|v| v.len()).sum()))
}

#[cfg(test)]
mod merge_tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_700_000_000_000;

    fn at(offset_ms: i64) -> String {
        chrono::DateTime::from_timestamp_millis(NOW + offset_ms).unwrap().to_rfc3339()
    }

    fn irc(id: &str, offset_ms: i64) -> serde_json::Value {
        json!({ "id": id, "action": "ban", "channel": "Chan", "target_user_name": "Bob",
                "timestamp": at(offset_ms), "source": "irc", "message": "last words",
                "details": { "raw": true } })
    }

    fn eventsub(id: &str, offset_ms: i64) -> serde_json::Value {
        json!({ "id": id, "action": "ban", "channel": "chan", "target_user_name": "bob",
                "timestamp": at(offset_ms), "source": "eventsub", "moderator_name": "Mod" })
    }

    #[test]
    fn a_second_windows_irc_report_resolves_to_the_first() {
        let mut list = Vec::new();
        let (first, changed) = merge_into(&mut list, irc("w1-random", 0), NOW);
        assert!(changed);
        assert!(first.get("details").is_none());
        let (second, changed) = merge_into(&mut list, irc("w2-random", 100), NOW + 100);
        assert!(!changed);
        assert_eq!(second["id"], "w1-random");
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn eventsub_upgrades_irc_in_place_and_keeps_what_irc_knew() {
        let mut list = vec![];
        merge_into(&mut list, irc("irc-1", 0), NOW);
        let (shown, changed) = merge_into(&mut list, eventsub("es-1", 50), NOW + 50);
        assert!(changed);
        assert_eq!(shown["id"], "irc-1");
        assert_eq!(shown["moderator_name"], "Mod");
        assert_eq!(shown["message"], "last words");
        assert_eq!(list.len(), 1);
        // A later IRC echo of the same action is a duplicate of the upgraded one.
        let (echo, changed) = merge_into(&mut list, irc("irc-2", 80), NOW + 80);
        assert!(!changed);
        assert_eq!(echo["source"], "eventsub");
    }

    #[test]
    fn the_same_action_later_is_a_new_action() {
        let mut list = vec![];
        merge_into(&mut list, irc("a", 0), NOW);
        let (_, changed) = merge_into(&mut list, irc("b", 6000), NOW + 6000);
        assert!(changed);
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn clear_chat_and_clear_are_one_action() {
        let mut list = vec![];
        let clear = |id: &str, action: &str| json!({ "id": id, "action": action, "channel": "c",
            "timestamp": at(0), "source": "irc" });
        merge_into(&mut list, clear("a", "clear"), NOW);
        let (shown, changed) = merge_into(&mut list, clear("b", "CLEAR_CHAT"), NOW);
        assert!(!changed);
        assert_eq!(shown["id"], "a");
    }
}

#[cfg(test)]
mod journal_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;

    /// Held for the whole of any test that touches the process-wide store,
    /// `JOURNAL_PATH` or `JOURNAL_LOCK`, so such tests never run at once.
    static GLOBALS_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// A commit that writes the first half of its batch and a fragment of the
    /// next line, then fails, like a disk that dies partway through a write.
    fn torn_commit(path: &Path, lines: &[(u64, String)]) -> Result<(), String> {
        use std::io::Write;
        let whole = lines.len() / 2;
        let mut bytes: Vec<u8> = lines[..whole].iter().flat_map(|(_, line)| line.as_bytes().iter().copied()).collect();
        if let Some((_, next)) = lines.get(whole) {
            bytes.extend_from_slice(&next.as_bytes()[..next.len() / 2]);
        }
        let mut file = fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        Err("injected write failure".to_string())
    }

    /// Queue the line `seq` (an entry with id `a{seq}`) and return the held lock.
    fn enqueue(lock: &Mutex<JournalQueue>, seq: u64) -> MutexGuard<'_, JournalQueue> {
        let op = JournalOp::Put { channel: "chan".into(), entry: json!({ "id": format!("a{seq}") }) };
        let mut queue = lock.lock().unwrap();
        queue.pending.push((seq, journal_line(seq, &op).unwrap()));
        queue
    }

    fn temp_paths(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sn-modlog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        (dir.join(MOD_LOGS_FILE), dir.join(MOD_LOGS_JOURNAL))
    }

    /// Removes a test's folder when dropped, so a failed assertion leaves nothing behind.
    struct TempDirGuard(PathBuf);

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn read_snapshot(snapshot: &Path) -> ModLogStorage {
        serde_json::from_str(&fs::read_to_string(snapshot).unwrap()).unwrap()
    }

    #[test]
    fn an_action_recorded_and_never_flushed_is_there_after_a_restart() {
        let (snapshot, journal) = temp_paths("kill");
        // A session records one action and dies before any fold.
        let mut live = ModLogStorage::default();
        let action = json!({ "id": "a1", "action": "ban", "channel": "chan", "target_user_name": "bob",
            "timestamp": "2026-10-24T00:00:00Z", "source": "irc" });
        let (shown, changed) = merge_into(live.channels.entry("chan".into()).or_default(), action, 0);
        assert!(changed);
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: shown }).unwrap();
        // The next launch has no file yet, only the journal.
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 1);
        assert_eq!(restored.channels["chan"][0]["id"], "a1");
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn an_upgrade_replaces_by_id_and_a_clear_removes_the_channel() {
        let (snapshot, journal) = temp_paths("ops");
        let saved = ModLogStorage {
            channels: [("chan".to_string(), vec![json!({ "id": "a1", "source": "irc" })])].into_iter().collect(),
            version: 0,
            journal_seq: 0,
        };
        fs::write(&snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1", "source": "eventsub" }) }).unwrap();
        append_journal(&journal, 2, &JournalOp::Put { channel: "other".into(), entry: json!({ "id": "b1" }) }).unwrap();
        append_journal(&journal, 3, &JournalOp::Clear { channel: "other".into() }).unwrap();
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 3);
        assert_eq!(restored.channels["chan"].len(), 1);
        assert_eq!(restored.channels["chan"][0]["source"], "eventsub");
        assert!(!restored.channels.contains_key("other"));
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn a_torn_last_line_is_skipped() {
        let (snapshot, journal) = temp_paths("torn");
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1" }) }).unwrap();
        let mut f = fs::OpenOptions::new().append(true).open(&journal).unwrap();
        std::io::Write::write_all(&mut f, b"{\"op\":\"put\",\"chan").unwrap();
        drop(f);
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 1);
        assert_eq!(restored.channels["chan"].len(), 1);
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn a_torn_line_never_merges_with_the_next_action() {
        let (snapshot, journal) = temp_paths("tear-merge");
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1" }) }).unwrap();
        let mut f = fs::OpenOptions::new().append(true).open(&journal).unwrap();
        std::io::Write::write_all(&mut f, b"{\"seq\":2,\"op\":\"put\",\"chan").unwrap();
        drop(f);
        // The seed cuts the fragment off before anything appends.
        let (_, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 1);
        append_journal(&journal, 2, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a2" }) }).unwrap();
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 2);
        let ids: Vec<_> = restored.channels["chan"].iter().map(|e| e["id"].clone()).collect();
        assert_eq!(ids, vec![json!("a1"), json!("a2")]);
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn a_burst_from_many_threads_lands_once_each_and_in_order() {
        let (snapshot, journal) = temp_paths("burst");
        let lock = Mutex::new(JournalQueue::new());
        let done = Condvar::new();
        let last_seq = AtomicU64::new(0);
        std::thread::scope(|scope| {
            for t in 0..8 {
                let (lock, done, last_seq, journal) = (&lock, &done, &last_seq, &journal);
                scope.spawn(move || {
                    for i in 0..200 {
                        let op = JournalOp::Put { channel: format!("t{t}"), entry: json!({ "id": format!("t{t}-{i}") }) };
                        let mut queue = lock.lock().unwrap();
                        let seq = last_seq.load(Ordering::SeqCst) + 1;
                        last_seq.store(seq, Ordering::SeqCst);
                        queue.pending.push((seq, journal_line(seq, &op).unwrap()));
                        wait_durable(lock, done, journal, queue, seq, commit_lines).unwrap();
                    }
                });
            }
        });
        let queue = lock.lock().unwrap();
        assert!(queue.pending.is_empty());
        assert!(!queue.writing);
        assert_eq!(queue.durable_seq, 1600);
        drop(queue);
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 1600);
        for t in 0..8 {
            let ids: Vec<_> = restored.channels[&format!("t{t}")].iter().map(|e| e["id"].clone()).collect();
            let expected: Vec<_> = (0..200).map(|i| json!(format!("t{t}-{i}"))).collect();
            assert_eq!(ids, expected);
        }
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn a_burst_with_failing_writes_lands_once_each_and_in_order() {
        let (snapshot, journal) = temp_paths("burst-failing");
        let lock = Mutex::new(JournalQueue::new());
        let done = Condvar::new();
        let last_seq = AtomicU64::new(0);
        let commits = AtomicUsize::new(0);
        let failed_commits = AtomicUsize::new(0);
        let observed_failures = AtomicUsize::new(0);
        // Every third commit writes half its batch and fails.
        let commit = |path: &Path, lines: &[(u64, String)]| {
            if commits.fetch_add(1, Ordering::SeqCst) % 3 == 2 {
                failed_commits.fetch_add(1, Ordering::SeqCst);
                torn_commit(path, lines)
            } else {
                commit_lines(path, lines)
            }
        };
        std::thread::scope(|scope| {
            for t in 0..8 {
                let (lock, done, last_seq, journal, commit, observed_failures) =
                    (&lock, &done, &last_seq, &journal, &commit, &observed_failures);
                scope.spawn(move || {
                    for i in 0..200 {
                        let op = JournalOp::Put { channel: format!("t{t}"), entry: json!({ "id": format!("t{t}-{i}") }) };
                        let mut queue = lock.lock().unwrap();
                        let seq = last_seq.load(Ordering::SeqCst) + 1;
                        last_seq.store(seq, Ordering::SeqCst);
                        queue.pending.push((seq, journal_line(seq, &op).unwrap()));
                        // A failed write is the failing writer's to see; the line
                        // stays queued and the record carries on.
                        if wait_durable(lock, done, journal, queue, seq, commit).is_err() {
                            observed_failures.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                });
            }
        });
        assert!(failed_commits.load(Ordering::SeqCst) > 0);
        assert_eq!(observed_failures.load(Ordering::SeqCst), failed_commits.load(Ordering::SeqCst));
        // Lines a failed write left queued go out with the next write.
        for _ in 0..4 {
            let queue = lock.lock().unwrap();
            if wait_durable(&lock, &done, &journal, queue, 1600, &commit).is_ok() {
                break;
            }
        }
        let queue = lock.lock().unwrap();
        assert!(queue.pending.is_empty());
        assert!(!queue.writing);
        assert_eq!(queue.durable_seq, 1600);
        drop(queue);
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 1600);
        for t in 0..8 {
            let ids: Vec<_> = restored.channels[&format!("t{t}")].iter().map(|e| e["id"].clone()).collect();
            let expected: Vec<_> = (0..200).map(|i| json!(format!("t{t}-{i}"))).collect();
            assert_eq!(ids, expected);
        }
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn lines_queued_behind_a_write_in_flight_go_out_in_one_commit() {
        const N: usize = 12;
        let (snapshot, journal) = temp_paths("batching");
        let lock = Mutex::new(JournalQueue::new());
        let done = Condvar::new();
        let last_seq = AtomicU64::new(0);
        let batches = Mutex::new(Vec::<usize>::new());
        let commit = |path: &Path, lines: &[(u64, String)]| {
            batches.lock().unwrap().push(lines.len());
            commit_lines(path, lines)
        };
        lock.lock().unwrap().writing = true;
        std::thread::scope(|scope| {
            for i in 0..N {
                let (lock, done, last_seq, journal, commit) = (&lock, &done, &last_seq, &journal, &commit);
                scope.spawn(move || {
                    let op = JournalOp::Put { channel: "chan".into(), entry: json!({ "id": format!("e{i}") }) };
                    let mut queue = lock.lock().unwrap();
                    let seq = last_seq.load(Ordering::SeqCst) + 1;
                    last_seq.store(seq, Ordering::SeqCst);
                    queue.pending.push((seq, journal_line(seq, &op).unwrap()));
                    wait_durable(lock, done, journal, queue, seq, commit).unwrap();
                });
            }
            // Every thread is waiting behind the write in flight; let it end.
            while lock.lock().unwrap().pending.len() < N {
                std::thread::yield_now();
            }
            let mut queue = lock.lock().unwrap();
            queue.writing = false;
            done.notify_all();
        });
        assert_eq!(*batches.lock().unwrap(), vec![N]);
        let queue = lock.lock().unwrap();
        assert!(queue.pending.is_empty());
        assert_eq!(queue.durable_seq, N as u64);
        drop(queue);
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, N);
        assert_eq!(restored.channels["chan"].len(), N);
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn a_write_after_a_failed_one_starts_on_a_fresh_line() {
        let (snapshot, journal) = temp_paths("torn-guard");
        let lock = Mutex::new(JournalQueue::new());
        let done = Condvar::new();
        let queue = enqueue(&lock, 1);
        assert!(wait_durable(&lock, &done, &journal, queue, 1, torn_commit).is_err());
        assert!(lock.lock().unwrap().torn);
        // The failed write left a fragment with no newline; the next write
        // carries the failed line again and must not be glued onto it.
        let queue = enqueue(&lock, 2);
        wait_durable(&lock, &done, &journal, queue, 2, commit_lines).unwrap();
        assert!(!lock.lock().unwrap().torn);
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 2);
        let ids: Vec<_> = restored.channels["chan"].iter().map(|e| e["id"].clone()).collect();
        assert_eq!(ids, vec![json!("a1"), json!("a2")]);
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn a_sessions_first_write_starts_on_a_fresh_line() {
        let (snapshot, journal) = temp_paths("first-write");
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        // A crash left a good line and a fragment with no newline, and nothing
        // cut it.
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1" }) }).unwrap();
        let mut f = fs::OpenOptions::new().append(true).open(&journal).unwrap();
        std::io::Write::write_all(&mut f, b"{\"seq\":2,\"op\":\"put\",\"chan").unwrap();
        drop(f);
        let lock = Mutex::new(JournalQueue::new());
        let done = Condvar::new();
        let queue = enqueue(&lock, 2);
        wait_durable(&lock, &done, &journal, queue, 2, commit_lines).unwrap();
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 2);
        let ids: Vec<_> = restored.channels["chan"].iter().map(|e| e["id"].clone()).collect();
        assert_eq!(ids, vec![json!("a1"), json!("a2")]);
    }

    /// The bytes of a journal line for entry `id` carrying `text` in `reason`,
    /// cut `keep` bytes into the first `marker` character found in it.
    fn line_cut_inside(seq: u64, id: &str, text: &str, marker: char, keep: usize) -> Vec<u8> {
        let op = JournalOp::Put { channel: "chan".into(), entry: json!({ "id": id, "reason": text }) };
        let line = journal_line(seq, &op).unwrap();
        let start = line.find(marker).unwrap();
        line.as_bytes()[..start + keep].to_vec()
    }

    #[test]
    fn a_fragment_ending_inside_a_multi_byte_character_fails_only_its_own_line() {
        for (name, text, marker, keep) in [("two-byte", "caf\u{e9}", '\u{e9}', 1), ("four-byte", "party \u{1F600}", '\u{1F600}', 2)] {
            let (snapshot, journal) = temp_paths(&format!("torn-{name}"));
            let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
            append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1" }) }).unwrap();
            // A cut line, then a newline and a whole line after it.
            let mut f = fs::OpenOptions::new().append(true).open(&journal).unwrap();
            std::io::Write::write_all(&mut f, &line_cut_inside(2, "torn", text, marker, keep)).unwrap();
            std::io::Write::write_all(&mut f, b"\n").unwrap();
            drop(f);
            append_journal(&journal, 2, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a2" }) }).unwrap();
            assert!(std::str::from_utf8(&fs::read(&journal).unwrap()).is_err(), "{name}: the fragment must be invalid UTF-8");
            let (restored, replayed) = load_storage(&snapshot, &journal);
            assert_eq!(replayed, 2, "{name}");
            let ids: Vec<_> = restored.channels["chan"].iter().map(|e| e["id"].clone()).collect();
            assert_eq!(ids, vec![json!("a1"), json!("a2")], "{name}");
        }
    }

    // The store, the journal path and the queue are process-wide and set once,
    // so this is the only test that goes through them; it covers both calls in
    // sequence. Any test that touches them takes GLOBALS_TEST_LOCK first.
    #[test]
    fn record_and_clear_return_ok_when_every_journal_commit_fails() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (snapshot, journal) = temp_paths("record-clear-failing-journal");
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        // A directory where the journal file belongs: every commit fails.
        fs::create_dir(&journal).unwrap();
        let entry = json!({ "id": "a1", "action": "ban", "channel": "Chan", "target_user_name": "bob",
            "timestamp": "2026-10-24T00:00:00Z", "source": "irc" });
        let shown = ModLogStorageService::record_at(|| Ok(snapshot.clone()), "Chan", entry).unwrap();
        assert_eq!(shown["id"], "a1");
        let saved = read_snapshot(&snapshot);
        assert_eq!(saved.journal_seq, 1);
        assert_eq!(saved.channels["chan"].len(), 1);
        assert_eq!(saved.channels["chan"][0]["id"], "a1");
        ModLogStorageService::clear_channel_at(|| Ok(snapshot.clone()), "Chan").unwrap();
        let saved = read_snapshot(&snapshot);
        assert_eq!(saved.journal_seq, 2);
        assert!(!saved.channels.contains_key("chan"));
    }

    #[test]
    fn a_failed_journal_write_folds_once_and_returns_without_an_error() {
        let (_snapshot, journal) = temp_paths("fold-on-failure");
        let lock = Mutex::new(JournalQueue::new());
        let done = Condvar::new();
        let folds = AtomicUsize::new(0);
        let queue = enqueue(&lock, 1);
        wait_or_fold(
            &lock,
            &done,
            &journal,
            queue,
            1,
            |_: &Path, _: &[(u64, String)]| Err("injected write failure".to_string()),
            || {
                folds.fetch_add(1, Ordering::SeqCst);
                // The journal lock is free while the fold runs.
                assert!(lock.try_lock().is_ok());
                Ok(())
            },
        );
        assert_eq!(folds.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(journal.parent().unwrap());
    }

    #[test]
    fn a_failed_fold_after_a_failed_journal_write_still_returns() {
        let (_snapshot, journal) = temp_paths("fold-fails");
        let lock = Mutex::new(JournalQueue::new());
        let done = Condvar::new();
        let folds = AtomicUsize::new(0);
        let queue = enqueue(&lock, 1);
        wait_or_fold(
            &lock,
            &done,
            &journal,
            queue,
            1,
            |_: &Path, _: &[(u64, String)]| Err("injected write failure".to_string()),
            || {
                folds.fetch_add(1, Ordering::SeqCst);
                Err("injected fold failure".to_string())
            },
        );
        assert_eq!(folds.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(journal.parent().unwrap());
    }

    #[test]
    fn a_journal_write_that_lands_never_folds() {
        let (_snapshot, journal) = temp_paths("no-fold-on-success");
        let lock = Mutex::new(JournalQueue::new());
        let done = Condvar::new();
        let folds = AtomicUsize::new(0);
        let queue = enqueue(&lock, 1);
        wait_or_fold(&lock, &done, &journal, queue, 1, commit_lines, || {
            folds.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(folds.load(Ordering::SeqCst), 0);
        assert_eq!(lock.lock().unwrap().durable_seq, 1);
        let _ = fs::remove_dir_all(journal.parent().unwrap());
    }

    #[test]
    fn a_failed_append_never_merges_into_the_next_line() {
        let (snapshot, journal) = temp_paths("failed-append");
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1" }) }).unwrap();
        // A write that died partway leaves a fragment with no newline; the
        // writer's failure path cuts it before anything else appends.
        let mut f = fs::OpenOptions::new().append(true).open(&journal).unwrap();
        std::io::Write::write_all(&mut f, b"{\"seq\":2,\"op\":\"put\",\"chan").unwrap();
        drop(f);
        cut_torn_tail(&journal);
        let retry = journal_line(2, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a2" }) }).unwrap();
        commit_lines(&journal, &[(2, retry)]).unwrap();
        let (restored, replayed) = load_storage(&snapshot, &journal);
        assert_eq!(replayed, 2);
        let ids: Vec<_> = restored.channels["chan"].iter().map(|e| e["id"].clone()).collect();
        assert_eq!(ids, vec![json!("a1"), json!("a2")]);
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn a_replay_after_a_fold_that_never_emptied_the_journal_applies_nothing_twice() {
        let (snapshot, journal) = temp_paths("replay-twice");
        // An entry with no id, an entry with one, a channel put then cleared,
        // and enough puts on one channel that the oldest is trimmed.
        let mut seq = 0;
        let mut journal_op = |op: JournalOp| {
            seq += 1;
            append_journal(&journal, seq, &op).unwrap();
        };
        journal_op(JournalOp::Put { channel: "chan".into(), entry: json!({ "action": "note" }) });
        journal_op(JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1" }) });
        journal_op(JournalOp::Put { channel: "other".into(), entry: json!({ "id": "b1" }) });
        journal_op(JournalOp::Clear { channel: "other".into() });
        for i in 0..=MAX_PER_CHANNEL {
            journal_op(JournalOp::Put { channel: "big".into(), entry: json!({ "id": format!("e{i}") }) });
        }
        let (folded, first) = load_storage(&snapshot, &journal);
        assert_eq!(first, 4 + MAX_PER_CHANNEL + 1);
        assert_eq!(folded.channels["big"].len(), MAX_PER_CHANNEL);
        assert_eq!(folded.channels["big"][0]["id"], "e1");
        // The fold renamed the snapshot into place, then the process died
        // before it emptied the journal.
        write_snapshot(&snapshot, &serde_json::to_string(&folded).unwrap()).unwrap();
        let (restored, again) = load_storage(&snapshot, &journal);
        assert_eq!(again, 0);
        assert_eq!(restored.channels, folded.channels);
        assert_eq!(restored.channels["chan"].len(), 2);
        assert_eq!(restored.channels["big"][0]["id"], "e1");
        assert!(!restored.channels.contains_key("other"));
        // Lines written after that fold still replay.
        append_journal(&journal, restored.journal_seq + 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a2" }) }).unwrap();
        let (latest, last) = load_storage(&snapshot, &journal);
        assert_eq!(last, 1);
        assert_eq!(latest.channels["chan"].len(), 3);
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }
}
