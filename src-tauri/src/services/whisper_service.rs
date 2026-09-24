use futures_util::{SinkExt, StreamExt};
use log::{debug, error};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");

#[derive(Debug, Deserialize)]
struct WebSocketMessage {
    metadata: Metadata,
    payload: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    message_id: String,
    message_type: String,
    message_timestamp: String,
}

#[derive(Debug, Deserialize)]
struct SessionPayload {
    session: Session,
}

#[derive(Debug, Deserialize)]
struct Session {
    id: String,
    keepalive_timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WhisperEvent {
    pub from_user_id: String,
    pub from_user_login: String,
    pub from_user_name: String,
    pub to_user_id: String,
    pub to_user_login: String,
    pub to_user_name: String,
    pub whisper_id: String,
    pub text: String,
}

pub struct WhisperService {
    is_connected: Arc<RwLock<bool>>,
}

impl WhisperService {
    pub fn new() -> Self {
        Self {
            is_connected: Arc::new(RwLock::new(false)),
        }
    }

    /// Start listening for whispers
    pub async fn start_listening(
        &self,
        user_id: String,
        access_token: String,
        app_handle: AppHandle,
    ) -> Result<(), String> {
        // Check if already connected and mark as connecting immediately to prevent race conditions
        {
            let mut connected = self.is_connected.write().await;
            if *connected {
                debug!("[WhisperService] Already connected to whisper EventSub");
                return Ok(());
            }
            // Mark as connecting immediately to prevent duplicate connections
            *connected = true;
        }

        let is_connected = self.is_connected.clone();

        // Spawn the WebSocket listener
        tokio::spawn(async move {
            debug!("[WhisperService] Connecting to Twitch EventSub for whispers...");

            match connect_async(EVENTSUB_URL).await {
                Ok((ws_stream, _)) => {
                    let (mut write, mut read) = ws_stream.split();

                    debug!("[WhisperService] Connected to EventSub WebSocket");

                    while let Some(msg) = read.next().await {
                        match msg {
                            Ok(Message::Text(text)) => {
                                if let Ok(ws_msg) = serde_json::from_str::<WebSocketMessage>(&text)
                                {
                                    match ws_msg.metadata.message_type.as_str() {
                                        "session_welcome" => {
                                            debug!(
                                                "[WhisperService] Received session welcome, subscribing to whispers..."
                                            );
                                            if let Ok(payload) =
                                                serde_json::from_value::<SessionPayload>(
                                                    ws_msg.payload.clone(),
                                                )
                                            {
                                                let session_id = payload.session.id;
                                                // Subscribe to whisper events
                                                if let Err(e) = Self::subscribe_to_whispers(
                                                    &session_id,
                                                    &user_id,
                                                    &access_token,
                                                )
                                                .await
                                                {
                                                    error!(
                                                        "[WhisperService] Failed to subscribe to whispers: {}",
                                                        e
                                                    );
                                                } else {
                                                    debug!(
                                                        "[WhisperService] Successfully subscribed to whisper events"
                                                    );
                                                }
                                            }
                                        }
                                        "notification" => {
                                            // Handle whisper notification
                                            if let Some(event) = ws_msg.payload.get("event") {
                                                if let Some(whisper) = event.get("whisper") {
                                                    let whisper_event = WhisperEvent {
                                                        from_user_id: event["from_user_id"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string(),
                                                        from_user_login: event["from_user_login"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string(),
                                                        from_user_name: event["from_user_name"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string(),
                                                        to_user_id: event["to_user_id"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string(),
                                                        to_user_login: event["to_user_login"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string(),
                                                        to_user_name: event["to_user_name"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string(),
                                                        whisper_id: event["whisper_id"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string(),
                                                        text: whisper["text"]
                                                            .as_str()
                                                            .unwrap_or("")
                                                            .to_string(),
                                                    };

                                                    debug!(
                                                        "[WhisperService] Received whisper from {}: {}",
                                                        whisper_event.from_user_name,
                                                        whisper_event.text
                                                    );

                                                    // Emit to frontend
                                                    if let Err(e) = app_handle
                                                        .emit("whisper-received", &whisper_event)
                                                    {
                                                        error!(
                                                            "[WhisperService] Failed to emit whisper event: {}",
                                                            e
                                                        );
                                                    }
                                                    // Persist it here, whether or not a window
                                                    // is open to see it.
                                                    tauri::async_runtime::spawn(
                                                        crate::services::whisper_inbox::record_incoming(
                                                            app_handle.clone(),
                                                            user_id.clone(),
                                                            whisper_event,
                                                            chrono::Utc::now().timestamp_millis(),
                                                        ),
                                                    );
                                                }
                                            }
                                        }
                                        "session_keepalive" => {
                                            // Just a keepalive, no action needed
                                        }
                                        "session_reconnect" => {
                                            debug!("[WhisperService] Server requested reconnect");
                                            // Handle reconnect if needed
                                        }
                                        _ => {
                                            debug!(
                                                "[WhisperService] Unknown message type: {}",
                                                ws_msg.metadata.message_type
                                            );
                                        }
                                    }
                                }
                            }
                            Ok(Message::Ping(data)) => {
                                // Respond to ping
                                if let Err(e) = write.send(Message::Pong(data)).await {
                                    error!("[WhisperService] Failed to send pong: {}", e);
                                }
                            }
                            Ok(Message::Close(_)) => {
                                debug!("[WhisperService] WebSocket closed");
                                *is_connected.write().await = false;
                                break;
                            }
                            Err(e) => {
                                error!("[WhisperService] WebSocket error: {}", e);
                                *is_connected.write().await = false;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    error!("[WhisperService] Failed to connect to EventSub: {}", e);
                }
            }

            *is_connected.write().await = false;
            debug!("[WhisperService] Whisper listener stopped");
        });

        Ok(())
    }

    async fn subscribe_to_whispers(
        session_id: &str,
        user_id: &str,
        access_token: &str,
    ) -> Result<(), String> {
        let client = crate::services::http::client().clone();

        let body = json!({
            "type": "user.whisper.message",
            "version": "1",
            "condition": {
                "user_id": user_id
            },
            "transport": {
                "method": "websocket",
                "session_id": session_id
            }
        });

        let response = client
            .post("https://api.twitch.tv/helix/eventsub/subscriptions")
            .header("Client-ID", CLIENT_ID)
            .header("Authorization", format!("Bearer {}", access_token))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Request failed: {}", e))?;

        let status = response.status();
        let response_text = response.text().await.unwrap_or_default();

        if status.is_success() {
            debug!("[WhisperService] Whisper subscription created successfully");
            Ok(())
        } else {
            Err(format!(
                "Failed to subscribe ({}): {}",
                status, response_text
            ))
        }
    }

    pub async fn is_connected(&self) -> bool {
        *self.is_connected.read().await
    }
}
