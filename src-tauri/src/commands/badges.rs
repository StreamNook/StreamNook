use crate::services::twitch_service::TwitchService;
use crate::services::universal_cache_service::{cache_item, get_cached_item, CacheType, UniversalCacheEntry};
use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
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
    /// The (set, version) pairs the drop feed pushed, with the time of each push in
    /// seconds. A version only the cache holds survives a refresh only while its push
    /// is recent. `None` on a cache written before this field existed.
    #[serde(default)]
    pub pushed: Option<PushedPairs>,
}

// --- TAURI COMMANDS ---

/// The (set, version) pairs of a gallery read.
fn gallery_pairs(badges: &HelixBadgesResponse) -> std::collections::HashSet<(String, String)> {
    badges
        .data
        .iter()
        .flat_map(|s| s.versions.iter().map(move |v| (s.set_id.clone(), v.id.clone())))
        .collect()
}

/// The pairs the cache held before this read. `peek_cached_entry` still
/// answers once the entry expired; `None` when there was no cache.
fn previous_gallery_pairs() -> Option<std::collections::HashSet<(String, String)>> {
    let entry = crate::services::universal_cache_service::peek_cached_entry("global_badges").ok().flatten()?;
    let data = serde_json::from_value::<CachedBadgesData>(entry.data).ok()?;
    Some(gallery_pairs(&data.badges))
}

/// Tell the rules which (set, version) pairs a read added.
fn note_arrivals(previous: Option<&std::collections::HashSet<(String, String)>>, badges: &HelixBadgesResponse) {
    let added = crate::services::account_automations::badges::new_pairs(previous, &gallery_pairs(badges));
    if !added.is_empty() {
        crate::services::account_automations::note_arrivals(added);
    }
}

/// Get current timestamp in seconds
fn get_current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// "set/version" to the push time in seconds; `CachedBadgesData::pushed` holds one.
pub type PushedPairs = BTreeMap<String, u64>;

/// A version only the cache holds is kept this long after the drop feed last pushed it.
const PUSHED_KEEP_SECS: u64 = 14 * 86_400;

/// A pushed pair's time moves up only once it is this old, so a busy feed does not
/// rewrite the cache on every batch.
const RESTAMP_AFTER_SECS: u64 = 86_400;

/// How long the cached list is used before a gallery read asks Twitch again.
const LIST_USE_SECS: u64 = 7 * 86_400;

/// Bumped under the cache lock by each writer of Twitch's list just before it
/// fetches; the bumped value is that fetch's start number.
static FETCH_STARTS: AtomicU64 = AtomicU64::new(0);

/// The start number of the newest fetch whose list was fetched and written. A
/// failed fetch or a failed write leaves it where it was.
static WRITTEN_START: AtomicU64 = AtomicU64::new(0);

/// The cached gallery from an entry as stored, expired or not. Pure on purpose: the
/// cache service hides an expired entry from its own readers, and a version the drop
/// feed pushed lives only in that entry.
fn previous_from(entry: Option<UniversalCacheEntry>) -> Option<CachedBadgesData> {
    serde_json::from_value::<CachedBadgesData>(entry?.data).ok()
}

/// The one cache read of every writer of this list. It answers after expiry and never
/// downloads the hosted manifest.
fn previous_cache() -> Option<CachedBadgesData> {
    previous_from(crate::services::universal_cache_service::peek_cached_entry("global_badges").ok().flatten())
}

/// The start number a writer stores after its cache write: the newer of the two when
/// the write worked, the old one when it did not, so a failed write never lets a
/// queued refresh skip its own fetch.
fn written_start_after(current: u64, n: u64, wrote: bool) -> u64 {
    if wrote {
        n.max(current)
    } else {
        current
    }
}

/// Whether a refresh that asked when `seen` fetches had started can use the list that
/// a fetch stored while it waited. Only a fetch that started after the ask counts: an
/// older one may hold Twitch's older answer.
fn coalesce(seen: u64, written_start: u64) -> bool {
    written_start > seen
}

#[derive(Debug, PartialEq, Eq)]
enum FetchDecision {
    UseCached,
    Refresh,
}

/// The seven-day decision of `fetch_global_badges`. A stamp in the future (a stepped
/// clock) is stale, and so is no cache at all.
fn fetch_decision(previous: Option<&CachedBadgesData>, now: u64) -> FetchDecision {
    match previous {
        Some(data) if data.cached_at <= now && now - data.cached_at < LIST_USE_SECS => FetchDecision::UseCached,
        _ => FetchDecision::Refresh,
    }
}

fn pair_key(set_id: &str, version: &str) -> String {
    format!("{set_id}/{version}")
}

/// Every (set, version) the list holds, each stamped `at`.
fn pairs_at(badges: &HelixBadgesResponse, at: u64) -> PushedPairs {
    badges
        .data
        .iter()
        .flat_map(|s| s.versions.iter().map(move |v| (pair_key(&s.set_id, &v.id), at)))
        .collect()
}

/// Keeps each version only the previous cache holds, while the drop feed's push of it
/// is at most 14 days old. A set Twitch no longer lists comes back with only its kept
/// versions and is left out when none is kept. Returns the pushed pairs to write back:
/// pairs Twitch now lists or older than 14 days are gone, and every time is
/// `min(t, now)` so a stamp from a stepped clock never outlives its real age.
fn merge_cache_only(
    fetched: &mut HelixBadgesResponse,
    previous: Option<&CachedBadgesData>,
    now: u64,
) -> PushedPairs {
    let mut kept = PushedPairs::new();
    let Some(previous) = previous else {
        return kept;
    };
    // A cache written before pushes were recorded counts every version it holds as
    // pushed now, once, so nothing a person already had vanishes at the first refresh.
    let seeded;
    let pushed = match &previous.pushed {
        Some(pushed) => pushed,
        None => {
            seeded = pairs_at(&previous.badges, now);
            &seeded
        }
    };
    for old in &previous.badges.data {
        let mut survivors = Vec::new();
        for version in &old.versions {
            let listed = fetched
                .data
                .iter()
                .any(|s| s.set_id == old.set_id && s.versions.iter().any(|v| v.id == version.id));
            if listed {
                continue;
            }
            let key = pair_key(&old.set_id, &version.id);
            let Some(&at) = pushed.get(&key) else {
                continue;
            };
            if now.saturating_sub(at) > PUSHED_KEEP_SECS {
                continue;
            }
            kept.insert(key, at.min(now));
            survivors.push(version.clone());
        }
        if survivors.is_empty() {
            continue;
        }
        match fetched.data.iter_mut().find(|s| s.set_id == old.set_id) {
            Some(set) => set.versions.extend(survivors),
            None => fetched.data.push(HelixBadgeSet {
                set_id: old.set_id.clone(),
                versions: survivors,
            }),
        }
    }
    kept
}

/// Gives every pair of a feed batch that `pushed` already holds, and whose stamp is a
/// day old or more, the time `now`. True when one moved.
fn restamp(pushed: &mut PushedPairs, pairs: &[(String, String)], now: u64) -> bool {
    let mut moved = false;
    for (set_id, version) in pairs {
        if let Some(at) = pushed.get_mut(&pair_key(set_id, version)) {
            if now.saturating_sub(*at) >= RESTAMP_AFTER_SECS {
                *at = now;
                moved = true;
            }
        }
    }
    moved
}

/// The cache with a batch's pairs re-stamped, or `None` when nothing needs writing.
/// The `cached_at` it read goes back untouched: a new one would make a stale gallery
/// look freshly read. A cache with no record of pushes gets one here too, every
/// version stamped now.
fn restamped(previous: CachedBadgesData, pairs: &[(String, String)], now: u64) -> Option<CachedBadgesData> {
    let map_is_new = previous.pushed.is_none();
    let mut pushed = previous.pushed.clone().unwrap_or_else(|| pairs_at(&previous.badges, now));
    let moved = restamp(&mut pushed, pairs, now);
    if !moved && !map_is_new {
        return None;
    }
    Some(CachedBadgesData {
        pushed: Some(pushed),
        ..previous
    })
}

/// What a push did to the cache.
#[derive(Debug, PartialEq, Eq)]
enum Push {
    Unchanged,
    Restamped,
    Added,
}

/// Records a pushed version: its pair always gets the time `now`, and the version joins
/// the list when it is not there. A cache with no record of pushes first stamps every
/// version it holds, once.
fn apply_push(cached: &mut CachedBadgesData, set_id: &str, version: HelixBadgeVersion, now: u64) -> Push {
    let map_is_new = cached.pushed.is_none();
    let mut pushed = cached.pushed.take().unwrap_or_else(|| pairs_at(&cached.badges, now));
    let moved = pushed.insert(pair_key(set_id, &version.id), now) != Some(now);
    cached.pushed = Some(pushed);
    let present = cached
        .badges
        .data
        .iter()
        .any(|s| s.set_id == set_id && s.versions.iter().any(|v| v.id == version.id));
    if present {
        return if moved || map_is_new {
            Push::Restamped
        } else {
            Push::Unchanged
        };
    }
    match cached.badges.data.iter_mut().find(|s| s.set_id == set_id) {
        Some(set) => set.versions.push(version),
        None => cached.badges.data.push(HelixBadgeSet {
            set_id: set_id.to_string(),
            versions: vec![version],
        }),
    }
    Push::Added
}

/// Proves the drop feed still holds the pairs of a batch, before the classifier skips
/// any unchanged re-push of them. One locked write, and only when a stamp moved.
/// Returns whether it wrote.
pub async fn restamp_pushed_pairs(pairs: &[(String, String)], now: u64) -> bool {
    let _guard = GLOBAL_BADGES_LOCK.lock().await;
    let Some(previous) = previous_cache() else {
        return false;
    };
    let Some(next) = restamped(previous, pairs, now) else {
        return false;
    };
    let Ok(json_value) = serde_json::to_value(&next) else {
        return false;
    };
    cache_item(CacheType::Badge, "global_badges".to_string(), json_value, "twitch".to_string(), 7)
        .await
        .is_ok()
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
    let cache_key = "global_badges";

    // The entry as stored, expired or not: a stale list is still the only holder of
    // the versions the drop feed pushed.
    let previous = previous_cache();
    match fetch_decision(previous.as_ref(), get_current_timestamp()) {
        FetchDecision::UseCached => {
            if let Some(cached_data) = previous.as_ref() {
                debug!("[Badges] Using cached badges (cached_at {})", cached_data.cached_at);
                note_row_view_badges(None, &cached_data.badges);
                return Ok(cached_data.badges.clone());
            }
        }
        FetchDecision::Refresh => {
            debug!("[Badges] No usable cached badges (none, 7 days old or more, or stamped in the future), fetching...");
        }
    }

    // Fetch from API
    let before = previous_gallery_pairs();
    let n = FETCH_STARTS.fetch_add(1, Ordering::SeqCst) + 1;
    let mut badges = fetch_badges_from_api(client_id, token).await?;
    let pushed = merge_cache_only(&mut badges, previous.as_ref(), get_current_timestamp());

    // Cache the result for 7 days
    let cached_data = CachedBadgesData {
        badges: badges.clone(),
        cached_at: get_current_timestamp(),
        pushed: Some(pushed),
    };

    let wrote = match serde_json::to_value(&cached_data) {
        Ok(json_value) => cache_item(CacheType::Badge, cache_key.to_string(), json_value, "twitch".to_string(), 7).await.is_ok(),
        Err(_) => false,
    };
    WRITTEN_START.store(written_start_after(WRITTEN_START.load(Ordering::SeqCst), n, wrote), Ordering::SeqCst);
    if wrote {
        debug!("[Badges] Cached global badges for 7 days");
    }

    note_arrivals(before.as_ref(), &badges);
    note_row_view_badges(None, &badges);
    Ok(badges)
}

/// The global badge list read fresh from Twitch whatever the cache's age, for
/// a deliberate refresh: the cached list is kept up to 7 days, so new drop
/// badges could stay off a phone's wall for a week. Versions only the cache
/// has (drops merged from the feed before Twitch lists them) are kept while
/// the feed's last push of them is recent. A call that waited on the lock while
/// another writer fetched and stored Twitch's list returns that list.
pub async fn refresh_global_badges() -> Result<HelixBadgesResponse, String> {
    // Read before anything waits, so a fetch that starts while this call waits for the
    // token or the lock counts as one made after the ask.
    let seen = FETCH_STARTS.load(Ordering::SeqCst);
    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;
    let _guard = GLOBAL_BADGES_LOCK.lock().await;
    let cache_key = "global_badges";
    let previous = previous_cache();
    if coalesce(seen, WRITTEN_START.load(Ordering::SeqCst)) {
        if let Some(stored) = previous.as_ref() {
            return Ok(stored.badges.clone());
        }
    }

    let before = previous_gallery_pairs();
    let n = FETCH_STARTS.fetch_add(1, Ordering::SeqCst) + 1;
    let mut badges = fetch_badges_from_api(client_id, token).await?;
    let pushed = merge_cache_only(&mut badges, previous.as_ref(), get_current_timestamp());

    let cached_data = CachedBadgesData {
        badges: badges.clone(),
        cached_at: get_current_timestamp(),
        pushed: Some(pushed),
    };
    let wrote = match serde_json::to_value(&cached_data) {
        Ok(json_value) => cache_item(CacheType::Badge, cache_key.to_string(), json_value, "twitch".to_string(), 7).await.is_ok(),
        Err(_) => false,
    };
    WRITTEN_START.store(written_start_after(WRITTEN_START.load(Ordering::SeqCst), n, wrote), Ordering::SeqCst);
    note_arrivals(before.as_ref(), &badges);
    note_row_view_badges(None, &badges);
    Ok(badges)
}

/// Hands badge art to the native row view, which stamps it onto rows (the web
/// renderers resolve it themselves from what these commands return).
fn note_row_view_badges(room_id: Option<&str>, badges: &HelixBadgesResponse) {
    crate::services::chat_row_view::note_twitch_badges(
        room_id,
        badges.data.iter().flat_map(|set| {
            set.versions
                .iter()
                .map(move |v| (set.set_id.as_str(), v.id.as_str(), v.image_url_2x.as_str(), v.title.as_str()))
        }),
    );
}

/// Seconds since the global badge list was read from Twitch; None with none
/// cached.
pub async fn global_badges_age_secs() -> Option<u64> {
    let cached = get_cached_item(CacheType::Badge, "global_badges").await.ok()??;
    let data = serde_json::from_value::<CachedBadgesData>(cached.data).ok()?;
    Some(get_current_timestamp().saturating_sub(data.cached_at))
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
                note_row_view_badges(None, &cached_data.badges);
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

/// Force refresh global badges from Twitch API (bypasses cache). A call that waited on
/// the lock while another writer fetched and stored Twitch's list returns that list.
#[tauri::command]
pub async fn force_refresh_global_badges() -> Result<HelixBadgesResponse, String> {
    // Read before the lock, so a fetch that starts while this call waits counts as one
    // made after the ask.
    let seen = FETCH_STARTS.load(Ordering::SeqCst);
    // Serialize against the socket-drop merge (see fetch_global_badges).
    let _guard = GLOBAL_BADGES_LOCK.lock().await;
    let previous = previous_cache();
    if coalesce(seen, WRITTEN_START.load(Ordering::SeqCst)) {
        if let Some(stored) = previous.as_ref() {
            return Ok(stored.badges.clone());
        }
    }
    debug!("[Badges] Force refreshing global badges from Twitch API...");

    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    // Fetch directly from API (bypassing cache check)
    let before = previous_gallery_pairs();
    let n = FETCH_STARTS.fetch_add(1, Ordering::SeqCst) + 1;
    let mut badges = fetch_badges_from_api(client_id, token).await?;
    let pushed = merge_cache_only(&mut badges, previous.as_ref(), get_current_timestamp());

    // Cache the result for 7 days
    let cached_data = CachedBadgesData {
        badges: badges.clone(),
        cached_at: get_current_timestamp(),
        pushed: Some(pushed),
    };

    let cache_key = "global_badges";
    let wrote = match serde_json::to_value(&cached_data) {
        Ok(json_value) => cache_item(CacheType::Badge, cache_key.to_string(), json_value, "twitch".to_string(), 7).await.is_ok(),
        Err(_) => false,
    };
    WRITTEN_START.store(written_start_after(WRITTEN_START.load(Ordering::SeqCst), n, wrote), Ordering::SeqCst);
    if wrote {
        debug!("[Badges] Force refreshed and cached {} badge sets", badges.data.len());
    }

    note_arrivals(before.as_ref(), &badges);
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
/// Idempotent for the list (an existing version is never added twice); every push
/// records its time in `pushed`, so a repeat push of a version already there moves
/// its time up. Preserves `cached_at` so the Helix refresh cadence is unchanged.
/// Returns Ok(true) when the gallery gained a version.
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

    // The entry as stored, expired or not, like every other writer of this list.
    let Some(mut cached_data) = previous_cache() else {
        return Ok(false);
    };

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

    let outcome = apply_push(&mut cached_data, &badge.badge_set_id, version, get_current_timestamp());
    if outcome == Push::Unchanged {
        return Ok(false);
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
    Ok(outcome == Push::Added)
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
    // The channel's FFZ room (its own moderator / VIP art) is read alongside
    // Helix while the viewer has the art on. Only a room never read before is
    // waited on, and only briefly: the chat's history waits on this set. Art
    // that lands later is announced (`ffz_room_badges::CHANGED_EVENT`) and every
    // window reads the set again; a stale room refreshes in the background.
    let role_art = crate::services::ffz_room_badges::enabled();
    let first_read = if role_art {
        let unread = !crate::services::ffz_room_badges::has_entry(&channel_id);
        crate::services::ffz_room_badges::refresh(&channel_id).filter(|_| unread)
    } else {
        None
    };
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

    let mut badges = response
        .json::<HelixBadgesResponse>()
        .await
        .map_err(|e| format!("Failed to parse channel badges: {}", e))?;

    if let Some(read) = first_read {
        let _ = tokio::time::timeout(crate::services::ffz_room_badges::FIRST_READ_WAIT, read).await;
    }
    crate::services::ffz_room_badges::apply_if_enabled(&channel_id, &mut badges, role_art);

    note_row_view_badges(Some(&channel_id), &badges);
    Ok(badges)
}

/// A channel's FrankerFaceZ moderator and VIP art at 72 px (Twitch's 4x),
/// finished the way chat draws it, whatever the viewer's own chat setting.
/// The stream overlay has its own switch for this art, so its preview asks
/// here instead of reading the chat's badge set.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct FfzRoomRoleArtDto {
    pub moderator: Option<String>,
    pub vip: Option<String>,
}

impl FfzRoomRoleArtDto {
    fn from_art(art: &crate::services::ffz_room_badges::RoomRoleArt) -> Self {
        let x4 = |role: &Option<crate::services::ffz_room_badges::RoleArt>| {
            role.as_ref().map(|r| r.x4.clone()).filter(|u| !u.is_empty())
        };
        Self { moderator: x4(&art.moderator), vip: x4(&art.vip) }
    }
}

/// How long the overlay preview waits on a room it has not read yet.
const ROLE_ART_READ_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[tauri::command]
pub async fn get_ffz_room_role_art(channel_id: String) -> Result<Option<FfzRoomRoleArtDto>, String> {
    if let Some(read) = crate::services::ffz_room_badges::refresh(&channel_id) {
        let _ = tokio::time::timeout(ROLE_ART_READ_WAIT, read).await;
    }
    Ok(crate::services::ffz_room_badges::cached(&channel_id)
        .map(|art| FfzRoomRoleArtDto::from_art(&art))
        .filter(|dto| dto.moderator.is_some() || dto.vip.is_some()))
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

#[cfg(test)]
mod arrival_tests {
    use super::*;
    use crate::services::universal_cache_service::CacheMetadata;

    fn gallery(sets: &[(&str, &[&str])]) -> HelixBadgesResponse {
        HelixBadgesResponse {
            data: sets
                .iter()
                .map(|(s, vs)| HelixBadgeSet {
                    set_id: s.to_string(),
                    versions: vs
                        .iter()
                        .map(|v| HelixBadgeVersion {
                            id: v.to_string(),
                            image_url_1x: String::new(),
                            image_url_2x: String::new(),
                            image_url_4x: String::new(),
                            title: String::new(),
                            description: String::new(),
                            click_action: None,
                            click_url: None,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn a_read_names_the_pairs_its_previous_cache_lacked() {
        let before = gallery_pairs(&gallery(&[("a", &["1"])]));
        let now = gallery_pairs(&gallery(&[("a", &["1", "2"]), ("b", &["1"])]));
        use crate::services::account_automations::badges::new_pairs;
        assert_eq!(new_pairs(Some(&before), &now), vec![("a".to_string(), "2".to_string()), ("b".to_string(), "1".to_string())]);
        assert!(new_pairs(None, &now).is_empty(), "no previous cache: nothing is new");
    }

    #[test]
    fn every_twitch_list_writer_reads_the_cache_before_it_writes() {
        let src = include_str!("badges.rs").replace("\r\n", "\n");
        for sig in ["pub async fn fetch_global_badges()", "pub async fn refresh_global_badges()", "pub async fn force_refresh_global_badges()"] {
            let start = src.find(sig).unwrap_or_else(|| panic!("{sig}"));
            let body = &src[start..start + src[start..].find("\n}\n").unwrap()];
            let read = body.find("previous_gallery_pairs()").unwrap_or_else(|| panic!("{sig} reads the old cache"));
            assert!(read < body.find("fetch_badges_from_api(").unwrap(), "{sig}");
            assert!(body.contains("note_arrivals(before.as_ref(), &badges)"), "{sig}");
            assert!(body.contains("merge_cache_only("), "{sig} keeps the versions only the cache holds");
        }
    }

    const NOW: u64 = 2_000_000_000;
    const DAY: u64 = 86_400;

    const WRITERS: [&str; 3] = [
        "pub async fn fetch_global_badges()",
        "pub async fn refresh_global_badges()",
        "pub async fn force_refresh_global_badges()",
    ];

    const FOUR_BODIES: [&str; 4] = [
        "pub async fn fetch_global_badges()",
        "pub async fn refresh_global_badges()",
        "pub async fn force_refresh_global_badges()",
        "pub async fn merge_pushed_badge_into_global_cache(",
    ];

    fn badges_src() -> String {
        include_str!("badges.rs").replace("\r\n", "\n")
    }

    /// The text of one function, from its signature to the closing brace at column 0. A
    /// signature that sits in the test module is refused, so a pin never reads a test body.
    fn fn_text(src: &str, sig: &str) -> String {
        let start = src.find(sig).unwrap_or_else(|| panic!("`{sig}` is missing"));
        if let Some(tests) = src.find("mod arrival_tests") {
            assert!(start < tests, "`{sig}` must sit above the test module");
        }
        let len = src[start..]
            .find("\n}\n")
            .unwrap_or_else(|| panic!("`{sig}` has no closing brace"));
        src[start..start + len].to_string()
    }

    /// Every needle appears, each after the one before it.
    fn in_order(body: &str, needles: &[&str]) {
        let mut last = 0;
        for needle in needles {
            let found = body[last..]
                .find(needle)
                .unwrap_or_else(|| panic!("`{needle}` missing or out of order (after byte {last}):\n{body}"));
            last += found + needle.len();
        }
    }

    fn cached(sets: &[(&str, &[&str])], pushed: Option<&[(&str, u64)]>, cached_at: u64) -> CachedBadgesData {
        CachedBadgesData {
            badges: gallery(sets),
            cached_at,
            pushed: pushed.map(|pairs| pairs.iter().map(|(key, at)| (key.to_string(), *at)).collect()),
        }
    }

    /// "set/version" of every version in the list, sorted.
    fn listed(badges: &HelixBadgesResponse) -> Vec<String> {
        let mut out: Vec<String> = badges
            .data
            .iter()
            .flat_map(|s| s.versions.iter().map(move |v| format!("{}/{}", s.set_id, v.id)))
            .collect();
        out.sort();
        out
    }

    fn map(pairs: &[(&str, u64)]) -> PushedPairs {
        pairs.iter().map(|(key, at)| (key.to_string(), *at)).collect()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn batch(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(s, v)| (s.to_string(), v.to_string())).collect()
    }

    fn badge_version(id: &str) -> HelixBadgeVersion {
        gallery(&[("x", &[id])]).data.remove(0).versions.remove(0)
    }

    fn entry(data: serde_json::Value, timestamp: u64) -> UniversalCacheEntry {
        UniversalCacheEntry {
            id: "global_badges".to_string(),
            cache_type: CacheType::Badge,
            data,
            metadata: CacheMetadata {
                timestamp,
                expiry_days: 7,
                source: "twitch".to_string(),
                version: 1,
            },
            position: None,
        }
    }

    // ---- the pure merge ----

    // Fails without the merge: a version Twitch does not list is gone from the list and the map.
    #[test]
    fn a_version_pushed_three_days_ago_survives_a_merge_and_stays_in_the_map() {
        let previous = cached(&[("a", &["1", "2"])], Some(&[("a/2", NOW - 3 * DAY)]), NOW - DAY);
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1", "a/2"]));
        assert_eq!(kept, map(&[("a/2", NOW - 3 * DAY)]));
    }

    // Fails without the 14-day limit: a push from 15 days ago would be kept forever.
    #[test]
    fn a_version_pushed_fifteen_days_ago_is_dropped_from_the_list_and_the_map() {
        let previous = cached(&[("a", &["1", "2"])], Some(&[("a/2", NOW - 15 * DAY)]), NOW - DAY);
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1"]));
        assert!(kept.is_empty());
    }

    // Fails on an off-by-one: exactly 14 days is kept, one second older is dropped.
    #[test]
    fn the_fourteen_day_limit_is_inclusive_to_the_second() {
        let previous = cached(
            &[("a", &["1", "2", "3"])],
            Some(&[("a/2", NOW - 14 * DAY), ("a/3", NOW - 14 * DAY - 1)]),
            NOW - DAY,
        );
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1", "a/2"]));
        assert_eq!(kept, map(&[("a/2", NOW - 14 * DAY)]));
    }

    // Fails with a plain subtraction (overflow panic) or without min(t, now) (a future stamp is written back).
    #[test]
    fn a_future_stamp_survives_is_written_back_as_now_and_never_overflows() {
        let previous = cached(
            &[("a", &["1", "2", "3"])],
            Some(&[("a/2", NOW + 3_600), ("a/3", u64::MAX)]),
            NOW,
        );
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1", "a/2", "a/3"]));
        assert_eq!(kept, map(&[("a/2", NOW), ("a/3", NOW)]));
    }

    // Fails if every cache-only version is kept: one Twitch removed has no push record and must leave.
    #[test]
    fn a_cache_only_version_with_no_push_record_is_dropped() {
        let previous = cached(&[("a", &["1", "2"])], Some(&[]), NOW - DAY);
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1"]));
        assert!(kept.is_empty());
    }

    // Fails if a pair Twitch now lists stays in the map: it would be re-added the day Twitch removes it.
    #[test]
    fn a_pushed_pair_twitch_now_lists_leaves_the_map() {
        let previous = cached(&[("a", &["1", "2"])], Some(&[("a/1", NOW - DAY), ("a/2", NOW - DAY)]), NOW - DAY);
        let mut fetched = gallery(&[("a", &["1", "2"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1", "a/2"]));
        assert!(kept.is_empty());
    }

    // Fails if a dropped set comes back whole (every version) or comes back empty.
    #[test]
    fn a_set_twitch_no_longer_lists_returns_with_only_its_kept_versions() {
        let previous = cached(
            &[("a", &["1"]), ("b", &["1", "2"])],
            Some(&[("b/2", NOW - DAY)]),
            NOW - DAY,
        );
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1", "b/2"]));
        assert_eq!(kept, map(&[("b/2", NOW - DAY)]));

        let none_kept = cached(&[("a", &["1"]), ("b", &["1", "2"])], Some(&[]), NOW - DAY);
        let mut fetched = gallery(&[("a", &["1"])]);
        merge_cache_only(&mut fetched, Some(&none_kept), NOW);
        assert_eq!(fetched.data.len(), 1, "a set with no kept version is left out");
    }

    // Fails without the first-write rule: a cache from before this change would lose every cache-only version.
    #[test]
    fn a_cache_with_no_push_record_keeps_every_cache_only_version_once_and_stamps_them_now() {
        let previous = cached(&[("a", &["1", "2"]), ("b", &["1"])], None, NOW - 2 * DAY);
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1", "a/2", "b/1"]));
        assert_eq!(kept, map(&[("a/2", NOW), ("b/1", NOW)]));
    }

    #[test]
    fn with_no_previous_cache_the_fetched_list_is_returned_as_it_came() {
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, None, NOW);
        assert_eq!(listed(&fetched), strings(&["a/1"]));
        assert!(kept.is_empty());
    }

    // Fails if the 7-day path writes Twitch's list as it came: the 3-day push must survive, the 20-day one not.
    #[test]
    fn the_seven_day_refresh_keeps_a_recent_push_and_drops_an_old_one() {
        let previous = cached(
            &[("a", &["1", "2", "3"])],
            Some(&[("a/2", NOW - 3 * DAY), ("a/3", NOW - 20 * DAY)]),
            NOW - 8 * DAY,
        );
        assert_eq!(fetch_decision(Some(&previous), NOW), FetchDecision::Refresh);
        let mut fetched = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut fetched, Some(&previous), NOW);
        assert_eq!(listed(&fetched), strings(&["a/1", "a/2"]));
        assert_eq!(kept, map(&[("a/2", NOW - 3 * DAY)]));
    }

    // ---- the seven-day decision and the cache read ----

    // Fails if a future stamp reads as fresh (a stepped clock) or the boundary moves.
    #[test]
    fn the_seven_day_decision_treats_a_future_or_missing_cache_as_stale() {
        let at = |cached_at: u64| cached(&[], Some(&[]), cached_at);
        assert_eq!(fetch_decision(Some(&at(NOW - DAY)), NOW), FetchDecision::UseCached);
        assert_eq!(fetch_decision(Some(&at(NOW)), NOW), FetchDecision::UseCached);
        assert_eq!(fetch_decision(Some(&at(NOW - 7 * DAY + 1)), NOW), FetchDecision::UseCached);
        assert_eq!(fetch_decision(Some(&at(NOW - 7 * DAY)), NOW), FetchDecision::Refresh);
        assert_eq!(fetch_decision(Some(&at(NOW - 30 * DAY)), NOW), FetchDecision::Refresh);
        assert_eq!(fetch_decision(Some(&at(NOW + 1)), NOW), FetchDecision::Refresh);
        assert_eq!(fetch_decision(None, NOW), FetchDecision::Refresh);
    }

    // Fails if previous_from filters by expiry: the entry below expired long ago and its data must still come back.
    #[test]
    fn the_cache_read_answers_for_an_expired_entry() {
        let data = serde_json::to_value(cached(&[("a", &["1", "2"])], Some(&[("a/2", NOW - DAY)]), 77)).unwrap();
        let read = previous_from(Some(entry(data, 1))).expect("an expired entry still answers");
        assert_eq!(read.cached_at, 77);
        assert_eq!(listed(&read.badges), strings(&["a/1", "a/2"]));
        assert_eq!(read.pushed, Some(map(&[("a/2", NOW - DAY)])));
    }

    // Fails if a cache written before the field existed stops parsing, or an unreadable entry panics.
    #[test]
    fn the_cache_read_defaults_pushed_and_refuses_unreadable_data() {
        let old = serde_json::json!({ "badges": { "data": [] }, "cached_at": 5 });
        let read = previous_from(Some(entry(old, NOW))).expect("a cache from before the field parses");
        assert_eq!(read.pushed, None);
        assert!(previous_from(Some(entry(serde_json::json!("not a cache"), NOW))).is_none());
        assert!(previous_from(None).is_none());
    }

    // ---- the counters ----

    // Fails if a failed write moves the start number: a refresh queued behind it would skip its own fetch.
    #[test]
    fn a_failed_write_leaves_the_written_start_alone() {
        assert_eq!(written_start_after(5, 7, true), 7);
        assert_eq!(written_start_after(9, 7, true), 9, "the newest start number is kept");
        assert_eq!(written_start_after(5, 7, false), 5);
        assert!(!coalesce(5, written_start_after(5, 7, false)), "a refresh behind a failed write fetches for itself");
    }

    // Fails with a "written since I looked" rule: the second case would reuse a list that may be older than the ask.
    #[test]
    fn a_refresh_reuses_only_a_list_fetched_after_it_asked() {
        // Asked when 3 fetches had started; fetch 4 started later and wrote.
        assert!(coalesce(3, written_start_after(3, 4, true)));
        // Asked while fetch 4 was already in flight; its write is not newer than the ask.
        assert!(!coalesce(4, written_start_after(0, 4, true)));
        // Nothing written since.
        assert!(!coalesce(4, 0));
    }

    // ---- the pushed record ----

    // Fails if a first push is not recorded with its time, or a repeat push does not move the time up.
    #[test]
    fn a_first_push_records_its_pair_and_a_repeat_push_moves_the_time_up() {
        let mut data = cached(&[("a", &["1"])], Some(&[]), NOW - 2 * DAY);
        assert_eq!(apply_push(&mut data, "a", badge_version("2"), NOW), Push::Added);
        assert_eq!(listed(&data.badges), strings(&["a/1", "a/2"]));
        assert_eq!(data.pushed, Some(map(&[("a/2", NOW)])));
        assert_eq!(data.cached_at, NOW - 2 * DAY, "a push never makes the list look freshly read");

        assert_eq!(apply_push(&mut data, "a", badge_version("2"), NOW + 100), Push::Restamped);
        assert_eq!(listed(&data.badges), strings(&["a/1", "a/2"]), "the repeat adds nothing");
        assert_eq!(data.pushed, Some(map(&[("a/2", NOW + 100)])));

        assert_eq!(apply_push(&mut data, "a", badge_version("2"), NOW + 100), Push::Unchanged);
    }

    // Fails if the first push into a cache with no record skips the seeding: its versions would age out at once.
    #[test]
    fn the_first_push_into_a_cache_with_no_record_stamps_every_version_it_holds() {
        let mut data = cached(&[("a", &["1"])], None, NOW - 2 * DAY);
        assert_eq!(apply_push(&mut data, "b", badge_version("1"), NOW), Push::Added);
        assert_eq!(listed(&data.badges), strings(&["a/1", "b/1"]));
        assert_eq!(data.pushed, Some(map(&[("a/1", NOW), ("b/1", NOW)])));

        let mut other = cached(&[("a", &["1"])], None, NOW - 2 * DAY);
        assert_eq!(apply_push(&mut other, "a", badge_version("1"), NOW), Push::Restamped, "the new record is written");
        assert_eq!(other.pushed, Some(map(&[("a/1", NOW)])));
    }

    // Fails if restamp moves a fresh pair, a pair outside the batch, or invents a pair the map lacks.
    #[test]
    fn restamp_moves_only_held_pairs_that_are_a_day_old_or_more() {
        let mut pushed = map(&[("a/1", NOW - DAY), ("a/2", NOW - 3_600), ("a/3", NOW - 10 * DAY)]);
        let moved = restamp(&mut pushed, &batch(&[("a", "1"), ("a", "2"), ("b", "9")]), NOW);
        assert!(moved);
        assert_eq!(pushed, map(&[("a/1", NOW), ("a/2", NOW - 3_600), ("a/3", NOW - 10 * DAY)]));
        assert!(!restamp(&mut pushed, &batch(&[("a", "1"), ("a", "2")]), NOW), "an hour old or now is left alone");
        let mut edge = map(&[("a/1", NOW - DAY + 1)]);
        assert!(!restamp(&mut edge, &batch(&[("a", "1")]), NOW), "one second short of a day is left alone");
    }

    // Fails if the write-back takes a new cached_at, or writes when nothing moved.
    #[test]
    fn a_restamp_keeps_the_cached_at_it_read_and_writes_only_when_something_changed() {
        let previous = cached(&[("a", &["1", "2"])], Some(&[("a/2", NOW - 2 * DAY)]), 123);
        let next = restamped(previous, &batch(&[("a", "2")]), NOW).expect("a stamp moved");
        assert_eq!(next.cached_at, 123);
        assert_eq!(next.pushed, Some(map(&[("a/2", NOW)])));

        let fresh = cached(&[("a", &["2"])], Some(&[("a/2", NOW - 3_600)]), 123);
        assert!(restamped(fresh, &batch(&[("a", "2")]), NOW).is_none());

        let old = cached(&[("a", &["1", "2"])], None, 123);
        let next = restamped(old, &batch(&[]), NOW).expect("a cache with no record gets one");
        assert_eq!(next.cached_at, 123);
        assert_eq!(next.pushed, Some(map(&[("a/1", NOW), ("a/2", NOW)])));
    }

    // Fails without the re-stamp: the pair pushed 20 days ago and re-pushed unchanged yesterday would age out today.
    #[test]
    fn an_unchanged_repush_keeps_a_pair_alive_through_the_next_refresh() {
        let stored = cached(&[("a", &["1", "2"])], Some(&[("a/2", NOW - 20 * DAY)]), NOW - 3 * DAY);

        let mut without = gallery(&[("a", &["1"])]);
        merge_cache_only(&mut without, Some(&stored), NOW);
        assert_eq!(listed(&without), strings(&["a/1"]), "control: without the re-stamp the pair is past 14 days");

        let yesterday = NOW - DAY;
        let restamped_cache = restamped(stored, &batch(&[("a", "2")]), yesterday).expect("the 19-day-old stamp moves");
        let mut with = gallery(&[("a", &["1"])]);
        let kept = merge_cache_only(&mut with, Some(&restamped_cache), NOW);
        assert_eq!(listed(&with), strings(&["a/1", "a/2"]));
        assert_eq!(kept, map(&[("a/2", NOW - DAY)]));
    }

    // ---- source pins (bodies are read by signature and must sit above the test module) ----

    #[test]
    fn a_signature_inside_the_test_module_is_refused() {
        let refused = std::panic::catch_unwind(|| {
            fn_text(&badges_src(), "fn a_signature_inside_the_test_module_is_refused()");
        });
        assert!(refused.is_err(), "fn_text must never read a test body");
    }

    // Fails if any of the four bodies gains a Rust bound; the file's two room-read timeouts are unrelated and stay.
    #[test]
    fn the_badge_refresh_has_no_rust_bound() {
        let src = badges_src();
        // Spelled in pieces so this test's own text never holds the needle.
        let timeout = concat!("tokio::time::", "timeout");
        assert!(
            fn_text(&src, "pub async fn fetch_channel_badges(").contains(timeout),
            "positive control: the needle matches real code (the room read keeps its own bound)"
        );
        let mut scanned = 0;
        for sig in [
            "pub async fn fetch_global_badges()",
            "pub async fn refresh_global_badges()",
            "pub async fn force_refresh_global_badges()",
            "fn merge_cache_only(",
        ] {
            let body = fn_text(&src, sig);
            assert!(body.lines().count() > 5, "{sig} was read");
            assert!(!body.contains(timeout), "{sig} has no Rust bound; the shared client bounds each fetch");
            scanned += 1;
        }
        assert_eq!(scanned, 4);
        let production = &src[..src.find("mod arrival_tests").unwrap()];
        assert!(!production.contains(concat!("locked_badge_", "fetch")));
    }

    // Fails if a writer reads the cache through the expiry-aware read, which hides an expired entry.
    #[test]
    fn the_four_cache_writers_read_through_previous_cache_only() {
        let src = badges_src();
        let hidden = concat!("get_cached_", "item(");
        for sig in FOUR_BODIES {
            let body = fn_text(&src, sig);
            assert!(body.contains("previous_cache()"), "{sig}");
            assert!(!body.contains(hidden), "{sig}");
        }
        assert!(
            fn_text(&src, "pub async fn global_badges_age_secs()").contains(hidden),
            "positive control: the plain readers keep the expiry-aware read"
        );
    }

    // Fails if the one cache read becomes anything but the pure read of the single entry.
    #[test]
    fn previous_cache_is_the_pure_read_of_the_single_entry() {
        let body = fn_text(&badges_src(), "fn previous_cache() -> Option<CachedBadgesData> {");
        let inner = body.split_once("{\n").unwrap().1.trim();
        assert_eq!(
            inner,
            "previous_from(crate::services::universal_cache_service::peek_cached_entry(\"global_badges\").ok().flatten())"
        );
    }

    // Fails if a writer skips the merge, bumps late, counts a failed fetch, or stores through anything but the seam.
    #[test]
    fn each_writer_counts_its_fetch_and_stores_the_start_only_after_a_good_write() {
        let src = badges_src();
        for sig in WRITERS {
            let body = fn_text(&src, sig);
            in_order(
                &body,
                &[
                    "GLOBAL_BADGES_LOCK.lock(",
                    "FETCH_STARTS.fetch_add(",
                    "fetch_badges_from_api(",
                    ".await?;",
                    "merge_cache_only(",
                    "let wrote =",
                    "cache_item(",
                    ".await.is_ok()",
                    "WRITTEN_START.store(written_start_after(",
                ],
            );
            assert!(body.contains("let n = FETCH_STARTS.fetch_add(1, Ordering::SeqCst) + 1;"), "{sig}");
            assert!(
                body.contains("WRITTEN_START.store(written_start_after(WRITTEN_START.load(Ordering::SeqCst), n, wrote), Ordering::SeqCst);"),
                "{sig}"
            );
            assert_eq!(body.matches("WRITTEN_START.store(").count(), 1, "{sig}");
            assert_eq!(body.matches("FETCH_STARTS.fetch_add(").count(), 1, "{sig}");
            assert!(!body.contains("let _ = cache_item("), "{sig} keeps the write's result");
        }
        assert!(
            fn_text(&src, "pub async fn prune_invalid_global_badges()").contains("let _ = cache_item("),
            "positive control: the needle matches real code"
        );
        let fetch = fn_text(&src, "pub async fn fetch_global_badges()");
        in_order(&fetch, &["previous_cache()", "fetch_decision(", "previous_gallery_pairs()", "FETCH_STARTS.fetch_add("]);
    }

    // Fails if a refresh reads its ask after the lock, or fetches whatever another writer stored meanwhile.
    #[test]
    fn the_two_refreshers_return_what_a_later_fetch_wrote_instead_of_fetching() {
        let src = badges_src();
        for sig in ["pub async fn refresh_global_badges()", "pub async fn force_refresh_global_badges()"] {
            let body = fn_text(&src, sig);
            in_order(
                &body,
                &[
                    "let seen = FETCH_STARTS.load(Ordering::SeqCst);",
                    "GLOBAL_BADGES_LOCK.lock(",
                    "previous_cache()",
                    "coalesce(seen, WRITTEN_START.load(Ordering::SeqCst))",
                    "fetch_badges_from_api(",
                ],
            );
        }
        let fetch = fn_text(&src, "pub async fn fetch_global_badges()");
        assert!(!fetch.contains("coalesce("), "the seven-day read is not a link read and never coalesces");
    }

    // Fails if the re-stamp write takes a new cached_at, or reads the cache another way.
    #[test]
    fn restamp_pushed_pairs_writes_back_the_stamp_it_read() {
        let body = fn_text(&badges_src(), "pub async fn restamp_pushed_pairs(");
        in_order(&body, &["GLOBAL_BADGES_LOCK.lock(", "previous_cache()", "restamped(", "cache_item("]);
        assert!(!body.contains("get_current_timestamp()"), "no clock in the write");
        assert!(!body.contains("cached_at"), "the cached_at it read goes back through `restamped`");
        assert!(!body.contains(concat!("get_cached_", "item(")));
    }

    // Fails if the app's ingest or the Android mirror classifies a batch before the re-stamp.
    #[test]
    fn the_app_ingest_and_the_android_mirror_restamp_before_they_classify() {
        let sources = [
            ("badge_service.rs", include_str!("badge_service.rs"), "pub(crate) async fn ingest_badges("),
            ("android_notify.rs", include_str!("../android_notify.rs"), "async fn collect_badges()"),
        ];
        for (name, text, sig) in sources {
            let src = text.replace("\r\n", "\n");
            let body = fn_text(&src, sig);
            in_order(&body, &["badge_index::record_feed_dates(", "restamp_pushed_pairs(", "feed::classify("]);
            assert_eq!(body.matches("restamp_pushed_pairs(").count(), 1, "{name}");
        }
    }

    // Fails if a push stops going through the pure seam.
    #[test]
    fn a_pushed_badge_is_recorded_through_apply_push() {
        let body = fn_text(&badges_src(), "pub async fn merge_pushed_badge_into_global_cache(");
        in_order(&body, &["GLOBAL_BADGES_LOCK.lock(", "previous_cache()", "apply_push(", "cache_item("]);
    }

    // Fails if a badge test starts to touch the cache store (the pure seams exist so none has to).
    #[test]
    fn no_badge_test_touches_the_cache_store() {
        let src = badges_src();
        let tests = &src[src.find("mod arrival_tests").unwrap()..];
        let needle = concat!("MANIFEST_", "MEMORY");
        assert!(
            include_str!("../services/universal_cache_service.rs").contains(needle),
            "positive control: the needle matches real code"
        );
        assert!(!tests.contains(needle));
    }
}
