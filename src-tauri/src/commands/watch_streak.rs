//! Watch Streak Commands
//!
//! Handles the "Share Watch Streak" feature that allows users to share
//! their watch streak milestone in chat when they've watched 3+ consecutive streams.

use crate::services::drops_auth_service::DropsAuthService;
use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// Use Twitch Android app client ID for GQL operations
const ANDROID_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");
const CLIENT_URL: &str = "https://www.twitch.tv";

// GQL hash for RewardList query
const REWARD_LIST_HASH: &str = "0b1471876d7647993731b9e3c6a13bf304c67fb31d07f06a945d42286ee377c4";

// ============================================================================
// TYPES
// ============================================================================

/// Represents an available watch streak milestone that can be shared
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchStreakMilestone {
    /// Milestone ID (base64 encoded, e.g. "U3RyZWFrQWN0...")
    pub milestone_id: String,
    /// Current consecutive streams watched
    pub streak_count: i32,
    /// Threshold needed to qualify (typically 3)
    pub threshold: i32,
    /// Share status: CAN_SHARE, SHARED, CAN_NOT_SHARE, etc.
    pub share_status: String,
    /// Channel points bonus amount for sharing
    pub copo_bonus: i32,
}

/// Represents the summary of a watch streak returned by the batch query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchStreakSummary {
    /// Current consecutive streams watched
    pub streak_count: i32,
    /// Share status: CAN_SHARE, SHARED, CAN_NOT_SHARE, etc.
    pub share_status: String,
}

// ============================================================================
// GQL RESPONSE TYPES (RewardList query)
// ============================================================================

#[derive(Debug, Deserialize)]
struct WatchStreakResponse {
    data: Option<WatchStreakData>,
}

#[derive(Debug, Deserialize)]
struct WatchStreakData {
    channel: Option<WatchStreakChannel>,
}

#[derive(Debug, Deserialize)]
struct WatchStreakChannel {
    #[serde(rename = "self")]
    self_connection: Option<WatchStreakSelfConnection>,
}

#[derive(Debug, Deserialize)]
struct WatchStreakSelfConnection {
    #[serde(rename = "watchStreakMilestone")]
    watch_streak_milestone: Option<GqlWatchStreakMilestoneOuter>,
}

#[derive(Debug, Deserialize)]
struct GqlWatchStreakMilestoneOuter {
    #[serde(rename = "watchStreakMilestone")]
    inner_milestone: GqlWatchStreakMilestoneInner,
    #[serde(rename = "watchStreakThreshold")]
    watch_streak_threshold: i32,
    #[serde(rename = "watchStreakCopoBonus")]
    watch_streak_copo_bonus: i32,
}

#[derive(Debug, Deserialize)]
struct GqlWatchStreakMilestoneInner {
    id: String,
    value: String,
    #[serde(rename = "shareStatus")]
    share_status: String,
}

// ============================================================================
// GQL RESPONSE TYPES (ShareMilestone mutation)
// ============================================================================

#[derive(Debug, Deserialize)]
struct ShareMilestoneResponse {
    data: Option<ShareMilestoneData>,
}

#[derive(Debug, Deserialize)]
struct ShareMilestoneData {
    #[serde(rename = "shareViewerMilestone")]
    share_viewer_milestone: Option<ShareMilestonePayload>,
}

#[derive(Debug, Deserialize)]
struct ShareMilestonePayload {
    error: Option<ShareMilestoneError>,
}

#[derive(Debug, Deserialize)]
struct ShareMilestoneError {
    code: Option<String>,
}

// ============================================================================
// HELPERS
// ============================================================================

const WEB_CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");

/// Create headers for GQL requests using the Web Client ID
fn create_web_gql_headers(token: &str) -> HeaderMap {
    let device_id = Uuid::new_v4().to_string().replace("-", "");
    let session_id = Uuid::new_v4().to_string().replace("-", "");

    let mut headers = HeaderMap::new();
    headers.insert("Client-ID", HeaderValue::from_static(WEB_CLIENT_ID));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("OAuth {}", token)).unwrap(),
    );
    headers.insert("Origin", HeaderValue::from_static(CLIENT_URL));
    headers.insert("Referer", HeaderValue::from_static(CLIENT_URL));
    headers.insert("X-Device-Id", HeaderValue::from_str(&device_id).unwrap());
    headers.insert(
        "Client-Session-Id",
        HeaderValue::from_str(&session_id).unwrap(),
    );
    headers
}

/// Create headers for GQL requests using the Android Client ID
fn create_gql_headers(token: &str) -> HeaderMap {
    let device_id = Uuid::new_v4().to_string().replace("-", "");
    let session_id = Uuid::new_v4().to_string().replace("-", "");

    let mut headers = HeaderMap::new();
    headers.insert("Client-ID", HeaderValue::from_static(ANDROID_CLIENT_ID));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    headers.insert("Accept-Encoding", HeaderValue::from_static("gzip"));
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("OAuth {}", token)).unwrap(),
    );
    headers.insert("Origin", HeaderValue::from_static(CLIENT_URL));
    headers.insert("Referer", HeaderValue::from_static(CLIENT_URL));
    headers.insert("X-Device-Id", HeaderValue::from_str(&device_id).unwrap());
    headers.insert(
        "Client-Session-Id",
        HeaderValue::from_str(&session_id).unwrap(),
    );
    headers
}

// ============================================================================
// COMMANDS
// ============================================================================

/// Check if the user has a shareable watch streak milestone for a channel.
/// Returns the milestone info if share_status == "CAN_SHARE", or null otherwise.
#[tauri::command]
pub async fn get_watch_streak(channel_id: String) -> Result<Option<WatchStreakMilestone>, String> {
    debug!(
        "[WatchStreak] Checking watch streak for channel: {}",
        channel_id
    );

    // Get auth token
    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get auth token: {}", e))?;

    let client = crate::services::http::client().clone();

    // Build GQL request for RewardList
    let request_body = serde_json::json!({
        "operationName": "RewardList",
        "variables": {
            "channelID": channel_id,
            "shouldIncludeAllSuspendedStreaks": false
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": REWARD_LIST_HASH
            }
        }
    });

    let response = client
        .post("https://gql.twitch.tv/gql")
        .headers(create_web_gql_headers(&token))
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("GQL request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GQL returned HTTP {}", response.status()));
    }

    let response_text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Parse response
    let parsed: WatchStreakResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    // Extract watch streak milestone if present
    let milestone = parsed
        .data
        .and_then(|d| d.channel)
        .and_then(|c| c.self_connection)
        .and_then(|s| s.watch_streak_milestone);

    match milestone {
        Some(m) => {
            let streak_val = m.inner_milestone.value.parse::<i32>().unwrap_or(0);

            debug!(
                "[WatchStreak] Found milestone: streak={}, threshold={}, status={}, bonus={}",
                streak_val,
                m.watch_streak_threshold,
                m.inner_milestone.share_status,
                m.watch_streak_copo_bonus
            );

            // Only return if the user can actually share
            if m.inner_milestone.share_status == "CAN_SHARE" {
                Ok(Some(WatchStreakMilestone {
                    milestone_id: m.inner_milestone.id,
                    streak_count: streak_val,
                    threshold: m.watch_streak_threshold,
                    share_status: m.inner_milestone.share_status,
                    copo_bonus: m.watch_streak_copo_bonus,
                }))
            } else {
                debug!(
                    "[WatchStreak] Streak exists but status is '{}', not shareable",
                    m.inner_milestone.share_status
                );
                Ok(None)
            }
        }
        None => {
            debug!("[WatchStreak] No watch streak milestone available for this channel");
            Ok(None)
        }
    }
}

/// Share (send) the watch streak milestone to chat.
/// Uses the ShareMilestone mutation with an inline GQL document (no persisted hash).
/// Returns true if successful, false otherwise.
#[tauri::command]
pub async fn share_watch_streak(
    channel_id: String,
    milestone_id: String,
    message: Option<String>,
) -> Result<bool, String> {
    debug!(
        "[WatchStreak] Sharing watch streak for channel: {}, milestone: {}, message: {:?}",
        channel_id, milestone_id, message
    );

    // Get auth token
    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get auth token: {}", e))?;

    let client = crate::services::http::client().clone();

    // Build the input object
    let mut input = serde_json::json!({
        "milestoneID": milestone_id,
        "channelID": channel_id
    });

    // Add optional message body
    if let Some(msg) = message {
        if !msg.is_empty() {
            input
                .as_object_mut()
                .unwrap()
                .insert("messageBody".to_string(), serde_json::Value::String(msg));
        }
    }

    // Build GQL request for ShareMilestone mutation
    let request_body = serde_json::json!({
        "operationName": "ShareMilestone",
        "variables": {
            "input": input
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": "25d20e60945d10123e8d466e30f21a1f1f578dfdea52c72095030b118eda9f39"
            }
        }
    });

    let response = client
        .post("https://gql.twitch.tv/gql")
        .headers(create_web_gql_headers(&token))
        .json(&request_body)
        .send()
        .await
        .map_err(|e| format!("GQL request failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("GQL returned HTTP {}", response.status()));
    }

    let response_text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    // Parse response
    let parsed: ShareMilestoneResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    // Check for errors in the mutation response
    let has_error = parsed
        .data
        .and_then(|d| d.share_viewer_milestone)
        .and_then(|p| p.error)
        .and_then(|e| e.code);

    match has_error {
        Some(code) => {
            debug!("[WatchStreak] Share failed with error code: {}", code);
            Ok(false)
        }
        None => {
            debug!("[WatchStreak] Watch streak shared successfully!");
            Ok(true)
        }
    }
}

/// Fetch watch streaks for multiple channels in a single batched GQL request.
/// Returns a map of channel_id -> WatchStreakSummary
#[tauri::command]
pub async fn get_watch_streaks_batch(
    channel_ids: Vec<String>,
) -> Result<HashMap<String, WatchStreakSummary>, String> {
    if channel_ids.is_empty() {
        return Ok(HashMap::new());
    }

    debug!(
        "[WatchStreak] Fetching batched watch streaks for {} channels",
        channel_ids.len()
    );

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get auth token: {}", e))?;

    let client = crate::services::http::client().clone();

    // Max 35 operations per batch per Twitch GQL limits.
    // However, if the frontend sends more than 35, we'll process them in chunks.
    let mut results_map = HashMap::new();

    for chunk in channel_ids.chunks(35) {
        let mut request_body = Vec::new();

        for channel_id in chunk {
            request_body.push(serde_json::json!({
                "operationName": "RewardList",
                "variables": {
                    "channelID": channel_id,
                    "shouldIncludeAllSuspendedStreaks": false
                },
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": REWARD_LIST_HASH
                    }
                }
            }));
        }

        let response = client
            .post("https://gql.twitch.tv/gql")
            .headers(create_web_gql_headers(&token))
            .json(&request_body)
            .send()
            .await
            .map_err(|e| format!("GQL request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("GQL returned HTTP {}", response.status()));
        }

        let response_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {}", e))?;

        debug!("[WatchStreak] Raw batch response: {}", response_text);

        // A batched query returns an array of response objects
        let parsed: Vec<WatchStreakResponse> = serde_json::from_str(&response_text)
            .map_err(|e| format!("Failed to parse response: {}", e))?;

        // Map results back to channel IDs. Since GQL batching guarantees the response
        // order matches the request array order, we can zip them.
        for (idx, res) in parsed.into_iter().enumerate() {
            let channel_id = &chunk[idx];

            let milestone = res
                .data
                .and_then(|d| d.channel)
                .and_then(|c| c.self_connection)
                .and_then(|s| s.watch_streak_milestone);

            if let Some(m) = milestone {
                // If it exists and is greater than 0, we track it
                let streak_val = m.inner_milestone.value.parse::<i32>().unwrap_or(0);
                if streak_val > 0 {
                    results_map.insert(
                        channel_id.clone(),
                        WatchStreakSummary {
                            streak_count: streak_val,
                            share_status: m.inner_milestone.share_status,
                        },
                    );
                }
            }
        }
    }

    Ok(results_map)
}
