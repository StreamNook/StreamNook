//! MultiChat windows' saved tabs and layout, kept by the core so they outlive
//! the webview's storage. MultiChat is the desktop's chats-without-a-stream
//! surface, as the Chats tab is on the phones: your chats come back with it.
//!
//! The page still reads its fast copy first (window storage, read before its
//! first frame); this is where that copy is restored from when the storage was
//! cleared, so a profile reset or a cleaned WebView no longer loses the tabs.
//! The state is the page's own shape, stored whole per window id.

use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

const FILE: &str = "multichat_windows.json";

/// Serializes the file's read-modify-write between two windows saving at once.
static LOCK: Mutex<()> = Mutex::new(());

fn path() -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir().ok().map(|d| d.join(FILE))
}

fn read_all() -> HashMap<String, Value> {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// A window id as a key: what the page uses (`main`, a stable id), nothing else.
fn clean_id(id: &str) -> Option<String> {
    let id = id.trim();
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then(|| id.to_string())
}

/// The window's saved state, or nothing when it was never saved.
#[tauri::command]
pub fn get_multichat_window_state(window_id: String) -> Option<Value> {
    let id = clean_id(&window_id)?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_all().remove(&id)
}

/// Saves the window's state (its tabs, layout and panes); `null` forgets it.
#[tauri::command]
pub fn save_multichat_window_state(window_id: String, state: Value) -> Result<(), String> {
    let id = clean_id(&window_id).ok_or("bad window id")?;
    let path = path().ok_or("no app data dir")?;
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = read_all();
    if state.is_null() {
        if all.remove(&id).is_none() {
            return Ok(());
        }
    } else if all.get(&id) == Some(&state) {
        return Ok(());
    } else {
        all.insert(id, state);
    }
    let json = serde_json::to_string(&all).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_ids_stay_plain() {
        assert_eq!(clean_id(" main ").as_deref(), Some("main"));
        assert_eq!(clean_id("a1-b_2").as_deref(), Some("a1-b_2"));
        assert_eq!(clean_id("../x"), None);
        assert_eq!(clean_id(""), None);
    }
}
