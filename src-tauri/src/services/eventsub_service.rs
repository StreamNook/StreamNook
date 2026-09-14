use crate::services::twitch_service::TwitchService;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const SUBSCRIPTION_DELAY_MS: u64 = 500; // Delay between subscription requests

#[derive(Debug, Deserialize)]
struct EventSubMessage {
    metadata: MessageMetadata,
    payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct MessageMetadata {
    message_id: String,
    message_type: String,
    message_timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subscription_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subscription_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SessionPayload {
    session: Session,
}

#[derive(Debug, Deserialize)]
struct Session {
    id: String,
    status: String,
    connected_at: String,
    keepalive_timeout_seconds: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reconnect_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NotificationPayload {
    subscription: Subscription,
    event: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Subscription {
    id: String,
    #[serde(rename = "type")]
    subscription_type: String,
    version: String,
    status: String,
    cost: i32,
    condition: serde_json::Value,
    transport: Transport,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct Transport {
    method: String,
    session_id: String,
}

// Response types for subscription list
#[derive(Debug, Deserialize)]
struct SubscriptionListResponse {
    data: Vec<SubscriptionInfo>,
    total: i32,
    total_cost: i32,
    max_total_cost: i32,
}

#[derive(Debug, Deserialize)]
struct SubscriptionInfo {
    id: String,
    #[serde(rename = "type")]
    subscription_type: String,
    status: String,
    transport: TransportInfo,
}

#[derive(Debug, Deserialize)]
struct TransportInfo {
    method: String,
    session_id: Option<String>,
}

// Event structures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RaidEvent {
    pub from_broadcaster_user_id: String,
    pub from_broadcaster_user_login: String,
    pub from_broadcaster_user_name: String,
    pub to_broadcaster_user_id: String,
    pub to_broadcaster_user_login: String,
    pub to_broadcaster_user_name: String,
    pub viewers: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOfflineEvent {
    pub broadcaster_user_id: String,
    pub broadcaster_user_login: String,
    pub broadcaster_user_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOnlineEvent {
    pub id: String,
    pub broadcaster_user_id: String,
    pub broadcaster_user_login: String,
    pub broadcaster_user_name: String,
    #[serde(rename = "type")]
    pub stream_type: String,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelUpdateEvent {
    pub broadcaster_user_id: String,
    pub broadcaster_user_login: String,
    pub broadcaster_user_name: String,
    pub title: String,
    pub language: String,
    pub category_id: String,
    pub category_name: String,
    pub content_classification_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPointsRedemptionEvent {
    pub id: String,
    pub broadcaster_user_id: String,
    pub broadcaster_user_name: String,
    pub user_id: String,
    pub user_name: String,
    pub user_input: String,
    pub status: String,
    pub reward: RewardInfo,
    pub redeemed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardInfo {
    pub id: String,
    pub title: String,
    pub cost: i32,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperReceivedEvent {
    pub from_user_id: String,
    pub from_user_login: String,
    pub from_user_name: String,
    pub to_user_id: String,
    pub to_user_login: String,
    pub to_user_name: String,
    pub whisper_id: String,
    pub whisper: WhisperContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperContent {
    pub text: String,
}

// Hype Train Event Structures (V2)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypeTrainContributor {
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    #[serde(rename = "type")]
    pub contribution_type: String, // "bits" | "subscription" | "other"
    pub total: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypeTrainBeginEvent {
    pub id: String,
    pub broadcaster_user_id: String,
    pub broadcaster_user_login: String,
    pub broadcaster_user_name: String,
    pub total: i32,
    pub progress: i32,
    pub goal: i32,
    pub top_contributions: Vec<HypeTrainContributor>,
    pub last_contribution: HypeTrainContributor,
    pub level: i32,
    pub started_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypeTrainProgressEvent {
    pub id: String,
    pub broadcaster_user_id: String,
    pub broadcaster_user_login: String,
    pub broadcaster_user_name: String,
    pub level: i32,
    pub total: i32,
    pub progress: i32,
    pub goal: i32,
    pub top_contributions: Vec<HypeTrainContributor>,
    pub last_contribution: HypeTrainContributor,
    pub started_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HypeTrainEndEvent {
    pub id: String,
    pub broadcaster_user_id: String,
    pub broadcaster_user_login: String,
    pub broadcaster_user_name: String,
    pub level: i32,
    pub total: i32,
    pub top_contributions: Vec<HypeTrainContributor>,
    pub started_at: String,
    pub ended_at: String,
    pub cooldown_ends_at: String,
}

pub struct EventSubService {
    connected: Arc<RwLock<bool>>,
    session_id: Arc<RwLock<Option<String>>>,
    subscriptions: Arc<RwLock<Vec<String>>>,
    // Extra channels (besides the main stream) the user moderates and currently
    // has open in MultiNook. We subscribe channel.moderate for each so the mod
    // view lights up there too. This SET is the desired state, re-applied on every
    // (re)connect by subscribe_to_events. `mod_sub_ids` maps broadcaster_id -> the
    // live subscription id so we can tear a single one down between reconnects.
    mod_channels: Arc<RwLock<HashSet<String>>>,
    mod_sub_ids: Arc<RwLock<HashMap<String, String>>>,
    // Shutdown signal sender - when dropped or sent, the background task will stop
    shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
}

impl EventSubService {
    pub fn new() -> Self {
        Self {
            connected: Arc::new(RwLock::new(false)),
            session_id: Arc::new(RwLock::new(None)),
            subscriptions: Arc::new(RwLock::new(Vec::new())),
            mod_channels: Arc::new(RwLock::new(HashSet::new())),
            mod_sub_ids: Arc::new(RwLock::new(HashMap::new())),
            shutdown_tx: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    pub async fn get_session_id(&self) -> Option<String> {
        self.session_id.read().await.clone()
    }

    /// Clean up any existing WebSocket subscriptions from Twitch API
    async fn cleanup_existing_subscriptions() {
        debug!("Cleaning up existing EventSub subscriptions...");

        let token = match TwitchService::get_token().await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to get token for cleanup: {}", e);
                return;
            }
        };

        let client_id = env!("TWITCH_APP_CLIENT_ID");
        let client = crate::services::http::client().clone();

        // Get all existing subscriptions
        let response = match client
            .get("https://api.twitch.tv/helix/eventsub/subscriptions")
            .header("Client-ID", client_id)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to list subscriptions: {}", e);
                return;
            }
        };

        if !response.status().is_success() {
            error!("Failed to list subscriptions: {}", response.status());
            return;
        }

        let subscriptions: SubscriptionListResponse = match response.json().await {
            Ok(s) => s,
            Err(e) => {
                error!("Failed to parse subscription list: {}", e);
                return;
            }
        };

        debug!(
            "Found {} existing subscriptions (cost: {}/{})",
            subscriptions.total, subscriptions.total_cost, subscriptions.max_total_cost
        );

        // Delete ALL WebSocket-based subscriptions to prevent "Too Many Requests" errors
        // This ensures we start fresh with each connection
        let mut deleted = 0;
        for sub in subscriptions.data {
            // Only clean up websocket subscriptions
            if sub.transport.method != "websocket" {
                continue;
            }

            // Delete ALL websocket subscriptions to ensure clean slate
            // This prevents "number of websocket transports limit exceeded" errors
            {
                if let Err(e) = client
                    .delete(format!(
                        "https://api.twitch.tv/helix/eventsub/subscriptions?id={}",
                        sub.id
                    ))
                    .header("Client-ID", client_id)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                {
                    error!("Failed to delete subscription {}: {}", sub.id, e);
                } else {
                    deleted += 1;
                }

                // Small delay between deletions to avoid rate limits
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        if deleted > 0 {
            debug!(" Deleted {} orphaned subscriptions", deleted);
        }
    }

    pub async fn connect_and_listen(
        &self,
        broadcaster_id: String,
        app_handle: AppHandle,
    ) -> Result<()> {
        // First, stop any existing connection
        self.disconnect().await;

        // Clean up orphaned subscriptions from previous sessions
        Self::cleanup_existing_subscriptions().await;

        let connected = self.connected.clone();
        let session_id = self.session_id.clone();
        let subscriptions = self.subscriptions.clone();
        let mod_channels = self.mod_channels.clone();
        let mod_sub_ids = self.mod_sub_ids.clone();

        // Create a shutdown channel
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        // Store the sender so we can signal shutdown later
        {
            let mut tx = self.shutdown_tx.write().await;
            *tx = Some(shutdown_tx);
        }

        tokio::spawn(async move {
            // Run once - no automatic reconnection loop
            // The frontend will handle reconnection when needed
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    debug!("EventSub shutdown signal received");
                }
                result = Self::run_connection(
                    broadcaster_id.clone(),
                    app_handle.clone(),
                    connected.clone(),
                    session_id.clone(),
                    subscriptions.clone(),
                    mod_channels.clone(),
                    mod_sub_ids.clone(),
                ) => {
                    match result {
                        Ok(_) => {
                            debug!("EventSub connection closed normally");
                        }
                        Err(e) => {
                            error!("EventSub connection error: {}", e);
                        }
                    }
                }
            }

            // Mark as disconnected
            {
                let mut conn = connected.write().await;
                *conn = false;
                let mut sess = session_id.write().await;
                *sess = None;
            }

            debug!("EventSub task ended");
        });

        Ok(())
    }

    async fn run_connection(
        broadcaster_id: String,
        app_handle: AppHandle,
        connected: Arc<RwLock<bool>>,
        session_id: Arc<RwLock<Option<String>>>,
        subscriptions: Arc<RwLock<Vec<String>>>,
        mod_channels: Arc<RwLock<HashSet<String>>>,
        mod_sub_ids: Arc<RwLock<HashMap<String, String>>>,
    ) -> Result<()> {
        debug!("Connecting to Twitch EventSub...");

        let (ws_stream, _) = connect_async(EVENTSUB_URL).await?;
        let (mut write, mut read) = ws_stream.split();

        debug!("Connected to Twitch EventSub, waiting for welcome message...");

        // Wait for welcome message
        let welcome_msg = read
            .next()
            .await
            .ok_or_else(|| anyhow::anyhow!("Connection closed before welcome message"))??;

        // Handle different message types and extract text
        let welcome_text = match &welcome_msg {
            Message::Text(text) => {
                debug!("Received text message");
                text.to_string()
            }
            Message::Binary(data) => {
                let text = String::from_utf8(data.to_vec())
                    .map_err(|e| anyhow::anyhow!("Failed to decode binary message: {}", e))?;
                debug!("Received binary message (decoded)");
                text
            }
            Message::Close(frame) => {
                debug!("Received close frame: {:?}", frame);
                return Err(anyhow::anyhow!("Connection closed by server"));
            }
            Message::Ping(_) | Message::Pong(_) => {
                debug!("Received ping/pong, waiting for welcome message...");
                let next_msg = read
                    .next()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Connection closed after ping/pong"))??;
                match &next_msg {
                    Message::Text(text) => text.to_string(),
                    _ => return Err(anyhow::anyhow!("Unexpected message type after ping/pong")),
                }
            }
            Message::Frame(_) => {
                return Err(anyhow::anyhow!(
                    "Received raw frame instead of welcome message"
                ));
            }
        };

        // Parse the welcome message
        let welcome: EventSubMessage = match serde_json::from_str(&welcome_text) {
            Ok(msg) => msg,
            Err(e) => {
                error!("Failed to parse welcome message: {}", e);
                error!("Raw message: {}", welcome_text);
                return Err(anyhow::anyhow!("Failed to parse welcome message: {}", e));
            }
        };

        if welcome.metadata.message_type != "session_welcome" {
            error!(
                "Expected session_welcome, got: {}",
                welcome.metadata.message_type
            );
            return Err(anyhow::anyhow!(
                "Expected session_welcome message, got: {}",
                welcome.metadata.message_type
            ));
        }

        let session_payload: SessionPayload = serde_json::from_value(welcome.payload)?;
        let session_info = session_payload.session;
        let keepalive_timeout = session_info.keepalive_timeout_seconds;

        // Store session ID
        {
            let mut sess = session_id.write().await;
            *sess = Some(session_info.id.clone());
        }

        // Mark as connected
        {
            let mut conn = connected.write().await;
            *conn = true;
        }

        debug!("EventSub session established: {}", session_info.id);
        debug!(" Keepalive timeout: {} seconds", keepalive_timeout);

        // Subscribe to events using the session ID
        let sess_id = session_info.id.clone();
        let app_handle_subs = app_handle.clone();
        tokio::spawn(async move {
            // Wait a moment for the connection to stabilize
            tokio::time::sleep(Duration::from_secs(1)).await;

            if let Err(e) = Self::subscribe_to_events(
                &broadcaster_id,
                &sess_id,
                &app_handle_subs,
                mod_channels,
                mod_sub_ids,
            )
            .await
            {
                error!("Failed to subscribe to EventSub events: {}", e);
            }
        });

        // Start keepalive monitoring
        let last_message_time = Arc::new(RwLock::new(std::time::Instant::now()));
        let last_message_clone = last_message_time.clone();
        let connected_clone = connected.clone();

        tokio::spawn(async move {
            // 5s granularity: the deadline itself is keepalive_timeout + 5 (>=15s),
            // so a 1Hz tick bought nothing but wakeups. Worst added detection
            // latency is one tick.
            let mut check_interval = interval(Duration::from_secs(5));
            loop {
                check_interval.tick().await;

                // Check if we should stop
                if !*connected_clone.read().await {
                    break;
                }

                let elapsed = last_message_clone.read().await.elapsed();
                if elapsed > Duration::from_secs(keepalive_timeout + 5) {
                    error!(
                        "No message received for {} seconds, connection may be dead",
                        elapsed.as_secs()
                    );
                    let mut conn = connected_clone.write().await;
                    *conn = false;
                    break;
                }
            }
        });

        // Listen for messages
        while let Some(msg) = read.next().await {
            // Check if we should stop
            if !*connected.read().await {
                debug!("Connection marked as disconnected, stopping listener");
                break;
            }

            match msg {
                Ok(Message::Text(text)) => {
                    // Update last message time
                    {
                        let mut last_msg = last_message_time.write().await;
                        *last_msg = std::time::Instant::now();
                    }

                    if let Err(e) = Self::handle_message(&text, &app_handle).await {
                        error!("Error handling message: {}", e);
                    }
                }
                Ok(Message::Close(_)) => {
                    debug!("EventSub connection closed by server");
                    break;
                }
                Ok(Message::Ping(data)) => {
                    // Respond to ping with pong
                    if let Err(e) = write.send(Message::Pong(data)).await {
                        error!("Failed to send pong: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        // Send close frame to properly close the connection
        let _ = write.send(Message::Close(None)).await;

        Ok(())
    }

    async fn handle_message(text: &str, app_handle: &AppHandle) -> Result<()> {
        // Handle potential empty messages
        if text.trim().is_empty() {
            debug!("Received empty message, ignoring");
            return Ok(());
        }

        let message: EventSubMessage = match serde_json::from_str(text) {
            Ok(msg) => msg,
            Err(e) => {
                error!("Failed to parse EventSub message: {}", e);
                error!("Raw message: {}", text);
                return Err(anyhow::anyhow!("Failed to parse message: {}", e));
            }
        };

        match message.metadata.message_type.as_str() {
            "session_welcome" => {
                debug!("Received welcome message");
            }
            "session_keepalive" => {
                debug!("Received keepalive");
            }
            "notification" => {
                let notification: NotificationPayload = serde_json::from_value(message.payload)?;
                Self::handle_notification(notification, app_handle).await?;
            }
            "session_reconnect" => {
                debug!("Server requested reconnection");
                let session_payload: SessionPayload = serde_json::from_value(message.payload)?;
                if let Some(reconnect_url) = session_payload.session.reconnect_url {
                    debug!("Reconnect URL: {}", reconnect_url);
                    // TODO: Implement reconnection logic
                }
            }
            "revocation" => {
                debug!(" Subscription revoked");
                let notification: NotificationPayload = serde_json::from_value(message.payload)?;
                debug!("Revoked subscription: {:?}", notification.subscription);
            }
            _ => {
                debug!("Unknown message type: {}", message.metadata.message_type);
            }
        }

        Ok(())
    }

    async fn handle_notification(
        notification: NotificationPayload,
        app_handle: &AppHandle,
    ) -> Result<()> {
        debug!(
            "Received notification: {}",
            notification.subscription.subscription_type
        );

        match notification.subscription.subscription_type.as_str() {
            "channel.raid" => {
                // Handle raid events
                if let Ok(raid_event) =
                    serde_json::from_value::<RaidEvent>(notification.event.clone())
                {
                    debug!(
                        "Raid: {} -> {} ({} viewers)",
                        raid_event.from_broadcaster_user_name,
                        raid_event.to_broadcaster_user_name,
                        raid_event.viewers
                    );
                    let _ = app_handle.emit("eventsub://raid", &raid_event);
                }
            }
            "stream.offline" => {
                // Handle stream offline events
                if let Ok(offline_event) =
                    serde_json::from_value::<StreamOfflineEvent>(notification.event.clone())
                {
                    debug!("Stream offline: {}", offline_event.broadcaster_user_name);
                    let _ = app_handle.emit("eventsub://offline", &offline_event);
                }
            }
            "stream.online" => {
                // Handle stream online events
                if let Ok(online_event) =
                    serde_json::from_value::<StreamOnlineEvent>(notification.event.clone())
                {
                    debug!("Stream online: {}", online_event.broadcaster_user_name);
                    let _ = app_handle.emit("eventsub://online", &online_event);
                }
            }
            "channel.update" => {
                // Handle channel update events
                if let Ok(update_event) =
                    serde_json::from_value::<ChannelUpdateEvent>(notification.event.clone())
                {
                    debug!(
                        "Channel updated: \"{}\" - {}",
                        update_event.title, update_event.category_name
                    );
                    let _ = app_handle.emit("eventsub://channel-update", &update_event);
                }
            }
            "channel.moderate" => {
                // Handle moderation events (pass raw JSON through to frontend)
                debug!(
                    "Moderation action: {}",
                    notification
                        .event
                        .get("action")
                        .and_then(|a| a.as_str())
                        .unwrap_or("unknown")
                );
                let _ = app_handle.emit("eventsub://channel-moderate", &notification.event);
            }
            "channel.channel_points_automatic_reward_redemption.add" => {
                // Handle automatic channel points rewards
                debug!("Automatic channel points reward redeemed");
                let _ = app_handle.emit("channel-points-automatic-reward", &notification.event);
            }
            "channel.channel_points_custom_reward_redemption.add" => {
                // Handle custom channel points reward redemptions
                if let Ok(redemption) = serde_json::from_value::<ChannelPointsRedemptionEvent>(
                    notification.event.clone(),
                ) {
                    debug!(
                        "Custom reward redeemed: {} by {}",
                        redemption.reward.title, redemption.user_name
                    );
                    let _ = app_handle.emit("channel-points-redemption", &redemption);
                }
            }
            "user.whisper.message" => {
                // Handle incoming whisper messages
                if let Ok(whisper_event) =
                    serde_json::from_value::<WhisperReceivedEvent>(notification.event.clone())
                {
                    debug!(
                        "Whisper received from {} (@{}): {}",
                        whisper_event.from_user_name,
                        whisper_event.from_user_login,
                        whisper_event.whisper.text
                    );

                    let whisper_data = serde_json::json!({
                        "from_user_id": whisper_event.from_user_id,
                        "from_user_login": whisper_event.from_user_login,
                        "from_user_name": whisper_event.from_user_name,
                        "to_user_id": whisper_event.to_user_id,
                        "to_user_login": whisper_event.to_user_login,
                        "to_user_name": whisper_event.to_user_name,
                        "whisper_id": whisper_event.whisper_id,
                        "text": whisper_event.whisper.text
                    });

                    let _ = app_handle.emit("whisper-received", &whisper_data);
                }
            }
            "channel.hype_train.begin" => {
                if let Ok(event) =
                    serde_json::from_value::<HypeTrainBeginEvent>(notification.event.clone())
                {
                    debug!(
                        "Hype Train started! Level {} - Goal: {}/{}",
                        event.level, event.progress, event.goal
                    );
                    let _ = app_handle.emit("eventsub://hype-train-begin", &event);
                }
            }
            "channel.hype_train.progress" => {
                if let Ok(event) =
                    serde_json::from_value::<HypeTrainProgressEvent>(notification.event.clone())
                {
                    debug!(
                        "Hype Train progress: Level {} - {}/{}",
                        event.level, event.progress, event.goal
                    );
                    let _ = app_handle.emit("eventsub://hype-train-progress", &event);
                }
            }
            "channel.hype_train.end" => {
                if let Ok(event) =
                    serde_json::from_value::<HypeTrainEndEvent>(notification.event.clone())
                {
                    debug!("Hype Train ended at Level {}!", event.level);
                    let _ = app_handle.emit("eventsub://hype-train-end", &event);
                }
            }
            _ => {
                debug!(
                    "Unhandled subscription type: {}",
                    notification.subscription.subscription_type
                );
            }
        }

        Ok(())
    }

    async fn subscribe_to_events(
        broadcaster_id: &str,
        session_id: &str,
        app_handle: &AppHandle,
        mod_channels: Arc<RwLock<HashSet<String>>>,
        mod_sub_ids: Arc<RwLock<HashMap<String, String>>>,
    ) -> Result<()> {
        let token = TwitchService::get_token().await?;
        let client_id = env!("TWITCH_APP_CLIENT_ID");
        let client = crate::services::http::client().clone();

        // Get the current user's ID for user-specific subscriptions
        let current_user_id = match TwitchService::get_user_info().await {
            Ok(user) => user.id,
            Err(e) => {
                error!("Failed to get current user info: {}", e);
                String::new()
            }
        };

        // List of subscriptions to create
        #[allow(clippy::useless_vec)]
        let subscriptions = vec![
            // Raid events (when streamer raids OUT to another channel)
            (
                "channel.raid",
                "1",
                serde_json::json!({
                    "from_broadcaster_user_id": broadcaster_id
                }),
            ),
            // Stream offline events
            (
                "stream.offline",
                "1",
                serde_json::json!({
                    "broadcaster_user_id": broadcaster_id
                }),
            ),
            // Stream online events
            (
                "stream.online",
                "1",
                serde_json::json!({
                    "broadcaster_user_id": broadcaster_id
                }),
            ),
            // Channel update events (title, category changes)
            (
                "channel.update",
                "2",
                serde_json::json!({
                    "broadcaster_user_id": broadcaster_id
                }),
            ),
            // NOTE: channel.moderate (the "mod view") is intentionally NOT here.
            // It is owned by the dedicated, chat-driven `eventsub_moderation`
            // service (one persistent socket, subscribed per open chat channel via
            // the IRC refcount) so the mod view follows chat, not the watched
            // stream, and works in offline chat / MultiNook / popouts. Keeping it
            // here too would double-subscribe the watched channel (409).
            // Hype Train events (V2)
            (
                "channel.hype_train.begin",
                "2",
                serde_json::json!({
                    "broadcaster_user_id": broadcaster_id
                }),
            ),
            (
                "channel.hype_train.progress",
                "2",
                serde_json::json!({
                    "broadcaster_user_id": broadcaster_id
                }),
            ),
            (
                "channel.hype_train.end",
                "2",
                serde_json::json!({
                    "broadcaster_user_id": broadcaster_id
                }),
            ),
        ];

        // Subscribe to each event with delays
        for (i, (event_type, version, condition)) in subscriptions.iter().enumerate() {
            let subscription_body = serde_json::json!({
                "type": event_type,
                "version": version,
                "condition": condition,
                "transport": {
                    "method": "websocket",
                    "session_id": session_id
                }
            });

            let response = client
                .post("https://api.twitch.tv/helix/eventsub/subscriptions")
                .header("Client-ID", client_id)
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .json(&subscription_body)
                .send()
                .await?;

            if response.status().is_success() {
                debug!("Subscribed to {}", event_type);
            } else {
                let status = response.status();
                let error_text = response.text().await?;

                // Hype Train events require moderator access - silently skip if 403
                if event_type.starts_with("channel.hype_train") && status.as_u16() == 403 {
                    // This is expected when not a moderator of the channel
                    debug!("Skipped {} (requires moderator access)", event_type);
                } else {
                    error!("Failed to subscribe to {}: {}", event_type, error_text);
                    // Surface the failure to the UI so a silently-broken feed
                    // (e.g. channel.moderate missing a scope) isn't invisible.
                    let _ = app_handle.emit(
                        "eventsub://subscription-failed",
                        serde_json::json!({
                            "type": event_type,
                            "status": status.as_u16(),
                            "error": error_text,
                        }),
                    );
                }
            }

            // Add delay between subscriptions (except after the last one)
            if i < subscriptions.len() - 1 {
                tokio::time::sleep(Duration::from_millis(SUBSCRIPTION_DELAY_MS)).await;
            }
        }

        // Subscribe to whisper messages if we have a user ID
        if !current_user_id.is_empty() {
            tokio::time::sleep(Duration::from_millis(SUBSCRIPTION_DELAY_MS)).await;

            let whisper_subscription_body = serde_json::json!({
                "type": "user.whisper.message",
                "version": "1",
                "condition": {
                    "user_id": current_user_id
                },
                "transport": {
                    "method": "websocket",
                    "session_id": session_id
                }
            });

            let response = client
                .post("https://api.twitch.tv/helix/eventsub/subscriptions")
                .header("Client-ID", client_id)
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .json(&whisper_subscription_body)
                .send()
                .await?;

            if response.status().is_success() {
                debug!(
                    "Subscribed to whisper messages for user {}",
                    current_user_id
                );
            } else {
                let error_text = response.text().await?;
                error!("Failed to subscribe to whispers: {}", error_text);
            }
        }

        // Extra channel.moderate subscriptions for other channels the user
        // moderates and currently has open (MultiNook). Re-applied here on every
        // (re)connect. The main broadcaster is already covered above. A 403 just
        // means you don't moderate that channel, so subscribe_channel_moderate
        // skips it silently (no error toast, unlike the main-stream sub).
        if !current_user_id.is_empty() {
            let extras: Vec<String> = mod_channels.read().await.iter().cloned().collect();
            for extra_id in extras {
                if extra_id.is_empty() || extra_id.as_str() == broadcaster_id {
                    continue;
                }
                tokio::time::sleep(Duration::from_millis(SUBSCRIPTION_DELAY_MS)).await;
                if let Some(sub_id) =
                    Self::subscribe_channel_moderate(&extra_id, &current_user_id, session_id).await
                {
                    mod_sub_ids.write().await.insert(extra_id, sub_id);
                }
            }
        }

        Ok(())
    }

    /// Subscribe to channel.moderate v2 for a single broadcaster on the given
    /// session. Returns Some(subscription_id) on success, or None on any failure.
    /// A 403 means you don't moderate that channel (expected), so we stay quiet
    /// and do NOT surface it the way the main-stream sub does.
    async fn subscribe_channel_moderate(
        broadcaster_id: &str,
        moderator_user_id: &str,
        session_id: &str,
    ) -> Option<String> {
        if broadcaster_id.is_empty() || moderator_user_id.is_empty() {
            return None;
        }

        let token = TwitchService::get_token().await.ok()?;
        let client_id = env!("TWITCH_APP_CLIENT_ID");
        let client = crate::services::http::client().clone();

        let body = serde_json::json!({
            "type": "channel.moderate",
            "version": "2",
            "condition": {
                "broadcaster_user_id": broadcaster_id,
                "moderator_user_id": moderator_user_id
            },
            "transport": { "method": "websocket", "session_id": session_id }
        });

        let response = client
            .post("https://api.twitch.tv/helix/eventsub/subscriptions")
            .header("Client-ID", client_id)
            .header("Authorization", format!("Bearer {}", token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .ok()?;

        if response.status().is_success() {
            let json: serde_json::Value = response.json().await.ok()?;
            let id = json
                .get("data")
                .and_then(|d| d.get(0))
                .and_then(|s| s.get("id"))
                .and_then(|i| i.as_str())
                .map(|s| s.to_string());
            debug!(
                "Subscribed to channel.moderate for broadcaster {}",
                broadcaster_id
            );
            id
        } else {
            debug!(
                "Skipped channel.moderate for broadcaster {} (HTTP {})",
                broadcaster_id,
                response.status().as_u16()
            );
            None
        }
    }

    /// Track a channel the user moderates and, if a session is live, subscribe to
    /// its moderation events now. Idempotent (re-adding a tracked channel is a
    /// no-op). Persisted in `mod_channels` so it's re-subscribed on every
    /// reconnect by `subscribe_to_events`.
    pub async fn add_mod_channel(&self, broadcaster_id: String) {
        if broadcaster_id.is_empty() {
            return;
        }
        {
            let mut set = self.mod_channels.write().await;
            if !set.insert(broadcaster_id.clone()) {
                return; // already tracked
            }
        }

        // If a session is live right now, subscribe immediately; otherwise it gets
        // picked up the next time a session connects.
        let session_id = self.session_id.read().await.clone();
        if let Some(sess_id) = session_id {
            let moderator_user_id = TwitchService::get_user_info()
                .await
                .map(|u| u.id)
                .unwrap_or_default();
            if let Some(sub_id) =
                Self::subscribe_channel_moderate(&broadcaster_id, &moderator_user_id, &sess_id)
                    .await
            {
                self.mod_sub_ids
                    .write()
                    .await
                    .insert(broadcaster_id, sub_id);
            }
        }
    }

    /// Stop tracking a moderated channel and tear down its live subscription.
    pub async fn remove_mod_channel(&self, broadcaster_id: String) {
        self.mod_channels.write().await.remove(&broadcaster_id);
        let sub_id = self.mod_sub_ids.write().await.remove(&broadcaster_id);
        if let Some(sub_id) = sub_id {
            if let Ok(token) = TwitchService::get_token().await {
                let client_id = env!("TWITCH_APP_CLIENT_ID");
                let client = crate::services::http::client().clone();
                let _ = client
                    .delete(format!(
                        "https://api.twitch.tv/helix/eventsub/subscriptions?id={}",
                        sub_id
                    ))
                    .header("Client-ID", client_id)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await;
                debug!(
                    "Removed channel.moderate sub for broadcaster {}",
                    broadcaster_id
                );
            }
        }
    }

    pub async fn disconnect(&self) {
        debug!("Disconnecting EventSub...");

        // Mark as disconnected first to stop all loops
        {
            let mut conn = self.connected.write().await;
            *conn = false;
        }

        // Send shutdown signal to the background task
        {
            let mut tx = self.shutdown_tx.write().await;
            if let Some(sender) = tx.take() {
                let _ = sender.send(()).await;
            }
        }

        // Clear session and subscriptions
        {
            let mut sess = self.session_id.write().await;
            *sess = None;
        }

        {
            let mut subs = self.subscriptions.write().await;
            subs.clear();
        }

        // Live moderation sub ids belong to the session that's going away; drop
        // them. The desired channel set in `mod_channels` is intentionally kept so
        // those channels are re-subscribed when a new session connects.
        {
            let mut sub_ids = self.mod_sub_ids.write().await;
            sub_ids.clear();
        }

        // Give the task a moment to clean up
        tokio::time::sleep(Duration::from_millis(100)).await;

        debug!("EventSub disconnected");
    }
}
