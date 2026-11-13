//! Persisted plugin registry: installed plugins, granted capabilities,
//! credential consent state, sources, and pinned author keys.
//! Stored at <app data>/plugins/registry.json.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::services::cache_service;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    #[serde(default)]
    pub plugins: Vec<InstalledPlugin>,
    #[serde(default)]
    pub sources: Vec<SourceEntry>,
    /// Trust-on-first-use pinned author keys: author handle -> minisign public key.
    #[serde(default)]
    pub author_keys: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: String,
    pub tier: String,
    pub description: String,
    #[serde(default)]
    pub homepage: Option<String>,
    pub enabled: bool,
    /// Source index URL, or "local-dev" for a folder install.
    pub source: String,
    /// Runtime kind: "process" (separate executable) or "ui" (in-app module).
    #[serde(default = "default_kind")]
    pub kind: String,
    /// Absolute directory the plugin runs from (contains plugin.toml and the entry).
    pub dir: String,
    pub entry: String,
    /// Optional in-app UI module for a process plugin (hybrid: sidecar + UI).
    #[serde(default)]
    pub ui_entry: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub granted: GrantedCaps,
    /// Per credential kind: "ask" (default), "always", or "revoked".
    #[serde(default)]
    pub credential_consent: HashMap<String, String>,
}

fn default_kind() -> String {
    "process".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GrantedCaps {
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub host_methods: Vec<String>,
    #[serde(default)]
    pub credentials: Vec<String>,
    #[serde(default)]
    pub network: String,
    #[serde(default)]
    pub ui: Vec<String>,
    /// Named hooks the plugin fills (actions it handles, status slots it
    /// pushes, feature flags it provides). The host routes generically by
    /// these names and never references a specific plugin.
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub status: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceEntry {
    pub url: String,
    pub name: String,
    pub operator: String,
    pub operator_pubkey: String,
    pub official: bool,
}

pub fn plugins_dir() -> Result<PathBuf> {
    let dir = cache_service::get_app_data_dir()
        .map_err(|e| anyhow!("app data dir unavailable: {e}"))?
        .join("plugins");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn plugin_state_dir(plugin_id: &str) -> Result<PathBuf> {
    let dir = plugins_dir()?.join(plugin_id);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Scratch directory a plugin may persist its own state in (handed over in
/// the initialize handshake).
pub fn plugin_data_dir(plugin_id: &str) -> Result<PathBuf> {
    let dir = plugin_state_dir(plugin_id)?.join("data");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn registry_path() -> Result<PathBuf> {
    Ok(plugins_dir()?.join("registry.json"))
}

pub fn load() -> Result<Registry> {
    let path = registry_path()?;
    let registry: Registry = if path.exists() {
        let json = std::fs::read_to_string(&path)?;
        serde_json::from_str(&json)?
    } else {
        Registry::default()
    };
    // Whether a drops-automation provider is installed and allowed decides if
    // the app may keep running with no window.
    #[cfg(desktop)]
    crate::services::background_mode::note_registry(&registry);
    Ok(registry)
}

/// Renames after a Windows sharing violation are retried this many times.
const RENAME_RETRIES: u32 = 5;

/// A rename that failed because something (a scanner, an indexer) briefly
/// holds the target open.
fn is_sharing_error(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied || matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

fn rename_with_retries(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < RENAME_RETRIES && is_sharing_error(&e) => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Every registry writer goes through here. The file is written beside the
/// registry and renamed over it, so a crash or a reader never sees half a
/// file.
pub fn save(registry: &Registry) -> Result<()> {
    let path = registry_path()?;
    let json = serde_json::to_string_pretty(registry)?;
    let mut tmp_name = path.as_os_str().to_owned();
    tmp_name.push(".tmp");
    let tmp = PathBuf::from(tmp_name);
    std::fs::write(&tmp, json)?;
    if let Err(e) = rename_with_retries(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    #[cfg(desktop)]
    crate::services::background_mode::note_registry(registry);
    Ok(())
}

/// Appends a line to the plugin's local audit log (credential handovers).
pub fn audit_append(plugin_id: &str, line: &str) {
    if let Ok(dir) = plugin_state_dir(plugin_id) {
        let path = dir.join("audit.log");
        let stamped = format!("{} {}\n", chrono::Utc::now().to_rfc3339(), line);
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, stamped.as_bytes()));
    }
}

pub fn audit_read(plugin_id: &str) -> Vec<String> {
    plugin_state_dir(plugin_id)
        .ok()
        .map(|dir| dir.join("audit.log"))
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| text.lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

/// Persisted panel state for a plugin (schema registered over RPC plus values).
pub fn panel_schema_path(plugin_id: &str) -> Result<PathBuf> {
    Ok(plugin_state_dir(plugin_id)?.join("panel.json"))
}

pub fn panel_values_path(plugin_id: &str) -> Result<PathBuf> {
    Ok(plugin_state_dir(plugin_id)?.join("panel_values.json"))
}

pub fn read_json_file(path: &PathBuf) -> Option<serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
}

pub fn write_json_file(path: &PathBuf, value: &serde_json::Value) -> Result<()> {
    std::fs::write(path, serde_json::to_string_pretty(value)?)?;
    Ok(())
}
