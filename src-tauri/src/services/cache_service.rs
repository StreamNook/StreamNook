use anyhow::{Context, Result};
use log::debug;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheMetadata {
    pub timestamp: u64,
    pub expiry_days: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EmoteCache {
    pub metadata: CacheMetadata,
    pub data: String, // JSON string of emote data
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BadgeCache {
    pub metadata: CacheMetadata,
    pub data: String, // JSON string of badge data
}

/// Get the StreamNook main directory (AppData/Local on Windows, the platform
/// data directory elsewhere).
///
/// # Why this is cfg-split
///
/// The Windows branch is deliberately byte-for-byte unchanged. 41 call sites
/// hang off this function (settings, caches, cookie jars, chat logs, drops, the
/// account store, the plugin registry, 7TV cosmetics), so relocating it would
/// strand every existing install's data.
///
/// Off Windows the original was silently catastrophic. `LOCALAPPDATA` and
/// `USERPROFILE` are both unset there, so it fell through to the literal string
/// `C:\Users\Default\AppData\Local`. Backslashes are ordinary filename
/// characters on POSIX, so that is not a path: it is one absurd directory
/// *name*, resolved relative to whatever CWD the process was launched with.
/// Launched from Finder the CWD is `/`, so `create_dir_all` needs root, fails,
/// and every settings read and write fails with it.
///
/// That is the bug where the app reopened into the welcome flow while still
/// signed in: Twitch tokens live under a different (already correct) path, so
/// login survived while every setting reset.
pub fn get_app_data_dir() -> Result<PathBuf> {
    // Use AppData/Local instead of Program Files (no admin rights needed)
    #[cfg(windows)]
    let base: PathBuf = {
        let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
            // Fallback to %USERPROFILE%\AppData\Local
            let user_profile = std::env::var("USERPROFILE")
                .unwrap_or_else(|_| "C:\\Users\\Default".to_string());
            format!("{}\\AppData\\Local", user_profile)
        });
        PathBuf::from(local_app_data)
    };

    // macOS -> ~/Library/Application Support, Linux -> $XDG_DATA_HOME or
    // ~/.local/share. `dirs` is already a dependency of this crate.
    #[cfg(not(windows))]
    let base: PathBuf =
        dirs::data_dir().context("could not resolve the platform data directory")?;

    // Check if we're in development mode by looking for TAURI_ENV or checking debug assertions
    let app_dir = if cfg!(debug_assertions) {
        // debug!("[CacheService] Running in DEBUG mode, using com.streamnook.dev");
        // In development, use com.streamnook.dev
        base.join("com.streamnook.dev")
    } else {
        // debug!("[CacheService] Running in RELEASE mode, using StreamNook");
        // In production, use StreamNook (without space, matching the actual folder name)
        base.join("StreamNook")
    };

    // Create the directory if it doesn't exist
    if !app_dir.exists() {
        fs::create_dir_all(&app_dir).context("Failed to create StreamNook directory")?;
    }

    Ok(app_dir)
}

/// Get the StreamNook cache directory in AppData/Local
pub fn get_cache_dir() -> Result<PathBuf> {
    let cache_dir = get_app_data_dir()?.join("cache");

    // Create the directory if it doesn't exist
    if !cache_dir.exists() {
        fs::create_dir_all(&cache_dir).context("Failed to create cache directory")?;
    }

    Ok(cache_dir)
}

/// Get current timestamp in seconds
fn get_current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Check if cache is expired
fn is_cache_expired(metadata: &CacheMetadata) -> bool {
    let current_time = get_current_timestamp();
    let expiry_seconds = metadata.expiry_days as u64 * 24 * 60 * 60;
    current_time > metadata.timestamp + expiry_seconds
}

/// Save individual emote to cache by emote ID
pub fn save_emote_to_cache(emote_id: &str, data: &str, expiry_days: u32) -> Result<()> {
    let cache_dir = get_cache_dir()?.join("emotes");

    // Create emotes subdirectory if it doesn't exist
    if !cache_dir.exists() {
        fs::create_dir_all(&cache_dir).context("Failed to create emotes cache directory")?;
    }

    let cache_file = cache_dir.join(format!("{}.json", emote_id));

    let cache = EmoteCache {
        metadata: CacheMetadata {
            timestamp: get_current_timestamp(),
            expiry_days,
        },
        data: data.to_string(),
    };

    let json = serde_json::to_string(&cache)?;
    fs::write(&cache_file, json).context("Failed to write emote cache file")?;

    Ok(())
}

/// Load individual emote from cache by emote ID
pub fn load_emote_from_cache(emote_id: &str) -> Result<Option<String>> {
    let cache_dir = get_cache_dir()?.join("emotes");
    let cache_file = cache_dir.join(format!("{}.json", emote_id));

    if !cache_file.exists() {
        return Ok(None);
    }

    let json = fs::read_to_string(&cache_file).context("Failed to read emote cache file")?;

    let cache: EmoteCache = serde_json::from_str(&json).context("Failed to parse emote cache")?;

    // Check if cache is expired
    if is_cache_expired(&cache.metadata) {
        // Delete expired cache
        let _ = fs::remove_file(&cache_file);
        return Ok(None);
    }

    Ok(Some(cache.data))
}

/// Save emote cache to disk (legacy - kept for backwards compatibility)
pub fn save_emote_cache(channel_id: &str, data: &str, expiry_days: u32) -> Result<()> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join(format!("emotes_{}.json", channel_id));

    let cache = EmoteCache {
        metadata: CacheMetadata {
            timestamp: get_current_timestamp(),
            expiry_days,
        },
        data: data.to_string(),
    };

    let json = serde_json::to_string(&cache)?;
    fs::write(&cache_file, json).context("Failed to write emote cache file")?;

    Ok(())
}

/// Load emote cache from disk (legacy - kept for backwards compatibility)
pub fn load_emote_cache(channel_id: &str) -> Result<Option<String>> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join(format!("emotes_{}.json", channel_id));

    if !cache_file.exists() {
        return Ok(None);
    }

    let json = fs::read_to_string(&cache_file).context("Failed to read emote cache file")?;

    let cache: EmoteCache = serde_json::from_str(&json).context("Failed to parse emote cache")?;

    // Check if cache is expired
    if is_cache_expired(&cache.metadata) {
        // Delete expired cache
        let _ = fs::remove_file(&cache_file);
        return Ok(None);
    }

    Ok(Some(cache.data))
}

/// Save badge cache to disk
pub fn save_badge_cache(
    cache_type: &str,
    channel_id: Option<&str>,
    data: &str,
    expiry_days: u32,
) -> Result<()> {
    let cache_dir = get_cache_dir()?;
    let filename = if let Some(id) = channel_id {
        format!("badges_{}_{}.json", cache_type, id)
    } else {
        format!("badges_{}.json", cache_type)
    };
    let cache_file = cache_dir.join(filename);

    let cache = BadgeCache {
        metadata: CacheMetadata {
            timestamp: get_current_timestamp(),
            expiry_days,
        },
        data: data.to_string(),
    };

    let json = serde_json::to_string(&cache)?;
    fs::write(&cache_file, json).context("Failed to write badge cache file")?;

    Ok(())
}

/// Load badge cache from disk
pub fn load_badge_cache(cache_type: &str, channel_id: Option<&str>) -> Result<Option<String>> {
    let cache_dir = get_cache_dir()?;
    let filename = if let Some(id) = channel_id {
        format!("badges_{}_{}.json", cache_type, id)
    } else {
        format!("badges_{}.json", cache_type)
    };
    let cache_file = cache_dir.join(filename);

    if !cache_file.exists() {
        return Ok(None);
    }

    let json = fs::read_to_string(&cache_file).context("Failed to read badge cache file")?;

    let cache: BadgeCache = serde_json::from_str(&json).context("Failed to parse badge cache")?;

    // Check if cache is expired
    if is_cache_expired(&cache.metadata) {
        // Delete expired cache
        let _ = fs::remove_file(&cache_file);
        return Ok(None);
    }

    Ok(Some(cache.data))
}

/// Delete all cache files
pub fn clear_all_cache() -> Result<()> {
    let cache_dir = get_cache_dir()?;

    if cache_dir.exists() {
        // Read all files in the cache directory
        let entries = fs::read_dir(&cache_dir).context("Failed to read cache directory")?;

        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }

    Ok(())
}

/// Get cache statistics
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CacheStats {
    pub total_files: usize,
    pub total_size_bytes: u64,
    pub cache_dir: String,
}

pub fn get_cache_stats() -> Result<CacheStats> {
    let cache_dir = get_cache_dir()?;
    let mut total_files = 0;
    let mut total_size = 0u64;

    if cache_dir.exists() {
        let entries = fs::read_dir(&cache_dir).context("Failed to read cache directory")?;

        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() {
                    total_files += 1;
                    if let Ok(metadata) = fs::metadata(&path) {
                        total_size += metadata.len();
                    }
                }
            }
        }
    }

    Ok(CacheStats {
        total_files,
        total_size_bytes: total_size,
        cache_dir: cache_dir.to_string_lossy().to_string(),
    })
}

/// Save favorite emotes to cache (never expires)
pub fn save_favorite_emotes(data: &str) -> Result<()> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join("favorite_emotes.json");

    fs::write(&cache_file, data).context("Failed to write favorite emotes cache file")?;

    Ok(())
}

/// Load favorite emotes from cache
pub fn load_favorite_emotes() -> Result<Option<String>> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join("favorite_emotes.json");

    if !cache_file.exists() {
        return Ok(None);
    }

    let data =
        fs::read_to_string(&cache_file).context("Failed to read favorite emotes cache file")?;

    Ok(Some(data))
}

/// Add a single favorite emote to the cache
pub fn add_favorite_emote(emote_data: &str) -> Result<()> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join("favorite_emotes.json");

    // Load existing favorites
    let mut favorites: Vec<serde_json::Value> = if cache_file.exists() {
        let data =
            fs::read_to_string(&cache_file).context("Failed to read favorite emotes cache file")?;
        serde_json::from_str(&data).unwrap_or_else(|_| Vec::new())
    } else {
        Vec::new()
    };

    // Parse the new emote
    let new_emote: serde_json::Value =
        serde_json::from_str(emote_data).context("Failed to parse emote data")?;

    // Check if emote already exists (by id)
    let emote_id = new_emote.get("id").and_then(|v| v.as_str());
    if let Some(id) = emote_id {
        if !favorites
            .iter()
            .any(|e| e.get("id").and_then(|v| v.as_str()) == Some(id))
        {
            favorites.push(new_emote);
        }
    }

    // Save back to file
    let json = serde_json::to_string(&favorites)?;
    fs::write(&cache_file, json).context("Failed to write favorite emotes cache file")?;

    Ok(())
}

/// Remove a favorite emote from the cache
pub fn remove_favorite_emote(emote_id: &str) -> Result<()> {
    let cache_dir = get_cache_dir()?;
    let cache_file = cache_dir.join("favorite_emotes.json");

    if !cache_file.exists() {
        return Ok(());
    }

    // Load existing favorites
    let data =
        fs::read_to_string(&cache_file).context("Failed to read favorite emotes cache file")?;
    let mut favorites: Vec<serde_json::Value> =
        serde_json::from_str(&data).unwrap_or_else(|_| Vec::new());

    // Remove the emote with matching id
    favorites.retain(|e| e.get("id").and_then(|v| v.as_str()) != Some(emote_id));

    // Save back to file
    let json = serde_json::to_string(&favorites)?;
    fs::write(&cache_file, json).context("Failed to write favorite emotes cache file")?;

    Ok(())
}
