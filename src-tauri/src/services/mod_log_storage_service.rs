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
// and folds resume. A fold that cuts the journal appends a header line holding
// the last seq the file covers, so the next seq is always known from the
// journal alone.
//
// A journal that cannot be read hands out no seq at all. The store then holds
// a partial view flagged journal_unread, and every action recorded meanwhile is
// merged into that view, kept in memory without a seq, and appended to the held
// file (mod_logs.held, or the first writable mod_logs.held.<n>), each line
// carrying a unique held id. Once the journal reads, the held actions (from
// memory and from the held files, in the order they were recorded) that no
// journal line carries yet are numbered after everything the journal and the
// snapshot hold and journaled in order, each line carrying its held id. The held
// files are deleted once those lines are durable, so an action held across a
// restart applies exactly once.
//
// Lines journaled while the snapshot could not be read carry "partial":true,
// because their seq may sit at or below the seq the snapshot later turns out to
// cover. A replay applies them after the whole lines, in seq order, and when
// the snapshot reads they are renumbered past the snapshot's journal_seq in a
// block appended to the journal. The store is complete as soon as that block is
// durable, and no fold runs before.
//
// The journal is only ever appended to. A cut or a rewrite appends one block in
// a single write plus a sync: a header {"op":"base","seq":B,"count":n,
// "overflow":K} followed by n renumbered lines flagged "rebased" (n is 0 for a
// cut). A replay starts at the last header whose n lines all follow it intact,
// so a torn block falls back to the previous one, and a rebased line outside
// the intact block is ignored. The older blocks are dropped by replacing the
// journal with its newest block, and only through a replace whose durability is
// proven: a write-through move on Windows (a refusal leaves the old journal), a
// rename followed by a successful directory sync elsewhere (a file system that
// cannot sync a directory is never renamed on).
//
// A journal append that fails is repeated in the first overflow file
// (mod_logs.journal.<n>) that takes it. A replay reads the main journal and
// then each overflow file the newest intact header has not absorbed. A fold or
// rewrite block that is durable records the absorbed files in its header and
// then removes them.

use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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
/// Actions recorded while the journal could not be read, one JSON line each
/// carrying a unique `held` id. A refused append goes to `mod_logs.held.<n>`.
const MOD_LOGS_HELD: &str = "mod_logs.held";
/// How many overflow files are tried, in order, for one refused journal append.
const OVERFLOW_TRIES: u64 = 8;
/// How many held files are tried, in order, for one refused held append.
const HELD_TRIES: u64 = 8;
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
/// Overflow files up to this index hold only lines the main journal's header or
/// the snapshot already covers, so a replay skips them and an append starts
/// after them. Only changed under `JOURNAL_LOCK`.
static OVERFLOW_ABSORBED: AtomicU64 = AtomicU64::new(0);
static FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);
/// Held for the whole of a seed attempt, so one thread reads the snapshot at a
/// time. Taken before `JOURNAL_LOCK` and the store lock, and never while
/// holding either.
static SEED_LOCK: Mutex<()> = Mutex::new(());
/// True until the session's first seed attempt starts. Only that attempt waits
/// out a snapshot or journal another process holds; later attempts read once
/// and return.
static FIRST_SEED_ATTEMPT: AtomicBool = AtomicBool::new(true);
/// Names this session's held ids: the start time and the process id.
static HELD_NONCE: OnceLock<String> = OnceLock::new();
/// The counter half of a held id; it only grows.
static HELD_COUNTER: AtomicU64 = AtomicU64::new(0);
/// The index of the held file this session appends to (0 is `mod_logs.held`).
/// It only moves forward, so held actions land in the files in the order they
/// were recorded, until the held files are deleted.
static HELD_FILE: AtomicU64 = AtomicU64::new(0);
/// Held files whose actions are all journaled but that could not be deleted
/// yet. The fold deletes them again before it cuts the journal.
static HELD_REMOVAL_OWED: AtomicBool = AtomicBool::new(false);
/// A journal replace renamed the file but could not sync its directory, so the
/// next journal write syncs the directory before it counts as durable.
#[cfg(not(windows))]
static RENAME_UNSYNCED: AtomicBool = AtomicBool::new(false);
/// How many times a snapshot read has slept before reading again.
#[cfg(test)]
static SNAPSHOT_RETRY_SLEEPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Makes the replace that drops old journal blocks refuse, as a platform that
/// cannot prove a rename does.
#[cfg(test)]
static SHRINK_REFUSED: AtomicBool = AtomicBool::new(false);
/// Makes the append of a block fail.
#[cfg(test)]
static BLOCK_REFUSED: AtomicBool = AtomicBool::new(false);

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
    /// True while the snapshot could not be read, or the journal rewrite that
    /// renumbers its partial lines is not durable yet: `channels` is never
    /// written over the snapshot.
    #[serde(skip)]
    partial: bool,
    /// True while the journal could not be read: no seq is handed out, and
    /// every action recorded meanwhile waits in `held`.
    #[serde(skip)]
    journal_unread: bool,
    /// The actions recorded while the journal was unread, oldest first, already
    /// merged into `channels`. Each is also appended to the held file.
    #[serde(skip)]
    held: Vec<HeldOp>,
}

/// An action recorded while the journal was unread, with the id its line in the
/// held file carries. Its journal line carries the same id.
#[derive(Debug, Clone)]
struct HeldOp {
    id: String,
    op: JournalOp,
}

/// Where an action already applied to the store goes next.
enum Route {
    /// The journal takes it, flagged when the store is partial.
    Journal(JournalOp, bool),
    /// The journal cannot be read, so no seq can be handed out: the action
    /// waits in the held file.
    Held(HeldOp),
}

impl ModLogStorage {
    /// Where an action already applied to the store goes next. With the journal
    /// unread it waits in `held` (the caller appends it to the held file),
    /// since no seq can be handed out. Otherwise it goes to the journal, with
    /// whether the store is partial, which marks its journal line.
    fn journal_or_hold(&mut self, op: JournalOp) -> Route {
        if self.journal_unread {
            let held = HeldOp { id: next_held_id(), op };
            self.held.push(held.clone());
            return Route::Held(held);
        }
        Route::Journal(op, self.partial)
    }
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

    /// Make the store complete when the snapshot and the journal can be read,
    /// and give it at least a partial view when they cannot. Call it with
    /// neither `JOURNAL_LOCK` nor the store lock held: a read may wait out a
    /// lock another process holds.
    ///
    /// A snapshot that cannot be read leaves a partial store (built from the
    /// journal alone, never folded); a journal that cannot be read leaves a
    /// partial store that hands out no seq. Both return `Ok`, and the next call
    /// reads them again. An error means there is no snapshot path to work with.
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
        let retries = first_attempt_retries();
        match load_snapshot(&path, retries) {
            Ok(snapshot) => Self::install(path, journal, Some(snapshot), retries),
            Err(e) => {
                warn!("[ModLogStorage] snapshot unreadable, working from the journal alone: {}", e);
                Self::install(path, journal, None, retries)
            }
        }
    }

    /// Put a store in place built from `snapshot` plus the journal, or, with no
    /// snapshot, a partial store built from the journal alone, or, with a
    /// journal that cannot be read, a partial store that holds new actions
    /// without a seq. Called with `SEED_LOCK` held. A store already in place
    /// (partial) is replaced, and every line it journaled meanwhile is in the
    /// rebuilt one; the actions it held are numbered after everything the
    /// journal and the snapshot hold and journaled in order. `retries` is how
    /// many times the first read of the journal is repeated.
    fn install(
        path: PathBuf,
        journal: PathBuf,
        snapshot: Option<ModLogStorage>,
        retries: u32,
    ) -> Result<(), String> {
        let seeded = store_is_seeded()?;
        // A partial view of the journal alone has nothing to learn from a
        // snapshot that still cannot be read.
        if seeded && snapshot.is_none() && !store_journal_unread() {
            return Ok(());
        }
        // No journal write can be in flight before the first install, so the
        // journal is read (and waited on, on the session's first attempt) here,
        // outside the locks.
        let mut unread = None;
        if !seeded {
            unread = read_journal_files(&journal, retries)
                .and_then(|_| cut_torn_tail(&journal))
                .err();
        }
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A write in flight finishes first, so the file holds every line that
        // has left the queue and the queue holds the rest.
        while queue.writing {
            queue = JOURNAL_DONE.wait(queue).unwrap_or_else(|e| e.into_inner());
        }
        let read = match unread {
            Some(e) => Err(e),
            None => read_journal_files(&journal, 0),
        };
        let files = match read {
            Ok(files) => files,
            Err(e) => {
                warn!("[ModLogStorage] journal unreadable, holding new actions until it reads: {}", e);
                if seeded {
                    return Ok(());
                }
                let mut storage = snapshot.unwrap_or_default();
                storage.partial = true;
                storage.journal_unread = true;
                let mut guard = store().lock().map_err(|e| e.to_string())?;
                let _ = STORE_PATH.set(path);
                let _ = JOURNAL_PATH.set(journal);
                *guard = Some(storage);
                return Ok(());
            }
        };
        let JournalFiles { mut bytes, absorbed, overflow_newest } = files;
        OVERFLOW_ABSORBED.store(absorbed, Ordering::SeqCst);
        for (_, line) in &queue.pending {
            bytes.push(b'\n');
            bytes.extend_from_slice(line.as_bytes());
        }
        // The held files are read here, outside the store lock. One that cannot
        // be read stays where it is for the next seed.
        let held_files = held_paths(&journal);
        let held_read = read_held_ops(&held_files);
        if let Err(e) = &held_read {
            warn!("[ModLogStorage] held actions unreadable, they stay on disk: {}", e);
        }
        // Every held id a journal line already carries: that action is in the
        // snapshot or replays from the journal.
        let journaled: HashSet<String> = journal_lines(&bytes).filter_map(|line| line.held).collect();
        let have_snapshot = snapshot.is_some();
        let mut storage = snapshot.unwrap_or_default();
        let base = storage.journal_seq;
        let lines = replay_lines(&bytes, base);
        // Lines journaled while the snapshot could not be read are renumbered
        // past the snapshot before the store counts as complete, and so is a
        // journal whose last such rewrite block could not be appended.
        let rewrite = have_snapshot && (queue.rewrite_owed || lines.iter().any(|line| line.partial));
        storage.partial = !have_snapshot || rewrite;
        let mut last = newest_seq(&bytes).max(base);
        let mut renumbered = false;
        if rewrite {
            let through = absorbed.max(overflow_newest);
            let appended = block_bytes(base, through, &lines).and_then(|block| {
                append_block(&journal, &block)?;
                Ok(block)
            });
            match appended {
                Ok(block) => {
                    // The queued lines are in the block, which is on disk.
                    last = base + lines.len() as u64;
                    renumbered = true;
                    queue.pending.clear();
                    queue.durable_seq = last;
                    queue.torn = false;
                    queue.rewrite_owed = false;
                    OVERFLOW_ABSORBED.store(through, Ordering::SeqCst);
                    JOURNAL_DONE.notify_all();
                    remove_overflow_files(&journal, through);
                    storage.partial = false;
                    // Held files still on disk are matched against the journal's
                    // held ids, so the older blocks stay until they are gone.
                    if held_files.is_empty() {
                        drop_old_blocks(&journal, &block);
                    }
                }
                Err(e) => {
                    warn!("[ModLogStorage] could not rewrite the journal, the store stays partial: {}", e);
                    queue.rewrite_owed = true;
                    queue.torn = true;
                }
            }
        }
        let replayed = apply_replay(&mut storage, lines);
        if renumbered {
            storage.journal_seq = last;
        }
        let partial = storage.partial;
        let rewrite_owed = queue.rewrite_owed;
        let had_held;
        let mut held_last = None;
        {
            let mut guard = store().lock().map_err(|e| e.to_string())?;
            // Held in memory and held on disk are one list: each action once,
            // in the order recorded, without those a journal line carries.
            let mut held: Vec<HeldOp> = guard.as_mut().map(|old| std::mem::take(&mut old.held)).unwrap_or_default();
            if let Ok(from_files) = &held_read {
                held.extend(from_files.iter().cloned());
            }
            held.sort_by_key(|item| held_order(&item.id));
            let mut seen = HashSet::new();
            held.retain(|item| !journaled.contains(&item.id) && seen.insert(item.id.clone()));
            if renumbered {
                LAST_SEQ.store(last, Ordering::SeqCst);
            } else {
                LAST_SEQ.fetch_max(last, Ordering::SeqCst);
            }
            had_held = !held.is_empty();
            for item in held {
                apply_journal_op(&mut storage, item.op.clone());
                let seq = LAST_SEQ.load(Ordering::SeqCst) + 1;
                let out = JournalLineOut { seq, partial, held: Some(item.id.as_str()), rebased: false, op: &item.op };
                match serialize_line(&out) {
                    Ok(line) => {
                        LAST_SEQ.store(seq, Ordering::SeqCst);
                        queue.pending.push((seq, line));
                        held_last = Some(seq);
                    }
                    Err(e) => debug!("[ModLogStorage] held journal line not serialized: {}", e),
                }
            }
            let _ = STORE_PATH.set(path);
            let _ = JOURNAL_PATH.set(journal);
            *guard = Some(storage);
        }
        if replayed > 0 {
            debug!("[ModLogStorage] replayed {} journal lines", replayed);
        }
        // Actions recorded after the last fold (a crash, or a kill before the
        // exit flush), journaled while the snapshot was locked, or held while
        // the journal was unread: the next fold writes them into the file. A
        // journal rewrite block that could not be appended is tried again at
        // the next tick.
        if ((replayed > 0 || had_held) && !partial) || rewrite_owed {
            mark_dirty();
        }
        // The held files go only once every action in them is on disk in the
        // journal: the lines just queued, or lines still waiting from before.
        let delete_held = held_read.is_ok() && !held_files.is_empty();
        let wait_for = held_last.or_else(|| {
            if delete_held {
                queue.pending.last().map(|(seq, _)| *seq)
            } else {
                None
            }
        });
        let mut durable = true;
        if let (Some(seq), Some(path)) = (wait_for, JOURNAL_PATH.get()) {
            if let Err(e) = wait_durable(&JOURNAL_LOCK, &JOURNAL_DONE, path, queue, seq, commit_lines) {
                durable = false;
                warn!("[ModLogStorage] journal write failed, the next fold saves the held actions: {}", e);
            }
        }
        if delete_held && durable {
            if remove_held_files(&held_files) {
                HELD_FILE.store(0, Ordering::SeqCst);
            } else {
                HELD_REMOVAL_OWED.store(true, Ordering::SeqCst);
            }
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
        // A partial store gets another read of its snapshot and journal before
        // anything folds.
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
        // A snapshot that could not be read is never written over, and no fold
        // runs while a line flagged partial may still sit in the journal; DIRTY
        // stays set, so the fold runs once the store is complete.
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
            // Every held action is in the file by now. Held files that are
            // still on disk keep the journal's lines that carry their ids, so
            // the cut waits until they are gone.
            if HELD_REMOVAL_OWED.load(Ordering::SeqCst) && remove_held_files(&held_paths(journal)) {
                HELD_REMOVAL_OWED.store(false, Ordering::SeqCst);
            }
            if !held_paths(journal).is_empty() {
                debug!("[ModLogStorage] held files remain; the journal is not cut");
                return Ok(());
            }
            // The file covers every overflow line, so the header absorbs the
            // overflow files and they are removed once it is on disk.
            let absorbed = OVERFLOW_ABSORBED.load(Ordering::SeqCst).max(newest_overflow(journal));
            let block = block_header(covered, 0, absorbed).into_bytes();
            match append_block(journal, &block) {
                // A journal whose last block is whole has no torn line to
                // guard against.
                Ok(()) => {
                    queue.torn = false;
                    OVERFLOW_ABSORBED.store(absorbed, Ordering::SeqCst);
                    remove_overflow_files(journal, absorbed);
                    drop_old_blocks(journal, &block);
                }
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
    /// journaled like any other. While the journal cannot be read it is merged
    /// into the view and held, with no seq, in memory and in the held file
    /// until the journal reads.
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
        let (shown, changed, line) = Self::with_seeded(move |storage| {
            let (shown, changed) = merge_into(storage.channels.entry(key).or_default(), entry, now_ms);
            if !changed {
                return (shown, false, None);
            }
            let op = JournalOp::Put { channel: channel_key, entry: shown.clone() };
            (shown, true, Some(storage.journal_or_hold(op)))
        })?;
        if changed {
            let seq = line.and_then(|route| send_route(&mut queue, route));
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
    /// partial view holds the channel, since the snapshot may. While the
    /// journal cannot be read it is held, with no seq, in memory and in the
    /// held file.
    fn clear_channel_at(
        resolve_path: impl Fn() -> Result<PathBuf, String>,
        channel: &str,
    ) -> Result<(), String> {
        let key = channel.to_lowercase();
        // Seeding can wait on a locked snapshot, so it happens before the
        // journal lock is taken.
        Self::seed(&resolve_path)?;
        let mut queue = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (journaled, line) = Self::with_seeded(|storage| {
            let removed = storage.channels.remove(&key).is_some();
            if !(removed || storage.partial) {
                return (false, None);
            }
            (true, Some(storage.journal_or_hold(JournalOp::Clear { channel: key.clone() })))
        })?;
        if journaled {
            debug!("[ModLogStorage] cleared {}", key);
            let seq = line.and_then(|route| send_route(&mut queue, route));
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

/// Whether the store is in place but its journal could not be read.
fn store_journal_unread() -> bool {
    store()
        .lock()
        .map(|guard| guard.as_ref().is_some_and(|storage| storage.journal_unread))
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

/// A journal line as read: the operation, its sequence number, whether it was
/// written while the store was partial, the held id of the action it records
/// (when that action was held while the journal was unread), and whether it
/// belongs to a block (a renumbered copy of lines the block replaces).
#[derive(Debug, Deserialize)]
struct JournalLine {
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    partial: bool,
    #[serde(default)]
    held: Option<String>,
    #[serde(default)]
    rebased: bool,
    #[serde(flatten)]
    op: JournalOp,
}

/// A journal line as written; reads back as a `JournalLine`. The flags and the
/// held id are only written when set.
#[derive(Serialize)]
struct JournalLineOut<'a> {
    seq: u64,
    #[serde(skip_serializing_if = "is_false")]
    partial: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    held: Option<&'a str>,
    #[serde(skip_serializing_if = "is_false")]
    rebased: bool,
    #[serde(flatten)]
    op: &'a JournalOp,
}

/// A held-file line as written; reads back as a `JournalLine` with its `held`
/// id set.
#[derive(Serialize)]
struct HeldLineOut<'a> {
    held: &'a str,
    #[serde(flatten)]
    op: &'a JournalOp,
}

fn is_false(flag: &bool) -> bool {
    !*flag
}

/// The sequence number of any journal line, the header included.
#[derive(Deserialize)]
struct JournalSeq {
    #[serde(default)]
    seq: u64,
}

/// The header line of a journal block: the last seq its snapshot covers, how
/// many renumbered lines follow it and, when overflow files were absorbed, the
/// index of the newest of them.
#[derive(Deserialize)]
struct JournalBase {
    op: String,
    #[serde(default)]
    seq: u64,
    #[serde(default)]
    count: u64,
    #[serde(default)]
    overflow: u64,
}

/// Write `json` to `path` through a sibling that is flushed to disk and then
/// renamed over the file, so a crash or power loss mid-write never leaves a
/// truncated or empty file for the next seed to read.
fn write_snapshot(path: &Path, json: &str) -> std::io::Result<Durability> {
    replace_file(path, &path.with_extension("json.tmp"), json.as_bytes())
}

/// The header line of a block: `seq` is the last sequence number the snapshot
/// covers, `count` how many renumbered lines follow it (a header alone is a
/// cut), and `absorbed`, when not 0, the index of the newest overflow file
/// whose lines the journal or the snapshot already holds.
fn block_header(seq: u64, count: usize, absorbed: u64) -> String {
    let mut header = format!("{{\"op\":\"base\",\"seq\":{}", seq);
    if count > 0 {
        header.push_str(&format!(",\"count\":{}", count));
    }
    if absorbed > 0 {
        header.push_str(&format!(",\"overflow\":{}", absorbed));
    }
    header.push_str("}\n");
    header
}

/// A cut's header line at `seq`.
#[cfg(test)]
fn base_header(seq: u64, absorbed: u64) -> String {
    block_header(seq, 0, absorbed)
}

/// A block of the journal: a header at `base_seq` followed by `lines`
/// renumbered from `base_seq + 1`, whole and in order, so none of them is
/// flagged partial and all of them sit past the snapshot that covers
/// `base_seq`. The lines keep the held id they carry.
fn block_bytes(base_seq: u64, absorbed: u64, lines: &[JournalLine]) -> std::io::Result<Vec<u8>> {
    let mut text = block_header(base_seq, lines.len(), absorbed);
    for (i, line) in lines.iter().enumerate() {
        let out = JournalLineOut {
            seq: base_seq + 1 + i as u64,
            partial: false,
            held: line.held.as_deref(),
            rebased: true,
            op: &line.op,
        };
        text.push_str(&serialize_line(&out).map_err(std::io::Error::other)?);
    }
    Ok(text.into_bytes())
}

#[cfg(test)]
fn block_refused() -> bool {
    BLOCK_REFUSED.load(Ordering::SeqCst)
}

#[cfg(not(test))]
fn block_refused() -> bool {
    false
}

#[cfg(test)]
fn shrink_refused() -> bool {
    SHRINK_REFUSED.load(Ordering::SeqCst)
}

#[cfg(not(test))]
fn shrink_refused() -> bool {
    false
}

/// Append `block` to the journal with one write and one sync, led by a newline
/// so it never glues onto a torn fragment. The block is the only way the
/// journal is cut or rewritten, and nothing depends on a rename. A failed write
/// cuts whatever torn line it left; a block that is only partly there is not
/// whole, so a replay ignores it.
fn append_block(journal: &Path, block: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if block_refused() {
        return Err(std::io::Error::other("the journal block was refused"));
    }
    let mut bytes = Vec::with_capacity(block.len() + 1);
    bytes.push(b'\n');
    bytes.extend_from_slice(block);
    let written = fs::OpenOptions::new().create(true).append(true).open(journal).and_then(|mut file| {
        file.write_all(&bytes)?;
        file.sync_data()?;
        settle_unsynced_rename(journal)
    });
    if written.is_err() {
        if let Err(cut) = cut_torn_tail(journal) {
            debug!("[ModLogStorage] could not cut the journal's torn line: {}", cut);
        }
    }
    written
}

/// Drop the journal's older blocks by replacing it with `block`, its newest.
/// Only a replace whose durability is proven is made: anything else leaves the
/// journal as it is, and a replay reads it the same way, only longer.
fn drop_old_blocks(journal: &Path, block: &[u8]) {
    match shrink_journal(journal, block) {
        Ok(true) => {}
        Ok(false) => debug!("[ModLogStorage] a rename cannot be proven here; the journal keeps its older blocks"),
        Err(e) => debug!("[ModLogStorage] could not drop the older journal blocks: {}", e),
    }
}

/// Replace the journal with `block` through a sibling that is flushed to disk,
/// and report whether the replace was made. A platform that cannot prove the
/// rename durable gets no rename.
fn shrink_journal(journal: &Path, block: &[u8]) -> std::io::Result<bool> {
    use std::io::Write;
    if shrink_refused() {
        return Ok(false);
    }
    let tmp = journal.with_extension("journal.tmp");
    let written = fs::File::create(&tmp).and_then(|mut file| {
        file.write_all(block)?;
        file.sync_all()
    });
    let moved = written.and_then(|()| rename_proven(&tmp, journal));
    if !matches!(moved, Ok(true)) {
        let _ = fs::remove_file(&tmp);
    }
    moved
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

/// Move `from` over `to`, replacing an existing `to`, and return only once the
/// move is on disk (a write-through move).
#[cfg(windows)]
fn move_write_through(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
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
    moved.map_err(|failure| std::io::Error::from_raw_os_error(failure.code().0 & 0xFFFF))
}

/// Rename `from` over `to`, replacing an existing `to`. `Durable` means the
/// rename itself is on disk, so a power loss afterwards cannot bring back the
/// old file; `Unproven` means the file is in place but the platform would not
/// confirm it.
///
/// A write-through move that is refused for access (a scanner or indexer
/// holding the destination) falls back to a plain rename and a flush of the
/// destination, which is not enough to prove the rename durable.
#[cfg(windows)]
fn rename_durable(from: &Path, to: &Path) -> std::io::Result<Durability> {
    use windows::Win32::Foundation::ERROR_ACCESS_DENIED;
    let Err(error) = move_write_through(from, to) else {
        return Ok(Durability::Durable);
    };
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
/// cannot bring back the old file; `Unproven` means the file system cannot
/// sync a directory.
#[cfg(not(windows))]
fn rename_durable(from: &Path, to: &Path) -> std::io::Result<Durability> {
    fs::rename(from, to)?;
    match sync_parent_dir(to) {
        Ok(()) => Ok(Durability::Durable),
        Err(e) if dir_sync_unsupported(&e) => Ok(Durability::Unproven),
        Err(e) => Err(e),
    }
}

/// Rename `from` over `to` only when the rename can be proven durable, and
/// report whether it was made. On Windows that is the write-through move alone:
/// a refusal leaves `to` as it was. Elsewhere a file system that cannot sync a
/// directory is found out before anything is renamed.
#[cfg(windows)]
fn rename_proven(from: &Path, to: &Path) -> std::io::Result<bool> {
    move_write_through(from, to).map(|()| true)
}

#[cfg(not(windows))]
fn rename_proven(from: &Path, to: &Path) -> std::io::Result<bool> {
    match sync_parent_dir(to) {
        Ok(()) => {}
        Err(e) if dir_sync_unsupported(&e) => return Ok(false),
        Err(e) => return Err(e),
    }
    fs::rename(from, to)?;
    // The rename is done, so a failed sync leaves it unproven: the next journal
    // write syncs the directory first, and does not count as durable without it.
    if let Err(e) = sync_parent_dir(to) {
        RENAME_UNSYNCED.store(true, Ordering::SeqCst);
        return Err(e);
    }
    Ok(true)
}

/// Make the directory sync a journal replace still owes, before a write to the
/// journal counts as durable.
#[cfg(not(windows))]
fn settle_unsynced_rename(journal: &Path) -> std::io::Result<()> {
    if RENAME_UNSYNCED.load(Ordering::SeqCst) {
        sync_parent_dir(journal)?;
        RENAME_UNSYNCED.store(false, Ordering::SeqCst);
    }
    Ok(())
}

#[cfg(windows)]
fn settle_unsynced_rename(_journal: &Path) -> std::io::Result<()> {
    Ok(())
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
    /// The block that renumbers lines written while the store was partial could
    /// not be appended. The store stays partial and the rewrite is repeated.
    rewrite_owed: bool,
}

impl JournalQueue {
    const fn new() -> Self {
        Self { pending: Vec::new(), writing: false, durable_seq: 0, torn: true, rewrite_owed: false }
    }
}

/// One journal line as written: JSON and a closing newline.
#[cfg(test)]
fn journal_line(seq: u64, op: &JournalOp) -> Result<String, String> {
    journal_line_with(seq, op, false)
}

/// `journal_line`, flagged when `partial` is set: the line was written while
/// the snapshot could not be read.
fn journal_line_with(seq: u64, op: &JournalOp, partial: bool) -> Result<String, String> {
    serialize_line(&JournalLineOut { seq, partial, held: None, rebased: false, op })
}

/// `line` as written: JSON and a closing newline.
fn serialize_line(line: &JournalLineOut<'_>) -> Result<String, String> {
    let mut text = serde_json::to_string(line).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(text)
}

/// Append `lines` to the journal with one write and one sync. A failed write
/// cuts whatever torn line it left, so the next append starts a clean line,
/// and repeats the lines in the first overflow file that takes them.
fn commit_lines(path: &Path, lines: &[(u64, String)]) -> Result<(), String> {
    use std::io::Write;
    let bytes: Vec<u8> = lines.iter().flat_map(|(_, line)| line.as_bytes().iter().copied()).collect();
    let written = fs::OpenOptions::new().create(true).append(true).open(path).and_then(|mut file| {
        file.write_all(&bytes)?;
        file.sync_data()?;
        settle_unsynced_rename(path)
    });
    if let Err(e) = written {
        if let Err(cut) = cut_torn_tail(path) {
            debug!("[ModLogStorage] could not cut the journal's torn line: {}", cut);
        }
        append_overflow(path, &bytes);
        return Err(e.to_string());
    }
    Ok(())
}

/// The overflow file for `journal` with index `n`.
fn overflow_path(journal: &Path, n: u64) -> PathBuf {
    let name = journal.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    journal.with_file_name(format!("{}.{}", name, n))
}

/// Repeat `bytes` in the first overflow file that takes them, so lines the main
/// journal refused are still on disk. Each write starts on a fresh line, and a
/// replay reads a line repeated here or in the journal once.
fn append_overflow(journal: &Path, bytes: &[u8]) {
    use std::io::Write;
    let first = OVERFLOW_ABSORBED.load(Ordering::SeqCst) + 1;
    let mut text = Vec::with_capacity(bytes.len() + 1);
    text.push(b'\n');
    text.extend_from_slice(bytes);
    for n in first..first + OVERFLOW_TRIES {
        let overflow = overflow_path(journal, n);
        let written = fs::OpenOptions::new().create(true).append(true).open(&overflow).and_then(|mut file| {
            file.write_all(&text)?;
            file.sync_data()
        });
        match written {
            Ok(()) => return,
            Err(e) => debug!("[ModLogStorage] overflow journal {} refused the lines: {}", n, e),
        }
    }
    warn!("[ModLogStorage] no overflow journal took the lines the journal refused");
}

/// The overflow files beside `journal`, oldest index first.
fn overflow_files(journal: &Path) -> std::io::Result<Vec<(u64, PathBuf)>> {
    let dir = match journal.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let name = journal.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let prefix = format!("{}.", name);
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut files: Vec<(u64, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let n = file.strip_prefix(&prefix)?.parse::<u64>().ok()?;
            Some((n, entry.path()))
        })
        .collect();
    files.sort_by_key(|(n, _)| *n);
    Ok(files)
}

/// The index of the newest overflow file beside `journal`; 0 when there is none.
fn newest_overflow(journal: &Path) -> u64 {
    overflow_files(journal).ok().and_then(|files| files.last().map(|(n, _)| *n)).unwrap_or(0)
}

/// Delete the overflow files beside `journal` with an index up to `through`.
/// One that stays is skipped by every replay, because the journal's header has
/// absorbed it.
fn remove_overflow_files(journal: &Path, through: u64) {
    let Ok(files) = overflow_files(journal) else {
        return;
    };
    for (n, path) in files {
        if n <= through {
            if let Err(e) = fs::remove_file(&path) {
                warn!("[ModLogStorage] could not delete an absorbed overflow journal {}: {}", path.display(), e);
            }
        }
    }
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
/// is dirty, so the next fold writes the action anyway. `partial` flags the
/// line as written while the snapshot could not be read.
fn journal_append(queue: &mut JournalQueue, op: &JournalOp, partial: bool) -> Option<u64> {
    JOURNAL_PATH.get()?;
    let seq = LAST_SEQ.load(Ordering::SeqCst) + 1;
    match journal_line_with(seq, op, partial) {
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

/// Send an action `route` decided for: a journal line is queued (the sequence
/// number returned is the one to wait for), a held action is appended to the
/// held file and has no seq. Called with `JOURNAL_LOCK` held, which orders the
/// held lines like the journal's.
fn send_route(queue: &mut JournalQueue, route: Route) -> Option<u64> {
    match route {
        Route::Journal(op, partial) => journal_append(queue, &op, partial),
        Route::Held(held) => {
            if let Some(journal) = JOURNAL_PATH.get() {
                append_held(journal, &held);
            }
            None
        }
    }
}

/// A new held id: this session's nonce and the next counter value.
fn next_held_id() -> String {
    let nonce = HELD_NONCE.get_or_init(|| {
        let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default().max(0);
        format!("{:x}-{:x}", nanos, std::process::id())
    });
    format!("{}-{}", nonce, HELD_COUNTER.fetch_add(1, Ordering::SeqCst) + 1)
}

/// Where a held id sorts: the start of its session, then its counter. Held
/// actions of an earlier session come before this session's.
fn held_order(id: &str) -> (u64, u64) {
    let mut parts = id.split('-');
    let nanos = parts.next().and_then(|part| u64::from_str_radix(part, 16).ok()).unwrap_or(0);
    let counter = parts.nth(1).and_then(|part| part.parse::<u64>().ok()).unwrap_or(0);
    (nanos, counter)
}

/// The held file with index `n` beside `journal` (0 is the first one).
fn held_path(journal: &Path, n: u64) -> PathBuf {
    let base = journal.with_file_name(MOD_LOGS_HELD);
    if n == 0 {
        base
    } else {
        overflow_path(&base, n)
    }
}

/// The held files that exist beside `journal`, the first one and then the
/// numbered ones in index order.
fn held_paths(journal: &Path) -> Vec<PathBuf> {
    let first = held_path(journal, 0);
    let mut paths = Vec::new();
    if first.exists() {
        paths.push(first.clone());
    }
    if let Ok(files) = overflow_files(&first) {
        paths.extend(files.into_iter().map(|(_, path)| path));
    }
    paths
}

/// Append one held action to the held file, and to the first numbered one that
/// takes it when that refuses. An action no file takes stays in memory, where
/// the next fold saves it. Each write starts on a fresh line, so a fragment a
/// failed write left is never glued onto it.
fn append_held(journal: &Path, held: &HeldOp) {
    use std::io::Write;
    let out = HeldLineOut { held: &held.id, op: &held.op };
    let mut line = match serde_json::to_string(&out) {
        Ok(line) => line,
        Err(e) => {
            debug!("[ModLogStorage] held action not serialized: {}", e);
            return;
        }
    };
    line.insert(0, '\n');
    line.push('\n');
    let first = HELD_FILE.load(Ordering::SeqCst);
    for n in first..first + HELD_TRIES {
        let written = fs::OpenOptions::new().create(true).append(true).open(held_path(journal, n)).and_then(|mut file| {
            file.write_all(line.as_bytes())?;
            file.sync_data()
        });
        match written {
            Ok(()) => {
                HELD_FILE.store(n, Ordering::SeqCst);
                return;
            }
            Err(e) => debug!("[ModLogStorage] held file {} refused the action: {}", n, e),
        }
    }
    warn!("[ModLogStorage] no held file took the action; it stays in memory");
}

/// The held actions the files hold, in the order they were recorded. A file
/// that cannot be read is an error, so none of them is deleted.
fn read_held_ops(paths: &[PathBuf]) -> Result<Vec<HeldOp>, String> {
    let mut held = Vec::new();
    for path in paths {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Failed to read {}: {}", path.display(), e)),
        };
        held.extend(journal_lines(&bytes).filter_map(|line| line.held.map(|id| HeldOp { id, op: line.op })));
    }
    held.sort_by_key(|item| held_order(&item.id));
    Ok(held)
}

/// Delete the held files; false when one stays.
fn remove_held_files(paths: &[PathBuf]) -> bool {
    let mut removed = true;
    for path in paths {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!("[ModLogStorage] could not delete the held file {}: {}", path.display(), e);
                removed = false;
            }
        }
    }
    removed
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
/// A journal that does not exist has none; one that cannot be read is an error.
fn cut_torn_tail(journal: &Path) -> Result<(), String> {
    let bytes = match fs::read(journal) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Failed to read mod logs journal: {}", e)),
    };
    if bytes.is_empty() || bytes.ends_with(b"\n") {
        return Ok(());
    }
    let keep = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let cut = fs::OpenOptions::new().write(true).open(journal).and_then(|file| {
        file.set_len(keep as u64)?;
        file.sync_all()
    });
    if let Err(e) = cut {
        warn!("[ModLogStorage] could not cut a torn journal line: {}", e);
    }
    Ok(())
}

/// How many times a snapshot or journal that cannot be read right now (locked
/// by another process, say) is read again before the session's first seed gives
/// up. Every later seed attempt reads once.
const SNAPSHOT_READ_RETRIES: u32 = 3;
const SNAPSHOT_READ_RETRY_AFTER: std::time::Duration = std::time::Duration::from_millis(50);

/// How many times a read is repeated: the retries if this is the session's
/// first seed attempt, none after it.
fn first_attempt_retries() -> u32 {
    if FIRST_SEED_ATTEMPT.swap(false, Ordering::SeqCst) {
        SNAPSHOT_READ_RETRIES
    } else {
        0
    }
}

/// A file's bytes, or `None` if it does not exist. Any other read error is
/// returned, after `retries` more reads, each after a short sleep.
fn read_file_waiting(path: &Path, mut retries_left: u32, what: &str) -> Result<Option<Vec<u8>>, String> {
    loop {
        match fs::read(path) {
            Ok(bytes) => return Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) if retries_left == 0 => {
                return Err(format!("Failed to read {}: {}", what, e));
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

/// Every byte of the journal on disk, and what its header says about the
/// overflow files.
struct JournalFiles {
    /// The main journal, then each overflow file its header has not absorbed,
    /// in index order, each on lines of its own.
    bytes: Vec<u8>,
    /// The index of the newest overflow file the header has absorbed.
    absorbed: u64,
    /// The index of the newest overflow file on disk.
    overflow_newest: u64,
}

/// Read the journal and its overflow files. A file that does not exist is
/// empty; one that cannot be read is an error, so no seq is handed out from a
/// journal that was not seen whole. `retries` is how often each read repeats.
fn read_journal_files(journal: &Path, retries: u32) -> Result<JournalFiles, String> {
    let mut bytes = read_file_waiting(journal, retries, "mod logs journal")?.unwrap_or_default();
    let absorbed = absorbed_overflow(&bytes);
    let files = overflow_files(journal).map_err(|e| format!("Failed to list the mod logs journal files: {}", e))?;
    let mut overflow_newest = 0;
    for (n, path) in files {
        overflow_newest = overflow_newest.max(n);
        if n <= absorbed {
            continue;
        }
        let more = read_file_waiting(&path, retries, "mod logs journal")?.unwrap_or_default();
        bytes.push(b'\n');
        bytes.extend_from_slice(&more);
    }
    Ok(JournalFiles { bytes, absorbed, overflow_newest })
}

/// The newest overflow index an intact header line of a journal has absorbed; 0
/// if none.
fn absorbed_overflow(bytes: &[u8]) -> u64 {
    intact_blocks(bytes).iter().map(|block| block.overflow).max().unwrap_or(0)
}

/// A block of the journal whose header line and every line it counts are
/// there, whole.
struct JournalBlock {
    /// Where the header line starts.
    start: usize,
    /// How many renumbered lines follow the header.
    count: usize,
    overflow: u64,
}

/// The intact blocks of a journal, oldest first. A block is intact when its
/// header line and the `count` lines after it all end in a newline, and those
/// lines are unflagged renumbered lines from `seq + 1` on. A torn write, or a
/// block that was cut short and followed by other lines, is not one.
fn intact_blocks(bytes: &[u8]) -> Vec<JournalBlock> {
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut from = 0;
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines.push((from, i));
            from = i + 1;
        }
    }
    let mut blocks = Vec::new();
    for (index, &(start, end)) in lines.iter().enumerate() {
        let Ok(header) = serde_json::from_slice::<JournalBase>(&bytes[start..end]) else {
            continue;
        };
        if header.op != "base" {
            continue;
        }
        let whole = (1..=header.count).all(|k| {
            lines.get(index + k as usize).is_some_and(|&(from, to)| {
                serde_json::from_slice::<JournalLine>(&bytes[from..to])
                    .is_ok_and(|line| line.rebased && !line.partial && line.seq == header.seq.saturating_add(k))
            })
        });
        if whole {
            blocks.push(JournalBlock { start, count: header.count as usize, overflow: header.overflow });
        }
    }
    blocks
}

/// The lines a replay reads, in file order: those of the newest intact block
/// and everything after it. A line flagged as part of a block is only read
/// within the block that is intact; one left by a block that was cut short is
/// not. A journal with no intact block is read whole.
fn live_lines(bytes: &[u8]) -> Vec<JournalLine> {
    let blocks = intact_blocks(bytes);
    let (live, count) = match blocks.last() {
        Some(block) => (&bytes[block.start..], block.count),
        None => (bytes, 0),
    };
    journal_lines(live)
        .enumerate()
        .filter(|(i, line)| !line.rebased || *i < count)
        .map(|(_, line)| line)
        .collect()
}

/// The snapshot's bytes, or `None` if there is no snapshot. Any other read
/// error is returned, after `retries` more reads.
fn read_snapshot_bytes(snapshot: &Path, retries: u32) -> Result<Option<Vec<u8>>, String> {
    read_file_waiting(snapshot, retries, "mod logs file")
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

/// The lines a replay applies over a base that covers `base_seq`, in the order
/// it applies them: the whole lines past the base in seq order, then the lines
/// written while the store was partial, which the base never covers whatever
/// their seq, in seq order. Each seq counts once within its group. Only the
/// lines of the newest intact block and those after it are read.
fn replay_lines(bytes: &[u8], base_seq: u64) -> Vec<JournalLine> {
    let (partial, whole): (Vec<JournalLine>, Vec<JournalLine>) =
        live_lines(bytes).into_iter().partition(|line| line.partial);
    let mut lines: Vec<JournalLine> = whole.into_iter().filter(|line| line.seq > base_seq).collect();
    lines.sort_by_key(|line| line.seq);
    lines.dedup_by_key(|line| line.seq);
    let mut partial = partial;
    partial.sort_by_key(|line| line.seq);
    partial.dedup_by_key(|line| line.seq);
    lines.extend(partial);
    lines
}

/// Apply `lines` to `storage` in order and return how many there were.
fn apply_replay(storage: &mut ModLogStorage, lines: Vec<JournalLine>) -> usize {
    let applied = lines.len();
    for JournalLine { seq, op, .. } in lines {
        apply_journal_op(storage, op);
        storage.journal_seq = storage.journal_seq.max(seq);
    }
    applied
}

/// Apply every readable journal line past the base's `journal_seq` to
/// `storage` (the lines written while the store was partial after them), once
/// each and in sequence order whatever order the lines sit in, and return how
/// many were applied.
#[cfg(test)]
fn replay_journal(storage: &mut ModLogStorage, bytes: &[u8]) -> usize {
    let lines = replay_lines(bytes, storage.journal_seq);
    apply_replay(storage, lines)
}

/// The store `snapshot` holds. A missing snapshot is an empty store; one that
/// does not parse is moved aside first and also gives an empty store, so the
/// whole journal replays over it; one that cannot be read, or moved aside, is
/// an error, so no fold can overwrite it. `retries` is how often an unreadable
/// snapshot is read again.
fn load_snapshot(snapshot: &Path, retries: u32) -> Result<ModLogStorage, String> {
    match read_snapshot_bytes(snapshot, retries)? {
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
    let mut storage = load_snapshot(snapshot, first_attempt_retries())?;
    cut_torn_tail(journal)?;
    let files = read_journal_files(journal, 0)?;
    let applied = replay_journal(&mut storage, &files.bytes);
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
        OVERFLOW_ABSORBED.store(0, Ordering::SeqCst);
        DIRTY.store(false, Ordering::Release);
        FIRST_SEED_ATTEMPT.store(false, Ordering::SeqCst);
        HELD_FILE.store(0, Ordering::SeqCst);
        HELD_REMOVAL_OWED.store(false, Ordering::SeqCst);
        SHRINK_REFUSED.store(false, Ordering::SeqCst);
        BLOCK_REFUSED.store(false, Ordering::SeqCst);
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
        // The store is complete before the journal starts failing.
        ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).unwrap();
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
        cut_torn_tail(&journal).unwrap();
        let retry =journal_line(2, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a2" }) }).unwrap();
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

    /// A ban of its own: no two ids merge into one action.
    fn ban(id: &str) -> serde_json::Value {
        json!({ "id": id, "action": "ban", "channel": "chan", "target_user_name": id,
            "timestamp": "2026-10-24T00:00:00Z", "source": "irc" })
    }

    fn ids(channels: &HashMap<String, Vec<serde_json::Value>>, channel: &str) -> Vec<serde_json::Value> {
        channels[channel].iter().map(|e| e["id"].clone()).collect()
    }

    fn write_snapshot_file(snapshot: &Path, channels: Vec<(&str, Vec<serde_json::Value>)>, journal_seq: u64) {
        let saved = ModLogStorage {
            channels: channels.into_iter().map(|(channel, entries)| (channel.to_string(), entries)).collect(),
            journal_seq,
            ..Default::default()
        };
        fs::write(snapshot, serde_json::to_string(&saved).unwrap()).unwrap();
    }

    /// A restart: the process-wide store and counters are a fresh session's.
    fn restart() {
        *store().lock().unwrap_or_else(|e| e.into_inner()) = None;
        reset_globals();
    }

    /// A snapshot write that lands durably.
    fn durable_write(path: &Path, json: &str) -> std::io::Result<Durability> {
        fs::write(path, json)?;
        Ok(Durability::Durable)
    }

    #[test]
    fn actions_recorded_while_the_snapshot_and_journal_are_both_held_are_numbered_after_both() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        // A folder where each file belongs cannot be read.
        fs::create_dir(&snapshot).unwrap();
        fs::create_dir(&journal).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x")).unwrap();
        // No seq is handed out from a journal that was not read, and nothing is journaled.
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 0);
        assert!(store_is_partial());
        assert!(store_journal_unread());
        assert!(DIRTY.load(Ordering::Acquire));
        // Both files read again: the journal is a header at 7 and the snapshot covers 7.
        fs::remove_dir(&snapshot).unwrap();
        fs::remove_dir(&journal).unwrap();
        fs::write(&journal, base_header(7, 0)).unwrap();
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 7);
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), vec![json!("x")]);
        assert_eq!(ids(&channels, "keep"), vec![json!("k1")]);
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 8);
        // The held action is journaled past the snapshot, so a restart keeps it.
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(ids(&restored.channels, "chan"), vec![json!("x")]);
    }

    #[test]
    fn a_journal_held_at_seed_replays_behind_the_snapshot_and_a_later_action_follows_it() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        let (snapshot, _journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        for fold_before_restart in [false, true] {
            restart();
            // A crash left the snapshot at 3 and the journal holding lines 4 and 5.
            let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
            write_snapshot_file(&snapshot, vec![("chan", vec![json!({ "id": "a1" })])], 3);
            let put = |id: &str| JournalOp::Put { channel: "chan".into(), entry: json!({ "id": id }) };
            let lines = format!("{}{}", journal_line(4, &put("a4")).unwrap(), journal_line(5, &put("a5")).unwrap());
            fs::create_dir(&journal).unwrap();
            assert!(ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).is_err());
            assert!(store_journal_unread());
            assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 0);
            fs::remove_dir(&journal).unwrap();
            fs::write(&journal, &lines).unwrap();
            ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x")).unwrap();
            let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
            let expected = vec![json!("a1"), json!("a4"), json!("a5"), json!("x")];
            assert_eq!(ids(&channels, "chan"), expected, "fold {fold_before_restart}");
            assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 6);
            if fold_before_restart {
                ModLogStorageService::flush_with(durable_write).unwrap();
                assert_eq!(fs::read(&journal).unwrap(), base_header(6, 0).as_bytes());
            }
            restart();
            let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
            assert_eq!(ids(&channels, "chan"), expected, "restart after fold {fold_before_restart}");
        }
    }

    #[test]
    fn a_clear_held_while_the_journal_is_unread_applies_after_the_journal_lines() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        write_snapshot_file(&snapshot, vec![("chan", vec![json!({ "id": "c1" })])], 0);
        fs::create_dir(&journal).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("n1")).unwrap();
        ModLogStorageService::clear_channel_at(|| Ok(snapshot.clone()), "chan").unwrap();
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 0);
        fs::remove_dir(&journal).unwrap();
        let held = JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "j1" }) };
        fs::write(&journal, journal_line(1, &held).unwrap()).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert!(!channels.contains_key("chan"));
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 3);
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(replayed, 3);
        assert!(!restored.channels.contains_key("chan"));
    }

    #[test]
    fn only_the_first_seed_attempt_waits_on_an_unreadable_journal() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        fs::create_dir(&journal).unwrap();
        FIRST_SEED_ATTEMPT.store(true, Ordering::SeqCst);
        let before = SNAPSHOT_RETRY_SLEEPS.load(Ordering::SeqCst);
        assert!(ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).is_err());
        let after = SNAPSHOT_RETRY_SLEEPS.load(Ordering::SeqCst);
        assert_eq!(after - before, SNAPSHOT_READ_RETRIES as usize);
        assert!(ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).is_err());
        assert_eq!(SNAPSHOT_RETRY_SLEEPS.load(Ordering::SeqCst), after);
        assert!(store_journal_unread());
    }

    #[test]
    fn a_journal_that_cannot_be_read_is_an_error_and_a_missing_one_is_not() {
        let (snapshot, journal) = temp_paths("journal-unreadable");
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        assert!(cut_torn_tail(&journal).is_ok());
        assert!(read_journal_files(&journal, 0).unwrap().bytes.is_empty());
        fs::create_dir(&journal).unwrap();
        assert!(cut_torn_tail(&journal).is_err());
        assert!(read_journal_files(&journal, 0).is_err());
    }

    #[test]
    fn lines_written_while_partial_replay_after_the_whole_lines_whatever_their_seq() {
        let op = |id: &str| JournalOp::Put { channel: "chan".into(), entry: json!({ "id": id }) };
        let mut bytes = base_header(5, 0).into_bytes();
        // Out of order, one partial line twice, and one whole line the base covers.
        for (seq, id, partial) in [
            (7, "w7", false),
            (6, "w6", false),
            (2, "p2", true),
            (2, "p2", true),
            (3, "p3", true),
            (4, "old", false),
        ] {
            bytes.extend_from_slice(journal_line_with(seq, &op(id), partial).unwrap().as_bytes());
        }
        let order: Vec<(u64, bool)> = replay_lines(&bytes, 5).iter().map(|line| (line.seq, line.partial)).collect();
        assert_eq!(order, vec![(6, false), (7, false), (2, true), (3, true)]);
    }

    #[test]
    fn a_line_numbered_at_or_below_the_snapshot_while_partial_is_kept_when_the_snapshot_reads() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        // A fold saved a and b at seq 2, but the journal write of b failed and
        // the cut failed, so the journal still holds only line 1.
        write_snapshot_file(&snapshot, vec![("chan", vec![json!({ "id": "a" }), json!({ "id": "b" })])], 2);
        append_journal(&journal, 1, &JournalOp::Put { channel: "chan".into(), entry: json!({ "id": "a" }) }).unwrap();
        // The next session cannot read the snapshot and numbers c from the journal alone.
        let saved = fs::read(&snapshot).unwrap();
        fs::remove_file(&snapshot).unwrap();
        fs::create_dir(&snapshot).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("c")).unwrap();
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 2);
        assert!(fs::read_to_string(&journal).unwrap().contains("\"partial\":true"));
        fs::remove_dir(&snapshot).unwrap();
        fs::write(&snapshot, saved).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), vec![json!("a"), json!("b"), json!("c")]);
        // c is renumbered past the snapshot and no line is flagged any more.
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 3);
        assert!(!fs::read_to_string(&journal).unwrap().contains("partial"));
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(ids(&restored.channels, "chan"), vec![json!("a"), json!("b"), json!("c")]);
    }

    #[test]
    fn a_journal_rewrite_that_fails_keeps_the_store_partial_until_it_lands() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        fs::create_dir(&snapshot).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x")).unwrap();
        fs::remove_dir(&snapshot).unwrap();
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 0);
        // The journal refuses the rewrite's block.
        BLOCK_REFUSED.store(true, Ordering::SeqCst);
        assert!(ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).is_err());
        assert!(store_is_partial());
        let writes = AtomicUsize::new(0);
        ModLogStorageService::flush_with(|path: &Path, json: &str| -> std::io::Result<Durability> {
            writes.fetch_add(1, Ordering::SeqCst);
            durable_write(path, json)
        })
        .unwrap();
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        assert!(fs::read_to_string(&journal).unwrap().contains("\"partial\":true"));
        // The next call rewrites the journal and the store is complete.
        BLOCK_REFUSED.store(false, Ordering::SeqCst);
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), vec![json!("x")]);
        assert!(!store_is_partial());
        assert!(!fs::read_to_string(&journal).unwrap().contains("partial"));
    }

    #[test]
    fn a_fold_after_a_journal_rewrite_whose_cut_is_lost_applies_nothing_twice() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        fs::create_dir(&snapshot).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x")).unwrap();
        fs::remove_dir(&snapshot).unwrap();
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 0);
        // The snapshot reads: x is renumbered to line 1 and y becomes line 2.
        ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("y")).unwrap();
        let before_cut = fs::read(&journal).unwrap();
        ModLogStorageService::flush_with(durable_write).unwrap();
        assert_eq!(read_snapshot(&snapshot).journal_seq, 2);
        // Power fails: the snapshot is on disk, the journal replacement is not.
        fs::write(&journal, before_cut).unwrap();
        restart();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), vec![json!("x"), json!("y")]);
        assert_eq!(ids(&channels, "keep"), vec![json!("k1")]);
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn overflow_files_replay_after_the_main_journal_unless_its_header_absorbed_them() {
        let (snapshot, journal) = temp_paths("overflow-read");
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let put = |id: &str| JournalOp::Put { channel: "chan".into(), entry: json!({ "id": id }) };
        append_journal(&journal, 1, &put("a1")).unwrap();
        fs::write(overflow_path(&journal, 1), journal_line(2, &put("a2")).unwrap()).unwrap();
        fs::write(overflow_path(&journal, 2), journal_line(3, &put("a3")).unwrap()).unwrap();
        let files = read_journal_files(&journal, 0).unwrap();
        assert_eq!((files.absorbed, files.overflow_newest), (0, 2));
        let mut storage = ModLogStorage::default();
        assert_eq!(replay_journal(&mut storage, &files.bytes), 3);
        assert_eq!(ids(&storage.channels, "chan"), vec![json!("a1"), json!("a2"), json!("a3")]);
        // A header that absorbed file 1 hides it and keeps file 2.
        fs::write(&journal, base_header(1, 1)).unwrap();
        let files = read_journal_files(&journal, 0).unwrap();
        assert_eq!((files.absorbed, files.overflow_newest), (1, 2));
        let mut storage = ModLogStorage { journal_seq: 1, ..Default::default() };
        assert_eq!(replay_journal(&mut storage, &files.bytes), 1);
        assert_eq!(ids(&storage.channels, "chan"), vec![json!("a3")]);
    }

    #[test]
    fn an_action_the_snapshot_and_journal_both_refused_is_recovered_from_the_overflow_file() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 0);
        ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).unwrap();
        // Folders where the snapshot's temporary file and the journal belong:
        // neither takes a write.
        let tmp = snapshot.with_extension("json.tmp");
        fs::create_dir(&tmp).unwrap();
        fs::create_dir(&journal).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("a1")).unwrap();
        let overflow = overflow_path(&journal, 1);
        assert!(overflow.is_file());
        assert_eq!(read_snapshot(&snapshot).journal_seq, 0);
        // A crash, then both take writes again: the action comes back from the overflow file.
        restart();
        fs::remove_dir(&tmp).unwrap();
        fs::remove_dir(&journal).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), vec![json!("a1")]);
        assert_eq!(ids(&channels, "keep"), vec![json!("k1")]);
        // The fold saves it, the new header absorbs the overflow file, and the file goes.
        ModLogStorageService::flush_with(durable_write).unwrap();
        assert_eq!(read_snapshot(&snapshot).journal_seq, 1);
        assert_eq!(fs::read(&journal).unwrap(), base_header(1, 1).as_bytes());
        assert!(!overflow.exists());
        // A stale copy of that file is skipped, and a refused append moves past it.
        fs::write(&overflow, journal_line_with(1, &JournalOp::Clear { channel: "chan".into() }, true).unwrap()).unwrap();
        restart();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), vec![json!("a1")]);
        assert_eq!(OVERFLOW_ABSORBED.load(Ordering::SeqCst), 1);
        fs::remove_file(&journal).unwrap();
        fs::create_dir(&journal).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("a2")).unwrap();
        assert!(overflow_path(&journal, 2).is_file());
    }

    #[test]
    fn actions_held_all_session_survive_an_exit_flush_and_apply_after_a_restart() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 0);
        // A folder where the journal belongs: it cannot be read all session.
        fs::create_dir(&journal).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x")).unwrap();
        assert!(held_path(&journal, 0).is_file());
        let before = fs::read(&snapshot).unwrap();
        // The fold tick and the exit flush have nothing to write, and the held
        // action stays on disk.
        ModLogStorageService::flush_with(durable_write).unwrap();
        ModLogStorageService::flush_now().unwrap();
        assert_eq!(fs::read(&snapshot).unwrap(), before);
        assert!(held_path(&journal, 0).is_file());
        // The next session finds the journal readable.
        restart();
        fs::remove_dir(&journal).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "keep"), vec![json!("k1")]);
        assert_eq!(ids(&channels, "chan"), vec![json!("x")]);
        assert!(held_paths(&journal).is_empty());
        let (restored, replayed) = load_storage(&snapshot, &journal).unwrap();
        assert_eq!(replayed, 1);
        assert_eq!(ids(&restored.channels, "chan"), vec![json!("x")]);
    }

    #[test]
    fn a_crash_between_journaling_the_held_actions_and_deleting_the_held_file_applies_each_once() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        // A crash left the snapshot at 3 and the journal holding lines 4 and 5.
        write_snapshot_file(&snapshot, vec![("chan", vec![json!({ "id": "a1" })])], 3);
        let put = |id: &str| JournalOp::Put { channel: "chan".into(), entry: json!({ "id": id }) };
        let lines = format!("{}{}", journal_line(4, &put("a4")).unwrap(), journal_line(5, &put("a5")).unwrap());
        fs::create_dir(&journal).unwrap();
        assert!(ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).is_err());
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x")).unwrap();
        let held_file = held_path(&journal, 0);
        let held_bytes = fs::read(&held_file).unwrap();
        fs::remove_dir(&journal).unwrap();
        fs::write(&journal, &lines).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        let expected = vec![json!("a1"), json!("a4"), json!("a5"), json!("x")];
        assert_eq!(ids(&channels, "chan"), expected);
        assert!(!held_file.exists());
        // The process died after journaling the held action and before it
        // deleted the held file.
        fs::write(&held_file, held_bytes).unwrap();
        restart();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), expected);
        assert!(!held_file.exists());
        assert_eq!(fs::read_to_string(&journal).unwrap().matches("\"held\"").count(), 1);
    }

    #[test]
    fn a_held_file_that_refuses_an_action_hands_it_to_the_next_one() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 0);
        fs::create_dir(&journal).unwrap();
        // A folder where the first held file belongs refuses the action.
        fs::create_dir(held_path(&journal, 0)).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x")).unwrap();
        assert!(held_path(&journal, 1).is_file());
        restart();
        fs::remove_dir(&journal).unwrap();
        fs::remove_dir(held_path(&journal, 0)).unwrap();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), vec![json!("x")]);
        assert_eq!(ids(&channels, "keep"), vec![json!("k1")]);
        assert!(held_paths(&journal).is_empty());
    }

    #[test]
    fn a_rewrite_whose_old_blocks_cannot_be_dropped_still_completes_the_store_and_keeps_later_actions() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        fs::create_dir(&snapshot).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("p")).unwrap();
        fs::remove_dir(&snapshot).unwrap();
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 0);
        // No rename can be proven, so the journal keeps its old blocks.
        SHRINK_REFUSED.store(true, Ordering::SeqCst);
        ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |_| ()).unwrap();
        assert!(!store_is_partial());
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("q")).unwrap();
        assert!(fs::read_to_string(&journal).unwrap().contains("\"partial\":true"));
        // A restart reads the newest block and what follows it, never the old
        // partial line.
        restart();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "keep"), vec![json!("k1")]);
        assert_eq!(ids(&channels, "chan"), vec![json!("p"), json!("q")]);
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_cut_whose_old_blocks_cannot_be_dropped_keeps_the_actions_after_it() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 0);
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x1")).unwrap();
        SHRINK_REFUSED.store(true, Ordering::SeqCst);
        ModLogStorageService::flush_with(durable_write).unwrap();
        assert_eq!(read_snapshot(&snapshot).journal_seq, 1);
        // The cut is a header appended after the old line.
        assert!(fs::read_to_string(&journal).unwrap().contains("\"op\":\"base\",\"seq\":1"));
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x2")).unwrap();
        restart();
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "keep"), vec![json!("k1")]);
        assert_eq!(ids(&channels, "chan"), vec![json!("x1"), json!("x2")]);
        assert_eq!(LAST_SEQ.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_platform_that_proves_no_rename_completes_the_store_on_the_first_call_and_folds() {
        let _globals = GLOBALS_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_globals();
        let (snapshot, journal) = temp_paths(GLOBALS_FOLDER);
        let _cleanup = TempDirGuard(snapshot.parent().unwrap().to_path_buf());
        let _restore = StoreRestore(store().lock().unwrap_or_else(|e| e.into_inner()).take());
        fs::create_dir(&snapshot).unwrap();
        ModLogStorageService::record_at(|| Ok(snapshot.clone()), "chan", ban("x")).unwrap();
        fs::remove_dir(&snapshot).unwrap();
        write_snapshot_file(&snapshot, vec![("keep", vec![json!({ "id": "k1" })])], 0);
        // Every rename is unproven.
        SHRINK_REFUSED.store(true, Ordering::SeqCst);
        let channels = ModLogStorageService::with_store_at(|| Ok(snapshot.clone()), |s| s.channels.clone()).unwrap();
        assert_eq!(ids(&channels, "chan"), vec![json!("x")]);
        assert!(!store_is_partial());
        let writes = AtomicUsize::new(0);
        ModLogStorageService::flush_with(|path: &Path, json: &str| -> std::io::Result<Durability> {
            writes.fetch_add(1, Ordering::SeqCst);
            fs::write(path, json)?;
            Ok(Durability::Unproven)
        })
        .unwrap();
        assert_eq!(writes.load(Ordering::SeqCst), 1);
        let saved = read_snapshot(&snapshot);
        assert_eq!(saved.journal_seq, 1);
        assert_eq!(ids(&saved.channels, "chan"), vec![json!("x")]);
        // The unproven snapshot leaves the journal as it is.
        assert!(!fs::read(&journal).unwrap().is_empty());
    }

    #[test]
    fn a_torn_block_falls_back_to_the_previous_header() {
        let put = |id: &str| JournalOp::Put { channel: "chan".into(), entry: json!({ "id": id }) };
        let rebased = |seq: u64, id: &str| {
            let op = put(id);
            serialize_line(&JournalLineOut { seq, partial: false, held: None, rebased: true, op: &op }).unwrap()
        };
        // An intact block of two lines past 2, a whole line, then a block that
        // counts three lines but got one down before other actions followed.
        let mut bytes = block_header(2, 2, 0).into_bytes();
        bytes.extend_from_slice(rebased(3, "r3").as_bytes());
        bytes.extend_from_slice(rebased(4, "r4").as_bytes());
        bytes.extend_from_slice(journal_line(5, &put("a5")).unwrap().as_bytes());
        bytes.extend_from_slice(block_header(5, 3, 9).as_bytes());
        bytes.extend_from_slice(rebased(6, "t6").as_bytes());
        bytes.extend_from_slice(journal_line(7, &put("a7")).unwrap().as_bytes());
        bytes.extend_from_slice(b"{\"seq\":8,\"op\":\"put\",\"chan");
        let seqs: Vec<u64> = replay_lines(&bytes, 2).iter().map(|line| line.seq).collect();
        assert_eq!(seqs, vec![3, 4, 5, 7]);
        // The torn block's claim on overflow files does not count.
        assert_eq!(absorbed_overflow(&bytes), 0);
        // A later block that is whole wins, and what the earlier ones held is
        // behind it.
        let mut later = bytes.clone();
        later.push(b'\n');
        later.extend_from_slice(block_header(7, 0, 4).as_bytes());
        later.extend_from_slice(journal_line(8, &put("a8")).unwrap().as_bytes());
        assert_eq!(absorbed_overflow(&later), 4);
        let seqs: Vec<u64> = replay_lines(&later, 7).iter().map(|line| line.seq).collect();
        assert_eq!(seqs, vec![8]);
        // A journal with no whole block is read whole, and a rebased line left
        // by a block that never finished is not applied.
        let mut only_torn = journal_line(1, &put("a1")).unwrap().into_bytes();
        only_torn.extend_from_slice(block_header(1, 2, 0).as_bytes());
        only_torn.extend_from_slice(rebased(2, "t2").as_bytes());
        let seqs: Vec<u64> = replay_lines(&only_torn, 0).iter().map(|line| line.seq).collect();
        assert_eq!(seqs, vec![1]);
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
