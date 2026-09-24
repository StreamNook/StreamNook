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
// Persistence follows the universal-cache pattern: an in-memory store seeded
// from disk ONCE per session, mutations in memory, and a debounced background
// task that flushes dirty state to disk (plus flush_now for shutdown paths).

use log::debug;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Manager};

const MOD_LOGS_FILE: &str = "mod_logs.json";
// Keep at most this many entries per channel on disk. Mod actions are
// infrequent, so this is plenty of history without unbounded growth.
const MAX_PER_CHANNEL: usize = 500;

// In-memory store, lazily seeded from disk on first touch (the session's one
// full-file read). None until seeded.
static STORE: OnceLock<Mutex<Option<ModLogStorage>>> = OnceLock::new();
// Resolved once at seed time so flush paths never need an AppHandle.
static STORE_PATH: OnceLock<PathBuf> = OnceLock::new();
static DIRTY: AtomicBool = AtomicBool::new(false);
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
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
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
            let storage = if path.exists() {
                match fs::read_to_string(&path) {
                    Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
                    Err(_) => ModLogStorage::default(),
                }
            } else {
                ModLogStorage::default()
            };
            let _ = STORE_PATH.set(path);
            *guard = Some(storage);
        }
        Ok(f(guard.as_mut().expect("seeded above")))
    }

    /// Synchronous write-if-dirty, for shutdown paths.
    pub fn flush_now() -> Result<(), String> {
        if !DIRTY.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        // DIRTY is only ever set after a seed stored the path.
        let Some(path) = STORE_PATH.get() else {
            return Ok(());
        };
        let json = {
            let guard = match store().lock() {
                Ok(g) => g,
                Err(e) => {
                    DIRTY.store(true, Ordering::Release);
                    return Err(e.to_string());
                }
            };
            let Some(storage) = guard.as_ref() else {
                return Ok(());
            };
            match serde_json::to_string(storage) {
                Ok(j) => j,
                Err(e) => {
                    DIRTY.store(true, Ordering::Release);
                    return Err(format!("Failed to serialize mod logs: {}", e));
                }
            }
        };
        if let Err(e) = fs::write(path, json) {
            DIRTY.store(true, Ordering::Release);
            return Err(format!("Failed to write mod logs file: {}", e));
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
        let (shown, changed) = Self::with_store(app_handle, move |storage| {
            merge_into(storage.channels.entry(key).or_default(), entry, now_ms)
        })?;
        if changed {
            mark_dirty();
        }
        Ok(shown)
    }

    /// Clear one channel's persisted entries.
    pub fn clear_channel(app_handle: &AppHandle, channel: &str) -> Result<(), String> {
        let key = channel.to_lowercase();
        let removed =
            Self::with_store(app_handle, |storage| storage.channels.remove(&key).is_some())?;
        if removed {
            debug!("[ModLogStorage] cleared {}", key);
            mark_dirty();
        }
        Ok(())
    }
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
