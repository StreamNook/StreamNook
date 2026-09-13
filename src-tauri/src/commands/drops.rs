// HTTP_CLIENT below kept as a thin file-local alias for the shared client so
// existing `HTTP_CLIENT.clone()` call sites continue to work; the underlying
// instance is the global default in `crate::services::http`.
lazy_static::lazy_static! { static ref HTTP_CLIENT: reqwest::Client = crate::services::http::client().clone(); }

use crate::models::drops::*;
use crate::models::settings::AppState;
use crate::services::drops_auth_service::{DropsAuthService, DropsDeviceCodeInfo};
use log::{debug, error, warn};
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub async fn get_drops_settings(state: State<'_, AppState>) -> Result<DropsSettings, String> {
    let drops_service = state.drops_service.lock().await;
    Ok(drops_service.get_settings().await)
}

#[tauri::command]
pub async fn update_drops_settings(
    settings: DropsSettings,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // Update the in-memory drops service settings
    let drops_service = state.drops_service.lock().await;
    drops_service.update_settings(settings.clone()).await;
    drop(drops_service); // Release the lock before accessing main settings

    // Also persist to the main settings file so changes survive app restart
    {
        let mut app_settings = state.settings.lock().map_err(|e| e.to_string())?;
        app_settings.drops = settings.clone();
    }

    // Save to disk through the shared (debounced) settings writer; the state
    // update above is what the flusher snapshots.
    let settings_to_save = {
        let app_settings = state.settings.lock().map_err(|e| e.to_string())?;
        app_settings.clone()
    };
    crate::commands::settings::write_settings_to_disk(&settings_to_save)?;

    // Keep the realtime points socket in line with the automation master toggle so
    // the plugin's background earns start (or stop) producing channel-points
    // notifications immediately, without waiting for a relaunch. No-op when the
    // flag is unchanged.
    state
        .background_service
        .lock()
        .await
        .set_automation_active(settings.auto_claim_channel_points)
        .await;

    Ok(())
}

#[tauri::command]
pub async fn get_active_drop_campaigns(
    state: State<'_, AppState>,
) -> Result<Vec<DropCampaign>, String> {
    let drops_service = state.drops_service.lock().await;
    // Use cached version to avoid excessive API calls when UI opens
    drops_service
        .get_all_active_campaigns_cached()
        .await
        .map_err(|e| e.to_string())
}

/// Re-fetch active campaigns to pick up a fresh `is_account_connected` after the user connects
/// their account, bypassing the 5-minute cache. Deliberately does NOT run the progress-map sync
/// (unlike `get_active_drop_campaigns`), so a connection refresh can't snap live automation
/// progress backward; it only re-primes the campaign cache so later opens stay consistent.
#[tauri::command]
pub async fn refresh_drops_connection_status(
    state: State<'_, AppState>,
) -> Result<Vec<DropCampaign>, String> {
    let drops_service = state.drops_service.lock().await;
    let campaigns = drops_service
        .fetch_all_active_campaigns_from_api()
        .await
        .map_err(|e| e.to_string())?;
    drops_service.prime_campaign_cache(&campaigns).await;
    Ok(campaigns)
}

#[tauri::command]
pub async fn get_drops_inventory(state: State<'_, AppState>) -> Result<InventoryResponse, String> {
    let drops_service = state.drops_service.lock().await;
    drops_service
        .fetch_inventory()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_drop_progress(state: State<'_, AppState>) -> Result<Vec<DropProgress>, String> {
    let drops_service = state.drops_service.lock().await;
    Ok(drops_service.get_drop_progress().await)
}

/// Which of these channels are currently offering the campaign, straight from
/// Twitch. The channel picker uses it so an allow-listed streamer is offered
/// when Twitch says the drop is earnable there and hidden when it isn't.
#[tauri::command]
pub async fn get_campaign_eligible_channels(
    campaign_id: String,
    channel_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let drops_service = state.drops_service.lock().await;
    drops_service
        .campaign_eligible_channels(&campaign_id, &channel_ids)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn claim_drop(
    drop_id: String,
    drop_instance_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let drops_service = state.drops_service.lock().await;
    drops_service
        .claim_drop(&drop_id, drop_instance_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_channel_points(
    channel_id: String,
    channel_name: String,
    state: State<'_, AppState>,
) -> Result<Option<ChannelPointsClaim>, String> {
    let drops_service = state.drops_service.lock().await;
    drops_service
        .check_channel_points(&channel_id, &channel_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn claim_channel_points(
    channel_id: String,
    channel_name: String,
    claim_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BonusClaimResult, String> {
    let result = {
        let drops_service = state.drops_service.lock().await;
        drops_service
            .claim_channel_points(&channel_id, &channel_name, &claim_id)
            .await
            .map_err(|e| e.to_string())?
    };

    // Surface the watched channel's claim as a channel-points notification on the
    // path that actually delivers — a successful GQL claim. The legacy PubSub
    // points-earned push (channel_points_websocket_service) is dead since Twitch
    // decommissioned pubsub-edge, so this re-emits the same event shape the socket
    // used. DynamicIsland's listener (gated by show_channel_points_notifications)
    // and the lifetime/history accumulator in background_service then behave
    // exactly as they did when the socket worked. Background-collected channels are
    // covered separately by the GQL balance poll in background_service.
    if result.points_earned > 0 {
        let _ = app.emit(
            "channel-points-earned",
            serde_json::json!({
                "channel_id": channel_id,
                "channel_login": channel_name,
                "channel_display_name": channel_name,
                "points": result.points_earned,
                "reason": "claim",
                "balance": result.new_balance,
            }),
        );
    }

    Ok(result)
}

#[tauri::command]
pub async fn get_drops_statistics(state: State<'_, AppState>) -> Result<DropsStatistics, String> {
    let drops_service = state.drops_service.lock().await;
    Ok(drops_service.get_statistics().await)
}

#[tauri::command]
pub async fn get_claimed_drops(state: State<'_, AppState>) -> Result<Vec<ClaimedDrop>, String> {
    let drops_service = state.drops_service.lock().await;
    Ok(drops_service.get_claimed_drops().await)
}

#[tauri::command]
pub async fn get_channel_points_history(
    state: State<'_, AppState>,
) -> Result<Vec<ChannelPointsClaim>, String> {
    let drops_service = state.drops_service.lock().await;
    Ok(drops_service.get_channel_points_history().await)
}

#[tauri::command]
pub async fn get_channel_points_balance(
    channel_id: String,
    state: State<'_, AppState>,
) -> Result<Option<ChannelPointsBalance>, String> {
    let drops_service = state.drops_service.lock().await;
    Ok(drops_service.get_channel_points_balance(&channel_id).await)
}

#[tauri::command]
pub async fn get_all_channel_points_balances(
    state: State<'_, AppState>,
) -> Result<Vec<ChannelPointsBalance>, String> {
    let drops_service = state.drops_service.lock().await;
    Ok(drops_service.get_all_channel_points_balances().await)
}

/// Record the live balance the chat widget reads when you open a channel, so
/// the leaderboard and points accolades have an accurate figure immediately
/// instead of waiting for the realtime socket's first watch-time earn.
#[tauri::command]
pub async fn record_channel_points_balance(
    channel_id: String,
    channel_name: String,
    balance: i32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let drops_service = state.drops_service.lock().await;
    drops_service
        .update_channel_points_balance(&channel_id, &channel_name, balance)
        .await;
    Ok(())
}

lazy_static::lazy_static! {
    static ref LAST_BALANCES_REFRESH: std::sync::Mutex<Option<std::time::Instant>> =
        std::sync::Mutex::new(None);
}

/// Query the channel-points balance of EVERY followed channel (live or not) and
/// populate the shared store, so the leaderboard and the points accolades show
/// your full holdings without opening each channel one by one. Balance is
/// account state queried by login, so nothing has to be live. Throttled to once
/// per 30s unless `force`, to absorb rapid remounts. Returns the full store.
#[tauri::command]
pub async fn refresh_followed_channel_points(
    force: Option<bool>,
    state: State<'_, AppState>,
) -> Result<Vec<ChannelPointsBalance>, String> {
    use crate::services::twitch_service::TwitchService;
    use serde_json::json;

    const REFRESH_THROTTLE: std::time::Duration = std::time::Duration::from_secs(30);
    if !force.unwrap_or(false) {
        let recent = LAST_BALANCES_REFRESH
            .lock()
            .ok()
            .and_then(|g| *g)
            .map(|last| last.elapsed() < REFRESH_THROTTLE)
            .unwrap_or(false);
        if recent {
            let drops_service = state.drops_service.lock().await;
            return Ok(drops_service.get_all_channel_points_balances().await);
        }
    }

    const CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");
    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;
    let client = HTTP_CLIENT.clone();

    // Drain the full followed list (live or offline): (login, channel_id).
    let mut channels: Vec<(String, String)> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        match TwitchService::get_all_followed_channels(100, cursor.clone()).await {
            Ok((page, next)) => {
                for s in page {
                    if !s.user_login.is_empty() && !s.user_id.is_empty() {
                        channels.push((s.user_login, s.user_id));
                    }
                }
                match next {
                    Some(c) => cursor = Some(c),
                    None => break,
                }
            }
            Err(e) => return Err(format!("Failed to list followed channels: {}", e)),
        }
    }

    // The same direct ChannelPointsContext query get_channel_points_for_channel
    // uses (inline, not the stale persisted hash), batched 35 ops per request
    // (Twitch GQL's per-batch ceiling).
    const QUERY: &str = r#"
    query ChannelPointsContext($channelLogin: String!) {
        user(login: $channelLogin) {
            channel {
                self {
                    communityPoints {
                        balance
                    }
                }
            }
        }
    }
    "#;

    // Collect off-lock so the network walk never stalls chat/automation, which also
    // hold the drops service mutex.
    let mut found: Vec<(String, String, i32)> = Vec::new(); // (channel_id, login, balance)
    for chunk in channels.chunks(crate::services::twitch_limits::GQL_MAX_BATCHED_OPERATIONS) {
        let body: Vec<serde_json::Value> = chunk
            .iter()
            .map(|(login, _id)| {
                json!({
                    "operationName": "ChannelPointsContext",
                    "query": QUERY,
                    "variables": { "channelLogin": login.to_lowercase() }
                })
            })
            .collect();

        let resp = match client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", CLIENT_ID)
            .header("Authorization", format!("OAuth {}", token))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("[ChannelPoints] balance batch failed: {}", e);
                continue;
            }
        };

        // Status first: an over-cap batch returns 400 with valid JSON, so
        // json() succeeds and the failure would pass unlogged.
        let status = resp.status();
        let parsed: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                error!("[ChannelPoints] balance batch parse failed: {}", e);
                continue;
            }
        };

        if !status.is_success() {
            error!(
                "[ChannelPoints] balance batch HTTP {} for {} operations (cap is {}): {}",
                status,
                chunk.len(),
                crate::services::twitch_limits::GQL_MAX_BATCHED_OPERATIONS,
                parsed
            );
            continue;
        }

        // Results map to requests BY INDEX, sound only while lengths agree;
        // a short array would misattribute balances.
        let Some(arr) = parsed.as_array() else {
            error!(
                "[ChannelPoints] balance batch returned a non-array body, skipping {} channels: {}",
                chunk.len(),
                parsed
            );
            continue;
        };
        if arr.len() != chunk.len() {
            error!(
                "[ChannelPoints] balance batch length mismatch (sent {}, got {}); \
                 refusing to map positionally",
                chunk.len(),
                arr.len()
            );
            continue;
        }

        for (idx, item) in arr.iter().enumerate() {
            let Some((login, channel_id)) = chunk.get(idx) else {
                continue;
            };
            if let Some(bal) = item
                .pointer("/data/user/channel/self/communityPoints/balance")
                .and_then(|v| v.as_i64())
            {
                if bal > 0 {
                    found.push((channel_id.clone(), login.clone(), bal as i32));
                }
            }
        }
    }

    {
        let drops_service = state.drops_service.lock().await;
        for (channel_id, login, balance) in &found {
            drops_service
                .update_channel_points_balance(channel_id, login, *balance)
                .await;
        }
    }

    if let Ok(mut g) = LAST_BALANCES_REFRESH.lock() {
        *g = Some(std::time::Instant::now());
    }

    let drops_service = state.drops_service.lock().await;
    Ok(drops_service.get_all_channel_points_balances().await)
}

/// Shared active-channel chokepoint: points the parity heartbeat at the
/// channel now on screen and forwards the matching stream event to plugins.
/// Every caller of start/update monitoring funnels through here, including
/// the chat hot-swap that tracks the focused tile in the multi-stream grid.
async fn report_active_channel(state: &State<'_, AppState>, channel_id: &str, login: &str) {
    let previous = state
        .watch_heartbeat
        .set_target(channel_id.to_string(), login.to_string())
        .await;
    let kind = match previous.as_deref() {
        None => "start",
        Some(prev) if prev != channel_id => "change",
        _ => return, // same channel re-registered; nothing changed
    };
    let _ = state
        .plugin_host
        .report_stream_event(
            kind,
            Some(channel_id.to_string()),
            Some(login.to_string()),
            None,
        )
        .await;
    // Follow the on-screen channel with the realtime socket so its bonus chest
    // surfaces instantly and its predictions come through.
    state
        .background_service
        .lock()
        .await
        .set_watched_channel(channel_id.to_string(), login.to_string())
        .await;
}

#[tauri::command]
pub async fn start_drops_monitoring(
    channel_id: String,
    channel_name: String,
    state: State<'_, AppState>,
    app_handle: AppHandle,
) -> Result<(), String> {
    {
        let drops_service = state.drops_service.lock().await;
        drops_service
            .start_monitoring(channel_id.clone(), channel_name.clone(), app_handle)
            .await;
    }
    report_active_channel(&state, &channel_id, &channel_name).await;
    Ok(())
}

#[tauri::command]
pub async fn stop_drops_monitoring(state: State<'_, AppState>) -> Result<(), String> {
    {
        let drops_service = state.drops_service.lock().await;
        drops_service.stop_monitoring().await;
    }
    if let Some(prev) = state.watch_heartbeat.clear_target().await {
        let _ = state
            .plugin_host
            .report_stream_event("stop", Some(prev), None, None)
            .await;
    }
    state
        .background_service
        .lock()
        .await
        .clear_watched_channel()
        .await;
    Ok(())
}

#[tauri::command]
pub async fn update_monitoring_channel(
    channel_id: String,
    channel_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let drops_service = state.drops_service.lock().await;
        drops_service
            .update_current_channel(channel_id.clone(), channel_name.clone())
            .await;
    }
    report_active_channel(&state, &channel_id, &channel_name).await;
    Ok(())
}

/// Player playback state for the parity heartbeat: minute-watched events
/// only send while the video is actually playing.
#[tauri::command]
pub async fn report_player_playing(
    playing: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    state.watch_heartbeat.set_playing(playing);
    Ok(())
}

// Drops Authentication commands
#[tauri::command]
pub async fn start_drops_device_flow() -> Result<DropsDeviceCodeInfo, String> {
    DropsAuthService::start_device_flow()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn poll_drops_token(
    app: AppHandle,
    device_code: String,
    interval: u64,
    expires_in: u64,
) -> Result<String, String> {
    let token = DropsAuthService::poll_for_token(&device_code, interval, expires_in)
        .await
        .map_err(|e| e.to_string())?;
    // Dismiss the in-app drops login overlay the instant we have the token, so it
    // doesn't linger after the user authorizes.
    crate::commands::twitch::dismiss_login_overlay(&app, "drops-login");
    Ok(token)
}

#[tauri::command]
pub async fn drops_logout() -> Result<(), String> {
    DropsAuthService::logout().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn is_drops_authenticated() -> Result<bool, String> {
    Ok(DropsAuthService::is_authenticated().await)
}

#[tauri::command]
pub async fn validate_drops_token() -> Result<bool, String> {
    DropsAuthService::validate_token()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_drop_details(app_handle: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    app_handle
        .opener()
        .open_url(url, None::<String>)
        .map_err(|e| format!("Failed to open drops details: {}", e))
}

// Prediction commands
#[tauri::command]
pub async fn place_prediction(
    event_id: String,
    outcome_id: String,
    points: i32,
    channel_id: String,
) -> Result<serde_json::Value, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Use the mobile/Android client ID for GQL queries
    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // Use the MakePrediction GQL mutation
    let mutation = r#"
    mutation MakePrediction($input: MakePredictionInput!) {
        makePrediction(input: $input) {
            prediction {
                id
                points
            }
            error {
                code
            }
        }
    }
    "#;

    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .json(&json!({
            "operationName": "MakePrediction",
            "query": mutation,
            "variables": {
                "input": {
                    "eventID": event_id,
                    "outcomeID": outcome_id,
                    "points": points,
                    "transactionID": uuid::Uuid::new_v4().to_string()
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to send prediction request: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse prediction response: {}", e))?;

    // Check for errors in the response
    if let Some(error) = result["data"]["makePrediction"]["error"].as_object() {
        let error_code = error
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        return Err(format!("Prediction failed: {}", error_code));
    }

    // Log successful prediction
    if let Some(prediction) = result["data"]["makePrediction"]["prediction"].as_object() {
        let pred_id = prediction.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let pred_points = prediction
            .get("points")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        debug!(
            "Prediction placed successfully! ID: {}, Points: {}",
            pred_id, pred_points
        );
    }

    Ok(result)
}

// Poll commands
//
// Twitch has no GQL read for polls (the web client learns them over PubSub), so
// there's no `get_active_poll` counterpart to `get_active_prediction` — the
// PubSub service feeds the overlay. This command is the interactive half: it
// casts a free (base) vote. Auth mirrors `place_prediction`: Android client id +
// OAuth drops token, using the persisted-query hash captured from the web vote.
#[tauri::command]
pub async fn vote_on_poll(
    poll_id: String,
    choice_id: String,
    channel_id: String,
) -> Result<serde_json::Value, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use serde_json::json;

    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");
    // Persisted hash for `ChannelPollContext_VoteInPoll`, captured via
    // scripts/twitch-gql-server.js. If Twitch rotates the query this hash goes
    // stale (PersistedQueryNotFound) — re-capture and update it here.
    const VOTE_IN_POLL_HASH: &str =
        "1280e27b0f3c7ae60b5714bd569771ea50635778473182e6e959e2dcfcc16e3c";

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // The vote's userID must be the drops-authenticated account (the token owner),
    // so resolve it from the token rather than trusting a frontend-supplied id.
    let validate: serde_json::Value = client
        .get("https://id.twitch.tv/oauth2/validate")
        .header("Authorization", format!("OAuth {}", token))
        .send()
        .await
        .map_err(|e| format!("Failed to validate token: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse token validation: {}", e))?;
    let user_id = validate["user_id"]
        .as_str()
        .ok_or("Could not resolve voter id from drops token")?
        .to_string();

    // Fresh 32-char hex idempotency key per vote (matches the web client's voteID).
    let vote_id = uuid::Uuid::new_v4().simple().to_string();

    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .json(&json!({
            "operationName": "ChannelPollContext_VoteInPoll",
            "variables": {
                "input": {
                    "pollID": poll_id,
                    "choiceID": choice_id,
                    "userID": user_id,
                    "voteID": vote_id,
                    // Free/base vote. Channel-points/bits votes would put the
                    // spend here; not supported yet.
                    "tokens": null
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": VOTE_IN_POLL_HASH
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to send vote request: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse vote response: {}", e))?;

    // Surface a GQL-level error (stale hash, bad token) if present.
    if let Some(errors) = result.get("errors").and_then(|v| v.as_array()) {
        if let Some(first) = errors.first() {
            let msg = first
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown GQL error");
            return Err(format!("Vote failed: {}", msg));
        }
    }

    // Surface a payload-level error (e.g. poll locked/closed).
    if let Some(error) = result["data"]["voteInPoll"]["error"].as_object() {
        if !error.is_empty() {
            let code = error
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");
            return Err(format!("Vote rejected: {}", code));
        }
    }

    debug!(
        "Poll vote cast on channel {}: poll {} choice {}",
        channel_id, poll_id, choice_id
    );

    Ok(result)
}

#[tauri::command]
pub async fn get_active_prediction(
    channel_login: String,
) -> Result<Option<serde_json::Value>, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // GQL query to fetch active prediction for a channel
    let query = r#"
    query GetChannelPrediction($login: String!) {
        channel(name: $login) {
            id
            activePredictionEvent {
                id
                status
                title
                predictionWindowSeconds
                createdAt
                lockedAt
                endedAt
                winningOutcome {
                    id
                }
                outcomes {
                    id
                    title
                    color
                    totalPoints
                    totalUsers
                }
            }
        }
    }
    "#;

    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .json(&json!({
            "operationName": "GetChannelPrediction",
            "query": query,
            "variables": {
                "login": channel_login.to_lowercase()
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch prediction: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse prediction response: {}", e))?;

    // Twitch REMOVED `Channel.activePredictionEvent`, so this query now fails
    // validation and the branch below can never be taken. Surface that instead of
    // reporting "no active prediction", which is what made it look like a working
    // late-join path for as long as it did.
    //
    // Nothing depends on it any more: the predictions-channel-v1 topic sends the
    // full event on `event-updated`, which fires on every bet, so an already
    // running prediction reaches the overlay within seconds of subscribing.
    if let Some(errors) = result.get("errors") {
        warn!(
            "[Prediction] get_active_prediction is no longer supported by Twitch              (PubSub is the only source): {}",
            errors
        );
        return Ok(None);
    }

    // Check if there's an active prediction
    if let Some(prediction) = result["data"]["channel"]["activePredictionEvent"].as_object() {
        let channel_id = result["data"]["channel"]["id"].as_str().unwrap_or("");

        // Transform to match the format we use in PredictionOverlay
        let status = prediction
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("ACTIVE");
        let prediction_id = prediction.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let title = prediction
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let prediction_window = prediction
            .get("predictionWindowSeconds")
            .and_then(|v| v.as_i64())
            .unwrap_or(60);
        let created_at = prediction
            .get("createdAt")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let winning_outcome_id = prediction
            .get("winningOutcome")
            .and_then(|v| v.get("id"))
            .and_then(|v| v.as_str());

        // Transform outcomes
        let mut outcomes = Vec::new();
        if let Some(outcomes_array) = prediction.get("outcomes").and_then(|v| v.as_array()) {
            for outcome in outcomes_array {
                outcomes.push(json!({
                    "id": outcome.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                    "title": outcome.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                    "color": outcome.get("color").and_then(|v| v.as_str()).unwrap_or("BLUE"),
                    "total_points": outcome.get("totalPoints").and_then(|v| v.as_i64()).unwrap_or(0),
                    "total_users": outcome.get("totalUsers").and_then(|v| v.as_i64()).unwrap_or(0)
                }));
            }
        }

        debug!(
            "Found active prediction on {}: {} (status: {})",
            channel_login, title, status
        );

        return Ok(Some(json!({
            "channel_id": channel_id,
            "prediction_id": prediction_id,
            "title": title,
            "status": status,
            "outcomes": outcomes,
            "prediction_window_seconds": prediction_window,
            "created_at": created_at,
            "winning_outcome_id": winning_outcome_id
        })));
    }

    Ok(None)
}

#[tauri::command]
pub async fn get_channel_points_for_channel(
    channel_login: String,
) -> Result<serde_json::Value, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Use the web client ID for this read query.
    const CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // Include communityPointsSettings to get custom points name and icon
    let query = r#"
    query ChannelPointsContext($channelLogin: String!) {
        user(login: $channelLogin) {
            id
            login
            displayName
            channel {
                id
                communityPointsSettings {
                    name
                    image {
                        url
                    }
                }
                self {
                    communityPoints {
                        balance
                        availableClaim {
                            id
                        }
                    }
                }
            }
        }
    }
    "#;

    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .json(&json!({
            "operationName": "ChannelPointsContext",
            "query": query,
            "variables": {
                "channelLogin": channel_login.to_lowercase()
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch channel points: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse channel points response: {}", e))?;

    Ok(result)
}

// Channel Points Rewards commands

// Persisted query hash for ChannelPointsContext - captured from Twitch's network traffic
// This should return communityPointsSettings including customRewards
const CHANNEL_POINTS_CONTEXT_HASH: &str =
    "374314de591e69925fce3ddc2bcf085796f56ebb8cad67a0daa3165c03adc345";

// Captured GQL hashes for channel points redemptions
const SEND_HIGHLIGHTED_CHAT_MESSAGE_HASH: &str =
    "bb187d763156dc5c25c6457e1b32da6c5033cb7504854e6d33a8b876d10444b6";
const UNLOCK_RANDOM_SUBSCRIBER_EMOTE_HASH: &str =
    "f548e89966b21d0094f3dc35233232eb6ec76d63e02594c8a494407712a85350";
// Modify single emote operations
#[allow(dead_code)]
const MODIFY_EMOTE_OWNED_EMOTES_HASH: &str =
    "e882551bf6a6abf14a1ec2deac4fe9a0af22f89f863818f7228da98d6b849cb4";
#[allow(dead_code)]
const AVAILABLE_EMOTES_FOR_CHANNEL_HASH: &str =
    "6c45e0ecaa823cc7db3ecdd1502af2223c775bdcfb0f18a3a0ce9a0b7db8ef6c";
const EMOTE_PICKER_USER_SUBSCRIPTION_PRODUCTS_HASH: &str =
    "511bebfb513d0127d24a7fe49aa2b7717306a611e1f4269a93e0cc76e8a65a81";
const UNLOCK_MODIFIED_EMOTE_HASH: &str =
    "30e8cc29b1d6d96809f5e35f5e7a550ae8bf5d26966a9637d919477ffd0bfc52";
// Note: UnlockChosenSubscriberEmote uses inline query (no persisted hash)

/// Parse a custom reward from the GQL response
fn parse_reward(
    reward: &serde_json::Value,
    _is_automatic: bool,
) -> Option<crate::models::drops::ChannelReward> {
    let id = reward["id"].as_str()?.to_string();
    let title = reward["title"].as_str().unwrap_or_default().to_string();
    let cost = reward["cost"].as_i64().unwrap_or(0) as i32;
    let prompt = reward["prompt"].as_str().map(|s| s.to_string());

    // Get image URL - prefer custom image, fallback to default
    let image_url = reward["image"]["url"]
        .as_str()
        .or_else(|| reward["defaultImage"]["url"].as_str())
        .map(|s| s.to_string());

    let background_color = reward["backgroundColor"]
        .as_str()
        .unwrap_or("#9147FF")
        .to_string();
    let is_enabled = reward["isEnabled"].as_bool().unwrap_or(true);
    let is_paused = reward["isPaused"].as_bool().unwrap_or(false);
    let is_in_stock = reward["isInStock"].as_bool().unwrap_or(true);
    let is_user_input_required = reward["isUserInputRequired"].as_bool().unwrap_or(false);
    let cooldown_expires_at = reward["cooldownExpiresAt"].as_str().map(|s| s.to_string());

    // Parse max per stream setting
    let max_per_stream = if reward["maxPerStreamSetting"]["isEnabled"]
        .as_bool()
        .unwrap_or(false)
    {
        reward["maxPerStreamSetting"]["maxPerStream"]
            .as_i64()
            .map(|v| v as i32)
    } else {
        None
    };

    // Parse max per user per stream setting
    let max_per_user_per_stream = if reward["maxPerUserPerStreamSetting"]["isEnabled"]
        .as_bool()
        .unwrap_or(false)
    {
        reward["maxPerUserPerStreamSetting"]["maxPerUserPerStream"]
            .as_i64()
            .map(|v| v as i32)
    } else {
        None
    };

    // Parse global cooldown setting
    let global_cooldown_seconds = if reward["globalCooldownSetting"]["isEnabled"]
        .as_bool()
        .unwrap_or(false)
    {
        reward["globalCooldownSetting"]["globalCooldownSeconds"]
            .as_i64()
            .map(|v| v as i32)
    } else {
        None
    };

    Some(crate::models::drops::ChannelReward {
        id,
        title,
        cost,
        prompt,
        image_url,
        background_color,
        is_enabled,
        is_paused,
        is_in_stock,
        is_user_input_required,
        cooldown_expires_at,
        max_per_stream,
        max_per_user_per_stream,
        global_cooldown_seconds,
    })
}

/// Parse an automatic (built-in) reward from the GQL response
fn parse_automatic_reward(
    reward: &serde_json::Value,
) -> Option<crate::models::drops::ChannelReward> {
    let id = reward["id"].as_str()?.to_string();
    let id_fallback = id.clone();
    let reward_type = reward["type"].as_str().unwrap_or(&id_fallback);
    let pricing_type = reward["pricingType"].as_str().unwrap_or("CHANNEL_POINTS");

    // Skip Bits-based rewards using the pricingType field
    if pricing_type == "BITS" {
        return None;
    }

    // Convert type to human-readable title for Channel Points rewards only
    let title = match reward_type {
        "SEND_HIGHLIGHTED_MESSAGE" => "Highlight My Message".to_string(),
        "SINGLE_MESSAGE_BYPASS_SUB_MODE" => "Send a Message in Sub-Only Mode".to_string(),
        "RANDOM_SUB_EMOTE_UNLOCK" => "Unlock a Random Sub Emote".to_string(),
        "CHOSEN_SUB_EMOTE_UNLOCK" => "Choose an Emote to Unlock".to_string(),
        "CHOSEN_MODIFIED_SUB_EMOTE_UNLOCK" => "Modify a Single Emote".to_string(),
        "SEND_GIGANTIFIED_EMOTE" => "Gigantify an Emote".to_string(),
        _ => reward_type.replace('_', " ").to_string(),
    };

    // Cost lookup for automatic rewards: streamer's override `cost` if set, else
    // `defaultCost` (the current tiered price the official Twitch web client sends in
    // the redeem mutation). `minimumCost` is just the floor — sending it instead of
    // defaultCost causes a server-side REWARD_COST_MISMATCH. Verified against captured
    // RedeemCustomReward/Unlock* mutations on 2026-05-18.
    let cost = reward["cost"]
        .as_i64()
        .or_else(|| reward["defaultCost"].as_i64())
        .unwrap_or(0) as i32;

    // Skip rewards with 0 cost (they might be event-based or subscription only)
    if cost == 0 {
        return None;
    }

    let is_enabled = reward["isEnabled"].as_bool().unwrap_or(true);

    // Skip disabled rewards only (don't filter isHiddenForSubs - show them anyway)
    if !is_enabled {
        return None;
    }

    // Get background color - prefer custom, fallback to default
    let background_color = reward["backgroundColor"]
        .as_str()
        .or_else(|| reward["defaultBackgroundColor"].as_str())
        .unwrap_or("#9147FF")
        .to_string();

    // Get image URL - prefer custom image, fallback to default (use url2x for better quality)
    let image_url = reward["image"]["url2x"]
        .as_str()
        .or_else(|| reward["image"]["url"].as_str())
        .or_else(|| reward["defaultImage"]["url2x"].as_str())
        .or_else(|| reward["defaultImage"]["url"].as_str())
        .map(|s| s.to_string());

    Some(crate::models::drops::ChannelReward {
        id,
        title,
        cost,
        prompt: None,
        image_url,
        background_color,
        is_enabled,
        is_paused: false,
        is_in_stock: reward["isInStock"].as_bool().unwrap_or(true),
        is_user_input_required: reward_type == "SEND_HIGHLIGHTED_MESSAGE"
            || reward_type == "SINGLE_MESSAGE_BYPASS_SUB_MODE",
        cooldown_expires_at: None,
        max_per_stream: None,
        max_per_user_per_stream: None,
        global_cooldown_seconds: reward["globalCooldownSeconds"].as_i64().map(|v| v as i32),
    })
}

#[tauri::command]
pub async fn get_channel_rewards(
    channel_id: String, // Actually channel login (username)
) -> Result<Vec<crate::models::drops::ChannelReward>, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Use mobile client ID for persisted queries (same as predictions)
    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // Use ChannelPointsContext with includeGoalTypes (captured from Twitch)
    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .json(&json!({
            "operationName": "ChannelPointsContext",
            "variables": {
                "channelLogin": channel_id.to_lowercase(),
                "includeGoalTypes": ["CREATOR", "BOOST"]
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": CHANNEL_POINTS_CONTEXT_HASH
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch channel rewards: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse channel rewards response: {}", e))?;

    debug!(
        "[ChannelRewards] Raw response: {}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );

    // Parse the response into ChannelReward structs
    let mut rewards = Vec::new();

    // Get the communityPointsSettings object
    let settings = result["data"]["community"]["channel"]["communityPointsSettings"]
        .as_object()
        .or_else(|| result["data"]["channel"]["communityPointsSettings"].as_object());

    if let Some(settings) = settings {
        // Parse custom rewards (streamer-defined)
        if let Some(custom_rewards) = settings.get("customRewards").and_then(|v| v.as_array()) {
            for reward in custom_rewards {
                if let Some(parsed) = parse_reward(reward, false) {
                    rewards.push(parsed);
                }
            }
        }

        // Parse automatic rewards (built-in Twitch rewards like Highlight Message)
        if let Some(auto_rewards) = settings.get("automaticRewards").and_then(|v| v.as_array()) {
            for reward in auto_rewards {
                if let Some(parsed) = parse_automatic_reward(reward) {
                    rewards.push(parsed);
                }
            }
        }
    }

    // Sort rewards by cost (cheapest first)
    rewards.sort_by(|a, b| a.cost.cmp(&b.cost));

    Ok(rewards)
}

#[tauri::command]
pub async fn redeem_channel_reward(
    channel_id: String,
    reward_id: String,
    cost: i32,
    title: String,
    prompt: Option<String>,
) -> Result<crate::models::drops::RedemptionResult, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Mirrors send_highlighted_message: Android client ID + Android-flow token + dashless IDs.
    // Persisted query hash captured from the official Twitch web client via scripts/twitch-gql-server.js
    // (operation renamed server-side from RedeemCommunityPointsCustomReward -> RedeemCustomReward,
    // and `pricingType` + `prompt` are now required in the input).
    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");
    const REDEEM_CUSTOM_REWARD_HASH: &str =
        "d56249a7adb4978898ea3412e196688d4ac3cea1c0c2dfd65561d229ea5dcc42";

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    let transaction_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let device_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let session_id = uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string();

    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .header("X-Device-Id", &device_id)
        .header("Client-Session-Id", &session_id)
        .json(&json!({
            "operationName": "RedeemCustomReward",
            "variables": {
                "input": {
                    "channelID": channel_id,
                    "cost": cost,
                    "pricingType": "POINTS",
                    "prompt": prompt.unwrap_or_default(),
                    "rewardID": reward_id,
                    "title": title,
                    "transactionID": transaction_id,
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": REDEEM_CUSTOM_REWARD_HASH
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to redeem reward: {}", e))?;

    let status = response.status();
    let response_text = response
        .text()
        .await
        .map_err(|e| format!("Failed to read redemption response body: {}", e))?;

    log::info!(
        "redeem_channel_reward response (status={}, channel={}, reward={}): {}",
        status,
        channel_id,
        reward_id,
        &response_text[..response_text.len().min(2000)]
    );

    let result: serde_json::Value = match serde_json::from_str(&response_text) {
        Ok(v) => v,
        Err(e) => {
            log::error!(
                "redeem_channel_reward: failed to parse body as JSON: {} | body: {}",
                e,
                &response_text[..response_text.len().min(2000)]
            );
            return Err(format!("Failed to parse redemption response: {}", e));
        }
    };

    // Surface top-level GraphQL errors (e.g. PersistedQueryNotFound, integrity errors)
    if let Some(errors) = result["errors"].as_array() {
        if !errors.is_empty() {
            let msg = errors[0]["message"]
                .as_str()
                .unwrap_or("unknown GraphQL error");
            let code = errors[0]["extensions"]["code"].as_str().unwrap_or("");
            log::error!(
                "redeem_channel_reward GraphQL errors: {}",
                serde_json::to_string(&errors).unwrap_or_default()
            );
            return Ok(crate::models::drops::RedemptionResult {
                success: false,
                error_code: Some(if code.is_empty() {
                    "GRAPHQL_ERROR".to_string()
                } else {
                    code.to_string()
                }),
                error_message: Some(format!("Twitch GQL error: {}", msg)),
                new_balance: None,
                unlocked_emote: None,
            });
        }
    }

    // Twitch's new RedeemCustomReward payload shape is just { error, __typename } — no
    // `redemption` object on success. `error: null` is the success indicator; non-null is failure.
    let payload = &result["data"]["redeemCommunityPointsCustomReward"];
    if payload.is_object() {
        if let Some(error) = payload["error"].as_object() {
            let error_code = error
                .get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("UNKNOWN");
            return Ok(crate::models::drops::RedemptionResult {
                success: false,
                error_code: Some(error_code.to_string()),
                error_message: Some(match error_code {
                    "INSUFFICIENT_POINTS" => "Not enough channel points".to_string(),
                    "NOT_AVAILABLE" => "Reward is not available".to_string(),
                    "MAX_PER_STREAM_EXCEEDED" => {
                        "Maximum redemptions per stream reached".to_string()
                    }
                    "MAX_PER_USER_PER_STREAM_EXCEEDED" => {
                        "You've already redeemed this reward".to_string()
                    }
                    "COOLDOWN" => "Reward is on cooldown".to_string(),
                    "PROPERTIES_MISMATCH" => "Reward changed — refresh and try again".to_string(),
                    _ => format!("Redemption failed: {}", error_code),
                }),
                new_balance: None,
                unlocked_emote: None,
            });
        }

        // error is explicitly null (or missing) on the payload → success
        debug!(
            "Successfully redeemed reward {} for {} points on channel {}",
            reward_id, cost, channel_id
        );
        return Ok(crate::models::drops::RedemptionResult {
            success: true,
            error_code: None,
            error_message: None,
            new_balance: None,
            unlocked_emote: None,
        });
    }

    // Unexpected response
    log::error!(
        "redeem_channel_reward unexpected shape (status={}, channel={}, reward={}): {}",
        status,
        channel_id,
        reward_id,
        serde_json::to_string(&result).unwrap_or_default()
    );
    Ok(crate::models::drops::RedemptionResult {
        success: false,
        error_code: Some("UNKNOWN".to_string()),
        error_message: Some("Unexpected response from Twitch".to_string()),
        new_balance: None,
        unlocked_emote: None,
    })
}

#[tauri::command]
pub async fn send_highlighted_message(
    channel_id: String,
    message: String,
    cost: i32,
) -> Result<crate::models::drops::RedemptionResult, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Use mobile client ID - less strict integrity requirements than web
    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // Generate transaction ID and device/session IDs (UUID without dashes, lowercase)
    let transaction_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let device_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let session_id = uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string();

    // Use the captured persisted query hash with required headers
    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .header("X-Device-Id", &device_id)
        .header("Client-Session-Id", &session_id)
        .json(&json!({
            "operationName": "SendHighlightedChatMessage",
            "variables": {
                "input": {
                    "channelID": channel_id,
                    "cost": cost,
                    "message": message,
                    "transactionID": transaction_id
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": SEND_HIGHLIGHTED_CHAT_MESSAGE_HASH
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to send highlighted message: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    debug!(
        "[SendHighlightedMessage] Response: {}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );

    // Check for errors
    if let Some(errors) = result["errors"].as_array() {
        if !errors.is_empty() {
            let error_msg = errors
                .first()
                .and_then(|e| e["message"].as_str())
                .unwrap_or("Unknown error");
            return Ok(crate::models::drops::RedemptionResult {
                success: false,
                error_code: Some("GQL_ERROR".to_string()),
                error_message: Some(error_msg.to_string()),
                new_balance: None,
                unlocked_emote: None,
            });
        }
    }

    // Check for mutation-specific errors
    if let Some(error) = result["data"]["sendHighlightedChatMessage"]["error"].as_object() {
        let error_code = error
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        return Ok(crate::models::drops::RedemptionResult {
            success: false,
            error_code: Some(error_code.to_string()),
            error_message: Some(match error_code {
                "INSUFFICIENT_POINTS" => "Not enough channel points".to_string(),
                "MESSAGE_TOO_LONG" => "Message is too long".to_string(),
                "RATE_LIMITED" => {
                    "Please wait before sending another highlighted message".to_string()
                }
                _ => format!("Failed to send: {}", error_code),
            }),
            new_balance: None,
            unlocked_emote: None,
        });
    }

    // Check for successful send
    if result["data"]["sendHighlightedChatMessage"].is_object() {
        debug!(
            "Successfully sent highlighted message to channel {} for {} points",
            channel_id, cost
        );

        return Ok(crate::models::drops::RedemptionResult {
            success: true,
            error_code: None,
            error_message: None,
            new_balance: None,
            unlocked_emote: None,
        });
    }

    // Unexpected response
    Ok(crate::models::drops::RedemptionResult {
        success: false,
        error_code: Some("UNKNOWN".to_string()),
        error_message: Some("Unexpected response from Twitch".to_string()),
        new_balance: None,
        unlocked_emote: None,
    })
}

#[tauri::command]
pub async fn unlock_random_emote(
    channel_id: String,
    cost: i32,
) -> Result<crate::models::drops::RedemptionResult, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Use mobile client ID - less strict integrity requirements
    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // Generate IDs
    let transaction_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let device_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let session_id = uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string();

    // Use the captured persisted query hash
    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .header("X-Device-Id", &device_id)
        .header("Client-Session-Id", &session_id)
        .json(&json!({
            "operationName": "UnlockRandomSubscriberEmote",
            "variables": {
                "input": {
                    "channelID": channel_id,
                    "cost": cost,
                    "transactionID": transaction_id
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": UNLOCK_RANDOM_SUBSCRIBER_EMOTE_HASH
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to unlock emote: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    debug!(
        "[UnlockRandomEmote] Response: {}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );

    // Check for errors
    if let Some(errors) = result["errors"].as_array() {
        if !errors.is_empty() {
            let error_msg = errors
                .first()
                .and_then(|e| e["message"].as_str())
                .unwrap_or("Unknown error");
            return Ok(crate::models::drops::RedemptionResult {
                success: false,
                error_code: Some("GQL_ERROR".to_string()),
                error_message: Some(error_msg.to_string()),
                new_balance: None,
                unlocked_emote: None,
            });
        }
    }

    // Check for mutation-specific errors
    if let Some(error) = result["data"]["unlockRandomSubscriberEmote"]["error"].as_object() {
        let error_code = error
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        return Ok(crate::models::drops::RedemptionResult {
            success: false,
            error_code: Some(error_code.to_string()),
            error_message: Some(match error_code {
                "INSUFFICIENT_POINTS" => "Not enough channel points".to_string(),
                "NOT_AVAILABLE" => "Emote unlock not available".to_string(),
                "ALREADY_UNLOCKED" => "You've already unlocked all emotes".to_string(),
                _ => format!("Failed to unlock: {}", error_code),
            }),
            new_balance: None,
            unlocked_emote: None,
        });
    }

    // Check for successful unlock - parse the emote info for the reveal popup
    if let Some(unlock_data) = result["data"]["unlockRandomSubscriberEmote"].as_object() {
        // Try to extract the unlocked emote info
        let unlocked_emote = unlock_data
            .get("unlockedEmote")
            .or_else(|| unlock_data.get("emote"))
            .and_then(|emote| {
                let id = emote["id"].as_str()?.to_string();
                let name = emote["token"]
                    .as_str()
                    .or_else(|| emote["name"].as_str())
                    .unwrap_or("Unknown")
                    .to_string();
                // Build emote image URL (Twitch CDN format)
                let image_url = emote["images"]["url"]
                    .as_str()
                    .or_else(|| emote["images"]["url2x"].as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        format!(
                            "https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/2.0",
                            id
                        )
                    });
                Some(crate::models::drops::UnlockedEmote {
                    id,
                    name,
                    image_url,
                })
            });

        debug!(
            "Successfully unlocked random emote on channel {} for {} points: {:?}",
            channel_id, cost, unlocked_emote
        );

        return Ok(crate::models::drops::RedemptionResult {
            success: true,
            error_code: None,
            error_message: None,
            new_balance: None,
            unlocked_emote,
        });
    }

    Ok(crate::models::drops::RedemptionResult {
        success: false,
        error_code: Some("UNKNOWN".to_string()),
        error_message: Some("Unexpected response from Twitch".to_string()),
        new_balance: None,
        unlocked_emote: None,
    })
}

/// Represents an available modification for an emote
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EmoteModification {
    pub id: String,          // The full modified emote ID (e.g., "1022569_BW")
    pub modifier_id: String, // The modifier type (e.g., "MOD_BW")
    pub token: String,       // The modified emote name (e.g., "hamzSleep_BW")
}

/// Represents an emote that can be modified via channel points
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModifiableEmote {
    pub id: String,
    pub token: String, // The emote name/code
    pub emote_type: Option<String>,
    pub modifications: Vec<EmoteModification>, // Available modifications
}

/// Get the list of emotes that can be modified for a channel
#[tauri::command]
pub async fn get_modifiable_emotes(channel_id: String) -> Result<Vec<ModifiableEmote>, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Use mobile Android client ID
    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // Generate required headers
    let device_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let session_id = uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string();

    // Use ChannelPointsContext to get emote variants with modifications
    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .header("X-Device-Id", &device_id)
        .header("Client-Session-Id", &session_id)
        .json(&json!({
            "operationName": "ChannelPointsContext",
            "variables": {
                "channelLogin": channel_id
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": CHANNEL_POINTS_CONTEXT_HASH
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch modifiable emotes: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    debug!(
        "[ChannelPointsContext] Fetching emote variants for channel {}",
        channel_id
    );

    // Parse the emotes from the response
    // Response structure: data.community.channel.communityPointsSettings.emoteVariants[]
    let mut emotes = Vec::new();

    if let Some(variants) = result
        .get("data")
        .and_then(|d| d.get("community"))
        .and_then(|c| c.get("channel"))
        .and_then(|ch| ch.get("communityPointsSettings"))
        .and_then(|s| s.get("emoteVariants"))
        .and_then(|e| e.as_array())
    {
        for variant in variants {
            // Check if this emote is unlockable
            let is_unlockable = variant
                .get("isUnlockable")
                .and_then(|u| u.as_bool())
                .unwrap_or(false);

            if !is_unlockable {
                continue;
            }

            // Get base emote info
            let base_emote = variant.get("emote");
            if base_emote.is_none() {
                continue;
            }
            let base_emote = base_emote.unwrap();

            let id = base_emote.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let token = base_emote
                .get("token")
                .and_then(|t| t.as_str())
                .unwrap_or(id);

            if id.is_empty() {
                continue;
            }

            // Parse modifications
            let mut modifications = Vec::new();
            if let Some(mods) = variant.get("modifications").and_then(|m| m.as_array()) {
                for modification in mods {
                    let mod_emote = modification.get("emote");
                    let modifier = modification.get("modifier");

                    if let (Some(mod_emote), Some(modifier)) = (mod_emote, modifier) {
                        let mod_id = mod_emote.get("id").and_then(|i| i.as_str()).unwrap_or("");
                        let mod_token = mod_emote
                            .get("token")
                            .and_then(|t| t.as_str())
                            .unwrap_or(mod_id);
                        let modifier_id = modifier.get("id").and_then(|i| i.as_str()).unwrap_or("");

                        if !mod_id.is_empty() {
                            modifications.push(EmoteModification {
                                id: mod_id.to_string(),
                                modifier_id: modifier_id.to_string(),
                                token: mod_token.to_string(),
                            });
                        }
                    }
                }
            }

            emotes.push(ModifiableEmote {
                id: id.to_string(),
                token: token.to_string(),
                emote_type: Some("SUBSCRIPTION".to_string()),
                modifications,
            });
        }
    }

    debug!(
        "[ChannelPointsContext] Found {} unlockable emotes for channel {}",
        emotes.len(),
        channel_id
    );
    Ok(emotes)
}

/// Unlock/modify a specific emote using channel points
#[tauri::command]
pub async fn unlock_modified_emote(
    channel_id: String,
    emote_id: String,
    cost: i32,
) -> Result<crate::models::drops::RedemptionResult, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Use mobile Android client ID
    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // Generate required headers and transaction ID
    let device_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let session_id = uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string();
    let transaction_id = uuid::Uuid::new_v4().to_string().replace("-", "");

    debug!(
        "[UnlockModifiedEmote] Unlocking emote {} on channel {} for {} points (txn: {})",
        emote_id, channel_id, cost, transaction_id
    );

    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .header("X-Device-Id", &device_id)
        .header("Client-Session-Id", &session_id)
        .json(&json!({
            "operationName": "UnlockModifiedEmote",
            "variables": {
                "input": {
                    "channelID": channel_id,
                    "emoteID": emote_id,
                    "cost": cost,
                    "transactionID": transaction_id
                }
            },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": UNLOCK_MODIFIED_EMOTE_HASH
                }
            }
        }))
        .send()
        .await
        .map_err(|e| format!("Failed to unlock emote: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    debug!(
        "[UnlockModifiedEmote] Response: {}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );

    // Check for GQL errors
    if let Some(errors) = result.get("errors").and_then(|e| e.as_array()) {
        if !errors.is_empty() {
            let error_msg = errors[0]
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return Ok(crate::models::drops::RedemptionResult {
                success: false,
                error_code: Some("GQL_ERROR".to_string()),
                error_message: Some(error_msg.to_string()),
                new_balance: None,
                unlocked_emote: None,
            });
        }
    }

    // Check for mutation-specific errors (response uses unlockChosenModifiedSubscriberEmote)
    if let Some(error) = result["data"]["unlockChosenModifiedSubscriberEmote"]["error"].as_object()
    {
        let error_code = error
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        return Ok(crate::models::drops::RedemptionResult {
            success: error_code == "EMOTE_ALREADY_ENTITLED", // Already owned counts as success
            error_code: Some(error_code.to_string()),
            error_message: Some(match error_code {
                "INSUFFICIENT_POINTS" => "Not enough channel points".to_string(),
                "NOT_AVAILABLE" => "This reward is not available".to_string(),
                "COOLDOWN" => "Reward is on cooldown".to_string(),
                "EMOTE_ALREADY_ENTITLED" => "You already own this modified emote!".to_string(),
                _ => format!("Failed to modify: {}", error_code),
            }),
            new_balance: result["data"]["unlockChosenModifiedSubscriberEmote"]["balance"]
                .as_i64()
                .map(|b| b as i32),
            unlocked_emote: None,
        });
    }

    // Check for success - error should be null and balance should be present
    let payload = &result["data"]["unlockChosenModifiedSubscriberEmote"];
    if payload.is_object() && payload["error"].is_null() {
        let new_balance = payload["balance"].as_i64().map(|b| b as i32);

        // Construct the unlocked emote from the emote_id we sent
        // Token is the emote code (e.g., hamzPoo_SQ)
        let emote_token = emote_id
            .replace("_", " ")
            .split(' ')
            .next()
            .unwrap_or(&emote_id);
        let unlocked_emote = Some(crate::models::drops::UnlockedEmote {
            id: emote_id.clone(),
            name: emote_id.clone(), // We don't have the token, use ID
            image_url: format!(
                "https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/2.0",
                emote_id
            ),
        });

        debug!(
            "Successfully modified emote {} on channel {} - New balance: {:?}",
            emote_id, channel_id, new_balance
        );

        return Ok(crate::models::drops::RedemptionResult {
            success: true,
            error_code: None,
            error_message: None,
            new_balance,
            unlocked_emote,
        });
    }

    // Unexpected response
    Ok(crate::models::drops::RedemptionResult {
        success: false,
        error_code: Some("UNKNOWN".to_string()),
        error_message: Some("Unexpected response from Twitch".to_string()),
        new_balance: None,
        unlocked_emote: None,
    })
}

/// Unlock a specific chosen emote using channel points (not random)
#[tauri::command]
pub async fn unlock_chosen_emote(
    channel_id: String,
    emote_id: String,
    cost: i32,
) -> Result<crate::models::drops::RedemptionResult, String> {
    use crate::services::drops_auth_service::DropsAuthService;
    use reqwest::Client;
    use serde_json::json;

    // Use mobile Android client ID
    const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

    let token = DropsAuthService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    let client = HTTP_CLIENT.clone();

    // Generate required headers and transaction ID
    let device_id = uuid::Uuid::new_v4().to_string().replace("-", "");
    let session_id = uuid::Uuid::new_v4().to_string().replace("-", "")[..16].to_string();
    let transaction_id = uuid::Uuid::new_v4().to_string().replace("-", "");

    debug!(
        "[UnlockChosenEmote] Unlocking emote {} on channel {} for {} points (txn: {})",
        emote_id, channel_id, cost, transaction_id
    );

    let payload = json!({
        "operationName": "UnlockChosenSubscriberEmote",
        "query": r#"mutation UnlockChosenSubscriberEmote($input: UnlockChosenSubscriberEmoteInput!) {
  unlockChosenSubscriberEmote(input: $input) {
    balance
    error {
      code
      __typename
    }
    __typename
  }
}"#,
        "variables": {
            "input": {
                "channelID": channel_id,
                "emoteID": emote_id,
                "cost": cost,
                "transactionID": transaction_id
            }
        }
    });

    debug!(
        "[UnlockChosenEmote] Sending payload: {}",
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    );

    let response = client
        .post("https://gql.twitch.tv/gql")
        .header("Client-Id", CLIENT_ID)
        .header("Authorization", format!("OAuth {}", token))
        .header("X-Device-Id", &device_id)
        .header("Client-Session-Id", &session_id)
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("Failed to unlock emote: {}", e))?;

    let result: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    debug!(
        "[UnlockChosenEmote] Response: {}",
        serde_json::to_string_pretty(&result).unwrap_or_default()
    );

    // Check for GQL errors
    if let Some(errors) = result.get("errors").and_then(|e| e.as_array()) {
        if !errors.is_empty() {
            let error_msg = errors[0]
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error");
            return Ok(crate::models::drops::RedemptionResult {
                success: false,
                error_code: Some("GQL_ERROR".to_string()),
                error_message: Some(error_msg.to_string()),
                new_balance: None,
                unlocked_emote: None,
            });
        }
    }

    // Check for mutation-specific errors
    if let Some(error) = result["data"]["unlockChosenSubscriberEmote"]["error"].as_object() {
        let error_code = error
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        return Ok(crate::models::drops::RedemptionResult {
            success: error_code == "EMOTE_ALREADY_ENTITLED", // Already owned counts as success
            error_code: Some(error_code.to_string()),
            error_message: Some(match error_code {
                "INSUFFICIENT_POINTS" => "Not enough channel points".to_string(),
                "NOT_AVAILABLE" => "This reward is not available".to_string(),
                "COOLDOWN" => "Reward is on cooldown".to_string(),
                "EMOTE_ALREADY_ENTITLED" => "You already own this emote!".to_string(),
                _ => format!("Failed to unlock: {}", error_code),
            }),
            new_balance: result["data"]["unlockChosenSubscriberEmote"]["balance"]
                .as_i64()
                .map(|b| b as i32),
            unlocked_emote: None,
        });
    }

    // Check for success - error should be null and balance should be present
    let payload = &result["data"]["unlockChosenSubscriberEmote"];
    if payload.is_object() && payload["error"].is_null() {
        let new_balance = payload["balance"].as_i64().map(|b| b as i32);

        // Construct the unlocked emote from the emote_id we sent
        // (the response doesn't include emote info)
        let unlocked_emote = Some(crate::models::drops::UnlockedEmote {
            id: emote_id.clone(),
            name: emote_id.clone(), // We'll use the token from frontend
            image_url: format!(
                "https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/2.0",
                emote_id
            ),
        });

        debug!(
            "Successfully unlocked emote {} on channel {} - New balance: {:?}",
            emote_id, channel_id, new_balance
        );

        return Ok(crate::models::drops::RedemptionResult {
            success: true,
            error_code: None,
            error_message: None,
            new_balance,
            unlocked_emote,
        });
    }

    // Unexpected response
    Ok(crate::models::drops::RedemptionResult {
        success: false,
        error_code: Some("UNKNOWN".to_string()),
        error_message: Some("Unexpected response from Twitch".to_string()),
        new_balance: None,
        unlocked_emote: None,
    })
}
