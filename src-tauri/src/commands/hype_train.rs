use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Use Twitch web client ID - works without authentication for read operations
const WEB_CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");
const GQL_URL: &str = "https://gql.twitch.tv/gql";

// GQL hash for BulkAllActiveHypeTrainStatusesQuery - checks if channel has active train
const BULK_HYPE_TRAIN_HASH: &str =
    "88e62c2cbd13b7bdce93cc8934727003a5cadd821938538f74848199fbfe84a0";

// GQL hash for GetHypeTrainExecution - gets full details including progress
const HYPE_TRAIN_EXECUTION_HASH: &str =
    "8a39e843c94c5109a4cfb9badc641733e2205c60f5ee30e9b55edf0ad9db870a";

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

// ============================================================================
// GQL RESPONSE STRUCTS - BulkAllActiveHypeTrainStatusesQuery
// ============================================================================

#[derive(Debug, Deserialize)]
struct BulkHypeTrainResponse {
    data: Option<BulkHypeTrainData>,
}

#[derive(Debug, Deserialize)]
struct BulkHypeTrainData {
    #[serde(rename = "allActiveHypeTrainStatuses")]
    all_active_hype_train_statuses: Option<Vec<ActiveHypeTrainStatus>>,
}

#[derive(Debug, Deserialize)]
struct ActiveHypeTrainStatus {
    id: String,
    channel: HypeTrainChannel,
    level: i32,
    #[serde(rename = "isGoldenKappaTrain")]
    is_golden_kappa_train: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainChannel {
    id: String,
}

// ============================================================================
// GQL RESPONSE STRUCTS - GetHypeTrainExecution (for progress details)
// Actual structure: user.channel.hypeTrain.execution.progress.level
// ============================================================================

#[derive(Debug, Deserialize)]
struct HypeTrainExecutionResponse {
    data: Option<HypeTrainExecutionData>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainExecutionData {
    user: Option<HypeTrainExecutionUser>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainExecutionUser {
    channel: Option<HypeTrainExecutionChannel>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainExecutionChannel {
    #[serde(rename = "hypeTrain")]
    hype_train: Option<HypeTrainContainer>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainContainer {
    execution: Option<HypeTrainExecution>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainExecution {
    id: String,
    #[serde(rename = "startedAt")]
    started_at: Option<String>,
    #[serde(rename = "expiresAt")]
    expires_at: Option<String>,
    #[serde(rename = "endedAt")]
    ended_at: Option<String>,
    progress: Option<HypeTrainProgress>,
}

#[derive(Debug, Deserialize)]
struct HypeTrainProgress {
    level: Option<HypeTrainLevel>,
    progression: Option<i32>, // current progress within level (this is the key field!)
    goal: Option<i32>,        // total points needed for current level
    total: Option<i32>,       // total contributions across all levels
}

#[derive(Debug, Deserialize)]
struct HypeTrainLevel {
    value: i32, // current level number
    goal: i32,  // points needed for this level
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
    pub is_golden_kappa: bool,
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
        }
    }
}

/// Lightweight status for bulk queries (badge display)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypeTrainBulkStatus {
    pub channel_id: String,
    pub is_active: bool,
    pub level: i32,
    pub is_golden_kappa: bool,
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

    let client = crate::services::http::client().clone();

    // BulkAllActiveHypeTrainStatusesQuery accepts an array of channel IDs
    let bulk_request = serde_json::json!({
        "operationName": "BulkAllActiveHypeTrainStatusesQuery",
        "variables": {
            "channelIDs": channel_ids
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": BULK_HYPE_TRAIN_HASH
            }
        }
    });

    let bulk_response = client
        .post(GQL_URL)
        .headers(create_gql_headers())
        .json(&bulk_request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !bulk_response.status().is_success() {
        return Err(format!("HTTP {}", bulk_response.status()));
    }

    let bulk_text = bulk_response
        .text()
        .await
        .map_err(|e| format!("Read error: {}", e))?;
    let bulk_data: BulkHypeTrainResponse =
        serde_json::from_str(&bulk_text).map_err(|e| format!("Parse error: {}", e))?;

    // Extract active trains from response
    let active_trains = bulk_data
        .data
        .and_then(|d| d.all_active_hype_train_statuses)
        .unwrap_or_default();

    // Create a map of channel_id -> train status for quick lookup
    let active_map: std::collections::HashMap<String, &ActiveHypeTrainStatus> = active_trains
        .iter()
        .map(|t| (t.channel.id.clone(), t))
        .collect();

    // Build result for all requested channel IDs
    let results: Vec<HypeTrainBulkStatus> = channel_ids
        .iter()
        .map(|channel_id| {
            if let Some(train) = active_map.get(channel_id) {
                HypeTrainBulkStatus {
                    channel_id: channel_id.clone(),
                    is_active: true,
                    level: train.level,
                    is_golden_kappa: train.is_golden_kappa_train.unwrap_or(false),
                }
            } else {
                HypeTrainBulkStatus {
                    channel_id: channel_id.clone(),
                    is_active: false,
                    level: 0,
                    is_golden_kappa: false,
                }
            }
        })
        .collect();

    Ok(results)
}

/// Fetch current Hype Train status for a channel via GQL
/// Uses BulkAllActiveHypeTrainStatusesQuery to check for active train,
/// then GetHypeTrainExecution for progress details
#[tauri::command]
pub async fn get_hype_train_status(
    channel_id: String,
    channel_login: String,
) -> Result<HypeTrainStatus, String> {
    let client = crate::services::http::client().clone();

    // Step 1: Check if channel has active Hype Train using bulk query
    let bulk_request = serde_json::json!({
        "operationName": "BulkAllActiveHypeTrainStatusesQuery",
        "variables": {
            "channelIDs": [channel_id]
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": BULK_HYPE_TRAIN_HASH
            }
        }
    });

    let bulk_response = client
        .post(GQL_URL)
        .headers(create_gql_headers())
        .json(&bulk_request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    if !bulk_response.status().is_success() {
        return Err(format!("HTTP {}", bulk_response.status()));
    }

    let bulk_text = bulk_response
        .text()
        .await
        .map_err(|e| format!("Read error: {}", e))?;
    let bulk_data: BulkHypeTrainResponse =
        serde_json::from_str(&bulk_text).map_err(|e| format!("Parse error: {}", e))?;

    // Check if there's an active train
    let active_train = bulk_data
        .data
        .and_then(|d| d.all_active_hype_train_statuses)
        .and_then(|statuses| statuses.into_iter().find(|s| s.channel.id == channel_id));

    let Some(train_status) = active_train else {
        // No active Hype Train
        return Ok(HypeTrainStatus::default());
    };

    // Step 2: Fetch detailed progress using GetHypeTrainExecution
    let detail_request = serde_json::json!({
        "operationName": "GetHypeTrainExecution",
        "variables": {
            "userLogin": channel_login
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": HYPE_TRAIN_EXECUTION_HASH
            }
        }
    });

    let detail_response = client
        .post(GQL_URL)
        .headers(create_gql_headers())
        .json(&detail_request)
        .send()
        .await;

    // Try to get detailed progress
    let (progress, goal, total, started_at, expires_at) = match detail_response {
        Ok(resp) if resp.status().is_success() => {
            match resp.text().await {
                Ok(text) => {
                    match serde_json::from_str::<HypeTrainExecutionResponse>(&text) {
                        Ok(data) => {
                            // Navigate: user.channel.hypeTrain.execution.progress
                            let execution = data
                                .data
                                .and_then(|d| d.user)
                                .and_then(|u| u.channel)
                                .and_then(|c| c.hype_train)
                                .and_then(|h| h.execution);

                            if let Some(exec) = execution {
                                if let Some(prog) = exec.progress {
                                    // Progress structure (from GQL capture):
                                    // - prog.progression = current points earned within this level
                                    // - prog.goal = total points needed to complete this level
                                    // - prog.total = total contributions across all levels
                                    let current_progress = prog.progression.unwrap_or(0);
                                    let level_goal = prog.goal.unwrap_or(0);
                                    let prog_total = prog.total.unwrap_or(0);
                                    let remaining = level_goal - current_progress;

                                    debug!(
                                        "[HypeTrain] Level {} - Progress: {}/{} (remaining: {}) on {}",
                                        train_status.level,
                                        current_progress,
                                        level_goal,
                                        remaining,
                                        channel_login
                                    );
                                    (
                                        current_progress,
                                        level_goal,
                                        prog_total,
                                        exec.started_at,
                                        exec.expires_at,
                                    )
                                } else {
                                    (0, 0, 0, exec.started_at, exec.expires_at)
                                }
                            } else {
                                (0, 0, 0, None, None)
                            }
                        }
                        Err(e) => {
                            debug!("[HypeTrain] Parse error in detail response: {}", e);
                            (0, 0, 0, None, None)
                        }
                    }
                }
                Err(_) => (0, 0, 0, None, None),
            }
        }
        _ => (0, 0, 0, None, None),
    };

    // Log if we only got level (no progress details)
    if progress == 0 && goal == 0 {
        debug!(
            "[HypeTrain] Level {} active on {} (no progress details available)",
            train_status.level, channel_login
        );
    }

    Ok(HypeTrainStatus {
        is_active: true,
        id: Some(train_status.id),
        level: train_status.level,
        progress,
        goal,
        total,
        started_at,
        expires_at,
        is_level_up: false,
        is_golden_kappa: train_status.is_golden_kappa_train.unwrap_or(false),
    })
}

/// Show `login`'s hype train in this window. One poll per channel serves every
/// surface in every window (see services/hype_train_watch.rs).
#[tauri::command]
pub async fn hype_train_watch(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
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
pub async fn hype_train_unwatch(window: tauri::WebviewWindow, login: String) -> Result<(), String> {
    crate::services::hype_train_watch::unwatch(window.label(), &login);
    Ok(())
}
