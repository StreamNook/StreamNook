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

    /// Append one entry to a channel, de-duped by its `id`, capped to
    /// MAX_PER_CHANNEL. If an entry with the same `id` already exists it is
    /// REPLACED in place (so an EventSub upgrade of an IRC entry persists too).
    pub fn append(
        app_handle: &AppHandle,
        channel: &str,
        entry: serde_json::Value,
    ) -> Result<(), String> {
        let key = channel.to_lowercase();
        if key.is_empty() {
            return Ok(());
        }
        Self::with_store(app_handle, move |storage| {
            let list = storage.channels.entry(key).or_default();

            let new_id = entry.get("id").and_then(|v| v.as_str()).map(String::from);
            if let Some(ref id) = new_id {
                if let Some(existing) = list
                    .iter_mut()
                    .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
                {
                    *existing = entry;
                    return;
                }
            }
            list.push(entry);
            if list.len() > MAX_PER_CHANNEL {
                let overflow = list.len() - MAX_PER_CHANNEL;
                list.drain(0..overflow);
            }
        })?;
        mark_dirty();
        Ok(())
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

/// (channels, entries) held in the in-memory mod-log store; `None` before the
/// first seed or while locked. Diagnostics for the resource line.
pub fn cache_counts() -> Option<(usize, usize)> {
    let store = STORE.get()?.try_lock().ok()?;
    let inner = store.as_ref()?;
    Some((inner.channels.len(), inner.channels.values().map(|v| v.len()).sum()))
}
