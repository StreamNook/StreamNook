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

const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");
/// Wait before retrying a failed connect; doubles per failure up to the max.
const RETRY_MIN_SECS: u64 = 5;
const RETRY_MAX_SECS: u64 = 300;
/// A session that dies sooner than this counts as a failure for the backoff,
/// so a socket that keeps closing right after subscribing cannot spin.
const HEALTHY_SESSION: Duration = Duration::from_secs(60);
/// The first wait after Twitch refused the subscription for the account's
/// socket limit: three sockets with subscriptions per client id and user, which
/// a phone and a desktop signed in as the same person share. Another device's
/// socket frees its slot when it closes, so this waits longer than a network
/// failure would, and still keeps trying.
const OVER_BUDGET_SECS: u64 = 60;

/// Wakes a listener waiting out its backoff, when the app comes back to the
/// front (the phones say so through `note_app_resumed`).
static WAKE: tokio::sync::Notify = tokio::sync::Notify::const_new();

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
    /// Twitch refused the subscription because this account already has its
    /// three sockets with subscriptions (another device, or this one's watch and
    /// moderation sockets). Retried after a longer wait.
    OverBudget,
    /// Connect, welcome or subscribe failed in a way worth retrying.
    Failed,
}

enum SubscribeError {
    Refused(String),
    OverBudget(String),
    Retry(String),
}

/// How a refused subscription is handled, from Twitch's status and body.
/// 429 with "transport" in it is the per-account socket limit ("number of
/// websocket transports limit exceeded"); any other 429 is a plain rate limit.
fn classify_refusal(status: u16, body: &str) -> SubscribeError {
    let message = format!("Failed to subscribe ({}): {}", status, body);
    match status {
        401 | 403 => SubscribeError::Refused(message),
        _ if crate::services::eventsub_budget::is_transport_limit(status, body) => SubscribeError::OverBudget(message),
        _ => SubscribeError::Retry(message),
    }
}

/// The wait before the next attempt, and the wait after that.
fn next_wait(retry: u64, ended: &Ended) -> (u64, u64) {
    match ended {
        Ended::OverBudget => {
            let wait = retry.max(OVER_BUDGET_SECS);
            (wait, (wait * 2).min(RETRY_MAX_SECS))
        }
        Ended::Dropped(lasted) if *lasted >= HEALTHY_SESSION => (RETRY_MIN_SECS, RETRY_MIN_SECS),
        _ => (retry, (retry * 2).min(RETRY_MAX_SECS)),
    }
}

/// The app came back to the front: a listener waiting to retry tries now.
/// A live session is left alone (it keeps its subscription and its slot).
pub fn resume() {
    WAKE.notify_waiters();
}

/// Follow the signed-in Twitch account: listen for its whispers, or stop when
/// nobody is signed in (or StreamNook needs the sign-in again). Called with
/// every account roster build, so sign-in, sign-out and an account switch all
/// land here on every platform; starting for the account already followed
/// changes nothing.
pub fn follow_account(app: &AppHandle, user_id: Option<String>) {
    use std::sync::Arc;
    use tauri::Manager;
    use tokio::sync::Mutex as TokioMutex;
    let Some(service) = app.try_state::<Arc<TokioMutex<WhisperService>>>() else { return };
    let service = service.inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let service = service.lock().await;
        match user_id {
            Some(id) => {
                let _ = service.start_listening(id, app).await;
            }
            None => service.stop(),
        }
    });
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
        log::info!("[WhisperService] listening for whispers to the signed-in account");
        *slot = Some((user_id.clone(), tokio::spawn(listen(user_id, app_handle))));
        Ok(())
    }

    /// Stop listening (signed out). Aborting the task drops its socket and its
    /// `LiveSession`, so nothing more is filed for the account that left.
    pub fn stop(&self) {
        let mut slot = self.task.lock().unwrap_or_else(|p| p.into_inner());
        if let Some((_, handle)) = slot.take() {
            if !handle.is_finished() {
                log::info!("[WhisperService] signed out, whisper listener stopped");
            }
            handle.abort();
        }
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
        let ended = run_session(&user_id, &token, &app_handle).await;
        if matches!(ended, Ended::Refused) {
            warn!("[WhisperService] Twitch refused the whisper subscription, listener stopped");
            return;
        }
        let (wait, after) = next_wait(retry, &ended);
        let over_budget = matches!(ended, Ended::OverBudget);
        if over_budget {
            if crate::services::eventsub_budget::note_over_budget(SocketOwner::Whispers) {
                crate::services::eventsub_budget::log_episode("whispers", wait);
            }
        } else {
            debug!("[WhisperService] Reconnecting in {}s", wait);
        }
        // Coming back to the app cuts the wait short, and so does another
        // socket of this app freeing its slot while this one waits out the
        // account's socket limit.
        tokio::select! {
            _ = sleep(Duration::from_secs(wait)) => {}
            _ = WAKE.notified() => debug!("[WhisperService] App resumed, reconnecting now"),
            woke = crate::services::eventsub_budget::wait(wait), if over_budget => {
                if woke {
                    debug!("[WhisperService] A socket slot may have freed, reconnecting now");
                }
            }
        }
        retry = after;
    }
}

async fn run_session(user_id: &str, token: &str, app_handle: &AppHandle) -> Ended {
    debug!("[WhisperService] Connecting to Twitch EventSub for whispers...");
    let (ws_stream, session) = match EventSubService::open_session(crate::services::eventsub_service::session_url()).await {
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
        Ok(()) => {
            debug!("[WhisperService] Successfully subscribed to whisper events");
            live.mark_subscribed();
            if crate::services::eventsub_budget::note_room(SocketOwner::Whispers) {
                crate::services::eventsub_budget::log_room("whispers");
            }
        }
        Err(SubscribeError::Refused(e)) => {
            error!("[WhisperService] Failed to subscribe to whispers: {}", e);
            return Ended::Refused;
        }
        Err(SubscribeError::OverBudget(e)) => {
            warn!("[WhisperService] Failed to subscribe to whispers: {}", e);
            return Ended::OverBudget;
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
                            live = live.moved(SocketOwner::Whispers, &session.id);
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
        Err(classify_refusal(status.as_u16(), &response_text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_limit_waits_longer_than_a_network_failure() {
        assert!(matches!(
            classify_refusal(429, r#"{"error":"Too Many Requests","status":429,"message":"number of websocket transports limit exceeded"}"#),
            SubscribeError::OverBudget(_)
        ));
        assert!(matches!(classify_refusal(429, "slow down"), SubscribeError::Retry(_)));
        assert!(matches!(classify_refusal(403, ""), SubscribeError::Refused(_)));
        assert!(matches!(classify_refusal(500, ""), SubscribeError::Retry(_)));
        // Over the limit: at least a minute, doubling to the cap.
        assert_eq!(next_wait(5, &Ended::OverBudget), (60, 120));
        assert_eq!(next_wait(240, &Ended::OverBudget), (240, 300));
        // A failure backs off from where it stands; a healthy session resets.
        assert_eq!(next_wait(5, &Ended::Failed), (5, 10));
        assert_eq!(next_wait(160, &Ended::Dropped(Duration::from_secs(1))), (160, 300));
        assert_eq!(next_wait(160, &Ended::Dropped(HEALTHY_SESSION)), (5, 5));
    }
}
