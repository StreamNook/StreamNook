use crate::services::universal_cache_service::{
    assign_badge_metadata_positions, auto_sync_if_stale, cache_item,
    cleanup_expired_entries, clear_universal_cache, export_manifest_for_github,
    get_all_cached_items_by_type, get_cached_file_path, get_cached_files_list, get_cached_item,
    get_cached_items_batch, get_universal_cache_dir, get_universal_cache_stats,
    sync_universal_cache, CacheType, UniversalCacheEntry, UniversalCacheStats,
};

use std::collections::HashMap;
use std::path::PathBuf;
use tauri::command;

/// Open the universal cache folder in the OS file manager. Local paths cannot go
/// through the shell plugin (its `open` scope only allows http/mailto/tel URLs),
/// so this launches the platform file manager directly.
#[command]
pub async fn open_universal_cache_folder() -> Result<(), String> {
    if cfg!(target_os = "ios") {
        return Err("Opening folders is not available on iPhone".into());
    }
    // On the blocking pool: resolving the folder checks the disk and may create it.
    let dir = tauri::async_runtime::spawn_blocking(get_universal_cache_dir)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    let program = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(dir.as_os_str())
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[command]
pub async fn get_universal_cached_item(
    cache_type: String,
    id: String,
) -> Result<Option<UniversalCacheEntry>, String> {
    let cache_type_enum = match cache_type.as_str() {
        "badge" => CacheType::Badge,
        "emote" => CacheType::Emote,
        "third-party-badge" => CacheType::ThirdPartyBadge,
        "cosmetic" => CacheType::Cosmetic,
        _ => return Err(format!("Invalid cache type: {}", cache_type)),
    };

    get_cached_item(cache_type_enum, &id)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn save_universal_cached_item(
    cache_type: String,
    id: String,
    data: serde_json::Value,
    source: String,
    expiry_days: u32,
) -> Result<(), String> {
    let cache_type_enum = match cache_type.as_str() {
        "badge" => CacheType::Badge,
        "emote" => CacheType::Emote,
        "third-party-badge" => CacheType::ThirdPartyBadge,
        "cosmetic" => CacheType::Cosmetic,
        _ => return Err(format!("Invalid cache type: {}", cache_type)),
    };

    cache_item(cache_type_enum, id, data, source, expiry_days)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn sync_universal_cache_data(cache_types: Vec<String>) -> Result<usize, String> {
    let mut types = Vec::new();

    for cache_type in cache_types {
        let cache_type_enum = match cache_type.as_str() {
            "badge" => CacheType::Badge,
            "emote" => CacheType::Emote,
            "third-party-badge" => CacheType::ThirdPartyBadge,
            "cosmetic" => CacheType::Cosmetic,
            _ => continue,
        };
        types.push(cache_type_enum);
    }

    sync_universal_cache(types).await.map_err(|e| e.to_string())
}

#[command]
pub async fn cleanup_universal_cache() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(cleanup_expired_entries)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[command]
pub async fn clear_all_universal_cache(app: crate::rt::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(clear_universal_cache)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    crate::services::asset_cache_queue::announce_cleared(&app);
    Ok(())
}

/// On the blocking pool: a statistics read clones the manifest and lists the
/// cache folder.
#[command]
pub async fn get_universal_cache_statistics() -> Result<UniversalCacheStats, String> {
    tauri::async_runtime::spawn_blocking(get_universal_cache_stats)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[command]
pub async fn assign_badge_positions() -> Result<usize, String> {
    assign_badge_metadata_positions()
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn export_manifest(output_path: String) -> Result<(), String> {
    let path = PathBuf::from(output_path);
    tauri::async_runtime::spawn_blocking(move || export_manifest_for_github(path))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[command]
pub async fn get_cached_file(cache_type: String, id: String) -> Result<Option<String>, String> {
    let cache_type_enum = match cache_type.as_str() {
        "badge" => CacheType::Badge,
        "emote" => CacheType::Emote,
        "third-party-badge" => CacheType::ThirdPartyBadge,
        "cosmetic" => CacheType::Cosmetic,
        _ => return Err(format!("Invalid cache type: {}", cache_type)),
    };

    // On the blocking pool: the lookup checks the file on disk. It is async only
    // because a stale manifest is downloaded first, which the closure waits for.
    tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(get_cached_file_path(cache_type_enum, &id))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}

#[command]
pub async fn get_cached_files(cache_type: String) -> Result<HashMap<String, String>, String> {
    let cache_type_enum = match cache_type.as_str() {
        "badge" => CacheType::Badge,
        "emote" => CacheType::Emote,
        "third-party-badge" => CacheType::ThirdPartyBadge,
        "cosmetic" => CacheType::Cosmetic,
        "streamnook" => CacheType::StreamNookArt,
        _ => return Err(format!("Invalid cache type: {}", cache_type)),
    };

    // On the blocking pool: the first listing of a type reads its folder. The
    // service function is async but never waits on anything.
    tauri::async_runtime::spawn_blocking(move || tauri::async_runtime::block_on(get_cached_files_list(cache_type_enum)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Get ALL cached items of a specific type - efficient batch lookup (single disk read)
#[command]
pub async fn get_all_universal_cached_items(
    cache_type: String,
) -> Result<HashMap<String, UniversalCacheEntry>, String> {
    let cache_type_enum = match cache_type.as_str() {
        "badge" => CacheType::Badge,
        "emote" => CacheType::Emote,
        "third-party-badge" => CacheType::ThirdPartyBadge,
        "cosmetic" => CacheType::Cosmetic,
        _ => return Err(format!("Invalid cache type: {}", cache_type)),
    };

    // On the blocking pool: a sync command runs on the UI thread, and the first
    // read of a type lists its folder.
    tauri::async_runtime::spawn_blocking(move || get_all_cached_items_by_type(cache_type_enum))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Get multiple cached items by their IDs - efficient batch lookup (single disk read)
#[command]
pub async fn get_universal_cached_items_batch(
    cache_type: String,
    ids: Vec<String>,
) -> Result<HashMap<String, UniversalCacheEntry>, String> {
    let cache_type_enum = match cache_type.as_str() {
        "badge" => CacheType::Badge,
        "emote" => CacheType::Emote,
        "third-party-badge" => CacheType::ThirdPartyBadge,
        "cosmetic" => CacheType::Cosmetic,
        _ => return Err(format!("Invalid cache type: {}", cache_type)),
    };

    tauri::async_runtime::spawn_blocking(move || get_cached_items_batch(cache_type_enum, &ids))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Auto-sync universal cache if stale (>24 hours since last sync)
/// Returns true if sync was triggered, false if cache was fresh
#[command]
pub async fn auto_sync_universal_cache_if_stale() -> Result<bool, String> {
    auto_sync_if_stale().await.map_err(|e| e.to_string())
}

/// Queue files for the shared disk-cache fill (services/asset_cache_queue.rs).
/// Results arrive as `asset-cache://cached`.
/// On the blocking pool: a sync command runs on the UI thread, and `enqueue`
/// reads the manifest (and lists a type folder the first time).
#[command]
pub async fn asset_cache_enqueue(
    app: crate::rt::AppHandle,
    kind: crate::services::asset_cache_queue::AssetKind,
    items: Vec<crate::services::asset_cache_queue::AssetRequest>,
    priority: Option<bool>,
) -> Result<(), String> {
    let priority = priority.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || crate::services::asset_cache_queue::enqueue(&app, kind, items, priority))
        .await
        .map_err(|e| e.to_string())
}

/// An emote picker opened or closed in the calling window.
#[command]
pub fn asset_cache_set_burst(app: crate::rt::AppHandle, window: crate::rt::WebviewWindow, active: bool) {
    crate::services::asset_cache_queue::set_burst(&app, window.label(), active);
}

/// The loopback server for cached images (`services/cache_file_server.rs`), or
/// none (not Windows, not bound yet, or the bind failed).
#[command]
pub async fn cache_file_server() -> Option<crate::services::cache_file_server::CacheFileServer> {
    crate::services::cache_file_server::info()
}
