use futures_util::{SinkExt, StreamExt};
use log::{debug, error, warn};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use crate::rt::AppHandle;
use tokio::task::JoinHandle;
use tokio::time::{interval_at, sleep, timeout, Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

use crate::services::account_automations::own_topics::{self, OwnState, SubscribeVerdict, TopicKind};
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

/// Set by the process's first successful subscribe.
static SUBSCRIBED_ONCE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const HELIX: &str = "https://api.twitch.tv/helix";
/// A topic that is neither held nor refused (a 5xx, a dropped request) is tried again this often.
const OWN_RETRY: Duration = Duration::from_secs(60);

/// What the engine asked of the live session.
enum OwnCmd {
    /// The wanted set changed: add what is missing, delete what is not wanted.
    Sync,
    /// Delete every own-channel subscription now; the sender hears when done.
    Release(tokio::sync::oneshot::Sender<()>),
}

/// The own-channel topics of the engine (D276): which it wants, the live
/// session's command channel, and what Twitch said.
struct OwnHub {
    /// Whose topics, and which, the engine wants. None: nothing.
    wanted: Option<(String, BTreeSet<TopicKind>)>,
    /// The live, subscribed session's command channel; None while there is none.
    tx: Option<tokio::sync::mpsc::UnboundedSender<OwnCmd>>,
    state: OwnState,
}

static OWN_HUB: Mutex<OwnHub> = Mutex::new(OwnHub { wanted: None, tx: None, state: OwnState::new() });

fn hub() -> std::sync::MutexGuard<'static, OwnHub> {
    OWN_HUB.lock().unwrap_or_else(|p| p.into_inner())
}

fn own_changed() {
    crate::services::account_automations::own_topics_changed();
}

/// Hold these own-channel topics on the whisper session while it is up.
/// Replaces the last set and returns at once; the session subscribes what is
/// missing and deletes what is not wanted. A topic no rule uses any more
/// forgets its refusal, so a later rule tries it afresh.
pub fn hold_own_topics(owner_id: &str, kinds: &BTreeSet<TopicKind>) {
    let mut h = hub();
    h.state.refused.retain(|k, _| kinds.contains(k));
    h.wanted = Some((owner_id.to_string(), kinds.clone()));
    if let Some(tx) = &h.tx {
        let _ = tx.send(OwnCmd::Sync);
    }
}

/// Delete the own-channel subscriptions (with the token that made them) and
/// want none. Waits for the deletes for 3 s at most.
pub async fn release_own_topics(_owner_id: &str) {
    let done = {
        let mut h = hub();
        h.wanted = None;
        h.state.refused.clear();
        let Some(tx) = h.tx.clone() else { return };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        if tx.send(OwnCmd::Release(sender)).is_err() {
            return;
        }
        receiver
    };
    let _ = timeout(Duration::from_secs(3), done).await;
}

/// What the session holds and what Twitch refused, for the rules' status.
pub fn own_topic_state() -> OwnState {
    hub().state.clone()
}

/// The listener is replaced or stopped (another account, sign-out): forget
/// everything. The old session's subscriptions end with its socket.
fn forget_own_topics() {
    {
        let mut h = hub();
        h.wanted = None;
        h.state = OwnState::new();
    }
    own_changed();
}

/// A topic Twitch ended (a revocation frame): not held, with its reason.
fn own_revoked(kind: TopicKind, status: &str) {
    {
        let mut h = hub();
        h.state.held.remove(&kind);
        h.state.refused.insert(kind, own_topics::revoked(status));
    }
    own_changed();
}

/// Publishes the session's command channel while it lives, and retires it
/// with the session whatever the exit.
struct OwnSession;

impl OwnSession {
    fn begin(tx: tokio::sync::mpsc::UnboundedSender<OwnCmd>) -> OwnSession {
        {
            let mut h = hub();
            h.tx = Some(tx);
            h.state.session_up = true;
            // A new session subscribes everything again; refusals are tried afresh.
            h.state.held.clear();
            h.state.refused.clear();
        }
        own_changed();
        OwnSession
    }
}

impl Drop for OwnSession {
    fn drop(&mut self) {
        {
            let mut h = hub();
            h.tx = None;
            h.state.session_up = false;
            h.state.held.clear();
        }
        own_changed();
    }
}

async fn post_subscription(client: &reqwest::Client, base: &str, token: &str, body: &serde_json::Value) -> Result<(u16, String), String> {
    let response = client
        .post(format!("{base}/eventsub/subscriptions"))
        .header("Client-ID", CLIENT_ID)
        .header("Authorization", format!("Bearer {token}"))
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;
    let status = response.status().as_u16();
    Ok((status, response.text().await.unwrap_or_default()))
}

async fn delete_subscription(client: &reqwest::Client, base: &str, token: &str, id: &str) -> Result<u16, String> {
    let response = client
        .delete(format!("{base}/eventsub/subscriptions?id={}", urlencoding::encode(id)))
        .header("Client-ID", CLIENT_ID)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;
    Ok(response.status().as_u16())
}

/// One own-channel subscription this session holds, and the token that made
/// it (a delete uses the same token).
struct HeldTopic {
    /// None for a 409 (already held, no id learned).
    id: Option<String>,
    token: String,
}

/// The own-channel subscriptions of one whisper session.
struct OwnSubs {
    client: reqwest::Client,
    base: String,
    user_id: String,
    held: BTreeMap<TopicKind, HeldTopic>,
    /// Read the account's current token for a new subscription (a test turns
    /// this off: no test reads a real token).
    fresh_token: bool,
}

impl OwnSubs {
    fn new(user_id: &str) -> OwnSubs {
        OwnSubs {
            client: crate::services::http::client().clone(),
            base: HELIX.to_string(),
            user_id: user_id.to_string(),
            held: BTreeMap::new(),
            fresh_token: true,
        }
    }

    /// Make the session's subscriptions match what the engine wants. Only the
    /// session's own user's topics are subscribed (the whisper socket follows
    /// the main account); each new one uses the session's current id.
    async fn sync(&mut self, session_id: &str, fallback_token: &str) {
        let (wanted, refused) = {
            let h = hub();
            (h.wanted.clone(), h.state.refused.clone())
        };
        let desired: BTreeSet<TopicKind> = match &wanted {
            Some((owner, kinds)) if *owner == self.user_id => kinds.clone(),
            _ => BTreeSet::new(),
        };
        let held: BTreeSet<TopicKind> = self.held.keys().copied().collect();
        let (add, remove) = own_topics::plan_sync(&desired, &held, &refused);
        for kind in remove {
            if let Some(topic) = self.held.remove(&kind) {
                if let Some(id) = topic.id {
                    let _ = delete_subscription(&self.client, &self.base, &topic.token, &id).await;
                }
            }
            hub().state.held.remove(&kind);
        }
        if !add.is_empty() {
            // A long session outlives its first token: new topics use the current one.
            let token = if self.fresh_token {
                TwitchService::get_token().await.unwrap_or_else(|_| fallback_token.to_string())
            } else {
                fallback_token.to_string()
            };
            for kind in add {
                let body = own_topics::subscription_body(kind, &self.user_id, session_id);
                match post_subscription(&self.client, &self.base, &token, &body).await {
                    Ok((status, text)) => match own_topics::classify_subscribe(status, &text) {
                        SubscribeVerdict::Held => {
                            self.held.insert(kind, HeldTopic { id: own_topics::subscription_id(&text), token: token.clone() });
                            hub().state.held.insert(kind);
                        }
                        SubscribeVerdict::Refused(why) => {
                            warn!("[WhisperService] Twitch refused {} ({status}): {}", kind.sub_type(), why.text());
                            hub().state.refused.insert(kind, why);
                        }
                        SubscribeVerdict::Retry => {
                            warn!("[WhisperService] Subscribing to {} failed ({status}); trying again later", kind.sub_type());
                        }
                    },
                    Err(e) => debug!("[WhisperService] Subscribing to {}: {e}", kind.sub_type()),
                }
            }
        }
        own_changed();
    }

    /// Delete every subscription, each with the token that made it.
    async fn release(&mut self) {
        for (_, topic) in std::mem::take(&mut self.held) {
            if let Some(id) = topic.id {
                let _ = delete_subscription(&self.client, &self.base, &topic.token, &id).await;
            }
        }
        hub().state.held.clear();
        own_changed();
    }
}

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
    crate::services::whisper_sync::note_resumed();
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
    // Synchronously first: the account's history and its generation follow
    // the same call, before anything below is spawned.
    crate::services::whisper_sync::follow_account(user_id.clone());
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
            // Another account: its topics end with the old socket.
            forget_own_topics();
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
            forget_own_topics();
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
        crate::services::whisper_sync::note_socket_down();
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
            // A resubscribe after the socket was down means whispers may have
            // been missed: the sync reads them back. The process's first
            // subscribe is not a gap.
            crate::services::whisper_sync::note_socket_up(!SUBSCRIBED_ONCE.swap(true, std::sync::atomic::Ordering::SeqCst));
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

    // Own-channel topics for automations ride this session (D276). The
    // command channel is published only now that the whisper subscription
    // went through; a guard retires it with the session, whatever the exit.
    // A `session_reconnect` keeps the subscriptions (nothing is sent again);
    // a new session subscribes everything afresh.
    let (own_tx, mut own_rx) = tokio::sync::mpsc::unbounded_channel::<OwnCmd>();
    let _own_session = OwnSession::begin(own_tx);
    let mut own = OwnSubs::new(user_id);
    own.sync(&session_id, token).await;
    let mut own_retry = interval_at(Instant::now() + OWN_RETRY, OWN_RETRY);
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
            Some(cmd) = own_rx.recv() => {
                match cmd {
                    OwnCmd::Sync => own.sync(&session_id, token).await,
                    OwnCmd::Release(done) => {
                        own.release().await;
                        let _ = done.send(());
                    }
                }
                continue;
            }
            _ = own_retry.tick() => {
                own.sync(&session_id, token).await;
                continue;
            }
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
    // Twitch's own clock, on every frame (keepalives too): whispers are
    // stamped with it, and our sends are stamped against it.
    let twitch_ms = chrono::DateTime::parse_from_rfc3339(&ws_msg.metadata.message_timestamp)
        .map(|d| d.timestamp_millis())
        .ok();
    if let Some(t) = twitch_ms {
        crate::services::whisper_sync::note_twitch_time(t);
    }
    match ws_msg.metadata.message_type.as_str() {
        "notification" => {
            // A redelivered whisper would be filed in the inbox twice.
            if !seen.first_time(&ws_msg.metadata.message_id) {
                debug!("[WhisperService] Dropped a repeated notification {}", ws_msg.metadata.message_id);
                return None;
            }
            // Own-channel topics are routed by `payload.subscription.type` to
            // the engine; only a whisper reaches the code below.
            if let Some(kind) = TopicKind::from_sub_type(ws_msg.payload["subscription"]["type"].as_str().unwrap_or("")) {
                if let Some(event) = ws_msg.payload.get("event") {
                    crate::services::account_automations::on_own_topic(kind, event);
                }
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

            // Persist it here, whether or not a window is open to see it.
            tauri::async_runtime::spawn(crate::services::whisper_inbox::record_incoming(
                app_handle.clone(),
                user_id.to_string(),
                whisper_event,
                twitch_ms.unwrap_or_else(|| crate::services::whisper_sync::twitch_now(chrono::Utc::now().timestamp_millis())),
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
            let sub = &ws_msg.payload["subscription"];
            if let Some(kind) = TopicKind::from_sub_type(sub["type"].as_str().unwrap_or("")) {
                warn!("[WhisperService] Own-channel subscription {} revoked: {}", kind.sub_type(), sub["status"].as_str().unwrap_or(""));
                own_revoked(kind, sub["status"].as_str().unwrap_or(""));
                return None;
            }
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

    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset() {
        let mut h = hub();
        h.wanted = None;
        h.tx = None;
        h.state = OwnState::new();
    }

    /// A fake Helix: answers each request with `answer(method, path)` and records
    /// (method, path-and-query, authorization header, body).
    async fn fake_helix(
        answer: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
    ) -> (String, std::sync::Arc<Mutex<Vec<(String, String, String, String)>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let mut got = Vec::new();
                loop {
                    let Ok(n) = sock.read(&mut buf).await else { break };
                    if n == 0 {
                        break;
                    }
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let length = text[..end]
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok()))
                            .unwrap_or(0);
                        if got.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&got).to_string();
                let mut lines = text.lines();
                let first = lines.next().unwrap_or("");
                let mut parts = first.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let auth = text.lines().find_map(|l| l.strip_prefix("authorization: ").or_else(|| l.strip_prefix("Authorization: "))).unwrap_or("").to_string();
                let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                let (status, reply) = answer(&method, &path);
                log.lock().unwrap().push((method, path, auth, body));
                let head = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (base, seen)
    }

    fn subs_on(base: &str) -> OwnSubs {
        let mut subs = OwnSubs::new("100");
        subs.base = base.to_string();
        // No test reads a real token: the fallback below signs the request.
        subs.fresh_token = false;
        subs
    }

    fn want(owner: &str, kinds: &[TopicKind]) {
        hub().wanted = Some((owner.to_string(), kinds.iter().copied().collect()));
    }

    #[tokio::test]
    async fn a_sync_subscribes_the_wanted_topics_once_and_a_second_sync_changes_nothing() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let (base, seen) = fake_helix(|_, _| (202, r#"{"data":[{"id":"sub-1"}]}"#.to_string())).await;
        let mut subs = subs_on(&base);
        want("100", &[TopicKind::Follow, TopicKind::Redemption]);
        subs.sync("S1", "T").await;
        subs.sync("S1", "T").await;
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 2, "one subscribe per topic, none repeated");
        let bodies: Vec<serde_json::Value> = requests.iter().map(|r| serde_json::from_str(&r.3).unwrap()).collect();
        assert!(bodies.iter().any(|b| b["type"] == "channel.follow" && b["version"] == "2"
            && b["condition"] == serde_json::json!({ "broadcaster_user_id": "100", "moderator_user_id": "100" })));
        assert!(bodies.iter().any(|b| b["type"] == "channel.channel_points_custom_reward_redemption.add"
            && b["condition"] == serde_json::json!({ "broadcaster_user_id": "100" })), "one unfiltered redemption subscription");
        assert!(bodies.iter().all(|b| b["transport"]["session_id"] == "S1" && b["transport"]["method"] == "websocket"));
        assert_eq!(own_topic_state().held, [TopicKind::Follow, TopicKind::Redemption].into());
        reset();
    }

    #[tokio::test]
    async fn a_topic_for_another_owner_is_never_subscribed() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let (base, seen) = fake_helix(|_, _| (202, "{}".to_string())).await;
        let mut subs = subs_on(&base);
        want("999", &[TopicKind::Follow]);
        subs.sync("S1", "T").await;
        assert!(seen.lock().unwrap().is_empty(), "the session's user is 100; the wanted owner is not");
        reset();
    }

    #[tokio::test]
    async fn a_429_is_recorded_by_its_body_and_never_retried() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let (base, seen) = fake_helix(|_, _| {
            (429, r#"{"message":"the request exceeds the number of subscriptions: limit of 3 enabled or pending subscriptions"}"#.to_string())
        })
        .await;
        let mut subs = subs_on(&base);
        want("100", &[TopicKind::PollBegin]);
        subs.sync("S1", "T").await;
        assert_eq!(own_topic_state().refused[&TopicKind::PollBegin].text(), "Listening on 3 other devices");
        subs.sync("S1", "T").await;
        assert_eq!(seen.lock().unwrap().len(), 1, "no retry");
        reset();
    }

    #[tokio::test]
    async fn a_403_drops_that_topic_and_leaves_the_others() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let (base, _) = fake_helix(|_, _| (202, r#"{"data":[{"id":"x"}]}"#.to_string())).await;
        let (base_403, _) = fake_helix(|_, _| (403, r#"{"message":"subscription missing proper authorization"}"#.to_string())).await;
        let mut subs = subs_on(&base);
        want("100", &[TopicKind::Follow]);
        subs.sync("S1", "T").await;
        subs.base = base_403;
        want("100", &[TopicKind::Follow, TopicKind::PredictionBegin]);
        subs.sync("S1", "T").await;
        let state = own_topic_state();
        assert!(state.held.contains(&TopicKind::Follow));
        assert_eq!(state.refused[&TopicKind::PredictionBegin].text(), "Sign in again");
        reset();
    }

    #[tokio::test]
    async fn a_release_deletes_each_subscription_with_the_token_that_made_it() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let (base, seen) = fake_helix(|method, _| if method == "POST" { (202, r#"{"data":[{"id":"sub-9"}]}"#.to_string()) } else { (204, String::new()) }).await;
        let mut subs = subs_on(&base);
        want("100", &[TopicKind::Follow]);
        subs.held.insert(TopicKind::Follow, HeldTopic { id: Some("sub-9".into()), token: "made-with-this".into() });
        subs.release().await;
        let requests = seen.lock().unwrap().clone();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].0, "DELETE");
        assert!(requests[0].1.contains("id=sub-9"));
        assert!(requests[0].2.contains("made-with-this"));
        assert!(own_topic_state().held.is_empty());
        reset();
    }

    #[test]
    fn a_new_session_starts_clean_and_a_dropped_one_retires_its_channel() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        hub().state.refused.insert(TopicKind::Follow, own_topics::TopicRefusal::LimitFull);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let session = OwnSession::begin(tx);
        assert!(own_topic_state().session_up && own_topic_state().refused.is_empty(), "a new session tries every refusal afresh");
        drop(session);
        assert!(!own_topic_state().session_up);
        assert!(hub().tx.is_none());
        reset();
    }

    #[test]
    fn holding_topics_signals_the_live_session_and_a_changed_set_forgets_old_refusals() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        hub().tx = Some(tx);
        hub().state.refused.insert(TopicKind::PollBegin, own_topics::TopicRefusal::LimitFull);
        hold_own_topics("100", &[TopicKind::Follow].into());
        assert!(matches!(rx.try_recv(), Ok(OwnCmd::Sync)), "the session is told through its command channel");
        assert!(own_topic_state().refused.is_empty(), "a topic no rule uses any more forgets its refusal");
        assert_eq!(hub().wanted, Some(("100".to_string(), [TopicKind::Follow].into())));
        reset();
    }

    #[test]
    fn another_account_forgets_every_topic_before_its_listener_starts() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        hold_own_topics("100", &[TopicKind::Follow].into());
        hub().state.held.insert(TopicKind::Follow);
        forget_own_topics();
        assert!(hub().wanted.is_none() && own_topic_state() == OwnState::new());
        // Source pin: the replace path and the sign-out path both forget, so a primary switch never
        // leaves the old account's topics wanted by the new account's session.
        let src = include_str!("whisper_service.rs");
        let start = &src[src.find("pub async fn start_listening(").unwrap()..];
        let start = &start[..start.find("pub fn stop(").unwrap()];
        assert!(start.contains("forget_own_topics();"));
        let stop = &src[src.find("pub fn stop(").unwrap()..];
        let stop = &stop[..stop.find("pub async fn is_connected(").unwrap()];
        assert!(stop.contains("forget_own_topics();"));
    }

    #[test]
    fn a_revocation_sets_that_topics_status() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        reset();
        hub().state.held.insert(TopicKind::Follow);
        own_revoked(TopicKind::Follow, "authorization_revoked");
        let state = own_topic_state();
        assert!(state.held.is_empty());
        assert_eq!(state.refused[&TopicKind::Follow].text(), "Sign in again");
        reset();
    }

    #[test]
    fn frames_are_routed_by_subscription_type() {
        // Source pin: the whisper arm runs only for a frame that is not an own topic.
        let src = include_str!("whisper_service.rs");
        let handler = &src[src.find("fn handle_text(").unwrap()..];
        let route = handler.find("TopicKind::from_sub_type").expect("routing by subscription type");
        let whisper = handler.find("event.get(\"whisper\")").expect("the whisper arm");
        assert!(route < whisper, "own topics are routed before the whisper parse");
        assert!(handler.contains("account_automations::on_own_topic(kind, event)"));
        assert!(handler.contains("own_revoked(kind"));
    }
}
