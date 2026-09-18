use crate::services::cache_service::{
    add_favorite_emote, clear_all_cache, get_cache_stats, load_badge_cache, load_emote_cache,
    load_emote_from_cache, load_favorite_emotes, remove_favorite_emote, save_badge_cache,
    save_emote_cache, save_emote_to_cache, save_favorite_emotes, CacheStats,
};
use tauri::command;

#[command]
pub async fn save_emote_by_id(
    emote_id: String,
    data: String,
    expiry_days: u32,
) -> Result<(), String> {
    save_emote_to_cache(&emote_id, &data, expiry_days).map_err(|e| e.to_string())
}

#[command]
pub async fn load_emote_by_id(emote_id: String) -> Result<Option<String>, String> {
    load_emote_from_cache(&emote_id).map_err(|e| e.to_string())
}

#[command]
pub async fn save_emotes_to_cache(
    channel_id: String,
    data: String,
    expiry_days: u32,
) -> Result<(), String> {
    save_emote_cache(&channel_id, &data, expiry_days).map_err(|e| e.to_string())
}

#[command]
pub async fn load_emotes_from_cache(channel_id: String) -> Result<Option<String>, String> {
    load_emote_cache(&channel_id).map_err(|e| e.to_string())
}

#[command]
pub async fn save_badges_to_cache(
    cache_type: String,
    channel_id: Option<String>,
    data: String,
    expiry_days: u32,
) -> Result<(), String> {
    save_badge_cache(&cache_type, channel_id.as_deref(), &data, expiry_days)
        .map_err(|e| e.to_string())
}

#[command]
pub async fn load_badges_from_cache(
    cache_type: String,
    channel_id: Option<String>,
) -> Result<Option<String>, String> {
    load_badge_cache(&cache_type, channel_id.as_deref()).map_err(|e| e.to_string())
}

#[command]
pub async fn clear_cache() -> Result<(), String> {
    clear_all_cache().map_err(|e| e.to_string())
}

#[command]
pub async fn get_cache_statistics() -> Result<CacheStats, String> {
    get_cache_stats().map_err(|e| e.to_string())
}

#[command]
pub async fn save_favorite_emotes_cache(data: String) -> Result<(), String> {
    save_favorite_emotes(&data).map_err(|e| e.to_string())
}

#[command]
pub async fn load_favorite_emotes_cache() -> Result<Option<String>, String> {
    load_favorite_emotes().map_err(|e| e.to_string())
}

#[command]
pub async fn add_favorite_emote_cache(emote_data: String) -> Result<(), String> {
    add_favorite_emote(&emote_data).map_err(|e| e.to_string())
}

#[command]
pub async fn remove_favorite_emote_cache(emote_id: String) -> Result<(), String> {
    remove_favorite_emote(&emote_id).map_err(|e| e.to_string())
}
