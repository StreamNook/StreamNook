use crate::services::twitch_service::TwitchService;
use crate::services::universal_cache_service::{cache_item, get_cached_item, CacheType};
use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex as TokioMutex;

// --- HELIX API STRUCTS ---

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HelixBadgeVersion {
    pub id: String,
    pub image_url_1x: String,
    pub image_url_2x: String,
    pub image_url_4x: String,
    pub title: String,
    pub description: String,
    pub click_action: Option<String>,
    pub click_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HelixBadgeSet {
    pub set_id: String,
    pub versions: Vec<HelixBadgeVersion>,
}

/// This is the top-level response from the Helix API
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HelixBadgesResponse {
    pub data: Vec<HelixBadgeSet>,
}

/// Cached badges with metadata
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CachedBadgesData {
    pub badges: HelixBadgesResponse,
    pub cached_at: u64,
}

// --- TAURI COMMANDS ---

/// Get current timestamp in seconds
fn get_current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Fetch global Twitch badges from API (no cache)
async fn fetch_badges_from_api(
    client_id: String,
    token: String,
) -> Result<HelixBadgesResponse, String> {
    let url = "https://api.twitch.tv/helix/chat/badges/global";

    debug!("[Badges] Fetching global badges from Twitch API...");

    let client = crate::services::http::client().clone();
    let response = client
        .get(url)
        .header("Client-Id", client_id)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch global badges: HTTP {} - {}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }

    let badges = response
        .json::<HelixBadgesResponse>()
        .await
        .map_err(|e| format!("Failed to parse global badges: {}", e))?;

    debug!(
        "[Badges] Successfully fetched {} badge sets from API",
        badges.data.len()
    );

    Ok(badges)
}

/// Fetch global Twitch badges with caching support
#[tauri::command]
pub async fn fetch_global_badges() -> Result<HelixBadgesResponse, String> {
    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;
    // Serialize against the socket-drop merge so a fetch and a merge can't
    // clobber each other's write of the shared global_badges entry.
    let _guard = GLOBAL_BADGES_LOCK.lock().await;
    // Try to get from cache first
    let cache_key = "global_badges";

    match get_cached_item(CacheType::Badge, cache_key).await {
        Ok(Some(cached)) => {
            match serde_json::from_value::<CachedBadgesData>(cached.data) {
                Ok(cached_data) => {
                    // Check if cache is less than 7 days old
                    let cache_age_days =
                        (get_current_timestamp() - cached_data.cached_at) / (24 * 60 * 60);

                    if cache_age_days < 7 {
                        debug!(
                            "[Badges] Using cached badges (age: {} days)",
                            cache_age_days
                        );
                        return Ok(cached_data.badges);
                    } else {
                        debug!(
                            "[Badges] Cache is {} days old, refreshing...",
                            cache_age_days
                        );
                    }
                }
                Err(e) => {
                    debug!("[Badges] Failed to parse cached badges: {}", e);
                }
            }
        }
        Ok(None) => {
            debug!("[Badges] No cached badges found");
        }
        Err(e) => {
            debug!("[Badges] Error checking cache: {}", e);
        }
    }

    // Fetch from API
    let badges = fetch_badges_from_api(client_id, token).await?;

    // Cache the result for 7 days
    let cached_data = CachedBadgesData {
        badges: badges.clone(),
        cached_at: get_current_timestamp(),
    };

    if let Ok(json_value) = serde_json::to_value(&cached_data) {
        let _ = cache_item(
            CacheType::Badge,
            cache_key.to_string(),
            json_value,
            "twitch".to_string(),
            7, // Cache for 7 days
        )
        .await;
        debug!("[Badges] Cached global badges for 7 days");
    }

    Ok(badges)
}

/// Get cached global badges without fetching from API
#[tauri::command]
pub async fn get_cached_global_badges() -> Result<Option<HelixBadgesResponse>, String> {
    let cache_key = "global_badges";

    match get_cached_item(CacheType::Badge, cache_key).await {
        Ok(Some(cached)) => match serde_json::from_value::<CachedBadgesData>(cached.data) {
            Ok(cached_data) => {
                debug!(
                    "[Badges] Retrieved {} badge sets from cache",
                    cached_data.badges.data.len()
                );
                Ok(Some(cached_data.badges))
            }
            Err(e) => {
                debug!("[Badges] Failed to parse cached badges: {}", e);
                Ok(None)
            }
        },
        Ok(None) => {
            debug!("[Badges] No cached badges found");
            Ok(None)
        }
        Err(e) => {
            debug!("[Badges] Error checking cache: {}", e);
            Err(format!("Cache error: {}", e))
        }
    }
}

/// Pre-fetch and cache global badges in the background
#[tauri::command]
pub async fn prefetch_global_badges() -> Result<(), String> {
    debug!("[Badges] Starting background badge pre-fetch...");

    match TwitchService::get_token().await {
        Ok(_) => match fetch_global_badges().await {
            Ok(badges) => {
                debug!(
                    "[Badges] Pre-fetch complete: {} badge sets cached",
                    badges.data.len()
                );
                Ok(())
            }
            Err(e) => {
                debug!("[Badges] Pre-fetch failed: {}", e);
                Err(e)
            }
        },
        Err(e) => {
            debug!("[Badges] Failed to get token for pre-fetch: {}", e);
            Err(format!("Failed to get token: {}", e))
        }
    }
}

/// Force refresh global badges from Twitch API (bypasses cache)
#[tauri::command]
pub async fn force_refresh_global_badges() -> Result<HelixBadgesResponse, String> {
    // Serialize against the socket-drop merge (see fetch_global_badges).
    let _guard = GLOBAL_BADGES_LOCK.lock().await;
    debug!("[Badges] Force refreshing global badges from Twitch API...");

    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    // Fetch directly from API (bypassing cache check)
    let badges = fetch_badges_from_api(client_id.clone(), token.clone()).await?;

    // Cache the result for 7 days
    let cached_data = CachedBadgesData {
        badges: badges.clone(),
        cached_at: get_current_timestamp(),
    };

    let cache_key = "global_badges";
    if let Ok(json_value) = serde_json::to_value(&cached_data) {
        let _ = cache_item(
            CacheType::Badge,
            cache_key.to_string(),
            json_value,
            "twitch".to_string(),
            7, // Cache for 7 days
        )
        .await;
        debug!(
            "[Badges] Force refreshed and cached {} badge sets",
            badges.data.len()
        );
    }

    Ok(badges)
}

/// Get badge cache age in days (returns None if not cached)
/// Reports the age of whichever is fresher: the local badge data or the GitHub manifest sync.
/// After a local force-refresh, this will show "0 days" instead of the stale GitHub sync age.
#[tauri::command]
pub async fn get_badge_cache_age() -> Result<Option<u64>, String> {
    use crate::services::universal_cache_service::{manifest_last_sync, peek_cached_entry};

    // Read the one entry and the sync stamp, never the whole manifest: cloning
    // every entry to look at one of them costs ~80 ms and ~20 MB on a
    // long-lived cache.
    // Check if locally-cached badge data is newer than the last GitHub sync
    let cache_key = "global_badges";
    let cached_entry =
        peek_cached_entry(cache_key).map_err(|e| format!("Failed to read manifest: {}", e))?;
    if let Some(entry) = cached_entry.as_ref() {
        let badge_timestamp = entry.metadata.timestamp;
        let manifest_sync = manifest_last_sync()
            .map_err(|e| format!("Failed to read manifest: {}", e))?
            .unwrap_or(0);

        // If badge data is newer than last GitHub sync, report badge data age
        if badge_timestamp > manifest_sync {
            let cache_age_days = (get_current_timestamp() - badge_timestamp) / (24 * 60 * 60);
            return Ok(Some(cache_age_days));
        }
    }

    // Fall back to manifest sync age
    match manifest_last_sync().map_err(|e| format!("Failed to read manifest: {}", e))? {
        Some(last_sync) => {
            let cache_age_days = (get_current_timestamp() - last_sync) / (24 * 60 * 60);
            Ok(Some(cache_age_days))
        }
        None => Ok(None),
    }
}

/// Check for new badges that don't have metadata cached yet
/// Returns list of badge identifiers (set_id/version) that need metadata fetching
#[tauri::command]
pub async fn get_badges_missing_metadata() -> Result<Vec<(String, String)>, String> {
    use crate::services::universal_cache_service::peek_cached_entry;

    debug!("[Badges] Checking for badges missing metadata...");

    // Read local cache only (no GitHub download), one entry rather than a full
    // manifest clone.
    let cache_key = "global_badges";
    let cached_entry =
        peek_cached_entry(cache_key).map_err(|e| format!("Failed to read manifest: {}", e))?;
    let badges = match cached_entry.as_ref() {
        Some(entry) => match serde_json::from_value::<CachedBadgesData>(entry.data.clone()) {
            Ok(cached_data) => {
                debug!(
                    "[Badges] Found {} badge sets in local cache",
                    cached_data.badges.data.len()
                );
                cached_data.badges
            }
            Err(e) => {
                debug!("[Badges] Failed to parse cached badges: {}", e);
                return Ok(vec![]);
            }
        },
        None => {
            debug!("[Badges] No global badges in local cache");
            return Ok(vec![]);
        }
    };

    let mut missing = Vec::new();
    let mut total_versions = 0;

    // Check each badge version for metadata in local cache only
    for badge_set in &badges.data {
        for version in &badge_set.versions {
            total_versions += 1;
            let metadata_key = format!("metadata:{}-v{}", badge_set.set_id, version.id);

            // Check local cache only - don't trigger GitHub download. One entry
            // per version, never a full-manifest clone per lookup.
            let entry = peek_cached_entry(&metadata_key)
                .map_err(|e| format!("Failed to read manifest: {}", e))?;
            let needs_refetch = match entry.as_ref() {
                None => true,
                Some(e) => {
                    // Stale entries scraped before the timezone-converter fix have a date
                    // range in human form but no ISO timestamp, which the UI can't classify.
                    let more_info = e.data.get("more_info").and_then(|v| v.as_str());
                    // Entries scraped before the source served usage figures have a null
                    // usage_stats, which leaves the most/least-used sort with nothing to
                    // order by. Re-scrape those to populate the count.
                    let usage_stats = e.data.get("usage_stats").and_then(|v| v.as_str());
                    crate::commands::badge_metadata::is_more_info_stale(more_info)
                        || crate::commands::badge_metadata::is_usage_stats_missing(usage_stats)
                }
            };
            if needs_refetch {
                debug!(
                    "[Badges] Missing or stale metadata for: {} v{} ({})",
                    badge_set.set_id, version.id, version.title
                );
                missing.push((badge_set.set_id.clone(), version.id.clone()));
            }
        }
    }

    debug!(
        "[Badges] Checked {} badge versions, found {} missing metadata",
        total_versions,
        missing.len()
    );
    Ok(missing)
}

/// Serializes every read-modify-write of the single `global_badges` cache entry
/// (the socket-drop merge below + the Helix fetch/refresh writers) so concurrent
/// startup-catch-up drops and a lazy gallery fetch can't lose each other's writes.
static GLOBAL_BADGES_LOCK: Lazy<TokioMutex<()>> = Lazy::new(|| TokioMutex::new(()));

/// Derive Twitch's 1x/2x/4x badge CDN URLs from a single pushed URL. Twitch badge
/// URLs end in a size segment (/1, /2, /3); if the pushed URL matches, build the
/// three variants, else reuse the same URL for every size so the tile still shows.
fn derive_badge_image_urls(url: &str) -> (String, String, String) {
    for suffix in ["/1", "/2", "/3"] {
        if let Some(base) = url.strip_suffix(suffix) {
            return (format!("{base}/1"), format!("{base}/2"), format!("{base}/3"));
        }
    }
    (url.to_string(), url.to_string(), url.to_string())
}

/// Real badge art always comes from Twitch's CDN. Anything else in a drop
/// payload is a test or malformed push, and merging it leaves a broken tile.
fn is_twitch_badge_image(url: &str) -> bool {
    url.starts_with("https://static-cdn.jtvnw.net/badges/")
}

/// Merge a relay-pushed badge drop into the cached `global_badges` set so the
/// Global Cosmetics gallery shows it without waiting for a Helix refresh.
/// Idempotent (an existing version is a no-op). Preserves `cached_at` so the
/// Helix refresh cadence is unchanged. Returns Ok(true) when the cache changed.
/// No-ops (returns Ok(false)) when the gallery cache hasn't been populated yet —
/// the badge then appears the next time the gallery fetches Helix.
pub async fn merge_pushed_badge_into_global_cache(
    badge: &crate::services::badge_polling_service::BadgeNotification,
) -> Result<bool, String> {
    if !is_twitch_badge_image(&badge.badge_image_url) {
        return Ok(false);
    }

    let _guard = GLOBAL_BADGES_LOCK.lock().await;
    let cache_key = "global_badges";

    let cached = match get_cached_item(CacheType::Badge, cache_key).await {
        Ok(Some(c)) => c,
        _ => return Ok(false),
    };
    let mut cached_data: CachedBadgesData = serde_json::from_value(cached.data)
        .map_err(|e| format!("parse cached badges: {e}"))?;

    let (u1, u2, u4) = derive_badge_image_urls(&badge.badge_image_url);
    let version = HelixBadgeVersion {
        id: badge.badge_version.clone(),
        image_url_1x: u1,
        image_url_2x: u2,
        image_url_4x: u4,
        title: badge.badge_name.clone(),
        description: badge.badge_description.clone().unwrap_or_default(),
        click_action: None,
        click_url: None,
    };

    if let Some(set) = cached_data
        .badges
        .data
        .iter_mut()
        .find(|s| s.set_id == badge.badge_set_id)
    {
        if set.versions.iter().any(|v| v.id == version.id) {
            return Ok(false);
        }
        set.versions.push(version);
    } else {
        cached_data.badges.data.push(HelixBadgeSet {
            set_id: badge.badge_set_id.clone(),
            versions: vec![version],
        });
    }

    if let Ok(json_value) = serde_json::to_value(&cached_data) {
        let _ = cache_item(
            CacheType::Badge,
            cache_key.to_string(),
            json_value,
            "twitch".to_string(),
            7,
        )
        .await;
    }
    Ok(true)
}

/// Drop gallery entries whose art does not come from Twitch's badge CDN, for
/// installs that merged a bad drop before the guard above existed. Runs once
/// per process, on the first feed ingest. Returns the number of sets removed.
pub async fn prune_invalid_global_badges() -> Result<usize, String> {
    static PRUNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if PRUNED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Ok(0);
    }

    let _guard = GLOBAL_BADGES_LOCK.lock().await;
    let cache_key = "global_badges";

    let cached = match get_cached_item(CacheType::Badge, cache_key).await {
        Ok(Some(c)) => c,
        _ => return Ok(0),
    };
    let mut cached_data: CachedBadgesData = serde_json::from_value(cached.data)
        .map_err(|e| format!("parse cached badges: {e}"))?;

    let before = cached_data.badges.data.len();
    cached_data.badges.data.retain(|set| {
        set.versions
            .iter()
            .any(|v| is_twitch_badge_image(&v.image_url_4x) || is_twitch_badge_image(&v.image_url_1x))
    });
    let removed = before - cached_data.badges.data.len();
    if removed == 0 {
        return Ok(0);
    }

    if let Ok(json_value) = serde_json::to_value(&cached_data) {
        let _ = cache_item(
            CacheType::Badge,
            cache_key.to_string(),
            json_value,
            "twitch".to_string(),
            7,
        )
        .await;
    }
    debug!("[Badges] Pruned {removed} gallery entr(ies) with non-Twitch art");
    Ok(removed)
}


/// Debug command: List all badge set IDs from Twitch API (bypasses all caching)
/// Returns a list of (set_id, version_count, version_ids)
#[tauri::command]
pub async fn debug_list_twitch_badges() -> Result<Vec<(String, usize, Vec<String>)>, String> {
    debug!("[Badges/Debug] Fetching badges directly from Twitch API...");

    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let badges = fetch_badges_from_api(client_id, token).await?;

    let mut result = Vec::new();
    for badge_set in &badges.data {
        let version_ids: Vec<String> = badge_set
            .versions
            .iter()
            .map(|v| format!("{} ({})", v.id, v.title))
            .collect();

        debug!(
            "[Badges/Debug] Set: {} - {} versions: {:?}",
            badge_set.set_id,
            badge_set.versions.len(),
            version_ids
        );

        result.push((
            badge_set.set_id.clone(),
            badge_set.versions.len(),
            version_ids,
        ));
    }

    debug!("[Badges/Debug] Total: {} badge sets", result.len());
    Ok(result)
}

/// Debug command: Compare Twitch API badges with cached badges
/// Returns (api_only, cached_only, both) - badges only in API, only in cache, or in both
#[tauri::command]
pub async fn debug_compare_badge_sources() -> Result<(Vec<String>, Vec<String>, Vec<String>), String>
{
    use crate::services::universal_cache_service::peek_cached_entry;
    use std::collections::HashSet;

    debug!("[Badges/Debug] Comparing Twitch API badges with cached badges...");

    // Get badges from Twitch API
    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let api_badges = fetch_badges_from_api(client_id, token).await?;

    // Get badges from cache (single entry, no full-manifest clone)
    let cached_entry = peek_cached_entry("global_badges")
        .map_err(|e| format!("Failed to read manifest: {}", e))?;
    let cached_badges = match cached_entry.as_ref() {
        Some(entry) => match serde_json::from_value::<CachedBadgesData>(entry.data.clone()) {
            Ok(data) => data.badges,
            Err(_) => return Err("Failed to parse cached badges".to_string()),
        },
        None => return Err("No cached badges found".to_string()),
    };

    // Build sets of badge identifiers
    let mut api_set: HashSet<String> = HashSet::new();
    for badge_set in &api_badges.data {
        for version in &badge_set.versions {
            api_set.insert(format!("{}/v{}", badge_set.set_id, version.id));
        }
    }

    let mut cache_set: HashSet<String> = HashSet::new();
    for badge_set in &cached_badges.data {
        for version in &badge_set.versions {
            cache_set.insert(format!("{}/v{}", badge_set.set_id, version.id));
        }
    }

    // Compare
    let api_only: Vec<String> = api_set.difference(&cache_set).cloned().collect();
    let cache_only: Vec<String> = cache_set.difference(&api_set).cloned().collect();
    let both: Vec<String> = api_set.intersection(&cache_set).cloned().collect();

    debug!("[Badges/Debug] API only: {:?}", api_only);
    debug!("[Badges/Debug] Cache only: {:?}", cache_only);
    debug!("[Badges/Debug] In both: {} badges", both.len());

    Ok((api_only, cache_only, both))
}

/// Fetch channel-specific Twitch badges using the Helix API
#[tauri::command]
pub async fn fetch_channel_badges(channel_id: String) -> Result<HelixBadgesResponse, String> {
    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;
    let url = format!(
        "https://api.twitch.tv/helix/chat/badges?broadcaster_id={}",
        channel_id
    );

    let client = crate::services::http::client().clone();
    let response = client
        .get(&url)
        .header("Client-Id", client_id)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch channel badges: HTTP {} - {}",
            response.status(),
            response.text().await.unwrap_or_default()
        ));
    }

    let badges = response
        .json::<HelixBadgesResponse>()
        .await
        .map_err(|e| format!("Failed to parse channel badges: {}", e))?;

    Ok(badges)
}

/// Get user's badges for a specific channel
/// Returns a badge string in the format "badge1/version1,badge2/version2"
/// Note: This uses the /users endpoint to get broadcaster_type and constructs badges from that
#[tauri::command]
pub async fn get_user_badges(
    user_id: String,
    channel_id: Option<String>,
) -> Result<String, String> {
    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = crate::services::http::client().clone();

    // Get user information to check broadcaster_type
    let user_url = format!("https://api.twitch.tv/helix/users?id={}", user_id);
    let user_response = client
        .get(&user_url)
        .header("Client-Id", &client_id)
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| format!("Failed to send request: {}", e))?;

    if !user_response.status().is_success() {
        return Err(format!(
            "Failed to fetch user info: HTTP {} - {}",
            user_response.status(),
            user_response.text().await.unwrap_or_default()
        ));
    }

    #[derive(Debug, Deserialize)]
    struct UserData {
        broadcaster_type: String,
    }

    #[derive(Debug, Deserialize)]
    struct UsersResponse {
        data: Vec<UserData>,
    }

    let users_response = user_response
        .json::<UsersResponse>()
        .await
        .map_err(|e| format!("Failed to parse user info: {}", e))?;

    let mut badges = Vec::new();

    // Add broadcaster badges based on broadcaster_type
    if let Some(user_data) = users_response.data.first() {
        match user_data.broadcaster_type.as_str() {
            "partner" => badges.push("partner/1".to_string()),
            "affiliate" => badges.push("affiliate/1".to_string()),
            _ => {}
        }
    }

    // If channel_id is provided, check for subscriber status
    if let Some(broadcaster_id) = channel_id {
        // Check if user is subscribed to the channel
        let sub_url = format!(
            "https://api.twitch.tv/helix/subscriptions/user?broadcaster_id={}&user_id={}",
            broadcaster_id, user_id
        );

        let sub_response = client
            .get(&sub_url)
            .header("Client-Id", &client_id)
            .header("Authorization", format!("Bearer {}", &token))
            .send()
            .await;

        if let Ok(response) = sub_response {
            if response.status().is_success() {
                #[derive(Debug, Deserialize)]
                struct SubData {
                    tier: String,
                }

                #[derive(Debug, Deserialize)]
                struct SubResponse {
                    data: Vec<SubData>,
                }

                if let Ok(sub_data) = response.json::<SubResponse>().await {
                    if let Some(sub) = sub_data.data.first() {
                        // Map tier to subscriber badge version
                        let badge_version = match sub.tier.as_str() {
                            "1000" => "0",    // Tier 1
                            "2000" => "2000", // Tier 2
                            "3000" => "3000", // Tier 3
                            _ => "0",
                        };
                        badges.push(format!("subscriber/{}", badge_version));
                    }
                }
            }
        }
    }

    Ok(badges.join(","))
}
