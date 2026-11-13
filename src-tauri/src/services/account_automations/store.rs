//! The automation files next to settings.json, and this device's name.
//!
//! Every write goes to a temporary file first and is renamed over the old
//! one, so a crash never leaves half a file. A file that cannot be read (or
//! that its reader refuses) is renamed to `.bad` and logged, and the engine
//! starts that part empty. The contents are the pure modules' own JSON (`Counts`,
//! `Log`, `Pending`); this file only moves text.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const DEVICE_FILE: &str = "automation_device.json";
/// The longest device name.
pub const MAX_DEVICE_NAME: usize = 40;

/// This device: a random id (only for the default name and the claim's
/// device hash) and the name other devices see.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub created_ms: i64,
}

impl Device {
    /// A new device with its default name.
    pub fn fresh() -> Device {
        let id = format!("{:016x}{:08x}", rand::random::<u64>(), rand::random::<u32>());
        let name = default_name(&id);
        Device { id, name, created_ms: super::now_ms() }
    }
}

/// The platform word of a default device name.
pub fn platform_word() -> &'static str {
    if cfg!(target_os = "ios") {
        "iPhone"
    } else if cfg!(target_os = "android") {
        "Android phone"
    } else if cfg!(target_os = "macos") {
        "Mac"
    } else if cfg!(target_os = "linux") {
        "Linux PC"
    } else {
        "Windows PC"
    }
}

/// "Windows PC 3f2a": the platform word and the id's last four characters,
/// never the host name.
pub fn default_name(id: &str) -> String {
    let tail: String = id.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{} {tail}", platform_word())
}

/// A name a user typed: trimmed, 1 to 40 characters, no control characters.
pub fn clean_device_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Give this device a name".into());
    }
    if name.chars().count() > MAX_DEVICE_NAME {
        return Err("Keep the name to 40 characters".into());
    }
    if name.chars().any(char::is_control) {
        return Err("Use letters, numbers and spaces".into());
    }
    Ok(name.to_string())
}

/// The folder the files live in.
pub struct Files {
    dir: PathBuf,
}

impl Files {
    /// The app data folder (settings.json's), when it can be found.
    pub fn open() -> Option<Files> {
        crate::services::cache_service::get_app_data_dir().ok().map(|dir| Files { dir })
    }

    #[cfg(test)]
    pub(crate) fn at(dir: &Path) -> Files {
        Files { dir: dir.to_path_buf() }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// A file's text; None when it is missing. An unreadable one is set aside.
    pub fn read(&self, name: &str) -> Option<String> {
        let path = self.path(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                self.set_aside(name, &e.to_string());
                None
            }
        }
    }

    /// Write through a temporary file and a rename; a failure is logged.
    pub fn write(&self, name: &str, text: &str) {
        if let Err(e) = write_atomic(&self.path(name), text) {
            log::warn!("[Automations] could not write {name}: {e}");
        }
    }

    /// Rename a file its reader refused to `.bad`, so it is kept for a look
    /// and never read again.
    pub fn set_aside(&self, name: &str, why: &str) {
        let path = self.path(name);
        log::warn!("[Automations] {} is unreadable ({why}); set aside as .bad", path.display());
        let bad = path.with_extension("json.bad");
        let _ = std::fs::remove_file(&bad);
        let _ = std::fs::rename(&path, &bad);
    }

    pub fn load_or_create_device(&self) -> Device {
        if let Some(text) = self.read(DEVICE_FILE) {
            match serde_json::from_str::<Device>(&text) {
                Ok(device) if !device.id.is_empty() && clean_device_name(&device.name).is_ok() => return device,
                Ok(_) => self.set_aside(DEVICE_FILE, "an empty id or name"),
                Err(e) => self.set_aside(DEVICE_FILE, &e.to_string()),
            }
        }
        let device = Device::fresh();
        if let Err(e) = self.save_device(&device) {
            log::warn!("[Automations] could not write {DEVICE_FILE}: {e}");
        }
        device
    }

    pub fn save_device(&self, device: &Device) -> Result<(), String> {
        let text = serde_json::to_string(device).map_err(|e| e.to_string())?;
        write_atomic(&self.path(DEVICE_FILE), &text)
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sn-automation-store-{name}-{:08x}", rand::random::<u32>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_device_is_made_once_and_named_by_platform() {
        let dir = scratch("device");
        let files = Files::at(&dir);
        let first = files.load_or_create_device();
        let again = files.load_or_create_device();
        assert_eq!(first, again);
        assert_eq!(first.id.len(), 24);
        assert!(first.name.starts_with(platform_word()));
        assert!(first.name.ends_with(&first.id[20..]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_refused_file_is_set_aside() {
        let dir = scratch("bad");
        std::fs::write(dir.join("automation_counts.json"), "{ not json").unwrap();
        let files = Files::at(&dir);
        assert!(files.read("automation_counts.json").is_some());
        files.set_aside("automation_counts.json", "not json");
        assert!(dir.join("automation_counts.json.bad").exists());
        assert!(files.read("automation_counts.json").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn writes_leave_no_temporary_file() {
        let dir = scratch("write");
        let files = Files::at(&dir);
        files.write("automation_activity.json", "{}");
        assert!(dir.join("automation_activity.json").exists());
        assert!(!dir.join("automation_activity.json.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn device_names_are_checked() {
        assert_eq!(clean_device_name("  Den PC "), Ok("Den PC".into()));
        assert!(clean_device_name("").is_err());
        assert!(clean_device_name(&"x".repeat(41)).is_err());
        assert!(clean_device_name("a\u{7}b").is_err());
        assert_eq!(default_name("0123456789abcdef01233f2a"), format!("{} 3f2a", platform_word()));
    }
}
