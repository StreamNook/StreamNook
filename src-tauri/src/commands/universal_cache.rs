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
    let dir = get_universal_cache_dir().map_err(|e| e.to_string())?;
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
    cleanup_expired_entries().map_err(|e| e.to_string())
}

#[command]
pub async fn clear_all_universal_cache(app: tauri::AppHandle) -> Result<(), String> {
    clear_universal_cache().map_err(|e| e.to_string())?;
    crate::services::asset_cache_queue::announce_cleared(&app);
    Ok(())
}

#[command]
pub async fn get_universal_cache_statistics() -> Result<UniversalCacheStats, String> {
    get_universal_cache_stats().map_err(|e| e.to_string())
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
    export_manifest_for_github(path).map_err(|e| e.to_string())
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

    get_cached_file_path(cache_type_enum, &id)
        .await
        .map_err(|e| e.to_string())
}

#[command]
pub async fn get_cached_files(cache_type: String) -> Result<HashMap<String, String>, String> {
    let cache_type_enum = match cache_type.as_str() {
        "badge" => CacheType::Badge,
        "emote" => CacheType::Emote,
        "third-party-badge" => CacheType::ThirdPartyBadge,
        "cosmetic" => CacheType::Cosmetic,
        _ => return Err(format!("Invalid cache type: {}", cache_type)),
    };

    get_cached_files_list(cache_type_enum)
        .await
        .map_err(|e| e.to_string())
}

/// Get ALL cached items of a specific type - efficient batch lookup (single disk read)
#[command]
pub fn get_all_universal_cached_items(
    cache_type: String,
) -> Result<HashMap<String, UniversalCacheEntry>, String> {
    let cache_type_enum = match cache_type.as_str() {
        "badge" => CacheType::Badge,
        "emote" => CacheType::Emote,
        "third-party-badge" => CacheType::ThirdPartyBadge,
        "cosmetic" => CacheType::Cosmetic,
        _ => return Err(format!("Invalid cache type: {}", cache_type)),
    };

    get_all_cached_items_by_type(cache_type_enum).map_err(|e| e.to_string())
}

/// Get multiple cached items by their IDs - efficient batch lookup (single disk read)
#[command]
pub fn get_universal_cached_items_batch(
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

    get_cached_items_batch(cache_type_enum, &ids).map_err(|e| e.to_string())
}

/// Auto-sync universal cache if stale (>24 hours since last sync)
/// Returns true if sync was triggered, false if cache was fresh
#[command]
pub async fn auto_sync_universal_cache_if_stale() -> Result<bool, String> {
    auto_sync_if_stale().await.map_err(|e| e.to_string())
}

/// Queue files for the shared disk-cache fill (services/asset_cache_queue.rs).
/// Results arrive as `asset-cache://cached`.
#[command]
pub fn asset_cache_enqueue(
    app: tauri::AppHandle,
    kind: crate::services::asset_cache_queue::AssetKind,
    items: Vec<crate::services::asset_cache_queue::AssetRequest>,
    priority: Option<bool>,
) {
    crate::services::asset_cache_queue::enqueue(&app, kind, items, priority.unwrap_or(false));
}

/// An emote picker opened or closed in the calling window.
#[command]
pub fn asset_cache_set_burst(app: tauri::AppHandle, window: tauri::WebviewWindow, active: bool) {
    crate::services::asset_cache_queue::set_burst(&app, window.label(), active);
}
