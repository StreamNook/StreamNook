use anyhow::Result;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex as TokioMutex, RwLock};
use tokio::task::JoinHandle;
use tokio::time::{interval, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::services::drops_auth_service::DropsAuthService;
use crate::services::twitch_limits::{
    PUBSUB_LISTEN_TOPICS_PER_FRAME, PUBSUB_MAX_TOPICS_PER_CONNECTION,
};

const PUBSUB_URL: &str = "wss://pubsub-edge.twitch.tv";

/// Mapping structure for channel information
#[derive(Debug, Clone)]
pub struct ChannelMapping {
    pub login: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct ChannelPointsWebSocketService {
    connections: Arc<RwLock<Vec<WebSocketConnection>>>,
    auth_token: Arc<RwLock<String>>,
    user_id: Arc<RwLock<String>>,
    app_handle: Option<AppHandle>,
    // Mapping of channel_id to channel info (login and display_name) for resolving channel names in events
    channel_mappings: Arc<RwLock<HashMap<String, ChannelMapping>>>,
    // Set of channel IDs the user is currently watching
    active_viewing_channels: Arc<RwLock<HashSet<String>>>,
    /// JoinHandles for the outer-spawned reader tasks (one per WS connection).
    /// Stored so `disconnect_all` can actually terminate them. Without this,
    /// the recursive reconnect at the end of `handle_connection` keeps the
    /// task alive past disconnect, causing stacked "ghost" connections to
    /// receive the same PubSub events 2-3 times.
    reader_task_handles: Arc<TokioMutex<Vec<JoinHandle<()>>>>,
    /// JoinHandle for the singleton ping-keeper task. Used to avoid spawning
    /// a duplicate keeper on each connect_to_channels call, and to terminate
    /// the keeper on disconnect_all.
    ping_keeper_handle: Arc<TokioMutex<Option<JoinHandle<()>>>>,
}

#[derive(Debug)]
struct WebSocketConnection {
    id: String,
    topics: Vec<String>,
    is_connected: bool,
    last_ping: chrono::DateTime<Utc>,
    last_pong: chrono::DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PubSubMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<PubSubData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PubSubData {
    topic: String,
    message: String,
}

impl ChannelPointsWebSocketService {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(Vec::new())),
            auth_token: Arc::new(RwLock::new(String::new())),
            user_id: Arc::new(RwLock::new(String::new())),
            app_handle: None,
            channel_mappings: Arc::new(RwLock::new(HashMap::new())),
            active_viewing_channels: Arc::new(RwLock::new(HashSet::new())),
            reader_task_handles: Arc::new(TokioMutex::new(Vec::new())),
            ping_keeper_handle: Arc::new(TokioMutex::new(None)),
        }
    }

    /// Register channel ID to login/display_name mapping for resolving channel names in events
    pub async fn register_channel_mapping(
        &self,
        channel_id: &str,
        channel_login: &str,
        channel_display_name: &str,
    ) {
        let mut mapping = self.channel_mappings.write().await;
        mapping.insert(
            channel_id.to_string(),
            ChannelMapping {
                login: channel_login.to_lowercase(),
                display_name: channel_display_name.to_string(),
            },
        );
        debug!(
            "Registered channel mapping: {} -> {} ({})",
            channel_id, channel_login, channel_display_name
        );
    }

    /// Register a channel as currently being watched
    pub async fn register_active_channel(&self, channel_id: &str) {
        let mut active = self.active_viewing_channels.write().await;
        active.insert(channel_id.to_string());
        debug!(
            "Registered active watching channel for predictions: {}",
            channel_id
        );
    }

    /// Unregister a channel that is no longer being watched
    pub async fn unregister_active_channel(&self, channel_id: &str) {
        let mut active = self.active_viewing_channels.write().await;
        active.remove(channel_id);
        debug!(
            "Unregistered active watching channel for predictions: {}",
            channel_id
        );
    }

    /// Get channel login from channel ID
    pub async fn get_channel_login(&self, channel_id: &str) -> Option<String> {
        let mapping = self.channel_mappings.read().await;
        mapping.get(channel_id).map(|m| m.login.clone())
    }

    /// Get channel display name from channel ID
    pub async fn get_channel_display_name(&self, channel_id: &str) -> Option<String> {
        let mapping = self.channel_mappings.read().await;
        mapping.get(channel_id).map(|m| m.display_name.clone())
    }

    /// Get full channel mapping from channel ID
    pub async fn get_channel_mapping(&self, channel_id: &str) -> Option<ChannelMapping> {
        let mapping = self.channel_mappings.read().await;
        mapping.get(channel_id).cloned()
    }

    /// Connect to multiple channels for real-time channel points monitoring
    pub async fn connect_to_channels(
        &mut self,
        channel_ids: Vec<String>,
        user_id: &str,
        auth_token: &str,
        app_handle: AppHandle,
    ) -> Result<()> {
        // Abort any prior session's reader tasks + ping keeper before starting
        // a new one. Without this, repeated connect_to_channels calls stack
        // sockets that all subscribe to the same channels — Twitch then sends
        // each PubSub event N times, one per stacked socket. The user logs
        // showed this firing 3× for the same "+10 points" event.
        self.disconnect_all().await;

        self.app_handle = Some(app_handle.clone());
        *self.auth_token.write().await = auth_token.to_string();
        *self.user_id.write().await = user_id.to_string();

        // 4 topics per channel + 2 global = 42 per connection, under the
        // connection cap. Safe only because LISTEN is batched; see twitch_limits.
        const MAX_CHANNELS_PER_CONNECTION: usize = 10;

        // Calculate how many WebSocket connections we need
        let num_connections =
            (channel_ids.len() + MAX_CHANNELS_PER_CONNECTION - 1) / MAX_CHANNELS_PER_CONNECTION;

        debug!(
            "Creating {} WebSocket connection(s) for {} channels",
            num_connections.min(10),
            channel_ids.len()
        );

        // Split channels into chunks for each connection (max 10 connections)
        let chunks: Vec<_> = channel_ids
            .chunks(MAX_CHANNELS_PER_CONNECTION)
            .take(10) // Max 10 connections per IP as recommended
            .collect();

        for (index, chunk) in chunks.iter().enumerate() {
            // Add longer delay before first connection and between connections
            if index == 0 {
                // Give token validation time before first connection
                tokio::time::sleep(Duration::from_millis(500)).await;
            } else {
                // Longer delay between subsequent connections
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            let connection_id = Uuid::new_v4().to_string();
            let topics = self.build_topics_for_channels(chunk, user_id);

            // Store connection info
            {
                let mut connections = self.connections.write().await;
                connections.push(WebSocketConnection {
                    id: connection_id.clone(),
                    topics: topics.clone(),
                    is_connected: false,
                    last_ping: Utc::now(),
                    last_pong: Utc::now(),
                });
            }

            // Spawn WebSocket connection handler
            let auth_token = auth_token.to_string();
            let app_handle_clone = app_handle.clone();
            let connections = self.connections.clone();
            let channel_mappings = self.channel_mappings.clone();
            let active_viewing_channels = self.active_viewing_channels.clone();

            let handle = tokio::spawn(async move {
                if let Err(e) = Self::handle_connection(
                    connection_id.clone(),
                    topics,
                    auth_token,
                    connections,
                    app_handle_clone,
                    index,
                    channel_mappings,
                    active_viewing_channels,
                )
                .await
                {
                    error!("WebSocket connection {} failed: {}", index, e);
                }
            });
            self.reader_task_handles.lock().await.push(handle);
        }

        // Start ping/pong keeper
        self.start_ping_keeper(app_handle).await;

        Ok(())
    }

    /// Build PubSub topics for a set of channels
    fn build_topics_for_channels(&self, channel_ids: &[String], user_id: &str) -> Vec<String> {
        let mut topics = Vec::new();

        // Add community points topics for the user (global) - MOST IMPORTANT
        topics.push(format!("community-points-user-v1.{}", user_id));

        // For each channel, add only essential topics (2 per channel instead of 3)
        for channel_id in channel_ids {
            // Video playback events (stream up/down)
            topics.push(format!("video-playback-by-id.{}", channel_id));

            // Predictions (if we want to participate)
            topics.push(format!("predictions-channel-v1.{}", channel_id));

            // Polls (channel-scoped: POLL_CREATE / POLL_UPDATE / POLL_COMPLETE)
            topics.push(format!("polls.{}", channel_id));

            // Channel-wide community points feed: every viewer's reward
            // redemption (reward-redeemed). Broadcast to all listeners with no
            // broadcaster auth, so it surfaces redemptions on any channel you
            // watch, not just ones you own or moderate.
            topics.push(format!("community-points-channel-v1.{}", channel_id));
        }

        // User predictions results
        topics.push(format!("predictions-user-v1.{}", user_id));

        // Note: Removed raid topics to reduce count per channel
        // This allows 24 channels per connection: 24*2 + 2 global = 50 topics exactly

        topics
    }

    /// Handle a single WebSocket connection
    async fn handle_connection(
        connection_id: String,
        topics: Vec<String>,
        auth_token: String,
        connections: Arc<RwLock<Vec<WebSocketConnection>>>,
        app_handle: AppHandle,
        index: usize,
        channel_mappings: Arc<RwLock<HashMap<String, ChannelMapping>>>,
        active_viewing_channels: Arc<RwLock<HashSet<String>>>,
    ) -> Result<()> {
        debug!(
            "Connecting WebSocket #{} with {} topics",
            index,
            topics.len()
        );

        let (ws_stream, _) = connect_async(PUBSUB_URL).await?;
        let (mut write, mut read) = ws_stream.split();

        // Mark as connected
        {
            let mut conns = connections.write().await;
            if let Some(conn) = conns.iter_mut().find(|c| c.id == connection_id) {
                conn.is_connected = true;
            }
        }

        // Batch: an oversized LISTEN frame is dropped silently, subscribing
        // to nothing. See twitch_limits.
        for (batch_index, batch) in topics.chunks(PUBSUB_LISTEN_TOPICS_PER_FRAME).enumerate() {
            let listen_message = json!({
                "type": "LISTEN",
                "nonce": Uuid::new_v4().to_string(),
                "data": {
                    "topics": batch,
                    "auth_token": auth_token
                }
            });

            write
                .send(Message::text(listen_message.to_string()))
                .await?;
            debug!(
                "WebSocket #{} sent LISTEN batch {} ({} topics)",
                index,
                batch_index,
                batch.len()
            );

            // Space the frames out so a burst does not trip rate limiting.
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
        debug!(
            "WebSocket #{} sent LISTEN for {} topics in {} batch(es)",
            index,
            topics.len(),
            topics.len().div_ceil(PUBSUB_LISTEN_TOPICS_PER_FRAME)
        );

        // Twitch answers every good LISTEN; a missing ack is the only signal
        // a frame was dropped, so count them and shout rather than fail quiet.
        let expected_acks = topics.len().div_ceil(PUBSUB_LISTEN_TOPICS_PER_FRAME);
        let listen_acks = Arc::new(AtomicUsize::new(0));
        let acks_probe = listen_acks.clone();
        let subscribe_watchdog = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let got = acks_probe.load(Ordering::Relaxed);
            if got < expected_acks {
                error!(
                    "WebSocket #{} only {}/{} LISTEN batches acknowledged.                      Twitch silently drops an oversized LISTEN frame, so these                      topics are subscribed to NOTHING. Lower                      PUBSUB_LISTEN_TOPICS_PER_FRAME (currently {}).",
                    index, got, expected_acks, PUBSUB_LISTEN_TOPICS_PER_FRAME
                );
            }
        });

        let connections_ping = connections.clone();
        let connection_id_ping = connection_id.clone();

        // Spawn ping task to keep connection alive
        let ping_task = tokio::spawn(async move {
            let mut ping_interval = interval(Duration::from_secs(240)); // Ping every 4 minutes
            ping_interval.tick().await; // Skip first immediate tick

            loop {
                ping_interval.tick().await;

                let ping_message = json!({
                    "type": "PING"
                });

                if let Err(e) = write.send(Message::text(ping_message.to_string())).await {
                    error!("WebSocket #{} failed to send PING: {}", index, e);
                    break;
                }

                // Update last ping time
                {
                    let mut conns = connections_ping.write().await;
                    if let Some(conn) = conns.iter_mut().find(|c| c.id == connection_id_ping) {
                        conn.last_ping = Utc::now();
                    }
                }

                debug!("WebSocket #{} sent PING", index);
            }
        });

        // Handle incoming messages
        let mut should_reconnect = false;
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(pubsub_msg) = serde_json::from_str::<PubSubMessage>(&text) {
                        Self::handle_pubsub_message(
                            pubsub_msg,
                            &app_handle,
                            &connections,
                            &connection_id,
                            index,
                            &channel_mappings,
                            &active_viewing_channels,
                            &listen_acks,
                        )
                        .await;
                    }
                }
                Ok(Message::Close(_)) => {
                    debug!("WebSocket #{} closed by server", index);
                    should_reconnect = true;
                    break;
                }
                Err(e) => {
                    error!("WebSocket #{} error: {}", index, e);
                    should_reconnect = true;
                    break;
                }
                _ => {}
            }
        }

        // Signal ping task to stop
        ping_task.abort();
        subscribe_watchdog.abort();

        // Mark as disconnected
        {
            let mut conns = connections.write().await;
            if let Some(conn) = conns.iter_mut().find(|c| c.id == connection_id) {
                conn.is_connected = false;
            }
        }

        // Attempt reconnection after delay if needed
        if should_reconnect {
            tokio::time::sleep(Duration::from_secs(60)).await;
            debug!("Attempting to reconnect WebSocket #{}...", index);

            // Recursively reconnect
            Box::pin(Self::handle_connection(
                connection_id,
                topics,
                auth_token,
                connections,
                app_handle,
                index,
                channel_mappings,
                active_viewing_channels,
            ))
            .await
        } else {
            Ok(())
        }
    }

    /// Handle incoming PubSub messages
    async fn handle_pubsub_message(
        msg: PubSubMessage,
        app_handle: &AppHandle,
        connections: &Arc<RwLock<Vec<WebSocketConnection>>>,
        connection_id: &str,
        index: usize,
        channel_mappings: &Arc<RwLock<HashMap<String, ChannelMapping>>>,
        active_viewing_channels: &Arc<RwLock<HashSet<String>>>,
        listen_acks: &Arc<AtomicUsize>,
    ) {
        match msg.msg_type.as_str() {
            "MESSAGE" => {
                if let Some(data) = msg.data {
                    if let Ok(message_data) = serde_json::from_str::<Value>(&data.message) {
                        // Parse the topic to get the type
                        let topic_parts: Vec<&str> = data.topic.split('.').collect();
                        let topic_type = topic_parts[0];
                        let channel_id = if topic_parts.len() > 1 {
                            Some(topic_parts[1].to_string())
                        } else {
                            None
                        };

                        match topic_type {
                            "community-points-user-v1" => {
                                Self::handle_points_event(
                                    message_data,
                                    app_handle,
                                    channel_id,
                                    channel_mappings,
                                )
                                .await;
                            }
                            "video-playback-by-id" => {
                                // Dead event (frontend uses EventSub), but topic is still needed for internal Twitch drops tracking
                            }
                            "predictions-user-v1" => {
                                Self::handle_self_prediction_event(message_data, app_handle).await;
                            }
                            "predictions-channel-v1" => {
                                Self::handle_prediction_event(
                                    message_data,
                                    app_handle,
                                    channel_id,
                                    channel_mappings,
                                    active_viewing_channels,
                                )
                                .await;
                            }
                            "polls" => {
                                Self::handle_poll_event(
                                    message_data,
                                    app_handle,
                                    channel_id,
                                    active_viewing_channels,
                                )
                                .await;
                            }
                            "community-points-channel-v1" => {
                                Self::handle_channel_redemption_event(
                                    message_data,
                                    app_handle,
                                    channel_id,
                                    active_viewing_channels,
                                )
                                .await;
                            }
                            _ => {}
                        }
                    }
                }
            }
            "PONG" => {
                debug!("WebSocket #{} received PONG", index);

                // Update last pong time
                let mut conns = connections.write().await;
                if let Some(conn) = conns.iter_mut().find(|c| c.id == *connection_id) {
                    conn.last_pong = Utc::now();
                }
            }
            "RECONNECT" => {
                debug!("WebSocket #{} received RECONNECT request", index);
                // Connection will automatically reconnect when closed
            }
            "RESPONSE" => {
                listen_acks.fetch_add(1, Ordering::Relaxed);
                if let Some(error) = msg.error {
                    // Only treat non-empty errors as actual errors
                    if !error.is_empty() {
                        error!("WebSocket #{} error response: {}", index, error);
                    } else {
                        debug!("WebSocket #{} LISTEN acknowledged", index);
                    }
                } else {
                    debug!("WebSocket #{} LISTEN acknowledged", index);
                }
            }
            _ => {}
        }
    }

    /// Handle channel points events
    async fn handle_points_event(
        message_data: Value,
        app_handle: &AppHandle,
        _topic_channel_id: Option<String>,
        channel_mappings: &Arc<RwLock<HashMap<String, ChannelMapping>>>,
    ) {
        if let Some(event_type) = message_data["type"].as_str() {
            match event_type {
                "points-earned" => {
                    let points = message_data["data"]["point_gain"]["total_points"]
                        .as_i64()
                        .unwrap_or(0);
                    let reason = message_data["data"]["point_gain"]["reason_code"]
                        .as_str()
                        .unwrap_or("unknown");
                    let balance = message_data["data"]["balance"]["balance"]
                        .as_i64()
                        .unwrap_or(0);

                    // Extract channel_id from point_gain or balance objects (where it actually is)
                    let channel_id = message_data["data"]["point_gain"]["channel_id"]
                        .as_str()
                        .or_else(|| message_data["data"]["balance"]["channel_id"].as_str())
                        .or_else(|| message_data["data"]["channel_id"].as_str())
                        .map(|s| s.to_string());

                    // Try to extract channel login from various possible paths in the message
                    let mut channel_login = message_data["data"]["channel_login"]
                        .as_str()
                        .or_else(|| message_data["data"]["channel"]["login"].as_str())
                        .or_else(|| message_data["data"]["point_gain"]["channel_login"].as_str())
                        .map(|s| s.to_string());

                    // Try to extract channel display name from various possible paths in the message
                    let mut channel_display_name = message_data["data"]["channel"]["display_name"]
                        .as_str()
                        .or_else(|| message_data["data"]["point_gain"]["channel_name"].as_str())
                        .map(|s| s.to_string());

                    // If we don't have the channel info but we have channel_id, try to resolve from mapping
                    if let Some(ref cid) = channel_id {
                        let mapping = channel_mappings.read().await;
                        if let Some(channel_info) = mapping.get(cid) {
                            // Use mapping values if we don't have them from the message
                            if channel_login.is_none() {
                                channel_login = Some(channel_info.login.clone());
                            }
                            if channel_display_name.is_none() {
                                channel_display_name = Some(channel_info.display_name.clone());
                            }
                        } else {
                            // Drop the read lock before doing API call
                            drop(mapping);

                            // Channel not in mapping - try to look it up via API (for drops automation channels)
                            debug!("Channel {} not in mapping, attempting API lookup...", cid);
                            if let Ok(Some((login, display_name))) =
                                Self::lookup_channel_by_id(cid).await
                            {
                                debug!("Resolved channel {} -> {} ({})", cid, login, display_name);

                                // Cache it for future use
                                let mut mapping_write = channel_mappings.write().await;
                                mapping_write.insert(
                                    cid.clone(),
                                    ChannelMapping {
                                        login: login.clone(),
                                        display_name: display_name.clone(),
                                    },
                                );
                                drop(mapping_write);

                                if channel_login.is_none() {
                                    channel_login = Some(login);
                                }
                                if channel_display_name.is_none() {
                                    channel_display_name = Some(display_name);
                                }
                            }
                        }
                    }

                    // Unwrap values for cleaner logging
                    let channel_id_str = channel_id.as_deref().unwrap_or("unknown");
                    let channel_login_str = channel_login.as_deref().unwrap_or("unknown");
                    let channel_display_str = channel_display_name.as_deref().unwrap_or("unknown");

                    debug!(
                        "Points earned: +{} (reason: {}) - New balance: {} - Channel: {} (ID: {}, Login: {})",
                        points,
                        reason,
                        balance,
                        channel_display_str,
                        channel_id_str,
                        channel_login_str
                    );

                    let _ = app_handle.emit(
                        "channel-points-earned",
                        json!({
                            "channel_id": channel_id,
                            "channel_login": channel_login,
                            "channel_display_name": channel_display_name,
                            "points": points,
                            "reason": reason,
                            "balance": balance
                        }),
                    );
                }
                "claim-available" => {
                    let claim_id = message_data["data"]["claim"]["id"].as_str().unwrap_or("");
                    let claim_channel_id = message_data["data"]["claim"]["channel_id"]
                        .as_str()
                        .map(|s| s.to_string());

                    debug!("Bonus claim available! ID: {}", claim_id);

                    let _ = app_handle.emit(
                        "channel-points-claim-available",
                        json!({
                            "channel_id": claim_channel_id,
                            "claim_id": claim_id
                        }),
                    );
                }
                "points-spent" => {
                    let points = message_data["data"]["point_cost"]["cost"]
                        .as_i64()
                        .unwrap_or(0);
                    let balance = message_data["data"]["balance"]["balance"]
                        .as_i64()
                        .unwrap_or(0);
                    let spent_channel_id = message_data["data"]["channel_id"]
                        .as_str()
                        .map(|s| s.to_string());

                    debug!("Points spent: -{} - New balance: {}", points, balance);

                    let _ = app_handle.emit(
                        "channel-points-spent",
                        json!({
                            "channel_id": spent_channel_id,
                            "points": points,
                            "balance": balance
                        }),
                    );
                }
                _ => {}
            }
        }
    }

    /// The viewer's OWN stake in a prediction, from `predictions-user-v1`.
    ///
    /// Twitch keeps one prediction record per user per event and mutates it in
    /// place, so `points` is always the running total rather than a delta, and
    /// `prediction-updated` carries the same id as the `prediction-made` before
    /// it. All three message types share one payload shape.
    ///
    /// This is the only source for a viewer's own stake: the GQL read path for
    /// predictions no longer exists, and the channel topic reports only pooled
    /// totals. Nothing replays it, so a stake placed before the app started is
    /// unknowable and the UI must say nothing rather than imply zero.
    async fn handle_self_prediction_event(message_data: Value, app_handle: &AppHandle) {
        let kind = message_data["type"].as_str().unwrap_or_default();
        let p = &message_data["data"]["prediction"];
        if !p.is_object() {
            return;
        }
        let str_of = |k: &str| p.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();

        let channel_id = str_of("channel_id");
        let event_id = str_of("event_id");
        let outcome_id = str_of("outcome_id");
        let points = p.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
        // `result` is null while the prediction is open and an object once it
        // resolves; the type inside is what decides win/lose/refund.
        let result_type = p
            .pointer("/result/type")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let points_won = p.pointer("/result/points_won").and_then(|v| v.as_i64());

        debug!(
            "[Prediction] self {} on {}: {} points on outcome {}{}",
            kind,
            channel_id,
            points,
            outcome_id,
            result_type
                .as_deref()
                .map(|r| format!(" -> {}", r))
                .unwrap_or_default()
        );

        let _ = app_handle.emit(
            "prediction-self-updated",
            json!({
                "kind": kind,
                "channel_id": channel_id,
                "prediction_id": str_of("id"),
                "event_id": event_id,
                "outcome_id": outcome_id,
                "points": points,
                "result_type": result_type,
                "points_won": points_won,
            }),
        );
    }

    /// Handle prediction events
    async fn handle_prediction_event(
        message_data: Value,
        app_handle: &AppHandle,
        channel_id: Option<String>,
        channel_mappings: &Arc<RwLock<HashMap<String, ChannelMapping>>>,
        active_viewing_channels: &Arc<RwLock<HashSet<String>>>,
    ) {
        // Fast path: if we aren't currently watching this channel, don't process prediction events
        if let Some(ref cid) = channel_id {
            if !active_viewing_channels.read().await.contains(cid) {
                return;
            }
        }
        // Resolve channel name from mapping or API
        let channel_name = if let Some(ref cid) = channel_id {
            let mapping = channel_mappings.read().await;
            if let Some(channel_info) = mapping.get(cid) {
                Some(channel_info.login.clone())
            } else {
                drop(mapping);
                // Try API lookup
                if let Ok(Some((login, display_name))) = Self::lookup_channel_by_id(cid).await {
                    // Cache for future use
                    let mut mapping_write = channel_mappings.write().await;
                    mapping_write.insert(
                        cid.clone(),
                        ChannelMapping {
                            login: login.clone(),
                            display_name: display_name.clone(),
                        },
                    );
                    Some(login)
                } else {
                    None
                }
            }
        } else {
            None
        };

        // Format display string: "channel_name (ID)" or just "ID"
        let channel_display = match (&channel_name, &channel_id) {
            (Some(name), Some(id)) => format!("{} ({})", name, id),
            (None, Some(id)) => id.clone(),
            _ => "unknown".to_string(),
        };

        if let Some(event_type) = message_data["type"].as_str() {
            match event_type {
                "event-created" => {
                    // Extract full prediction details
                    if let Some(event) = message_data["data"]["event"].as_object() {
                        let title = event.get("title").and_then(|v| v.as_str()).unwrap_or("");
                        let prediction_id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let prediction_window_seconds = event
                            .get("prediction_window_seconds")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        let created_at = event
                            .get("created_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let status = event
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("ACTIVE");

                        // Extract outcomes (the prediction options)
                        let mut outcomes: Vec<Value> = Vec::new();
                        if let Some(outcomes_array) =
                            event.get("outcomes").and_then(|v| v.as_array())
                        {
                            for outcome in outcomes_array {
                                if let Some(outcome_obj) = outcome.as_object() {
                                    let outcome_id = outcome_obj
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let outcome_title = outcome_obj
                                        .get("title")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let color = outcome_obj
                                        .get("color")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("BLUE");
                                    let total_points = outcome_obj
                                        .get("total_points")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);
                                    let total_users = outcome_obj
                                        .get("total_users")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);

                                    outcomes.push(json!({
                                        "id": outcome_id,
                                        "title": outcome_title,
                                        "color": color,
                                        "total_points": total_points,
                                        "total_users": total_users
                                    }));
                                }
                            }
                        }

                        debug!(
                            "Prediction created on {}: {} (ID: {}, {} outcomes)",
                            channel_display,
                            title,
                            prediction_id,
                            outcomes.len()
                        );

                        // Emit to frontend
                        let emit_result = app_handle.emit(
                            "prediction-created",
                            json!({
                                "channel_id": channel_id,
                                "prediction_id": prediction_id,
                                "title": title,
                                "outcomes": outcomes,
                                "prediction_window_seconds": prediction_window_seconds,
                                "created_at": created_at,
                                "status": status
                            }),
                        );
                        debug!(
                            "Emitted prediction-created event to frontend: {:?}",
                            emit_result.is_ok()
                        );
                    }
                }
                "event-updated" => {
                    // Handle prediction updates (new bets, status changes)
                    if let Some(event) = message_data["data"]["event"].as_object() {
                        let prediction_id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let status = event.get("status").and_then(|v| v.as_str()).unwrap_or("");
                        let title = event.get("title").and_then(|v| v.as_str()).unwrap_or("");
                        // Extract winning_outcome_id for RESOLVED predictions
                        let winning_outcome_id =
                            event.get("winning_outcome_id").and_then(|v| v.as_str());

                        // Extract updated outcomes
                        let mut outcomes: Vec<Value> = Vec::new();
                        if let Some(outcomes_array) =
                            event.get("outcomes").and_then(|v| v.as_array())
                        {
                            for outcome in outcomes_array {
                                if let Some(outcome_obj) = outcome.as_object() {
                                    let outcome_id = outcome_obj
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let outcome_title = outcome_obj
                                        .get("title")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let color = outcome_obj
                                        .get("color")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("BLUE");
                                    let total_points = outcome_obj
                                        .get("total_points")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);
                                    let total_users = outcome_obj
                                        .get("total_users")
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0);

                                    outcomes.push(json!({
                                        "id": outcome_id,
                                        "title": outcome_title,
                                        "color": color,
                                        "total_points": total_points,
                                        "total_users": total_users
                                    }));
                                }
                            }
                        }

                        debug!(
                            "Prediction updated on {}: {} - Status: {} - Winner: {:?}",
                            channel_display, title, status, winning_outcome_id
                        );

                        // Emit update to frontend
                        let _ = app_handle.emit(
                            "prediction-updated",
                            json!({
                                "channel_id": channel_id,
                                "prediction_id": prediction_id,
                                "title": title,
                                "status": status,
                                "outcomes": outcomes,
                                "winning_outcome_id": winning_outcome_id
                            }),
                        );
                    }
                }
                "event-locked" => {
                    // Prediction locked - no more bets allowed
                    if let Some(event) = message_data["data"]["event"].as_object() {
                        let prediction_id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let title = event.get("title").and_then(|v| v.as_str()).unwrap_or("");

                        debug!("Prediction locked on {}: {}", channel_display, title);

                        let _ = app_handle.emit(
                            "prediction-locked",
                            json!({
                                "channel_id": channel_id,
                                "prediction_id": prediction_id,
                                "title": title
                            }),
                        );
                    }
                }
                "event-ended" => {
                    // Prediction ended - has a winner
                    if let Some(event) = message_data["data"]["event"].as_object() {
                        let prediction_id = event.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let title = event.get("title").and_then(|v| v.as_str()).unwrap_or("");
                        let winning_outcome_id =
                            event.get("winning_outcome_id").and_then(|v| v.as_str());

                        let winner_display = winning_outcome_id.unwrap_or("cancelled");
                        debug!(
                            "Prediction ended on {}: {} - Winner: {}",
                            channel_display, title, winner_display
                        );

                        let _ = app_handle.emit(
                            "prediction-ended",
                            json!({
                                "channel_id": channel_id,
                                "prediction_id": prediction_id,
                                "title": title,
                                "winning_outcome_id": winning_outcome_id
                            }),
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// Handle poll events (channel-scoped `polls.{id}` topic).
    ///
    /// Twitch delivers the whole poll object on every event, so a single
    /// normalizer covers create/update/complete. The web client has no GQL
    /// read for polls — it learns the poll purely from `POLL_CREATE` — so a
    /// viewer who joins mid-poll picks it up on the next `POLL_UPDATE` (which
    /// fires on each vote). The payload is snake_case; we flatten it into the
    /// camelCase-ish shape the frontend PollOverlay expects.
    async fn handle_poll_event(
        message_data: Value,
        app_handle: &AppHandle,
        channel_id: Option<String>,
        active_viewing_channels: &Arc<RwLock<HashSet<String>>>,
    ) {
        // Fast path: ignore polls for channels we aren't currently watching.
        if let Some(ref cid) = channel_id {
            if !active_viewing_channels.read().await.contains(cid) {
                return;
            }
        }

        let event_type = match message_data["type"].as_str() {
            Some(t) => t,
            None => return,
        };

        let poll = match message_data["data"]["poll"].as_object() {
            Some(p) => p,
            None => return,
        };

        let poll_id = poll.get("poll_id").and_then(|v| v.as_str()).unwrap_or("");
        let title = poll.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let status = poll.get("status").and_then(|v| v.as_str()).unwrap_or("ACTIVE");
        let duration_seconds = poll
            .get("duration_seconds")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let remaining_ms = poll
            .get("remaining_duration_milliseconds")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let total_voters = poll.get("total_voters").and_then(|v| v.as_i64()).unwrap_or(0);
        let total_votes = poll
            .get("votes")
            .and_then(|v| v.get("total"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let started_at = poll.get("started_at").and_then(|v| v.as_str()).unwrap_or("");

        // Poll-level vote settings (whether bits/channel-points voting is on).
        let channel_points_voting = poll["settings"]["channel_points_votes"]["is_enabled"]
            .as_bool()
            .unwrap_or(false);
        let channel_points_cost = poll["settings"]["channel_points_votes"]["cost"]
            .as_i64()
            .unwrap_or(0);

        // Normalize choices to { id, title, total_votes, total_voters }, keeping
        // Twitch's created order (stable across events).
        let mut choices: Vec<Value> = Vec::new();
        if let Some(arr) = poll.get("choices").and_then(|v| v.as_array()) {
            for choice in arr {
                let id = choice.get("choice_id").and_then(|v| v.as_str()).unwrap_or("");
                let choice_title = choice.get("title").and_then(|v| v.as_str()).unwrap_or("");
                let votes = choice
                    .get("votes")
                    .and_then(|v| v.get("total"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let voters = choice.get("total_voters").and_then(|v| v.as_i64()).unwrap_or(0);
                choices.push(json!({
                    "id": id,
                    "title": choice_title,
                    "total_votes": votes,
                    "total_voters": voters
                }));
            }
        }

        let payload = json!({
            "channel_id": channel_id,
            "poll_id": poll_id,
            "title": title,
            "status": status,
            "duration_seconds": duration_seconds,
            "remaining_ms": remaining_ms,
            "total_voters": total_voters,
            "total_votes": total_votes,
            "started_at": started_at,
            "channel_points_voting": channel_points_voting,
            "channel_points_cost": channel_points_cost,
            "choices": choices
        });

        // Map Twitch's event type to a frontend event. TERMINATE/ARCHIVE aren't
        // in the captured set but are handled defensively as an end-of-poll.
        let emit_event = match event_type {
            "POLL_CREATE" => "poll-created",
            "POLL_UPDATE" => "poll-updated",
            "POLL_COMPLETE" | "POLL_ARCHIVE" | "POLL_TERMINATE" => "poll-completed",
            _ => return,
        };

        debug!(
            "Poll {} on channel {:?}: {} ({} voters, {} votes, status {})",
            emit_event, channel_id, title, total_voters, total_votes, status
        );

        let _ = app_handle.emit(emit_event, payload);
    }

    /// Handle the channel-wide community points feed (`community-points-channel-v1`).
    ///
    /// This is broadcast to every viewer with no broadcaster auth, so it's how we
    /// surface OTHER viewers' reward redemptions on a channel we're only watching.
    /// We forward every `reward-redeemed` event with an `is_input_required` flag;
    /// the frontend shows the no-input ones (input rewards' text isn't public, and
    /// message-style rewards already appear in chat on their own).
    async fn handle_channel_redemption_event(
        message_data: Value,
        app_handle: &AppHandle,
        channel_id: Option<String>,
        active_viewing_channels: &Arc<RwLock<HashSet<String>>>,
    ) {
        // Only for channels we're actively watching (skip background farming channels).
        if let Some(ref cid) = channel_id {
            if !active_viewing_channels.read().await.contains(cid) {
                return;
            }
        }

        if message_data["type"].as_str() != Some("reward-redeemed") {
            return;
        }

        let redemption = match message_data["data"]["redemption"].as_object() {
            Some(r) => r,
            None => return,
        };
        let user = redemption.get("user");
        let reward = redemption.get("reward");

        // Twitch's redemption id — a stable dedupe key so the same redemption
        // injected by more than one open chat view collapses to one row.
        let redemption_id = redemption.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let user_id = user.and_then(|u| u["id"].as_str()).unwrap_or("");
        let user_login = user.and_then(|u| u["login"].as_str()).unwrap_or("");
        let user_name = user
            .and_then(|u| u["display_name"].as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(user_login);

        let reward_id = reward.and_then(|r| r["id"].as_str()).unwrap_or("");
        let reward_title = reward.and_then(|r| r["title"].as_str()).unwrap_or("");
        let reward_cost = reward.and_then(|r| r["cost"].as_i64()).unwrap_or(0);
        let reward_prompt = reward.and_then(|r| r["prompt"].as_str()).unwrap_or("");
        let is_input_required = reward
            .and_then(|r| r["is_user_input_required"].as_bool())
            .unwrap_or(false);
        let user_input = redemption
            .get("user_input")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let background_color = reward
            .and_then(|r| r["background_color"].as_str())
            .unwrap_or("");

        // Reward image (image / default_image can be null); prefer the largest.
        let image_url = reward
            .and_then(|r| r.get("image").or_else(|| r.get("default_image")))
            .and_then(|img| {
                img["url_4x"]
                    .as_str()
                    .or_else(|| img["url_2x"].as_str())
                    .or_else(|| img["url_1x"].as_str())
            })
            .unwrap_or("");

        if reward_title.is_empty() {
            return;
        }

        debug!(
            "Community redemption on {:?}: {} redeemed '{}' ({} pts)",
            channel_id, user_name, reward_title, reward_cost
        );

        let _ = app_handle.emit(
            "channel-points-community-redemption",
            json!({
                "channel_id": channel_id,
                "redemption_id": redemption_id,
                "user_id": user_id,
                "user_login": user_login,
                "user_name": user_name,
                "reward_id": reward_id,
                "reward_title": reward_title,
                "reward_cost": reward_cost,
                "reward_prompt": reward_prompt,
                "is_input_required": is_input_required,
                "user_input": user_input,
                "background_color": background_color,
                "image_url": image_url,
            }),
        );
    }

    /// Start ping keeper to maintain connections. Idempotent — if a keeper is
    /// already running, this is a no-op. `disconnect_all` aborts the keeper so
    /// the next `connect_to_channels` gets a fresh one.
    async fn start_ping_keeper(&self, _app_handle: AppHandle) {
        {
            let guard = self.ping_keeper_handle.lock().await;
            if let Some(h) = guard.as_ref() {
                if !h.is_finished() {
                    return;
                }
            }
        }

        let connections = self.connections.clone();
        let handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(240)); // Ping every 4 minutes

            loop {
                interval.tick().await;

                let conns = connections.read().await;
                for (index, conn) in conns.iter().enumerate() {
                    if conn.is_connected {
                        let elapsed = Utc::now().signed_duration_since(conn.last_pong);
                        if elapsed.num_minutes() > 5 {
                            debug!(
                                "WebSocket #{} hasn't received PONG in {} minutes",
                                index,
                                elapsed.num_minutes()
                            );
                        }
                    }
                }
            }
        });

        *self.ping_keeper_handle.lock().await = Some(handle);
    }

    /// Disconnect all WebSocket connections. Aborts the spawned reader tasks
    /// AND the ping keeper — without this, the recursive reconnect path at
    /// the bottom of `handle_connection` keeps the task alive past disconnect,
    /// causing stacked "ghost" sockets to keep firing PubSub events on the
    /// next connect_to_channels call.
    pub async fn disconnect_all(&self) {
        // Abort all reader tasks.
        {
            let mut handles = self.reader_task_handles.lock().await;
            for h in handles.drain(..) {
                h.abort();
            }
        }

        // Abort the ping keeper.
        if let Some(h) = self.ping_keeper_handle.lock().await.take() {
            h.abort();
        }

        let mut connections = self.connections.write().await;
        connections.clear();
        drop(connections);

        // register_channel_mapping inserts on every watched-channel change and
        // nothing ever removed an entry, so this grew for the life of the
        // process while channel hopping. It exists only to decorate events for
        // channels we are currently connected to, and those were just dropped,
        // so it has nothing left to describe.
        self.channel_mappings.write().await.clear();

        debug!("All WebSocket connections closed (reader + ping tasks aborted)");
    }

    /// Look up channel info by ID via Twitch API (fallback for unknown channels)
    async fn lookup_channel_by_id(channel_id: &str) -> Result<Option<(String, String)>> {
        // Use drops client ID for GQL query
        const CLIENT_ID: &str = env!("TWITCH_ANDROID_CLIENT_ID");

        let token = match DropsAuthService::get_token().await {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to get token for channel lookup: {}", e);
                return Ok(None);
            }
        };

        let client = crate::services::http::client().clone();

        let query = r#"
        query GetUserById($userId: ID!) {
            user(id: $userId) {
                id
                login
                displayName
            }
        }
        "#;

        let response = client
            .post("https://gql.twitch.tv/gql")
            .header("Client-Id", CLIENT_ID)
            .header("Authorization", format!("Bearer {}", token))
            .json(&json!({
                "query": query,
                "variables": {
                    "userId": channel_id
                }
            }))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if let Ok(result) = resp.json::<Value>().await {
                    if let Some(user) = result["data"]["user"].as_object() {
                        let login = user["login"].as_str().unwrap_or("").to_string();
                        let display_name =
                            user["displayName"].as_str().unwrap_or(&login).to_string();

                        if !login.is_empty() {
                            return Ok(Some((login, display_name)));
                        }
                    }
                }
                Ok(None)
            }
            Err(e) => {
                error!("API lookup failed for channel {}: {}", channel_id, e);
                Ok(None)
            }
        }
    }
}
