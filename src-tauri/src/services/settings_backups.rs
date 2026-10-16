//! Daily copies of the portable settings, kept beside settings.json, so a
//! settings file that no longer loads is rebuilt from the newest copy instead
//! of falling back to a fresh install, and so a viewer can step back to an
//! earlier day's settings.

use crate::models::settings::Settings;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// How many daily copies are kept.
pub const KEEP: usize = 7;
const PREFIX: &str = "settings-";

fn dir() -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir().ok().map(|d| d.join("settings-backups"))
}

/// The settings as a backup holds them: every portable key, none of the keys
/// tied to this machine's session (accounts, setup state, last-seen version).
pub fn portable(settings: &Settings) -> Option<Value> {
    let mut value = serde_json::to_value(settings).ok()?;
    if let Some(obj) = value.as_object_mut() {
        for key in crate::commands::settings::NON_PORTABLE_KEYS {
            obj.remove(*key);
        }
    }
    Some(value)
}

fn file_name(day: &str) -> String {
    format!("{PREFIX}{day}.json")
}

/// Writes today's copy if there is none yet, then keeps the newest `KEEP`.
/// Called after every settings write; costs one directory read when today's
/// copy exists.
pub fn snapshot_if_due(settings: &Settings) {
    let Some(dir) = dir() else { return };
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let path = dir.join(file_name(&today));
    if path.exists() {
        return;
    }
    if !settings.setup_complete {
        // A fresh install (or the defaults before a real file loads) is not worth keeping.
        return;
    }
    let Some(value) = portable(settings) else { return };
    let Ok(json) = serde_json::to_string_pretty(&value) else { return };
    if std::fs::create_dir_all(&dir).is_err() || std::fs::write(&path, json).is_err() {
        return;
    }
    prune(&dir);
}

fn prune(dir: &Path) {
    let mut names = list_names(dir);
    while names.len() > KEEP {
        let oldest = names.remove(0);
        let _ = std::fs::remove_file(dir.join(oldest));
    }
}

/// Backup file names, oldest first (the date in the name sorts them).
fn list_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| n.starts_with(PREFIX) && n.ends_with(".json"))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// One kept copy, newest first in `list`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct BackupInfo {
    /// The day it was saved, `YYYY-MM-DD`; also how it is restored.
    pub id: String,
    pub bytes: u64,
}

pub fn list() -> Vec<BackupInfo> {
    let Some(dir) = dir() else { return Vec::new() };
    let mut out: Vec<BackupInfo> = list_names(&dir)
        .into_iter()
        .map(|name| {
            let bytes = std::fs::metadata(dir.join(&name)).map(|m| m.len()).unwrap_or(0);
            let id = name.trim_start_matches(PREFIX).trim_end_matches(".json").to_string();
            BackupInfo { id, bytes }
        })
        .collect();
    out.reverse();
    out
}

/// A kept copy's settings, by day.
pub fn read(id: &str) -> Result<Value, String> {
    if !id.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return Err("That backup does not exist.".into());
    }
    let dir = dir().ok_or("No backups folder.")?;
    let text = std::fs::read_to_string(dir.join(file_name(id))).map_err(|_| "That backup does not exist.".to_string())?;
    serde_json::from_str(&text).map_err(|_| "That backup could not be read.".to_string())
}

/// The settings rebuilt from `backup` over a fresh install: every portable key
/// from the copy, the rest default, and setup marked done (a copy exists only
/// for an install that finished it).
pub fn settings_from(backup: &Value) -> Option<Settings> {
    let mut base = serde_json::to_value(Settings::default()).ok()?;
    let obj = base.as_object_mut()?;
    for (key, value) in backup.as_object()? {
        if !crate::commands::settings::NON_PORTABLE_KEYS.contains(&key.as_str()) {
            obj.insert(key.clone(), value.clone());
        }
    }
    let mut settings: Settings = serde_json::from_value(base).ok()?;
    settings.setup_complete = true;
    Some(settings)
}

/// A settings.json that will not load: keep it aside for a look later, and
/// rebuild the settings from the newest copy that still reads.
pub fn recover(settings_path: &Path, error: &str) -> Option<Settings> {
    let dir = dir()?;
    for name in list_names(&dir).into_iter().rev() {
        let Ok(text) = std::fs::read_to_string(dir.join(&name)) else { continue };
        let Ok(backup) = serde_json::from_str::<Value>(&text) else { continue };
        let Some(settings) = settings_from(&backup) else { continue };
        let aside = settings_path.with_file_name(format!("settings.unreadable-{}.json", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        let _ = std::fs::copy(settings_path, &aside);
        log::warn!("[Settings] settings.json did not load ({error}); restored from {name}, the unreadable file is kept as {}", aside.display());
        return Some(settings);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_backup_restores_over_defaults_and_marks_setup_done() {
        let backup = json!({ "theme": "dracula", "accounts": ["x"], "chat_design": { "font_size": 20, "show_dividers": true,
            "alternating_backgrounds": false, "message_spacing": 8, "font_weight": 400, "mention_color": "#ff4444",
            "reply_color": "#ff6b6b", "mention_animation": true } });
        let settings = settings_from(&backup).expect("rebuilt");
        assert_eq!(settings.theme, "dracula");
        assert_eq!(settings.chat_design.font_size, 20);
        assert!(settings.setup_complete);
        assert!(settings.accounts.is_empty(), "session keys never come from a backup");
    }

    #[test]
    fn backups_hold_no_session_keys() {
        let mut settings = Settings::default();
        settings.accounts = vec!["someone".into()];
        let value = portable(&settings).unwrap();
        assert!(value.get("accounts").is_none());
        assert!(value.get("setup_complete").is_none());
        assert!(value.get("theme").is_some());
    }

    #[test]
    fn ids_are_dates_only() {
        assert!(read("../settings").is_err());
    }
}
