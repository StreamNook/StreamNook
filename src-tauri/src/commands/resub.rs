//! Resub Notification Commands
//!
//! Handles the "Share Resub Notification" feature that allows users to share
//! their subscription anniversary in chat with a custom message.

use crate::services::drops_auth_service::DropsAuthService;
use log::debug;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Use Twitch Android app client ID for GQL operations
const ANDROID_CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");
const CLIENT_URL: &str = "https://www.twitch.tv";

// GQL hashes for resub operations
const CHANNEL_DATA_HASH: &str = "beb55e2ecdbae3dd29c51a60597014d526466bc8f94fb88f3c3482110f4da1aa";
const USE_TOKEN_HASH: &str = "61045d4a4bb10d25080bc0a01a74232f1fa67a6a530e0f2ebf05df2f1ba3fa59";

// ============================================================================
// TYPES
// ============================================================================

/// Represents an available resub notification token
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResubNotification {
    /// Base64 encoded token ID (use this for tokenID param)
    pub id: String,
    /// Decoded token string (userID:channelID:months:type)
    pub token: String,
    /// Total months subscribed
    pub cumulative_tenure_months: i32,
    /// Current consecutive streak months
    pub streak_tenure_months: i32,
    /// Months in this specific subscription period
    pub months: i32,
    /// Whether this subscription was a gift
    pub is_gift_subscription: bool,
    /// Display name of the gifter (if applicable)
    pub gifter_display_name: Option<String>,
}

// ============================================================================
// GQL RESPONSE TYPES
// ============================================================================

#[derive(Debug, Deserialize)]
struct ChannelDataResponse {
    data: Option<ChannelDataData>,
}

#[derive(Debug, Deserialize)]
struct ChannelDataData {
    user: Option<ChannelDataUser>,
}

#[derive(Debug, Deserialize)]
struct ChannelDataUser {
    #[serde(rename = "self")]
    self_connection: Option<UserSelfConnection>,
}

#[derive(Debug, Deserialize)]
struct UserSelfConnection {
    #[serde(rename = "resubNotification")]
    resub_notification: Option<GqlResubNotification>,
}

#[derive(Debug, Deserialize)]
struct GqlResubNotification {
    id: String,
    token: String,
    #[serde(rename = "cumulativeTenureMonths")]
    cumulative_tenure_months: i32,
    #[serde(rename = "streakTenureMonths")]
    streak_tenure_months: i32,
    months: i32,
    #[serde(rename = "isGiftSubscription")]
    is_gift_subscription: bool,
    gifter: Option<GqlGifter>,
}

#[derive(Debug, Deserialize)]
struct GqlGifter {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UseTokenResponse {
    data: Option<UseTokenData>,
}

#[derive(Debug, Deserialize)]
struct UseTokenData {
    #[serde(rename = "useChatNotificationToken")]
    use_chat_notification_token: Option<UseTokenPayload>,
}

#[derive(Debug, Deserialize)]
struct UseTokenPayload {
    #[serde(rename = "isSuccess")]
    is_success: bool,
}

// ============================================================================
// HELPERS
// ============================================================================

/// Create headers for GQL requests
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

/// Check if the user has an available resub notification token for a channel.
/// Returns the resub notification info if available, or null if not.
#[tauri::command]
pub async fn get_resub_notification(
    channel_login: String,
) -> Result<Option<ResubNotification>, String> {
    debug!(
        "[Resub] Checking resub notification for channel: {}",
        channel_login
    );

    // Get auth token
    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get auth token: {}", e))?;

    let client = crate::services::http::client().clone();

    // Build GQL request for Chat_ShareResub_ChannelData
    let request_body = serde_json::json!({
        "operationName": "Chat_ShareResub_ChannelData",
        "variables": {
            "channelLogin": channel_login,
            "giftRecipientLogin": "",
            "withStandardGifting": false
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": CHANNEL_DATA_HASH
            }
        }
    });

    let response = client
        .post("https://gql.twitch.tv/gql")
        .headers(create_gql_headers(&token))
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
    let parsed: ChannelDataResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    // Extract resub notification if present
    let resub = parsed
        .data
        .and_then(|d| d.user)
        .and_then(|u| u.self_connection)
        .and_then(|s| s.resub_notification);

    match resub {
        Some(r) => {
            debug!(
                "[Resub] Found resub token! {} months cumulative, {} months streak",
                r.cumulative_tenure_months, r.streak_tenure_months
            );
            Ok(Some(ResubNotification {
                id: r.id,
                token: r.token,
                cumulative_tenure_months: r.cumulative_tenure_months,
                streak_tenure_months: r.streak_tenure_months,
                months: r.months,
                is_gift_subscription: r.is_gift_subscription,
                gifter_display_name: r.gifter.and_then(|g| g.display_name),
            }))
        }
        None => {
            debug!("[Resub] No resub notification available for this channel");
            Ok(None)
        }
    }
}

/// Use the resub token to share the subscription anniversary in chat.
/// Returns true if successful, false otherwise.
#[tauri::command]
pub async fn use_resub_token(
    channel_login: String,
    message: Option<String>,
    include_streak: bool,
    token_id: Option<String>,
) -> Result<bool, String> {
    debug!(
        "[Resub] Using resub token for channel: {}, message: {:?}, includeStreak: {}",
        channel_login, message, include_streak
    );

    // Get auth token
    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get auth token: {}", e))?;

    let client = crate::services::http::client().clone();

    // Build the input object
    let mut input = serde_json::json!({
        "channelLogin": channel_login,
        "includeStreak": include_streak
    });

    // Add optional fields
    if let Some(msg) = message {
        input
            .as_object_mut()
            .unwrap()
            .insert("message".to_string(), serde_json::Value::String(msg));
    }
    if let Some(tid) = token_id {
        input
            .as_object_mut()
            .unwrap()
            .insert("tokenID".to_string(), serde_json::Value::String(tid));
    }

    // Build GQL request for Chat_ShareResub_UseResubToken
    let request_body = serde_json::json!({
        "operationName": "Chat_ShareResub_UseResubToken",
        "variables": {
            "input": input
        },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": USE_TOKEN_HASH
            }
        }
    });

    let response = client
        .post("https://gql.twitch.tv/gql")
        .headers(create_gql_headers(&token))
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
    let parsed: UseTokenResponse = serde_json::from_str(&response_text)
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    // Check if successful
    let is_success = parsed
        .data
        .and_then(|d| d.use_chat_notification_token)
        .map(|p| p.is_success)
        .unwrap_or(false);

    if is_success {
        debug!("[Resub] Resub notification sent successfully!");
    } else {
        debug!("[Resub] Failed to send resub notification");
    }

    Ok(is_success)
}
