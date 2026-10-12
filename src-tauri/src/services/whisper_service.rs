use futures_util::{SinkExt, StreamExt};
use log::{debug, error, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Mutex;
use crate::rt::AppHandle;
use tauri::Emitter;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

use crate::services::eventsub_service::{EventSubService, LiveSession, PendingSession, SeenMessages, SocketOwner};
use crate::services::twitch_service::TwitchService;

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");
/// Wait before retrying a failed connect; doubles per failure up to the max.
const RETRY_MIN_SECS: u64 = 5;
const RETRY_MAX_SECS: u64 = 300;
/// A session that dies sooner than this counts as a failure for the backoff,
/// so a socket that keeps closing right after subscribing cannot spin.
const HEALTHY_SESSION: Duration = Duration::from_secs(60);

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

/// The only owner of `user.whisper.message`: one always-on socket per signed-in
/// user, which reconnects on its own and files every whisper in the inbox.
pub struct WhisperService {
    // The listener task and the user id it listens for.
    task: Mutex<Option<(String, JoinHandle<()>)>>,
}

/// How one connection ended.
enum Ended {
    /// The session was up and subscribed for this long, then closed or went quiet.
    Dropped(Duration),
    /// Twitch refused the subscription itself (401/403): retrying repeats it.
    Refused,
    /// Connect, welcome or subscribe failed in a way worth retrying.
    Failed,
}

enum SubscribeError {
    Refused(String),
    Retry(String),
}

impl WhisperService {
    pub fn new() -> Self {
        Self {
            task: Mutex::new(None),
        }
    }

    /// Start listening for whispers to `user_id`. A no-op while a listener for
    /// the same user runs; a different user replaces it.
    pub async fn start_listening(&self, user_id: String, app_handle: AppHandle) -> Result<(), String> {
        let mut slot = self.task.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((current, handle)) = slot.as_ref() {
            if *current == user_id && !handle.is_finished() {
                debug!("[WhisperService] Already listening for whispers");
                return Ok(());
            }
        }
        if let Some((_, handle)) = slot.take() {
            handle.abort();
        }
        *slot = Some((user_id.clone(), tokio::spawn(listen(user_id, app_handle))));
        Ok(())
    }

    pub async fn is_connected(&self) -> bool {
        self.task
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|(_, handle)| !handle.is_finished())
    }
}

/// Keep one whisper session up while the user stays signed in.
async fn listen(user_id: String, app_handle: AppHandle) {
    let mut retry = RETRY_MIN_SECS;
    loop {
        // Fetched per attempt: the token from sign-in has usually been
        // refreshed by the time a long-lived socket has to resubscribe.
        let Ok(token) = TwitchService::get_token().await else {
            debug!("[WhisperService] Signed out, whisper listener stopped");
            return;
        };
        let failed = match run_session(&user_id, &token, &app_handle).await {
            Ended::Dropped(lasted) => lasted < HEALTHY_SESSION,
            Ended::Failed => true,
            Ended::Refused => {
                warn!("[WhisperService] Twitch refused the whisper subscription, listener stopped");
                return;
            }
        };
        if !failed {
            retry = RETRY_MIN_SECS;
        }
        debug!("[WhisperService] Reconnecting in {}s", retry);
        sleep(Duration::from_secs(retry)).await;
        if failed {
            retry = (retry * 2).min(RETRY_MAX_SECS);
        }
    }
}

async fn run_session(user_id: &str, token: &str, app_handle: &AppHandle) -> Ended {
    debug!("[WhisperService] Connecting to Twitch EventSub for whispers...");
    let (ws_stream, session) = match EventSubService::open_session(EVENTSUB_URL.to_string()).await {
        Ok(conn) => conn,
        Err(e) => {
            error!("[WhisperService] Failed to connect to EventSub: {}", e);
            return Ended::Failed;
        }
    };
    let (mut write, mut read) = ws_stream.split();
    let mut session_id = session.id;
    let mut keepalive_secs = session.keepalive_timeout_seconds.unwrap_or(10);
    // Registered before subscribing, so the watch socket's connect cleanup
    // never deletes this session's subscription.
    let mut live = LiveSession::open(SocketOwner::Whispers, &session_id);

    match subscribe_to_whispers(&session_id, user_id, token).await {
        Ok(()) => debug!("[WhisperService] Successfully subscribed to whisper events"),
        Err(SubscribeError::Refused(e)) => {
            error!("[WhisperService] Failed to subscribe to whispers: {}", e);
            return Ended::Refused;
        }
        Err(SubscribeError::Retry(e)) => {
            error!("[WhisperService] Failed to subscribe to whispers: {}", e);
            return Ended::Failed;
        }
    }

    let started = Instant::now();
    // The new connection Twitch asked for, opening while this one keeps
    // delivering. Twitch's rule: close the old one only after the new
    // welcome arrives; the subscription carries over, so nothing is re-posted.
    let mut migrating: Option<PendingSession> = None;
    let mut seen = SeenMessages::new();
    loop {
        let deadline = Duration::from_secs(keepalive_secs + 10);
        let frame = tokio::select! {
            frame = timeout(deadline, read.next()) => frame,
            moved = async { migrating.as_mut().expect("guarded by is_some").await }, if migrating.is_some() => {
                migrating = None;
                match moved {
                    Ok((ws_stream, session)) => {
                        let _ = write.send(Message::Close(None)).await;
                        (write, read) = ws_stream.split();
                        keepalive_secs = session.keepalive_timeout_seconds.unwrap_or(keepalive_secs);
                        if session.id != session_id {
                            // Register the new id before the old guard drops.
                            drop(std::mem::replace(
                                &mut live,
                                LiveSession::open(SocketOwner::Whispers, &session.id),
                            ));
                            session_id = session.id;
                        }
                        debug!("[WhisperService] Moved to the reconnect URL, subscription carried over");
                    }
                    Err(e) => {
                        warn!("[WhisperService] Reconnect URL failed, staying on the old connection until it closes: {}", e);
                    }
                }
                continue;
            }
        };
        match frame {
            Err(_) => {
                warn!("[WhisperService] No frames in {}s, reconnecting", deadline.as_secs());
                return Ended::Dropped(started.elapsed());
            }
            Ok(None) => {
                debug!("[WhisperService] WebSocket ended");
                return Ended::Dropped(started.elapsed());
            }
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Some(url) = handle_text(&text, user_id, app_handle, &mut seen) {
                    if migrating.is_none() {
                        debug!("[WhisperService] Server requested reconnect, moving to the new URL");
                        migrating = Some(Box::pin(EventSubService::open_session(url)));
                    }
                }
            }
            Ok(Some(Ok(Message::Ping(data)))) => {
                if let Err(e) = write.send(Message::Pong(data)).await {
                    error!("[WhisperService] Failed to send pong: {}", e);
                    return Ended::Dropped(started.elapsed());
                }
            }
            Ok(Some(Ok(Message::Close(_)))) => {
                debug!("[WhisperService] WebSocket closed");
                return Ended::Dropped(started.elapsed());
            }
            Ok(Some(Err(e))) => {
                error!("[WhisperService] WebSocket error: {}", e);
                return Ended::Dropped(started.elapsed());
            }
            Ok(Some(Ok(_))) => {}
        }
    }
}

/// Handle one server frame. Returns the reconnect URL when Twitch sends
/// `session_reconnect`.
fn handle_text(text: &str, user_id: &str, app_handle: &AppHandle, seen: &mut SeenMessages) -> Option<String> {
    let Ok(ws_msg) = serde_json::from_str::<WebSocketMessage>(text) else {
        return None;
    };
    match ws_msg.metadata.message_type.as_str() {
        "notification" => {
            // A redelivered whisper would be filed in the inbox twice.
            if !seen.first_time(&ws_msg.metadata.message_id) {
                debug!("[WhisperService] Dropped a repeated notification {}", ws_msg.metadata.message_id);
                return None;
            }
            let event = ws_msg.payload.get("event")?;
            let whisper = event.get("whisper")?;
            let field = |key: &str| event[key].as_str().unwrap_or("").to_string();
            let whisper_event = WhisperEvent {
                from_user_id: field("from_user_id"),
                from_user_login: field("from_user_login"),
                from_user_name: field("from_user_name"),
                to_user_id: field("to_user_id"),
                to_user_login: field("to_user_login"),
                to_user_name: field("to_user_name"),
                whisper_id: field("whisper_id"),
                text: whisper["text"].as_str().unwrap_or("").to_string(),
            };

            debug!(
                "[WhisperService] Received whisper from {}: {}",
                whisper_event.from_user_name, whisper_event.text
            );

            // Emit to frontend
            if let Err(e) = app_handle.emit("whisper-received", &whisper_event) {
                error!("[WhisperService] Failed to emit whisper event: {}", e);
            }
            // Persist it here, whether or not a window is open to see it.
            tauri::async_runtime::spawn(crate::services::whisper_inbox::record_incoming(
                app_handle.clone(),
                user_id.to_string(),
                whisper_event,
                chrono::Utc::now().timestamp_millis(),
            ));
            None
        }
        "session_reconnect" => {
            let url = EventSubService::reconnect_url(ws_msg.payload).ok().flatten();
            if url.is_none() {
                warn!("[WhisperService] session_reconnect without a usable URL: {}", text);
            }
            url
        }
        "revocation" => {
            warn!("[WhisperService] Whisper subscription revoked: {}", text);
            None
        }
        "session_keepalive" => None,
        other => {
            debug!("[WhisperService] Unknown message type: {}", other);
            None
        }
    }
}

async fn subscribe_to_whispers(
    session_id: &str,
    user_id: &str,
    access_token: &str,
) -> Result<(), SubscribeError> {
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
        .map_err(|e| SubscribeError::Retry(format!("Request failed: {}", e)))?;

    let status = response.status();
    let response_text = response.text().await.unwrap_or_default();

    if status.is_success() {
        debug!("[WhisperService] Whisper subscription created successfully");
        Ok(())
    } else {
        let message = format!("Failed to subscribe ({}): {}", status, response_text);
        if matches!(status.as_u16(), 401 | 403) {
            Err(SubscribeError::Refused(message))
        } else {
            Err(SubscribeError::Retry(message))
        }
    }
}
