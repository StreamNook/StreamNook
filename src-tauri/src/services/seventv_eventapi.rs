// 7TV EventAPI WebSocket client.
//
// One shared connection for the whole app (not per window), mirroring the
// channel_points_websocket_service pattern. It subscribes to a channel's 7TV
// resources when the channel is JOINed (refcount 0 to 1 in irc_service) and
// unsubscribes when the last consumer leaves (1 to 0). Updates are pushed to
// every WebView window via app_handle.emit, the same idiom eventsub_service
// uses, since these events are infrequent.
//
// emote_set.update is applied as a DELTA. The dispatch carries the whole emote
// (id, alias, flags, host files), so the change goes straight into the parse
// dictionary, the picker cache and the disk dictionary, and windows receive the
// composed rows to patch in place. Nothing re-downloads the channel document: on
// a large channel that document is 14 MB and takes 5 to 8 s, and re-fetching it
// on every change inside this read loop both stalled the socket and, whenever
// the fetch timed out, dropped the change with no retry (2026-09-07).
//
// Cosmetics (entitlement.*) ride the same connection; see handle_entitlement.
//
// Reconnects RESUME the previous session first (the server replays missed
// dispatches), fall back to a fresh subscribe plus a per-channel resync, back
// off exponentially with jitter, and honour the close codes the protocol
// documents (4007 maintenance waits at least five minutes).

use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, timeout, Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::services::emote_service::{self, Emote, EmoteService, SeventvIds, SeventvSetDelta};
use crate::services::irc_service::IrcService;

const EVENTAPI_URL: &str = "wss://events.7tv.io/v3";
const SEVENTV_GQL_URL: &str = "https://7tv.io/v4/gql";

// Opcodes, per the protocol README (github.com/SevenTV/EventAPI, archived).
const OP_DISPATCH: u64 = 0;
const OP_HELLO: u64 = 1;
const OP_RECONNECT: u64 = 4;
const OP_ACK: u64 = 5;
const OP_ERROR: u64 = 6;
const OP_END_OF_STREAM: u64 = 7;
const OP_RESUME: u64 = 34;
const OP_SUBSCRIBE: u64 = 35;
const OP_UNSUBSCRIBE: u64 = 36;

const HELLO_TIMEOUT: Duration = Duration::from_secs(30);
const RESUME_ACK_TIMEOUT: Duration = Duration::from_secs(5);
// Heartbeats are server-to-client only and were measured at 46.9 to 48.6 s
// (2026-09-07); the README calls the connection dead after three missed cycles.
// The live value comes from HELLO each session; this is only the fallback.
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(150);
const HEARTBEAT_MISSES: u64 = 3;
const BACKOFF_MIN: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(60);
// Close code 4007 (maintenance): "reconnect with a significantly greater
// delay, i.e. at least 5 minutes". 4005: rate limited.
const MAINTENANCE_DELAY: Duration = Duration::from_secs(300);
const RATE_LIMITED_DELAY: Duration = Duration::from_secs(60);

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

/// Desired subscription state for one channel. The map of these is the single
/// source of truth: the connection task re-subscribes every entry on each fresh
/// connect, so a dropped socket self-heals.
#[derive(Clone)]
struct ChannelSub {
    channel_name: String, // lowercase channel key (login / slug / identifier)
    channel_id: String,   // the platform's own user id
    // Which platform `channel_id` belongs to, lowercased ("twitch" / "kick" /
    // "youtube"). Translated for the EventAPI condition by `seventv_platform`.
    platform: String,
    emote_set_id: Option<String>,
    // The channel owner's 7TV user id (NOT the platform id): the subject of the
    // passive presence POST. The platform id in this slot got a silent 400 for
    // months; see `resolve_ids`.
    seventv_user_id: Option<String>,
}

enum Cmd {
    Subscribe(ChannelSub),
    Unsubscribe(ChannelSub),
}

struct Service {
    http: reqwest::Client,
    subs: Arc<RwLock<HashMap<String, ChannelSub>>>, // keyed by lowercase channel name
    cmd_tx: mpsc::UnboundedSender<Cmd>,
}

static SERVICE: OnceLock<Service> = OnceLock::new();

/// Initialize the singleton and spawn its connection task. Idempotent.
pub fn init(app_handle: AppHandle, emote_service: Arc<RwLock<EmoteService>>) {
    if SERVICE.get().is_some() {
        return;
    }

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<Cmd>();
    let subs: Arc<RwLock<HashMap<String, ChannelSub>>> = Arc::new(RwLock::new(HashMap::new()));
    let http = crate::services::http::client().clone();

    let service = Service {
        http: http.clone(),
        subs: subs.clone(),
        cmd_tx,
    };

    // The connection task connects immediately and idles until the first
    // subscribe; it re-subscribes from `subs` on every fresh connect.
    // Use tauri::async_runtime::spawn (not tokio::spawn): init() runs from the
    // Tauri setup hook, which is OUTSIDE the Tokio runtime context, so a bare
    // tokio::spawn panics with "there is no reactor running".
    tauri::async_runtime::spawn(connection_loop(
        app_handle,
        emote_service,
        http,
        subs,
        cmd_rx,
    ));

    let _ = SERVICE.set(service);
}

/// Subscribe to a Twitch channel's 7TV events. Thin wrapper over
/// `subscribe_channel_on` so every existing Twitch call site is unchanged.
pub async fn subscribe_channel(channel_name: &str, channel_id: &str) {
    subscribe_channel_on(channel_name, channel_id, "twitch").await;
}

/// Subscribe to a channel's 7TV events on a given platform.
///
/// 7TV supports Kick and YouTube identities natively, so provider channels get
/// the same live emote-set updates and cosmetics entitlements Twitch does. If a
/// platform is not on 7TV, `resolve_ids` finds nothing and we skip quietly,
/// which leaves the join-time fetch as the only source, exactly as before.
///
/// Resolving the ids costs one small GQL lookup at most (or nothing, when the
/// emote fetch already parsed them), never the channel document. Callers on the
/// join path still spawn this rather than await it.
pub async fn subscribe_channel_on(channel_name: &str, channel_id: &str, platform: &str) {
    let Some(svc) = SERVICE.get() else {
        return;
    };

    let key = channel_name.to_lowercase();
    if svc.subs.read().await.contains_key(&key) {
        return; // already subscribed (e.g. IRC reconnect re-running the hook)
    }

    let ids = resolve_ids(&svc.http, channel_id, platform).await;
    if ids.emote_set_id.is_none() && ids.user_id.is_none() {
        debug!(
            "[7TV EventAPI] {} not on 7TV, skipping subscription",
            channel_name
        );
        return;
    }

    let sub = ChannelSub {
        channel_name: key.clone(),
        channel_id: channel_id.to_string(),
        platform: platform.to_lowercase(),
        emote_set_id: ids.emote_set_id,
        seventv_user_id: ids.user_id,
    };
    {
        // Two windows joining the same channel at once both pass the read check
        // above; only the first past this write may register.
        let mut map = svc.subs.write().await;
        if map.contains_key(&key) {
            return;
        }
        map.insert(key.clone(), sub.clone());
    }
    let _ = svc.cmd_tx.send(Cmd::Subscribe(sub.clone()));
    info!(
        "[7TV EventAPI] subscribed {} (set {})",
        key,
        sub.emote_set_id.as_deref().unwrap_or("none")
    );
}

/// Unsubscribe a channel (last consumer left).
pub async fn unsubscribe_channel(channel_name: &str) {
    let Some(svc) = SERVICE.get() else {
        return;
    };
    let key = channel_name.to_lowercase();
    if let Some(sub) = svc.subs.write().await.remove(&key) {
        let _ = svc.cmd_tx.send(Cmd::Unsubscribe(sub));
        debug!("[7TV EventAPI] unsubscribed channel {}", key);
    }
}

/// Drop all subscriptions (full chat teardown). The socket stays up but idle.
pub async fn clear_all() {
    let Some(svc) = SERVICE.get() else {
        return;
    };
    let drained: Vec<ChannelSub> = {
        let mut map = svc.subs.write().await;
        map.drain().map(|(_, v)| v).collect()
    };
    for sub in drained {
        let _ = svc.cmd_tx.send(Cmd::Unsubscribe(sub));
    }
    // Personal emotes are keyed by user, not channel; a full teardown clears them.
    IrcService::clear_all_personal_emotes().await;
}

/// Resolve the channel's active emote set id and 7TV user id.
///
/// First from the ids the emote fetch parsed out of the channel document (no
/// request at all), else one small v4 GQL lookup (~1 KB). This used to download
/// the whole channel document, 14 MB on a large channel, inline on the join
/// path, and then read the PLATFORM id off its root as the 7TV user id, which
/// made every presence POST a silent 400.
async fn resolve_ids(http: &reqwest::Client, channel_id: &str, platform: &str) -> SeventvIds {
    if let Some(ids) = emote_service::seventv_ids_cached(channel_id).await {
        return ids;
    }

    let body = json!({
        "query": "query($platform: Platform!, $id: String!) { users { userByConnection(platform: $platform, platformId: $id) { id style { activeEmoteSet { id } } } } }",
        "variables": { "platform": seventv_platform(platform), "id": channel_id }
    });
    let resp = match http.post(SEVENTV_GQL_URL).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => resp,
        Ok(resp) => {
            warn!(
                "[7TV EventAPI] id lookup for {} answered {}",
                channel_id,
                resp.status()
            );
            return SeventvIds::default();
        }
        Err(e) => {
            warn!("[7TV EventAPI] id lookup failed for {}: {}", channel_id, e);
            return SeventvIds::default();
        }
    };
    let v: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            warn!("[7TV EventAPI] id lookup for {} did not parse: {}", channel_id, e);
            return SeventvIds::default();
        }
    };
    if let Some(errs) = v.get("errors") {
        warn!("[7TV EventAPI] id lookup GraphQL errors for {}: {}", channel_id, errs);
    }
    let user = v.pointer("/data/users/userByConnection");
    let ids = SeventvIds {
        user_id: user
            .and_then(|u| u.get("id"))
            .and_then(|x| x.as_str())
            .map(String::from),
        emote_set_id: user
            .and_then(|u| u.pointer("/style/activeEmoteSet/id"))
            .and_then(|x| x.as_str())
            .map(String::from),
    };
    if ids.user_id.is_some() || ids.emote_set_id.is_some() {
        emote_service::seventv_ids_store(channel_id, ids.clone()).await;
    }
    ids
}

/// Our provider id translated to 7TV's platform name.
///
/// These are NOT the same vocabulary: 7TV calls YouTube **GOOGLE**, and sending
/// "YOUTUBE" is rejected outright rather than ignored. Verified against the live
/// v4 schema, whose Platform enum is TWITCH, DISCORD, GOOGLE, KICK.
fn seventv_platform(provider: &str) -> &'static str {
    match provider {
        "kick" => "KICK",
        "youtube" => "GOOGLE",
        _ => "TWITCH",
    }
}

// Cosmetics entitlement event types subscribed per channel. Subscribing to these
// makes 7TV deliver the currently-present users' entitlements to our session
// unprompted (verified 2026-09-07); the subscription then carries deltas (new
// arrivals, paint/badge changes, removals). The presence POST is a hint on top.
const ENTITLEMENT_TYPES: [&str; 3] = [
    "entitlement.create",
    "entitlement.update",
    "entitlement.delete",
];

// Subscription frames sent over the socket for a channel: the channel's emote
// set (live emote add/remove/rename) plus the channel cosmetics entitlements.
fn subscribe_frames(sub: &ChannelSub) -> Vec<String> {
    frames_for(sub, OP_SUBSCRIBE)
}

fn unsubscribe_frames(sub: &ChannelSub) -> Vec<String> {
    frames_for(sub, OP_UNSUBSCRIBE)
}

fn frames_for(sub: &ChannelSub, op: u64) -> Vec<String> {
    let mut frames = Vec::new();
    if let Some(set_id) = &sub.emote_set_id {
        frames.push(
            json!({
                "op": op,
                "d": { "type": "emote_set.update", "condition": { "object_id": set_id } }
            })
            .to_string(),
        );
    }
    for t in ENTITLEMENT_TYPES {
        frames.push(
            json!({
                "op": op,
                "d": {
                    "type": t,
                    "condition": { "ctx": "channel", "platform": seventv_platform(&sub.platform), "id": sub.channel_id }
                }
            })
            .to_string(),
        );
    }
    frames
}

// POST a passive presence so 7TV counts this session as watching the channel.
// Mirrors the official extension's passive bootstrap. No auth required. The
// subject is the channel owner's 7TV user id (the platform id gets a 400).
// Fire-and-forget and never under a lock: 7TV has answered these in 1 to 7 s,
// and the reconnect path used to hold `subs` for the whole series.
fn spawn_presence(http: reqwest::Client, sub: ChannelSub, session_id: Option<String>) {
    let (Some(subject), Some(session)) = (sub.seventv_user_id.clone(), session_id) else {
        return;
    };
    tokio::spawn(async move {
        let url = format!("https://7tv.io/v3/users/{}/presences", subject);
        let body = json!({
            "kind": 1,
            "passive": true,
            "session_id": session,
            "data": { "platform": seventv_platform(&sub.platform), "id": sub.channel_id }
        });
        match http.post(&url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {}
            Ok(resp) => warn!(
                "[7TV EventAPI] presence for {} answered {}",
                sub.channel_name,
                resp.status()
            ),
            Err(e) => warn!(
                "[7TV EventAPI] presence post failed for {}: {}",
                sub.channel_name, e
            ),
        }
    });
}

/// Re-pull one channel's set after a reconnect that could not RESUME: anything
/// dispatched during the gap was never applied. Spawned; each platform's own
/// store does its own fetch and install.
fn spawn_resync(sub: &ChannelSub, emote_service: Arc<RwLock<EmoteService>>) {
    let sub = sub.clone();
    tokio::spawn(async move {
        match sub.platform.as_str() {
            "kick" => {
                if let Ok(uid) = sub.channel_id.parse::<u64>() {
                    super::providers::kick_emotes::invalidate(&sub.channel_name);
                    super::providers::kick_emotes::refresh(&sub.channel_name, uid).await;
                }
            }
            "youtube" => {
                super::providers::youtube_emotes::invalidate(&sub.channel_name);
                super::providers::youtube_emotes::refresh(&sub.channel_name, &sub.channel_id)
                    .await;
            }
            _ => {
                IrcService::resync_channel_emotes(&sub.channel_name, &sub.channel_id, emote_service)
                    .await;
            }
        }
    });
}

/// How one session ended, for the reconnect policy.
#[derive(Default)]
struct SessionReport {
    /// HELLO arrived, so the backoff resets.
    connected: bool,
    /// The session to RESUME next time, when the server gave us one.
    session_id: Option<String>,
    /// A minimum delay the server asked for (maintenance, rate limit).
    delay_floor: Option<Duration>,
}

/// What the read loop should do after a frame.
enum Flow {
    Continue,
    Reconnect { floor: Option<Duration> },
}

fn op_of(v: &Value) -> u64 {
    v.get("op").and_then(|o| o.as_u64()).unwrap_or(u64::MAX)
}

/// The documented per-close-code delay, if the code carries one.
fn close_delay_floor(code: u16) -> Option<Duration> {
    match code {
        4007 => Some(MAINTENANCE_DELAY),
        4005 => Some(RATE_LIMITED_DELAY),
        _ => None,
    }
}

fn close_floor(frame: Option<tokio_tungstenite::tungstenite::protocol::CloseFrame>) -> Option<Duration> {
    let frame = frame?;
    let code = u16::from(frame.code);
    debug!("[7TV EventAPI] socket closed by server ({}: {})", code, frame.reason);
    close_delay_floor(code)
}

async fn connection_loop(
    app_handle: AppHandle,
    emote_service: Arc<RwLock<EmoteService>>,
    http: reqwest::Client,
    subs: Arc<RwLock<HashMap<String, ChannelSub>>>,
    mut cmd_rx: mpsc::UnboundedReceiver<Cmd>,
) {
    let mut backoff = BACKOFF_MIN;
    let mut resume_session: Option<String> = None;
    loop {
        let report = match connect_and_run(
            &app_handle,
            &emote_service,
            &http,
            &subs,
            &mut cmd_rx,
            resume_session.clone(),
        )
        .await
        {
            Ok(report) => report,
            Err(e) => {
                error!("[7TV EventAPI] connection ended: {}", e);
                SessionReport::default()
            }
        };
        if report.connected {
            backoff = BACKOFF_MIN;
        }
        // Keep the last real session for RESUME across a failed connect attempt;
        // a fresh session replaces it.
        if report.session_id.is_some() {
            resume_session = report.session_id;
        }
        let base = report.delay_floor.map_or(backoff, |floor| floor.max(backoff));
        // Jitter up to a quarter of the delay, so a fleet of clients does not
        // return to 7TV in lockstep after an outage.
        let jitter_ms = rand::random::<u64>() % (base.as_millis() as u64 / 4 + 1);
        let delay = base + Duration::from_millis(jitter_ms);
        debug!("[7TV EventAPI] reconnecting in {:.1}s", delay.as_secs_f64());
        sleep(delay).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Send the frames for one queued command; presence rides a subscribe.
async fn apply_cmd(
    write: &mut WsSink,
    http: &reqwest::Client,
    cmd: Cmd,
    session_id: Option<&str>,
) -> anyhow::Result<()> {
    match cmd {
        Cmd::Subscribe(sub) => {
            for frame in subscribe_frames(&sub) {
                write.send(Message::text(frame)).await?;
            }
            spawn_presence(http.clone(), sub, session_id.map(String::from));
        }
        Cmd::Unsubscribe(sub) => {
            for frame in unsubscribe_frames(&sub) {
                write.send(Message::text(frame)).await?;
            }
        }
    }
    Ok(())
}

async fn connect_and_run(
    app_handle: &AppHandle,
    emote_service: &Arc<RwLock<EmoteService>>,
    http: &reqwest::Client,
    subs: &Arc<RwLock<HashMap<String, ChannelSub>>>,
    cmd_rx: &mut mpsc::UnboundedReceiver<Cmd>,
    resume_session: Option<String>,
) -> anyhow::Result<SessionReport> {
    let mut report = SessionReport::default();
    let (ws, _) = connect_async(EVENTAPI_URL).await?;
    let (mut write, mut read) = ws.split();

    // Read until HELLO (op 1): it carries the session id (presence and RESUME
    // need it) and the heartbeat interval the read timeout is derived from.
    let mut session_id: Option<String> = None;
    let mut read_timeout = DEFAULT_READ_TIMEOUT;
    loop {
        match timeout(HELLO_TIMEOUT, read.next()).await {
            Ok(Some(Ok(Message::Text(txt)))) => {
                let Ok(v) = serde_json::from_str::<Value>(&txt) else {
                    continue;
                };
                if op_of(&v) == OP_HELLO {
                    session_id = v
                        .pointer("/d/session_id")
                        .and_then(|s| s.as_str())
                        .map(String::from);
                    if let Some(ms) = v
                        .pointer("/d/heartbeat_interval")
                        .and_then(|h| h.as_u64())
                        .filter(|ms| *ms > 0)
                    {
                        read_timeout = Duration::from_millis(ms.saturating_mul(HEARTBEAT_MISSES));
                    }
                    break;
                }
            }
            Ok(Some(Ok(Message::Close(frame)))) => {
                report.delay_floor = close_floor(frame);
                return Ok(report);
            }
            Ok(None) => return Ok(report),
            Ok(Some(Err(e))) => return Err(e.into()),
            Err(_) => {
                warn!(
                    "[7TV EventAPI] no HELLO in {}s, reconnecting",
                    HELLO_TIMEOUT.as_secs()
                );
                return Ok(report);
            }
            _ => {}
        }
    }
    report.connected = true;
    report.session_id = session_id.clone();
    info!(
        "[7TV EventAPI] connected (session {}, read timeout {}s)",
        session_id.as_deref().unwrap_or("?"),
        read_timeout.as_secs()
    );

    // RESUME the previous session when we have one: the server restores its
    // subscriptions and replays every dispatch we missed, in order. Anything
    // other than an ACK for it within the window means a fresh start.
    let mut resumed = false;
    if let Some(prev) = resume_session.as_deref() {
        write
            .send(Message::text(
                json!({ "op": OP_RESUME, "d": { "session_id": prev } }).to_string(),
            ))
            .await?;
        let deadline = Instant::now() + RESUME_ACK_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                warn!("[7TV EventAPI] RESUME unanswered in {}s", RESUME_ACK_TIMEOUT.as_secs());
                break;
            }
            match timeout(remaining, read.next()).await {
                Ok(Some(Ok(Message::Text(txt)))) => {
                    let Ok(v) = serde_json::from_str::<Value>(&txt) else {
                        continue;
                    };
                    match op_of(&v) {
                        OP_ACK if v.pointer("/d/command").and_then(|c| c.as_str()) == Some("RESUME") => {
                            resumed = true;
                            break;
                        }
                        OP_ERROR => {
                            warn!(
                                "[7TV EventAPI] RESUME refused: {}",
                                v.get("d").map(|d| d.to_string()).unwrap_or_default()
                            );
                            break;
                        }
                        _ => {
                            // A replayed dispatch, or a heartbeat: handle it.
                            if let Flow::Reconnect { floor } =
                                handle_text(&txt, app_handle, emote_service, subs).await
                            {
                                report.delay_floor = floor;
                                return Ok(report);
                            }
                        }
                    }
                }
                Ok(Some(Ok(Message::Close(frame)))) => {
                    report.delay_floor = close_floor(frame);
                    return Ok(report);
                }
                Ok(None) => return Ok(report),
                Ok(Some(Err(e))) => return Err(e.into()),
                Err(_) => {
                    warn!("[7TV EventAPI] RESUME unanswered in {}s", RESUME_ACK_TIMEOUT.as_secs());
                    break;
                }
                _ => {}
            }
        }
    }

    if resumed {
        info!(
            "[7TV EventAPI] resumed session {}; missed dispatches replay",
            resume_session.as_deref().unwrap_or("?")
        );
        // The joins and leaves that happened while the socket was down are
        // exactly the queued commands: apply them on top of the restored state.
        while let Ok(cmd) = cmd_rx.try_recv() {
            apply_cmd(&mut write, http, cmd, session_id.as_deref()).await?;
        }
        let snapshot: Vec<ChannelSub> = subs.read().await.values().cloned().collect();
        for sub in &snapshot {
            spawn_presence(http.clone(), sub.clone(), session_id.clone());
        }
    } else {
        // Fresh session. The desired state is the map; the queued commands are
        // already reflected in it, so drop them and subscribe from a snapshot.
        // The snapshot is taken OUTSIDE the lock's lifetime: no network call
        // ever runs while `subs` is held.
        while cmd_rx.try_recv().is_ok() {}
        let snapshot: Vec<ChannelSub> = subs.read().await.values().cloned().collect();
        for sub in &snapshot {
            for frame in subscribe_frames(sub) {
                write.send(Message::text(frame)).await?;
            }
            spawn_presence(http.clone(), sub.clone(), session_id.clone());
        }
        if resume_session.is_some() && !snapshot.is_empty() {
            warn!(
                "[7TV EventAPI] could not resume; resyncing {} channel set(s)",
                snapshot.len()
            );
            for sub in &snapshot {
                spawn_resync(sub, Arc::clone(emote_service));
            }
        }
    }

    loop {
        tokio::select! {
            read_res = timeout(read_timeout, read.next()) => {
                match read_res {
                    Err(_) => {
                        warn!(
                            "[7TV EventAPI] no frames in {}s ({} heartbeat cycles), reconnecting",
                            read_timeout.as_secs(),
                            HEARTBEAT_MISSES
                        );
                        return Ok(report);
                    }
                    Ok(None) => return Ok(report),
                    Ok(Some(Ok(Message::Text(txt)))) => {
                        if let Flow::Reconnect { floor } =
                            handle_text(&txt, app_handle, emote_service, subs).await
                        {
                            report.delay_floor = floor;
                            return Ok(report);
                        }
                    }
                    Ok(Some(Ok(Message::Close(frame)))) => {
                        report.delay_floor = close_floor(frame);
                        return Ok(report);
                    }
                    Ok(Some(Err(e))) => return Err(e.into()),
                    Ok(Some(Ok(_))) => {} // ping/pong/binary: ignore
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(cmd) => apply_cmd(&mut write, http, cmd, session_id.as_deref()).await?,
                    None => return Ok(report), // sender dropped (never, SERVICE holds it)
                }
            }
        }
    }
}

/// Handle one text frame. Never awaits the network: a dispatch is applied from
/// its own body, so the read loop keeps draining while 7TV is slow.
async fn handle_text(
    txt: &str,
    app_handle: &AppHandle,
    emote_service: &Arc<RwLock<EmoteService>>,
    subs: &Arc<RwLock<HashMap<String, ChannelSub>>>,
) -> Flow {
    let Ok(msg) = serde_json::from_str::<Value>(txt) else {
        return Flow::Continue;
    };

    match op_of(&msg) {
        OP_DISPATCH => {
            let Some(d) = msg.get("d") else {
                return Flow::Continue;
            };
            let dispatch_type = d.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if dispatch_type == "emote_set.update" {
                if let Some(body) = d.get("body") {
                    handle_emote_set_update(body, app_handle, emote_service, subs).await;
                }
            } else if dispatch_type.starts_with("entitlement.") {
                handle_entitlement(d, dispatch_type, app_handle);
            }
            Flow::Continue
        }
        OP_RECONNECT => {
            info!("[7TV EventAPI] server asked us to reconnect");
            Flow::Reconnect { floor: None }
        }
        OP_END_OF_STREAM => {
            let code = msg
                .pointer("/d/code")
                .and_then(|c| c.as_u64())
                .unwrap_or(0) as u16;
            let message = msg
                .pointer("/d/message")
                .and_then(|m| m.as_str())
                .unwrap_or("");
            warn!(
                "[7TV EventAPI] end of stream ({}: {}), reconnecting",
                code, message
            );
            Flow::Reconnect {
                floor: close_delay_floor(code),
            }
        }
        OP_ERROR => {
            // A rejected subscribe or resume; the frame says which.
            warn!(
                "[7TV EventAPI] server error: {}",
                msg.get("d").map(|d| d.to_string()).unwrap_or_default()
            );
            Flow::Continue
        }
        _ => Flow::Continue, // HELLO / HEARTBEAT / ACK: no action needed
    }
}

/// An `ActiveEmote` object's emote id: the full emote under `data` when present,
/// else the row's own `id` (the two agree in every payload seen so far).
fn active_emote_id(ae: &Value) -> Option<String> {
    ae.pointer("/data/id")
        .or_else(|| ae.get("id"))
        .and_then(|v| v.as_str())
        .map(String::from)
}

async fn handle_emote_set_update(
    body: &Value,
    app_handle: &AppHandle,
    emote_service: &Arc<RwLock<EmoteService>>,
    subs: &Arc<RwLock<HashMap<String, ChannelSub>>>,
) {
    let set_id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if set_id.is_empty() {
        return;
    }

    // Map the emote set back to the channel we subscribed it for. The channel
    // entitlement subscription also delivers other people's PERSONAL sets on
    // this same event type; those match no channel and are ignored here.
    let channel = {
        let map = subs.read().await;
        map.values()
            .find(|s| s.emote_set_id.as_deref() == Some(set_id))
            .map(|s| (s.channel_name.clone(), s.channel_id.clone(), s.platform.clone()))
    };
    let Some((channel_name, channel_id, platform)) = channel else {
        return;
    };

    let actor_name = body
        .pointer("/actor/display_name")
        .and_then(|v| v.as_str())
        .or_else(|| body.pointer("/actor/username").and_then(|v| v.as_str()))
        .unwrap_or("Someone")
        .to_string();

    // The ChangeMap carries whole ActiveEmote rows, the same shape the channel
    // set fetch parses, so the rows below are exactly what a full fetch would
    // have produced for them. `name` is the channel alias shown in chat.
    let pushed_values: Vec<Value> = body
        .get("pushed")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|c| c.get("value").cloned()).collect())
        .unwrap_or_default();
    let added_rows: Vec<Emote> = emote_service::parse_seventv_active_emotes(&pushed_values);

    let removed_rows: Vec<(String, String)> = body
        .get("pulled")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let ae = c.get("old_value").or_else(|| c.get("value"))?;
                    let name = ae.get("name").and_then(|v| v.as_str())?;
                    Some((active_emote_id(ae)?, name.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();

    let updated_rows: Vec<(String, String, Emote)> = body
        .get("updated")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let old_name = c
                        .pointer("/old_value/name")
                        .and_then(|v| v.as_str())?
                        .to_string();
                    let new_value = c.get("value").cloned()?;
                    let row = emote_service::parse_seventv_active_emotes(&[new_value])
                        .into_iter()
                        .next()?;
                    Some((row.id.clone(), old_name, row))
                })
                .collect()
        })
        .unwrap_or_default();

    if added_rows.is_empty() && removed_rows.is_empty() && updated_rows.is_empty() {
        return;
    }

    // Names for the in-chat notices, taken before the rows move into the delta.
    let added_names: Vec<String> = added_rows.iter().map(|e| e.name.clone()).collect();
    let removed_names: Vec<String> = removed_rows.iter().map(|(_, n)| n.clone()).collect();
    let renamed_json: Vec<Value> = updated_rows
        .iter()
        .filter(|(_, old, row)| old != &row.name)
        .map(|(_, old, row)| json!({ "old": old, "new": row.name }))
        .collect();

    // Apply the delta to every copy this process holds. Kick and YouTube keep
    // their own name-keyed stores; until those grow a delta entry point they
    // refetch, spawned so the read loop never waits on it.
    let composed: Option<Value> = match platform.as_str() {
        "kick" | "youtube" => {
            let (name, id, p) = (channel_name.clone(), channel_id.clone(), platform.clone());
            tokio::spawn(async move {
                if p == "kick" {
                    if let Ok(uid) = id.parse::<u64>() {
                        super::providers::kick_emotes::invalidate(&name);
                        super::providers::kick_emotes::refresh(&name, uid).await;
                    }
                } else {
                    super::providers::youtube_emotes::invalidate(&name);
                    super::providers::youtube_emotes::refresh(&name, &id).await;
                }
            });
            None
        }
        _ => {
            let globals = emote_service::seventv_globals_snapshot().await;
            let delta = SeventvSetDelta {
                added: added_rows,
                removed: removed_rows,
                updated: updated_rows,
            };
            // The parse dictionary (chat) and the picker cache get the same
            // patch; the disk copy follows on a debounce inside the first call.
            let composed =
                IrcService::apply_seventv_delta(&channel_name, &channel_id, &delta, &globals).await;
            let svc = emote_service.read().await;
            let _ = svc
                .apply_seventv_delta_cached(&channel_id, &delta, &globals)
                .await;
            match composed {
                Some(c) => serde_json::to_value(&c).ok(),
                None => {
                    // Channel set not in memory (no chat open on it here): there
                    // is nothing to patch, and a window that fetches next must
                    // not get the pre-change cached set.
                    svc.invalidate_channel(&channel_id).await;
                    None
                }
            }
        }
    };

    let composed_present = composed.is_some();
    let _ = app_handle.emit(
        "7tv://emote-set-update",
        json!({
            "channel": channel_name,
            "channel_id": channel_id,
            // Which platform's chat key / emote store this refers to; without it
            // the frontend would treat every update as Twitch.
            "platform": platform,
            "actor_name": actor_name,
            "added": added_names,
            "removed": removed_names,
            "renamed": renamed_json,
            // The composed dictionary delta: rows to drop (by id AND name) then
            // rows to add, including any global a removal stopped shadowing, so
            // a window patches its cached set in place with no fetch. Null when
            // this process holds no copy of the set; the window refetches then.
            "composed": composed,
        }),
    );

    info!(
        "[7TV EventAPI] {} emote set: +{} -{} ~{} (by {}){}",
        channel_name,
        added_names.len(),
        removed_names.len(),
        renamed_json.len(),
        actor_name,
        if composed_present { "" } else { ", refetch" }
    );
}

// A user's entitlement changed in a subscribed channel. Two kinds matter:
//
// EMOTE_SET: the user was granted (or lost) a 7TV personal emote set, usable in
// any channel. We resolve the set body and cache the personal-use emotes keyed
// by their Twitch id so message parsing can overlay them. This is what makes a
// user's personal emotes render even in streams that never added them.
//
// BADGE / PAINT: cosmetics. We extract the Twitch id and let the frontend
// re-resolve the authoritative render shape via the existing v4 GQL path (cheap,
// cached, coalesced). The WS is the trigger; GQL is the resolver.
fn handle_entitlement(d: &Value, dispatch_type: &str, app_handle: &AppHandle) {
    let Some(body) = d.get("body") else {
        return;
    };

    let kind = body
        .pointer("/object/kind")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let twitch_id = body
        .pointer("/object/user/connections")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|c| c.get("platform").and_then(|p| p.as_str()) == Some("TWITCH"))
        })
        .and_then(|c| c.get("id"))
        .and_then(|i| i.as_str());
    let action = dispatch_type
        .strip_prefix("entitlement.")
        .unwrap_or("update");

    if kind == "EMOTE_SET" {
        let set_id = body
            .pointer("/object/ref_id")
            .and_then(|v| v.as_str())
            .map(String::from);
        let twitch_id = twitch_id.map(String::from);
        match action {
            "create" | "update" => {
                if let (Some(tid), Some(set_id)) = (twitch_id, set_id) {
                    enqueue_entitlement_fetch(tid, set_id);
                }
            }
            "delete" => {
                if let Some(tid) = twitch_id {
                    tokio::spawn(async move {
                        IrcService::clear_personal_emotes(&tid, set_id.as_deref()).await;
                    });
                }
            }
            _ => {}
        }
        return;
    }

    let Some(twitch_id) = twitch_id else {
        return; // delete events without a user object, or non-twitch users
    };

    let _ = app_handle.emit(
        "7tv://cosmetic-update",
        json!({ "twitch_id": twitch_id, "action": action }),
    );
}

// Entitlement fetch lane. A reconnect / presence rebootstrap re-delivers the
// same EMOTE_SET entitlement for every personal-emote user in the channel at
// once; spawning a task per dispatch meant hundreds of concurrent tasks all
// contending the personal-emote store and racing HTTP fetches. A bounded POOL
// drains a bounded queue; the inflight set dedupes before anything is queued,
// and a dropped entry self-heals on the next reconnect's re-delivery.
//
// The pool used to be a single worker, which fixed the stampede by going all the
// way to serial. That is invisible while 7TV answers in ~100ms and awful when it
// does not: measured 2026-08-29 at 2.4-4.2s per call, one entitlement landed
// every ~3s in delivery order, which reads as cosmetics popping in one user at a
// time long after their message. Concurrency here is per-user and independent,
// NOT a batch: each fetch resolves and applies on its own, so widening the pool
// costs no user any extra wait.
const ENTITLEMENT_CONCURRENCY: usize = 6;
static ENTITLEMENT_TX: OnceLock<tokio::sync::mpsc::Sender<(String, String)>> = OnceLock::new();
static ENTITLEMENT_INFLIGHT: OnceLock<std::sync::Mutex<std::collections::HashSet<(String, String)>>> =
    OnceLock::new();

fn entitlement_inflight() -> &'static std::sync::Mutex<std::collections::HashSet<(String, String)>>
{
    ENTITLEMENT_INFLIGHT.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// Rate gate for the slow-fetch line above: true at most once every 5s.
fn slow_entitlement_log_due() -> bool {
    static LAST: OnceLock<std::sync::Mutex<Option<std::time::Instant>>> = OnceLock::new();
    let Ok(mut last) = LAST.get_or_init(|| std::sync::Mutex::new(None)).lock() else {
        return false;
    };
    let due = last
        .map(|t| t.elapsed() >= std::time::Duration::from_secs(5))
        .unwrap_or(true);
    if due {
        *last = Some(std::time::Instant::now());
    }
    due
}

fn enqueue_entitlement_fetch(twitch_id: String, set_id: String) {
    let key = (twitch_id.clone(), set_id.clone());
    {
        let Ok(mut inflight) = entitlement_inflight().lock() else {
            return;
        };
        if !inflight.insert(key.clone()) {
            return;
        }
    }
    let tx = ENTITLEMENT_TX.get_or_init(|| {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, String)>(512);
        tokio::spawn(async move {
            // The semaphore is what keeps this a POOL rather than the stampede
            // the queue replaced: the receiver runs ahead freely, but only
            // ENTITLEMENT_CONCURRENCY fetches are ever in flight. Acquiring
            // before the spawn is deliberate, so a slow provider applies
            // backpressure to the queue instead of piling up detached tasks.
            let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(ENTITLEMENT_CONCURRENCY));
            while let Some((tid, set_id)) = rx.recv().await {
                let Ok(permit) = permits.clone().acquire_owned().await else {
                    break; // semaphore closed; the lane is shutting down
                };
                tokio::spawn(async move {
                    let _permit = permit;
                    if !IrcService::has_personal_set(&tid, &set_id).await {
                        // This lane spends seconds in HTTP and said nothing about
                        // it, which is why a pool of one hid here for a whole
                        // release. Reported only when slow, and at most once every
                        // 5s, so a busy channel cannot turn it into a firehose.
                        let started = std::time::Instant::now();
                        let emotes = emote_service::fetch_personal_emote_set(&set_id).await;
                        let took = started.elapsed();
                        if took >= std::time::Duration::from_secs(1) && slow_entitlement_log_due() {
                            log::info!(
                                "[7TV] personal set {} took {}ms (lane holds {} in flight)",
                                set_id,
                                took.as_millis(),
                                ENTITLEMENT_CONCURRENCY
                            );
                        }
                        IrcService::set_personal_emotes(tid.clone(), set_id.clone(), emotes).await;
                    }
                    if let Ok(mut inflight) = entitlement_inflight().lock() {
                        inflight.remove(&(tid, set_id));
                    }
                });
            }
        });
        tx
    });
    if tx.try_send(key.clone()).is_err() {
        if let Ok(mut inflight) = entitlement_inflight().lock() {
            inflight.remove(&key);
        }
        log::warn!("[7TV] entitlement lane full; dropped fetch (re-delivered on next reconnect)");
    }
}

/// Channels the 7TV EventAPI socket is subscribed to (try-read). Diagnostics
/// for the resource line.
pub fn sub_count() -> Option<usize> {
    SERVICE.get()?.subs.try_read().ok().map(|s| s.len())
}
