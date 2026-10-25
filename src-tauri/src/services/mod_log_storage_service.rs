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
// action appended to the journal at once, and a background task that folds the
// store into the file every 30 s and empties the journal (flush_now does the
// same for shutdown paths). A crash loses nothing, and a busy moderated chat
// costs a line per action instead of a rewrite of every channel's log.

use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
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
/// Held by every journal append (with the store change it records) and by
/// every fold, so a fold never empties the journal of a line its snapshot
/// does not hold.
static JOURNAL_LOCK: Mutex<()> = Mutex::new(());
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
        let mut guard = store().lock().map_err(|e| e.to_string())?;
        if guard.is_none() {
            let path = Self::get_storage_path(app_handle)?;
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
        let _journal = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
        // Everything journaled is in the file now. A truncate that fails leaves
        // lines the file's journal_seq already covers, which a replay skips.
        if let Some(journal) = JOURNAL_PATH.get() {
            if let Err(e) = fs::write(journal, b"") {
                warn!("[ModLogStorage] could not empty the journal after a fold: {}", e);
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
        let key = channel.to_lowercase();
        if key.is_empty() {
            return Ok(entry);
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let _journal = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let channel_key = key.clone();
        let (shown, changed) = Self::with_store(app_handle, move |storage| {
            merge_into(storage.channels.entry(key).or_default(), entry, now_ms)
        })?;
        if changed {
            journal_append(&JournalOp::Put { channel: channel_key, entry: shown.clone() });
            mark_dirty();
        }
        Ok(shown)
    }

    /// Clear one channel's persisted entries.
    pub fn clear_channel(app_handle: &AppHandle, channel: &str) -> Result<(), String> {
        let key = channel.to_lowercase();
        let _journal = JOURNAL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let removed =
            Self::with_store(app_handle, |storage| storage.channels.remove(&key).is_some())?;
        if removed {
            debug!("[ModLogStorage] cleared {}", key);
            journal_append(&JournalOp::Clear { channel: key });
            mark_dirty();
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

/// Append one line to the journal and flush it to disk.
fn append_journal(path: &Path, seq: u64, op: &JournalOp) -> Result<(), String> {
    use std::io::Write;
    let mut line = serde_json::to_string(&JournalLineOut { seq, op }).map_err(|e| e.to_string())?;
    line.push('\n');
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    file.sync_data().map_err(|e| e.to_string())
}

/// Append to this session's journal. Called with `JOURNAL_LOCK` held, which is
/// what orders the sequence numbers. A failure is only logged: the store is
/// dirty, so the next fold writes the action anyway.
fn journal_append(op: &JournalOp) {
    let Some(path) = JOURNAL_PATH.get() else {
        return;
    };
    let seq = LAST_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    if let Err(e) = append_journal(path, seq, op) {
        debug!("[ModLogStorage] journal append failed: {}", e);
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
    if let Ok(text) = fs::read_to_string(journal) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            if let Ok(JournalLine { seq, op }) = serde_json::from_str::<JournalLine>(line) {
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

    fn temp_paths(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("sn-modlog-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        (dir.join(MOD_LOGS_FILE), dir.join(MOD_LOGS_JOURNAL))
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
