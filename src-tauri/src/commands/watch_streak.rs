//! Watch Streak Commands
//!
//! Handles the "Share Watch Streak" feature that allows users to share
//! their watch streak milestone in chat when they've watched 3+ consecutive streams.

use crate::services::drops_auth_service::DropsAuthService;
use log::{debug, info, warn};
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
    /// GraphQL's own errors. Twitch answers a refused share (unauthenticated, a failed integrity
    /// check) with HTTP 200, these, and a null `shareViewerMilestone`.
    #[serde(default)]
    errors: Vec<GqlError>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    #[serde(default)]
    message: String,
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

            // What Twitch offers in this room, in the core log: a missing banner is then either
            // Twitch's answer or our bug, never a guess.
            info!(
                "[WatchStreak] offer: channel={} streak={} threshold={} status={} bonus={}",
                channel_id,
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
                Ok(None)
            }
        }
        // No milestone: Twitch has none to offer, or the read itself was refused (GraphQL errors
        // come back with HTTP 200 and an empty viewer edge, which reads the same).
        None if response_text.contains("\"errors\"") => {
            warn!("[WatchStreak] offer check failed: channel={} answer={}", channel_id, response_text);
            Ok(None)
        }
        None => {
            info!("[WatchStreak] offer: channel={} none", channel_id);
            Ok(None)
        }
    }
}

/// What a ShareMilestone answer means: `Ok(true)` only when Twitch returned the share's payload
/// with no error of any kind; `Ok(false)` when Twitch refused it with a code; `Err` with Twitch's
/// words when GraphQL failed it outright. Anything less than a clean payload is not a share.
fn share_outcome(response_text: &str) -> Result<bool, String> {
    let parsed: ShareMilestoneResponse =
        serde_json::from_str(response_text).map_err(|e| format!("Failed to parse response: {}", e))?;
    if !parsed.errors.is_empty() {
        let words: Vec<&str> = parsed.errors.iter().map(|e| e.message.as_str()).collect();
        return Err(format!("Twitch refused the share: {}", words.join("; ")));
    }
    let Some(payload) = parsed.data.and_then(|d| d.share_viewer_milestone) else {
        return Err("Twitch answered the share with no result".to_string());
    };
    Ok(payload.error.and_then(|e| e.code).is_none())
}

/// Share (send) the watch streak milestone to chat.
/// Uses the ShareMilestone mutation (persisted hash) with the Android client, the client the
/// drops token belongs to: the same pairing MakePrediction and the poll vote use, since Twitch
/// holds web-client mutations to an integrity check this app cannot pass. Returns true only when
/// Twitch confirms the share.
#[tauri::command]
pub async fn share_watch_streak(
    channel_id: String,
    milestone_id: String,
    message: Option<String>,
) -> Result<bool, String> {
    info!(
        "[WatchStreak] share start: channel={} milestone={} with_message={}",
        channel_id,
        milestone_id,
        message.as_deref().is_some_and(|m| !m.is_empty())
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
        .headers(create_gql_headers(&token))
        .json(&request_body)
        .send()
        .await
        .map_err(|e| {
            warn!("[WatchStreak] share failed: channel={} request error: {}", channel_id, e);
            format!("GQL request failed: {}", e)
        })?;

    if !response.status().is_success() {
        warn!("[WatchStreak] share failed: channel={} HTTP {}", channel_id, response.status());
        return Err(format!("GQL returned HTTP {}", response.status()));
    }

    let response_text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    match share_outcome(&response_text) {
        Ok(true) => {
            info!("[WatchStreak] share sent: channel={} milestone={}", channel_id, milestone_id);
            Ok(true)
        }
        Ok(false) => {
            warn!("[WatchStreak] share refused: channel={} answer={}", channel_id, response_text);
            Ok(false)
        }
        Err(why) => {
            warn!("[WatchStreak] share failed: channel={} {} answer={}", channel_id, why, response_text);
            Err(why)
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

#[cfg(test)]
mod share_tests {
    use super::share_outcome;

    #[test]
    fn a_clean_payload_is_a_share() {
        assert_eq!(share_outcome(r#"{"data":{"shareViewerMilestone":{"error":null}}}"#), Ok(true));
    }

    #[test]
    fn a_refusal_code_is_not_a_share() {
        assert_eq!(
            share_outcome(r#"{"data":{"shareViewerMilestone":{"error":{"code":"MILESTONE_ALREADY_SHARED"}}}}"#),
            Ok(false)
        );
    }

    /// Twitch's real answer to an unauthenticated share (captured anonymously, fake ids): HTTP 200,
    /// a GraphQL error and a null payload. It used to count as shared.
    #[test]
    fn graphql_errors_with_a_null_payload_are_not_a_share() {
        let answer = r#"{"errors":[{"message":"unauthenticated","path":["shareViewerMilestone"],"extensions":{"code":"Unauthenticated"}}],"data":{"shareViewerMilestone":null},"extensions":{"durationMilliseconds":6,"operationName":"ShareMilestone"}}"#;
        let outcome = share_outcome(answer);
        assert!(matches!(&outcome, Err(why) if why.contains("unauthenticated")), "{outcome:?}");
    }

    #[test]
    fn no_payload_at_all_is_not_a_share() {
        assert!(share_outcome(r#"{"data":{"shareViewerMilestone":null}}"#).is_err());
        assert!(share_outcome(r#"{"data":null}"#).is_err());
    }
}
