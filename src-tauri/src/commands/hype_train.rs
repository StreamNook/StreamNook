use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::services::hype_train_kind::{train_look, Facts, TrainLook, HEIST_THEME_UNTIL_MS};

// Use Twitch web client ID - works without authentication for read operations
const WEB_CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");
const GQL_URL: &str = "https://gql.twitch.tv/gql";

// Raw documents, not persisted hashes: the train's kind, treasure and shared
// details are newer than any hash Twitch keeps for anonymous callers (the
// web's own newer bulk hash answers PersistedQueryNotFound). All public.

/// One request per watch poll: whether the channel runs a train and of what
/// kind, and the train itself (progress, treasure discount, shared
/// participants with their categories, the channel's color, its category).
const LIVE_QUERY: &str = "query HypeTrainLive($channelIDs: [ID!]!, $login: String!) { allActiveHypeTrainStatuses(channelIDs: $channelIDs) { id level hypeTrainType isGoldenKappaTrain isAllTimeHighTrain isTreasureTrain isSharedTrain channel { id } } user(login: $login) { stream { game { id } } channel { hypeTrain { execution { id startedAt expiresAt isTreasureTrain treasureTrainDetails { discountPercentage discountLevelThreshold } sharedHypeTrainDetails { sharedProgress { user { id stream { game { id } } } } } config { primaryHexColor } progress { progression goal total } } } } } }";

/// The stream cards' badges: which of many channels run a train, and its kind.
const BADGES_QUERY: &str = "query HypeTrainBadges($channelIDs: [ID!]!) { allActiveHypeTrainStatuses(channelIDs: $channelIDs) { id level hypeTrainType isGoldenKappaTrain isAllTimeHighTrain isTreasureTrain isSharedTrain channel { id } } }";

/// The categories of the channels running a regular train, asked only while
/// the Heist Train theme runs (a heist is a regular train on GTA VI).
const GAMES_QUERY: &str = "query HypeTrainGames($ids: [ID!]!) { users(ids: $ids) { id stream { game { id } } } }";

/// Create headers for GQL requests (no auth required for read operations)
fn create_gql_headers() -> HeaderMap {
    let device_id = Uuid::new_v4().to_string().replace("-", "");
    let session_id = Uuid::new_v4().to_string().replace("-", "");

    let mut headers = HeaderMap::new();
    headers.insert("Client-ID", HeaderValue::from_static(WEB_CLIENT_ID));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert("X-Device-Id", HeaderValue::from_str(&device_id).unwrap());
    headers.insert(
        "Client-Session-Id",
        HeaderValue::from_str(&session_id).unwrap(),
    );
    headers
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ============================================================================
// GQL RESPONSE STRUCTS
// ============================================================================

#[derive(Debug, Deserialize)]
struct GqlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GqlError>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct StatusesData {
    #[serde(rename = "allActiveHypeTrainStatuses")]
    all_active_hype_train_statuses: Option<Vec<ActiveHypeTrainStatus>>,
}

#[derive(Debug, Deserialize)]
struct ActiveHypeTrainStatus {
    id: String,
    channel: IdOnly,
    level: i32,
    #[serde(rename = "hypeTrainType")]
    hype_train_type: Option<String>,
    #[serde(rename = "isGoldenKappaTrain")]
    is_golden_kappa_train: Option<bool>,
    #[serde(rename = "isAllTimeHighTrain")]
    is_all_time_high_train: Option<bool>,
    #[serde(rename = "isTreasureTrain")]
    is_treasure_train: Option<bool>,
    #[serde(rename = "isSharedTrain")]
    is_shared_train: Option<bool>,
}

impl ActiveHypeTrainStatus {
    fn facts<'a>(&'a self, games: Vec<&'a str>, now_ms: i64) -> Facts<'a> {
        Facts {
            train_type: self.hype_train_type.as_deref(),
            golden_kappa: self.is_golden_kappa_train.unwrap_or(false),
            treasure: self.is_treasure_train.unwrap_or(false),
            shared: self.is_shared_train.unwrap_or(false),
            all_time_high: self.is_all_time_high_train.unwrap_or(false),
            games,
            now_ms,
            ..Facts::default()
        }
    }
}

#[derive(Debug, Deserialize)]
struct IdOnly {
    id: String,
}

#[derive(Debug, Deserialize)]
struct LiveData {
    #[serde(rename = "allActiveHypeTrainStatuses")]
    all_active_hype_train_statuses: Option<Vec<ActiveHypeTrainStatus>>,
    user: Option<LiveUser>,
}

#[derive(Debug, Deserialize)]
struct LiveUser {
    stream: Option<StreamGame>,
    channel: Option<LiveChannel>,
}

#[derive(Debug, Deserialize)]
struct StreamGame {
    game: Option<IdOnly>,
}

#[derive(Debug, Deserialize)]
struct LiveChannel {
    #[serde(rename = "hypeTrain")]
    hype_train: Option<HypeTrainContainer>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainContainer {
    execution: Option<HypeTrainExecution>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainExecution {
    #[serde(rename = "startedAt")]
    started_at: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<String>,
    #[serde(rename = "isTreasureTrain")]
    is_treasure_train: Option<bool>,
    #[serde(rename = "treasureTrainDetails")]
    treasure_train_details: Option<TreasureDetails>,
    #[serde(rename = "sharedHypeTrainDetails")]
    shared_hype_train_details: Option<SharedDetails>,
    config: Option<TrainConfig>,
    progress: Option<HypeTrainProgress>,
}

#[derive(Debug, Deserialize)]
struct TreasureDetails {
    #[serde(rename = "discountPercentage")]
    discount_percentage: Option<i32>,
    #[serde(rename = "discountLevelThreshold")]
    discount_level_threshold: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct SharedDetails {
    #[serde(rename = "sharedProgress", default)]
    shared_progress: Option<Vec<SharedProgress>>,
}

#[derive(Debug, Deserialize)]
struct SharedProgress {
    user: Option<SharedUser>,
}

#[derive(Debug, Deserialize)]
struct SharedUser {
    stream: Option<StreamGame>,
}

#[derive(Debug, Deserialize)]
struct TrainConfig {
    #[serde(rename = "primaryHexColor")]
    primary_hex_color: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainProgress {
    progression: Option<i32>, // current progress within level (this is the key field!)
    goal: Option<i32>,        // total points needed for current level
    total: Option<i32>,       // total contributions across all levels
}

#[derive(Debug, Deserialize)]
struct GamesData {
    users: Option<Vec<Option<GameUser>>>,
}

#[derive(Debug, Deserialize)]
struct GameUser {
    id: String,
    stream: Option<StreamGame>,
}

/// One raw document to Twitch, anonymously; GraphQL errors are an error.
async fn post<T: for<'de> Deserialize<'de>>(operation: &str, query: &str, variables: serde_json::Value) -> Result<T, String> {
    let body = serde_json::json!({ "operationName": operation, "query": query, "variables": variables });
    let response = crate::services::http::client()
        .post(GQL_URL)
        .headers(create_gql_headers())
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let text = response.text().await.map_err(|e| format!("Read error: {}", e))?;
    let parsed: GqlResponse<T> = serde_json::from_str(&text).map_err(|e| format!("Parse error: {}", e))?;
    if let Some(first) = parsed.errors.first() {
        return Err(format!("{operation}: {}", first.message));
    }
    parsed.data.ok_or_else(|| format!("{operation}: no data"))
}

// ============================================================================
// PUBLIC API STRUCTS (sent to frontend)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypeTrainStatus {
    pub is_active: bool,
    pub id: Option<String>,
    pub level: i32,
    pub progress: i32,
    pub goal: i32,
    pub total: i32,
    pub started_at: Option<String>,
    pub expires_at: Option<String>,
    pub is_level_up: bool,
    /// Kept for older builds; `look.kind` says it now.
    pub is_golden_kappa: bool,
    /// The kind of train and how the bars draw it.
    pub look: TrainLook,
}

impl Default for HypeTrainStatus {
    fn default() -> Self {
        HypeTrainStatus {
            is_active: false,
            id: None,
            level: 1,
            progress: 0,
            goal: 0,
            total: 0,
            started_at: None,
            expires_at: None,
            is_level_up: false,
            is_golden_kappa: false,
            look: TrainLook::default(),
        }
    }
}

/// Lightweight status for bulk queries (badge display)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HypeTrainBulkStatus {
    pub channel_id: String,
    pub is_active: bool,
    pub level: i32,
    pub is_golden_kappa: bool,
    /// The kind of train, for the badge's color and word.
    pub look: TrainLook,
}

/// Fetch Hype Train status for multiple channels in a single GQL request
/// Used for badge display on stream cards in Home and Sidebar
#[tauri::command]
pub async fn get_bulk_hype_train_status(
    channel_ids: Vec<String>,
) -> Result<Vec<HypeTrainBulkStatus>, String> {
    if channel_ids.is_empty() {
        return Ok(vec![]);
    }
    let data: StatusesData =
        post("HypeTrainBadges", BADGES_QUERY, serde_json::json!({ "channelIDs": channel_ids })).await?;
    let active_trains = data.all_active_hype_train_statuses.unwrap_or_default();
    let now = now_ms();

    // A heist is a regular train on GTA VI: while the theme runs, ask the
    // categories of the channels running a regular train (a small second ask,
    // only when there are any). Without an answer they stay regular.
    let regular: Vec<String> = active_trains
        .iter()
        .filter(|t| now < HEIST_THEME_UNTIL_MS && t.hype_train_type.as_deref().map_or(true, |k| k.eq_ignore_ascii_case("REGULAR")))
        .map(|t| t.channel.id.clone())
        .collect();
    let mut games: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if !regular.is_empty() {
        match post::<GamesData>("HypeTrainGames", GAMES_QUERY, serde_json::json!({ "ids": regular })).await {
            Ok(found) => {
                for user in found.users.unwrap_or_default().into_iter().flatten() {
                    if let Some(game) = user.stream.and_then(|s| s.game) {
                        games.insert(user.id, game.id);
                    }
                }
            }
            Err(e) => debug!("[HypeTrain] categories for the badges: {e}"),
        }
    }

    let active_map: std::collections::HashMap<&str, &ActiveHypeTrainStatus> =
        active_trains.iter().map(|t| (t.channel.id.as_str(), t)).collect();

    Ok(channel_ids
        .iter()
        .map(|channel_id| match active_map.get(channel_id.as_str()) {
            Some(train) => {
                let game: Vec<&str> = games.get(channel_id).map(|g| vec![g.as_str()]).unwrap_or_default();
                HypeTrainBulkStatus {
                    channel_id: channel_id.clone(),
                    is_active: true,
                    level: train.level,
                    is_golden_kappa: train.is_golden_kappa_train.unwrap_or(false),
                    look: train_look(&train.facts(game, now)),
                }
            }
            None => HypeTrainBulkStatus {
                channel_id: channel_id.clone(),
                is_active: false,
                level: 0,
                is_golden_kappa: false,
                look: TrainLook::default(),
            },
        })
        .collect())
}

/// Fetch current Hype Train status for a channel via GQL: one request asks
/// whether it runs a train (and its kind) and reads the train itself.
#[tauri::command]
pub async fn get_hype_train_status(
    channel_id: String,
    channel_login: String,
) -> Result<HypeTrainStatus, String> {
    let data: LiveData = post(
        "HypeTrainLive",
        LIVE_QUERY,
        serde_json::json!({ "channelIDs": [channel_id], "login": channel_login }),
    )
    .await?;

    let Some(train_status) = data
        .all_active_hype_train_statuses
        .unwrap_or_default()
        .into_iter()
        .find(|s| s.channel.id == channel_id)
    else {
        return Ok(HypeTrainStatus::default());
    };

    let user = data.user;
    let channel_game = user.as_ref().and_then(|u| u.stream.as_ref()).and_then(|s| s.game.as_ref()).map(|g| g.id.clone());
    let execution = user.and_then(|u| u.channel).and_then(|c| c.hype_train).and_then(|h| h.execution);

    let (progress, goal, total) = execution
        .as_ref()
        .and_then(|e| e.progress.as_ref())
        .map(|p| (p.progression.unwrap_or(0), p.goal.unwrap_or(0), p.total.unwrap_or(0)))
        .unwrap_or((0, 0, 0));
    if goal == 0 {
        debug!("[HypeTrain] Level {} active on {} (no progress details available)", train_status.level, channel_login);
    }

    // Every category the train runs in: the channel's and each shared participant's.
    let mut games: Vec<String> = channel_game.into_iter().collect();
    if let Some(shared) = execution.as_ref().and_then(|e| e.shared_hype_train_details.as_ref()) {
        for part in shared.shared_progress.iter().flatten() {
            if let Some(game) = part.user.as_ref().and_then(|u| u.stream.as_ref()).and_then(|s| s.game.as_ref()) {
                games.push(game.id.clone());
            }
        }
    }
    let now = now_ms();
    let mut facts = train_status.facts(games.iter().map(String::as_str).collect(), now);
    if let Some(exec) = execution.as_ref() {
        facts.treasure |= exec.is_treasure_train.unwrap_or(false);
        facts.treasure_details = exec.treasure_train_details.as_ref().and_then(|d| {
            Some((d.discount_percentage?, d.discount_level_threshold.unwrap_or(1)))
        });
        facts.channel_color = exec.config.as_ref().and_then(|c| c.primary_hex_color.as_deref());
    }
    let look = train_look(&facts);

    debug!(
        "[HypeTrain] {} level {} ({}{}) {}/{} on {}",
        look.name, train_status.level, look.kind, look.theme.as_deref().map(|t| format!(", {t}")).unwrap_or_default(),
        progress, goal, channel_login
    );

    Ok(HypeTrainStatus {
        is_active: true,
        is_golden_kappa: train_status.is_golden_kappa_train.unwrap_or(false),
        id: Some(train_status.id.clone()),
        level: train_status.level,
        progress,
        goal,
        total,
        started_at: execution.as_ref().and_then(|e| e.started_at.clone()),
        expires_at: execution.as_ref().and_then(|e| e.expires_at.clone()),
        is_level_up: false,
        look,
    })
}

/// Show `login`'s hype train in this window. One poll per channel serves every
/// surface in every window (see services/hype_train_watch.rs).
#[tauri::command]
pub async fn hype_train_watch(
    app: crate::rt::AppHandle,
    window: crate::rt::WebviewWindow,
    login: String,
    channel_id: Option<String>,
    name: Option<String>,
) -> Result<(), String> {
    crate::services::hype_train_watch::watch(
        &app,
        window.label(),
        &login,
        channel_id,
        name.as_deref().unwrap_or_default(),
    );
    Ok(())
}

/// Stop showing `login`'s hype train in this window.
#[tauri::command]
pub async fn hype_train_unwatch(window: crate::rt::WebviewWindow, login: String) -> Result<(), String> {
    crate::services::hype_train_watch::unwatch(window.label(), &login);
    Ok(())
}
