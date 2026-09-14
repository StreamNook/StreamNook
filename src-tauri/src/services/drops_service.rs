use crate::models::drops::*;
use crate::services::drops_auth_service::DropsAuthService;
use anyhow::Result;
use chrono::{DateTime, Utc};
use log::{debug, error, info, warn};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use tokio::time::Duration;
use uuid::Uuid;

// Use Twitch ANDROID APP client ID for GQL operations (required for drops API access)
// This is what the Twitch web client uses - it works with NO SCOPES
const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");
const CLIENT_URL: &str = "https://www.twitch.tv";

// Persisted-query hash for the Inventory GQL operation (same one the Twitch
// web client sends). Shared by the UI inventory fetch and the monitor's
// progress overlay.
const INVENTORY_QUERY_HASH: &str =
    "d86775d0ef16a63a33ad52e80eaff963b2d5b72fada7c991504a57496e1d8e4b";

// Your app's client ID (for reference - used for other Helix API calls)
const APP_CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");

#[derive(Debug, Deserialize)]
struct GraphQLResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQLError>>,
}

#[derive(Debug, Deserialize)]
struct GraphQLError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct DropCampaignsData {
    #[serde(rename = "currentUser")]
    current_user: Option<CurrentUserDrops>,
}

#[derive(Debug, Deserialize)]
struct CurrentUserDrops {
    #[serde(rename = "dropCampaigns")]
    drop_campaigns: Vec<GraphQLDropCampaign>,
}

#[derive(Debug, Deserialize)]
struct GraphQLDropCampaign {
    id: String,
    name: String,
    game: GameInfo,
    description: String,
    #[serde(rename = "imageURL")]
    image_url: String,
    #[serde(rename = "startAt")]
    start_at: String,
    #[serde(rename = "endAt")]
    end_at: String,
    #[serde(rename = "timeBasedDrops")]
    time_based_drops: Vec<GraphQLTimeBasedDrop>,
}

#[derive(Debug, Deserialize)]
struct GameInfo {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct GraphQLTimeBasedDrop {
    id: String,
    name: String,
    #[serde(rename = "requiredMinutesWatched")]
    required_minutes_watched: i32,
    #[serde(rename = "benefitEdges")]
    benefit_edges: Vec<BenefitEdge>,
    #[serde(rename = "self")]
    self_progress: Option<DropSelfProgress>,
}

#[derive(Debug, Deserialize)]
struct BenefitEdge {
    benefit: Benefit,
}

#[derive(Debug, Deserialize)]
struct Benefit {
    id: String,
    name: String,
    #[serde(rename = "imageAssetURL")]
    image_asset_url: String,
}

#[derive(Debug, Deserialize)]
struct DropSelfProgress {
    #[serde(rename = "currentMinutesWatched")]
    current_minutes_watched: i32,
    #[serde(rename = "isClaimed")]
    is_claimed: bool,
}

// Response structure for ChannelPointsContext persisted query
#[derive(Debug, Deserialize)]
struct ChannelPointsContextData {
    channel: Option<ChannelPointsChannel>,
}

#[derive(Debug, Deserialize)]
struct ChannelPointsChannel {
    id: String,
    #[serde(rename = "self")]
    self_data: Option<ChannelPointsSelf>,
}

#[derive(Debug, Deserialize)]
struct ChannelPointsSelf {
    #[serde(rename = "communityPoints")]
    community_points: Option<CommunityPointsInfo>,
}

#[derive(Debug, Deserialize)]
struct CommunityPointsInfo {
    balance: i32,
    #[serde(rename = "availableClaim")]
    available_claim: Option<AvailableClaimInfo>,
    #[serde(rename = "activeMultipliers")]
    active_multipliers: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
struct AvailableClaimInfo {
    id: String,
    #[serde(rename = "pointsEarnedBaseline")]
    points_earned_baseline: Option<i32>,
    #[serde(rename = "pointsEarnedTotal")]
    points_earned_total: Option<i32>,
}

pub struct DropsService {
    client: Client,
    settings: Arc<RwLock<DropsSettings>>,
    drop_progress: Arc<RwLock<HashMap<String, DropProgress>>>,
    claimed_drops: Arc<RwLock<Vec<ClaimedDrop>>>,
    channel_points_history: Arc<RwLock<Vec<ChannelPointsClaim>>>,
    /// Cumulative channel points the app has auto-claimed, persisted across sessions.
    lifetime_points_collected: Arc<RwLock<i64>>,
    channel_points_balances: Arc<RwLock<HashMap<String, ChannelPointsBalance>>>,
    monitoring_active: Arc<RwLock<bool>>,
    current_channel: Arc<RwLock<Option<(String, String)>>>, // (channel_id, channel_name)
    cached_active_campaigns_count: Arc<RwLock<i32>>, // Cache campaign count to avoid repeated API calls
    cached_campaigns: Arc<RwLock<Option<(Vec<DropCampaign>, DateTime<Utc>)>>>, // Cache campaigns with timestamp
    attempted_claims: Arc<RwLock<std::collections::HashSet<String>>>, // Track drops we've already attempted to claim
    device_id: String,
    session_id: String,
}

/// File (in the app data dir) that persists lifetime drops-automation stats across sessions.
const LIFETIME_STATS_FILE: &str = "drops_lifetime_stats.json";

fn lifetime_stats_path() -> Option<std::path::PathBuf> {
    crate::services::cache_service::get_app_data_dir()
        .ok()
        .map(|dir| dir.join(LIFETIME_STATS_FILE))
}

/// Read the persisted cumulative auto-claimed channel points. Returns 0 if absent or unreadable.
fn load_lifetime_points_collected() -> i64 {
    let path = match lifetime_stats_path() {
        Some(p) => p,
        None => return 0,
    };
    match std::fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str::<serde_json::Value>(&contents)
            .ok()
            // Prefer the current key; fall back to the legacy key so an existing
            // stats file keeps its cumulative total across the rename.
            .and_then(|v| v["points_collected"].as_i64().or_else(|| v["points_mined"].as_i64()))
            .unwrap_or(0),
        Err(_) => 0,
    }
}

/// Persist the cumulative auto-claimed channel points. Best-effort; IO errors are ignored.
fn save_lifetime_points_collected(points: i64) {
    if let Some(path) = lifetime_stats_path() {
        if let Ok(json) =
            serde_json::to_string_pretty(&serde_json::json!({ "points_collected": points }))
        {
            let _ = std::fs::write(&path, json);
        }
    }
}

impl DropsService {
    pub fn new() -> Self {
        Self::new_with_settings(DropsSettings::default())
    }

    /// Create a new DropsService with the given initial settings
    /// Use this to restore persisted settings on app startup
    pub fn new_with_settings(initial_settings: DropsSettings) -> Self {
        // Generate persistent device ID and session ID (like the Twitch web client does)
        let device_id = Uuid::new_v4().to_string().replace("-", "");
        let session_id = Uuid::new_v4().to_string().replace("-", "");

        Self {
            client: crate::services::http::client().clone(),
            settings: Arc::new(RwLock::new(initial_settings)),
            drop_progress: Arc::new(RwLock::new(HashMap::new())),
            claimed_drops: Arc::new(RwLock::new(Vec::new())),
            channel_points_history: Arc::new(RwLock::new(Vec::new())),
            lifetime_points_collected: Arc::new(RwLock::new(load_lifetime_points_collected())),
            channel_points_balances: Arc::new(RwLock::new(HashMap::new())),
            monitoring_active: Arc::new(RwLock::new(false)),
            current_channel: Arc::new(RwLock::new(None)),
            cached_active_campaigns_count: Arc::new(RwLock::new(0)),
            cached_campaigns: Arc::new(RwLock::new(None)),
            attempted_claims: Arc::new(RwLock::new(std::collections::HashSet::new())),
            device_id,
            session_id,
        }
    }

    /// Create headers for GQL requests (mimicking the Twitch web client's auth_state.headers())
    fn create_gql_headers(&self, token: &str) -> HeaderMap {
        Self::gql_headers(token, &self.device_id, &self.session_id)
    }

    /// Builds the Android-client GQL headers from owned values, so the detached
    /// watched-channel monitor task can issue authenticated reads without `&self`.
    fn gql_headers(token: &str, device_id: &str, session_id: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("Client-ID", HeaderValue::from_static(CLIENT_ID));
        headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
        headers.insert("Accept-Language", HeaderValue::from_static("en-US"));
        headers.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("OAuth {}", token)).unwrap(),
        );
        headers.insert("Origin", HeaderValue::from_static(CLIENT_URL));
        headers.insert("Referer", HeaderValue::from_static(CLIENT_URL));
        headers.insert("X-Device-Id", HeaderValue::from_str(device_id).unwrap());
        headers.insert(
            "Client-Session-Id",
            HeaderValue::from_str(session_id).unwrap(),
        );
        headers
    }

    pub async fn get_settings(&self) -> DropsSettings {
        self.settings.read().await.clone()
    }

    pub async fn update_settings(&self, new_settings: DropsSettings) {
        let mut settings = self.settings.write().await;
        *settings = new_settings;
    }

    /// Fetch inventory (in-progress campaigns) using the Inventory GQL operation
    /// This matches the Twitch web client's fetch_inventory() function
    pub async fn fetch_inventory(&self) -> Result<InventoryResponse> {
        debug!("[fetch_inventory] Fetching inventory (in-progress campaigns)...");

        let token = match DropsAuthService::get_token().await {
            Ok(t) => {
                debug!("[fetch_inventory] Got token");
                t
            }
            Err(e) => {
                debug!("[fetch_inventory] Failed to get token: {}", e);
                return Err(e);
            }
        };

        // Use the exact same GQL operation as the Twitch web client
        let response = self.client
            .post("https://gql.twitch.tv/gql")
            .headers(self.create_gql_headers(&token))
            .json(&serde_json::json!({
                "operationName": "Inventory",
                "variables": {
                    "fetchRewardCampaigns": false
                },
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": INVENTORY_QUERY_HASH
                    }
                }
            }))
            .send()
            .await?;

        debug!("[fetch_inventory] Response status: {}", response.status());

        let response_text = response.text().await?;
        let response_json: serde_json::Value = match serde_json::from_str(&response_text) {
            Ok(json) => json,
            Err(e) => {
                debug!("Failed to parse JSON: {}", e);
                return Err(anyhow::anyhow!("Failed to parse response as JSON: {}", e));
            }
        };

        // Check for errors
        if let Some(errors) = response_json.get("errors") {
            debug!("GraphQL errors found: {:?}", errors);
            return Err(anyhow::anyhow!("GraphQL errors: {:?}", errors));
        }

        if response_json["data"].is_null() || response_json["data"]["currentUser"].is_null() {
            return Err(anyhow::anyhow!("Unable to fetch inventory data"));
        }

        // Parse in-progress campaigns
        let inventory = &response_json["data"]["currentUser"]["inventory"];
        let campaigns_array = inventory["dropCampaignsInProgress"]
            .as_array()
            .map(|v| v.to_vec())
            .unwrap_or_else(Vec::new);

        // Parse gameEventDrops for claimed benefits tracking
        let empty_game_events = Vec::new();
        let game_event_drops = inventory["gameEventDrops"]
            .as_array()
            .unwrap_or(&empty_game_events);

        let mut claimed_benefits: std::collections::HashMap<String, DateTime<Utc>> =
            std::collections::HashMap::new();
        for event in game_event_drops {
            if let (Some(id), Some(last_awarded)) =
                (event["id"].as_str(), event["lastAwardedAt"].as_str())
            {
                if let Ok(dt) = DateTime::parse_from_rfc3339(last_awarded) {
                    claimed_benefits.insert(id.to_string(), dt.with_timezone(&Utc));
                }
            }
        }

        debug!("Found {} in-progress campaigns", campaigns_array.len());

        let mut items = Vec::new();
        let mut active_count = 0;
        let mut upcoming_count = 0;
        let mut expired_count = 0;
        let now = Utc::now();

        for campaign_json in &campaigns_array {
            // Parse game info
            let game = &campaign_json["game"];
            if game.is_null() {
                continue;
            }

            let game_id = game["id"].as_str().unwrap_or("").to_string();
            let game_name = game["displayName"]
                .as_str()
                .or_else(|| game["name"].as_str())
                .unwrap_or("")
                .to_string();
            let image_url = game["boxArtURL"].as_str().unwrap_or("").to_string();

            if game_name.is_empty() {
                continue;
            }

            // Parse dates
            let start_at = campaign_json["startAt"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|| Utc::now());

            let end_at = campaign_json["endAt"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|| Utc::now() + chrono::Duration::days(365));

            // Determine status
            let status = if start_at > now {
                upcoming_count += 1;
                CampaignStatus::Upcoming
            } else if end_at < now {
                expired_count += 1;
                CampaignStatus::Expired
            } else {
                active_count += 1;
                CampaignStatus::Active
            };

            // Parse allowed channels (ACL)
            // A non-empty `allow.channels` list is an enforced allowlist unless
            // `isEnabled` is explicitly false. `isEnabled` is often absent on
            // special-event campaigns (e.g. EWC), so default to true to match the
            // DropCampaignDetails parser instead of silently dropping the ACL.
            let mut allowed_channels = Vec::new();
            let mut is_acl_based = false;

            if let Some(allow) = campaign_json["allow"].as_object() {
                if let Some(channels) = allow.get("channels").and_then(|v| v.as_array()) {
                    if !channels.is_empty() {
                        let is_enabled = allow
                            .get("isEnabled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);

                        if is_enabled {
                            is_acl_based = true;
                            for channel in channels {
                                if let (Some(id), Some(name)) =
                                    (channel["id"].as_str(), channel["name"].as_str())
                                {
                                    allowed_channels.push(AllowedChannel {
                                        id: id.to_string(),
                                        name: name.to_string(),
                                    });
                                }
                            }

                            info!(
                                "[Drops/ACL] inventory campaign '{}' game '{}': is_acl_based={}, raw_channels={}, parsed_channels={}, allow.isEnabled={:?}",
                                campaign_json["name"].as_str().unwrap_or("unknown"),
                                campaign_json["game"]["name"].as_str().unwrap_or("?"),
                                is_acl_based,
                                channels.len(),
                                allowed_channels.len(),
                                allow.get("isEnabled"),
                            );

                            // An ACL campaign that parses to zero channels is
                            // unfarmable: the farmer has no candidates and reports
                            // "no channels live" even while streams are up. That
                            // means the entries didn't match `{id, name}` — dump the
                            // first raw one so the actual field shape is visible.
                            if allowed_channels.is_empty() {
                                warn!(
                                    "[Drops/ACL] campaign '{}' is ACL-restricted but parsed 0 of {} channels — unfarmable. First raw entry: {}",
                                    campaign_json["name"].as_str().unwrap_or("unknown"),
                                    channels.len(),
                                    channels.first().map(|c| c.to_string()).unwrap_or_else(|| "none".into()),
                                );
                            }
                        }
                    }
                }
            }

            // Parse time-based drops
            let mut time_based_drops: Vec<TimeBasedDrop> = Vec::new();
            let mut total_drops = 0;
            let mut claimed_drops = 0;
            let mut drops_in_progress = 0;
            let mut total_progress: f32 = 0.0;

            if let Some(drops) = campaign_json["timeBasedDrops"].as_array() {
                total_drops = drops.len() as i32;

                for drop_json in drops {
                    let drop_id = drop_json["id"].as_str().unwrap_or("").to_string();
                    let drop_name = drop_json["name"].as_str().unwrap_or("").to_string();
                    let required_minutes =
                        drop_json["requiredMinutesWatched"].as_i64().unwrap_or(0) as i32;

                    // Parse benefits
                    let mut benefit_edges = Vec::new();
                    if let Some(edges) = drop_json["benefitEdges"].as_array() {
                        for edge in edges {
                            if let Some(benefit) = edge.get("benefit") {
                                benefit_edges.push(DropBenefit {
                                    id: benefit["id"].as_str().unwrap_or("").to_string(),
                                    name: benefit["name"].as_str().unwrap_or("").to_string(),
                                    image_url: benefit["imageAssetURL"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    distribution_type: benefit["distributionType"]
                                        .as_str()
                                        .map(|s| s.to_string()),
                                });
                            }
                        }
                    }

                    // Parse progress
                    let mut progress = None;
                    let mut is_claimed = false;
                    let mut current_minutes = 0;

                    if let Some(self_data) = drop_json.get("self") {
                        current_minutes =
                            self_data["currentMinutesWatched"].as_i64().unwrap_or(0) as i32;
                        is_claimed = self_data["isClaimed"].as_bool().unwrap_or(false);

                        // Parse the dropInstanceID - this is the key for claiming drops!
                        let drop_instance_id =
                            self_data["dropInstanceID"].as_str().map(|s| s.to_string());

                        if drop_instance_id.is_some() {
                            debug!(
                                "Found dropInstanceID for {}: {:?}",
                                drop_id, drop_instance_id
                            );
                        }

                        progress = Some(DropProgress {
                            campaign_id: campaign_json["id"].as_str().unwrap_or("").to_string(),
                            drop_id: drop_id.clone(),
                            current_minutes_watched: current_minutes,
                            required_minutes_watched: required_minutes,
                            is_claimed,
                            last_updated: Utc::now(),
                            drop_instance_id, // Store the dropInstanceID for claiming!
                        });
                    } else {
                        // Check claimed_benefits to determine if claimed
                        // If a benefit was EVER claimed (exists in claimed_benefits),
                        // mark the drop as claimed - this handles badge drops and re-run campaigns
                        for benefit in &benefit_edges {
                            if claimed_benefits.contains_key(&benefit.id) {
                                is_claimed = true;
                                break;
                            }
                        }
                    }

                    if is_claimed {
                        claimed_drops += 1;
                        total_progress += 1.0;
                    } else if current_minutes > 0 {
                        drops_in_progress += 1;
                        if required_minutes > 0 {
                            total_progress +=
                                (current_minutes as f32 / required_minutes as f32).min(1.0);
                        }
                    }

                    time_based_drops.push(TimeBasedDrop {
                        id: drop_id,
                        name: drop_name,
                        required_minutes_watched: required_minutes,
                        benefit_edges,
                        progress,
                        // Drops with 0 required minutes are event-based/badge drops that cannot be auto-collected
                        is_collectible: required_minutes > 0,
                    });
                }
            }

            let progress_percentage = if total_drops > 0 {
                (total_progress / total_drops as f32) * 100.0
            } else {
                0.0
            };

            let is_account_connected = campaign_json["self"]["isAccountConnected"]
                .as_bool()
                .unwrap_or(true);

            let campaign = DropCampaign {
                id: campaign_json["id"].as_str().unwrap_or("").to_string(),
                name: campaign_json["name"].as_str().unwrap_or("").to_string(),
                game_id,
                game_name,
                description: campaign_json["description"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                image_url,
                start_at,
                end_at,
                time_based_drops,
                is_account_connected,
                allowed_channels,
                is_acl_based,
                details_url: None,
                account_link: None,
            };

            items.push(InventoryItem {
                campaign,
                status,
                progress_percentage,
                total_drops,
                claimed_drops,
                drops_in_progress,
            });
        }

        let total_campaigns = items.len() as i32;

        debug!(
            "Inventory summary: {} total, {} active, {} upcoming, {} expired",
            total_campaigns, active_count, upcoming_count, expired_count
        );

        // Parse completed drops from gameEventDrops array
        let mut completed_drops = Vec::new();
        for event in game_event_drops {
            if let (Some(id), Some(name), Some(image_url), Some(last_awarded)) = (
                event["id"].as_str(),
                event["name"].as_str(),
                event["imageURL"].as_str(),
                event["lastAwardedAt"].as_str(),
            ) {
                if let Ok(dt) = DateTime::parse_from_rfc3339(last_awarded) {
                    completed_drops.push(CompletedDrop {
                        id: id.to_string(),
                        name: name.to_string(),
                        image_url: image_url.to_string(),
                        game_name: event["game"]["name"].as_str().map(|s| s.to_string()),
                        is_connected: event["isConnected"].as_bool().unwrap_or(false),
                        required_account_link: event["requiredAccountLink"]
                            .as_str()
                            .map(|s| s.to_string()),
                        last_awarded_at: dt.with_timezone(&Utc),
                        total_count: event["totalCount"].as_i64().unwrap_or(1) as i32,
                    });
                }
            }
        }

        // Sort completed drops by most recent first
        completed_drops.sort_by(|a, b| b.last_awarded_at.cmp(&a.last_awarded_at));

        debug!(
            "Found {} completed drops in user's permanent inventory",
            completed_drops.len()
        );

        Ok(InventoryResponse {
            items,
            total_campaigns,
            active_campaigns: active_count,
            upcoming_campaigns: upcoming_count,
            expired_campaigns: expired_count,
            completed_drops,
        })
    }

    /// Get all active campaigns with smart caching (for UI display)
    /// Uses cached campaigns if available and not stale (5 minute TTL)
    /// Only fetches from API if cache is empty or expired
    pub async fn get_all_active_campaigns_cached(&self) -> Result<Vec<DropCampaign>> {
        const CACHE_TTL_SECONDS: i64 = 300; // 5 minutes

        // Check if we have valid cached data
        {
            let cache = self.cached_campaigns.read().await;
            if let Some((campaigns, cached_at)) = cache.as_ref() {
                let age = Utc::now().signed_duration_since(*cached_at);
                if age.num_seconds() < CACHE_TTL_SECONDS {
                    debug!("Using cached campaigns ({} seconds old)", age.num_seconds());
                    return Ok(campaigns.clone());
                } else {
                    debug!("Campaign cache expired ({} seconds old)", age.num_seconds());
                }
            }
        }

        // Cache miss or expired - fetch from API
        debug!("Fetching fresh campaigns from API");
        let campaigns = self.fetch_all_active_campaigns_from_api().await?;

        // IMPORTANT:
        // Keep the internal drop_progress map in sync with what the UI receives.
        // The UI calls `get_drop_progress` separately from `get_active_drop_campaigns`,
        // so if we don't update the progress map here, the frontend will see 0 minutes
        // watched for every campaign until a automation websocket event happens.
        self.update_campaigns_and_progress(&campaigns).await;

        Ok(campaigns)
    }

    /// Internal method to fetch campaigns from API (no caching)
    /// This should only be called by get_all_active_campaigns_cached or during automation operations
    pub(crate) async fn fetch_all_active_campaigns_from_api(&self) -> Result<Vec<DropCampaign>> {
        Self::fetch_active_campaigns(&self.client, &self.device_id, &self.session_id).await
    }

    /// Overwrite the active-campaign cache without touching the live progress map. Used by the
    /// connection-status refresh, which must not disturb in-flight automation progress the way a
    /// full `update_campaigns_and_progress` would.
    pub async fn prime_campaign_cache(&self, campaigns: &[DropCampaign]) {
        let mut cache = self.cached_campaigns.write().await;
        *cache = Some((campaigns.to_vec(), Utc::now()));
    }

    /// Fetches all active drop campaigns with per-account progress (each drop's
    /// `self.currentMinutesWatched` / `isClaimed`). Takes only owned values so
    /// the watched-channel monitor task can refresh progress without `&self`.
    /// Drop progress on Twitch is account-wide, so this reads back exactly what
    /// the watch heartbeat earned on the on-screen channel.
    pub(crate) async fn fetch_active_campaigns(
        client: &Client,
        device_id: &str,
        session_id: &str,
    ) -> Result<Vec<DropCampaign>> {
        debug!("[fetch_all_active_campaigns_from_api] Starting (no filters)...");

        let token = match DropsAuthService::get_token().await {
            Ok(t) => {
                debug!(
                    "[get_all_active_campaigns] Got token (first 10 chars): {}",
                    &t[..10.min(t.len())]
                );
                t
            }
            Err(e) => {
                debug!("[get_all_active_campaigns] Failed to get token: {}", e);
                return Err(e);
            }
        };

        debug!("Fetching drops campaigns using Android app client ID...");

        // Use a full GraphQL query that includes timeBasedDrops with requiredMinutesWatched
        // The ViewerDropsDashboard persisted query doesn't include these fields
        let query = r#"
        query DropCampaigns {
            currentUser {
                id
                dropCampaigns {
                    id
                    name
                    owner { id name }
                    game {
                        id
                        displayName
                        boxArtURL
                    }
                    status
                    startAt
                    endAt
                    description
                    detailsURL
                    accountLinkURL
                    self {
                        isAccountConnected
                    }
                    allow {
                        isEnabled
                        channels {
                            id
                            name
                        }
                    }
                    timeBasedDrops {
                        id
                        name
                        requiredMinutesWatched
                        benefitEdges {
                            benefit {
                                id
                                name
                                imageAssetURL
                            }
                        }
                        self {
                            currentMinutesWatched
                            isClaimed
                            dropInstanceID
                        }
                    }
                }
            }
        }
        "#;

        let response = client
            .post("https://gql.twitch.tv/gql")
            .headers(Self::gql_headers(&token, device_id, session_id))
            .json(&serde_json::json!({
                "query": query,
                "variables": {}
            }))
            .send()
            .await?;

        debug!("Response status: {}", response.status());

        // Get the raw response text first
        let response_text = response.text().await?;

        // Try to parse it as JSON
        let response_json: serde_json::Value = match serde_json::from_str(&response_text) {
            Ok(json) => json,
            Err(e) => {
                debug!("Failed to parse JSON: {}", e);
                return Err(anyhow::anyhow!("Failed to parse response as JSON: {}", e));
            }
        };

        // Check for authorization errors
        if let Some(error_msg) = response_json.get("error").and_then(|e| e.as_str()) {
            if error_msg == "Unauthorized" {
                return Err(anyhow::anyhow!(
                    "Drops API requires authentication with Twitch web client."
                ));
            }
        }

        if let Some(errors) = response_json.get("errors") {
            debug!("GraphQL errors found: {:?}", errors);
            return Err(anyhow::anyhow!("GraphQL errors: {:?}", errors));
        }

        // Check if data and currentUser exist
        if response_json["data"].is_null() {
            debug!("Response data is null");
            return Err(anyhow::anyhow!(
                "Unable to fetch drops data. This is likely due to client ID mismatch."
            ));
        }

        if response_json["data"]["currentUser"].is_null() {
            debug!("currentUser is null - token/client ID mismatch");
            return Err(anyhow::anyhow!(
                "Authentication mismatch: Token was issued for app client ID but drops API requires web client ID"
            ));
        }

        let mut result = Vec::new();

        // Parse campaigns from DropCampaignDetails response
        // This query returns ALL campaigns with full details including timeBasedDrops
        let campaigns_array = response_json["data"]["currentUser"]["dropCampaigns"]
            .as_array()
            .unwrap_or(&Vec::new())
            .to_vec();

        debug!(
            "Raw campaigns response: {} campaigns found",
            campaigns_array.len()
        );

        if !campaigns_array.is_empty() {
            for campaign_json in &campaigns_array {
                // Check campaign status - accept ACTIVE and UPCOMING campaigns
                let status = campaign_json["status"].as_str().unwrap_or("");

                // Skip only EXPIRED campaigns
                if status == "EXPIRED" {
                    continue;
                }

                // Parse game info - handle null game gracefully
                let game = &campaign_json["game"];
                if game.is_null() {
                    continue;
                }

                let game_id = game["id"].as_str().unwrap_or("").to_string();
                let game_name = game["displayName"]
                    .as_str()
                    .or_else(|| game["name"].as_str())
                    .unwrap_or("")
                    .to_string();

                let image_url = game["boxArtURL"].as_str().unwrap_or("").to_string();

                if game_name.is_empty() {
                    continue;
                }

                // Parse allowed channels (ACL)
                // If allow.channels exists and is not empty, this is an ACL-restricted campaign
                // The isEnabled field may or may not be present - we check channels directly
                let mut allowed_channels = Vec::new();
                let mut is_acl_based = false;

                if let Some(allow) = campaign_json["allow"].as_object() {
                    // Check if channels array exists and is not empty
                    if let Some(channels) = allow.get("channels").and_then(|v| v.as_array()) {
                        if !channels.is_empty() {
                            // If isEnabled exists and is explicitly false, skip ACL
                            // Otherwise (isEnabled is true or not present), use the channels
                            let is_enabled = allow
                                .get("isEnabled")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true); // Default to true if not present

                            if is_enabled {
                                is_acl_based = true;
                                for channel in channels {
                                    if let (Some(id), Some(name)) =
                                        (channel["id"].as_str(), channel["name"].as_str())
                                    {
                                        allowed_channels.push(AllowedChannel {
                                            id: id.to_string(),
                                            name: name.to_string(),
                                        });
                                    }
                                }
                                debug!(
                                    "[Drops/ACL] details campaign '{}' game '{}': is_acl_based={}, raw_channels={}, parsed_channels={}, allow.isEnabled={:?}",
                                    campaign_json["name"].as_str().unwrap_or("unknown"),
                                    campaign_json["game"]["name"].as_str().unwrap_or("?"),
                                    is_acl_based,
                                    channels.len(),
                                    allowed_channels.len(),
                                    allow.get("isEnabled"),
                                );

                                // ACL campaign that parsed to zero channels is
                                // unfarmable (no candidates) even with live streams;
                                // dump the first raw entry to expose the field shape.
                                if allowed_channels.is_empty() {
                                    warn!(
                                        "[Drops/ACL] campaign '{}' is ACL-restricted but parsed 0 of {} channels — unfarmable. First raw entry: {}",
                                        campaign_json["name"].as_str().unwrap_or("unknown"),
                                        channels.len(),
                                        channels.first().map(|c| c.to_string()).unwrap_or_else(|| "none".into()),
                                    );
                                }
                            }
                        }
                    }
                }

                // Parse time-based drops - manually parse to handle camelCase field names
                let mut time_based_drops: Vec<TimeBasedDrop> = Vec::new();
                if let Some(drops) = campaign_json["timeBasedDrops"].as_array() {
                    for drop_json in drops {
                        let drop_id = drop_json["id"].as_str().unwrap_or("").to_string();
                        let drop_name = drop_json["name"].as_str().unwrap_or("").to_string();
                        let required_minutes =
                            drop_json["requiredMinutesWatched"].as_i64().unwrap_or(0) as i32;

                        // Parse benefit edges manually (nested structure)
                        let mut benefit_edges = Vec::new();
                        if let Some(edges) = drop_json["benefitEdges"].as_array() {
                            for edge in edges {
                                if let Some(benefit) = edge.get("benefit") {
                                    benefit_edges.push(DropBenefit {
                                        id: benefit["id"].as_str().unwrap_or("").to_string(),
                                        name: benefit["name"].as_str().unwrap_or("").to_string(),
                                        image_url: benefit["imageAssetURL"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        distribution_type: benefit["distributionType"]
                                            .as_str()
                                            .map(|s| s.to_string()),
                                    });
                                }
                            }
                        }

                        // Parse progress from "self" field if present
                        let progress = if let Some(self_data) = drop_json.get("self") {
                            if !self_data.is_null() {
                                let drop_instance_id =
                                    self_data["dropInstanceID"].as_str().map(|s| s.to_string());
                                Some(DropProgress {
                                    campaign_id: campaign_json["id"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    drop_id: drop_id.clone(),
                                    current_minutes_watched: self_data["currentMinutesWatched"]
                                        .as_i64()
                                        .unwrap_or(0)
                                        as i32,
                                    required_minutes_watched: required_minutes,
                                    is_claimed: self_data["isClaimed"].as_bool().unwrap_or(false),
                                    last_updated: Utc::now(),
                                    drop_instance_id,
                                })
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        // Determine if drop is mineable (has watch time requirement)
                        // Drops with 0 required minutes are event-based/badge drops that cannot be auto-collected
                        let is_collectible = required_minutes > 0;

                        debug!("[fetch_all_active_campaigns] Drop '{}': required_minutes={}, is_collectible={}", 
                            drop_name, required_minutes, is_collectible);

                        time_based_drops.push(TimeBasedDrop {
                            id: drop_id,
                            name: drop_name,
                            required_minutes_watched: required_minutes,
                            benefit_edges,
                            progress,
                            is_collectible,
                        });
                    }
                }

                // Parse dates
                let start_at = campaign_json["startAt"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|| Utc::now());

                let end_at = campaign_json["endAt"]
                    .as_str()
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|| Utc::now() + chrono::Duration::days(365));

                // Check if campaign is active (not upcoming or expired)
                let now = Utc::now();
                if start_at > now || end_at < now {
                    continue;
                }

                // Look for detailsURL or any URL field that might be the "about this drop" link
                let details_url = campaign_json["detailsURL"]
                    .as_str()
                    .or_else(|| campaign_json["aboutDropsURL"].as_str())
                    .or_else(|| campaign_json["aboutURL"].as_str())
                    .or_else(|| campaign_json["url"].as_str())
                    .map(|s| s.to_string());

                // Publisher "connect account" URL: present when this campaign requires
                // linking your Twitch account to the game before drops will credit.
                let account_link = campaign_json["accountLinkURL"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                result.push(DropCampaign {
                    id: campaign_json["id"].as_str().unwrap_or("").to_string(),
                    name: campaign_json["name"].as_str().unwrap_or("").to_string(),
                    game_id,
                    game_name,
                    description: campaign_json["description"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    image_url,
                    start_at,
                    end_at,
                    time_based_drops,
                    is_account_connected: campaign_json["self"]["isAccountConnected"]
                        .as_bool()
                        .unwrap_or(true),
                    allowed_channels,
                    is_acl_based,
                    details_url,
                    account_link,
                });
            }
        }

        debug!("Returning {} total campaigns (unfiltered)", result.len());
        Ok(result)
    }

    /// Of the given channels, the ones where Twitch itself is offering this
    /// campaign right now.
    ///
    /// `channel.viewerDropCampaigns` is Twitch's own answer to "can this drop be
    /// earned here, right now", so it settles a question the app cannot answer
    /// from campaign fields alone. Being on a campaign's allow-list is usually
    /// not enough - an allow-listed streamer live in some other category earns
    /// nothing (measured across Apex/ALGS, Albion and EVE rosters: every
    /// out-of-category channel is refused, every in-category one offered). But
    /// creator campaigns break that rule: `Ironmouse Subathon 2026` is filed
    /// under Special Events and Twitch offers it on her channel whatever she is
    /// playing. Guessing either way is wrong for the other case, so ask.
    ///
    /// One aliased document covers a whole chunk of channels, so a roster costs
    /// a request per 50 live channels rather than one per channel.
    pub async fn campaign_eligible_channels(
        &self,
        campaign_id: &str,
        channel_ids: &[String],
    ) -> Result<Vec<String>> {
        // Channel ids are numeric and go into the query text, so anything else
        // is dropped rather than escaped.
        let ids: Vec<&String> = channel_ids
            .iter()
            .filter(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let token = DropsAuthService::get_token().await?;
        let mut eligible = Vec::new();

        for chunk in ids.chunks(50) {
            let fields: String = chunk
                .iter()
                .enumerate()
                .map(|(i, id)| format!("c{i}: channel(id: \"{id}\") {{ id viewerDropCampaigns {{ id }} }} "))
                .collect();
            let query = format!("query {{ {fields}}}");

            let response = self
                .client
                .post("https://gql.twitch.tv/gql")
                .headers(self.create_gql_headers(&token))
                .json(&serde_json::json!({ "query": query }))
                .send()
                .await?;

            let body: serde_json::Value = response.json().await?;
            if let Some(errors) = body.get("errors") {
                return Err(anyhow::anyhow!("GraphQL errors: {:?}", errors));
            }

            for (i, id) in chunk.iter().enumerate() {
                let campaigns = &body["data"][format!("c{i}")]["viewerDropCampaigns"];
                let offered = campaigns
                    .as_array()
                    .map(|list| list.iter().any(|c| c["id"].as_str() == Some(campaign_id)))
                    .unwrap_or(false);
                if offered {
                    eligible.push((*id).clone());
                }
            }
        }

        debug!(
            "[Drops/Eligibility] campaign {}: {} of {} channels are offering it",
            campaign_id,
            eligible.len(),
            ids.len()
        );
        Ok(eligible)
    }

    /// Builds the drop-progress map from campaign data. Account-wide progress
    /// lives in each drop's `self` field. Shared by the UI sync path and the
    /// watched-channel monitor refresh so the two never drift.
    fn progress_from_campaigns(campaigns: &[DropCampaign]) -> HashMap<String, DropProgress> {
        let mut map = HashMap::new();
        for campaign in campaigns {
            for drop in &campaign.time_based_drops {
                if let Some(mut progress) = drop.progress.clone() {
                    progress.campaign_id = campaign.id.clone();
                    progress.drop_id = drop.id.clone();
                    map.insert(drop.id.clone(), progress);
                }
            }
        }
        map
    }

    /// Per-drop progress from the Inventory query, keyed by drop id. The
    /// campaign list's `self` progress lags while minutes are being earned;
    /// the inventory is where live minutes, claim state, and the
    /// dropInstanceIDs needed for claiming actually appear. The watched-channel
    /// monitor overlays this on top of the campaign snapshot so its auto-claim
    /// check can ever see a drop reach 100%.
    async fn fetch_inventory_progress(
        client: &Client,
        device_id: &str,
        session_id: &str,
    ) -> Result<HashMap<String, DropProgress>> {
        let token = DropsAuthService::get_token().await?;

        let response = client
            .post("https://gql.twitch.tv/gql")
            .headers(Self::gql_headers(&token, device_id, session_id))
            .json(&serde_json::json!({
                "operationName": "Inventory",
                "variables": { "fetchRewardCampaigns": false },
                "extensions": {
                    "persistedQuery": { "version": 1, "sha256Hash": INVENTORY_QUERY_HASH }
                }
            }))
            .send()
            .await?;

        let body: serde_json::Value = response.json().await?;
        let mut map = HashMap::new();
        let Some(campaigns) =
            body["data"]["currentUser"]["inventory"]["dropCampaignsInProgress"].as_array()
        else {
            return Ok(map);
        };
        for campaign in campaigns {
            let campaign_id = campaign["id"].as_str().unwrap_or("").to_string();
            let Some(drops) = campaign["timeBasedDrops"].as_array() else {
                continue;
            };
            for drop in drops {
                let Some(drop_id) = drop["id"].as_str() else { continue };
                let self_data = &drop["self"];
                if self_data.is_null() {
                    continue;
                }
                map.insert(
                    drop_id.to_string(),
                    DropProgress {
                        campaign_id: campaign_id.clone(),
                        drop_id: drop_id.to_string(),
                        current_minutes_watched: self_data["currentMinutesWatched"]
                            .as_i64()
                            .unwrap_or(0) as i32,
                        required_minutes_watched: drop["requiredMinutesWatched"]
                            .as_i64()
                            .unwrap_or(0) as i32,
                        is_claimed: self_data["isClaimed"].as_bool().unwrap_or(false),
                        last_updated: Utc::now(),
                        drop_instance_id: self_data["dropInstanceID"]
                            .as_str()
                            .map(String::from),
                    },
                );
            }
        }
        Ok(map)
    }

    /// Updates the service's internal state with fresh campaign data and calculates progress.
    pub async fn update_campaigns_and_progress(&self, campaigns: &[DropCampaign]) {
        {
            let mut progress_map = self.drop_progress.write().await;
            *progress_map = Self::progress_from_campaigns(campaigns);
        }

        // Update cached campaign count
        let mut cached_count = self.cached_active_campaigns_count.write().await;
        *cached_count = campaigns.len() as i32;

        // Update campaigns cache when automation fetches them
        let mut cache = self.cached_campaigns.write().await;
        *cache = Some((campaigns.to_vec(), Utc::now()));
    }

    /// Get active campaigns with settings filters applied (for automation)
    pub async fn get_active_campaigns(&self) -> Result<Vec<DropCampaign>> {
        // First get all campaigns from the API
        let all_campaigns = self.fetch_all_active_campaigns_from_api().await?;

        // Update internal progress map (this is a simplified version of the original logic)
        self.update_campaigns_and_progress(&all_campaigns).await;

        // Apply settings filters
        let settings = self.settings.read().await;
        let mut filtered_result = Vec::new();

        debug!("Applying filters to {} campaigns", all_campaigns.len());

        for campaign in all_campaigns {
            // Skip excluded games
            if settings.excluded_games.contains(&campaign.game_name) {
                debug!("  Filtered out: {} (excluded)", campaign.game_name);
                continue;
            }

            // Apply priority mode filter
            if settings.priority_mode == PriorityMode::PriorityOnly
                && !settings.priority_games.is_empty()
                && !settings.priority_games.contains(&campaign.game_name)
            {
                debug!(
                    "  Filtered out: {} (not in priority list)",
                    campaign.game_name
                );
                continue;
            }

            debug!("  Included: {} ({})", campaign.name, campaign.game_name);
            filtered_result.push(campaign);
        }

        debug!("Returning {} filtered campaigns", filtered_result.len());
        Ok(filtered_result)
    }

    pub async fn claim_drop(
        &self,
        drop_id: &str,
        provided_drop_instance_id: Option<&str>,
    ) -> Result<()> {
        let token = DropsAuthService::get_token().await?;

        // First check if a drop_instance_id was provided directly from the frontend
        // This is the most reliable method - the frontend extracts it from inventory data
        let drop_instance_id = if let Some(provided_id) = provided_drop_instance_id {
            debug!(
                "Using provided dropInstanceID from frontend: {}",
                provided_id
            );
            provided_id.to_string()
        } else {
            // Second, check if we have a stored drop_instance_id from the API response
            let (stored_instance_id, campaign_id) = {
                let progress_map = self.drop_progress.read().await;
                if let Some(progress) = progress_map.get(drop_id) {
                    (
                        progress.drop_instance_id.clone(),
                        progress.campaign_id.clone(),
                    )
                } else {
                    (None, String::new())
                }
            };

            if let Some(instance_id) = stored_instance_id {
                // Use the stored dropInstanceID from the API
                debug!("Using stored dropInstanceID from API: {}", instance_id);
                instance_id
            } else {
                // Fallback: Generate dropInstanceID in format: user_id#campaign_id#drop_id
                // This is how the Twitch web client constructs the claim ID when not available
                let user_id = self.get_user_id_from_token(&token).await?;

                if campaign_id.is_empty() {
                    // Last resort: just use drop_id
                    debug!("No campaign_id available, using drop_id as fallback");
                    drop_id.to_string()
                } else {
                    let generated_id = format!("{}#{}#{}", user_id, campaign_id, drop_id);
                    debug!("Generated dropInstanceID: {}", generated_id);
                    generated_id
                }
            }
        };

        debug!(
            "Claiming drop: {} with dropInstanceID: {}",
            drop_id, drop_instance_id
        );

        // Use persisted query format like the Twitch web client does
        let response = self
            .client
            .post("https://gql.twitch.tv/gql")
            .headers(self.create_gql_headers(&token))
            .json(&serde_json::json!({
                "operationName": "DropsPage_ClaimDropRewards",
                "variables": {
                    "input": {
                        "dropInstanceID": drop_instance_id
                    }
                },
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "a455deea71bdc9015b78eb49f4acfbce8baa7ccbedd28e549bb025bd0f751930"
                    }
                }
            }))
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await?;

        debug!("Claim response status: {}", status);
        debug!("Claim response: {}", response_text);

        if !status.is_success() {
            return Err(anyhow::anyhow!("Failed to claim drop: {}", response_text));
        }

        // Parse response to check for errors
        let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

        if let Some(errors) = response_json.get("errors") {
            debug!("GraphQL errors: {:?}", errors);
            return Err(anyhow::anyhow!("GraphQL errors: {:?}", errors));
        }

        // Check claimDropRewards response status
        if let Some(data) = response_json.get("data") {
            if let Some(claim_result) = data.get("claimDropRewards") {
                let result_status = claim_result
                    .get("status")
                    .and_then(|s| s.as_str())
                    .unwrap_or("UNKNOWN");

                debug!("Claim result status: {}", result_status);

                match result_status {
                    "ELIGIBLE_FOR_ALL" | "DROP_INSTANCE_ALREADY_CLAIMED" => {
                        debug!("Drop claimed successfully!");
                    }
                    _ => {
                        debug!("Unexpected claim status: {}", result_status);
                    }
                }
            }
        }

        // Update progress to mark as claimed
        let mut progress_map = self.drop_progress.write().await;
        if let Some(progress) = progress_map.get_mut(drop_id) {
            progress.is_claimed = true;
            progress.last_updated = Utc::now();
        }

        Ok(())
    }

    /// Get user ID from token by validating it with Twitch
    async fn get_user_id_from_token(&self, token: &str) -> Result<String> {
        let response = self
            .client
            .get("https://id.twitch.tv/oauth2/validate")
            .header(AUTHORIZATION, format!("OAuth {}", token))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Failed to validate token"));
        }

        let validation: serde_json::Value = response.json().await?;
        let user_id = validation["user_id"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("No user_id in token validation response"))?;

        Ok(user_id.to_string())
    }

    pub async fn check_channel_points(
        &self,
        channel_id: &str,
        channel_name: &str,
    ) -> Result<Option<ChannelPointsClaim>> {
        let token = DropsAuthService::get_token().await?;

        // Use persisted query like the Twitch web client
        let response = self
            .client
            .post("https://gql.twitch.tv/gql")
            .headers(self.create_gql_headers(&token))
            .json(&serde_json::json!({
                "operationName": "ChannelPointsContext",
                "variables": {
                    "channelLogin": channel_name.to_lowercase()
                },
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "9988086babc615a918a1e9a722ff41d98847acac822645209ac7379eecb27152"
                    }
                }
            }))
            .send()
            .await?;

        let response_json: serde_json::Value = response.json().await?;

        // Check for errors
        if let Some(errors) = response_json.get("errors") {
            return Err(anyhow::anyhow!("GraphQL errors: {:?}", errors));
        }

        // Parse the response - structure is: data.channel.self.communityPoints
        if let Some(data) = response_json.get("data") {
            if let Some(channel) = data.get("channel") {
                let channel_id_from_response = channel
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(channel_id);

                if let Some(self_data) = channel.get("self") {
                    if let Some(community_points) = self_data.get("communityPoints") {
                        let balance_val = community_points
                            .get("balance")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as i32;

                        // Update balance
                        let balance = ChannelPointsBalance {
                            channel_id: channel_id_from_response.to_string(),
                            channel_name: channel_name.to_string(),
                            balance: balance_val,
                            last_updated: Utc::now(),
                            points_name: None, // Not fetched via persisted query
                            points_icon_url: None,
                        };

                        let mut balances = self.channel_points_balances.write().await;
                        balances.insert(channel_id_from_response.to_string(), balance);

                        // Check if there's a claim available
                        if let Some(available_claim) = community_points.get("availableClaim") {
                            if !available_claim.is_null() {
                                let claim_id = available_claim
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let points_earned = available_claim
                                    .get("pointsEarnedTotal")
                                    .or_else(|| available_claim.get("pointsEarnedBaseline"))
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(50)
                                    as i32;

                                return Ok(Some(ChannelPointsClaim {
                                    id: claim_id,
                                    channel_id: channel_id_from_response.to_string(),
                                    channel_name: channel_name.to_string(),
                                    points_earned,
                                    claimed_at: Utc::now(),
                                    claim_type: ChannelPointsClaimType::Watch,
                                }));
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    pub async fn claim_channel_points(
        &self,
        channel_id: &str,
        _channel_name: &str,
        claim_id: &str,
    ) -> Result<BonusClaimResult> {
        let token = DropsAuthService::get_token().await?;

        // Field selection mirrors the official web client's ClaimCommunityPoints
        // (verified capture): `claim.pointsEarnedTotal` is the exact credited
        // amount (multipliers included), `currentPoints` is the new balance.
        let mutation = r#"
        mutation ClaimCommunityPoints($input: ClaimCommunityPointsInput!) {
            claimCommunityPoints(input: $input) {
                claim {
                    pointsEarnedTotal
                    pointsEarnedBaseline
                }
                currentPoints
                error {
                    code
                }
            }
        }
        "#;

        let response = self
            .client
            .post("https://gql.twitch.tv/gql")
            .headers(self.create_gql_headers(&token))
            .json(&serde_json::json!({
                "query": mutation,
                "variables": {
                    "input": {
                        "channelID": channel_id,
                        "claimID": claim_id
                    }
                }
            }))
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!(
                "Failed to claim channel points: {}",
                error_text
            ));
        }

        let result: serde_json::Value = response.json().await?;
        let payload = &result["data"]["claimCommunityPoints"];
        let new_balance = payload["currentPoints"].as_i64().unwrap_or(0) as i32;
        // Bonus chests are 50 at baseline; fall back to that if the response
        // omits the earned fields for any reason.
        let points_earned = payload["claim"]["pointsEarnedTotal"]
            .as_i64()
            .or_else(|| payload["claim"]["pointsEarnedBaseline"].as_i64())
            .unwrap_or(50) as i32;

        Ok(BonusClaimResult {
            new_balance,
            points_earned,
        })
    }

    pub async fn get_drop_progress(&self) -> Vec<DropProgress> {
        self.drop_progress.read().await.values().cloned().collect()
    }

    pub async fn get_claimed_drops(&self) -> Vec<ClaimedDrop> {
        self.claimed_drops.read().await.clone()
    }

    pub async fn get_channel_points_history(&self) -> Vec<ChannelPointsClaim> {
        self.channel_points_history.read().await.clone()
    }

    pub async fn get_statistics(&self) -> DropsStatistics {
        let claimed_drops = self.claimed_drops.read().await;
        let channel_points_history = self.channel_points_history.read().await;
        let drop_progress = self.drop_progress.read().await;

        // Lifetime cumulative of points the app has auto-claimed, persisted across sessions,
        // rather than only this session's history. Clamp into the i32 wire type.
        let total_channel_points_earned: i32 =
            (*self.lifetime_points_collected.read().await).clamp(0, i32::MAX as i64) as i32;

        let drops_in_progress = drop_progress
            .values()
            .filter(|p| !p.is_claimed && p.current_minutes_watched > 0)
            .count() as i32;

        // Use cached campaign count instead of fetching
        let active_campaigns = *self.cached_active_campaigns_count.read().await;

        DropsStatistics {
            total_drops_claimed: claimed_drops.len() as i32,
            total_channel_points_earned,
            active_campaigns,
            drops_in_progress,
            recent_claims: claimed_drops.iter().rev().take(10).cloned().collect(),
            channel_points_history: channel_points_history
                .iter()
                .rev()
                .take(20)
                .cloned()
                .collect(),
        }
    }

    pub async fn add_claimed_drop(&self, claimed_drop: ClaimedDrop) {
        let mut claimed_drops = self.claimed_drops.write().await;
        claimed_drops.push(claimed_drop);
    }

    pub async fn add_channel_points_claim(&self, claim: ChannelPointsClaim) {
        let claimed_points = claim.points_earned.max(0) as i64;
        {
            let mut history = self.channel_points_history.write().await;
            history.push(claim);
        }
        if claimed_points > 0 {
            let total = {
                let mut lifetime = self.lifetime_points_collected.write().await;
                *lifetime += claimed_points;
                *lifetime
            };
            save_lifetime_points_collected(total);
        }
    }

    pub async fn get_channel_points_balance(
        &self,
        channel_id: &str,
    ) -> Option<ChannelPointsBalance> {
        let balances = self.channel_points_balances.read().await;
        balances.get(channel_id).cloned()
    }

    pub async fn get_all_channel_points_balances(&self) -> Vec<ChannelPointsBalance> {
        let balances = self.channel_points_balances.read().await;
        balances.values().cloned().collect()
    }

    /// Upsert a channel's current balance. Fed by the realtime points-earned
    /// socket, which reports the new balance with every earn — the only thing
    /// that keeps this store current now that the automation loop is gone. Powers
    /// the channel-points leaderboard and the points accolades.
    pub async fn update_channel_points_balance(
        &self,
        channel_id: &str,
        channel_name: &str,
        balance: i32,
    ) {
        let mut balances = self.channel_points_balances.write().await;
        balances
            .entry(channel_id.to_string())
            .and_modify(|b| {
                b.balance = balance;
                b.last_updated = Utc::now();
                // Keep the friendliest name we've seen; the socket may report
                // an earn before any name has resolved.
                if !channel_name.is_empty() {
                    b.channel_name = channel_name.to_string();
                }
            })
            .or_insert_with(|| ChannelPointsBalance {
                channel_id: channel_id.to_string(),
                channel_name: channel_name.to_string(),
                balance,
                last_updated: Utc::now(),
                points_name: None,
                points_icon_url: None,
            });
    }

    pub async fn start_monitoring(
        &self,
        channel_id: String,
        channel_name: String,
        app_handle: AppHandle,
    ) {
        // Set current channel
        {
            let mut current = self.current_channel.write().await;
            *current = Some((channel_id.clone(), channel_name.clone()));
        }

        // Check if already monitoring
        {
            let mut monitoring = self.monitoring_active.write().await;
            if *monitoring {
                return; // Already monitoring
            }
            *monitoring = true;
        }

        // Clone Arc references for the background task
        let settings = self.settings.clone();
        let drop_progress = self.drop_progress.clone();
        let claimed_drops = self.claimed_drops.clone();
        let monitoring_active = self.monitoring_active.clone();
        let current_channel = self.current_channel.clone();
        let attempted_claims = self.attempted_claims.clone();
        let client = self.client.clone();
        let device_id = self.device_id.clone();
        let session_id = self.session_id.clone();

        // Spawn background monitoring task
        tokio::spawn(async move {
            debug!(
                "Started drops and channel points monitoring for {}",
                channel_name
            );

            // Watched-channel drop progress is account-wide: the heartbeat earns
            // it server-side, and this poll reads it back so the Drops center and
            // the finished-drop auto-claim below work with no background plugin.
            // Refreshed on its own cadence rather than every check tick.
            const PROGRESS_REFRESH_SECS: i64 = 120;
            let mut last_progress_refresh: Option<DateTime<Utc>> = None;
            // Claim-failure backoff: drop_id -> (attempts, last attempt). A
            // transient claim failure retries a few times instead of silently
            // giving up for the whole session.
            const CLAIM_MAX_ATTEMPTS: u32 = 3;
            const CLAIM_RETRY_SECS: i64 = 600;
            let mut failed_claims: HashMap<String, (u32, DateTime<Utc>)> = HashMap::new();
            // Drops already announced as ready, so the drop-ready event fires
            // once per drop instead of on every check tick.
            let mut notified_ready: std::collections::HashSet<String> =
                std::collections::HashSet::new();

            loop {
                // Check if monitoring should continue
                let should_continue = *monitoring_active.read().await;
                if !should_continue {
                    debug!("Stopping drops monitoring");
                    break;
                }

                // Get current settings
                let current_settings = settings.read().await.clone();
                let check_interval = Duration::from_secs(current_settings.check_interval_seconds);

                // Get current channel info
                let channel_info = current_channel.read().await.clone();
                if channel_info.is_some() {
                    // The watched channel's bonus chest is claimed by the
                    // frontend (ChatWidget, `auto_claim_points_watching`), the
                    // single user-present surface. The background multi-channel
                    // sweep lives in the opt-in automation plugin. This loop only
                    // keeps drop progress fresh and claims finished drops below.

                    // Refresh account-wide drop progress so the watched channel's
                    // earned minutes (credited by the heartbeat) reach the Drops
                    // center display and the auto-claim below. This is the core,
                    // always-on watched-channel path; the background plugin is a
                    // separate opt-in plugin.
                    let refresh_due = last_progress_refresh
                        .map(|t| {
                            Utc::now().signed_duration_since(t).num_seconds()
                                >= PROGRESS_REFRESH_SECS
                        })
                        .unwrap_or(true);
                    if refresh_due {
                        let campaign_snapshot =
                            match Self::fetch_active_campaigns(&client, &device_id, &session_id)
                                .await
                            {
                                Ok(campaigns) => Some(Self::progress_from_campaigns(&campaigns)),
                                Err(e) => {
                                    debug!("Watched-channel drop progress refresh failed: {}", e);
                                    None
                                }
                            };
                        // The campaign list lags earned minutes and never carries
                        // dropInstanceIDs; the inventory is the live source. Overlay
                        // it so the auto-claim check below actually sees completion.
                        let inventory_overlay =
                            match Self::fetch_inventory_progress(&client, &device_id, &session_id)
                                .await
                            {
                                Ok(map) => Some(map),
                                Err(e) => {
                                    debug!("Inventory progress refresh failed: {}", e);
                                    None
                                }
                            };
                        if campaign_snapshot.is_some() || inventory_overlay.is_some() {
                            let mut progress_map = drop_progress.write().await;
                            if let Some(snapshot) = campaign_snapshot {
                                *progress_map = snapshot;
                            }
                            if let Some(overlay) = inventory_overlay {
                                progress_map.extend(overlay);
                            }
                            last_progress_refresh = Some(Utc::now());
                        }
                    }

                    // Check for claimable drops from the refreshed progress map.
                    // attempted_claims holds SUCCESSFUL claims only; failures live
                    // in failed_claims with a retry budget.
                    let claimable_drops: Vec<DropProgress> = {
                        let progress_map = drop_progress.read().await;
                        let attempted = attempted_claims.read().await;
                        progress_map
                            .values()
                            .filter(|p| {
                                !p.is_claimed
                                    && p.current_minutes_watched >= p.required_minutes_watched
                                    && p.required_minutes_watched > 0 // Only collectible drops
                                    && !attempted.contains(&p.drop_id) // Skip already-claimed
                            })
                            .cloned()
                            .collect()
                    };

                    for progress in claimable_drops {
                        // Announce a ready drop once, not on every check tick.
                        if current_settings.notify_on_drop_available
                            && notified_ready.insert(progress.drop_id.clone())
                        {
                            let _ = app_handle.emit("drop-ready", &progress);
                        }

                        // Auto-claim if enabled
                        if current_settings.auto_claim_drops {
                            // Respect the failure backoff: retry a failed claim a few
                            // times, spaced out, instead of giving up for the session.
                            if let Some((attempts, last)) = failed_claims.get(&progress.drop_id) {
                                if *attempts >= CLAIM_MAX_ATTEMPTS
                                    || Utc::now().signed_duration_since(*last).num_seconds()
                                        < CLAIM_RETRY_SECS
                                {
                                    continue;
                                }
                            }

                            match Self::claim_drop_internal(
                                &client,
                                &progress.drop_id,
                                &drop_progress,
                            )
                            .await
                            {
                                Ok(_) => {
                                    debug!("Auto-claimed drop: {}", progress.drop_id);
                                    failed_claims.remove(&progress.drop_id);
                                    attempted_claims
                                        .write()
                                        .await
                                        .insert(progress.drop_id.clone());

                                    // Create claimed drop record
                                    let claimed = ClaimedDrop {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        campaign_id: progress.campaign_id.clone(),
                                        drop_id: progress.drop_id.clone(),
                                        drop_name: "Drop".to_string(), // Would need to fetch from campaign
                                        game_name: "Game".to_string(),
                                        benefit_name: "Reward".to_string(),
                                        benefit_image_url: String::new(),
                                        claimed_at: Utc::now(),
                                    };

                                    let mut claimed_drops_lock = claimed_drops.write().await;
                                    claimed_drops_lock.push(claimed.clone());

                                    if current_settings.notify_on_drop_claimed {
                                        let _ = app_handle.emit("drop-claimed", &claimed);
                                    }
                                }
                                Err(e) => {
                                    let entry = failed_claims
                                        .entry(progress.drop_id.clone())
                                        .or_insert((0, Utc::now()));
                                    entry.0 += 1;
                                    entry.1 = Utc::now();
                                    error!(
                                        "Failed to auto-claim drop (attempt {}/{}): {}",
                                        entry.0, CLAIM_MAX_ATTEMPTS, e
                                    );
                                }
                            }
                        }
                    }
                }

                // Wait for next check interval
                tokio::time::sleep(check_interval).await;
            }
        });
    }

    pub async fn stop_monitoring(&self) {
        let mut monitoring = self.monitoring_active.write().await;
        *monitoring = false;

        let mut current = self.current_channel.write().await;
        *current = None;
    }

    pub async fn update_current_channel(&self, channel_id: String, channel_name: String) {
        let mut current = self.current_channel.write().await;
        *current = Some((channel_id, channel_name));
    }

    /// Update drop progress from WebSocket events
    pub async fn update_drop_progress_from_websocket(
        &self,
        drop_id: String,
        current_minutes: i32,
        required_minutes: i32,
    ) {
        let mut progress_map = self.drop_progress.write().await;

        if let Some(progress) = progress_map.get_mut(&drop_id) {
            // Update existing progress
            progress.current_minutes_watched = current_minutes;
            progress.required_minutes_watched = required_minutes;
            progress.last_updated = Utc::now();

            debug!(
                "Updated drop progress from WebSocket: {}/{} minutes for drop {}",
                current_minutes, required_minutes, drop_id
            );
        } else {
            // Create new progress entry if it doesn't exist
            let progress = DropProgress {
                campaign_id: String::new(), // Will be filled in later
                drop_id: drop_id.clone(),
                current_minutes_watched: current_minutes,
                required_minutes_watched: required_minutes,
                is_claimed: false,
                last_updated: Utc::now(),
                drop_instance_id: None, // Will be populated when we fetch from API
            };
            progress_map.insert(drop_id.clone(), progress);

            debug!(
                "Created new drop progress from WebSocket: {}/{} minutes for drop {}",
                current_minutes, required_minutes, drop_id
            );
        }
    }

    // Internal helper methods that don't require &self
    async fn get_active_campaigns_internal(
        client: &Client,
        drop_progress: &Arc<RwLock<HashMap<String, DropProgress>>>,
    ) -> Result<Vec<DropCampaign>> {
        let token = DropsAuthService::get_token().await?;

        // Create headers similar to the main methods
        let mut headers = HeaderMap::new();
        headers.insert("Client-ID", HeaderValue::from_static(CLIENT_ID));
        headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
        headers.insert("Accept-Language", HeaderValue::from_static("en-US"));
        headers.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("OAuth {}", token)).unwrap(),
        );
        headers.insert("Origin", HeaderValue::from_static(CLIENT_URL));
        headers.insert("Referer", HeaderValue::from_static(CLIENT_URL));

        let query = r#"
        query DropCampaigns {
            currentUser {
                dropCampaigns {
                    id
                    name
                    game {
                        id
                        name
                    }
                    description
                    imageURL
                    startAt
                    endAt
                    timeBasedDrops {
                        id
                        name
                        requiredMinutesWatched
                        benefitEdges {
                            benefit {
                                id
                                name
                                imageAssetURL
                            }
                        }
                        self {
                            currentMinutesWatched
                            isClaimed
                        }
                    }
                }
            }
        }
        "#;

        let response = client
            .post("https://gql.twitch.tv/gql")
            .headers(headers)
            .json(&serde_json::json!({
                "query": query,
                "variables": {}
            }))
            .send()
            .await?;

        let gql_response: GraphQLResponse<DropCampaignsData> = response.json().await?;

        if let Some(errors) = gql_response.errors {
            return Err(anyhow::anyhow!("GraphQL errors: {:?}", errors));
        }

        let campaigns = gql_response
            .data
            .and_then(|d| d.current_user)
            .map(|u| u.drop_campaigns)
            .unwrap_or_default();

        let mut result = Vec::new();
        let mut progress_map = drop_progress.write().await;

        for campaign in campaigns {
            let time_based_drops: Vec<TimeBasedDrop> = campaign
                .time_based_drops
                .iter()
                .map(|drop| {
                    // Update progress tracking
                    if let Some(self_progress) = &drop.self_progress {
                        let progress = DropProgress {
                            campaign_id: campaign.id.clone(),
                            drop_id: drop.id.clone(),
                            current_minutes_watched: self_progress.current_minutes_watched,
                            required_minutes_watched: drop.required_minutes_watched,
                            is_claimed: self_progress.is_claimed,
                            last_updated: Utc::now(),
                            drop_instance_id: None, // Internal query doesn't return dropInstanceID
                        };
                        progress_map.insert(drop.id.clone(), progress);
                    }

                    TimeBasedDrop {
                        id: drop.id.clone(),
                        name: drop.name.clone(),
                        required_minutes_watched: drop.required_minutes_watched,
                        benefit_edges: drop
                            .benefit_edges
                            .iter()
                            .map(|edge| DropBenefit {
                                id: edge.benefit.id.clone(),
                                name: edge.benefit.name.clone(),
                                image_url: edge.benefit.image_asset_url.clone(),
                                distribution_type: None, // GQL struct doesn't have this field
                            })
                            .collect(),
                        progress: None,
                        is_collectible: drop.required_minutes_watched > 0,
                    }
                })
                .collect();

            result.push(DropCampaign {
                id: campaign.id,
                name: campaign.name,
                game_id: campaign.game.id,
                game_name: campaign.game.name,
                description: campaign.description,
                image_url: campaign.image_url,
                start_at: DateTime::parse_from_rfc3339(&campaign.start_at)
                    .unwrap_or_else(|_| {
                        DateTime::parse_from_rfc3339("2000-01-01T00:00:00Z").unwrap()
                    })
                    .with_timezone(&Utc),
                end_at: DateTime::parse_from_rfc3339(&campaign.end_at)
                    .unwrap_or_else(|_| {
                        DateTime::parse_from_rfc3339("2099-12-31T23:59:59Z").unwrap()
                    })
                    .with_timezone(&Utc),
                time_based_drops,
                is_account_connected: true, // Internal campaigns are always connected
                allowed_channels: Vec::new(),
                is_acl_based: false,
                details_url: None, // Will be populated from the main fetch method
                account_link: None,
            });
        }

        Ok(result)
    }

    async fn claim_drop_internal(
        client: &Client,
        drop_id: &str,
        drop_progress: &Arc<RwLock<HashMap<String, DropProgress>>>,
    ) -> Result<()> {
        let token = DropsAuthService::get_token().await?;

        // First, check if we have a stored drop_instance_id from the API response
        let (stored_instance_id, campaign_id) = {
            let progress_map = drop_progress.read().await;
            if let Some(progress) = progress_map.get(drop_id) {
                (
                    progress.drop_instance_id.clone(),
                    progress.campaign_id.clone(),
                )
            } else {
                (None, String::new())
            }
        };

        // Determine the dropInstanceID to use
        let drop_instance_id = if let Some(instance_id) = stored_instance_id {
            // Use the stored dropInstanceID from the API (this is the correct one!)
            debug!(
                "[Auto] Using stored dropInstanceID from API: {}",
                instance_id
            );
            instance_id
        } else {
            // Fallback: Generate dropInstanceID in format: user_id#campaign_id#drop_id
            // Get user_id from token validation
            let validation_response = client
                .get("https://id.twitch.tv/oauth2/validate")
                .header(AUTHORIZATION, format!("OAuth {}", token))
                .send()
                .await?;

            if !validation_response.status().is_success() {
                return Err(anyhow::anyhow!("Failed to validate token for auto-claim"));
            }

            let validation: serde_json::Value = validation_response.json().await?;
            let user_id = validation["user_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("No user_id in token validation"))?;

            if campaign_id.is_empty() {
                drop_id.to_string()
            } else {
                let generated_id = format!("{}#{}#{}", user_id, campaign_id, drop_id);
                debug!("[Auto] Generated dropInstanceID: {}", generated_id);
                generated_id
            }
        };

        debug!(
            "[Auto] Claiming drop: {} with dropInstanceID: {}",
            drop_id, drop_instance_id
        );

        // Create headers
        let mut headers = HeaderMap::new();
        headers.insert("Client-ID", HeaderValue::from_static(CLIENT_ID));
        headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
        headers.insert("Accept-Language", HeaderValue::from_static("en-US"));
        headers.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("OAuth {}", token)).unwrap(),
        );
        headers.insert("Origin", HeaderValue::from_static(CLIENT_URL));
        headers.insert("Referer", HeaderValue::from_static(CLIENT_URL));

        // Use persisted query format
        let response = client
            .post("https://gql.twitch.tv/gql")
            .headers(headers)
            .json(&serde_json::json!({
                "operationName": "DropsPage_ClaimDropRewards",
                "variables": {
                    "input": {
                        "dropInstanceID": drop_instance_id
                    }
                },
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "a455deea71bdc9015b78eb49f4acfbce8baa7ccbedd28e549bb025bd0f751930"
                    }
                }
            }))
            .send()
            .await?;

        let status = response.status();
        let response_text = response.text().await?;

        debug!("[Auto] Claim response status: {}", status);
        debug!("[Auto] Claim response: {}", response_text);

        if !status.is_success() {
            return Err(anyhow::anyhow!("Failed to claim drop: {}", response_text));
        }

        // Parse response to check for errors
        let response_json: serde_json::Value = serde_json::from_str(&response_text)?;

        if let Some(errors) = response_json.get("errors") {
            debug!("[Auto] GraphQL errors: {:?}", errors);
            return Err(anyhow::anyhow!("GraphQL errors: {:?}", errors));
        }

        // Update progress to mark as claimed
        let mut progress_map = drop_progress.write().await;
        if let Some(progress) = progress_map.get_mut(drop_id) {
            progress.is_claimed = true;
            progress.last_updated = Utc::now();
        }

        Ok(())
    }
}
