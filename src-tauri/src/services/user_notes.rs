//! Private per-user notes (Chatterino "user notes"): moderator memory such as
//! "warned twice for spoilers". Keyed by Twitch user id so a rename keeps the
//! note. Persisted as `user_notes.json` in the app data dir, loaded once,
//! written on every change (notes are rare, a debounce would only add a
//! way to lose one on exit).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct UserNote {
    pub note: String,
    /// Unix ms of the last edit.
    pub updated_ms: i64,
}

static NOTES: OnceLock<Mutex<Option<HashMap<String, UserNote>>>> = OnceLock::new();

fn cell() -> &'static Mutex<Option<HashMap<String, UserNote>>> {
    NOTES.get_or_init(|| Mutex::new(None))
}

fn path() -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir()
        .ok()
        .map(|d| d.join("user_notes.json"))
}

fn load_if_needed(map: &mut Option<HashMap<String, UserNote>>) {
    if map.is_some() {
        return;
    }
    let loaded = path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<HashMap<String, UserNote>>(&s).ok())
        .unwrap_or_default();
    *map = Some(loaded);
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct UserNotes;

impl UserNotes {
    pub fn get(user_id: &str) -> Option<UserNote> {
        let mut guard = cell().lock().ok()?;
        load_if_needed(&mut guard);
        guard.as_ref()?.get(user_id).cloned()
    }

    /// Empty note deletes the entry. Returns the stored note (None when deleted).
    pub fn set(user_id: &str, note: &str) -> Result<Option<UserNote>, String> {
        if user_id.is_empty() {
            return Err("missing user id".into());
        }
        let snapshot;
        let stored;
        {
            let mut guard = cell().lock().map_err(|e| e.to_string())?;
            load_if_needed(&mut guard);
            let map = guard.as_mut().expect("loaded");
            let trimmed = note.trim();
            if trimmed.is_empty() {
                map.remove(user_id);
                stored = None;
            } else {
                let entry = UserNote {
                    note: trimmed.chars().take(4000).collect(),
                    updated_ms: now_ms(),
                };
                map.insert(user_id.to_string(), entry.clone());
                stored = Some(entry);
            }
            snapshot = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
        }
        let Some(p) = path() else {
            return Err("no app data dir".into());
        };
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, snapshot).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &p).map_err(|e| e.to_string())?;
        Ok(stored)
    }

    pub fn count() -> usize {
        cell()
            .lock()
            .ok()
            .and_then(|mut g| {
                load_if_needed(&mut g);
                g.as_ref().map(|m| m.len())
            })
            .unwrap_or(0)
    }
}
