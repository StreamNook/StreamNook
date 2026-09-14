// ProfileCache Service - Unified user profile data aggregation
// Handles 7TV, FFZ, Chatterino, and Homies badge/cosmetic fetching in Rust
// Thread-safe caching with memory management

use anyhow::{Context, Result};
use log::debug;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

// ============================================================================
// DATA MODELS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: String,
    pub username: String,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    pub seventv_cosmetics: SevenTVCosmetics,
    pub third_party_badges: Vec<ThirdPartyBadge>,
    pub last_updated: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVCosmetics {
    pub paints: Vec<SevenTVPaint>,
    pub badges: Vec<SevenTVBadge>,
    pub seventv_user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVPaint {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub data: PaintData,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaintData {
    pub layers: Vec<PaintLayer>,
    pub shadows: Vec<PaintShadow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaintLayer {
    pub id: String,
    pub ty: serde_json::Value, // Complex type with multiple variants
    pub opacity: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaintShadow {
    #[serde(rename = "offsetX")]
    pub offset_x: f32,
    #[serde(rename = "offsetY")]
    pub offset_y: f32,
    pub blur: f32,
    pub color: ColorData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorData {
    pub hex: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SevenTVBadge {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub selected: bool,
    pub local_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThirdPartyBadge {
    pub id: String,
    pub provider: String, // "ffz", "chatterino", "homies"
    pub title: String,
    pub image_url: String,
    pub link: Option<String>,
    pub local_url: Option<String>,
}

// ============================================================================
// GLOBAL BADGE DATABASES (cached in memory)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FFZBadgeDatabase {
    badges: Vec<serde_json::Value>,
    users: HashMap<String, Vec<u32>>,
    timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatterinoBadgeDatabase {
    badges: Vec<serde_json::Value>,
    timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HomiesBadgeDatabase {
    badges: Vec<serde_json::Value>,
    timestamp: u64,
}

// ============================================================================
// PROFILE CACHE SERVICE
// ============================================================================

pub struct ProfileCacheService {
    // User profile cache
    // LRU-bounded: one profile was retained per user card ever opened, with
    // the TTL gating freshness but never evicting.
    profiles: Arc<RwLock<lru::LruCache<String, CachedProfile>>>,

    // Global badge databases (all users)
    ffz_database: Arc<RwLock<Option<FFZBadgeDatabase>>>,
    chatterino_database: Arc<RwLock<Option<ChatterinoBadgeDatabase>>>,
    homies_database: Arc<RwLock<Option<HomiesBadgeDatabase>>>,

    // HTTP client for API requests
    client: Client,

    // Cache durations
    profile_cache_duration: Duration,
    badge_db_cache_duration: Duration,
}

struct CachedProfile {
    profile: UserProfile,
    timestamp: u64,
}

impl ProfileCacheService {
    /// Resident profiles in the LRU (try-read). Diagnostics for the resource line.
    pub fn cache_len(&self) -> Option<usize> {
        self.profiles.try_read().ok().map(|c| c.len())
    }


    pub fn new() -> Self {
        Self {
            profiles: Arc::new(RwLock::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(500).expect("nonzero"),
            ))),
            ffz_database: Arc::new(RwLock::new(None)),
            chatterino_database: Arc::new(RwLock::new(None)),
            homies_database: Arc::new(RwLock::new(None)),
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            profile_cache_duration: Duration::from_secs(5 * 60), // 5 minutes
            badge_db_cache_duration: Duration::from_secs(10 * 60), // 10 minutes
        }
    }

    fn current_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    // ========================================================================
    // PUBLIC API
    // ========================================================================

    /// Get complete user profile (cache-first with background refresh)
    pub async fn get_user_profile(
        &self,
        user_id: String,
        username: String,
        channel_id: Option<String>,
        channel_name: Option<String>,
    ) -> Result<UserProfile> {
        let now = Self::current_timestamp();

        // Check cache first
        {
            let cache = self.profiles.read().await;
            if let Some(cached) = cache.peek(&user_id) {
                if now - cached.timestamp < self.profile_cache_duration.as_secs() {
                    debug!("[ProfileCache] Cache hit for user {}", user_id);
                    return Ok(cached.profile.clone());
                }
            }
        }

        // Cache miss or expired - fetch fresh data
        debug!(
            "[ProfileCache] Cache miss for user {}, fetching fresh data",
            user_id
        );
        self.fetch_and_cache_profile(user_id, username, channel_id, channel_name)
            .await
    }

    /// Refresh user profile in background (non-blocking)
    pub async fn refresh_user_profile(
        &self,
        user_id: String,
        username: String,
        channel_id: Option<String>,
        channel_name: Option<String>,
    ) -> Result<()> {
        debug!("[ProfileCache] Background refresh for user {}", user_id);
        self.fetch_and_cache_profile(user_id, username, channel_id, channel_name)
            .await?;
        Ok(())
    }

    /// Clear all caches (useful for testing or manual refresh)
    pub async fn clear_all_caches(&self) {
        self.profiles.write().await.clear();
        *self.ffz_database.write().await = None;
        *self.chatterino_database.write().await = None;
        *self.homies_database.write().await = None;
        debug!("[ProfileCache] All caches cleared");
    }

    /// Preload global badge databases (call on app startup)
    pub async fn preload_badge_databases(&self) -> Result<()> {
        debug!("[ProfileCache] Preloading badge databases...");

        let (ffz_result, chatterino_result, homies_result) = tokio::join!(
            self.fetch_ffz_database(),
            self.fetch_chatterino_database(),
            self.fetch_homies_database()
        );

        // Log any errors but don't fail
        if let Err(e) = ffz_result {
            debug!("[ProfileCache] Failed to preload FFZ database: {}", e);
        }
        if let Err(e) = chatterino_result {
            debug!(
                "[ProfileCache] Failed to preload Chatterino database: {}",
                e
            );
        }
        if let Err(e) = homies_result {
            debug!("[ProfileCache] Failed to preload Homies database: {}", e);
        }

        debug!("[ProfileCache] Badge databases preloaded");
        Ok(())
    }

    // ========================================================================
    // INTERNAL METHODS - PROFILE FETCHING
    // ========================================================================

    async fn fetch_and_cache_profile(
        &self,
        user_id: String,
        username: String,
        channel_id: Option<String>,
        channel_name: Option<String>,
    ) -> Result<UserProfile> {
        // Fetch all data in parallel
        let (seventv_result, badges_result) = tokio::join!(
            self.fetch_seventv_cosmetics(&user_id),
            self.fetch_all_third_party_badges(&user_id)
        );

        let seventv_cosmetics = seventv_result.unwrap_or_else(|e| {
            debug!("[ProfileCache] Failed to fetch 7TV cosmetics: {}", e);
            SevenTVCosmetics {
                paints: vec![],
                badges: vec![],
                seventv_user_id: None,
            }
        });

        let third_party_badges = badges_result.unwrap_or_else(|e| {
            debug!("[ProfileCache] Failed to fetch third-party badges: {}", e);
            vec![]
        });

        let profile = UserProfile {
            user_id: user_id.clone(),
            username,
            channel_id,
            channel_name,
            seventv_cosmetics,
            third_party_badges,
            last_updated: Self::current_timestamp(),
        };

        // Store in cache
        let cached = CachedProfile {
            profile: profile.clone(),
            timestamp: Self::current_timestamp(),
        };
        self.profiles.write().await.put(user_id, cached);

        Ok(profile)
    }

    // ========================================================================
    // 7TV GRAPHQL FETCHING
    // ========================================================================

    async fn fetch_seventv_cosmetics(&self, twitch_id: &str) -> Result<SevenTVCosmetics> {
        let query = self.build_seventv_query(twitch_id);

        let response = self
            .client
            .post("https://7tv.io/v4/gql")
            .header("Content-Type", "application/json")
            .json(&serde_json::json!({ "query": query }))
            .send()
            .await
            .context("Failed to send 7TV GraphQL request")?;

        if !response.status().is_success() {
            anyhow::bail!("7TV API returned status: {}", response.status());
        }

        let json: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse 7TV response")?;

        self.parse_seventv_response(json)
    }

    fn build_seventv_query(&self, twitch_id: &str) -> String {
        format!(
            r#"{{
                users {{
                    userByConnection(platform: TWITCH, platformId: "{}") {{
                        id
                        style {{
                            activePaint {{ id }}
                            activeBadge {{ id description }}
                        }}
                        inventory {{
                            paints {{
                                to {{
                                    paint {{
                                        id
                                        name
                                        description
                                        data {{
                                            layers {{
                                                id
                                                ty {{
                                                    ... on PaintLayerTypeImage {{
                                                        __typename
                                                        images {{
                                                            url
                                                            mime
                                                            size
                                                            scale
                                                            width
                                                            height
                                                            frameCount
                                                        }}
                                                    }}
                                                    ... on PaintLayerTypeRadialGradient {{
                                                        __typename
                                                        repeating
                                                        shape
                                                        stops {{
                                                            at
                                                            color {{ hex r g b a }}
                                                        }}
                                                    }}
                                                    ... on PaintLayerTypeLinearGradient {{
                                                        __typename
                                                        angle
                                                        repeating
                                                        stops {{
                                                            at
                                                            color {{ hex r g b a }}
                                                        }}
                                                    }}
                                                    ... on PaintLayerTypeSingleColor {{
                                                        __typename
                                                        color {{ hex r g b a }}
                                                    }}
                                                }}
                                                opacity
                                            }}
                                            shadows {{
                                                offsetX
                                                offsetY
                                                blur
                                                color {{ hex r g b a }}
                                            }}
                                        }}
                                    }}
                                }}
                            }}
                            badges {{
                                to {{
                                    badge {{
                                        id
                                        name
                                        description
                                    }}
                                }}
                            }}
                        }}
                    }}
                }}
            }}"#,
            twitch_id
        )
        .replace('\n', "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
    }

    fn parse_seventv_response(&self, json: serde_json::Value) -> Result<SevenTVCosmetics> {
        let user_data = json
            .get("data")
            .and_then(|d| d.get("users"))
            .and_then(|u| u.get("userByConnection"))
            .context("Invalid 7TV response structure")?;

        let seventv_user_id = user_data
            .get("id")
            .and_then(|id| id.as_str())
            .map(|s| s.to_string());

        let active_paint_id = user_data
            .get("style")
            .and_then(|s| s.get("activePaint"))
            .and_then(|p| p.get("id"))
            .and_then(|id| id.as_str());

        let active_badge_id = user_data
            .get("style")
            .and_then(|s| s.get("activeBadge"))
            .and_then(|b| b.get("id"))
            .and_then(|id| id.as_str());

        // Parse paints
        let mut paints = vec![];
        if let Some(paint_array) = user_data
            .get("inventory")
            .and_then(|inv| inv.get("paints"))
            .and_then(|p| p.as_array())
        {
            for item in paint_array {
                if let Some(paint_data) = item.get("to").and_then(|t| t.get("paint")) {
                    if let Ok(mut paint) =
                        serde_json::from_value::<SevenTVPaint>(paint_data.clone())
                    {
                        paint.selected = active_paint_id == Some(paint.id.as_str());
                        paints.push(paint);
                    }
                }
            }
        }

        // Parse badges
        let mut badges = vec![];
        if let Some(badge_array) = user_data
            .get("inventory")
            .and_then(|inv| inv.get("badges"))
            .and_then(|b| b.as_array())
        {
            for item in badge_array {
                if let Some(badge_data) = item.get("to").and_then(|t| t.get("badge")) {
                    if let Ok(mut badge) =
                        serde_json::from_value::<SevenTVBadge>(badge_data.clone())
                    {
                        badge.selected = active_badge_id == Some(badge.id.as_str());
                        badge.local_url = None; // Will be populated by cache service
                        badges.push(badge);
                    }
                }
            }
        }

        Ok(SevenTVCosmetics {
            paints,
            badges,
            seventv_user_id,
        })
    }

    // ========================================================================
    // FFZ BADGE FETCHING
    // ========================================================================

    async fn fetch_ffz_database(&self) -> Result<()> {
        let now = Self::current_timestamp();

        // Check if database is still valid
        {
            let db = self.ffz_database.read().await;
            if let Some(cached) = &*db {
                if now - cached.timestamp < self.badge_db_cache_duration.as_secs() {
                    return Ok(());
                }
            }
        }

        debug!("[ProfileCache] Fetching FFZ badge database");
        let response = self
            .client
            .get("https://api.frankerfacez.com/v1/badges/ids")
            .send()
            .await
            .context("Failed to fetch FFZ badges")?;

        if !response.status().is_success() {
            anyhow::bail!("FFZ API returned status: {}", response.status());
        }

        let json: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse FFZ response")?;

        let badges = json
            .get("badges")
            .and_then(|b| b.as_array())
            .cloned()
            .unwrap_or_default();

        let users: HashMap<String, Vec<u32>> = json
            .get("users")
            .and_then(|u| serde_json::from_value(u.clone()).ok())
            .unwrap_or_default();

        let database = FFZBadgeDatabase {
            badges,
            users,
            timestamp: now,
        };

        *self.ffz_database.write().await = Some(database);
        debug!("[ProfileCache] FFZ badge database cached");
        Ok(())
    }

    async fn get_ffz_badges(&self, user_id: &str) -> Result<Vec<ThirdPartyBadge>> {
        // Ensure database is loaded
        self.fetch_ffz_database().await?;

        let db = self.ffz_database.read().await;
        let database = db.as_ref().context("FFZ database not loaded")?;

        let mut badges = vec![];

        if let Some(badge_ids) = database.users.get(user_id) {
            for badge_id in badge_ids {
                if let Some(badge_info) = database
                    .badges
                    .iter()
                    .find(|b| b.get("id").and_then(|id| id.as_u64()) == Some(*badge_id as u64))
                {
                    let title = badge_info
                        .get("title")
                        .or_else(|| badge_info.get("name"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("FFZ Badge")
                        .to_string();

                    // Get highest resolution image
                    let image_url = badge_info
                        .get("urls")
                        .and_then(|urls| {
                            urls.get("4")
                                .or_else(|| urls.get("2"))
                                .or_else(|| urls.get("1"))
                        })
                        .and_then(|url| url.as_str())
                        .unwrap_or("")
                        .to_string();

                    if !image_url.is_empty() {
                        badges.push(ThirdPartyBadge {
                            id: format!("ffz-{}", badge_id),
                            provider: "ffz".to_string(),
                            title,
                            image_url,
                            link: Some("https://www.frankerfacez.com/badges".to_string()),
                            local_url: None,
                        });
                    }
                }
            }
        }

        Ok(badges)
    }

    // ========================================================================
    // CHATTERINO BADGE FETCHING
    // ========================================================================

    async fn fetch_chatterino_database(&self) -> Result<()> {
        let now = Self::current_timestamp();

        // Check if database is still valid
        {
            let db = self.chatterino_database.read().await;
            if let Some(cached) = &*db {
                if now - cached.timestamp < self.badge_db_cache_duration.as_secs() {
                    return Ok(());
                }
            }
        }

        debug!("[ProfileCache] Fetching Chatterino badge database");
        let response = self
            .client
            .get("https://api.chatterino.com/badges")
            .send()
            .await
            .context("Failed to fetch Chatterino badges")?;

        if !response.status().is_success() {
            anyhow::bail!("Chatterino API returned status: {}", response.status());
        }

        let json: serde_json::Value = response
            .json()
            .await
            .context("Failed to parse Chatterino response")?;

        let badges = json
            .get("badges")
            .and_then(|b| b.as_array())
            .cloned()
            .unwrap_or_default();

        let database = ChatterinoBadgeDatabase {
            badges,
            timestamp: now,
        };

        *self.chatterino_database.write().await = Some(database);
        debug!("[ProfileCache] Chatterino badge database cached");
        Ok(())
    }

    async fn get_chatterino_badges(&self, user_id: &str) -> Result<Vec<ThirdPartyBadge>> {
        // Ensure database is loaded
        self.fetch_chatterino_database().await?;

        let db = self.chatterino_database.read().await;
        let database = db.as_ref().context("Chatterino database not loaded")?;

        let mut badges = vec![];

        for badge in &database.badges {
            if let Some(users) = badge.get("users").and_then(|u| u.as_array()) {
                let has_user = users.iter().any(|u| u.as_str() == Some(user_id));
                if has_user {
                    let title = badge
                        .get("tooltip")
                        .and_then(|t| t.as_str())
                        .unwrap_or("Chatterino Badge")
                        .to_string();

                    let image_url = badge
                        .get("image3")
                        .or_else(|| badge.get("image2"))
                        .or_else(|| badge.get("image1"))
                        .and_then(|url| url.as_str())
                        .unwrap_or("")
                        .to_string();

                    if !image_url.is_empty() {
                        badges.push(ThirdPartyBadge {
                            id: format!("chatterino-{}", title),
                            provider: "chatterino".to_string(),
                            title,
                            image_url,
                            link: Some("https://chatterino.com/".to_string()),
                            local_url: None,
                        });
                    }
                }
            }
        }

        Ok(badges)
    }

    // ========================================================================
    // HOMIES BADGE FETCHING
    // ========================================================================

    async fn fetch_homies_database(&self) -> Result<()> {
        let now = Self::current_timestamp();

        // Check if database is still valid
        {
            let db = self.homies_database.read().await;
            if let Some(cached) = &*db {
                if now - cached.timestamp < self.badge_db_cache_duration.as_secs() {
                    return Ok(());
                }
            }
        }

        debug!("[ProfileCache] Fetching Homies badge database");

        // Fetch both badge sources in parallel
        let (result1, result2) = tokio::join!(
            self.client.get("https://itzalex.github.io/badges").send(),
            self.client.get("https://itzalex.github.io/badges2").send()
        );

        let mut all_badges = vec![];

        // Merge badges from both sources
        if let Ok(response1) = result1 {
            if response1.status().is_success() {
                if let Ok(json) = response1.json::<serde_json::Value>().await {
                    if let Some(badges) = json.get("badges").and_then(|b| b.as_array()) {
                        all_badges.extend(badges.clone());
                    }
                }
            }
        }

        if let Ok(response2) = result2 {
            if response2.status().is_success() {
                if let Ok(json) = response2.json::<serde_json::Value>().await {
                    if let Some(badges) = json.get("badges").and_then(|b| b.as_array()) {
                        // Merge, avoiding duplicates
                        for badge in badges {
                            let tooltip = badge.get("tooltip").and_then(|t| t.as_str());
                            let exists = all_badges
                                .iter()
                                .any(|b| b.get("tooltip").and_then(|t| t.as_str()) == tooltip);

                            if !exists {
                                all_badges.push(badge.clone());
                            } else {
                                // Merge users
                                if let Some(existing) = all_badges
                                    .iter_mut()
                                    .find(|b| b.get("tooltip").and_then(|t| t.as_str()) == tooltip)
                                {
                                    if let Some(users) =
                                        badge.get("users").and_then(|u| u.as_array())
                                    {
                                        if let Some(existing_users) =
                                            existing.get_mut("users").and_then(|u| u.as_array_mut())
                                        {
                                            existing_users.extend(users.clone());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let database = HomiesBadgeDatabase {
            badges: all_badges,
            timestamp: now,
        };

        *self.homies_database.write().await = Some(database);
        debug!("[ProfileCache] Homies badge database cached");
        Ok(())
    }

    async fn get_homies_badges(&self, user_id: &str) -> Result<Vec<ThirdPartyBadge>> {
        // Ensure database is loaded
        self.fetch_homies_database().await?;

        let db = self.homies_database.read().await;
        let database = db.as_ref().context("Homies database not loaded")?;

        let mut badges = vec![];

        for badge in &database.badges {
            if let Some(users) = badge.get("users").and_then(|u| u.as_array()) {
                let has_user = users.iter().any(|u| u.as_str() == Some(user_id));
                if has_user {
                    let title = badge
                        .get("tooltip")
                        .and_then(|t| t.as_str())
                        .unwrap_or("Homies Badge")
                        .to_string();

                    let image_url = badge
                        .get("image3")
                        .or_else(|| badge.get("image2"))
                        .or_else(|| badge.get("image1"))
                        .and_then(|url| url.as_str())
                        .unwrap_or("")
                        .to_string();

                    if !image_url.is_empty() {
                        badges.push(ThirdPartyBadge {
                            id: format!("homies-{}", title),
                            provider: "homies".to_string(),
                            title,
                            image_url,
                            link: Some("https://chatterinohomies.com/".to_string()),
                            local_url: None,
                        });
                    }
                }
            }
        }

        Ok(badges)
    }

    // ========================================================================
    // COMBINED THIRD-PARTY BADGE FETCHING
    // ========================================================================

    async fn fetch_all_third_party_badges(&self, user_id: &str) -> Result<Vec<ThirdPartyBadge>> {
        let (ffz_result, chatterino_result, homies_result) = tokio::join!(
            self.get_ffz_badges(user_id),
            self.get_chatterino_badges(user_id),
            self.get_homies_badges(user_id)
        );

        let mut all_badges = vec![];

        if let Ok(ffz) = ffz_result {
            all_badges.extend(ffz);
        }
        if let Ok(chatterino) = chatterino_result {
            all_badges.extend(chatterino);
        }
        if let Ok(homies) = homies_result {
            all_badges.extend(homies);
        }

        Ok(all_badges)
    }
}

// ============================================================================
// GLOBAL INSTANCE
// ============================================================================

lazy_static::lazy_static! {
    pub static ref PROFILE_CACHE: ProfileCacheService = ProfileCacheService::new();
}
