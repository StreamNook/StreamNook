// Dedicated EventSub WebSocket for the moderator view (channel.moderate v2).
//
// Separate from eventsub_service.rs, which is tied to the WATCHED STREAM (raid /
// offline / online / channel.update / hype_train for one broadcaster). This one
// is tied to CHAT: it subscribes channel.moderate for each channel the user has
// a chat open in, driven by the IRC channel refcount in irc_service (join 0->1
// -> subscribe_channel, leave 1->0 -> unsubscribe_channel), exactly like the 7TV
// EventAPI client. One shared connection for the whole app; it self-heals on
// disconnect by re-subscribing the tracked set. It emits the SAME
// `eventsub://channel-moderate` event the frontend already consumes, so the mod
// log enriches with the acting moderator's identity wherever chat is open
// (single, offline, MultiNook, popout), with no stream required.
//
// Two Twitch-specific behaviors shape the design:
//   * Twitch closes a subscription-less WebSocket ~10s after welcome, so the
//     socket is opened lazily on the first subscribe and dropped when the last
//     channel leaves.
//   * Subscribing channel.moderate for a channel you don't moderate returns 403;
//     we prune that channel from the tracked set so it isn't retried (otherwise a
//     non-mod with chats open would churn reconnects on a sub-less session).

use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::services::twitch_service::TwitchService;

const EVENTSUB_URL: &str = "wss://eventsub.wss.twitch.tv/ws";
const RECONNECT_DELAY_SECS: u64 = 5;
const WELCOME_TIMEOUT_SECS: u64 = 15;

#[derive(Clone)]
struct ChannelSub {
    channel_name: String, // lowercase twitch login (chat key)
    broadcaster_id: String,
}

enum Cmd {
    Subscribe(ChannelSub),
    Unsubscribe(String), // channel name
}

struct Service {
    // Desired state: channels with an open chat that we should subscribe (keyed
    // by lowercase channel name). The connection task re-subscribes every entry
    // on each (re)connect, so a dropped socket self-heals.
    subs: Arc<RwLock<HashMap<String, ChannelSub>>>,
    cmd_tx: mpsc::UnboundedSender<Cmd>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

/// Initialize the singleton. The socket is opened lazily on the first subscribe
/// (Twitch closes a subscription-less connection), so this just wires up the
/// command channel and spawns the background task. Idempotent.
pub fn init(app_handle: AppHandle) {
    if SERVICE.get().is_some() {
        return;
    }
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    let subs: Arc<RwLock<HashMap<String, ChannelSub>>> = Arc::new(RwLock::new(HashMap::new()));
    let service = Service {
        subs: subs.clone(),
        cmd_tx,
    };
    // init() runs from the Tauri setup hook, which is OUTSIDE the Tokio runtime,
    // so a bare tokio::spawn panics. Use tauri::async_runtime::spawn (same idiom
    // as seventv_eventapi).
    tauri::async_runtime::spawn(connection_loop(app_handle, subs, cmd_rx));
    let _ = SERVICE.set(service);
}

/// Subscribe channel.moderate for a channel whose chat just opened. Idempotent.
/// Safe to call for every joined channel: if you don't moderate it the Helix sub
/// 403s and the channel is pruned, so it costs one request and never retries.
pub async fn subscribe_channel(channel_name: &str, broadcaster_id: &str) {
    let Some(svc) = SERVICE.get() else {
        return;
    };
    if broadcaster_id.is_empty() {
        return;
    }
    let key = channel_name.to_lowercase();
    let sub = ChannelSub {
        channel_name: key.clone(),
        broadcaster_id: broadcaster_id.to_string(),
    };
    {
        let mut map = svc.subs.write().await;
        if map.contains_key(&key) {
            return; // already tracked (IRC reconnect re-running the hook)
        }
        map.insert(key, sub.clone());
    }
    let _ = svc.cmd_tx.send(Cmd::Subscribe(sub));
    debug!("[EventSub Mod] tracking channel {}", channel_name);
}

/// Stop tracking a channel (last chat consumer left).
pub async fn unsubscribe_channel(channel_name: &str) {
    let Some(svc) = SERVICE.get() else {
        return;
    };
    let key = channel_name.to_lowercase();
    if svc.subs.write().await.remove(&key).is_some() {
        let _ = svc.cmd_tx.send(Cmd::Unsubscribe(key));
    }
}

/// Drop all subscriptions (full chat teardown, e.g. logout / stop()).
pub async fn clear_all() {
    let Some(svc) = SERVICE.get() else {
        return;
    };
    let keys: Vec<String> = {
        let mut map = svc.subs.write().await;
        let ks = map.keys().cloned().collect();
        map.clear();
        ks
    };
    for k in keys {
        let _ = svc.cmd_tx.send(Cmd::Unsubscribe(k));
    }
}

enum SubOutcome {
    Created(String),
    Forbidden, // not a mod (403) or missing scope (401): don't retry this channel
    Transient, // network / 5xx / parse: leave tracked, retry on reconnect
}

/// Every topic subscribed per moderated channel. `channel.moderate` is the
/// gate: a 403 there means "not a mod" and prunes the channel. The others
/// need scopes older tokens may lack (`moderator:manage:automod`,
/// `moderator:read:suspicious_users`); a 401/403 on them skips the topic
/// for this session without touching the channel.
const TOPICS: &[(&str, &str)] = &[
    ("channel.moderate", "2"),
    ("automod.message.hold", "2"),
    ("automod.message.update", "2"),
    ("channel.suspicious_user.message", "1"),
    ("channel.suspicious_user.update", "1"),
];

/// Subscribe every topic for one channel. Returns the created ids, or
/// `Forbidden` when the gate topic itself is refused.
async fn subscribe_all(
    broadcaster_id: &str,
    moderator_user_id: &str,
    session_id: &str,
) -> Result<Vec<String>, SubOutcome> {
    let mut ids = Vec::new();
    for (i, (topic, version)) in TOPICS.iter().enumerate() {
        match create_subscription(topic, version, broadcaster_id, moderator_user_id, session_id).await {
            SubOutcome::Created(id) => ids.push(id),
            SubOutcome::Forbidden if i == 0 => return Err(SubOutcome::Forbidden),
            SubOutcome::Transient if i == 0 => return Err(SubOutcome::Transient),
            _ => {} // optional topic unavailable (scope); keep going
        }
    }
    Ok(ids)
}

/// POST one EventSub subscription bound to this websocket session.
async fn create_subscription(
    sub_type: &str,
    version: &str,
    broadcaster_id: &str,
    moderator_user_id: &str,
    session_id: &str,
) -> SubOutcome {
    if broadcaster_id.is_empty() || moderator_user_id.is_empty() {
        return SubOutcome::Transient;
    }
    let token = match TwitchService::get_token().await {
        Ok(t) => t,
        Err(_) => return SubOutcome::Transient,
    };
    let client_id = env!("TWITCH_APP_CLIENT_ID");
    let client = crate::services::http::client().clone();
    let body = serde_json::json!({
        "type": sub_type,
        "version": version,
        "condition": {
            "broadcaster_user_id": broadcaster_id,
            "moderator_user_id": moderator_user_id
        },
        "transport": { "method": "websocket", "session_id": session_id }
    });
    let resp = match client
        .post("https://api.twitch.tv/helix/eventsub/subscriptions")
        .header("Client-ID", client_id)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => return SubOutcome::Transient,
    };
    let status = resp.status().as_u16();
    if resp.status().is_success() {
        let id = resp.json::<Value>().await.ok().and_then(|j| {
            j.pointer("/data/0/id")
                .and_then(|v| v.as_str())
                .map(String::from)
        });
        match id {
            Some(id) => {
                debug!(
                    "[EventSub Mod] subscribed {} for {}",
                    sub_type, broadcaster_id
                );
                SubOutcome::Created(id)
            }
            None => SubOutcome::Transient,
        }
    } else if status == 403 || status == 401 {
        debug!(
            "[EventSub Mod] {} not authorized for {} (HTTP {}), skipping",
            sub_type, broadcaster_id, status
        );
        SubOutcome::Forbidden
    } else {
        debug!(
            "[EventSub Mod] {} subscribe for {} failed (HTTP {})",
            sub_type, broadcaster_id, status
        );
        SubOutcome::Transient
    }
}

async fn delete_subscription(sub_id: &str) {
    let Ok(token) = TwitchService::get_token().await else {
        return;
    };
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
}

async fn connection_loop(
    app_handle: AppHandle,
    subs: Arc<RwLock<HashMap<String, ChannelSub>>>,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
) {
    loop {
        // Don't hold an idle socket: Twitch closes a subscription-less session
        // ~10s after welcome. Wait for the first channel before connecting.
        if subs.read().await.is_empty() {
            match cmd_rx.recv().await {
                Some(Cmd::Subscribe(_)) => {} // queued channel is in `subs`; connect
                Some(Cmd::Unsubscribe(_)) => continue,
                None => return,
            }
            if subs.read().await.is_empty() {
                continue;
            }
        }

        if let Err(e) = connect_and_run(&app_handle, &subs, &mut cmd_rx).await {
            error!("[EventSub Mod] connection ended: {}", e);
        }

        // Only reconnect if channels still want coverage.
        if !subs.read().await.is_empty() {
            sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
            debug!("[EventSub Mod] reconnecting...");
        }
    }
}

async fn connect_and_run(
    app_handle: &AppHandle,
    subs: &Arc<RwLock<HashMap<String, ChannelSub>>>,
    cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
) -> Result<()> {
    let (ws, _) = connect_async(EVENTSUB_URL).await?;
    let (mut write, mut read) = ws.split();

    // Wait for session_welcome -> session id + keepalive timeout.
    let mut session_id: Option<String> = None;
    let mut keepalive_secs: u64 = 30;
    while session_id.is_none() {
        match timeout(Duration::from_secs(WELCOME_TIMEOUT_SECS), read.next()).await {
            Ok(Some(Ok(Message::Text(txt)))) => {
                if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                    if v.pointer("/metadata/message_type").and_then(|s| s.as_str())
                        == Some("session_welcome")
                    {
                        session_id = v
                            .pointer("/payload/session/id")
                            .and_then(|s| s.as_str())
                            .map(String::from);
                        if let Some(k) = v
                            .pointer("/payload/session/keepalive_timeout_seconds")
                            .and_then(|s| s.as_u64())
                        {
                            keepalive_secs = k;
                        }
                    }
                }
            }
            Ok(Some(Ok(Message::Ping(d)))) => {
                write.send(Message::Pong(d)).await?;
            }
            Ok(Some(Ok(Message::Close(_)))) | Ok(None) => return Ok(()),
            Ok(Some(Err(e))) => return Err(e.into()),
            Err(_) => {
                warn!("[EventSub Mod] no welcome in {}s", WELCOME_TIMEOUT_SECS);
                return Ok(());
            }
            _ => {}
        }
    }
    let session_id = session_id.unwrap();
    debug!("[EventSub Mod] connected (session {})", session_id);

    // The moderator_user_id condition = the authenticated user.
    let moderator_user_id = TwitchService::get_user_info()
        .await
        .map(|u| u.id)
        .unwrap_or_default();

    // Active subscription ids for THIS session, keyed by channel name.
    let mut active: HashMap<String, Vec<String>> = HashMap::new();

    // (Re)subscribe the full desired set (covers reconnect + channels added while
    // the socket was down). Prune channels we don't moderate.
    {
        let snapshot: Vec<ChannelSub> = subs.read().await.values().cloned().collect();
        for sub in snapshot {
            match subscribe_all(&sub.broadcaster_id, &moderator_user_id, &session_id).await {
                Ok(ids) => {
                    active.insert(sub.channel_name.clone(), ids);
                }
                Err(SubOutcome::Forbidden) => {
                    subs.write().await.remove(&sub.channel_name);
                }
                Err(_) => {}
            }
        }
    }

    // If nothing is (or can be) subscribed, drop the socket rather than let
    // Twitch close it on the idle timeout.
    if active.is_empty() && subs.read().await.is_empty() {
        return Ok(());
    }

    let read_timeout = Duration::from_secs(keepalive_secs + 10);

    loop {
        tokio::select! {
            read_res = timeout(read_timeout, read.next()) => {
                match read_res {
                    Err(_) => {
                        warn!("[EventSub Mod] no frames in {}s, reconnecting", read_timeout.as_secs());
                        return Ok(());
                    }
                    Ok(None) => return Ok(()),
                    Ok(Some(Ok(Message::Text(txt)))) => {
                        if handle_text(&txt, app_handle) {
                            return Ok(()); // server asked us to reconnect
                        }
                    }
                    Ok(Some(Ok(Message::Ping(d)))) => {
                        write.send(Message::Pong(d)).await?;
                    }
                    Ok(Some(Ok(Message::Close(_)))) => return Ok(()),
                    Ok(Some(Err(e))) => return Err(e.into()),
                    Ok(Some(Ok(_))) => {}
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(Cmd::Subscribe(sub)) => {
                        if !active.contains_key(&sub.channel_name) {
                            match subscribe_all(&sub.broadcaster_id, &moderator_user_id, &session_id).await {
                                Ok(ids) => { active.insert(sub.channel_name.clone(), ids); }
                                Err(SubOutcome::Forbidden) => { subs.write().await.remove(&sub.channel_name); }
                                Err(_) => {}
                            }
                        }
                    }
                    Some(Cmd::Unsubscribe(name)) => {
                        if let Some(ids) = active.remove(&name) {
                            for id in ids {
                                delete_subscription(&id).await;
                            }
                        }
                        if subs.read().await.is_empty() {
                            return Ok(()); // no channels left; drop the socket
                        }
                    }
                    None => return Ok(()),
                }
            }
        }
    }
}

/// Handle one server frame. Returns true if the server asked us to reconnect.
fn handle_text(txt: &str, app_handle: &AppHandle) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(txt) else {
        return false;
    };
    match v
        .pointer("/metadata/message_type")
        .and_then(|s| s.as_str())
        .unwrap_or("")
    {
        "notification" => {
            let sub_type = v
                .pointer("/metadata/subscription_type")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            let Some(event) = v.pointer("/payload/event") else {
                return false;
            };
            match sub_type {
                "channel.moderate" => {
                    // Same event name the stream EventSub service used; the main
                    // window and each popout feed their own mod-log store.
                    let _ = app_handle.emit("eventsub://channel-moderate", event);
                }
                "automod.message.hold" => {
                    if let Some(row) = crate::services::automod_queue::AutomodQueue::from_hold_event(event) {
                        let _ = app_handle.emit("eventsub://automod-hold", &row);
                        crate::services::automod_queue::AutomodQueue::hold(row);
                    }
                }
                "automod.message.update" => {
                    if let Some((channel, message_id, status)) =
                        crate::services::automod_queue::AutomodQueue::update(event)
                    {
                        let _ = app_handle.emit(
                            "eventsub://automod-update",
                            serde_json::json!({
                                "channel": channel,
                                "message_id": message_id,
                                "status": status,
                                "moderator": event.pointer("/moderator_user_name").and_then(|x| x.as_str()),
                            }),
                        );
                    }
                }
                "channel.suspicious_user.message" => {
                    let status = crate::services::suspicious_users::SuspiciousUsers::note(event);
                    // Restricted users never reach IRC: their message enters chat
                    // through the ordinary publish path (rule engine stamps it).
                    if status.as_deref() == Some("restricted") {
                        if let Some(msg) = crate::services::suspicious_users::SuspiciousUsers::synthesize(event) {
                            tauri::async_runtime::spawn(async move {
                                crate::services::providers::publish_chat_message(&msg).await;
                            });
                        }
                    }
                }
                "channel.suspicious_user.update" => {
                    let status = crate::services::suspicious_users::SuspiciousUsers::note(event);
                    let _ = app_handle.emit(
                        "eventsub://suspicious-user-update",
                        serde_json::json!({
                            "channel": event.pointer("/broadcaster_user_login").and_then(|x| x.as_str()).map(|s| s.to_lowercase()),
                            "user_id": event.pointer("/user_id").and_then(|x| x.as_str()),
                            "user_login": event.pointer("/user_login").and_then(|x| x.as_str()),
                            "status": status,
                            "moderator": event.pointer("/moderator_user_name").and_then(|x| x.as_str()),
                        }),
                    );
                }
                _ => {}
            }
            false
        }
        "session_reconnect" => {
            debug!("[EventSub Mod] server requested reconnect");
            true
        }
        "revocation" => {
            warn!("[EventSub Mod] subscription revoked: {}", txt);
            false
        }
        _ => false, // session_keepalive, etc.
    }
}
