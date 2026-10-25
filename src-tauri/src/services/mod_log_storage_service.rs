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
// task that folds the store into the file every 30 s and cuts the journal
// (flush_now does the same for shutdown paths). A crash loses nothing, and a
// busy moderated chat costs a line per action instead of a rewrite of every
// channel's log. Actions recorded while a write is in flight queue up and go
// to disk together, so a burst costs one sync per batch, not one per action.
//
// A snapshot that cannot be read (locked by another process) is never written
// over. The store then holds a partial view built from the journal alone, and
// every action recorded meanwhile is merged into that view and journaled like
// any other, so the journal is the one total order of actions. Each later call
// (and the fold tick) reads the snapshot again; once it reads, the store is
// rebuilt from it plus every journal line past its journal_seq, in seq order,
// and folds resume. A fold that cuts the journal replaces it with one header
// line holding the last seq the file covers, so the next seq is always known
// from the journal alone.

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
/// Actions recorded since the file's last write, one JSON line each, led by a
/// header line once a fold has cut it.
const MOD_LOGS_JOURNAL: &str = "mod_logs.journal";
/// How many snapshots set aside as unreadable are kept.
const KEEP_UNREADABLE: usize = 3;
/// How often the store is folded into the file (and the journal cut).
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
/// every fold, so a fold never cuts the journal of a line its snapshot
/// does not hold. It also guards the queue of lines waiting for the disk.
static JOURNAL_LOCK: Mutex<JournalQueue> = Mutex::new(JournalQueue::new());
/// Signaled whenever a write ends, so waiters re-check their line.
static JOURNAL_DONE: Condvar = Condvar::new();
/// The sequence number of the last journal line written; the next line takes
/// one more. Only advanced under `JOURNAL_LOCK`.
static LAST_SEQ: AtomicU64 = AtomicU64::new(0);
static FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);
/// Held for the whole of a seed attempt, so one thread reads the snapshot at a
/// time. Taken before `JOURNAL_LOCK` and the store lock, and never while
/// holding either.
static SEED_LOCK: Mutex<()> = Mutex::new(());
/// True until the session's first snapshot read starts. Only that read waits
/// out a snapshot another process holds; later attempts read once and return.
static FIRST_SEED_ATTEMPT: AtomicBool = AtomicBool::new(true);
/// How many times a snapshot read has slept before reading again.
#[cfg(test)]
static SNAPSHOT_RETRY_SLEEPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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
    /// (a crash between the rename and the cut) is never applied twice.
    #[serde(default)]
    pub journal_seq: u64,
    /// True while the snapshot could not be read: `channels` is built from the
    /// journal alone and is never written over the snapshot.
    #[serde(skip)]
    partial: bool,
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
    /// only runs when the store is not complete yet. An error means the
    /// snapshot could not be read, so the store only holds a partial view; the
    /// next call tries again.
    fn with_store_at<R>(
        resolve_path: impl Fn() -> Result<PathBuf, String>,
        f: impl FnOnce(&mut ModLogStorage) -> R,
    ) -> Result<R, String> {
        Self::seed(&resolve_path)?;
        let (result, partial) = Self::with_seeded(|storage| {
            let result = f(&mut *storage);
            (result, storage.partial)
        })?;
        if partial {
            return Err("the mod log snapshot cannot be read yet".to_string());
        }
        Ok(result)
    }

    /// Run `f` on the store, which must already be seeded.
    fn with_seeded<R>(f: impl FnOnce(&mut ModLogStorage) -> R) -> Result<R, String> {
        let mut guard = store().lock().map_err(|e| e.to_string())?;
        let storage = guard.as_mut().ok_or_else(|| "the mod log store is not seeded".to_string())?;
        Ok(f(storage))
    }

    /// Make the store complete when the snapshot can be read, and give it at
    /// least a partial view when it cannot. Call it with neither
    /// `JOURNAL_LOCK` nor the store lock held: the snapshot read may wait out a
    /// lock another process holds.
    ///
    /// A snapshot that cannot be read leaves a partial store (built from the
    /// journal alone, never folded) and returns `Ok`; the next call reads the
    /// snapshot again. An error means there is no snapshot path to work with.
    fn seed(resolve_path: &impl Fn() -> Result<PathBuf, String>) -> Result<(), String> {
        if store_is_complete()? {
            return Ok(());
        }
        let _seeding = SEED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        if store_is_complete()? {
            return Ok(());
        }
        let path = resolve_path()?;
        let journal = path.with_file_name(MOD_LOGS_JOURNAL);
        match load_snapshot(&path) {
            Ok(snapshot) => Self::install(path, journal, Some(snapshot)),
            Err(e) => {
                warn!("[ModLogStorage] snapshot unreadable, working from the journal alone: {}", e);
                if store_is_seeded()? {
                    return Ok(());
                }
                Self::install(path, journal, None)
            }
        }
    }

    /// Put a store in place built from `snapshot` plus the journal, or, with no
    /// snapshot, a partial store built from the journal alone. Called with
    /// `SEED_LOCK` held. A store already in place (partial) is replaced, and
    /// every line it journaled meanwhile is in the rebuilt one.
    fn install(path: PathBuf, journal: PathBuf, snapshot: Option<ModLogStorage>) -> Result<(), String> {
        let seeded = store_is_seeded()?;
        if !seeded {
            cut_torn_tail(&journal);
        }
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A write in flight finishes first, so the file holds every line that
        // has left the queue and the queue holds the rest.
        while queue.writing {
            queue = JOURNAL_DONE.wait(queue).unwrap_or_else(|e| e.into_inner());
        }
        let mut bytes = fs::read(&journal).unwrap_or_default();
        for (_, line) in &queue.pending {
            bytes.push(b'\n');
            bytes.extend_from_slice(line.as_bytes());
        }
        let partial = snapshot.is_none();
        let mut storage = snapshot.unwrap_or_default();
        storage.partial = partial;
        let newest = newest_seq(&bytes).max(storage.journal_seq);
        let replayed = replay_journal(&mut storage, &bytes);
        {
            let mut guard = store().lock().map_err(|e| e.to_string())?;
            let _ = STORE_PATH.set(path);
            let _ = JOURNAL_PATH.set(journal);
            LAST_SEQ.fetch_max(newest, Ordering::SeqCst);
            *guard = Some(storage);
        }
        if replayed > 0 {
            debug!("[ModLogStorage] replayed {} journal lines", replayed);
        }
        // Actions recorded after the last fold (a crash, or a kill before the
        // exit flush), or journaled while the snapshot was locked: the next
        // fold writes them into the file.
        if replayed > 0 && !partial {
            mark_dirty();
        }
        Ok(())
    }

    /// Synchronous write-if-dirty, for shutdown paths.
    pub fn flush_now() -> Result<(), String> {
        Self::flush_with(write_snapshot)
    }

    /// `flush_now` with the snapshot written by `write`.
    fn flush_with(
        write: impl FnOnce(&Path, &str) -> std::io::Result<Durability>,
    ) -> Result<(), String> {
        // A partial store gets another read of its snapshot before anything
        // folds.
        if let Some(path) = STORE_PATH.get() {
            if store_is_partial() {
                Self::seed(&|| Ok::<PathBuf, String>(path.clone()))?;
            }
        }
        // Held to the end: nothing is journaled between the snapshot below and
        // the journal being cut after the write.
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A write already in flight finishes first; no new one can start while
        // this holds the lock.
        while queue.writing {
            queue = JOURNAL_DONE.wait(queue).unwrap_or_else(|e| e.into_inner());
        }
        // A snapshot that could not be read is never written over; DIRTY stays
        // set, so the fold runs once the snapshot reads.
        if store_is_partial() {
            return Ok(());
        }
        if !DIRTY.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        // DIRTY is only ever set after a seed stored the path.
        let Some(path) = STORE_PATH.get() else {
            return Ok(());
        };
        // Every line journaled so far is in this snapshot. The sequence number
        // only advances under the journal lock, which is held to the end.
        let covered = LAST_SEQ.load(Ordering::SeqCst);
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
            storage.journal_seq = covered;
            match serde_json::to_string(&*storage) {
                Ok(j) => j,
                Err(e) => {
                    DIRTY.store(true, Ordering::Release);
                    return Err(format!("Failed to serialize mod logs: {}", e));
                }
            }
        };
        let durability = match write(path, &json) {
            Ok(durability) => durability,
            Err(e) => {
                DIRTY.store(true, Ordering::Release);
                return Err(format!("Failed to write mod logs file: {}", e));
            }
        };
        if durability == Durability::Unproven {
            // The rename may not survive a power loss, so the journal stays as
            // it is and lines still waiting for the disk stay queued for the
            // next journal write. A replay skips every line the file covers,
            // and the next durable fold cuts the journal.
            debug!("[ModLogStorage] snapshot saved but its rename is not proven durable; the journal is kept");
            return Ok(());
        }
        // Everything journaled is in the file now, including lines still
        // waiting for the disk, whose callers are released here.
        queue.pending.clear();
        queue.durable_seq = covered;
        JOURNAL_DONE.notify_all();
        // A cut that fails leaves lines the file's journal_seq already covers,
        // which a replay skips.
        if let Some(journal) = JOURNAL_PATH.get() {
            match cut_journal(journal, covered) {
                // A journal holding one whole line has no torn line to guard
                // against.
                Ok(_) => queue.torn = false,
                Err(e) => warn!("[ModLogStorage] could not cut the journal after a fold: {}", e),
            }
        }
        Ok(())
    }

    /// Load one channel's persisted entries (oldest first); empty if none. An
    /// error means the snapshot could not be read yet (the entries so far come
    /// from the journal alone), so the caller can ask again.
    pub fn load_channel(app_handle: &AppHandle, channel: &str) -> Result<Vec<serde_json::Value>, String> {
        let key = channel.to_lowercase();
        Self::with_store(app_handle, |storage| {
            storage.channels.get(&key).cloned().unwrap_or_default()
        })
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

    /// `record` with the snapshot path supplied by `resolve_path`. While the
    /// snapshot cannot be read the action is merged into the partial view and
    /// journaled like any other.
    fn record_at(
        resolve_path: impl Fn() -> Result<PathBuf, String>,
        channel: &str,
        entry: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let key = channel.to_lowercase();
        if key.is_empty() {
            return Ok(entry);
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        // Seeding can wait on a locked snapshot, so it happens before the
        // journal lock is taken.
        Self::seed(&resolve_path)?;
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let channel_key = key.clone();
        let (shown, changed) = Self::with_seeded(move |storage| {
            merge_into(storage.channels.entry(key).or_default(), entry, now_ms)
        })?;
        if changed {
            let op = JournalOp::Put { channel: channel_key, entry: shown.clone() };
            let seq = journal_append(&mut queue, &op);
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

    /// `clear_channel` with the snapshot path supplied by `resolve_path`. While
    /// the snapshot cannot be read the clear is journaled whether or not the
    /// partial view holds the channel, since the snapshot may.
    fn clear_channel_at(
        resolve_path: impl Fn() -> Result<PathBuf, String>,
        channel: &str,
    ) -> Result<(), String> {
        let key = channel.to_lowercase();
        // Seeding can wait on a locked snapshot, so it happens before the
        // journal lock is taken.
        Self::seed(&resolve_path)?;
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let journaled = Self::with_seeded(|storage| {
            let removed = storage.channels.remove(&key).is_some();
            removed || storage.partial
        })?;
        if journaled {
            debug!("[ModLogStorage] cleared {}", key);
            let op = JournalOp::Clear { channel: key };
            let seq = journal_append(&mut queue, &op);
            mark_dirty();
            if let (Some(seq), Some(path)) = (seq, JOURNAL_PATH.get()) {
                wait_or_fold(&JOURNAL_LOCK, &JOURNAL_DONE, path, queue, seq, commit_lines, Self::flush_now);
            }
        }
        Ok(())
    }
}

/// Whether a saved snapshot's rename is known to be on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Durability {
    Durable,
    /// The file is in place, but the rename could not be flushed (the platform
    /// refused the write-through move, or cannot sync a directory).
    Unproven,
}

fn store_is_seeded() -> Result<bool, String> {
    Ok(store().lock().map_err(|e| e.to_string())?.is_some())
}

/// Whether the store is in place and built from a snapshot that was read.
fn store_is_complete() -> Result<bool, String> {
    Ok(store().lock().map_err(|e| e.to_string())?.as_ref().is_some_and(|storage| !storage.partial))
}

/// Whether the store is in place but built from the journal alone.
fn store_is_partial() -> bool {
    store()
        .lock()
        .map(|guard| guard.as_ref().is_some_and(|storage| storage.partial))
        .unwrap_or(false)
}

/// One journal line.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// The sequence number of any journal line, the header included.
#[derive(Deserialize)]
struct JournalSeq {
    #[serde(default)]
    seq: u64,
}

/// Write `json` to `path` through a sibling that is flushed to disk and then
/// renamed over the file, so a crash or power loss mid-write never leaves a
/// truncated or empty file for the next seed to read.
fn write_snapshot(path: &Path, json: &str) -> std::io::Result<Durability> {
    replace_file(path, &path.with_extension("json.tmp"), json.as_bytes())
}

/// Replace the journal with one holding only a header line, `seq` being the
/// last sequence number the snapshot covers. The next seq is read back from it
/// (or from a later line), so a cut journal still knows where it stands.
fn cut_journal(journal: &Path, seq: u64) -> std::io::Result<Durability> {
    let header = format!("{{\"op\":\"base\",\"seq\":{}}}\n", seq);
    replace_file(journal, &journal.with_extension("journal.tmp"), header.as_bytes())
}

/// Write `bytes` to `tmp`, flush them to disk, and rename `tmp` over `path`.
fn replace_file(path: &Path, tmp: &Path, bytes: &[u8]) -> std::io::Result<Durability> {
    use std::io::Write;
    let mut file = fs::File::create(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    rename_durable(tmp, path)
}

/// Rename `from` over `to`, replacing an existing `to`. `Durable` means the
/// rename itself is on disk, so a power loss afterwards cannot bring back the
/// old file under a journal that was already cut; `Unproven` means the file is
/// in place but the platform would not confirm it.
///
/// A write-through move that is refused for access (a scanner or indexer
/// holding the destination) falls back to a plain rename and a flush of the
/// destination, which is not enough to prove the rename durable.
#[cfg(windows)]
fn rename_durable(from: &Path, to: &Path) -> std::io::Result<Durability> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide = |path: &Path| -> Vec<u16> {
        path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    };
    let (from_w, to_w) = (wide(from), wide(to));
    // SAFETY: both buffers are NUL-terminated and outlive the call.
    let moved = unsafe {
        MoveFileExW(
            PCWSTR(from_w.as_ptr()),
            PCWSTR(to_w.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    // A Win32 failure arrives as an HRESULT whose low 16 bits are the error code.
    let Err(failure) = moved else {
        return Ok(Durability::Durable);
    };
    let error = std::io::Error::from_raw_os_error(failure.code().0 & 0xFFFF);
    if error.raw_os_error() != Some(ERROR_ACCESS_DENIED.0 as i32) {
        return Err(error);
    }
    fs::rename(from, to)?;
    // Best effort: the rename is already done, and a failed flush is the same
    // unproven outcome the caller is told about.
    let _ = fs::OpenOptions::new().write(true).open(to).and_then(|file| file.sync_all());
    Ok(Durability::Unproven)
}

/// Rename `from` over `to`, replacing an existing `to`. `Durable` means the
/// parent directory was synced after the rename, so a power loss afterwards
/// cannot bring back the old file under a journal that was already cut;
/// `Unproven` means the file system cannot sync a directory.
#[cfg(not(windows))]
fn rename_durable(from: &Path, to: &Path) -> std::io::Result<Durability> {
    fs::rename(from, to)?;
    match sync_parent_dir(to) {
        Ok(()) => Ok(Durability::Durable),
        Err(e) if dir_sync_unsupported(&e) => Ok(Durability::Unproven),
        Err(e) => Err(e),
    }
}

/// Flush `path`'s parent directory to disk.
#[cfg(not(windows))]
fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    fs::File::open(dir)?.sync_all()
}

/// A directory sync the file system refuses outright (as opposed to one that
/// failed): EINVAL, ENOTSUP or EOPNOTSUPP, ENOTTY or EBADF.
#[cfg(not(windows))]
fn dir_sync_unsupported(error: &std::io::Error) -> bool {
    const EBADF: i32 = 9;
    const EINVAL: i32 = 22;
    const ENOTTY: i32 = 25;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const NOT_SUPPORTED: [i32; 2] = [45, 102];
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    const NOT_SUPPORTED: [i32; 1] = [95];
    error.kind() == std::io::ErrorKind::Unsupported
        || error
            .raw_os_error()
            .is_some_and(|code| [EBADF, EINVAL, ENOTTY].contains(&code) || NOT_SUPPORTED.contains(&code))
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

/// How many times a snapshot that cannot be read right now (locked by another
/// process, say) is read again before the session's first seed gives up. Every
/// later seed attempt reads once.
const SNAPSHOT_READ_RETRIES: u32 = 3;
const SNAPSHOT_READ_RETRY_AFTER: std::time::Duration = std::time::Duration::from_millis(50);

/// The snapshot's bytes, or `None` if there is no snapshot. Any other read
/// error is returned, after waiting it out if this is the session's first seed
/// attempt.
fn read_snapshot_bytes(snapshot: &Path) -> Result<Option<Vec<u8>>, String> {
    let mut retries_left = if FIRST_SEED_ATTEMPT.swap(false, Ordering::SeqCst) { SNAPSHOT_READ_RETRIES } else { 0 };
    loop {
        match fs::read(snapshot) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) if retries_left == 0 => {
                return Err(format!("Failed to read mod logs file: {}", e));
            }
            Err(_) => {
                retries_left -= 1;
                #[cfg(test)]
                SNAPSHOT_RETRY_SLEEPS.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(SNAPSHOT_READ_RETRY_AFTER);
            }
        }
    }
}

/// Move a snapshot that does not parse next to itself under a name no fold
/// writes to, so its bytes stay recoverable, and delete all but the newest
/// `KEEP_UNREADABLE` of them.
fn set_aside_unreadable(snapshot: &Path) -> Result<(), String> {
    let name = snapshot.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let aside = snapshot.with_file_name(format!("{}.unreadable-{}", name, chrono::Utc::now().timestamp_millis()));
    fs::rename(snapshot, &aside)
        .map_err(|e| format!("Failed to move the unreadable mod logs file aside: {}", e))?;
    warn!("[ModLogStorage] mod logs file did not parse; kept as {}", aside.display());
    prune_set_aside(snapshot, &name);
    Ok(())
}

/// Delete every set-aside copy of the snapshot named `name` beside it except
/// the newest `KEEP_UNREADABLE`, newest by the timestamp in the file name.
fn prune_set_aside(snapshot: &Path, name: &str) {
    let dir = match snapshot.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{}.unreadable-", name);
    let mut aside: Vec<(i64, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let stamp = file.strip_prefix(&prefix)?.parse::<i64>().ok()?;
            Some((stamp, entry.path()))
        })
        .collect();
    aside.sort_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
    for (_, path) in aside.into_iter().skip(KEEP_UNREADABLE) {
        if let Err(e) = fs::remove_file(&path) {
            warn!("[ModLogStorage] could not delete an old unreadable mod logs file {}: {}", path.display(), e);
        }
    }
}

/// The readable lines of a journal. Read as bytes and split on newlines: a
/// fragment that ends inside a multi-byte character is not valid UTF-8 and
/// fails only its own line. The header line is not an operation, so it is
/// skipped here like any other line that is not one.
fn journal_lines(bytes: &[u8]) -> impl Iterator<Item = JournalLine> + '_ {
    bytes
        .split(|b| *b == b'\n')
        .filter(|l| !l.iter().all(|b| b.is_ascii_whitespace()))
        .filter_map(|l| serde_json::from_slice::<JournalLine>(l).ok())
}

/// The highest sequence number on any readable line of a journal, header
/// included; 0 for a journal with none. The next line takes one more.
fn newest_seq(bytes: &[u8]) -> u64 {
    bytes
        .split(|b| *b == b'\n')
        .filter_map(|l| serde_json::from_slice::<JournalSeq>(l).ok())
        .map(|line| line.seq)
        .max()
        .unwrap_or(0)
}

/// Apply every readable journal line past the base's `journal_seq` to
/// `storage`, once each and in sequence order whatever order the lines sit in,
/// and return how many were applied.
fn replay_journal(storage: &mut ModLogStorage, bytes: &[u8]) -> usize {
    let mut lines: Vec<JournalLine> = journal_lines(bytes).filter(|line| line.seq > storage.journal_seq).collect();
    lines.sort_by_key(|line| line.seq);
    lines.dedup_by_key(|line| line.seq);
    let applied = lines.len();
    for JournalLine { seq, op } in lines {
        apply_journal_op(storage, op);
        storage.journal_seq = seq;
    }
    applied
}

/// The store `snapshot` holds. A missing snapshot is an empty store; one that
/// does not parse is moved aside first and also gives an empty store, so the
/// whole journal replays over it; one that cannot be read, or moved aside, is
/// an error, so no fold can overwrite it.
fn load_snapshot(snapshot: &Path) -> Result<ModLogStorage, String> {
    match read_snapshot_bytes(snapshot)? {
        None => Ok(ModLogStorage::default()),
        Some(bytes) => match serde_json::from_slice(&bytes) {
            Ok(parsed) => Ok(parsed),
            Err(_) => {
                set_aside_unreadable(snapshot)?;
                Ok(ModLogStorage::default())
            }
        },
    }
}

/// The store from `snapshot` plus every readable journal line past the
/// snapshot's `journal_seq`, and how many lines it applied. A torn last line
/// from a crash is cut off the journal first.
#[cfg(test)]
fn load_storage(snapshot: &Path, journal: &Path) -> Result<(ModLogStorage, usize), String> {
    let mut storage = load_snapshot(snapshot)?;
    cut_torn_tail(journal);
    let applied = replay_journal(&mut storage, &fs::read(journal).unwrap_or_default());
    Ok((storage, applied))
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

/// The raw payload is never rendered; keep the stored and shown copy lean.
fn strip_details(entry: &mut serde_json::Value) {
    if let Some(fields) = entry.as_object_mut() {
        fields.remove("details");
    }
}

/// Apply the merge rule to one channel's list (oldest first). Returns the entry
/// to show and whether the list changed.
fn merge_into(
    list: &mut Vec<serde_json::Value>,
    mut entry: serde_json::Value,
    now_ms: i64,
) -> (serde_json::Value, bool) {
    strip_details(&mut entry);
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
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(restored.channels["chan"][0]["id"], "a1");
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    #[test]
    fn an_upgrade_replaces_by_id_and_a_clear_removes_the_channel() {
        let (snapshot, journal) = temp_paths("ops");
        let saved = ModLogStorage {
            channels: [("chan".to_string(), vec![json!({ "id": "a1", "source": "irc" })])].into_iter().collect(),
            ..Default::default()
        };
        fs::write(&snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1", "source": "eventsub" }) }).unwrap();
        append_journal(&journal, 2, &JournalOp::Put { channel: "other".into(), entry: json!({ "id": "b1" }) }).unwrap();
        append_journal(&journal, 3, &JournalOp::Clear { channel: "other".into() }).unwrap();
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
        let (_, replayed) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(replayed, 1);
        append_journal(&journal, 2, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a2" }) }).unwrap();
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
            let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
            assert_eq!(replayed, 2, "{name}");
            let ids: Vec<_> = restored.channels["chan"].iter().map(|e| e["id"].clone()).collect();
            assert_eq!(ids, vec![json!("a1"), json!("a2")], "{name}");
        }
    }

    /// The folder every test that goes through the process-wide store uses: the
    /// snapshot and journal paths are set once per process, so they must be the
    /// same ones each time.
    const GLOBALS_FOLDER: &str = "globals";

    /// Puts the process-wide queue and counters back to a fresh session's. A
    /// test that wants the session's first snapshot read to wait out a lock
    /// sets `FIRST_SEED_ATTEMPT` itself.
    fn reset_globals() {
        *JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner()) = JournalQueue::new();
        LAST_SEQ.store(0, Ordering::SeqCst);
        DIRTY.store(false, Ordering::Release);
        FIRST_SEED_ATTEMPT.store(false, Ordering::SeqCst);
    }

    // The store, the journal path and the queue are process-wide and set once.
    // Any test that touches them takes GLOBALS_TEST_LOCK first, starts from
    // reset_globals and uses GLOBALS_FOLDER.
    #[test]
    fn record_and_clear_return_ok_when_every_journal_commit_fails() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
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
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
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
        let (folded, first) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(first, 4 + MAX_PER_CHANNEL + 1);
        assert_eq!(folded.channels["big"].len(), MAX_PER_CHANNEL);
        assert_eq!(folded.channels["big"][0]["id"], "e1");
        // The fold renamed the snapshot into place, then the process died
        // before it emptied the journal.
        write_snapshot(&snapshot, &serde_json::to_string(&folded).unwrap()).unwrap();
        let (restored, again) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(again, 0);
        assert_eq!(restored.channels, folded.channels);
        assert_eq!(restored.channels["chan"].len(), 2);
        assert_eq!(restored.channels["big"][0]["id"], "e1");
        assert!(!restored.channels.contains_key("other"));
        // Lines written after that fold still replay.
        append_journal(&journal, restored.journal_seq + 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a2" }) }).unwrap();
        let (latest, last) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(last, 1);
        assert_eq!(latest.channels["chan"].len(), 3);
        let _ = fs::remove_dir_all(snapshot.parent().unwrap());
    }

    /// Puts the process-wide store back as it was when dropped.
    struct StoreRestore(Option<ModLogStorage>);

    impl Drop for StoreRestore {
        fn drop(&mut self) {
            *store().lock().unwrap_or_else(|e| e.into_inner()) = self.0.take();
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_locked_snapshot_leaves_a_partial_store_and_is_never_overwritten() {
        use std::os::windows::fs::OpenOptionsExt;
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let saved = ModLogStorage {
            channels: [("chan".to_string(), vec![json!({ "id": "a1" })])].into_iter().collect(),
            ..Default::default()
        };
        fs::write(&snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
        let before = fs::read(&snapshot).unwrap();
        // The store is empty for the length of this test.
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        // Held the way another process would hold it: open, sharing nothing.
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(&snapshot).unwrap();
        assert!(load_storage(&snapshot, &journal).is_err());
        assert!(ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).is_err());
        assert!(store_is_partial());
        assert!(!DIRTY.load(Ordering::Acquire));
        // A fold has nothing to write, so the file is untouched.
        ModLogStorageService::flush_now().unwrap();
        drop(held);
        assert_eq!(fs::read(&snapshot).unwrap(), before);
        // Once the lock is gone the same call reads the file.
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(channels["chan"][0]["id"], "a1");
        assert!(!store_is_partial());
    }

    #[test]
    fn an_unparseable_snapshot_is_kept_aside_and_the_journal_still_replays() {
        let (snapshot, journal) = temp_paths("unparseable-snapshot");
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let bytes: &[u8] = b"{\"channels\":";
        fs::write(&snapshot, bytes).unwrap();
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1" }) }).unwrap();
        append_journal(&journal, 2, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a2" }) }).unwrap();
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(replayed, 2);
        let ids: Vec<_> = restored.channels["chan"].iter().map(|e| e["id"].clone()).collect();
        assert_eq!(ids, vec![json!("a1"), json!("a2")]);
        assert!(!snapshot.exists());
        let aside: Vec<PathBuf> = fs::read_dir(snapshot.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("mod_logs.json.unreadable-"))
            })
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(fs::read(&aside[0]).unwrap(), bytes);
    }

    #[test]
    fn rename_durable_replaces_an_existing_destination() {
        let (snapshot, _journal) = temp_paths("rename-durable");
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let tmp = snapshot.with_extension("json.tmp");
        fs::write(&snapshot, b"old").unwrap();
        fs::write(&tmp, b"new").unwrap();
        rename_durable(&tmp, &snapshot).unwrap();
        assert_eq!(fs::read(&snapshot).unwrap(), b"new");
        assert!(!tmp.exists());
    }

    #[cfg(windows)]
    #[test]
    fn actions_recorded_while_the_snapshot_is_held_collapse_and_apply_in_order_once_it_is_free() {
        use std::os::windows::fs::OpenOptionsExt;
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        let saved = ModLogStorage {
            channels: [
                ("keep".to_string(), vec![json!({ "id": "k1" })]),
                ("old".to_string(), vec![json!({ "id": "o1" })]),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        fs::write(&snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
        let before = fs::read(&snapshot).unwrap();
        let now = chrono::Utc::now().to_rfc3339();
        let report = |id: &str, source: &str| {
            json!({ "id": id, "action": "ban", "channel": "chan", "target_user_name": "bob",
                "timestamp": now, "source": source, "details": { "raw": true } })
        };
        // Held the way another process would hold it: open, sharing nothing.
        FIRST_SEED_ATTEMPT.store(true, Ordering::SeqCst);
        let held = fs::OpenOptions::new().read(true).share_mode(0).open(&snapshot).unwrap();
        let first = ModLogStorageService::record_at(|| Ok(snapshot.clone()), "Chan", report("irc-1", "irc")).unwrap();
        assert_eq!(first["id"], "irc-1");
        assert!(first.get("details").is_none());
        // Only the session's first seed attempt waits on the lock.
        let sleeps_before = SNAPSHOT_RETRY_SLEEPS.load(Ordering::SeqCst);
        let second = ModLogStorageService::record_at(|| Ok(snapshot.clone()), "Chan", report("irc-2", "irc")).unwrap();
        let third = ModLogStorageService::record_at(|| Ok(snapshot.clone()), "Chan", report("es-1", "eventsub")).unwrap();
        assert_eq!(SNAPSHOT_RETRY_SLEEPS.load(Ordering::SeqCst), sleeps_before);
        assert_eq!(second["id"], "irc-1");
        assert_eq!(third["id"], "irc-1");
        // The same action reported three times is one entry, upgraded in place.
        let view = ModLogStorageService::with_seeded(|s| s.channels["chan"].clone()).unwrap();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0]["source"], "eventsub");
        ModLogStorageService::clear_channel_at(|| Ok(snapshot.clone()), "Chan").unwrap();
        // The snapshot may hold the channel even though this view does not.
        ModLogStorageService::clear_channel_at(|| Ok(snapshot.clone()), "Old").unwrap();
        let other = json!({ "id": "y1", "action": "ban", "channel": "other", "target_user_name": "amy",
            "timestamp": now, "source": "irc" });
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "Other", other).unwrap();
        assert!(store_is_partial());
        assert!(DIRTY.load(Ordering::Acquire));
        // A fold has nothing to write while the snapshot is unreadable.
        ModLogStorageService::flush_now().unwrap();
        drop(held);
        assert_eq!(fs::read(&snapshot).unwrap(), before);
        // The next call reads the snapshot: its old entries and the journaled
        // actions apply in order, so both cleared channels stay cleared.
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert!(!channels.contains_key("chan"));
        assert!(!channels.contains_key("old"));
        assert_eq!(channels["other"][0]["id"], "y1");
        assert_eq!(channels["keep"][0]["id"], "k1");
        assert!(!store_is_partial());
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(replayed, 5);
        assert!(!restored.channels.contains_key("chan"));
        assert!(!restored.channels.contains_key("old"));
        assert_eq!(restored.channels["other"][0]["id"], "y1");
        assert_eq!(restored.journal_seq, 5);
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn a_journal_cut_at_a_seq_numbers_the_next_action_after_it_while_the_snapshot_is_unreadable() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        fs::write(&journal, b"{\"op\":\"base\",\"seq\":41}\n").unwrap();
        // A folder where the snapshot belongs cannot be read.
        fs::create_dir(&snapshot).unwrap();
        let action = json!({ "id": "x1", "action": "ban", "channel": "chan", "target_user_name": "bob",
            "timestamp": "2026-10-24T00:00:00Z", "source": "irc" });
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", action).unwrap();
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 42);
        assert_eq!(newest_seq(&fs::read(&journal).unwrap()), 42);
        // The snapshot the cut journal followed reads now: x1 replays on top.
        fs::remove_dir(&snapshot).unwrap();
        let saved = ModLogStorage {
            channels: [("keep".to_string(), vec![json!({ "id": "k1" })])].into_iter().collect(),
            journal_seq: 41,
            ..Default::default()
        };
        fs::write(&snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(channels["chan"][0]["id"], "x1");
        assert_eq!(channels["keep"][0]["id"], "k1");
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 42);
    }

    #[test]
    fn a_fold_is_never_written_while_the_view_is_partial() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        fs::create_dir(&snapshot).unwrap();
        let action = json!({ "id": "x1", "action": "ban", "channel": "chan", "target_user_name": "bob",
            "timestamp": "2026-10-24T00:00:00Z", "source": "irc" });
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", action).unwrap();
        assert!(DIRTY.load(Ordering::Acquire));
        let writes = AtomicUsize::new(0);
        ModLogStorageService::flush_with(|path: &Path, json: &str| -> std::io::Result<Durability> {
            writes.fetch_add(1, Ordering::SeqCst);
            fs::write(path, json)?;
            Ok(Durability::Durable)
        })
        .unwrap();
        ModLogStorageService::flush_now().unwrap();
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        assert!(snapshot.is_dir());
        assert!(DIRTY.load(Ordering::Acquire));
        // The snapshot reads at the next fold, which then writes it with x1.
        fs::remove_dir(&snapshot).unwrap();
        let saved = ModLogStorage {
            channels: [("keep".to_string(), vec![json!({ "id": "k1" })])].into_iter().collect(),
            ..Default::default()
        };
        fs::write(&snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
        ModLogStorageService::flush_with(|path: &Path, json: &str| -> std::io::Result<Durability> {
            writes.fetch_add(1, Ordering::SeqCst);
            fs::write(path, json)?;
            Ok(Durability::Durable)
        })
        .unwrap();
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        let written = read_snapshot(&snapshot);
        assert_eq!(written.journal_seq, 1);
        assert_eq!(written.channels["chan"][0]["id"], "x1");
        assert_eq!(written.channels["keep"][0]["id"], "k1");
        assert_eq!(fs::read(&journal).unwrap(), b"{\"op\":\"base\",\"seq\":1}\n");
    }

    #[test]
    fn a_snapshot_that_outlived_its_journal_cut_skips_the_lines_it_covers() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        // The fold saved everything up to line 3, then power failed before the
        // journal was replaced: lines 1 to 3 are still in it.
        let saved = ModLogStorage {
            channels: [("chan".to_string(), vec![json!({ "action": "note" })])].into_iter().collect(),
            journal_seq: 3,
            ..Default::default()
        };
        fs::write(&snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "action": "note" }) }).unwrap();
        append_journal(&journal, 2, &JournalOp::Put { channel: "gone".into(), entry: json!({ "id": "b1" }) }).unwrap();
        append_journal(&journal, 3, &JournalOp::Clear { channel: "gone".into() }).unwrap();
        append_journal(&journal, 4, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a4" }) }).unwrap();
        append_journal(&journal, 5, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a5" }) }).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        // The note is not applied a second time and the newer lines are.
        assert_eq!(channels["chan"].len(), 3);
        assert_eq!(channels["chan"][1]["id"], "a4");
        assert_eq!(channels["chan"][2]["id"], "a5");
        assert!(!channels.contains_key("gone"));
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 5);
    }

    #[test]
    fn the_page_is_told_to_ask_again_while_the_view_is_partial() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, _journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        fs::create_dir(&snapshot).unwrap();
        assert!(ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).is_err());
        assert!(store_is_partial());
        fs::remove_dir(&snapshot).unwrap();
        let saved = ModLogStorage {
            channels: [("chan".to_string(), vec![json!({ "id": "a1" })])].into_iter().collect(),
            ..Default::default()
        };
        fs::write(&snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(channels["chan"][0]["id"], "a1");
        assert!(!store_is_partial());
    }

    #[test]
    fn replay_follows_seq_order_and_skips_the_header() {
        let op = |source: &str| JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a1", "source": source }) };
        let mut bytes = b"{\"op\":\"base\",\"seq\":7}\n".to_vec();
        // Written out of order, and one line twice.
        for (seq, source) in [(9, "eventsub"), (8, "irc"), (9, "eventsub")] {
            bytes.extend_from_slice(journal_line(seq, &op(source)).unwrap().as_bytes());
        }
        bytes.extend_from_slice(b"{\"seq\":10,\"op\":\"put\",\"chan");
        assert_eq!(newest_seq(&bytes), 9);
        assert_eq!(newest_seq(b""), 0);
        let mut storage = ModLogStorage { journal_seq: 7, ..Default::default() };
        assert_eq!(replay_journal(&mut storage, &bytes), 2);
        assert_eq!(storage.journal_seq, 9);
        assert_eq!(storage.channels["chan"].len(), 1);
        assert_eq!(storage.channels["chan"][0]["source"], "eventsub");
    }

    #[test]
    fn an_unproven_save_keeps_the_journal_and_a_later_durable_one_cuts_it() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        let action = |id: &str| {
            json!({ "id": id, "action": "ban", "channel": "chan", "target_user_name": id,
                "timestamp": "2026-10-24T00:00:00Z", "source": "irc" })
        };
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", action("a1")).unwrap();
        assert!(!fs::read(&journal).unwrap().is_empty());
        ModLogStorageService::flush_with(|path: &Path, json: &str| -> std::io::Result<Durability> {
            fs::write(path, json)?;
            Ok(Durability::Unproven)
        })
        .unwrap();
        assert!(!DIRTY.load(Ordering::Acquire));
        assert!(!fs::read(&journal).unwrap().is_empty());
        assert_eq!(read_snapshot(&snapshot).journal_seq, 1);
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", action("a2")).unwrap();
        ModLogStorageService::flush_with(|path: &Path, json: &str| -> std::io::Result<Durability> {
            fs::write(path, json)?;
            Ok(Durability::Durable)
        })
        .unwrap();
        // The cut leaves a header holding the last seq the file covers.
        assert_eq!(fs::read(&journal).unwrap(), b"{\"op\":\"base\",\"seq\":2}\n");
        assert_eq!(read_snapshot(&snapshot).journal_seq, 2);
    }

    #[test]
    fn only_the_newest_three_unreadable_snapshots_are_kept() {
        let (snapshot, journal) = temp_paths("prune-aside");
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        for round in 0..5 {
            fs::write(&snapshot, format!("{{\"channels\":{round}")).unwrap();
            load_storage(&snapshot, &journal).unwrap();
            // The set-aside names carry a millisecond stamp.
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let mut kept: Vec<Vec<u8>> = fs::read_dir(snapshot.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("mod_logs.json.unreadable-"))
            })
            .map(|path| fs::read(path).unwrap())
            .collect();
        kept.sort();
        let expected: Vec<Vec<u8>> = (2..5).map(|round| format!("{{\"channels\":{round}").into_bytes()).collect();
        assert_eq!(kept, expected);
    }
}
