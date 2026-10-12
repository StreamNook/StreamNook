use crate::services::twitch_service::TwitchService;
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use crate::rt::AppHandle;
use tauri::Emitter;
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, timeout, Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
/// Wait before the one retry of a subscription that failed in transit, was
/// rate limited, or hit a Twitch server error.
const SUBSCRIBE_RETRY: Duration = Duration::from_secs(1);
const WELCOME_TIMEOUT_SECS: u64 = 15;
/// Wait before a fresh reconnect after the socket ends; doubles per failure.
const RECONNECT_MIN_SECS: u64 = 5;
const RECONNECT_MAX_SECS: u64 = 300;
/// A connection that ends sooner than this counts as a failure for the
/// backoff, so a socket Twitch keeps closing cannot spin.
const HEALTHY_CONNECTION: Duration = Duration::from_secs(60);

pub(crate) type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
/// A reconnect-URL connection still waiting for its welcome. All three
/// sockets hold one while the old connection keeps delivering.
pub(crate) type PendingSession = Pin<Box<dyn Future<Output = Result<(WsStream, Session)>> + Send>>;

/// The wait before the next fresh reconnect, and the wait after that.
fn reconnect_backoff(retry_secs: u64, healthy: bool) -> (u64, u64) {
    if healthy {
        (RECONNECT_MIN_SECS, RECONNECT_MIN_SECS)
    } else {
        (retry_secs, (retry_secs * 2).min(RECONNECT_MAX_SECS))
    }
}

/// The app's EventSub sockets. Twitch allows three sockets with enabled
/// subscriptions per user token and the app runs one of each, all listed by
/// the same `GET /eventsub/subscriptions`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketOwner {
    /// This service: the watched stream (raid, online/offline, update, hype train).
    Watch,
    /// `eventsub_moderation`: mod log, AutoMod and suspicious users per open chat.
    Moderation,
    /// `whisper_service`: `user.whisper.message`.
    Whispers,
}

impl SocketOwner {
    fn label(self) -> &'static str {
        match self {
            SocketOwner::Watch => "watch",
            SocketOwner::Moderation => "moderation",
            SocketOwner::Whispers => "whispers",
        }
    }
}

#[derive(Debug, Clone)]
struct KnownSession {
    id: String,
    owner: SocketOwner,
    live: bool,
}

/// Every session this process opened: the live ones, and the last few that
/// ended so their owner still recognises its leftovers on the next connect.
static SESSIONS: std::sync::Mutex<Vec<KnownSession>> = std::sync::Mutex::new(Vec::new());
const RETIRED_SESSIONS_KEPT: usize = 16;

fn known_sessions() -> Vec<KnownSession> {
    SESSIONS.lock().unwrap_or_else(|p| p.into_inner()).clone()
}

/// Marks an EventSub session live for as long as the guard lives. Hold it in
/// the connection's own scope so every exit (close, error, a cancelled
/// future) retires the session.
pub struct LiveSession {
    id: String,
}

impl LiveSession {
    pub fn open(owner: SocketOwner, id: &str) -> Self {
        let mut book = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
        book.retain(|s| s.id != id);
        book.push(KnownSession {
            id: id.to_string(),
            owner,
            live: true,
        });
        Self { id: id.to_string() }
    }
}

impl Drop for LiveSession {
    fn drop(&mut self) {
        let mut book = SESSIONS.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = book.iter_mut().find(|s| s.id == self.id) {
            s.live = false;
        }
        // Oldest first, so the earliest retired entries go.
        let mut excess = book
            .iter()
            .filter(|s| !s.live)
            .count()
            .saturating_sub(RETIRED_SESSIONS_KEPT);
        book.retain(|s| {
            if !s.live && excess > 0 {
                excess -= 1;
                false
            } else {
                true
            }
        });
    }
}

/// Aborts a spawned task when its owner goes away, so a connection's helper
/// task cannot outlive the connection.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Message ids a socket already handled. Twitch may deliver a message more
/// than once, around a reconnect-URL move in particular, and its docs say to
/// drop a repeated `message_id`. One per connection, carried across a move.
pub(crate) struct SeenMessages {
    order: VecDeque<String>,
    ids: HashSet<String>,
}

impl SeenMessages {
    const KEEP: usize = 256;

    pub(crate) fn new() -> Self {
        Self { order: VecDeque::new(), ids: HashSet::new() }
    }

    /// True the first time `id` is seen. An empty id is always new.
    pub(crate) fn first_time(&mut self, id: &str) -> bool {
        if id.is_empty() {
            return true;
        }
        if !self.ids.insert(id.to_string()) {
            return false;
        }
        self.order.push_back(id.to_string());
        if self.order.len() > Self::KEEP {
            if let Some(oldest) = self.order.pop_front() {
                self.ids.remove(&oldest);
            }
        }
        true
    }
}

/// What the connect-time cleanup does with one listed subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupVerdict {
    /// Bound to a socket that is open in this process.
    KeepLive(SocketOwner),
    /// Twitch still lists it enabled and no socket here owns it: another app
    /// instance or device signed in as this user, or another socket's session
    /// Twitch has not marked closed yet.
    KeepUnowned,
    /// Left behind by an earlier session of the socket that is connecting.
    DropOwn,
    /// Twitch reports its session gone (any status other than `enabled`).
    DropDead,
}

/// Judge one subscription for the socket `me`. `None` for transports other
/// than websocket, which no socket here created.
fn cleanup_verdict(
    sub: &SubscriptionInfo,
    sessions: &[KnownSession],
    me: SocketOwner,
) -> Option<CleanupVerdict> {
    if sub.transport.method != "websocket" {
        return None;
    }
    let session_id = sub.transport.session_id.as_deref().unwrap_or("");
    Some(match sessions.iter().find(|s| s.id == session_id) {
        Some(s) if s.live => CleanupVerdict::KeepLive(s.owner),
        Some(s) if s.owner == me => CleanupVerdict::DropOwn,
        _ if sub.status != "enabled" => CleanupVerdict::DropDead,
        _ => CleanupVerdict::KeepUnowned,
    })
}

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
pub(crate) struct Session {
    pub(crate) id: String,
    status: String,
    connected_at: String,
    // Null in `session_reconnect`; only the welcome carries it.
    pub(crate) keepalive_timeout_seconds: Option<u64>,
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

pub struct EventSubService {
    connected: Arc<RwLock<bool>>,
    session_id: Arc<RwLock<Option<String>>>,
    // The broadcaster the running task follows (connected, connecting or
    // waiting to reconnect), with the generation that started it. None once
    // the task has stopped: shut down, or given up after a sign-out.
    following: Arc<RwLock<Option<(u64, String)>>>,
    // Shutdown signal sender - when dropped or sent, the background task will stop
    shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
    // Bumped by every connect. A replaced connection's task only resets the
    // shared status when no newer connection has started since.
    generation: Arc<AtomicU64>,
}

impl EventSubService {
    pub fn new() -> Self {
        Self {
            connected: Arc::new(RwLock::new(false)),
            session_id: Arc::new(RwLock::new(None)),
            following: Arc::new(RwLock::new(None)),
            shutdown_tx: Arc::new(RwLock::new(None)),
            generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The broadcaster a running watch socket follows, if one runs.
    pub async fn following(&self) -> Option<String> {
        self.following.read().await.as_ref().map(|(_, id)| id.clone())
    }

    pub async fn is_connected(&self) -> bool {
        *self.connected.read().await
    }

    pub async fn get_session_id(&self) -> Option<String> {
        self.session_id.read().await.clone()
    }

    /// Delete the subscriptions this socket is entitled to remove before it
    /// connects: leftovers from its own earlier sessions, and any whose
    /// session Twitch reports gone. The list covers every socket on this user
    /// token, so subscriptions bound to the live moderation and whisper
    /// sockets (or another instance's) are left alone.
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

        let sessions = known_sessions();
        let mut kept_live: HashMap<&'static str, usize> = HashMap::new();
        let (mut kept_unowned, mut own, mut dead, mut deleted) = (0, 0, 0, 0);
        for sub in &subscriptions.data {
            let Some(verdict) = cleanup_verdict(sub, &sessions, SocketOwner::Watch) else {
                continue;
            };
            debug!(
                "[EventSub] cleanup: {} ({}) on session {} -> {:?}",
                sub.subscription_type,
                sub.status,
                sub.transport.session_id.as_deref().unwrap_or("-"),
                verdict
            );
            match verdict {
                CleanupVerdict::KeepLive(owner) => {
                    *kept_live.entry(owner.label()).or_default() += 1;
                    continue;
                }
                CleanupVerdict::KeepUnowned => {
                    kept_unowned += 1;
                    continue;
                }
                CleanupVerdict::DropOwn => own += 1,
                CleanupVerdict::DropDead => dead += 1,
            }

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

        let mut live: Vec<String> = kept_live
            .iter()
            .map(|(owner, n)| format!("{owner} {n}"))
            .collect();
        live.sort();
        info!(
            "[EventSub] watch connect cleanup: deleted {deleted} (own old sessions {own}, dead {dead}); kept on live sockets [{}], unowned enabled {kept_unowned}",
            live.join(", ")
        );
    }

    /// Follow `broadcaster_id` on a fresh connection. Returns at once: the
    /// cleanup, connect and subscribes run on the spawned task, which keeps
    /// the socket up until `disconnect` or the next call.
    pub async fn connect_and_listen(
        &self,
        broadcaster_id: String,
        app_handle: AppHandle,
    ) -> Result<()> {
        // First, stop any existing connection
        self.disconnect().await;

        let connected = self.connected.clone();
        let session_id = self.session_id.clone();
        let following = self.following.clone();
        let generation = self.generation.clone();
        let my_generation = generation.fetch_add(1, Ordering::SeqCst) + 1;
        *following.write().await = Some((my_generation, broadcaster_id.clone()));

        // Create a shutdown channel
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);

        // Store the sender so we can signal shutdown later
        {
            let mut tx = self.shutdown_tx.write().await;
            *tx = Some(shutdown_tx);
        }

        tokio::spawn(async move {
            // Runs until shutdown (disconnect, or the next connect_and_listen),
            // or until the user signs out.
            tokio::select! {
                _ = shutdown_rx.recv() => {
                    debug!("EventSub shutdown signal received");
                }
                _ = Self::connection_loop(
                    &broadcaster_id,
                    &app_handle,
                    &connected,
                    &session_id,
                    &generation,
                    my_generation,
                ) => {}
            }

            // Mark as disconnected, unless a newer connection owns the status now.
            if generation.load(Ordering::SeqCst) == my_generation {
                let mut conn = connected.write().await;
                *conn = false;
                let mut sess = session_id.write().await;
                *sess = None;
            }
            // Checked under the lock: a newer connect may have set it already.
            {
                let mut follows = following.write().await;
                if follows.as_ref().is_some_and(|(g, _)| *g == my_generation) {
                    *follows = None;
                }
            }

            debug!("EventSub task ended");
        });

        Ok(())
    }

    /// Keep the watch socket up until shutdown drops this future. A
    /// `session_reconnect` moves to the new edge inside `run_connection`
    /// without losing subscriptions; any other end reconnects fresh and
    /// resubscribes, with backoff. Stops only when the user signs out.
    async fn connection_loop(
        broadcaster_id: &str,
        app_handle: &AppHandle,
        connected: &Arc<RwLock<bool>>,
        session_id: &Arc<RwLock<Option<String>>>,
        generation: &Arc<AtomicU64>,
        my_generation: u64,
    ) {
        let mut retry = RECONNECT_MIN_SECS;
        loop {
            // This socket's leftovers from earlier sessions go before it
            // subscribes again, so they never count against the cost budget.
            Self::cleanup_existing_subscriptions().await;
            let healthy = match Self::run_connection(broadcaster_id, app_handle, connected, session_id).await
            {
                Ok(lasted) => lasted >= HEALTHY_CONNECTION,
                Err(e) => {
                    error!("EventSub connection error: {}", e);
                    false
                }
            };

            if generation.load(Ordering::SeqCst) == my_generation {
                *connected.write().await = false;
                *session_id.write().await = None;
            }

            let (wait, next) = reconnect_backoff(retry, healthy);
            retry = next;
            info!("[EventSub] watch socket ended, reconnecting in {}s", wait);
            sleep(Duration::from_secs(wait)).await;

            if TwitchService::get_token().await.is_err() {
                debug!("[EventSub] signed out, watch socket stays down");
                return;
            }
        }
    }

    /// Connect to `url` and wait for its `session_welcome`. Shared by all
    /// three sockets, for the first connect and for a reconnect-URL move.
    pub(crate) async fn open_session(url: String) -> Result<(WsStream, Session)> {
        let (mut ws, _) = connect_async(url.as_str()).await?;
        loop {
            let frame = timeout(Duration::from_secs(WELCOME_TIMEOUT_SECS), ws.next())
                .await
                .map_err(|_| anyhow::anyhow!("No welcome message in {}s", WELCOME_TIMEOUT_SECS))?
                .ok_or_else(|| anyhow::anyhow!("Connection closed before welcome message"))??;
            let text = match frame {
                Message::Text(text) => text.to_string(),
                Message::Binary(data) => String::from_utf8(data.to_vec())
                    .map_err(|e| anyhow::anyhow!("Failed to decode binary message: {}", e))?,
                Message::Close(frame) => {
                    return Err(anyhow::anyhow!("Connection closed by server: {:?}", frame));
                }
                // Ping, pong and raw frames: keep waiting for the welcome.
                _ => continue,
            };
            let welcome: EventSubMessage = serde_json::from_str(&text).map_err(|e| {
                error!("Raw message: {}", text);
                anyhow::anyhow!("Failed to parse welcome message: {}", e)
            })?;
            if welcome.metadata.message_type != "session_welcome" {
                return Err(anyhow::anyhow!(
                    "Expected session_welcome message, got: {}",
                    welcome.metadata.message_type
                ));
            }
            let payload: SessionPayload = serde_json::from_value(welcome.payload)?;
            return Ok((ws, payload.session));
        }
    }

    /// One connection, from welcome until it ends. Returns how long it was up.
    async fn run_connection(
        broadcaster_id: &str,
        app_handle: &AppHandle,
        connected: &Arc<RwLock<bool>>,
        session_id: &Arc<RwLock<Option<String>>>,
    ) -> Result<Duration> {
        debug!("Connecting to Twitch EventSub...");
        let (ws_stream, session_info) = Self::open_session(EVENTSUB_URL.to_string()).await?;
        let started = Instant::now();
        let (mut write, mut read) = ws_stream.split();
        let mut keepalive_timeout = session_info.keepalive_timeout_seconds.unwrap_or(10);
        let mut current_id = session_info.id.clone();

        // Store session ID
        {
            let mut sess = session_id.write().await;
            *sess = Some(current_id.clone());
        }

        // Mark as connected
        {
            let mut conn = connected.write().await;
            *conn = true;
        }

        debug!("EventSub session established: {}", current_id);
        debug!(" Keepalive timeout: {} seconds", keepalive_timeout);
        let mut live = LiveSession::open(SocketOwner::Watch, &current_id);

        // Subscribe right away (the welcome means the session is ready).
        // Aborted with this connection, so a replaced session stops posting
        // subscriptions.
        let sess_id = current_id.clone();
        let broadcaster = broadcaster_id.to_string();
        let app_handle_subs = app_handle.clone();
        let _subscriber = AbortOnDrop(tokio::spawn(async move {
            if let Err(e) = Self::subscribe_to_events(&broadcaster, &sess_id, &app_handle_subs).await {
                error!("Failed to subscribe to EventSub events: {}", e);
            }
        }));

        // The new connection Twitch asked for, opening while this one keeps
        // delivering. Twitch's rule: close the old one only after the new
        // welcome arrives.
        let mut migrating: Option<PendingSession> = None;
        let mut seen = SeenMessages::new();

        // The keepalive deadline is enforced on this connection's own reads.
        // Every connection shares the `connected` flag, so a separate task
        // polling it can outlive its connection and switch the next one off.
        // Shutdown arrives by dropping this future (see connect_and_listen),
        // so the loop never reads that flag either.
        loop {
            let keepalive_deadline = Duration::from_secs(keepalive_timeout + 5);
            let frame = tokio::select! {
                frame = timeout(keepalive_deadline, read.next()) => frame,
                moved = async { migrating.as_mut().expect("guarded by is_some").await }, if migrating.is_some() => {
                    migrating = None;
                    match moved {
                        Ok((ws_stream, session)) => {
                            let _ = write.send(Message::Close(None)).await;
                            (write, read) = ws_stream.split();
                            keepalive_timeout = session.keepalive_timeout_seconds.unwrap_or(keepalive_timeout);
                            if session.id != current_id {
                                // Register the new id before the old guard drops.
                                live = LiveSession::open(SocketOwner::Watch, &session.id);
                                current_id = session.id;
                                *session_id.write().await = Some(current_id.clone());
                            }
                            info!("[EventSub] watch socket moved to the reconnect URL, subscriptions carried over");
                        }
                        Err(e) => {
                            warn!("[EventSub] reconnect URL failed, staying on the old connection until it closes: {}", e);
                        }
                    }
                    continue;
                }
            };

            let msg = match frame {
                Ok(Some(msg)) => msg,
                Ok(None) => break,
                Err(_) => {
                    error!(
                        "No message received for {} seconds, connection may be dead",
                        keepalive_deadline.as_secs()
                    );
                    break;
                }
            };

            match msg {
                Ok(Message::Text(text)) => match Self::handle_message(&text, app_handle, &mut seen).await {
                    Ok(Some(reconnect_url)) if migrating.is_none() => {
                        info!("[EventSub] Twitch asked the watch socket to reconnect, moving to the new URL");
                        migrating = Some(Box::pin(Self::open_session(reconnect_url)));
                    }
                    Ok(_) => {}
                    Err(e) => error!("Error handling message: {}", e),
                },
                Ok(Message::Close(frame)) => {
                    warn!("EventSub connection closed by server: {:?}", frame);
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
        drop(live);

        Ok(started.elapsed())
    }

    /// The URL from a `session_reconnect` payload.
    pub(crate) fn reconnect_url(payload: serde_json::Value) -> Result<Option<String>> {
        let session_payload: SessionPayload = serde_json::from_value(payload)?;
        Ok(session_payload.session.reconnect_url)
    }

    /// Handle one server frame. Returns the reconnect URL when Twitch sends
    /// `session_reconnect`.
    async fn handle_message(text: &str, app_handle: &AppHandle, seen: &mut SeenMessages) -> Result<Option<String>> {
        // Handle potential empty messages
        if text.trim().is_empty() {
            debug!("Received empty message, ignoring");
            return Ok(None);
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
                if !seen.first_time(&message.metadata.message_id) {
                    debug!("[EventSub] dropped a repeated notification {}", message.metadata.message_id);
                    return Ok(None);
                }
                let notification: NotificationPayload = serde_json::from_value(message.payload)?;
                Self::handle_notification(notification, app_handle).await?;
            }
            "session_reconnect" => {
                debug!("Server requested reconnection");
                return Self::reconnect_url(message.payload);
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

        Ok(None)
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
                    crate::services::watch_session::on_raid(app_handle, &raid_event);
                }
            }
            "stream.offline" => {
                // Handle stream offline events
                if let Ok(offline_event) =
                    serde_json::from_value::<StreamOfflineEvent>(notification.event.clone())
                {
                    debug!("Stream offline: {}", offline_event.broadcaster_user_name);
                    let _ = app_handle.emit("eventsub://offline", &offline_event);
                    crate::services::watch_session::on_offline(
                        app_handle,
                        &offline_event.broadcaster_user_id,
                        &offline_event.broadcaster_user_login,
                    );
                }
            }
            "stream.online" => {
                // Handle stream online events
                if let Ok(online_event) =
                    serde_json::from_value::<StreamOnlineEvent>(notification.event.clone())
                {
                    debug!("Stream online: {}", online_event.broadcaster_user_name);
                    let _ = app_handle.emit("eventsub://online", &online_event);
                    crate::services::watch_session::on_online(app_handle, &online_event);
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
                    crate::services::watch_session::on_channel_update(app_handle, &update_event);
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
    ) -> Result<()> {
        let token = TwitchService::get_token().await?;
        let client_id = env!("TWITCH_APP_CLIENT_ID");
        let client = crate::services::http::client().clone();

        // Cost 1 each: 4 of the user token's budget of 10.
        let subscriptions = [
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
            // NOTE: no channel.hype_train.* either. Those need the
            // broadcaster's own token, so they fail for any other channel, and
            // every hype-train banner comes from the GQL poll in
            // hype_train_watch.rs.
        ];

        // Each subscription stands alone: one that fails does not stop the
        // rest. A request with no answer, a 429 or a 5xx is retried once; any
        // other refusal is final.
        for (event_type, version, condition) in &subscriptions {
            let subscription_body = serde_json::json!({
                "type": event_type,
                "version": version,
                "condition": condition,
                "transport": {
                    "method": "websocket",
                    "session_id": session_id
                }
            });

            let mut failure: Option<(u16, String)> = None;
            for attempt in 0..2 {
                if attempt > 0 {
                    sleep(SUBSCRIBE_RETRY).await;
                }
                let sent = client
                    .post("https://api.twitch.tv/helix/eventsub/subscriptions")
                    .header("Client-ID", client_id)
                    .header("Authorization", format!("Bearer {}", token))
                    .header("Content-Type", "application/json")
                    .json(&subscription_body)
                    .send()
                    .await;
                match sent {
                    Ok(response) if response.status().is_success() => {
                        debug!("Subscribed to {}", event_type);
                        failure = None;
                        break;
                    }
                    Ok(response) => {
                        let status = response.status();
                        let error_text = response.text().await.unwrap_or_default();
                        failure = Some((status.as_u16(), error_text));
                        if status.as_u16() != 429 && !status.is_server_error() {
                            break;
                        }
                    }
                    Err(e) => failure = Some((0, e.to_string())),
                }
            }

            if let Some((status, error_text)) = failure {
                error!("Failed to subscribe to {}: {}", event_type, error_text);
                // Surface the failure to the UI so a silently-broken feed
                // (e.g. a token missing a scope) isn't invisible. Status 0
                // means the request never got an answer.
                let _ = app_handle.emit(
                    "eventsub://subscription-failed",
                    serde_json::json!({
                        "type": event_type,
                        "status": status,
                        "error": error_text,
                    }),
                );
            }
        }

        // NOTE: user.whisper.message is not here either. whisper_service owns
        // it on its own always-on socket, which also files each whisper in
        // the inbox; a second subscription here would deliver whispers that
        // bypass the inbox and die with the watched stream.

        Ok(())
    }

    pub async fn disconnect(&self) {
        debug!("Disconnecting EventSub...");

        // Status only; the shutdown signal below is what ends the connection.
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

        // Clear the session and what it followed
        {
            let mut sess = self.session_id.write().await;
            *sess = None;
        }
        *self.following.write().await = None;

        debug!("EventSub disconnected");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `GET /helix/eventsub/subscriptions` page as Twitch returns it for a
    /// user token: the three sockets' subscriptions side by side, plus
    /// leftovers, another instance's session and a webhook.
    const FIXTURE: &str = r#"{
        "data": [
            {"id": "watch-old-online", "status": "websocket_disconnected", "type": "stream.online", "version": "1",
             "condition": {"broadcaster_user_id": "100"}, "created_at": "2026-01-01T00:00:00Z", "cost": 1,
             "transport": {"method": "websocket", "session_id": "S_watch_old", "connected_at": "2026-01-01T00:00:00Z", "disconnected_at": "2026-01-01T00:05:00Z"}},
            {"id": "watch-old-raid", "status": "enabled", "type": "channel.raid", "version": "1",
             "condition": {"from_broadcaster_user_id": "100"}, "created_at": "2026-01-01T00:00:00Z", "cost": 1,
             "transport": {"method": "websocket", "session_id": "S_watch_old", "connected_at": "2026-01-01T00:00:00Z"}},
            {"id": "mod-moderate", "status": "enabled", "type": "channel.moderate", "version": "2",
             "condition": {"broadcaster_user_id": "200", "moderator_user_id": "1"}, "created_at": "2026-01-01T00:00:00Z", "cost": 0,
             "transport": {"method": "websocket", "session_id": "S_mod_live", "connected_at": "2026-01-01T00:00:00Z"}},
            {"id": "mod-automod", "status": "enabled", "type": "automod.message.hold", "version": "2",
             "condition": {"broadcaster_user_id": "200", "moderator_user_id": "1"}, "created_at": "2026-01-01T00:00:00Z", "cost": 0,
             "transport": {"method": "websocket", "session_id": "S_mod_live", "connected_at": "2026-01-01T00:00:00Z"}},
            {"id": "whisper", "status": "enabled", "type": "user.whisper.message", "version": "1",
             "condition": {"user_id": "1"}, "created_at": "2026-01-01T00:00:00Z", "cost": 0,
             "transport": {"method": "websocket", "session_id": "S_whisper_live", "connected_at": "2026-01-01T00:00:00Z"}},
            {"id": "mod-old-closed", "status": "websocket_disconnected", "type": "channel.moderate", "version": "2",
             "condition": {"broadcaster_user_id": "300", "moderator_user_id": "1"}, "created_at": "2026-01-01T00:00:00Z", "cost": 0,
             "transport": {"method": "websocket", "session_id": "S_mod_old", "connected_at": "2026-01-01T00:00:00Z", "disconnected_at": "2026-01-01T00:01:00Z"}},
            {"id": "mod-old-lagging", "status": "enabled", "type": "channel.moderate", "version": "2",
             "condition": {"broadcaster_user_id": "301", "moderator_user_id": "1"}, "created_at": "2026-01-01T00:00:00Z", "cost": 0,
             "transport": {"method": "websocket", "session_id": "S_mod_old", "connected_at": "2026-01-01T00:00:00Z"}},
            {"id": "other-instance", "status": "enabled", "type": "stream.online", "version": "1",
             "condition": {"broadcaster_user_id": "400"}, "created_at": "2026-01-01T00:00:00Z", "cost": 1,
             "transport": {"method": "websocket", "session_id": "S_other_instance", "connected_at": "2026-01-01T00:00:00Z"}},
            {"id": "previous-run", "status": "websocket_failed_ping_pong", "type": "user.whisper.message", "version": "1",
             "condition": {"user_id": "1"}, "created_at": "2026-01-01T00:00:00Z", "cost": 0,
             "transport": {"method": "websocket", "session_id": "S_previous_run", "connected_at": "2026-01-01T00:00:00Z", "disconnected_at": "2026-01-01T00:02:00Z"}},
            {"id": "webhook", "status": "enabled", "type": "stream.online", "version": "1",
             "condition": {"broadcaster_user_id": "500"}, "created_at": "2026-01-01T00:00:00Z", "cost": 1,
             "transport": {"method": "webhook", "callback": "https://example.com/eventsub"}}
        ],
        "total": 10,
        "total_cost": 4,
        "max_total_cost": 10,
        "pagination": {}
    }"#;

    fn session(id: &str, owner: SocketOwner, live: bool) -> KnownSession {
        KnownSession {
            id: id.to_string(),
            owner,
            live,
        }
    }

    /// This process: the watch socket reconnecting (its old session retired),
    /// the moderation socket live after one earlier reconnect of its own, and
    /// the whisper socket live.
    fn book() -> Vec<KnownSession> {
        vec![
            session("S_watch_old", SocketOwner::Watch, false),
            session("S_mod_old", SocketOwner::Moderation, false),
            session("S_mod_live", SocketOwner::Moderation, true),
            session("S_whisper_live", SocketOwner::Whispers, true),
        ]
    }

    fn verdicts(me: SocketOwner) -> Vec<(String, Option<CleanupVerdict>)> {
        let list: SubscriptionListResponse = serde_json::from_str(FIXTURE).unwrap();
        let sessions = book();
        list.data
            .iter()
            .map(|sub| (sub.id.clone(), cleanup_verdict(sub, &sessions, me)))
            .collect()
    }

    fn deleted(me: SocketOwner) -> Vec<String> {
        verdicts(me)
            .into_iter()
            .filter(|(_, v)| matches!(v, Some(CleanupVerdict::DropOwn | CleanupVerdict::DropDead)))
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn watch_reconnect_never_touches_live_sockets() {
        use CleanupVerdict::*;
        let got: HashMap<String, Option<CleanupVerdict>> = verdicts(SocketOwner::Watch).into_iter().collect();
        assert_eq!(got["watch-old-online"], Some(DropOwn));
        // Its own retired session goes even while Twitch still lists it enabled.
        assert_eq!(got["watch-old-raid"], Some(DropOwn));
        assert_eq!(got["mod-moderate"], Some(KeepLive(SocketOwner::Moderation)));
        assert_eq!(got["mod-automod"], Some(KeepLive(SocketOwner::Moderation)));
        assert_eq!(got["whisper"], Some(KeepLive(SocketOwner::Whispers)));
        assert_eq!(got["mod-old-closed"], Some(DropDead));
        // Another socket's retired session is not ours to judge until Twitch
        // marks it closed.
        assert_eq!(got["mod-old-lagging"], Some(KeepUnowned));
        assert_eq!(got["other-instance"], Some(KeepUnowned));
        assert_eq!(got["previous-run"], Some(DropDead));
        assert_eq!(got["webhook"], None);
    }

    #[test]
    fn each_socket_deletes_only_its_own_and_dead_sessions() {
        assert_eq!(
            deleted(SocketOwner::Watch),
            ["watch-old-online", "watch-old-raid", "mod-old-closed", "previous-run"]
        );
        // Seen from the moderation socket, its own lagging leftover goes and
        // the watch socket's enabled leftover stays.
        assert_eq!(
            deleted(SocketOwner::Moderation),
            ["watch-old-online", "mod-old-closed", "mod-old-lagging", "previous-run"]
        );
        for me in [SocketOwner::Watch, SocketOwner::Moderation, SocketOwner::Whispers] {
            let gone = deleted(me);
            for live in ["mod-moderate", "mod-automod", "whisper", "other-instance", "webhook"] {
                assert!(!gone.iter().any(|id| id == live), "{me:?} deleted {live}");
            }
        }
    }

    /// `session_reconnect` in the shape Twitch documents: the keepalive is
    /// null there, which a plain `u64` field refused to parse.
    #[test]
    fn reconnect_frame_yields_its_url() {
        let frame = r#"{
            "metadata": {"message_id": "84c1e79a-2a4b-4c13-ba0b-4312293e9308", "message_type": "session_reconnect",
                         "message_timestamp": "2022-11-18T09:10:11.634234626Z"},
            "payload": {"session": {"id": "AQoQexAWVYKSTIu4ec_2VAxyuhAB", "status": "reconnecting",
                                    "keepalive_timeout_seconds": null,
                                    "reconnect_url": "wss://eventsub.wss.twitch.tv/ws?reconnect_token=abc",
                                    "connected_at": "2022-11-16T10:11:12.634234626Z"}}
        }"#;
        let message: EventSubMessage = serde_json::from_str(frame).unwrap();
        assert_eq!(message.metadata.message_type, "session_reconnect");
        assert_eq!(
            EventSubService::reconnect_url(message.payload).unwrap().as_deref(),
            Some("wss://eventsub.wss.twitch.tv/ws?reconnect_token=abc")
        );
    }

    #[test]
    fn welcome_frame_carries_id_and_keepalive() {
        let frame = r#"{
            "metadata": {"message_id": "96a3f3b5-5dec-4eed-908e-e11ee657416c", "message_type": "session_welcome",
                         "message_timestamp": "2023-07-19T14:56:51.634234626Z"},
            "payload": {"session": {"id": "AQoQILE98gtqShGmLD7AM6yJThAB", "status": "connected",
                                    "connected_at": "2023-07-19T14:56:51.616329898Z",
                                    "keepalive_timeout_seconds": 10, "reconnect_url": null}}
        }"#;
        let message: EventSubMessage = serde_json::from_str(frame).unwrap();
        let session = serde_json::from_value::<SessionPayload>(message.payload).unwrap().session;
        assert_eq!(session.id, "AQoQILE98gtqShGmLD7AM6yJThAB");
        assert_eq!(session.keepalive_timeout_seconds, Some(10));
        assert_eq!(session.reconnect_url, None);
    }

    #[test]
    fn reconnect_backoff_doubles_to_the_cap_and_resets_when_healthy() {
        let mut retry = RECONNECT_MIN_SECS;
        let mut waits = Vec::new();
        for _ in 0..8 {
            let (wait, next) = reconnect_backoff(retry, false);
            waits.push(wait);
            retry = next;
        }
        assert_eq!(waits, [5, 10, 20, 40, 80, 160, 300, 300]);
        assert_eq!(reconnect_backoff(retry, true), (RECONNECT_MIN_SECS, RECONNECT_MIN_SECS));
    }

    #[test]
    fn live_session_guard_retires_on_drop_and_caps_history() {
        let in_book = |id: &str| known_sessions().into_iter().find(|s| s.id == id);
        {
            let _live = LiveSession::open(SocketOwner::Whispers, "T_guard_whispers");
            let s = in_book("T_guard_whispers").unwrap();
            assert!(s.live);
            assert_eq!(s.owner, SocketOwner::Whispers);
        }
        assert!(!in_book("T_guard_whispers").unwrap().live);

        for i in 0..RETIRED_SESSIONS_KEPT + 4 {
            drop(LiveSession::open(SocketOwner::Watch, &format!("T_guard_cap_{i}")));
        }
        assert!(in_book("T_guard_whispers").is_none());
        assert!(in_book("T_guard_cap_0").is_none());
        assert!(in_book(&format!("T_guard_cap_{}", RETIRED_SESSIONS_KEPT + 3)).is_some());
        assert!(known_sessions().iter().filter(|s| !s.live).count() <= RETIRED_SESSIONS_KEPT);
    }

    #[test]
    fn a_repeated_message_id_is_dropped_within_the_window() {
        let mut seen = SeenMessages::new();
        assert!(seen.first_time("m1"));
        assert!(!seen.first_time("m1"), "a redelivery is dropped");
        assert!(seen.first_time(""), "a frame without an id always passes");
        assert!(seen.first_time(""));
        for i in 0..SeenMessages::KEEP {
            assert!(seen.first_time(&format!("fill{i}")));
        }
        // m1 has aged out of the window; the newest are still remembered.
        assert!(seen.first_time("m1"));
        assert!(!seen.first_time(&format!("fill{}", SeenMessages::KEEP - 1)));
    }
}
