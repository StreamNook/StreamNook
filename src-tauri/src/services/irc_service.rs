use super::default_name_color;
use crate::models::chat_layout::{
    Badge, ChatMessage, EmotePos, LayoutResult, MessageMetadata, MessageSegment, ReplyInfo,
};
use crate::models::settings::AppState;
use crate::services::chat_history::ChatHistory;
use crate::services::chat_rules::ChatRules;
use crate::plugin_host::PluginHost;
use crate::services::chat_logger_service::ChatLoggerService;
use crate::services::emoji_service;
use crate::services::link_detect;
use crate::services::emote_service::{Emote, EmoteService, EmoteSet};
use crate::services::layout_service::LayoutService;
use crate::services::twitch_service::TwitchService;
use crate::services::user_message_history_service::UserMessageHistoryService;
use crate::services::irc_transport::{self, IrcTransport, IrcWriter};
use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn};
use rand::Rng;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::OnceLock;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{broadcast, Mutex};
use warp::Filter;

pub struct IrcService;

static WS_SERVER_HANDLE: OnceLock<Mutex<Option<tokio::task::JoinHandle<()>>>> = OnceLock::new();
// Serializes bring-up of the local WS bridge so concurrent first-acquires (MultiChat
// restoring N channels at boot) don't each spin up a separate server + broadcaster,
// which leaves the frontend attached to a dead or message-less socket.
static BRIDGE_BRINGUP_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static CURRENT_CHANNELS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static MESSAGE_BROADCASTER: OnceLock<Mutex<Option<Arc<broadcast::Sender<String>>>>> =
    OnceLock::new();
static MESSAGE_QUEUE: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
static IRC_HANDLE: OnceLock<Mutex<Option<tokio::task::JoinHandle<()>>>> = OnceLock::new();
// Abort handles for the keepalive tasks spawned INSIDE the IRC task (ping +
// frontend heartbeat). Aborting IRC_HANDLE alone orphans them: the ping task's
// writer Arc keeps the socket's write half alive, so it would keep PINGing a
// half-open connection indefinitely after stop().
static IRC_PING_ABORT: OnceLock<Mutex<Option<tokio::task::AbortHandle>>> = OnceLock::new();
static IRC_HEARTBEAT_ABORT: OnceLock<Mutex<Option<tokio::task::AbortHandle>>> = OnceLock::new();
static IRC_WRITER: OnceLock<Mutex<Option<Arc<Mutex<IrcWriter>>>>> = OnceLock::new();
static SHARED_CHAT_ROOMS: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
// Fast-path gate for enhance_message_with_shared_chat: the overwhelming
// majority of sessions never see a shared-chat room, so the per-message
// lock + line copy is skipped entirely until one is detected.
static SHARED_CHAT_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// Process-wide port of the local WebSocket bridge. Stored so a second
// `start_chat` call (typically from a popout window like StreamNook MultiChat
// opening its own JS store) can be made idempotent — instead of tearing the
// running IRC connection down, we return the existing port and JOIN the new
// channel onto the connection that's already live.
static WS_PORT: OnceLock<Mutex<Option<u16>>> = OnceLock::new();
// Per-channel caches (lowercase channel name -> value). Multi-channel chat
// (StreamNook MultiChat) needs each JOINed channel to retain its own badges,
// room state, and emote set so split-mode rendering and late-mount tab opens
// don't get cross-channel state.
static USER_BADGES_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
// The connected user's own chat color from USERSTATE, keyed per channel. Lets the
// frontend paint its own optimistic messages in the real color from the first
// frame instead of flashing a default until the IRC echo round-trips.
static USER_COLOR_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
static ROOM_STATE_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
static CHANNEL_EMOTES: OnceLock<Mutex<HashMap<String, EmoteSet>>> = OnceLock::new();
// Per-channel cheermote sets from Helix `bits/cheermotes?broadcaster_id=`,
// which returns Twitch's globals PLUS the channel's own `channel_custom`
// prefixes — the only source of custom cheer art, so a static prefix list can
// never render them. Arc so parse_text_segment snapshots without cloning tier
// data per message; evicted with the other per-channel caches on PART/stop.
static CHANNEL_CHEERMOTES: OnceLock<std::sync::RwLock<HashMap<String, Arc<CheermoteSet>>>> =
    OnceLock::new();
// Per-channel consumer claims: lowercase channel -> set of window labels.
// JOIN and PART fire only on the empty <-> non-empty transitions, so two
// windows can hand a channel between them without it leaving. Sets rather
// than counts: a claim may repeat, and a window may die without releasing.
// Ensure-only callers (the warm-up, the defensive re-JOIN) must not claim.
static CHANNEL_CONSUMERS: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();
// Handle to the plugin host so parsed chat lines can be forwarded to plugins
// subscribed to on_chat_message. Set once, on the first chat start.
static PLUGIN_HOST: OnceLock<Arc<PluginHost>> = OnceLock::new();
// The logged-in user's (login, user id), for attributing locally sent
// messages: Twitch IRC does not echo your own PRIVMSG back.
static OWN_IDENTITY: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();
// A 7TV subscriber's personal emotes, keyed by sender id rather than by
// channel: they render in every channel. Value is (set id, name -> emote).
// PERSONAL_EMOTES_PRESENT lets the per-message path skip the lock while
// no user has any. LRU-bounded; entitlements are only revoked explicitly.
#[allow(clippy::type_complexity)]
static PERSONAL_EMOTES: OnceLock<
    std::sync::RwLock<lru::LruCache<String, (String, Arc<HashMap<String, Emote>>)>>,
> = OnceLock::new();
static PERSONAL_EMOTES_PRESENT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

// Serializes start()'s check-then-spawn body. Two concurrent fresh starts
// (boot storm, or two windows' watchdogs escalating together) could each
// spawn a supervisor, leaking one forever on a duplicate socket.
static START_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

// Monotonic read-age clock. Wall clock would misreport after NTP or
// sleep/resume jumps; a backwards jump could make a dead connection look
// freshly read.
static PROCESS_EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
/// 24-hour timestamps (settings.chat_design.timestamp_format == "24h").
/// Written by ChatRules::refresh on every settings change.
pub static TIMESTAMP_24H: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LAST_IRC_READ_ELAPSED_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

// Ring of recent connection-lifecycle events, pullable from a packaged build
// (which has no stderr) via the get_chat_lifecycle_log command.
static LIFECYCLE_LOG: OnceLock<std::sync::Mutex<VecDeque<String>>> = OnceLock::new();
const LIFECYCLE_LOG_CAP: usize = 100;

// We PING every 30s and the server answers, so a healthy link never goes 75s
// without a completed read. Reference clients ping every 5s, so Twitch has
// ample tolerance for this cadence.
const IRC_PING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
const IRC_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(75);
const HANDSHAKE_STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
// Read timeout plus slack: past this the frontend must be allowed to see
// silence so its own watchdog can act.
const HEARTBEAT_SUPPRESS_AFTER_MS: u64 = 90_000;
// After a clean drop, reconnect almost immediately: a read-liveness timeout is
// our own local detection (no server pressure, so zero wait; on a phone this
// fires the moment the app resumes from suspension and the user is staring at
// the chat), and server-initiated closes get one polite second (reference
// clients use a 1s base). The flap guard in the supervisor keeps short-lived
// sessions from hot-looping on these fast delays.
const RECONNECT_DELAY_AFTER_TIMEOUT: std::time::Duration = std::time::Duration::ZERO;
const RECONNECT_DELAY_AFTER_DROP: std::time::Duration = std::time::Duration::from_secs(1);
// A session that dies this quickly never really established; count it toward
// the failure backoff instead of resetting it, or a connect-drop cycle would
// spin at the fast delays forever.
const SESSION_FLAP_THRESHOLD_MS: u64 = 10_000;
// Twitch caps JOINs at 20 per rolling 10s per connection. Burst this many at
// handshake, pace the rest; the headroom absorbs concurrent ensure_joined
// JOINs from user actions.
const JOIN_BURST_BUDGET: usize = 15;
const JOIN_PACE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(12_500);
// A JOIN Twitch never acknowledged (no ROOMSTATE/USERSTATE/JOIN echo, no channel
// message) is re-issued after this window. Twitch can silently drop JOINs (rate
// limits, room-server hiccups); before this tracker existed such a channel stayed
// deaf forever while the socket looked perfectly healthy.
const JOIN_CONFIRM_TIMEOUT_MS: u64 = 12_000;
const JOIN_MAX_ATTEMPTS: u32 = 3;
const JOIN_WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
// After exhausting re-issues the watchdog drops the session so the supervisor
// rebuilds and re-JOINs everything — but at most once per this window, so one
// permanently unjoinable channel can't put chat in reconnect churn.
const JOIN_DROP_COOLDOWN_MS: u64 = 300_000;
// The read loop must never park forever inside message handling (Helix calls,
// plugin hosts, file IO): past this the session is dropped as a detected failure
// the supervisor heals, instead of a permanent undetectable freeze.
const HANDLER_STALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

// JOIN acknowledgment tracking. CURRENT_CHANNELS stays the desired-state set;
// this tracker holds the ACTUAL state: which desired channels the server has
// acknowledged (ROOMSTATE/USERSTATE/JOIN echo/any channel message) and which
// JOIN writes still await an ack. Per-session: cleared between supervisor
// sessions and on stop, since a fresh socket re-JOINs everything.
static JOIN_TRACKER: OnceLock<Mutex<JoinTracker>> = OnceLock::new();
// pending-count mirror of the tracker, so the per-message hot path can skip the
// tracker mutex entirely while no JOIN is awaiting confirmation.
static JOIN_PENDING_HINT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static IRC_JOINWATCH_ABORT: OnceLock<Mutex<Option<tokio::task::AbortHandle>>> = OnceLock::new();
static LAST_JOIN_DROP_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct PendingJoin {
    deadline_ms: u64,
    attempts: u32,
    // A nudge JOIN (frontend stale-watchdog probe on an already-joined channel)
    // is never retried and never escalates: if Twitch doesn't re-ack it, the
    // entry just lingers as an awaiting-confirm marker until session end.
    nudge_only: bool,
}

#[derive(Default)]
struct JoinTracker {
    pending: HashMap<String, PendingJoin>,
    confirmed: HashSet<String>,
}

impl JoinTracker {
    /// Record a JOIN write. Re-recording the same key bumps its attempt count so
    /// the watchdog can exhaust; `pace_slot_ms` defers the deadline for JOINs the
    /// pacer only writes later, so pacing never reads as a lost JOIN.
    fn record_sent(&mut self, key: &str, now_ms: u64, pace_slot_ms: u64, nudge_only: bool) {
        let attempts = self.pending.get(key).map(|p| p.attempts).unwrap_or(0) + 1;
        self.pending.insert(
            key.to_string(),
            PendingJoin {
                deadline_ms: now_ms + JOIN_CONFIRM_TIMEOUT_MS + pace_slot_ms,
                attempts,
                nudge_only,
            },
        );
    }

    /// Any server frame for the channel proves membership. Returns true the
    /// first time a channel becomes confirmed.
    fn confirm(&mut self, key: &str) -> bool {
        self.pending.remove(key);
        self.confirmed.insert(key.to_string())
    }

    /// (confirmed, pending) for the key.
    fn is_settled(&self, key: &str) -> (bool, bool) {
        (
            self.confirmed.contains(key),
            self.pending.contains_key(key),
        )
    }

    /// Non-nudge entries past their deadline: (key, attempts so far).
    fn due(&self, now_ms: u64) -> Vec<(String, u32)> {
        self.pending
            .iter()
            .filter(|(_, p)| !p.nudge_only && now_ms >= p.deadline_ms)
            .map(|(k, p)| (k.clone(), p.attempts))
            .collect()
    }

    fn drop_pending(&mut self, key: &str) {
        self.pending.remove(key);
    }

    fn unconfirm(&mut self, key: &str) {
        self.confirmed.remove(key);
    }

    fn forget(&mut self, key: &str) {
        self.pending.remove(key);
        self.confirmed.remove(key);
    }

    fn clear(&mut self) {
        self.pending.clear();
        self.confirmed.clear();
    }
}

fn get_join_tracker() -> &'static Mutex<JoinTracker> {
    JOIN_TRACKER.get_or_init(|| Mutex::new(JoinTracker::default()))
}

fn refresh_join_hint(t: &JoinTracker) {
    JOIN_PENDING_HINT.store(t.pending.len(), std::sync::atomic::Ordering::Relaxed);
}

async fn tracker_record_sent(key: &str, pace_slot_ms: u64, nudge_only: bool) {
    let mut t = get_join_tracker().lock().await;
    t.record_sent(key, mono_ms(), pace_slot_ms, nudge_only);
    refresh_join_hint(&t);
}

async fn confirm_join(key: &str) {
    let mut t = get_join_tracker().lock().await;
    if t.confirm(key) {
        record_lifecycle(&format!("JOIN #{} confirmed", key));
    }
    refresh_join_hint(&t);
}

/// Hot-path confirm for per-message frames: one atomic load while nothing is
/// pending (the overwhelmingly common state), the mutex only while JOINs are
/// actually outstanding. ROOMSTATE provides the durable confirm either way.
async fn confirm_join_if_pending(key: &str) {
    if JOIN_PENDING_HINT.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return;
    }
    confirm_join(key).await;
}

async fn tracker_forget(key: &str) {
    let mut t = get_join_tracker().lock().await;
    t.forget(key);
    refresh_join_hint(&t);
}

async fn tracker_clear() {
    let mut t = get_join_tracker().lock().await;
    t.clear();
    refresh_join_hint(&t);
}

/// Command-token parse of a JOIN frame (":nick!user@host JOIN #chan"), tags
/// stripped. Strict on purpose: never match a PRIVMSG whose text contains the
/// word. With twitch.tv/membership active, ANY user's JOIN for a channel proves
/// we are in it (Twitch only relays membership for channels you have joined).
fn parse_join_channel(line: &str) -> Option<String> {
    let mut t = line.trim();
    if t.starts_with('@') {
        t = t.split_once(' ').map(|(_, rest)| rest)?;
    }
    let mut parts = t.split_whitespace();
    let first = parts.next()?;
    let (cmd, chan) = if first.starts_with(':') {
        (parts.next()?, parts.next()?)
    } else {
        (first, parts.next()?)
    };
    if cmd != "JOIN" {
        return None;
    }
    let chan = chan.trim_start_matches(':').trim_start_matches('#');
    if chan.is_empty() {
        return None;
    }
    Some(chan.to_lowercase())
}

/// Per-message side-effect payload for the ordered lane below.
struct MessageSideEffects {
    msg: ChatMessage,
    add_history: bool,
    /// Key the persisted history is stored under. Twitch uses the bare user id
    /// (so existing entries keep resolving); other platforms are namespaced
    /// `provider:id`, because platform id spaces overlap — Kick user 676 and
    /// Twitch user 676 are different people and must not share a bucket.
    history_key: String,
}

// Ordered side-effect lane: history LRU, chat logger and plugin fan-out.
// Kept off the IRC read loop so slow file IO cannot stall the reader, and
// single-consumer so chat-log line order survives. Bounded, dropping
// oldest; drops are counted per channel and reported in the affected log.
const SIDE_EFFECT_CAP: usize = 2048;

struct SideEffectLane {
    queue: std::sync::Mutex<VecDeque<MessageSideEffects>>,
    notify: tokio::sync::Notify,
    /// channel -> messages dropped while the lane was saturated.
    dropped: std::sync::Mutex<HashMap<String, u64>>,
}

static SIDE_EFFECT_LANE: OnceLock<Arc<SideEffectLane>> = OnceLock::new();

fn side_effect_lane() -> &'static Arc<SideEffectLane> {
    SIDE_EFFECT_LANE.get_or_init(|| {
        let lane = Arc::new(SideEffectLane {
            queue: std::sync::Mutex::new(VecDeque::new()),
            notify: tokio::sync::Notify::new(),
            dropped: std::sync::Mutex::new(HashMap::new()),
        });
        let consumer = lane.clone();
        tokio::spawn(async move {
            loop {
                consumer.notify.notified().await;
                // Drain to EMPTY per wake: Notify coalesces permits, so a
                // one-item-per-wake loop would lose wakeups.
                loop {
                    let next = consumer.queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some(se) = next else { break };
                    if se.add_history && !se.history_key.is_empty() {
                        UserMessageHistoryService::global()
                            .add_message(&se.history_key, &se.msg)
                            .await;
                    }
                    ChatLoggerService::log_message(&se.msg);
                    if let Some(host) = PLUGIN_HOST.get() {
                        if host.wants_chat_messages().await {
                            host.emit_chat_message(chat_event_params(&se.msg)).await;
                        }
                    }
                }
                // Caught up: make any saturation loss visible in the logs it hit.
                let flushed: Vec<(String, u64)> = consumer
                    .dropped
                    .lock()
                    .map(|mut d| d.drain().collect())
                    .unwrap_or_default();
                for (channel, count) in flushed {
                    ChatLoggerService::log_dropped_marker(&channel, count);
                }
            }
        });
        lane
    })
}

fn enqueue_side_effect(se: MessageSideEffects) {
    let lane = side_effect_lane();
    let dropped_channel = {
        let Ok(mut q) = lane.queue.lock() else { return };
        let dropped = if q.len() >= SIDE_EFFECT_CAP {
            q.pop_front().map(|old| old.msg.channel)
        } else {
            None
        };
        q.push_back(se);
        dropped
    };
    if let Some(channel) = dropped_channel {
        if let Ok(mut d) = lane.dropped.lock() {
            *d.entry(channel).or_insert(0) += 1;
        }
        // Rate-limited: one warn per 30s however fast the lane overflows.
        static LAST_WARN_S: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let now_s = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let last = LAST_WARN_S.load(std::sync::atomic::Ordering::Relaxed);
        if now_s.saturating_sub(last) >= 30
            && LAST_WARN_S
                .compare_exchange(
                    last,
                    now_s,
                    std::sync::atomic::Ordering::Relaxed,
                    std::sync::atomic::Ordering::Relaxed,
                )
                .is_ok()
        {
            warn!("[IRC Chat] side-effect lane full ({SIDE_EFFECT_CAP}); dropping oldest");
        }
    }
    lane.notify.notify_one();
}

/// Runs the shared per-message side effects (persisted user history,
/// chat-log write, plugin fan-out) for a message that did not come from
/// the Twitch IRC reader.
///
/// Provider adapters publish straight onto the broadcast, so they must go
/// through here to be logged and recorded. Using the lane keeps chat-log
/// line order intact and the file IO off the caller's task.
pub fn run_message_side_effects(msg: ChatMessage) {
    let add_history = !msg.user_id.is_empty();
    let history_key = history_key_for(&msg);
    enqueue_side_effect(MessageSideEffects {
        msg,
        add_history,
        history_key,
    });
}

/// The persisted-history key for a message: the bare id on Twitch, `provider:id`
/// elsewhere. The frontend builds the same key when reading it back.
fn history_key_for(msg: &ChatMessage) -> String {
    if msg.provider.is_empty() || msg.provider == "twitch" {
        msg.user_id.clone()
    } else {
        format!("{}:{}", msg.provider, msg.user_id)
    }
}

fn get_start_lock() -> &'static Mutex<()> {
    START_LOCK.get_or_init(|| Mutex::new(()))
}

fn mono_ms() -> u64 {
    PROCESS_EPOCH
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

fn mark_irc_read() {
    LAST_IRC_READ_ELAPSED_MS.store(mono_ms(), std::sync::atomic::Ordering::Relaxed);
}

fn irc_read_age_ms() -> u64 {
    mono_ms().saturating_sub(LAST_IRC_READ_ELAPSED_MS.load(std::sync::atomic::Ordering::Relaxed))
}

// 2s, 4s, 8s, 16s, 32s, 60s cap for transient failures; flat 300s when
// Twitch rejected our credentials so an expired login never hot-loops.
fn reconnect_delay(consecutive_failures: u32, auth_failure: bool) -> std::time::Duration {
    if auth_failure {
        return std::time::Duration::from_secs(300);
    }
    let n = consecutive_failures.clamp(1, 6);
    std::time::Duration::from_secs((2u64 << (n - 1)).min(60))
}

// Set once any session in this PROCESS reaches the read loop (IRC_CONNECTED
// went out). Splits "lost an established session the user was watching"
// (IRC_RECONNECTING, which the frontend may surface) from "still trying to
// establish one" (IRC_CONNECT_RETRY, which stays quiet). Process-global on
// purpose: the frontend's stale-ladder recovery restarts the supervisor, and a
// supervisor-local flag would relabel a real ongoing outage as a first connect
// after that restart, hiding it forever.
static EVER_ESTABLISHED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

fn outage_frame(ever_established: bool) -> &'static str {
    if ever_established {
        "IRC_RECONNECTING"
    } else {
        "IRC_CONNECT_RETRY"
    }
}

// ":tmi.twitch.tv RECONNECT" as the command token. Strips a tag prefix
// defensively; a PRIVMSG whose text contains the word can never match
// because its command token is PRIVMSG.
fn is_server_reconnect(line: &str) -> bool {
    let mut t = line.trim();
    if t.starts_with('@') {
        t = t.split_once(' ').map(|(_, rest)| rest).unwrap_or(t);
    }
    t == "RECONNECT" || (t.starts_with(':') && t.split_whitespace().nth(1) == Some("RECONNECT"))
}

pub fn record_lifecycle(event: &str) {
    info!("[IRC Chat] {}", event);
    let buf = LIFECYCLE_LOG
        .get_or_init(|| std::sync::Mutex::new(VecDeque::with_capacity(LIFECYCLE_LOG_CAP)));
    if let Ok(mut b) = buf.lock() {
        if b.len() >= LIFECYCLE_LOG_CAP {
            b.pop_front();
        }
        b.push_back(format!("{} {}", chrono::Utc::now().to_rfc3339(), event));
    }
}

pub fn lifecycle_snapshot() -> Vec<String> {
    LIFECYCLE_LOG
        .get()
        .and_then(|m| m.lock().ok().map(|b| b.iter().cloned().collect()))
        .unwrap_or_default()
}

enum SessionError {
    Auth(anyhow::Error),
    Transient(anyhow::Error),
}

impl From<std::io::Error> for SessionError {
    fn from(e: std::io::Error) -> Self {
        SessionError::Transient(e.into())
    }
}

fn get_ws_server_handle() -> &'static Mutex<Option<tokio::task::JoinHandle<()>>> {
    WS_SERVER_HANDLE.get_or_init(|| Mutex::new(None))
}

fn get_bridge_bringup_lock() -> &'static Mutex<()> {
    BRIDGE_BRINGUP_LOCK.get_or_init(|| Mutex::new(()))
}

fn get_current_channels() -> &'static Mutex<HashSet<String>> {
    CURRENT_CHANNELS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn get_message_broadcaster() -> &'static Mutex<Option<Arc<broadcast::Sender<String>>>> {
    MESSAGE_BROADCASTER.get_or_init(|| Mutex::new(None))
}

fn get_message_queue() -> &'static Mutex<VecDeque<String>> {
    MESSAGE_QUEUE.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn get_irc_handle() -> &'static Mutex<Option<tokio::task::JoinHandle<()>>> {
    IRC_HANDLE.get_or_init(|| Mutex::new(None))
}

fn get_irc_ping_abort() -> &'static Mutex<Option<tokio::task::AbortHandle>> {
    IRC_PING_ABORT.get_or_init(|| Mutex::new(None))
}

fn get_irc_heartbeat_abort() -> &'static Mutex<Option<tokio::task::AbortHandle>> {
    IRC_HEARTBEAT_ABORT.get_or_init(|| Mutex::new(None))
}

fn get_irc_joinwatch_abort() -> &'static Mutex<Option<tokio::task::AbortHandle>> {
    IRC_JOINWATCH_ABORT.get_or_init(|| Mutex::new(None))
}

/// Abort the ping + heartbeat + JOIN-watchdog keepalive tasks, if running.
/// Idempotent.
async fn abort_keepalive_tasks() {
    if let Some(h) = get_irc_ping_abort().lock().await.take() {
        h.abort();
    }
    if let Some(h) = get_irc_heartbeat_abort().lock().await.take() {
        h.abort();
    }
    if let Some(h) = get_irc_joinwatch_abort().lock().await.take() {
        h.abort();
    }
}

/// Sends a frame to the local WS bridge, resolving the broadcaster at call
/// time because the bridge can be rebuilt mid-session. Returns whether a
/// receiver took it.
///
/// `queue_on_fail` holds chat payloads for the next client attach. Status
/// frames must pass `false`: replayed later, a status would misreport the
/// current state to the frontend watchdog.
async fn send_to_bridge(msg: String, queue_on_fail: bool) -> bool {
    let tx = get_message_broadcaster().lock().await.clone();
    let delivered = match tx {
        Some(tx) => tx.send(msg.clone()).is_ok(),
        None => false,
    };
    if !delivered && queue_on_fail {
        let mut queue = get_message_queue().lock().await;
        queue.push_back(msg);
        if queue.len() > 500 {
            queue.pop_front();
        }
    }
    delivered
}

fn get_irc_writer() -> &'static Mutex<Option<Arc<Mutex<IrcWriter>>>> {
    IRC_WRITER.get_or_init(|| Mutex::new(None))
}

fn get_shared_chat_rooms() -> &'static Mutex<HashMap<String, Vec<String>>> {
    SHARED_CHAT_ROOMS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_user_badges_cache() -> &'static Mutex<HashMap<String, String>> {
    USER_BADGES_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_user_color_cache() -> &'static Mutex<HashMap<String, String>> {
    USER_COLOR_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_room_state_cache() -> &'static Mutex<HashMap<String, String>> {
    ROOM_STATE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Chat messages published while nothing was subscribed to the bus.
///
/// `broadcast::send` discards when there are no receivers, and a provider's
/// join backlog is published before the frontend WebSocket client attaches.
/// Bounded, and drained by the first client to attach. The frontend dedupes
/// by message id, so a replayed row cannot double up one that arrived live.
static PENDING_MESSAGES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
/// Enough for a full join backlog with headroom; past this the oldest go, because
/// a buffer that grows without a listener is a leak, not a feature.
const PENDING_MESSAGES_MAX: usize = 200;

fn get_pending_messages() -> &'static Mutex<Vec<String>> {
    PENDING_MESSAGES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Hold a message that had no subscriber, for replay when one attaches.
pub async fn hold_undelivered_message(json: String) {
    let mut pending = get_pending_messages().lock().await;
    if pending.len() >= PENDING_MESSAGES_MAX {
        let overflow = pending.len() + 1 - PENDING_MESSAGES_MAX;
        pending.drain(0..overflow);
    }
    pending.push(json);
}

/// Take everything held, leaving the buffer empty.
pub async fn take_undelivered_messages() -> Vec<String> {
    std::mem::take(&mut *get_pending_messages().lock().await)
}

/// Cache a ROOMSTATE frame published by a non-Twitch provider, keyed by its full
/// composite channel key ("kick:slug").
///
/// Providers emit the same frame Twitch does, and this is the same cache the
/// local-WS handshake replays to every newly attached client, so a MultiChat pane
/// that mounts after the room state arrived still learns the current modes.
pub async fn cache_provider_room_state(channel_key: &str, frame: String) {
    get_room_state_cache()
        .lock()
        .await
        .insert(channel_key.to_lowercase(), frame);
}

/// Drop a provider's cached ROOMSTATE when its channel is released, mirroring the
/// Twitch PART cleanup so a parted channel can't leak a stale entry.
pub async fn remove_provider_room_state(channel_key: &str) {
    get_room_state_cache()
        .lock()
        .await
        .remove(&channel_key.to_lowercase());
}

fn get_channel_emotes() -> &'static Mutex<HashMap<String, EmoteSet>> {
    CHANNEL_EMOTES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Parse the `gifs` PRIVMSG tag (Twitch, 2026-07-17) into positions that ride
/// the emote list. Format: comma-separated `<start>-<end>|<gifID>|<gifURL>`
/// with zero-based INCLUSIVE codepoint indices, the same convention as
/// `emotes`, so `parse_message_segments` places them with the same arithmetic
/// and the reply-mention offset applies once. The URL is used exactly as sent
/// (Twitch: "must not be modified"); IRCv3 tag escapes are undone first, which
/// a Giphy URL never needs but the spec allows. Malformed entries are skipped.
fn parse_gifs_tag(value: &str) -> Vec<EmotePos> {
    let mut out = Vec::new();
    if value.is_empty() {
        return out;
    }
    for entry in value.split(',') {
        let mut fields = entry.splitn(3, '|');
        let (Some(range), Some(id), Some(url)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let Some((start_s, end_s)) = range.split_once('-') else {
            continue;
        };
        let (Ok(start), Ok(end)) = (start_s.parse::<usize>(), end_s.parse::<usize>()) else {
            continue;
        };
        if id.is_empty() || url.is_empty() || start > end {
            continue;
        }
        out.push(EmotePos {
            id: unescape_irc_tag(id),
            start,
            end,
            url: unescape_irc_tag(url),
            gif: true,
        });
    }
    out
}

/// Undo IRCv3 message-tag value escapes (`\:` for `;`, `\s` for space, `\\`,
/// `\r`, `\n`). Tag values are kept raw in the tag map and unescaped per field.
fn unescape_irc_tag(value: &str) -> String {
    if !value.contains('\\') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(':') => out.push(';'),
            Some('s') => out.push(' '),
            Some('\\') => out.push('\\'),
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Immutable per-channel parse context. Rebuilt only when the channel's
/// third-party sets change; per-message parsing reads it through an Arc with
/// no locks held and no data cloned. Personal emotes are deliberately NOT in
/// here: they are per-sender (a separate map consulted first in the word
/// tier), and the 7TV-override filter below must never match them.
pub(crate) struct EmoteLookup {
    /// name -> emote, inserted bttv, ffz, seven_tv in order (later insert
    /// wins), preserving the word tier's 7TV > FFZ > BTTV priority.
    by_name: HashMap<String, Emote>,
}

impl EmoteLookup {
    fn build(set: &EmoteSet) -> Arc<Self> {
        let mut by_name =
            HashMap::with_capacity(set.bttv.len() + set.ffz.len() + set.seven_tv.len());
        for e in set.bttv.iter().chain(&set.ffz).chain(&set.seven_tv) {
            by_name.insert(e.name.clone(), e.clone());
        }
        Arc::new(Self { by_name })
    }

    fn get(&self, name: &str) -> Option<&Emote> {
        self.by_name.get(name)
    }

    /// 7TV art for a Twitch-native emote name. The map slot holds the 7TV
    /// entry whenever the composed 7TV dictionary carries the name (7TV is
    /// inserted last, so it beats FFZ and BTTV on a name collision). Within
    /// 7TV there is nothing left to arbitrate: `compose_seventv` keys the
    /// dictionary by name with channel rows first, so every consumer,
    /// first-wins or last-wins, resolves a name to the same row.
    fn seventv_override(&self, name: &str) -> Option<&Emote> {
        self.by_name
            .get(name)
            .filter(|e| e.provider == crate::services::emote_service::EmoteProvider::SevenTV)
    }
}

static CHANNEL_PARSE_LOOKUP: OnceLock<std::sync::RwLock<HashMap<String, Arc<EmoteLookup>>>> =
    OnceLock::new();

fn channel_parse_lookup() -> &'static std::sync::RwLock<HashMap<String, Arc<EmoteLookup>>> {
    CHANNEL_PARSE_LOOKUP.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

fn rebuild_parse_lookup(key: &str, set: &EmoteSet) {
    let lookup = EmoteLookup::build(set);
    if let Ok(mut guard) = channel_parse_lookup().write() {
        guard.insert(key.to_string(), lookup);
    }
}

fn drop_parse_lookup(key: &str) {
    if let Ok(mut guard) = channel_parse_lookup().write() {
        guard.remove(key);
    }
}

fn clear_parse_lookups() {
    if let Ok(mut guard) = channel_parse_lookup().write() {
        guard.clear();
    }
}

/// Owned Arc snapshots backing a ParseCtx. Gathered once per message; the
/// borrows in ParseCtx keep parsing itself allocation- and lock-free.
struct ParseSnapshots {
    channel: Option<Arc<EmoteLookup>>,
    personal: Option<Arc<HashMap<String, Emote>>>,
    cheermotes: Option<Arc<CheermoteSet>>,
}

impl ParseSnapshots {
    fn ctx(&self) -> ParseCtx<'_> {
        ParseCtx {
            channel: self.channel.as_deref(),
            personal: self.personal.as_deref(),
            cheermotes: self.cheermotes.as_deref(),
        }
    }
}

/// Borrowed per-message parse context. `personal` is the sender's 7TV personal
/// map, consulted first WITHIN the emote tier only (after the URL and
/// cheermote checks), matching the old personal-inserted-last map priority.
#[derive(Default)]
struct ParseCtx<'a> {
    channel: Option<&'a EmoteLookup>,
    personal: Option<&'a HashMap<String, Emote>>,
    cheermotes: Option<&'a CheermoteSet>,
}

/// One cheermote tier from Helix: bits threshold, hex color, animated dark art.
#[derive(Debug, Clone)]
pub struct CheermoteTier {
    pub min_bits: u32,
    pub color: String,
    pub url: String,
}

/// Lowercase prefix -> tiers ascending by `min_bits`.
pub type CheermoteSet = HashMap<String, Vec<CheermoteTier>>;

fn get_channel_cheermotes() -> &'static std::sync::RwLock<HashMap<String, Arc<CheermoteSet>>> {
    CHANNEL_CHEERMOTES.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

fn get_ws_port() -> &'static Mutex<Option<u16>> {
    WS_PORT.get_or_init(|| Mutex::new(None))
}

fn get_channel_consumers() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    CHANNEL_CONSUMERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_own_identity() -> &'static Mutex<Option<(String, String)>> {
    OWN_IDENTITY.get_or_init(|| Mutex::new(None))
}

#[allow(clippy::type_complexity)]
fn get_personal_emotes(
) -> &'static std::sync::RwLock<lru::LruCache<String, (String, Arc<HashMap<String, Emote>>)>> {
    PERSONAL_EMOTES.get_or_init(|| {
        std::sync::RwLock::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(512).expect("nonzero"),
        ))
    })
}

/// The lean wire shape of the on_chat_message plugin event (PROTOCOL.md):
/// identity, text, and event metadata only. Render data (segments, layout,
/// emote URLs) stays out so the payload is small and stable.
fn chat_event_params(msg: &ChatMessage) -> Value {
    let ts = msg
        .timestamp
        .parse::<i64>()
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    json!({
        "channel": msg.channel,
        "message": {
            "id": msg.id,
            "user_id": msg.user_id,
            "login": msg.username,
            "display_name": msg.display_name,
            "color": msg.color,
            "badges": msg
                .badges
                .iter()
                .map(|b| json!({ "name": b.name, "version": b.version }))
                .collect::<Vec<_>>(),
            "text": msg.content,
            "is_action": msg.metadata.is_action,
            "msg_type": msg.metadata.msg_type,
            "system_message": msg.metadata.system_message,
            "bits": msg.metadata.bits_amount,
            "ts": ts,
        }
    })
}

// Extract the channel name (lowercase, no leading #) from a raw IRC line.
// Used by ROOMSTATE/USERSTATE/CLEARMSG/CLEARCHAT parsing to key per-channel
// caches and tag synthetic WS messages.
fn extract_channel_from_irc_line(line: &str) -> Option<String> {
    let idx = line.find(" #")?;
    let after = &line[idx + 2..];
    let end = after.find([' ', '\r', '\n']).unwrap_or(after.len());
    let name = &after[..end];
    if name.is_empty() {
        None
    } else {
        Some(name.to_lowercase())
    }
}

impl IrcService {
    /// `claim` marks the caller as a real consumer (a window's chat store
    /// acquiring the channel); the stream-start warm-up passes false and just
    /// needs the bridge up and the channel JOINed. `reattach` is set by the
    /// reconnect path, whose store still holds channels: it suppresses the
    /// stale-claim sweep that a fresh first-acquire start performs. `window`
    /// is the calling window's label, the key consumer claims are recorded
    /// under.
    pub async fn start(
        channel: &str,
        state: &AppState,
        claim: bool,
        reattach: bool,
        window: &str,
    ) -> Result<u16> {
        // Serialize the whole check-then-spawn body: without this, two
        // concurrent fresh starts could each spawn an IRC supervisor and leak
        // one forever on a duplicate socket.
        let _start_guard = get_start_lock().lock().await;

        let layout_service = state.layout_service.clone();
        let emote_service = state.emote_service.clone();
        let _ = PLUGIN_HOST.set(state.plugin_host.clone());
        ChatLoggerService::init(state.settings.clone());

        // Idempotency: if the IRC service is already running, don't tear it
        // down. Instead, JOIN the requested channel onto the existing
        // connection (if not already joined) and return the existing WS port.
        // This is critical for multi-window setups — the MultiChat popout's JS
        // store calls start_chat as its first action, and if we tore down the
        // main app's connection here every popout would freeze the main app's
        // chat.
        {
            // A JoinHandle stays Some after its task finishes; is_some() alone
            // reported a finished IRC task as alive forever, so every later
            // start_chat short-circuited onto the corpse (a webview reload
            // could never revive chat). Same check for the WS bridge task.
            let irc_alive = {
                let mut handle = get_irc_handle().lock().await;
                match handle.as_ref() {
                    Some(h) if !h.is_finished() => true,
                    Some(_) => {
                        record_lifecycle("previous IRC task is dead; restarting");
                        handle.take();
                        false
                    }
                    None => false,
                }
            };
            if !irc_alive {
                *get_irc_writer().lock().await = None;
            }
            let ws_alive = matches!(
                get_ws_server_handle().lock().await.as_ref(),
                Some(h) if !h.is_finished()
            );
            let existing_port = *get_ws_port().lock().await;
            if irc_alive && ws_alive {
                if let Some(port) = existing_port {
                    let key = channel.to_lowercase();
                    // `join_channel` is claim-aware: it records this window in
                    // the channel's consumer set and only sends IRC JOIN when
                    // no consumer held it. Ensure-only callers (warm-up) skip
                    // the claim. Best-effort: failures here just mean the
                    // channel hasn't been added to the existing IRC session;
                    // the caller will see that messages aren't arriving and
                    // can recover.
                    let join_result = if claim {
                        let r = Self::join_channel(&key, window).await;
                        // Claims still recorded for this window are leftovers from a previous
                        // JS context (webview reload); sweep them so their rooms PART. The
                        // reconnect re-attach path skips this and re-claims instead.
                        if !reattach {
                            Self::release_window_claims(window, Some(&key)).await;
                        }
                        r
                    } else {
                        let r = Self::ensure_joined(&key).await;
                        // A warm-up means a stream is starting; any channel
                        // still joined with no consumer is a leftover from a
                        // previous warm-up that no UI ever claimed.
                        Self::part_unclaimed(&key).await;
                        r
                    };
                    if let Err(e) = join_result {
                        log::warn!("[IRC Chat] idempotent JOIN failed for {}: {}", key, e);
                    }
                    // Make this channel parseable so segment parsing matches
                    // what the user sees. Seeds from the disk dictionary and
                    // returns; the provider refresh runs after, because the
                    // caller is a UI blocked on the chat socket and 7TV has
                    // been measured at 2.4-3.1s.
                    Self::seed_emotes_deferring_refresh(&key, emote_service.clone()).await;
                    return Ok(port);
                }
            }
        }

        // Stop any partially-alive remnants before fresh setup. When the WS
        // bridge is healthy (dead-IRC restart) or a non-Twitch provider is
        // using it, only clear the Twitch IRC remnants - a full stop() would
        // tear the bridge down and drop every window's live WS connection
        // along with any provider consumers.
        let bridge_healthy = matches!(
            get_ws_server_handle().lock().await.as_ref(),
            Some(h) if !h.is_finished()
        );
        if bridge_healthy || crate::services::providers::has_active_bridge_users() {
            Self::stop_irc_only().await;
        } else {
            Self::stop().await?;
        }

        debug!(
            "[IRC Chat] Starting IRC chat service for channel: {}",
            channel
        );

        // Store current channel (lowercased so set lookups match IRC frames,
        // which always carry lowercase channel names).
        get_current_channels()
            .lock()
            .await
            .insert(channel.to_lowercase());

        // Clear all per-channel caches on a fresh start. stop() also clears these,
        // but be defensive in case start() is called without a preceding stop().
        get_user_badges_cache().lock().await.clear();
        get_room_state_cache().lock().await.clear();
        get_pending_messages().lock().await.clear();
        get_channel_emotes().lock().await.clear();
        clear_parse_lookups();
        if let Ok(mut g) = get_channel_cheermotes().write() { g.clear(); }
        // Seed the consumer claims: this is the first window to ask for the
        // initial channel; the IRC JOIN is performed implicitly by
        // run_irc_connection below, so we just account for it here. Ensure-only
        // cold starts seed nothing: the first real acquire claims via
        // join_channel (which sees the channel already joined and only records
        // the claim). Clearing wipes other windows' claims along with the dead
        // connection; their stores reconnect and re-claim on their own.
        {
            let mut consumers = get_channel_consumers().lock().await;
            consumers.clear();
            if claim {
                consumers.insert(channel.to_lowercase(), HashSet::from([window.to_string()]));
            }
        }

        // Fast fail for the caller; the supervisor re-fetches per attempt.
        if TwitchService::get_token().await.is_err() {
            return Err(anyhow::anyhow!(
                "Not authenticated. Please log in to Twitch first."
            ));
        }

        // Get user info
        let user_info = TwitchService::get_user_info().await?;

        debug!("[IRC Chat] User: {} ({})", user_info.login, user_info.id);

        *get_own_identity().lock().await = Some((user_info.login.clone(), user_info.id.clone()));
        ChatRules::set_own_identity(&user_info.login, &user_info.id);

        // Bring up (or reuse) the local WS bridge that fans parsed messages to
        // the frontend. Extracted into ensure_local_ws_bridge so non-Twitch
        // providers can publish onto the same bus without a Twitch chat open.
        let port = Self::ensure_local_ws_bridge().await?;
        // Fail fast if bring-up didn't leave a broadcaster; the session itself
        // resolves the CURRENT broadcaster at every send (send_to_bridge), so
        // this handle is deliberately not passed down — a bridge rebuilt
        // mid-session must not strand the supervisor on a dead sender.
        if Self::broadcaster().await.is_none() {
            return Err(anyhow::anyhow!(
                "WS bridge broadcaster missing after bring-up"
            ));
        }

        // Start IRC connection
        let username = user_info.login.clone();
        let initial_channel = channel.to_string();

        let irc_handle = tokio::spawn(async move {
            Self::run_irc_connection(
                &username,
                &initial_channel,
                layout_service,
                Arc::clone(&emote_service),
            )
            .await;
        });

        *get_irc_handle().lock().await = Some(irc_handle);

        debug!("[IRC Chat] Chat service started on port {}", port);

        Ok(port)
    }

    // Supervisor: never returns. Every failure mode retries with capped
    // backoff; nothing can permanently kill chat short of stop()'s abort.
    // The old single-function loop let any handshake `?` escape the task,
    // which died silently and left start_chat vouching for a corpse.
    async fn run_irc_connection(
        username: &str,
        initial_channel: &str,
        layout_service: Arc<LayoutService>,
        emote_service: Arc<tokio::sync::RwLock<EmoteService>>,
    ) {
        irc_transport::load_hint().await;
        let mut consecutive_failures: u32 = 0;
        loop {
            // Re-fetched every attempt: get_token refreshes an expiring token.
            // The old loop reused the token captured at start and died on auth
            // once it expired. A fetch error is transient (network / refresh
            // endpoint), not proof of a bad login; only a PASS rejection is.
            let token = match TwitchService::get_token().await {
                Ok(t) => t,
                Err(e) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    let d = reconnect_delay(consecutive_failures, false);
                    record_lifecycle(&format!(
                        "token fetch failed (attempt {}): {}; retry in {}s",
                        consecutive_failures,
                        e,
                        d.as_secs()
                    ));
                    send_to_bridge(
                        outage_frame(EVER_ESTABLISHED.load(std::sync::atomic::Ordering::Relaxed))
                            .to_string(),
                        false,
                    )
                    .await;
                    tokio::time::sleep(d).await;
                    continue;
                }
            };

            let session_started = mono_ms();
            let outcome = Self::irc_session(
                username,
                &token,
                initial_channel,
                &layout_service,
                &emote_service,
            )
            .await;
            let session_lived_ms = mono_ms().saturating_sub(session_started);

            // Between sessions: fail sends/JOINs fast instead of writing into
            // a dead socket, retire this session's keepalive tasks, and drop
            // its JOIN-ack state — the next session re-JOINs and re-confirms
            // everything in CURRENT_CHANNELS from scratch.
            *get_irc_writer().lock().await = None;
            abort_keepalive_tasks().await;
            tracker_clear().await;

            let delay = match outcome {
                Ok(reason) => {
                    // An Ok return proves IRC_CONNECTED went out this attempt
                    // (irc_session's contract), so this loss is a real one.
                    EVER_ESTABLISHED.store(true, std::sync::atomic::Ordering::Relaxed);
                    send_to_bridge("IRC_RECONNECTING".to_string(), false).await;
                    if session_lived_ms < SESSION_FLAP_THRESHOLD_MS {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        let d = reconnect_delay(consecutive_failures, false);
                        record_lifecycle(&format!(
                            "session ended after {}ms ({}); flap backoff {}s",
                            session_lived_ms,
                            reason,
                            d.as_secs()
                        ));
                        d
                    } else {
                        consecutive_failures = 0;
                        let d = if reason == "read liveness timeout" {
                            RECONNECT_DELAY_AFTER_TIMEOUT
                        } else {
                            RECONNECT_DELAY_AFTER_DROP
                        };
                        record_lifecycle(&format!(
                            "session ended: {}; reconnecting in {}s",
                            reason,
                            d.as_secs()
                        ));
                        d
                    }
                }
                Err(SessionError::Transient(e)) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    let d = reconnect_delay(consecutive_failures, false);
                    record_lifecycle(&format!(
                        "connect failed (attempt {}): {}; retry in {}s",
                        consecutive_failures,
                        e,
                        d.as_secs()
                    ));
                    send_to_bridge(
                        outage_frame(EVER_ESTABLISHED.load(std::sync::atomic::Ordering::Relaxed))
                            .to_string(),
                        false,
                    )
                    .await;
                    d
                }
                Err(SessionError::Auth(e)) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    record_lifecycle(&format!("authentication rejected: {}", e));
                    send_to_bridge(
                        "CONNECTION_WARNING:Chat sign-in failed. Your Twitch session may have expired; try signing out and back in."
                            .to_string(),
                        false,
                    )
                    .await;
                    reconnect_delay(consecutive_failures, true)
                }
            };
            tokio::time::sleep(delay).await;
        }
    }

    // One connect-handshake-read lifetime. Ok(reason) = the established
    // session later dropped (normal reconnect); Err = it never established.
    async fn irc_session(
        username: &str,
        token: &str,
        initial_channel: &str,
        layout_service: &Arc<LayoutService>,
        emote_service: &Arc<tokio::sync::RwLock<EmoteService>>,
    ) -> std::result::Result<&'static str, SessionError> {
        debug!("[IRC Chat] Connecting to Twitch IRC...");

        let connect_started = std::time::Instant::now();
        let connected = irc_transport::connect_transport()
            .await
            .map_err(|reason| SessionError::Transient(anyhow::anyhow!("{}", reason)))?;
        let transport = connected.transport;
        if transport == IrcTransport::WebSocket {
            record_lifecycle(&format!(
                "connected via websocket in {}ms{}",
                connect_started.elapsed().as_millis(),
                connected
                    .tcp_failed
                    .as_deref()
                    .map(|e| format!(" (tcp 6667 failed: {})", e))
                    .unwrap_or_default()
            ));
        }
        let mut reader = connected.reader;
        // The global IRC_WRITER is published only after auth succeeds, so
        // ensure_joined/send_message can never write into an unauthenticated
        // or mid-handshake socket.
        let writer = Arc::new(Mutex::new(connected.writer));

        // The handshake runs in its own block so a transient failure after a
        // successful connect can be attributed to the transport that carried
        // it (a firewall that accepts the SYN but blackholes the data looks
        // exactly like this). Auth rejections are transport-agnostic.
        let mut line = String::new();
        let handshake: std::result::Result<(), SessionError> = async {
            // IMPORTANT: CAP negotiation must happen BEFORE authentication
            // Step 1: Request capabilities first
            {
                let mut w = writer.lock().await;
                debug!("[IRC Chat] Requesting capabilities...");
                w.send_line("CAP REQ :twitch.tv/tags twitch.tv/commands twitch.tv/membership\r\n")
                    .await?;
            }

            // Step 2: Wait for CAP ACK before authenticating
            let mut cap_acknowledged = false;

            while !cap_acknowledged {
                line.clear();
                let n = tokio::time::timeout(HANDSHAKE_STEP_TIMEOUT, reader.read_line(&mut line))
                    .await
                    .map_err(|_| {
                        SessionError::Transient(anyhow::anyhow!("timed out waiting for CAP ACK"))
                    })??;
                if n == 0 {
                    return Err(SessionError::Transient(anyhow::anyhow!(
                        "connection closed during capability negotiation"
                    )));
                }

                debug!("[IRC Chat] Server response: {}", line.trim());

                if line.contains("CAP * ACK") {
                    cap_acknowledged = true;
                    debug!("[IRC Chat] Capabilities acknowledged");
                }
            }

            // Step 3: Now authenticate with PASS and NICK
            {
                let mut w = writer.lock().await;
                // IRC requires "oauth:" prefix for the password
                let auth_token = format!("oauth:{}", token);

                debug!("[IRC Chat] Authenticating with username: {}", username);

                w.send_line(&format!("PASS {}\r\n", auth_token)).await?;
                w.send_line(&format!("NICK {}\r\n", username.to_lowercase())).await?;
            }

            // Step 4: Wait for authentication confirmation
            let mut authenticated = false;

            while !authenticated {
                line.clear();
                let n = tokio::time::timeout(HANDSHAKE_STEP_TIMEOUT, reader.read_line(&mut line))
                    .await
                    .map_err(|_| {
                        SessionError::Transient(anyhow::anyhow!(
                            "timed out waiting for auth confirmation"
                        ))
                    })??;
                if n == 0 {
                    return Err(SessionError::Transient(anyhow::anyhow!(
                        "connection closed during authentication"
                    )));
                }

                debug!("[IRC Chat] Auth response: {}", line.trim());

                if line.contains("001") {
                    authenticated = true;
                } else if line.contains("NOTICE")
                    && (line.contains("Login unsuccessful")
                        || line.contains("Login authentication failed"))
                {
                    return Err(SessionError::Auth(anyhow::anyhow!(
                        "IRC authentication failed - token may be invalid or expired"
                    )));
                }
            }
            Ok(())
        }
        .await;
        if let Err(e) = handshake {
            if matches!(e, SessionError::Transient(_)) {
                irc_transport::note_handshake_failure(transport);
            }
            return Err(e);
        }
        irc_transport::note_authenticated(transport);

        *get_irc_writer().lock().await = Some(writer.clone());
        mark_irc_read();
        record_lifecycle("authenticated");

        // Re-JOIN every tracked channel, not just the initial one. `join_channel`
        // will not re-issue a JOIN while a channel's refcount is above zero, so
        // channels added during the session stay PARTed unless re-joined here.
        {
            let mut channels: Vec<String> = get_current_channels()
                .lock()
                .await
                .iter()
                .cloned()
                .collect();
            // On a fresh connect the set already holds the initial channel;
            // this fallback only covers the unexpected-empty case so we never
            // connect with zero joins.
            if channels.is_empty() {
                channels.push(initial_channel.to_lowercase());
            }
            // The initial channel joins in the first burst so the visible chat
            // is never the one waiting on a paced batch.
            let initial_key = initial_channel.to_lowercase();
            if let Some(pos) = channels.iter().position(|c| *c == initial_key) {
                channels.swap(0, pos);
            }
            let remainder = channels.split_off(channels.len().min(JOIN_BURST_BUDGET));
            {
                let mut w = writer.lock().await;
                for ch in &channels {
                    w.send_line(&format!("JOIN #{}\r\n", ch)).await?;
                }
            }
            {
                // Everything this session will JOIN goes into the ack tracker up
                // front. Paced channels get their deadline pushed out by their
                // batch slot, so waiting on the pacer never reads as a lost JOIN.
                let mut t = get_join_tracker().lock().await;
                let now = mono_ms();
                for ch in &channels {
                    t.record_sent(ch, now, 0, false);
                }
                let pace_ms = JOIN_PACE_INTERVAL.as_millis() as u64;
                for (idx, ch) in remainder.iter().enumerate() {
                    let slot = (idx / JOIN_BURST_BUDGET) as u64 + 1;
                    t.record_sent(ch, now, slot * pace_ms, false);
                }
                refresh_join_hint(&t);
            }
            record_lifecycle(&format!(
                "joined {} channel(s): {:?}",
                channels.len(),
                channels
            ));
            if !remainder.is_empty() {
                // Pace the overflow instead of tripping the JOIN rate limit.
                // The task holds this session's writer half, so its writes
                // fail and it exits once the socket dies; the channels stay in
                // CURRENT_CHANNELS either way, so the next session retries.
                record_lifecycle(&format!("pacing {} remaining JOIN(s)", remainder.len()));
                let writer_join = writer.clone();
                tokio::spawn(async move {
                    for chunk in remainder.chunks(JOIN_BURST_BUDGET) {
                        tokio::time::sleep(JOIN_PACE_INTERVAL).await;
                        let mut w = writer_join.lock().await;
                        for ch in chunk {
                            if w.send_line(&format!("JOIN #{}\r\n", ch)).await.is_err() {
                                return;
                            }
                        }
                        record_lifecycle(&format!("paced JOIN batch: {:?}", chunk));
                    }
                });
            }
        }

        // Emote and subscription setup for the initial channel, skipped if it has
        // since been left. Spawned rather than awaited so the read loop starts
        // draining immediately after the JOIN burst. Re-checks CURRENT_CHANNELS
        // before applying; every call is idempotent.
        {
            let init_channel = initial_channel.to_string();
            let emote_svc = Arc::clone(emote_service);
            tokio::spawn(async move {
                if !get_current_channels()
                    .lock()
                    .await
                    .contains(&init_channel.to_lowercase())
                {
                    return;
                }
                let initial_channel_id =
                    Self::fetch_and_store_emotes(&init_channel, emote_svc).await;

                // Idempotent, so a reconnect re-calling this is a no-op for an
                // already-subscribed channel.
                if let Some(cid) = initial_channel_id {
                    crate::services::seventv_eventapi::subscribe_channel(&init_channel, &cid)
                        .await;
                    // Subscribe the moderator view (channel.moderate) for this
                    // chat. Silently skipped server-side if you don't moderate
                    // the channel.
                    crate::services::eventsub_moderation::subscribe_channel(&init_channel, &cid)
                        .await;
                } else {
                    record_lifecycle(&format!(
                        "post-connect init: emote/id fetch failed for #{} (next session or join retries it)",
                        init_channel
                    ));
                }
            });
        }

        // Send connection success notification
        send_to_bridge("IRC_CONNECTED".to_string(), false).await;

        // Flush queued messages (dropped if no receiver is attached yet; the
        // WS handshake also drains this queue when a client connects).
        let queued: Vec<String> = {
            let mut queue = get_message_queue().lock().await;
            queue.drain(..).collect()
        };
        if !queued.is_empty() {
            debug!("[IRC Chat] Flushing {} queued messages", queued.len());
            for msg in queued {
                send_to_bridge(msg, false).await;
            }
        }

        // Start ping task to keep IRC connection alive. The cadence also
        // bounds dead-connection detection: every PING elicits a PONG read,
        // so the read timeout can sit just above this interval.
        let writer_clone = writer.clone();
        let ping_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(IRC_PING_INTERVAL);
            loop {
                interval.tick().await;
                let mut w = writer_clone.lock().await;
                if w.send_line("PING :tmi.twitch.tv\r\n").await.is_err() {
                    break;
                }
            }
        });

        // Start heartbeat task to notify frontend that connection is alive
        // (every 30s), preventing false "stale connection" warnings when chat
        // is quiet. It suppresses itself when the reader has heard nothing for
        // longer than the read timeout: a heartbeat must not vouch for a deaf
        // connection, and going silent is what lets the frontend watchdog
        // recover a wedged backend.
        let heartbeat_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                let age = irc_read_age_ms();
                if age > HEARTBEAT_SUPPRESS_AFTER_MS {
                    warn!(
                        "[IRC Chat] suppressing heartbeat: no IRC read for {}s",
                        age / 1000
                    );
                    continue;
                }
                if !send_to_bridge("HEARTBEAT".to_string(), false).await {
                    // No receivers (or no bridge), stop heartbeat
                    break;
                }
            }
        });

        // JOIN acknowledgment watchdog: re-issues JOINs the server never acked
        // (no ROOMSTATE/USERSTATE/JOIN echo/channel message), and after
        // JOIN_MAX_ATTEMPTS drops the session so the supervisor rebuilds it —
        // the one lever that recovers a socket that is TCP-alive but deaf.
        // Holds this session's writer, so it dies with the socket.
        let writer_watch = writer.clone();
        let joinwatch_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(JOIN_WATCH_INTERVAL);
            loop {
                interval.tick().await;
                let now = mono_ms();
                let due = { get_join_tracker().lock().await.due(now) };
                for (key, attempts) in due {
                    if attempts >= JOIN_MAX_ATTEMPTS {
                        // Exhausted. Rate-limit the session-drop escalation so a
                        // permanently unjoinable channel can't churn reconnects.
                        let last = LAST_JOIN_DROP_MS.load(std::sync::atomic::Ordering::Relaxed);
                        {
                            let mut t = get_join_tracker().lock().await;
                            t.drop_pending(&key);
                            refresh_join_hint(&t);
                        }
                        if now.saturating_sub(last) >= JOIN_DROP_COOLDOWN_MS {
                            LAST_JOIN_DROP_MS
                                .store(now, std::sync::atomic::Ordering::Relaxed);
                            record_lifecycle(&format!(
                                "JOIN #{} unconfirmed after {} attempts; dropping session to rebuild",
                                key, attempts
                            ));
                            let _ = writer_watch.lock().await.shutdown().await;
                            return;
                        }
                        record_lifecycle(&format!(
                            "JOIN #{} unconfirmed after {} attempts; within drop cooldown, deferring to next session",
                            key, attempts
                        ));
                    } else {
                        record_lifecycle(&format!(
                            "JOIN #{} unconfirmed; re-issuing (attempt {})",
                            key,
                            attempts + 1
                        ));
                        {
                            let mut w = writer_watch.lock().await;
                            if w.send_line(&format!("JOIN #{}\r\n", key)).await.is_err() {
                                return;
                            }
                        }
                        tracker_record_sent(&key, 0, false).await;
                    }
                }
            }
        });

        // Register the keepalive tasks so stop()/stop_irc_only() and the
        // supervisor's between-sessions cleanup can abort them; aborting only
        // the parent IRC task leaves them orphaned (tokio::spawn children are
        // independent of their parent).
        *get_irc_ping_abort().lock().await = Some(ping_handle.abort_handle());
        *get_irc_heartbeat_abort().lock().await = Some(heartbeat_handle.abort_handle());
        *get_irc_joinwatch_abort().lock().await = Some(joinwatch_handle.abort_handle());

        // Listen for messages. The timeout is the half-open detector: we PING
        // every 30s and the server answers, so 75s without a completed read
        // means the socket is dead even if the OS never reports it.
        loop {
            line.clear();
            let end_reason: Option<&'static str> =
                match tokio::time::timeout(IRC_READ_TIMEOUT, reader.read_line(&mut line)).await {
                    Err(_) => {
                        warn!(
                            "[IRC Chat] no IRC traffic for {}s, connection presumed dead",
                            IRC_READ_TIMEOUT.as_secs()
                        );
                        Some("read liveness timeout")
                    }
                    Ok(Ok(0)) => Some("closed by server"),
                    Ok(Ok(_)) => {
                        mark_irc_read();
                        if is_server_reconnect(&line) {
                            Some("server RECONNECT")
                        } else {
                            // The handler's own timeout is the belt-and-braces
                            // stall detector: the read timeout above only covers
                            // read_line, so a handler parked on a wedged await
                            // used to freeze the reader forever while sends kept
                            // working. A stall now becomes a detected drop.
                            match tokio::time::timeout(
                                HANDLER_STALL_TIMEOUT,
                                Self::handle_irc_message(&line, &writer, layout_service),
                            )
                            .await
                            {
                                Err(_) => {
                                    warn!(
                                        "[IRC Chat] message handler stalled for {}s, dropping session",
                                        HANDLER_STALL_TIMEOUT.as_secs()
                                    );
                                    // Name the offending line so a field log
                                    // can attribute the stall, not just count it.
                                    let snippet: String = line.trim().chars().take(120).collect();
                                    record_lifecycle(&format!(
                                        "handler stalled >{}s on: {}",
                                        HANDLER_STALL_TIMEOUT.as_secs(),
                                        snippet
                                    ));
                                    Some("handler stall")
                                }
                                Ok(Err(e)) => {
                                    error!("[IRC Chat] Error handling message: {}", e);
                                    None
                                }
                                Ok(Ok(())) => None,
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        error!("[IRC Chat] Read error: {}", e);
                        Some("read error")
                    }
                };

            if let Some(reason) = end_reason {
                return Ok(reason);
            }
        }
    }

    async fn handle_irc_message(
        line: &str,
        writer: &Arc<Mutex<IrcWriter>>,
        layout_service: &LayoutService,
    ) -> Result<()> {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            return Ok(());
        }

        // Handle PING - extract the server data after "PING "
        if trimmed.starts_with("PING") {
            // Safe slice: extract everything after "PING " (5 chars), or empty if too short
            let ping_data = if trimmed.len() > 5 { &trimmed[5..] } else { "" };
            // Bound the wait for the lock, never an in-flight write: cancelling
            // `write_all` can leave a partial protocol line on a live socket. Missing
            // one PONG is recoverable; a torn line is not.
            match tokio::time::timeout(std::time::Duration::from_secs(5), writer.lock()).await {
                Ok(mut w) => {
                    w.send_line(&format!("PONG {}\r\n", ping_data)).await?;
                }
                Err(_) => {
                    record_lifecycle("PONG skipped: writer lock busy >5s (socket congested)");
                }
            }
            return Ok(());
        }

        // Parse and handle different message types
        if trimmed.contains("PRIVMSG") {
            // Regular chat message - forward as-is with shared chat detection
            let enhanced_message = Self::enhance_message_with_shared_chat(trimmed).await;
            let enhanced_message: &str = &enhanced_message;

            // Debug: Log cheer/bits messages (raw IRC data)
            if enhanced_message.contains("bits=") {
                debug!(
                    "\n[IRC CHEER DEBUG] ========== RAW BITS MESSAGE ==========\n{}\n[IRC CHEER DEBUG] =========================================\n",
                    enhanced_message
                );
            }

            // Debug: Log the received IRC message to see what we're parsing
            if enhanced_message.contains(":Stare") || enhanced_message.contains(" Stare ") {
                debug!(
                    "[IRC Chat DEBUG] Received PRIVMSG with 'Stare': {}",
                    enhanced_message
                );
            }

            // Parse and layout
            if let Some(mut chat_msg) = Self::parse_privmsg(&enhanced_message) {
                debug!(
                    "[IRC Chat DEBUG] Parsed message from {}: content='{}', {} segments",
                    chat_msg.username,
                    chat_msg.content,
                    chat_msg.segments.len()
                );

                // DOM-FIRST ARCHITECTURE: Frontend measures heights via ResizeObserver
                // Backend only provides message data, not layout calculations
                // Set a placeholder height - frontend will measure and set the real value
                chat_msg.layout = LayoutResult {
                    height: 60.0, // Placeholder - frontend DOM measurement is authoritative
                    width: 0.0,   // Not used anymore
                    has_reply: chat_msg.metadata.reply_info.is_some(),
                    is_first_message: chat_msg.metadata.is_first_message,
                };

                // Receiving a channel message proves its JOIN landed — the
                // strongest, cheapest ack signal for the tracker.
                confirm_join_if_pending(
                    chat_msg.channel.trim_start_matches('#').to_lowercase().as_str(),
                )
                .await;

                // Rule engine: ignores, highlights, mentions, saved filters,
                // history ring. One snapshot read, stamps onto metadata. An
                // ignored message never reaches a window but still feeds the
                // side-effect lane: the log is the record, not the display.
                let rules = ChatRules::snapshot();
                let verdict = ChatRules::evaluate(&mut chat_msg, &rules);

                // Deliver to the frontend FIRST (wire order is the only order
                // the UI needs), then hand the slow side effects (history LRU,
                // chat logger, plugins) to the ordered lane so they can never
                // block the read loop.
                if !verdict.drop {
                    if let Ok(json_msg) = serde_json::to_string(&chat_msg) {
                        send_to_bridge(json_msg, true).await;
                    }
                }
                enqueue_side_effect(MessageSideEffects {
                    history_key: history_key_for(&chat_msg),
                    msg: chat_msg,
                    add_history: true,
                });
            } else {
                // Fallback to sending raw string if parsing fails
                send_to_bridge(enhanced_message.to_string(), true).await;
            }
        } else if trimmed.contains("USERNOTICE") {
            // Subscription, resub, gift sub, etc.
            // Parse USERNOTICE messages - layout will be measured by frontend
            if let Some(mut chat_msg) = Self::parse_usernotice(trimmed) {
                // Skip USERNOTICE messages with no visible content
                // These render as blank/ghost messages (e.g., "onetapgiftredeemed" with no text)
                let has_content = !chat_msg.content.is_empty();
                let has_system_msg = chat_msg
                    .metadata
                    .system_message
                    .as_ref()
                    .is_some_and(|s| !s.is_empty());

                if !has_content && !has_system_msg {
                    debug!(
                        "[IRC Chat] Skipping empty USERNOTICE: type={:?}",
                        chat_msg.metadata.msg_type
                    );
                    return Ok(());
                }

                // DOM-FIRST ARCHITECTURE: Frontend measures heights via ResizeObserver
                // Backend only provides message data, not layout calculations
                chat_msg.layout = LayoutResult {
                    height: 100.0, // Larger placeholder for subscription messages
                    width: 0.0,
                    has_reply: false,
                    is_first_message: false,
                };

                debug!(
                    "[IRC Chat] Parsed USERNOTICE: type={:?}, user_content_len={}",
                    chat_msg.metadata.msg_type,
                    chat_msg.content.len()
                );

                // A USERNOTICE for a channel is membership proof, same as PRIVMSG.
                confirm_join_if_pending(
                    chat_msg.channel.trim_start_matches('#').to_lowercase().as_str(),
                )
                .await;

                // Rule engine (raid tint, sub-message filters, ignores).
                let rules = ChatRules::snapshot();
                let verdict = ChatRules::evaluate(&mut chat_msg, &rules);

                // Frontend first, side effects on the ordered lane (no history:
                // USERNOTICE never fed the profile-card history).
                if !verdict.drop {
                    if let Ok(json_msg) = serde_json::to_string(&chat_msg) {
                        send_to_bridge(json_msg, true).await;
                    }
                }
                enqueue_side_effect(MessageSideEffects {
                    history_key: history_key_for(&chat_msg),
                    msg: chat_msg,
                    add_history: false,
                });
            } else {
                // Fallback to raw string if parsing fails
                send_to_bridge(trimmed.to_string(), true).await;
            }
        } else if trimmed.contains("ROOMSTATE") {
            // Room state updates (slow mode, sub-only, etc.)
            debug!("[IRC Chat] Room state update: {}", trimmed);

            // Extract channel so the synthetic message is routable and the cache
            // is keyed per channel.
            let channel_name = extract_channel_from_irc_line(trimmed);

            // Forward room state to frontend — only include tags actually present
            // Twitch sends FULL roomstate on join, PARTIAL on setting changes
            let mut room_state = serde_json::Map::new();
            room_state.insert("type".into(), serde_json::json!("ROOMSTATE"));
            if let Some(ref ch) = channel_name {
                room_state.insert("channel".into(), serde_json::json!(ch));
            }

            if let Some(v) = Self::extract_tag_value(trimmed, "followers-only") {
                if let Ok(n) = v.parse::<i64>() {
                    room_state.insert("followers_only".into(), serde_json::json!(n));
                }
            }
            if let Some(v) = Self::extract_tag_value(trimmed, "slow") {
                if let Ok(n) = v.parse::<u64>() {
                    room_state.insert("slow".into(), serde_json::json!(n));
                }
            }
            if let Some(v) = Self::extract_tag_value(trimmed, "subs-only") {
                if let Ok(n) = v.parse::<u8>() {
                    room_state.insert("subs_only".into(), serde_json::json!(n == 1));
                }
            }
            if let Some(v) = Self::extract_tag_value(trimmed, "emote-only") {
                if let Ok(n) = v.parse::<u8>() {
                    room_state.insert("emote_only".into(), serde_json::json!(n == 1));
                }
            }
            if let Some(v) = Self::extract_tag_value(trimmed, "r9k") {
                if let Ok(n) = v.parse::<u8>() {
                    room_state.insert("r9k".into(), serde_json::json!(n == 1));
                }
            }

            let room_state_str = serde_json::Value::Object(room_state).to_string();

            // Cache the room state per channel so late-mounting MultiChat tabs
            // for any subscribed channel get its current state on connect.
            if let Some(ref ch) = channel_name {
                get_room_state_cache()
                    .lock()
                    .await
                    .insert(ch.clone(), room_state_str.clone());
                // Twitch always sends ROOMSTATE on a successful join — the
                // deterministic JOIN ack.
                confirm_join(ch).await;
            }

            send_to_bridge(room_state_str, false).await;

            // Check for shared chat information. Spawned: this is a Helix HTTP
            // round-trip (pure cache refresh — enhance_message reads the cache
            // on later PRIVMSGs) and ROOMSTATE fires on every setting change,
            // so it must never sit on the read loop.
            if let Some(room_id) = Self::extract_tag_value(trimmed, "room-id") {
                tokio::spawn(async move {
                    Self::check_shared_chat_status(&room_id).await;
                });
            }
        } else if trimmed.contains("USERSTATE") {
            // User state in channel (mod status, badges, etc.)
            // USERSTATE is sent when joining a channel AND after sending a message
            // It contains the user's badges which we need for optimistic message display
            debug!("[IRC Chat] User state update: {}", trimmed);

            // Extract channel so the user's per-channel badges are keyed and the
            // synthetic wire message carries the channel for frontend routing.
            let channel_name = extract_channel_from_irc_line(trimmed);

            // USERSTATE arrives on join (and after own sends) — a JOIN ack.
            if let Some(ref ch) = channel_name {
                confirm_join(ch).await;
            }

            // Extract badges from USERSTATE and cache them per channel
            if let Some(badges) = Self::extract_tag_value(trimmed, "badges") {
                debug!(
                    "[IRC Chat] Caching user badges from USERSTATE for {:?}: {}",
                    channel_name, badges
                );
                if let Some(ref ch) = channel_name {
                    get_user_badges_cache()
                        .lock()
                        .await
                        .insert(ch.clone(), badges.clone());
                }

                // Send badges to frontend tagged with the channel they apply to.
                // Format: USER_BADGES:#<channel>:<badges>. The leading '#' lets
                // the frontend parser locate the channel-prefix segment unambiguously.
                let badges_message = match &channel_name {
                    Some(ch) => format!("USER_BADGES:#{}:{}", ch, badges),
                    None => format!("USER_BADGES:{}", badges),
                };
                send_to_bridge(badges_message, false).await;
            }

            // Cache and forward the user's own chat color. An empty tag means the
            // user never set one (Twitch leaves the client to pick a default), so
            // only propagate a real value and let the frontend default stand.
            if let Some(color) = Self::extract_tag_value(trimmed, "color") {
                if !color.is_empty() {
                    if let Some(ref ch) = channel_name {
                        get_user_color_cache()
                            .lock()
                            .await
                            .insert(ch.clone(), color.clone());
                    }
                    let color_message = match &channel_name {
                        Some(ch) => format!("USER_COLOR:#{}:{}", ch, color),
                        None => format!("USER_COLOR:{}", color),
                    };
                    send_to_bridge(color_message, false).await;
                }
            }

            // Extract emote-sets to fetch user's subscribed emotes
            if let Some(emote_sets) = Self::extract_tag_value(trimmed, "emote-sets") {
                debug!(
                    "[IRC Chat] User has {} emote sets available",
                    emote_sets.split(',').count()
                );
                // TODO: Fetch emotes from user's subscribed sets
                // This would require additional API calls to get emotes from each set
            }
        } else if trimmed.contains("CLEARMSG") {
            // Single message deleted by mod
            // Format: @login=<user>;room-id=<room>;target-msg-id=<msg-id>;tmi-sent-ts=<ts> :tmi.twitch.tv CLEARMSG #<channel> :<message>
            debug!("[IRC Chat] Message deleted: {}", trimmed);

            if let Some(target_msg_id) = Self::extract_tag_value(trimmed, "target-msg-id") {
                let channel_name = extract_channel_from_irc_line(trimmed);
                // The deleted message text is the trailing param after the last " :".
                let deleted_text = trimmed
                    .rfind(" :")
                    .map(|idx| trimmed[idx + 2..].trim().to_string());
                let login = Self::extract_tag_value(trimmed, "login").unwrap_or_default();
                if let Some(ch) = &channel_name {
                    ChatLoggerService::log_deleted_message(ch, &login, deleted_text.as_deref());
                    ChatHistory::mark_deleted(ch, &target_msg_id);
                }
                // Send deletion event to frontend, tagged with channel for routing
                let delete_event = json!({
                    "type": "CLEARMSG",
                    "channel": channel_name,
                    "target_msg_id": target_msg_id,
                    "login": login,
                    "message": deleted_text
                });
                send_to_bridge(delete_event.to_string(), false).await;
            }
        } else if trimmed.contains("CLEARCHAT") {
            // User timed out/banned (clear all their messages) or chat cleared
            // Format: @ban-duration=<sec>;room-id=<room>;target-user-id=<id>;tmi-sent-ts=<ts> :tmi.twitch.tv CLEARCHAT #<channel> :<user>
            // Or for full chat clear: :tmi.twitch.tv CLEARCHAT #<channel>
            debug!("[IRC Chat] Chat clear/timeout: {}", trimmed);

            let target_user_id = Self::extract_tag_value(trimmed, "target-user-id");
            let ban_duration = Self::extract_tag_value(trimmed, "ban-duration");
            let channel_name = extract_channel_from_irc_line(trimmed);

            // Extract target username from the message content (after the colon at the end).
            // For full chat clears there's no trailing " :" so be careful not to misinterpret
            // an earlier inline colon as the username delimiter.
            let target_user = if let Some(idx) = trimmed.rfind(" :") {
                Some(trimmed[idx + 2..].trim().to_string())
            } else {
                None
            };

            let ban_duration_secs = ban_duration.map(|d| d.parse::<u64>().unwrap_or(0));
            if let Some(ch) = &channel_name {
                match &target_user {
                    Some(user) => ChatLoggerService::log_timeout(ch, user, ban_duration_secs),
                    None => ChatLoggerService::log_chat_cleared(ch),
                }
                if let Some(uid) = &target_user_id {
                    ChatHistory::mark_user_cleared(ch, uid);
                }
            }

            let clear_event = json!({
                "type": "CLEARCHAT",
                "channel": channel_name,
                "target_user_id": target_user_id,
                "target_user": target_user,
                "ban_duration": ban_duration_secs
            });
            send_to_bridge(clear_event.to_string(), false).await;
        } else if trimmed.contains("NOTICE") {
            // System notices — forward to frontend for user-facing handling
            debug!("[IRC Chat] Notice: {}", trimmed);

            // Extract the msg-id tag (e.g. "msg_followersonly", "msg_subsonly")
            // Present when twitch.tv/tags capability is active (requested at connect)
            let msg_id = Self::extract_tag_value(trimmed, "msg-id");

            // A suspended channel can never confirm its JOIN; forget it so the
            // JOIN watchdog doesn't drop sessions chasing it forever.
            if msg_id.as_deref() == Some("msg_channel_suspended") {
                if let Some(ch) = extract_channel_from_irc_line(trimmed) {
                    record_lifecycle(&format!("#{} suspended; dropping from desired set", ch));
                    tracker_forget(&ch).await;
                    get_current_channels().lock().await.remove(&ch);
                }
            }

            // Extract the human-readable notice text after the last " :"
            let notice_text = trimmed
                .rfind(" :")
                .map(|idx| trimmed[idx + 2..].trim().to_string());

            let notice_event = serde_json::json!({
                "type": "NOTICE",
                "channel": extract_channel_from_irc_line(trimmed),
                "msg_id": msg_id,
                "message": notice_text,
            });
            send_to_bridge(notice_event.to_string(), false).await;
        } else if let Some(join_ch) = parse_join_channel(trimmed) {
            // JOIN frame — ours or any member's (twitch.tv/membership relays
            // them only for channels we are in). Membership proof for the ack
            // tracker; hint-gated so big-channel join floods stay off the lock.
            confirm_join_if_pending(&join_ch).await;
        }

        Ok(())
    }

    async fn enhance_message_with_shared_chat(message: &str) -> std::borrow::Cow<'_, str> {
        // No shared-chat session known anywhere: borrow the line untouched
        // (this ran an unconditional String copy per PRIVMSG before).
        if !SHARED_CHAT_ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
            return std::borrow::Cow::Borrowed(message);
        }
        // Extract room-id from the message to determine source channel
        if let Some(room_id) = Self::extract_tag_value(message, "room-id") {
            // Check if this room is part of a shared chat session
            let shared_rooms = get_shared_chat_rooms().lock().await;

            // If this room has shared chat partners
            if let Some(_partners) = shared_rooms.get(&room_id) {
                // Add shared-chat-room tag to indicate which room the message is from
                if let Some(_user_login) = Self::extract_user_login_from_message(message) {
                    // Try to determine which partner room this user is from
                    // This would require additional API calls to check user's subscription status
                    // For now, we'll just mark it as shared chat

                    let mut enhanced = message.to_string();

                    // Insert shared-chat-room tag
                    if let Some(tag_end) = enhanced.find(" :") {
                        let shared_tag = format!("shared-chat-room={};", room_id);
                        enhanced.insert_str(tag_end - 1, &shared_tag);
                    }

                    return std::borrow::Cow::Owned(enhanced);
                }
            }
        }

        std::borrow::Cow::Borrowed(message)
    }

    async fn check_shared_chat_status(room_id: &str) {
        // Check if this broadcaster is in a shared chat session
        match TwitchService::get_token().await {
            Ok(token) => {
                let client = crate::services::http::client().clone();
                let url = format!(
                    "https://api.twitch.tv/helix/shared_chat/session?broadcaster_id={}",
                    room_id
                );

                match client
                    .get(&url)
                    .header("Client-Id", env!("TWITCH_APP_CLIENT_ID"))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await
                {
                    Ok(response) => {
                        if response.status().is_success() {
                            if let Ok(json) = response.json::<serde_json::Value>().await {
                                if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
                                    if let Some(session) = data.first() {
                                        // Extract participant broadcaster IDs
                                        if let Some(participants) =
                                            session.get("participants").and_then(|p| p.as_array())
                                        {
                                            let mut shared_rooms =
                                                get_shared_chat_rooms().lock().await;
                                            let partner_ids: Vec<String> = participants
                                                .iter()
                                                .filter_map(|p| {
                                                    p.get("broadcaster_id")
                                                        .and_then(|id| id.as_str())
                                                })
                                                .map(|s| s.to_string())
                                                .collect();

                                            // Store all participants as shared chat partners
                                            for id in &partner_ids {
                                                shared_rooms
                                                    .insert(id.clone(), partner_ids.clone());
                                            }
                                            if !partner_ids.is_empty() {
                                                SHARED_CHAT_ACTIVE.store(
                                                    true,
                                                    std::sync::atomic::Ordering::Relaxed,
                                                );
                                            }

                                            debug!(
                                                "[IRC Chat] Detected shared chat session with {} participants",
                                                partner_ids.len()
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("[IRC Chat] Failed to check shared chat status: {}", e);
                    }
                }
            }
            Err(_) => {}
        }
    }

    /// Fetch the channel's cheermote set (Twitch globals + its `channel_custom`
    /// prefixes) from Helix and cache it. Session-cached: a hit is a no-op, a
    /// PART evicts, so a re-JOIN refreshes. On any failure nothing is cached
    /// and parse_cheermote falls back to its static global list.
    async fn fetch_and_store_cheermotes(key: String, broadcaster_id: String) {
        if get_channel_cheermotes().read().is_ok_and(|g| g.contains_key(&key)) {
            return;
        }
        let token = match TwitchService::get_token().await {
            Ok(t) => t,
            Err(_) => return,
        };
        let client = crate::services::http::client().clone();
        let url = format!(
            "https://api.twitch.tv/helix/bits/cheermotes?broadcaster_id={}",
            broadcaster_id
        );
        let response = match client
            .get(&url)
            .header("Client-Id", env!("TWITCH_APP_CLIENT_ID"))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                error!(
                    "[IRC Chat] Cheermote fetch for {} returned {}",
                    key,
                    r.status()
                );
                return;
            }
            Err(e) => {
                error!("[IRC Chat] Cheermote fetch for {} failed: {}", key, e);
                return;
            }
        };
        let json: serde_json::Value = match response.json().await {
            Ok(j) => j,
            Err(e) => {
                error!("[IRC Chat] Cheermote response for {} unreadable: {}", key, e);
                return;
            }
        };
        let set = Self::cheermote_set_from_helix(&json);
        if set.is_empty() {
            return;
        }
        debug!(
            "[IRC Chat] Cached {} cheermote prefixes for {}",
            set.len(),
            key
        );
        if let Ok(mut g) = get_channel_cheermotes().write() { g.insert(key, Arc::new(set)); }
    }

    /// Convert a raw Helix `bits/cheermotes` response into the parse map.
    /// Chat renders cheermotes at ~28px, so the 2x dark animated image is
    /// preferred (1x fallback); a tier with no animated art at all falls back
    /// to its static image rather than dropping. Tiers a viewer can't cheer
    /// (`can_cheer: false`) are kept: they still RENDER when someone with
    /// access used them.
    fn cheermote_set_from_helix(json: &serde_json::Value) -> CheermoteSet {
        let mut set: CheermoteSet = HashMap::new();
        let Some(data) = json.get("data").and_then(|d| d.as_array()) else {
            return set;
        };
        for entry in data {
            let Some(prefix) = entry.get("prefix").and_then(|p| p.as_str()) else {
                continue;
            };
            let Some(tiers) = entry.get("tiers").and_then(|t| t.as_array()) else {
                continue;
            };
            let mut parsed: Vec<CheermoteTier> = tiers
                .iter()
                .filter_map(|t| {
                    let min_bits = t.get("min_bits").and_then(|m| m.as_u64())? as u32;
                    let color = t.get("color").and_then(|c| c.as_str())?.to_string();
                    let dark = t.get("images")?.get("dark")?;
                    let url = [("animated", "2"), ("animated", "1"), ("static", "2"), ("static", "1")]
                        .iter()
                        .find_map(|(kind, size)| dark.get(kind)?.get(size)?.as_str())?
                        .to_string();
                    Some(CheermoteTier {
                        min_bits,
                        color,
                        url,
                    })
                })
                .collect();
            if parsed.is_empty() {
                continue;
            }
            parsed.sort_by_key(|t| t.min_bits);
            set.insert(prefix.to_lowercase(), parsed);
        }
        set
    }

    fn extract_tag_value(message: &str, tag_name: &str) -> Option<String> {
        if !message.starts_with('@') {
            return None;
        }

        let tag_section = message.split(' ').next()?;
        let tags = &tag_section[1..]; // Skip the '@'

        for tag in tags.split(';') {
            let parts: Vec<&str> = tag.splitn(2, '=').collect();
            if parts.len() == 2 && parts[0] == tag_name {
                return Some(parts[1].to_string());
            }
        }

        None
    }

    fn extract_user_login_from_message(message: &str) -> Option<String> {
        // Extract from the prefix: :username!username@username.tmi.twitch.tv
        let parts: Vec<&str> = message.split(' ').collect();
        if parts.len() > 1 {
            let prefix = parts[1];
            if prefix.starts_with(':') && prefix.contains('!') {
                let username = prefix[1..].split('!').next()?;
                return Some(username.to_string());
            }
        }
        None
    }

    async fn handle_local_ws(
        local_socket: warp::ws::WebSocket,
        tx: Arc<broadcast::Sender<String>>,
    ) {
        let (mut local_tx, _local_rx) = local_socket.split();
        let mut rx = tx.subscribe();

        debug!("[WS] New local WebSocket client connected");

        // Replay cached room state + badges so a late-mounting tab doesn't wait for
        // the next ROOMSTATE/USERSTATE. Snapshot under the lock, release BEFORE
        // awaiting sends (holding it across an await can deadlock the reader).
        let room_states: Vec<String> = {
            let cache = get_room_state_cache().lock().await;
            cache.values().cloned().collect()
        };
        for state in room_states {
            let _ = local_tx.send(warp::ws::Message::text(state)).await;
        }

        // Then anything published before this client existed. Ordered AFTER room
        // state so the pane knows the channel's modes before its first rows land.
        let held = take_undelivered_messages().await;
        if !held.is_empty() {
            info!("[WS] replaying {} message(s) held for a late client", held.len());
            for msg in held {
                let _ = local_tx.send(warp::ws::Message::text(msg)).await;
            }
        }

        let badge_entries: Vec<(String, String)> = {
            let cache = get_user_badges_cache().lock().await;
            cache
                .iter()
                .map(|(ch, badges)| (ch.clone(), badges.clone()))
                .collect()
        };
        for (channel, badges) in badge_entries {
            let badges_message = format!("USER_BADGES:#{}:{}", channel, badges);
            let _ = local_tx.send(warp::ws::Message::text(badges_message)).await;
        }

        let color_entries: Vec<(String, String)> = {
            let cache = get_user_color_cache().lock().await;
            cache
                .iter()
                .map(|(ch, color)| (ch.clone(), color.clone()))
                .collect()
        };
        for (channel, color) in color_entries {
            let color_message = format!("USER_COLOR:#{}:{}", channel, color);
            let _ = local_tx.send(warp::ws::Message::text(color_message)).await;
        }

        // Send any queued messages first
        let mut queue = get_message_queue().lock().await;
        let queued_count = queue.len();
        if queued_count > 0 {
            debug!(
                "[WS] Sending {} queued messages to new client",
                queued_count
            );
            while let Some(msg) = queue.pop_front() {
                if local_tx.send(warp::ws::Message::text(msg)).await.is_err() {
                    debug!("[WS] Client disconnected while sending queued messages");
                    return;
                }
            }
        }
        drop(queue);

        // Forward messages from the broadcast to the local client.
        //
        // `while let Ok(..) = rx.recv()` is wrong here: it exits on
        // `RecvError::Lagged`, which fires whenever a subscriber falls behind the
        // channel capacity. Lagged is a recoverable miss, so log it and keep
        // draining. Only `Closed` tears the handler down.
        loop {
            match rx.recv().await {
                Ok(text) => {
                    if local_tx.send(warp::ws::Message::text(text)).await.is_err() {
                        debug!("[WS] Client disconnected");
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!(
                        "[WS] Subscriber lagged behind by {} messages; continuing",
                        n
                    );
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => {
                    debug!("[WS] Broadcast channel closed");
                    break;
                }
            }
        }
    }

    pub async fn send_message(
        message: &str,
        reply_parent_msg_id: Option<&str>,
        target_channel: Option<&str>,
    ) -> Result<()> {
        // Resolve the target channel without locking the channel set across the
        // send. Falls back to "the only currently-joined channel" when the caller
        // didn't supply one (legacy single-channel callers); otherwise uses the
        // caller's explicit target.
        let channel = match target_channel {
            Some(c) => c.to_lowercase(),
            None => {
                let channels = get_current_channels().lock().await;
                channels
                    .iter()
                    .next()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("No active chat connection"))?
            }
        };

        // A window's JS store can believe it's JOINed while Rust's
        // `current_channels` no longer contains the channel (state lost across
        // a bridge restart, or a racing PART). Re-JOIN defensively when the
        // caller is sending to a channel we don't think is active. Ensure-only:
        // the sender's window already claimed its consumer slot when it
        // acquired the channel, so claiming another here would leave a slot
        // nothing releases. Cheap on success, recoverable on conflict.
        let needs_join = !get_current_channels().lock().await.contains(&channel);
        if needs_join {
            log::warn!(
                "[IRC Chat] send_message for {} but channel not in current set; defensive re-JOIN",
                channel
            );
            if let Err(e) = Self::ensure_joined(&channel).await {
                return Err(anyhow::anyhow!(
                    "Failed to re-JOIN channel {} before send: {}",
                    channel,
                    e
                ));
            }
        }

        let writer_lock = get_irc_writer().lock().await;
        let writer = writer_lock
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("IRC connection not established"))?
            .clone();
        drop(writer_lock);

        let mut w = writer.lock().await;

        // Format message with reply if needed
        let formatted_message = if let Some(parent_id) = reply_parent_msg_id {
            format!(
                "@reply-parent-msg-id={} PRIVMSG #{} :{}\r\n",
                parent_id, channel, message
            )
        } else {
            format!("PRIVMSG #{} :{}\r\n", channel, message)
        };

        debug!("[IRC Chat] Sending message: {}", message);
        w.send_line(&formatted_message).await?;
        drop(w);

        // Messages sent over THIS connection get no IRC echo (Helix sends do,
        // and reach the parsed-message path like anyone else's). Surface them
        // to the chat logger and plugins from here, mirroring the chat UI's
        // local echo. Slash-commands other than /me are not chat lines (their
        // effects, like timeouts, are logged where the server reports them).
        let is_command = message.starts_with('/') && !message.starts_with("/me ");
        if !is_command {
            let login = get_own_identity()
                .lock()
                .await
                .as_ref()
                .map(|(login, _)| login.clone())
                .unwrap_or_default();
            ChatLoggerService::log_own_message(&channel, &login, message);
        }
        // Plugin delivery uses an empty id (no server-assigned id exists).
        if !is_command {
            if let Some(host) = PLUGIN_HOST.get() {
                if host.wants_chat_messages().await {
                    let (login, user_id) =
                        get_own_identity().lock().await.clone().unwrap_or_default();
                    let (text, is_action) = match message.strip_prefix("/me ") {
                        Some(rest) => (rest, true),
                        None => (message, false),
                    };
                    host.emit_chat_message(json!({
                        "channel": channel,
                        "message": {
                            "id": "",
                            "user_id": user_id,
                            "login": login,
                            "display_name": login,
                            "color": Value::Null,
                            "badges": [],
                            "text": text,
                            "is_action": is_action,
                            "msg_type": Value::Null,
                            "system_message": Value::Null,
                            "bits": Value::Null,
                            "ts": chrono::Utc::now().to_rfc3339(),
                        }
                    }))
                    .await;
                }
            }
        }

        Ok(())
    }

    /// Wait up to `max_attempts` × 50ms for the IRC writer to be set. `start`
    /// spawns `run_irc_connection` (which sets the writer once the TCP socket is
    /// up) and returns the WS port *before* that task has connected. So a JOIN
    /// issued right after a fresh `start_chat` — e.g. several chats opening at
    /// once, where the first channel's connection is still in flight — can
    /// momentarily see no writer. Polling smooths over that startup window
    /// instead of failing the JOIN outright.
    async fn wait_for_irc_writer(
        max_attempts: u32,
    ) -> Option<Arc<Mutex<IrcWriter>>> {
        for attempt in 0..max_attempts {
            if let Some(writer) = get_irc_writer().lock().await.as_ref() {
                return Some(writer.clone());
            }
            if attempt + 1 < max_attempts {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }
        }
        None
    }

    pub async fn join_channel(channel: &str, window: &str) -> Result<()> {
        let key = channel.to_lowercase();

        // Claim first. Multiple windows may JOIN the same channel concurrently
        // (main + N MultiChat popouts); each records its window label in the
        // channel's consumer set, and only the transition from no consumers
        // sends the IRC JOIN. A window re-claiming a channel it already holds
        // (a webview reload re-acquiring, a reconnect re-attach) is a no-op,
        // which is what keeps claims from inflating: the set can never hold a
        // window twice, so releases always balance.
        let newly_claimed = {
            let mut consumers = get_channel_consumers().lock().await;
            consumers
                .entry(key.clone())
                .or_default()
                .insert(window.to_string())
        };

        if !newly_claimed {
            debug!(
                "[IRC Chat] join_channel({}): window {} already a consumer, reusing JOIN",
                key, window
            );
            // Still run the JOIN health probe: for a confirmed channel this is
            // two set lookups, but for a channel whose JOIN the server silently
            // dropped it re-issues the JOIN — which is what turns the user's
            // refresh into a real recovery instead of an IRC no-op.
            return Self::ensure_joined(&key).await;
        }

        // Make sure the channel is actually JOINed (no-op when another window
        // got there first). On failure, give back the claim so a later retry
        // can JOIN cleanly instead of seeing an existing consumer.
        if let Err(e) = Self::ensure_joined(&key).await {
            let mut consumers = get_channel_consumers().lock().await;
            if let Some(entry) = consumers.get_mut(&key) {
                entry.remove(window);
                if entry.is_empty() {
                    consumers.remove(&key);
                }
            }
            return Err(e);
        }

        Ok(())
    }

    /// Send the IRC JOIN for `key` (lowercase) and set up its per-channel
    /// subscriptions. For a channel already in the desired set this is a health
    /// probe: no-op while the JOIN is confirmed or in flight, but a channel the
    /// server silently un-JOINed (desired, yet neither confirmed nor pending)
    /// gets its JOIN re-issued. Never touches the consumer sets; callers decide
    /// whether a consumer claim is recorded.
    async fn ensure_joined(key: &str) -> Result<()> {
        let newly_desired = get_current_channels().lock().await.insert(key.to_string());
        if !newly_desired {
            let (confirmed, pending) = get_join_tracker().lock().await.is_settled(key);
            if confirmed || pending {
                return Ok(());
            }
            record_lifecycle(&format!(
                "JOIN #{} lost (desired but unconfirmed); re-issuing",
                key
            ));
        }

        // The connection may still be establishing: start_chat spawns the IRC
        // task and returns before the writer is set, so when several chats open
        // at once an additional channel's JOIN can arrive before the first
        // channel finishes connecting. Wait for the writer instead of failing —
        // bailing here would also skip the per-channel mod-view / 7TV
        // subscriptions below, which is exactly why a second chat opened in the
        // same burst would silently receive no moderator events.
        let supervisor_alive = matches!(
            get_irc_handle().lock().await.as_ref(),
            Some(h) if !h.is_finished()
        );
        match Self::wait_for_irc_writer(100).await {
            Some(writer) => {
                let write_result = async {
                    let mut w = writer.lock().await;
                    w.send_line(&format!("JOIN #{}\r\n", key)).await
                }
                .await;
                match write_result {
                    Ok(()) => {
                        tracker_record_sent(key, 0, false).await;
                        debug!("[IRC Chat] Joined channel: #{}", key);
                    }
                    Err(e) => {
                        // Undo the desired-state insert so join_channel's claim
                        // rollback leaves clean state for a later retry.
                        if newly_desired {
                            get_current_channels().lock().await.remove(key);
                        }
                        return Err(e.into());
                    }
                }
            }
            // Between supervisor sessions (reconnect/backoff) there is no
            // writer. Keep the channel recorded anyway: CURRENT_CHANNELS is the
            // desired-state set and the next session JOINs everything in it.
            // Without this, a channel switch during a reconnect window lost
            // its JOIN permanently. Deliberately NOT recorded in the ack
            // tracker — the next session's burst records it when it actually
            // writes the JOIN.
            None if supervisor_alive => {
                record_lifecycle(&format!("JOIN #{} deferred to next session", key));
            }
            None => {
                if newly_desired {
                    get_current_channels().lock().await.remove(key);
                }
                return Err(anyhow::anyhow!("IRC connection not established"));
            }
        }

        // First time this channel is wanted: shared-chat lookup, 7TV EventAPI and
        // mod-view subscriptions. A health-probe re-issue must not repeat them.
        // Spawned rather than awaited so the socket handoff does not wait on three
        // network calls. Re-checks CURRENT_CHANNELS; every call is idempotent.
        if newly_desired {
            let key = key.to_string();
            tokio::spawn(async move {
                let Ok(broadcaster_info) = TwitchService::get_user_by_login(&key).await else {
                    return;
                };
                if !get_current_channels().lock().await.contains(&key) {
                    return; // parted before the lookup came back
                }
                Self::check_shared_chat_status(&broadcaster_info.id).await;
                crate::services::seventv_eventapi::subscribe_channel(&key, &broadcaster_info.id)
                    .await;
                crate::services::eventsub_moderation::subscribe_channel(&key, &broadcaster_info.id)
                    .await;
            });
        }

        Ok(())
    }

    pub async fn leave_channel(channel: &str, window: &str) -> Result<()> {
        let key = channel.to_lowercase();

        // Release the claim first. The MultiChat popout flow fires `start_chat`
        // for the new window's channel and then unmounts main's ChatWidget,
        // which fires `leave_chat_channel` for the same channel. Without the
        // consumer sets, the unconditional PART here would race with — and
        // usually lose to — the popout's start_chat, leaving the popout
        // subscribed to a channel nobody is JOINed to. Only the last
        // consumer's leave actually PARTs.
        let remaining = {
            let mut consumers = get_channel_consumers().lock().await;
            match consumers.get_mut(&key) {
                Some(entry) => {
                    entry.remove(window);
                    let n = entry.len();
                    if n == 0 {
                        consumers.remove(&key);
                    }
                    n
                }
                None => 0,
            }
        };

        if remaining > 0 {
            debug!(
                "[IRC Chat] leave_channel({}): {} consumer(s) remain, keeping JOIN",
                key, remaining
            );
            return Ok(());
        }

        Self::part_channel(&key).await?;
        debug!("[IRC Chat] Left channel: #{} (last consumer)", key);
        Ok(())
    }

    /// Drops every consumer claim held by `window`, PARTing channels whose
    /// consumer set empties. `keep` exempts a channel the window is in the
    /// middle of claiming.
    ///
    /// Called when a window is destroyed, since its React cleanup never runs,
    /// and on a fresh-claim `start_chat`, where any claims still recorded for
    /// the window belong to a previous JS context of it.
    pub async fn release_window_claims(window: &str, keep: Option<&str>) {
        let emptied: Vec<String> = {
            let mut consumers = get_channel_consumers().lock().await;
            let mut emptied = Vec::new();
            consumers.retain(|channel, set| {
                if keep == Some(channel.as_str()) {
                    return true;
                }
                set.remove(window);
                if set.is_empty() {
                    emptied.push(channel.clone());
                    false
                } else {
                    true
                }
            });
            emptied
        };

        for channel in emptied {
            debug!(
                "[IRC Chat] releasing stale claim on {} held by window {}",
                channel, window
            );
            if let Err(e) = Self::part_channel(&channel).await {
                log::warn!("[IRC Chat] stale-claim PART for {} failed: {}", channel, e);
            }
        }
    }

    /// PART any joined channel that no window claims, except `keep`. Joined-
    /// but-unclaimed channels come from the ensure-only paths: the stream
    /// warm-up JOINs before any chat UI mounts, and if no UI ever claims on
    /// top (chat hidden), there is no release to retire the room on a stream
    /// switch. The next warm-up reaps them here instead.
    async fn part_unclaimed(keep: &str) {
        let joined: Vec<String> = get_current_channels()
            .lock()
            .await
            .iter()
            .cloned()
            .collect();
        let orphans: Vec<String> = {
            let consumers = get_channel_consumers().lock().await;
            joined
                .into_iter()
                .filter(|c| c != keep && consumers.get(c).is_none_or(|s| s.is_empty()))
                .collect()
        };
        for channel in orphans {
            debug!("[IRC Chat] PARTing unclaimed channel {}", channel);
            if let Err(e) = Self::part_channel(&channel).await {
                log::warn!("[IRC Chat] unclaimed PART for {} failed: {}", channel, e);
            }
        }
    }

    /// Send the IRC PART for `key` (lowercase) and tear down its per-channel
    /// caches and subscriptions. Consumer accounting is the caller's job.
    async fn part_channel(key: &str) -> Result<()> {
        // Best-effort wire PART: between supervisor sessions there is no
        // writer, and the socket may be dead anyway. Membership bookkeeping
        // must happen regardless; removal from CURRENT_CHANNELS is what
        // guarantees the next session does not re-JOIN a channel nobody
        // watches.
        if let Some(writer) = get_irc_writer().lock().await.as_ref().cloned() {
            let mut w = writer.lock().await;
            let _ = w.send_line(&format!("PART #{}\r\n", key)).await;
        }

        get_current_channels().lock().await.remove(key);
        tracker_forget(key).await;

        // Drop per-channel caches so PARTed channels don't accumulate memory.
        // If the user re-JOINs later, fetch_and_store_emotes runs again and
        // USERSTATE/ROOMSTATE refill from the next IRC frames.
        get_channel_emotes().lock().await.remove(key);
        drop_parse_lookup(key);
        if let Ok(mut g) = get_channel_cheermotes().write() { g.remove(key); }
        get_user_badges_cache().lock().await.remove(key);
        get_user_color_cache().lock().await.remove(key);
        get_room_state_cache().lock().await.remove(key);
        // The search ring lives and dies with the channel's last consumer.
        ChatHistory::clear_channel(key);
        crate::services::automod_queue::AutomodQueue::clear_channel(key);
        crate::services::suspicious_users::SuspiciousUsers::clear_channel(key);

        // Stop receiving 7TV EventAPI updates for this channel.
        crate::services::seventv_eventapi::unsubscribe_channel(key).await;
        // Stop the moderator-view subscription for this channel.
        crate::services::eventsub_moderation::unsubscribe_channel(key).await;

        Ok(())
    }

    /// Fetch and store channel emotes for the current channel. Returns the
    /// resolved Twitch channel id (broadcaster user id) on success so callers
    /// can drive the 7TV EventAPI subscription off the same lookup.
    /// Fetch this channel's emotes, WAITING for the providers to answer.
    ///
    /// Only for callers that must have the live set in hand before they parse
    /// anything (VOD replay). The chat start path must use
    /// [`Self::seed_emotes_deferring_refresh`] instead.
    pub async fn fetch_and_store_emotes(
        channel_name: &str,
        emote_service: Arc<tokio::sync::RwLock<EmoteService>>,
    ) -> Option<String> {
        Self::resolve_emotes(channel_name, emote_service, false).await
    }

    /// Makes the channel parseable and returns, refreshing from the network
    /// afterwards.
    ///
    /// The chat socket must not wait on third-party emote providers. Only the
    /// work that decides correctness runs first: the broadcaster lookup and the
    /// disk-dictionary seed, which is what the visible backlog and the first
    /// live messages tokenize against.
    ///
    /// Callers that must hold the live set before parsing anything should use
    /// [`Self::fetch_and_store_emotes`] instead. On a channel with no saved
    /// dictionary yet, emotes briefly render as their names.
    pub async fn seed_emotes_deferring_refresh(
        channel_name: &str,
        emote_service: Arc<tokio::sync::RwLock<EmoteService>>,
    ) -> Option<String> {
        Self::resolve_emotes(channel_name, emote_service, true).await
    }

    async fn resolve_emotes(
        channel_name: &str,
        emote_service: Arc<tokio::sync::RwLock<EmoteService>>,
        defer_refresh: bool,
    ) -> Option<String> {
        debug!("[IRC Chat] Fetching emotes for channel: {}", channel_name);

        // Pull the OAuth token so the cache write here matches what the
        // frontend's token-bearing fetch produces. Without this, an IRC-side
        // write can race ahead of the frontend's call and leave the shared
        // EmoteService cache containing only the 15 hardcoded globals — which
        // a freshly-opened MultiChat popout then reads and displays.
        let access_token = TwitchService::get_token().await.ok();

        // Get broadcaster ID from channel name
        match TwitchService::get_user_by_login(channel_name).await {
            Ok(user) => {
                let key = channel_name.to_lowercase();

                // Cheermotes ride the same join: Helix `bits/cheermotes` with
                // this broadcaster id is the only source of the channel's own
                // custom prefixes. Spawned so a slow Helix call can't delay the
                // emote path; the fetch no-ops if the channel is already cached.
                {
                    let cheer_key = key.clone();
                    let broadcaster_id = user.id.clone();
                    tokio::spawn(async move {
                        Self::fetch_and_store_cheermotes(cheer_key, broadcaster_id).await;
                    });
                }

                // Seed from the saved per-channel dictionary so emotes resolve with no
                // network round-trip, even when a provider is slow or down. Seed only when
                // nothing is in memory: an in-memory set was fetched or delta-patched this
                // session and is at least as fresh, so a second window joining the same
                // channel must not replace it.
                let mut seeded = false;
                if let Some(disk_set) = crate::services::emote_set_cache::load(&user.id) {
                    let mut map = get_channel_emotes().lock().await;
                    if !map.contains_key(&key) {
                        debug!(
                            "[IRC Chat] Seeded {} from disk dictionary (7TV: {})",
                            channel_name,
                            disk_set.seven_tv.len()
                        );
                        rebuild_parse_lookup(&key, &disk_set);
                        map.insert(key.clone(), disk_set);
                        seeded = true;
                    }
                }

                // A caller that asked to wait still gets the disk-seeded map back
                // at once when one landed: the channel document budget is 25 s
                // now (large channels need it) and nothing that can already parse
                // should sit on that. Only a channel with no dictionary at all is
                // worth waiting for.
                if defer_refresh || seeded {
                    // The chat socket does NOT wait for emote providers. See
                    // `refresh_channel_emotes` for why, and for the measurement.
                    //
                    // Gated, because deferring removed the start lock that used to
                    // serialize this. See `try_begin_emote_refresh`.
                    if let Some(gate) = try_begin_emote_refresh(&key) {
                        let name = channel_name.to_string();
                        let k = key.clone();
                        let uid = user.id.clone();
                        let svc = emote_service.clone();
                        tokio::spawn(async move {
                            let _gate = gate;
                            let _permit = emote_refresh_permits().acquire_owned().await.ok();
                            Self::refresh_channel_emotes(name, k, uid, access_token, svc).await;
                        });
                    }
                } else {
                    Self::refresh_channel_emotes(
                        channel_name.to_string(),
                        key.clone(),
                        user.id.clone(),
                        access_token,
                        emote_service.clone(),
                    )
                    .await;
                }
                Some(user.id)
            }
            Err(e) => {
                error!(
                    "[IRC Chat] Failed to get user info for {}: {}",
                    channel_name, e
                );
                None
            }
        }
    }

    /// Pulls the live third-party set and installs it as the channel's parse
    /// map.
    ///
    /// Installs only when the 7TV channel fetch definitively succeeded. A
    /// partial result, such as globals-only from a tripped circuit breaker or a
    /// timed-out channel set, leaves the disk-seeded set in place. An
    /// authoritative result is written through to disk so the next join is
    /// disk-first and removals persist.
    async fn refresh_channel_emotes(
        channel_name: String,
        key: String,
        user_id: String,
        access_token: Option<String>,
        emote_service: Arc<tokio::sync::RwLock<EmoteService>>,
    ) {
        // Snapshot the service out of the RwLock (guard drops at end of
        // statement) so the lock is never held across the network fetch; a
        // future writer would otherwise convoy every reader behind an
        // in-flight HTTP call.
        let emote_svc = emote_service.read().await.clone();
        match emote_svc
            .fetch_channel_emotes_checked(
                Some(channel_name.clone()),
                Some(user_id.clone()),
                access_token,
                // This path is Twitch's own IRC service.
                None,
            )
            .await
        {
            Ok((emote_set, seven_tv_ok)) => {
                debug!(
                    "[IRC Chat] Fetched {} total emotes for {} (Twitch: {}, BTTV: {}, 7TV: {}, FFZ: {}); 7TV channel ok: {}",
                    emote_set.total_count(),
                    channel_name,
                    emote_set.twitch.len(),
                    emote_set.bttv.len(),
                    emote_set.seven_tv.len(),
                    emote_set.ffz.len(),
                    seven_tv_ok
                );
                if seven_tv_ok {
                    crate::services::emote_set_cache::save_force(&user_id, &emote_set);
                    rebuild_parse_lookup(&key, &emote_set);
                    get_channel_emotes().lock().await.insert(key, emote_set);
                } else {
                    debug!(
                        "[IRC Chat] Keeping disk-seeded set for {}; 7TV channel fetch was deficient (7TV {})",
                        channel_name,
                        emote_set.seven_tv.len()
                    );
                }
            }
            Err(e) => {
                error!("[IRC Chat] Failed to fetch channel emotes: {}", e);
            }
        }
    }

    /// Applies a live 7TV set change to this channel's parse dictionary without
    /// a network fetch; the dispatch carries the emote itself.
    ///
    /// Returns what changed in the composed dictionary (channel rows, plus any
    /// global that a removal stops shadowing) so every other copy can be patched
    /// the same way, or `None` when the channel is not in memory.
    pub async fn apply_seventv_delta(
        key: &str,
        user_id: &str,
        delta: &crate::services::emote_service::SeventvSetDelta,
        globals: &[Emote],
    ) -> Option<crate::services::emote_service::SeventvComposedDelta> {
        let composed = {
            let mut map = get_channel_emotes().lock().await;
            let set = map.get_mut(key)?;
            let composed = crate::services::emote_service::apply_seventv_delta(
                &mut set.seven_tv,
                delta,
                globals,
            );
            rebuild_parse_lookup(key, set);
            composed
        };
        schedule_dictionary_write(key.to_string(), user_id.to_string());
        Some(composed)
    }

    /// Re-pull a channel's set from the providers and install it (authoritative
    /// only), for the EventAPI's resync after a reconnect it could not RESUME:
    /// anything dispatched during the gap was never applied. Runs under the same
    /// per-channel gate and permit as the join-time refresh.
    pub async fn resync_channel_emotes(
        channel_name: &str,
        user_id: &str,
        emote_service: Arc<tokio::sync::RwLock<EmoteService>>,
    ) {
        let key = channel_name.to_lowercase();
        let Some(gate) = try_begin_emote_refresh(&key) else {
            return; // a refresh is already in flight; it lands the same result
        };
        let _gate = gate;
        let _permit = emote_refresh_permits().acquire_owned().await.ok();
        {
            let svc = emote_service.read().await;
            svc.invalidate_channel(user_id).await;
        }
        let access_token = TwitchService::get_token().await.ok();
        Self::refresh_channel_emotes(
            channel_name.to_string(),
            key,
            user_id.to_string(),
            access_token,
            emote_service,
        )
        .await;
    }

    /// Ensure the channel's third-party emote set is in the parse cache for a
    /// channel we never JOIN over IRC (VOD chat replay). No-ops once a 7TV-bearing
    /// set is cached, so it fetches at most once per channel per session. Awaited
    /// so a following `parse_privmsg` resolves 7TV/BTTV/FFZ emotes.
    pub async fn ensure_channel_emotes_for_parse(
        channel_name: &str,
        emote_service: Arc<tokio::sync::RwLock<EmoteService>>,
    ) {
        let key = channel_name.to_lowercase();
        {
            let map = get_channel_emotes().lock().await;
            if map.get(&key).map(|s| s.seven_tv.len()).unwrap_or(0) > 0 {
                return;
            }
        }
        let _ = Self::fetch_and_store_emotes(channel_name, emote_service).await;
        Self::reap_parse_only_channels(&key).await;
    }

    /// Channels entered ONLY through the parse path (VOD replay) never JOIN, so
    /// part_channel's eviction never runs for them and their emote sets would
    /// accumulate for the session. A small ring reaps the oldest once more than
    /// a handful are held; membership in CURRENT_CHANNELS is re-checked at
    /// evict time so a channel that later genuinely JOINed is never touched.
    async fn reap_parse_only_channels(key: &str) {
        const PARSE_ONLY_MAX: usize = 8;
        static PARSE_ONLY_KEYS: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();
        let ring = PARSE_ONLY_KEYS.get_or_init(|| Mutex::new(VecDeque::new()));

        if get_current_channels().lock().await.contains(key) {
            return;
        }
        let evict = {
            let mut g = ring.lock().await;
            if !g.iter().any(|k| k == key) {
                g.push_back(key.to_string());
            }
            if g.len() > PARSE_ONLY_MAX {
                g.pop_front()
            } else {
                None
            }
        };
        if let Some(old_key) = evict {
            if !get_current_channels().lock().await.contains(&old_key) {
                get_channel_emotes().lock().await.remove(&old_key);
                drop_parse_lookup(&old_key);
                if let Ok(mut g) = get_channel_cheermotes().write() {
                    g.remove(&old_key);
                }
            }
        }
    }

    /// Parse message content into segments (text, emotes, emojis, links)
    /// This is the "endgame" - all parsing done in Rust, zero regex on main thread
    ///
    /// `channel` selects which channel's 7TV/FFZ/BTTV emote set to use. Empty string
    /// (or a channel with no cached emotes) yields no third-party emote matches but
    /// still parses Twitch native emotes and URLs.
    /// True if this user's personal set is already loaded for the same set id,
    /// so the 7TV EventAPI can skip a redundant re-fetch on presence rebootstrap
    /// or reconnect (the same entitlement is re-delivered every time).
    pub async fn has_personal_set(twitch_id: &str, set_id: &str) -> bool {
        get_personal_emotes()
            .read()
            .map(|g| g.peek(twitch_id).map(|(s, _)| s == set_id).unwrap_or(false))
            .unwrap_or(false)
    }

    /// Store a user's personal-use emotes (already filtered to the personal set).
    /// Stored even when empty so the set id is recorded for dedup; the presence
    /// flag only flips when there is at least one emote to overlay.
    pub async fn set_personal_emotes(twitch_id: String, set_id: String, emotes: Vec<Emote>) {
        let has_any = !emotes.is_empty();
        let map: HashMap<String, Emote> = emotes.into_iter().map(|e| (e.name.clone(), e)).collect();
        if let Ok(mut g) = get_personal_emotes().write() {
            g.put(twitch_id, (set_id, Arc::new(map)));
        }
        if has_any {
            PERSONAL_EMOTES_PRESENT.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Drop a user's personal emotes when their EMOTE_SET entitlement is revoked.
    /// Only clears when the revoked set matches what we hold (a stale delete for
    /// a set they no longer have must not wipe a newer one).
    pub async fn clear_personal_emotes(twitch_id: &str, set_id: Option<&str>) {
        let Ok(mut g) = get_personal_emotes().write() else {
            return;
        };
        match set_id {
            Some(sid) => {
                if g.peek(twitch_id).map(|(s, _)| s == sid).unwrap_or(false) {
                    g.pop(twitch_id);
                }
            }
            None => {
                g.pop(twitch_id);
            }
        }
        // Recompute rather than leave the fast-path gate latched on: this was
        // the one path that never reset it, so a single personal set ever seen
        // taxed every message for the rest of the session.
        PERSONAL_EMOTES_PRESENT.store(
            g.iter().any(|(_, (_, m))| !m.is_empty()),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// Wipe all personal emotes (full chat teardown).
    pub async fn clear_all_personal_emotes() {
        if let Ok(mut g) = get_personal_emotes().write() {
            g.clear();
        }
        PERSONAL_EMOTES_PRESENT.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Everything per-message parsing reads, snapshotted once per message with
    /// three brief uncontended lock reads. Parsing itself then runs fully
    /// synchronously with no locks and no data clones - the block_in_place +
    /// block_on bridge this replaced was a runtime-wide scheduling event per
    /// chat message.
    fn gather_parse_snapshots(channel: &str, sender_id: &str) -> ParseSnapshots {
        let channel_lookup = channel_parse_lookup()
            .read()
            .ok()
            .and_then(|g| g.get(channel).cloned());
        let personal = if sender_id.is_empty()
            || !PERSONAL_EMOTES_PRESENT.load(std::sync::atomic::Ordering::Relaxed)
        {
            None
        } else {
            get_personal_emotes()
                .read()
                .ok()
                .and_then(|g| g.peek(sender_id).map(|(_, m)| m.clone()))
        };
        let cheermotes = get_channel_cheermotes()
            .read()
            .ok()
            .and_then(|g| g.get(channel).cloned());
        ParseSnapshots {
            channel: channel_lookup,
            personal,
            cheermotes,
        }
    }

    /// Byte index in `content` right after a leading "@<name>" mention plus its
    /// following whitespace run, or 0 when no alternative matches. The boundary
    /// is REQUIRED (whitespace or end of message): with multiple alternatives, a
    /// short name that prefixes a longer one must never partially strip
    /// ("@foobarbaz" with login "foobar"). Case-folded per char; alternatives
    /// are tried in order (login before display name).
    fn reply_mention_end(content: &str, alts: &[&str]) -> usize {
        let Some(rest) = content.strip_prefix('@') else {
            return 0;
        };
        for alt in alts {
            if alt.is_empty() {
                continue;
            }
            let mut rest_chars = rest.char_indices();
            let mut alt_chars = alt.chars();
            let matched_end = loop {
                match alt_chars.next() {
                    // Name fully matched: the end is the next char's byte index
                    // (offset by the leading '@'), or end of message.
                    None => {
                        break Some(
                            rest_chars
                                .next()
                                .map(|(i, _)| 1 + i)
                                .unwrap_or(content.len()),
                        )
                    }
                    Some(ac) => match rest_chars.next() {
                        Some((_, rc)) if rc.to_lowercase().eq(ac.to_lowercase()) => {}
                        _ => break None,
                    },
                }
            };
            let Some(end) = matched_end else {
                continue;
            };
            let tail = &content[end..];
            if tail.is_empty() {
                return content.len();
            }
            if tail.starts_with(char::is_whitespace) {
                // Consume the whole whitespace run (the old pattern's greedy \s+).
                return end + (tail.len() - tail.trim_start().len());
            }
            // Boundary violated - the name only prefixes a longer word; try the
            // next alternative.
        }
        0
    }

    fn parse_message_segments(
        content: &str,
        twitch_emotes: &[EmotePos],
        ctx: &ParseCtx<'_>,
    ) -> Vec<MessageSegment> {
        let mut segments = Vec::new();

        // Handle empty content
        if content.is_empty() {
            return segments;
        }

        // CRITICAL: Twitch sends emote positions as CHARACTER indices, not byte indices!
        // Rust strings are byte-indexed, so we need to convert.
        // Build a mapping from character index to byte index for safe slicing.
        let char_to_byte: Vec<usize> = content
            .char_indices()
            .map(|(byte_idx, _)| byte_idx)
            .collect();
        let char_count = char_to_byte.len();

        // Helper to safely convert char index to byte index
        let char_to_byte_idx = |char_idx: usize| -> Option<usize> {
            if char_idx < char_count {
                Some(char_to_byte[char_idx])
            } else if char_idx == char_count {
                // One past the last character = end of string
                Some(content.len())
            } else {
                None
            }
        };

        // First, split by Twitch native emotes
        let mut last_char_index = 0;
        let mut sorted_emotes = twitch_emotes.to_vec();
        sorted_emotes.sort_by_key(|e| e.start);

        for emote in &sorted_emotes {
            // Validate emote bounds (character indices)
            if emote.start >= char_count || emote.end >= char_count || emote.start > emote.end {
                error!(
                    "[IRC Chat] Skipping invalid emote position: start={}, end={}, char_count={}",
                    emote.start, emote.end, char_count
                );
                continue;
            }

            // Convert character indices to byte indices
            let Some(start_byte) = char_to_byte_idx(emote.start) else {
                continue;
            };
            let Some(end_byte_exclusive) = char_to_byte_idx(emote.end + 1) else {
                continue;
            };
            let Some(last_byte) = char_to_byte_idx(last_char_index) else {
                continue;
            };

            // Add text before emote
            if emote.start > last_char_index {
                let text = &content[last_byte..start_byte];
                if !text.is_empty() {
                    // Parse text for third-party emotes, emojis, and links
                    segments.extend(Self::parse_text_segment(text, ctx));
                }
            }

            // Add Twitch emote (check for 7TV override) - bounds already validated above
            let emote_name = &content[start_byte..end_byte_exclusive];

            // A Twitch chat GIF: the span is a bracketed description, not an
            // emote code, so it takes no 7TV override and no text parsing.
            if emote.gif {
                segments.push(MessageSegment::Gif {
                    content: emote_name.to_string(),
                    gif_id: emote.id.clone(),
                    gif_url: emote.url.clone(),
                });
                last_char_index = emote.end + 1;
                continue;
            }

            // Check if 7TV has an emote with the same name (7TV takes priority)
            let seventv_override = ctx.channel.and_then(|c| c.seventv_override(emote_name));

            if let Some(seventv_emote) = seventv_override {
                // Use 7TV version instead of Twitch
                segments.push(MessageSegment::Emote {
                    content: emote_name.to_string(),
                    emote_id: Some(seventv_emote.id.clone()),
                    emote_url: seventv_emote.url.clone(),
                    is_zero_width: seventv_emote.is_zero_width,
                    modifier_flags: None,
                    is_personal: None,
                });
            } else {
                // Use Twitch emote
                segments.push(MessageSegment::Emote {
                    content: emote_name.to_string(),
                    emote_id: Some(emote.id.clone()),
                    emote_url: emote.url.clone(),
                    is_zero_width: None,
                    modifier_flags: None,
                    is_personal: None,
                });
            }

            last_char_index = emote.end + 1;
        }

        // Add remaining text
        if last_char_index < char_count {
            if let Some(last_byte) = char_to_byte_idx(last_char_index) {
                let text = &content[last_byte..];
                if !text.is_empty() {
                    segments.extend(Self::parse_text_segment(text, ctx));
                }
            }
        }

        // If no segments were created, return the original content as text
        if segments.is_empty() {
            segments.push(MessageSegment::Text {
                content: content.to_string(),
            });
        }

        segments
    }

    /// Parse a text segment for third-party emotes, emojis, and links.
    /// `ctx` carries the channel's prebuilt name lookup, the sender's personal
    /// emotes, and the cheermote set - all snapshotted once per message.
    fn parse_text_segment(text: &str, ctx: &ParseCtx<'_>) -> Vec<MessageSegment> {
        let mut segments = Vec::new();


        // Split by spaces to check each word
        let words: Vec<&str> = text.split(' ').collect();

        for (i, word) in words.iter().enumerate() {
            // Empty words come from doubled/leading spaces in split(' ').
            // Pushing Text("") for them breaks the frontend's zero-width /
            // modifier look-back, which expects the segment before a spacer
            // to be the target emote. Emit only the spacer and move on.
            if word.is_empty() {
                if i < words.len() - 1 {
                    segments.push(MessageSegment::Text {
                        content: " ".to_string(),
                    });
                }
                continue;
            }

            // Check if word is a URL. Schemes, www. hosts and bare domains all
            // count; see services/link_detect.rs for why a bare domain needs a
            // TLD table rather than just a dot. Sentence punctuation trailing
            // the link becomes its own text run, so "see test.fr." does not
            // put the full stop inside the href.
            if let Some((link, trailing)) = link_detect::split_link(word) {
                segments.push(MessageSegment::Link {
                    content: link.to_string(),
                    url: link_detect::link_url(link),
                });
                if !trailing.is_empty() {
                    segments.push(MessageSegment::Text {
                        content: trailing.to_string(),
                    });
                }
            } else if let Some((prefix, bits, tier, color, cheermote_url)) =
                Self::parse_cheermote(word, ctx.cheermotes)
            {
                // Found a cheermote pattern (e.g., Cheer500, Party1000)
                segments.push(MessageSegment::Cheermote {
                    content: word.to_string(),
                    prefix,
                    bits,
                    tier,
                    color,
                    cheermote_url,
                });
            } else if let Some((emote, from_personal)) = {
                // Personal emotes win over channel emotes for this sender,
                // matching the official client (its per-user emote map is
                // consulted before the room's).
                let personal_hit = ctx.personal.and_then(|p| p.get(*word));
                personal_hit
                    .map(|e| (e, true))
                    .or_else(|| ctx.channel.and_then(|c| c.get(word)).map(|e| (e, false)))
            } {
                // Found a third-party emote (BTTV, FFZ, or 7TV, or the sender's
                // personal set).
                segments.push(MessageSegment::Emote {
                    content: word.to_string(),
                    emote_id: Some(emote.id.clone()),
                    emote_url: emote.url.clone(),
                    is_zero_width: emote.is_zero_width,
                    modifier_flags: emote.modifier_flags,
                    is_personal: from_personal.then_some(true),
                });
            } else {
                // Convert emoji shortcodes first
                let converted = emoji_service::convert_emoji_shortcodes(word);

                // Parse for Unicode emojis (both converted shortcodes and direct emoji input)
                // This will emit Emoji segments with Apple CDN URLs for iOS-style rendering
                let emoji_segments = emoji_service::parse_emoji_segments(&converted);

                if emoji_segments.is_empty() {
                    // No content (shouldn't happen, but safety)
                    segments.push(MessageSegment::Text {
                        content: word.to_string(),
                    });
                } else {
                    // Add all parsed segments (text and emoji)
                    segments.extend(emoji_segments);
                }
            }

            // Add space between words (except after last word)
            if i < words.len() - 1 {
                segments.push(MessageSegment::Text {
                    content: " ".to_string(),
                });
            }
        }

        segments
    }

    /// Parses a cheermote word of the form `<prefix><bits>`, e.g. `Cheer500`.
    ///
    /// Prefixes are alphanumeric and can nest, so matching takes the longest
    /// known prefix whose remainder is all digits rather than splitting on the
    /// first digit. `cheerwhal` extends `cheer`, and a short match would leave
    /// `whal100` as the amount.
    ///
    /// `channel_set` supplies the channel's own prefixes alongside the globals;
    /// without it only the static global list can match.
    /// Returns `Some((prefix, bits, tier, color, url))` when valid.
    fn parse_cheermote(
        word: &str,
        channel_set: Option<&CheermoteSet>,
    ) -> Option<(String, u32, String, String, String)> {
        // Cheap reject: a cheermote word ends in a digit and contains a letter.
        if !word.ends_with(|c: char| c.is_ascii_digit())
            || !word.chars().any(|c| c.is_ascii_alphabetic())
        {
            return None;
        }

        // Case-insensitive prefix matching
        let word_lower = word.to_lowercase();

        if let Some(set) = channel_set {
            let mut best: Option<(&str, &Vec<CheermoteTier>)> = None;
            for (prefix, tiers) in set {
                if !word_lower.starts_with(prefix.as_str()) {
                    continue;
                }
                let rest = &word_lower[prefix.len()..];
                if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                if best.is_none_or(|(b, _)| prefix.len() > b.len()) {
                    best = Some((prefix, tiers));
                }
            }
            let (prefix, tiers) = best?;
            let bits: u32 = word_lower[prefix.len()..].parse().ok()?;
            if bits == 0 {
                return None;
            }
            // Highest tier whose threshold the amount clears; below the lowest
            // threshold (shouldn't happen: globals start at 1) use the first.
            let tier = tiers
                .iter()
                .rev()
                .find(|t| t.min_bits <= bits)
                .or_else(|| tiers.first())?;
            return Some((
                prefix.to_string(),
                bits,
                tier.min_bits.to_string(),
                tier.color.clone(),
                tier.url.clone(),
            ));
        }

        // Fallback while the Helix fetch hasn't landed (or failed): Twitch's
        // global prefixes with the classic CDN art pattern. Channel customs
        // cannot match here — their art only exists in the Helix response.
        const CHEERMOTE_PREFIXES: &[&str] = &[
            "cheer",
            "cheerwhal",
            "corgo",
            "scoops",
            "uni",
            "showlove",
            "party",
            "seemsgood",
            "pride",
            "kappa",
            "frankerz",
            "heyguys",
            "dansgame",
            "elegiggle",
            "trihard",
            "kreygasm",
            "4head",
            "swiftrage",
            "notlikethis",
            "failfish",
            "vohiyo",
            "pjsalt",
            "mrdestructoid",
            "bday",
            "ripcheer",
            "shamrock",
            "biblethump",
            "doodlecheer",
            "streamlabs",
            "muxy",
            "bitboss",
            "anon",
        ];

        // Longest matching prefix whose remainder is all digits.
        let matched_prefix = CHEERMOTE_PREFIXES
            .iter()
            .filter(|&&prefix| {
                word_lower.len() > prefix.len()
                    && word_lower.starts_with(prefix)
                    && word_lower[prefix.len()..].chars().all(|c| c.is_ascii_digit())
            })
            .max_by_key(|prefix| prefix.len())?;

        let bits: u32 = word_lower[matched_prefix.len()..].parse().ok()?;

        // Must have at least 1 bit
        if bits == 0 {
            return None;
        }

        // Determine tier and color based on bits amount
        let (tier, color) = match bits {
            10000.. => ("10000", "#ff1f1f"),    // Red
            5000..=9999 => ("5000", "#0099fe"), // Blue
            1000..=4999 => ("1000", "#1db2a6"), // Teal
            100..=999 => ("100", "#9c3ee8"),    // Purple
            _ => ("1", "#979797"),              // Gray
        };

        // Construct the animated GIF URL using Twitch CDN pattern
        let cheermote_url = format!(
            "https://d3aqoihi2n8ty8.cloudfront.net/actions/{}/dark/animated/{}/2.gif",
            matched_prefix, tier
        );

        Some((
            matched_prefix.to_string(),
            bits,
            tier.to_string(),
            color.to_string(),
            cheermote_url,
        ))
    }

    fn parse_privmsg(raw: &str) -> Option<ChatMessage> {
        let tags = if raw.starts_with('@') {
            let tag_end = raw.find(' ')?;
            &raw[1..tag_end]
        } else {
            ""
        };

        let mut tag_map = HashMap::new();
        for tag in tags.split(';') {
            let mut parts = tag.splitn(2, '=');
            if let (Some(key), Some(val)) = (parts.next(), parts.next()) {
                tag_map.insert(key, val);
            }
        }

        // Parsing similar to frontend logic
        let username = tag_map
            .get("display-name")
            .map(|s| s.to_string())
            .or_else(|| {
                // extract from :user!user@...
                if let Some(idx) = raw.find(" PRIVMSG") {
                    let prefix = &raw[..idx];
                    if let Some(excl) = prefix.find('!') {
                        // find start of prefix (after tags space)
                        let start = raw.find(' ').map(|i| i + 1).unwrap_or(0);
                        if start < excl {
                            Some(prefix[start + 1..excl].to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "unknown".to_string());

        // Content - check for ACTION message (/me command)
        let mut is_action = false;
        let content = if let Some(idx) = raw.find("PRIVMSG") {
            let rest = &raw[idx..];
            let mut result_msg = "".to_string();

            // Support both standard IRC format " :" and optimized IVR format (space after channel)
            if let Some(colon) = rest.find(" :") {
                result_msg = rest[colon + 2..].trim_end().to_string();
            } else if let Some(space_idx) = rest.find(" #") {
                let after_hash = &rest[space_idx + 1..];
                if let Some(payload_start) = after_hash.find(' ') {
                    result_msg = after_hash[payload_start + 1..].trim_end().to_string();
                }
            }

            let mut msg = result_msg;
            // Check for ACTION wrapper: \x01ACTION message\x01
            // Minimum valid: "\x01ACTION X\x01" = 10 chars (8 for header + 1 content + 1 closing)
            if msg.len() >= 10 && msg.starts_with("\x01ACTION ") && msg.ends_with('\x01') {
                is_action = true;
                msg = msg[8..msg.len() - 1].to_string();
            }
            msg
        } else {
            "".to_string()
        };

        let id = tag_map
            .get("id")
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let tags_owned: HashMap<String, String> = tag_map
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let display_name = tag_map
            .get("display-name")
            .map(|s| s.to_string())
            .unwrap_or_else(|| username.clone());

        let user_id = tag_map
            .get("user-id")
            .map(|s| s.to_string())
            .unwrap_or_default();

        // An empty `color` tag means the chatter never picked one; fill the
        // deterministic default here so every surface agrees (see the module).
        let color = Some(default_name_color::resolve_name_color(
            tag_map.get("color").copied(),
            &user_id,
            &username,
        ));

        let timestamp = tag_map
            .get("tmi-sent-ts")
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
                    .to_string()
            });

        // Pre-format timestamps for frontend (THE ENDGAME - no date parsing in React)
        let (formatted_timestamp, formatted_timestamp_with_seconds) =
            Self::format_timestamp(&timestamp);

        // For shared chat messages, prefer source-badges over badges
        let badges_str = tag_map
            .get("source-badges")
            .or_else(|| tag_map.get("badges"))
            .unwrap_or(&"");

        let badges: Vec<Badge> = badges_str
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|b_str| {
                let mut p = b_str.split('/');
                Badge {
                    name: p.next().unwrap_or("").to_string(),
                    version: p.next().unwrap_or("").to_string(),
                    image_url_1x: None,
                    image_url_2x: None,
                    image_url_4x: None,
                    title: None,
                    description: None,
                }
            })
            .collect();

        // Use EmotePos struct
        // emotes format: 25:0-4,12-16/1902:6-10 ...
        let emotes_str = tag_map.get("emotes").unwrap_or(&"");
        let mut emotes = Vec::new();
        if !emotes_str.is_empty() {
            for emote_group in emotes_str.split('/') {
                let mut parts = emote_group.split(':');
                if let (Some(id), Some(ranges)) = (parts.next(), parts.next()) {
                    for range in ranges.split(',') {
                        let mut bounds = range.split('-');
                        if let (Some(start_s), Some(end_s)) = (bounds.next(), bounds.next()) {
                            if let (Ok(start), Ok(end)) =
                                (start_s.parse::<usize>(), end_s.parse::<usize>())
                            {
                                // url: https://static-cdn.jtvnw.net/emoticons/v2/{id}/default/dark/3.0
                                let url = format!(
                                    "https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/3.0",
                                    id
                                );
                                emotes.push(EmotePos {
                                    id: id.to_string(),
                                    start,
                                    end,
                                    url,
                                    gif: false,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Twitch chat GIFs: the text carries a bracketed description at the
        // GIF's span and the `gifs` tag carries its id and URL with the same
        // zero-based inclusive codepoint positions `emotes` uses, so they join
        // the same position list and get the same reply-mention offset below.
        emotes.extend(parse_gifs_tag(tag_map.get("gifs").unwrap_or(&"")));

        // Parse reply info FIRST (needed to strip @mention before segment parsing)
        let reply_parent_user_login = tag_map
            .get("reply-parent-user-login")
            .map(|s| s.to_string());
        let reply_info = tag_map
            .get("reply-parent-msg-id")
            .map(|parent_id| ReplyInfo {
                parent_msg_id: parent_id.to_string(),
                parent_display_name: tag_map
                    .get("reply-parent-display-name")
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                parent_msg_body: tag_map
                    .get("reply-parent-msg-body")
                    .map(|s| s.replace("\\s", " "))
                    .unwrap_or_default(),
                parent_user_id: tag_map
                    .get("reply-parent-user-id")
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                parent_user_login: reply_parent_user_login.clone().unwrap_or_default(),
            });

        // Strip redundant @mention from reply messages BEFORE parsing segments.
        // The UI shows reply context, so the leading @username is redundant.
        // Twitch's composer inserts the DISPLAY name, not the login; matching
        // login alone silently no-oped for localized display names and left
        // the mention doubled, so match either, case-insensitively. The
        // char-walker replaced a per-reply Regex::new compile; login is tried
        // before display name, matching the old alternation order.
        let reply_parent_display_name = tag_map
            .get("reply-parent-display-name")
            .map(|s| s.to_string());
        let (content_for_segments, stripped_codepoints) = {
            let mut alts: Vec<&str> = Vec::new();
            if let Some(ref login) = reply_parent_user_login {
                if !login.is_empty() {
                    alts.push(login);
                }
            }
            if let Some(ref disp) = reply_parent_display_name {
                if !disp.is_empty() {
                    alts.push(disp);
                }
            }
            if alts.is_empty() {
                (content.clone(), 0usize)
            } else {
                let mention_end = Self::reply_mention_end(&content, &alts);
                let rest = &content[mention_end..];
                // Trim parity with the old regex path: whitespace is trimmed
                // whether or not a mention matched (replies only).
                let rest_no_lead = rest.trim_start();
                // Emote positions are CODEPOINT indices (see char_to_byte in
                // parse_message_segments), so the offset must count codepoints
                // consumed from the FRONT - the old byte-length delta mis-shifted
                // emotes whenever a localized display name was stripped, and
                // wrongly counted trailing trim too.
                let front_bytes = content.len() - rest_no_lead.len();
                let front_codepoints = content[..front_bytes].chars().count();
                (rest_no_lead.trim_end().to_string(), front_codepoints)
            }
        };

        // Also update emote positions if we stripped the @mention
        let emotes_adjusted = if stripped_codepoints > 0 {
            let offset = stripped_codepoints;
            emotes
                .into_iter()
                .filter_map(|mut e| {
                    // Skip emotes that were in the stripped portion
                    if e.start < offset {
                        return None;
                    }
                    e.start -= offset;
                    e.end -= offset;
                    Some(e)
                })
                .collect()
        } else {
            emotes
        };

        // Extract the channel this PRIVMSG was sent to so segment parsing uses
        // the right per-channel emote set. Falls back to empty string if the
        // line is malformed (third-party emotes simply won't match).
        let privmsg_channel = extract_channel_from_irc_line(raw).unwrap_or_default();

        // Parse message content into segments (using stripped content).
        // Snapshots gathered once; parsing is fully synchronous.
        let snapshots = Self::gather_parse_snapshots(&privmsg_channel, &user_id);
        let segments =
            Self::parse_message_segments(&content_for_segments, &emotes_adjusted, &snapshots.ctx());

        // Shared chat detection
        let source_room_id = tag_map.get("source-room-id").map(|s| s.to_string());
        let room_id = tag_map.get("room-id").map(|s| s.to_string());
        let is_from_shared_chat = source_room_id.is_some()
            && room_id.is_some()
            && source_room_id.as_ref() != room_id.as_ref();

        // First message detection
        let is_first_message = tag_map.get("first-msg").is_some_and(|v| *v == "1");

        // Bits amount for cheer messages
        let bits_amount = tag_map
            .get("bits")
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&b| b > 0);

        // Message type
        let msg_type = tag_map.get("msg-id").map(|s| s.to_string());

        // System message (for subscriptions, etc.)
        let system_message = tag_map.get("system-msg").map(|s| s.replace("\\s", " "));

        // Build metadata (THE ENDGAME - all computation done here)
        let metadata = MessageMetadata {
            is_action,
            is_mentioned: false, // Set by frontend based on current user context
            is_first_message,
            formatted_timestamp,
            formatted_timestamp_with_seconds,
            reply_info,
            source_room_id,
            is_from_shared_chat,
            msg_type,
            bits_amount,
            system_message,
            ..Default::default()
        };

        // Extract channel
        let channel = if let Some(idx) = raw.find(" PRIVMSG #") {
            let rest = &raw[idx + 10..];
            if let Some(space_idx) = rest.find(' ') {
                rest[..space_idx].to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        Some(ChatMessage {
            id,
            user_id,
            username,
            display_name,
            color,
            badges,
            timestamp,
            content: content_for_segments,
            provider: "twitch".to_string(),
            channel,
            emotes: emotes_adjusted,
            tags: tags_owned,
            layout: LayoutResult {
                height: 0.0,
                width: 0.0,
                has_reply: metadata.reply_info.is_some(),
                is_first_message: metadata.is_first_message,
            },
            segments,
            metadata,
        })
    }

    /// Parse USERNOTICE messages (subscriptions, resubs, gift subs, etc.)
    /// These have a different format than PRIVMSG but contain similar data
    fn parse_usernotice(raw: &str) -> Option<ChatMessage> {
        let tags = if raw.starts_with('@') {
            let tag_end = raw.find(' ')?;
            &raw[1..tag_end]
        } else {
            ""
        };

        let mut tag_map = HashMap::new();
        for tag in tags.split(';') {
            let mut parts = tag.splitn(2, '=');
            if let (Some(key), Some(val)) = (parts.next(), parts.next()) {
                tag_map.insert(key, val);
            }
        }

        // Extract username from login tag or display-name
        let username = tag_map
            .get("login")
            .or_else(|| tag_map.get("display-name"))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        // Content - extract optional user message after USERNOTICE
        // Format: :tmi.twitch.tv USERNOTICE #channel :optional message
        let content = if let Some(idx) = raw.find("USERNOTICE") {
            let rest = &raw[idx..];
            // Support both standard IRC format " :" and optimized IVR format (space after channel)
            if let Some(colon) = rest.find(" :") {
                rest[colon + 2..].trim_end().to_string()
            } else if let Some(space_idx) = rest.find(" #") {
                let after_hash = &rest[space_idx + 1..];
                if let Some(payload_start) = after_hash.find(' ') {
                    after_hash[payload_start + 1..].trim_end().to_string()
                } else {
                    "".to_string()
                }
            } else {
                "".to_string()
            }
        } else {
            "".to_string()
        };

        let id = tag_map
            .get("id")
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let tags_owned: HashMap<String, String> = tag_map
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        let display_name = tag_map
            .get("display-name")
            .map(|s| s.to_string())
            .unwrap_or_else(|| username.clone());

        let user_id = tag_map
            .get("user-id")
            .map(|s| s.to_string())
            .unwrap_or_default();

        // An empty `color` tag means the chatter never picked one; fill the
        // deterministic default here so every surface agrees (see the module).
        let color = Some(default_name_color::resolve_name_color(
            tag_map.get("color").copied(),
            &user_id,
            &username,
        ));

        let timestamp = tag_map
            .get("tmi-sent-ts")
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis()
                    .to_string()
            });

        // Pre-format timestamps
        let (formatted_timestamp, formatted_timestamp_with_seconds) =
            Self::format_timestamp(&timestamp);

        // For shared chat messages, prefer source-badges over badges
        let badges_str = tag_map
            .get("source-badges")
            .or_else(|| tag_map.get("badges"))
            .unwrap_or(&"");

        let badges: Vec<Badge> = badges_str
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|b_str| {
                let mut p = b_str.split('/');
                Badge {
                    name: p.next().unwrap_or("").to_string(),
                    version: p.next().unwrap_or("").to_string(),
                    image_url_1x: None,
                    image_url_2x: None,
                    image_url_4x: None,
                    title: None,
                    description: None,
                }
            })
            .collect();

        // Parse emotes from user's message content (if any)
        let emotes_str = tag_map.get("emotes").unwrap_or(&"");
        let mut emotes = Vec::new();
        if !emotes_str.is_empty() && !content.is_empty() {
            for emote_group in emotes_str.split('/') {
                let mut parts = emote_group.split(':');
                if let (Some(id), Some(ranges)) = (parts.next(), parts.next()) {
                    for range in ranges.split(',') {
                        let mut bounds = range.split('-');
                        if let (Some(start_s), Some(end_s)) = (bounds.next(), bounds.next()) {
                            if let (Ok(start), Ok(end)) =
                                (start_s.parse::<usize>(), end_s.parse::<usize>())
                            {
                                let url = format!(
                                    "https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/3.0",
                                    id
                                );
                                emotes.push(EmotePos {
                                    id: id.to_string(),
                                    start,
                                    end,
                                    url,
                                    gif: false,
                                });
                            }
                        }
                    }
                }
            }
        }

        // Twitch chat GIFs ride the same position list as emotes (see the
        // PRIVMSG path for the tag shape).
        if !content.is_empty() {
            emotes.extend(parse_gifs_tag(tag_map.get("gifs").unwrap_or(&"")));
        }

        // Extract channel from the USERNOTICE line so segment parsing uses the
        // correct per-channel emote set.
        let usernotice_channel = extract_channel_from_irc_line(raw).unwrap_or_default();

        // Parse message content into segments (if there's user content)
        let segments = if !content.is_empty() {
            let snapshots = Self::gather_parse_snapshots(&usernotice_channel, &user_id);
            Self::parse_message_segments(&content, &emotes, &snapshots.ctx())
        } else {
            Vec::new()
        };

        // Shared chat detection
        let source_room_id = tag_map.get("source-room-id").map(|s| s.to_string());
        let room_id = tag_map.get("room-id").map(|s| s.to_string());
        let is_from_shared_chat = source_room_id.is_some()
            && room_id.is_some()
            && source_room_id.as_ref() != room_id.as_ref();

        // Message type (sub, resub, subgift, submysterygift, etc.)
        // For shared chat, check source-msg-id first
        let msg_type = tag_map
            .get("source-msg-id")
            .or_else(|| tag_map.get("msg-id"))
            .map(|s| s.to_string());

        // System message (the auto-generated subscription message)
        let system_message = tag_map.get("system-msg").map(|s| s.replace("\\s", " "));

        // Build metadata
        let metadata = MessageMetadata {
            is_action: false,
            is_mentioned: false,
            is_first_message: false,
            formatted_timestamp,
            formatted_timestamp_with_seconds,
            reply_info: None,
            source_room_id,
            is_from_shared_chat,
            msg_type,
            bits_amount: None,
            system_message,
            ..Default::default()
        };

        // Extract channel
        let channel = if let Some(idx) = raw.find(" USERNOTICE #") {
            let rest = &raw[idx + 13..];
            if let Some(space_idx) = rest.find(' ') {
                rest[..space_idx].to_string()
            } else if let Some(colon_idx) = rest.find(" :") {
                rest[..colon_idx].to_string()
            } else {
                rest.trim().to_string()
            }
        } else {
            String::new()
        };

        Some(ChatMessage {
            id,
            user_id,
            username,
            display_name,
            color,
            badges,
            timestamp,
            content,
            provider: "twitch".to_string(),
            channel,
            emotes,
            tags: tags_owned,
            layout: LayoutResult {
                height: 0.0,
                width: 0.0,
                has_reply: false,
                is_first_message: false,
            },
            segments,
            metadata,
        })
    }

    /// Format timestamp for display - pre-computed in Rust (THE ENDGAME)
    /// Returns (formatted_without_seconds, formatted_with_seconds)
    fn format_timestamp(tmi_sent_ts: &str) -> (Option<String>, Option<String>) {
        if let Ok(ts_ms) = tmi_sent_ts.parse::<i64>() {
            use chrono::{Local, TimeZone};

            // Chat messages cluster within the same second, and both display
            // strings only have second resolution - memoize per second so a
            // busy channel pays the two chrono formats + timezone lookups once
            // per second instead of per message.
            static LAST: std::sync::Mutex<Option<(i64, String, String)>> =
                std::sync::Mutex::new(None);
            let ts_s = ts_ms.div_euclid(1000);
            let h24 = TIMESTAMP_24H.load(std::sync::atomic::Ordering::Relaxed);
            // The one-entry cache is keyed by the second AND the format, so a
            // settings flip never serves the other clock for the same second.
            let cache_key = if h24 { -ts_s - 1 } else { ts_s };
            if let Ok(guard) = LAST.lock() {
                if let Some((cached_s, ref without, ref with)) = *guard {
                    if cached_s == cache_key {
                        return (Some(without.clone()), Some(with.clone()));
                    }
                }
            }

            if let Some(datetime) = Local.timestamp_millis_opt(ts_ms).single() {
                let (fmt_short, fmt_long) = if h24 {
                    ("%H:%M", "%H:%M:%S")
                } else {
                    ("%l:%M %p", "%l:%M:%S %p")
                };
                let without_seconds = datetime.format(fmt_short).to_string().trim().to_string();
                let with_seconds = datetime.format(fmt_long).to_string().trim().to_string();

                if let Ok(mut guard) = LAST.lock() {
                    *guard = Some((cache_key, without_seconds.clone(), with_seconds.clone()));
                }
                return (Some(without_seconds), Some(with_seconds));
            }
        }
        (None, None)
    }

    /// Parse multiple IRC messages (historical messages from IVR API)
    /// Layout height is set to 0.0 - the browser handles all layout via CSS content-visibility
    pub async fn parse_historical_messages(raw_messages: Vec<String>) -> Vec<ChatMessage> {
        let mut results = Vec::with_capacity(raw_messages.len());
        let rules = ChatRules::snapshot();

        for raw in raw_messages {
            if let Some(mut chat_msg) = Self::parse_privmsg(&raw) {
                // Layout is handled by browser - just use placeholder values
                chat_msg.layout = LayoutResult {
                    height: 0.0,
                    width: 0.0,
                    has_reply: false,
                    is_first_message: false,
                };

                // Same rules as live rows: a hidden user's backfill is hidden
                // too, and highlights are stamped so the row needs no matcher.
                if ChatRules::evaluate(&mut chat_msg, &rules).drop {
                    continue;
                }
                chat_msg.metadata.from_backfill = true;
                results.push(chat_msg);
            }
        }

        results
    }

    pub async fn stop() -> Result<()> {
        debug!("[IRC Chat] Stopping chat service");

        // Stop IRC connection + its keepalive children (ping / heartbeat),
        // which aborting the parent alone would orphan.
        if let Some(handle) = get_irc_handle().lock().await.take() {
            handle.abort();
        }
        abort_keepalive_tasks().await;

        // Clear IRC writer
        *get_irc_writer().lock().await = None;

        // Stop WS server
        if let Some(handle) = get_ws_server_handle().lock().await.take() {
            handle.abort();
        }

        // Clear message queue
        get_message_queue().lock().await.clear();

        // Clear channels + their JOIN-ack state
        get_current_channels().lock().await.clear();
        tracker_clear().await;

        // Clear shared chat rooms
        get_shared_chat_rooms().lock().await.clear();
        SHARED_CHAT_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);

        // Clear all per-channel caches
        get_channel_emotes().lock().await.clear();
        clear_parse_lookups();
        if let Ok(mut g) = get_channel_cheermotes().write() { g.clear(); }
        get_user_badges_cache().lock().await.clear();
        get_room_state_cache().lock().await.clear();
        get_pending_messages().lock().await.clear();
        get_channel_consumers().lock().await.clear();

        // Drop all 7TV EventAPI subscriptions so the idle socket stops
        // receiving updates for channels nobody is viewing anymore.
        crate::services::seventv_eventapi::clear_all().await;
        // Same for the moderator-view subscriptions.
        crate::services::eventsub_moderation::clear_all().await;

        // Drop the WS port marker so the next start_chat does a full cold
        // bring-up rather than thinking a stale port is still serving.
        *get_ws_port().lock().await = None;

        debug!("[IRC Chat] Chat service stopped");

        Ok(())
    }

    /// Public form of `stop_irc_only`, for callers outside this module that have
    /// already established a provider is holding the bridge (see
    /// `ChatService::stop`).
    pub async fn stop_twitch_only() {
        Self::stop_irc_only().await;
    }

    /// Clear Twitch IRC state (connection, writer, per-channel caches, consumer
    /// claims) WITHOUT tearing down the shared local-WS bridge. Used when a
    /// non-Twitch provider is keeping the bridge alive and we only need to
    /// (re)start the Twitch IRC connection on top of it.
    async fn stop_irc_only() {
        if let Some(handle) = get_irc_handle().lock().await.take() {
            handle.abort();
        }
        abort_keepalive_tasks().await;
        *get_irc_writer().lock().await = None;
        get_current_channels().lock().await.clear();
        tracker_clear().await;
        get_shared_chat_rooms().lock().await.clear();
        SHARED_CHAT_ACTIVE.store(false, std::sync::atomic::Ordering::Relaxed);
        get_channel_emotes().lock().await.clear();
        clear_parse_lookups();
        if let Ok(mut g) = get_channel_cheermotes().write() { g.clear(); }
        get_user_badges_cache().lock().await.clear();
        get_room_state_cache().lock().await.clear();
        get_pending_messages().lock().await.clear();
        get_channel_consumers().lock().await.clear();
        crate::services::seventv_eventapi::clear_all().await;
        crate::services::eventsub_moderation::clear_all().await;
    }

    /// Bring up (or reuse) the local WebSocket bridge that streams parsed chat
    /// frames to the frontend. Idempotent: returns the existing port if a bridge
    /// is already serving, otherwise creates the broadcast channel + warp server.
    /// Non-Twitch providers call this so they can publish onto the same bus the
    /// frontend already listens to, with or without a Twitch chat open.
    pub async fn ensure_local_ws_bridge() -> Result<u16> {
        // Serialize bring-up: on boot MultiChat restores N channels that all hit this
        // at once. Without this lock each ran its own bring-up — separate warp servers
        // + broadcasters overwriting each other — so the frontend ended up on a socket
        // whose broadcaster never received publishes (no messages) or whose random-port
        // bind lost a collision (connection refused). With the lock, the first caller
        // brings the bridge up and the rest reuse it.
        let _guard = get_bridge_bringup_lock().lock().await;

        {
            // is_finished: a bridge task that died (e.g. bind panic) must not
            // be reported alive forever, or every start_chat would return a
            // port nothing serves. Clear the stale pieces so this call falls
            // through to a fresh bring-up.
            let ws_alive = {
                let mut handle = get_ws_server_handle().lock().await;
                match handle.as_ref() {
                    Some(h) if !h.is_finished() => true,
                    Some(_) => {
                        record_lifecycle("WS bridge task is dead; rebuilding bridge");
                        handle.take();
                        false
                    }
                    None => false,
                }
            };
            if !ws_alive {
                *get_message_broadcaster().lock().await = None;
                *get_ws_port().lock().await = None;
            }
            let has_tx = get_message_broadcaster().lock().await.is_some();
            let port = *get_ws_port().lock().await;
            if ws_alive && has_tx {
                if let Some(p) = port {
                    return Ok(p);
                }
            }
        }

        // Fresh bring-up: broadcast channel + local warp WS server.
        let (tx, _rx) = broadcast::channel::<String>(1000);
        let tx = Arc::new(tx);

        let tx_for_warp = tx.clone();
        let local_ws = warp::ws().map(move |ws: warp::ws::Ws| {
            let tx_clone = tx_for_warp.clone();
            ws.on_upgrade(move |socket| Self::handle_local_ws(socket, tx_clone))
        });
        // Pick a free port via an ephemeral probe bind, then hand it to warp. (warp 0.4
        // exposes no ephemeral bind that returns the port, and a fixed random port in a
        // small range can collide; an OS-allocated port + the bring-up lock above make a
        // collision negligible.) The old random-port path could return a port whose
        // server silently failed to bind, leaving callers to hit "refused".
        let probe = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| anyhow::anyhow!("WS bridge port probe failed: {}", e))?;
        let port = probe
            .local_addr()
            .map_err(|e| anyhow::anyhow!("WS bridge local_addr failed: {}", e))?
            .port();
        drop(probe);
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let handle = tokio::spawn(async move {
            warp::serve(local_ws).run(addr).await;
        });
        // Let warp re-bind the freed port before callers connect (frontend also retries).
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        // Only now (bind succeeded) publish the live socket so providers/the frontend
        // attach to a server that is actually listening.
        *get_message_broadcaster().lock().await = Some(tx);
        *get_ws_server_handle().lock().await = Some(handle);
        *get_ws_port().lock().await = Some(port);
        // Field-proof line: a rebuild under a live IRC session used to orphan
        // the session's captured sender; sends now resolve the broadcaster per
        // call, and this records that the swap happened.
        if matches!(get_irc_handle().lock().await.as_ref(), Some(h) if !h.is_finished()) {
            record_lifecycle("WS bridge rebuilt while IRC session live; broadcaster swapped");
        }
        Ok(port)
    }

    /// The shared broadcast sender for the local WS bridge, if it is up. Provider
    /// adapters serialize a `ChatMessage`/`ActivityEvent` frame and `send` it here
    /// to reach the frontend over the same socket the Twitch path uses.
    pub async fn broadcaster() -> Option<Arc<broadcast::Sender<String>>> {
        get_message_broadcaster().lock().await.clone()
    }

    /// Dev-only failure lever: close the live IRC connection (FIN on TCP, Close
    /// frame on WebSocket) so the full drop-reconnect-rejoin-backfill path can
    /// be exercised on demand.
    pub async fn debug_shutdown_socket() -> Result<()> {
        let writer = get_irc_writer()
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no IRC connection"))?;
        writer.lock().await.shutdown().await?;
        Ok(())
    }

    /// Dev-only failure lever: raw PART with NO bookkeeping — the channel stays
    /// desired and confirmed while the server drops our membership, exactly
    /// simulating a silently lost JOIN so the recovery paths (refresh probe,
    /// frontend nudge ladder) can be exercised on demand.
    pub async fn debug_send_part(channel: &str) -> Result<()> {
        let key = channel.to_lowercase();
        let writer = get_irc_writer()
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no IRC connection"))?;
        let mut w = writer.lock().await;
        w.send_line(&format!("PART #{}\r\n", key)).await?;
        record_lifecycle(&format!("debug: raw PART #{} sent (state untouched)", key));
        Ok(())
    }

    /// Frontend stale-watchdog stage 1: for every desired channel not already
    /// awaiting a JOIN ack, unconfirm it and rewrite its JOIN. A healthy
    /// channel re-acks (JOIN echo + ROOMSTATE) — which both re-confirms it and
    /// puts a frame on the bridge, resetting the frontend's stale timer — while
    /// a lost one stays unconfirmed so the refresh probe / stage-2 escalation
    /// can act. Nudge entries never retry and never drop the session, so a
    /// quiet-but-healthy channel costs one JOIN line per stale window. Capped
    /// at JOIN_BURST_BUDGET per call to stay clear of Twitch's JOIN rate wall;
    /// the next stale window nudges the rest.
    pub async fn nudge_channels() -> Result<usize> {
        let channels: Vec<String> = get_current_channels()
            .lock()
            .await
            .iter()
            .cloned()
            .collect();
        let writer = get_irc_writer()
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("no IRC connection"))?;
        let mut nudged = 0usize;
        for key in channels {
            if nudged >= JOIN_BURST_BUDGET {
                break;
            }
            {
                let mut t = get_join_tracker().lock().await;
                let (_, pending) = t.is_settled(&key);
                if pending {
                    continue;
                }
                t.unconfirm(&key);
                t.record_sent(&key, mono_ms(), 0, true);
                refresh_join_hint(&t);
            }
            let mut w = writer.lock().await;
            w.send_line(&format!("JOIN #{}\r\n", key)).await?;
            nudged += 1;
        }
        record_lifecycle(&format!(
            "nudged {} channel(s) after frontend stale report",
            nudged
        ));
        Ok(nudged)
    }
}


/// Entry counts of the per-channel caches, for the `[Resource]` line. Every
/// lock is a try-lock: the line is diagnostics and must never wait behind
/// the chat hot path. `None` means "contended this tick".
pub fn cache_counts() -> Vec<(&'static str, Option<usize>)> {
    let tl = |m: &Mutex<HashSet<String>>| m.try_lock().ok().map(|g| g.len());
    let tm = |m: &Mutex<HashMap<String, String>>| m.try_lock().ok().map(|g| g.len());
    vec![
        ("irc_channels", tl(get_current_channels())),
        ("channel_emote_sets", get_channel_emotes().try_lock().ok().map(|g| g.len())),
        ("personal_emote_users", get_personal_emotes().try_read().ok().map(|g| g.len())),
        ("user_badge_strings", tm(get_user_badges_cache())),
        ("user_colors", tm(get_user_color_cache())),
        ("channel_consumers", get_channel_consumers().try_lock().ok().map(|g| g.len())),
    ]
}

#[cfg(test)]
mod tests {
    // Verbatim `gifs` tag value from Twitch's IRC tags reference (2026-07-17).
    const TWITCH_GIFS_TAG: &str = "0-33|joSNxeswxuc74Juo8X|https://media4.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif?cid=095d7a5dzizsiwgabonagkmigggv8v1spfai91ac3x0dsiy0&ep=v1_gifs_trending&rid=giphy.gif&ct=g";

    #[test]
    fn gifs_tag_parses_the_documented_example_verbatim() {
        let pos = parse_gifs_tag(TWITCH_GIFS_TAG);
        assert_eq!(pos.len(), 1);
        assert!(pos[0].gif);
        assert_eq!(pos[0].id, "joSNxeswxuc74Juo8X");
        assert_eq!((pos[0].start, pos[0].end), (0, 33));
        // Every query parameter survives: Twitch says the URL must not be modified.
        assert_eq!(pos[0].url, &TWITCH_GIFS_TAG[24..]);
    }

    #[test]
    fn gifs_tag_skips_malformed_entries_and_unescapes_ircv3() {
        let pos = parse_gifs_tag("bad,5-9|abc|https://x/y.gif?a=1\\:b\\sc,3|x|y");
        assert_eq!(pos.len(), 1);
        assert_eq!((pos[0].start, pos[0].end), (5, 9));
        assert_eq!(pos[0].url, "https://x/y.gif?a=1;b c");
        assert!(parse_gifs_tag("").is_empty());
    }

    #[test]
    fn gif_position_becomes_a_gif_segment_and_the_placeholder_never_renders_as_text() {
        let content = "[Y A Y Yes GIF by Djemilah Birnie] nice";
        let pos = parse_gifs_tag(TWITCH_GIFS_TAG);
        let ctx = ParseCtx {
            channel: None,
            personal: None,
            cheermotes: None,
        };
        let segments = IrcService::parse_message_segments(content, &pos, &ctx);
        match &segments[0] {
            MessageSegment::Gif {
                content,
                gif_id,
                gif_url,
            } => {
                assert_eq!(content, "[Y A Y Yes GIF by Djemilah Birnie]");
                assert_eq!(gif_id, "joSNxeswxuc74Juo8X");
                assert!(gif_url.starts_with(
                    "https://media4.giphy.com/media/joSNxeswxuc74Juo8X/giphy.gif?cid="
                ));
            }
            other => panic!("expected a gif segment first, got {other:?}"),
        }
        // The trailing text survives as text and never contains the placeholder.
        let rest: String = segments[1..]
            .iter()
            .map(|s| match s {
                MessageSegment::Text { content } => content.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(rest.trim(), "nice");
    }

    use super::*;

    #[test]
    fn outage_frame_picks_lost_vs_pre() {
        assert_eq!(outage_frame(false), "IRC_CONNECT_RETRY");
        assert_eq!(outage_frame(true), "IRC_RECONNECTING");
    }

    #[test]
    fn reconnect_delay_backs_off_and_caps() {
        let secs: Vec<u64> = (1..=8).map(|n| reconnect_delay(n, false).as_secs()).collect();
        assert_eq!(secs, vec![2, 4, 8, 16, 32, 60, 60, 60]);
        assert_eq!(reconnect_delay(0, false).as_secs(), 2);
        assert_eq!(reconnect_delay(1, true).as_secs(), 300);
        assert_eq!(reconnect_delay(9, true).as_secs(), 300);
    }

    #[test]
    fn join_tracker_confirms_and_clears_pending() {
        let mut t = JoinTracker::default();
        t.record_sent("xqc", 1_000, 0, false);
        assert_eq!(t.is_settled("xqc"), (false, true));
        assert!(t.confirm("xqc"));
        assert_eq!(t.is_settled("xqc"), (true, false));
        // Re-confirming is not a "first confirm" again.
        assert!(!t.confirm("xqc"));
        assert!(t.due(u64::MAX).is_empty());
    }

    #[test]
    fn join_tracker_reissues_after_deadline_and_exhausts() {
        let mut t = JoinTracker::default();
        t.record_sent("xqc", 1_000, 0, false);
        // Before the deadline: not due.
        assert!(t.due(1_000 + JOIN_CONFIRM_TIMEOUT_MS - 1).is_empty());
        // Past it: due with 1 attempt so far.
        let due = t.due(1_000 + JOIN_CONFIRM_TIMEOUT_MS);
        assert_eq!(due, vec![("xqc".to_string(), 1)]);
        // Re-issues bump attempts toward exhaustion.
        t.record_sent("xqc", 20_000, 0, false);
        t.record_sent("xqc", 40_000, 0, false);
        let due = t.due(40_000 + JOIN_CONFIRM_TIMEOUT_MS);
        assert_eq!(due, vec![("xqc".to_string(), 3)]);
        assert!(due[0].1 >= JOIN_MAX_ATTEMPTS);
    }

    #[test]
    fn join_tracker_pace_slots_defer_deadlines() {
        let mut t = JoinTracker::default();
        let pace = JOIN_PACE_INTERVAL.as_millis() as u64;
        t.record_sent("burst", 0, 0, false);
        t.record_sent("batch1", 0, pace, false);
        t.record_sent("batch2", 0, 2 * pace, false);
        // Only the burst channel is due after one timeout window.
        let due = t.due(JOIN_CONFIRM_TIMEOUT_MS);
        assert_eq!(due, vec![("burst".to_string(), 1)]);
        // batch1 becomes due only after its slot plus the window.
        let mut due: Vec<String> = t
            .due(pace + JOIN_CONFIRM_TIMEOUT_MS)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        due.sort();
        assert_eq!(due, vec!["batch1".to_string(), "burst".to_string()]);
    }

    #[test]
    fn join_tracker_nudge_entries_never_escalate() {
        let mut t = JoinTracker::default();
        t.confirm("xqc");
        t.unconfirm("xqc");
        t.record_sent("xqc", 0, 0, true);
        // A nudge entry is pending (so refresh probes treat it as in flight)
        // but never becomes due, no matter how much time passes.
        assert_eq!(t.is_settled("xqc"), (false, true));
        assert!(t.due(u64::MAX).is_empty());
        // A late ack still resolves it.
        assert!(t.confirm("xqc"));
        assert_eq!(t.is_settled("xqc"), (true, false));
    }

    #[test]
    fn parse_join_channel_matches_join_frames_only() {
        assert_eq!(
            parse_join_channel(":nick!nick@nick.tmi.twitch.tv JOIN #xqc\r\n"),
            Some("xqc".to_string())
        );
        assert_eq!(
            parse_join_channel("@tag=1 :nick!nick@host JOIN #XQC\r\n"),
            Some("xqc".to_string())
        );
        assert_eq!(
            parse_join_channel(":other!other@host JOIN :#chan\r\n"),
            Some("chan".to_string())
        );
        // A PRIVMSG whose TEXT contains a JOIN must never match.
        assert_eq!(
            parse_join_channel(":nick!nick@host PRIVMSG #chan :please JOIN #other now\r\n"),
            None
        );
        assert_eq!(parse_join_channel(":nick!nick@host PART #chan\r\n"), None);
        assert_eq!(parse_join_channel("JOIN"), None);
    }

    #[test]
    fn server_reconnect_matches_command_token_only() {
        assert!(is_server_reconnect(":tmi.twitch.tv RECONNECT\r\n"));
        assert!(is_server_reconnect("RECONNECT"));
        assert!(is_server_reconnect("@x=y :tmi.twitch.tv RECONNECT\r\n"));
        assert!(!is_server_reconnect(
            ":nick!nick@nick.tmi.twitch.tv PRIVMSG #chan :please RECONNECT now\r\n"
        ));
        assert!(!is_server_reconnect(
            "@msg-id=slow_on :tmi.twitch.tv NOTICE #chan :We will RECONNECT shortly\r\n"
        ));
    }

    // Doubled/leading spaces must never emit empty Text segments: the
    // frontend's zero-width/modifier grouping looks back past exactly one
    // whitespace segment to find the target emote, and a Text("") in that
    // position orphans the modifier.
    #[test]
    fn no_empty_text_segments_from_extra_spaces() {
        let segs = IrcService::parse_text_segment("a  b", &ParseCtx::default());
        for s in &segs {
            if let MessageSegment::Text { content } = s {
                assert!(!content.is_empty(), "empty Text segment emitted");
            }
        }
        // "a  b" -> a, space, space, b
        assert_eq!(segs.len(), 4);
    }

    #[test]
    fn reply_mention_strip_boundaries() {
        // Exact name + space: mention and the whole whitespace run consumed.
        assert_eq!(IrcService::reply_mention_end("@foo  hi", &["foo"]), 6);
        // Name at end of message: everything consumed.
        assert_eq!(IrcService::reply_mention_end("@foo", &["foo"]), 4);
        // Boundary REQUIRED: a name prefixing a longer word must not strip.
        assert_eq!(IrcService::reply_mention_end("@foobarbaz hi", &["foobar"]), 0);
        // Case-insensitive match.
        assert_eq!(IrcService::reply_mention_end("@FoO hi", &["foo"]), 5);
        // Second alternative (display name) matches when login does not.
        assert_eq!(IrcService::reply_mention_end("@ふー hi", &["foo", "ふー"]), 8);
        // No leading @: untouched.
        assert_eq!(IrcService::reply_mention_end("foo hi", &["foo"]), 0);
    }

    #[test]
    fn emote_lookup_priority_and_override() {
        use crate::services::emote_service::EmoteProvider;
        let mk = |id: &str, name: &str, provider: EmoteProvider| Emote {
            id: id.to_string(),
            name: name.to_string(),
            url: format!("https://example.test/{id}.webp"),
            provider,
            is_zero_width: None,
            local_url: None,
            emote_type: None,
            owner_id: None,
            owner_name: None,
            width: None,
            modifier_flags: None,
            ffz_sub_only: None,
        };
        let set = EmoteSet {
            twitch: Vec::new(),
            bttv: vec![mk("b1", "Clash", EmoteProvider::BTTV), mk("b2", "BttvOnly", EmoteProvider::BTTV)],
            ffz: vec![mk("f1", "Clash", EmoteProvider::FFZ)],
            seven_tv: vec![mk("s1", "Clash", EmoteProvider::SevenTV)],
            kick: Vec::new(),
            seven_tv_ok: true,
        };
        let lookup = EmoteLookup::build(&set);
        // Word tier: 7TV wins name collisions (inserted last).
        assert_eq!(lookup.get("Clash").unwrap().id, "s1");
        // Override tier: only a 7TV winner overrides a Twitch-native emote.
        assert_eq!(lookup.seventv_override("Clash").unwrap().id, "s1");
        assert!(lookup.seventv_override("BttvOnly").is_none());
        assert!(lookup.seventv_override("Missing").is_none());
    }

    fn tiers(spec: &[(u32, &str)]) -> Vec<CheermoteTier> {
        spec.iter()
            .map(|(min_bits, color)| CheermoteTier {
                min_bits: *min_bits,
                color: color.to_string(),
                url: format!("https://example.test/{}.gif", min_bits),
            })
            .collect()
    }

    // Prefixes nest (`cheerwhal` extends `cheer`), so matching must take the
    // LONGEST prefix whose remainder is all digits. The old first-match walk
    // hit `cheer`, left `whal100`, failed the digit check and rendered a
    // Twitch GLOBAL cheermote as plain text.
    #[test]
    fn static_fallback_prefers_longest_prefix() {
        let (prefix, bits, ..) = IrcService::parse_cheermote("Cheerwhal100", None).unwrap();
        assert_eq!(prefix, "cheerwhal");
        assert_eq!(bits, 100);
    }

    // Digits sit anywhere in real prefixes: `4Head` starts with one.
    #[test]
    fn static_fallback_handles_digit_leading_prefix() {
        let (prefix, bits, ..) = IrcService::parse_cheermote("4Head500", None).unwrap();
        assert_eq!(prefix, "4head");
        assert_eq!(bits, 500);
    }

    // A channel's own custom prefix (from the Helix channel_custom entries)
    // must resolve with art and color taken from the fetched tiers, picking
    // the highest tier whose threshold the amount clears.
    #[test]
    fn channel_custom_prefix_resolves_with_fetched_tiers() {
        let mut set: CheermoteSet = HashMap::new();
        set.insert(
            "mathox1cheer".to_string(),
            tiers(&[(1, "#979797"), (100, "#9c3ee8"), (1000, "#1db2a6")]),
        );
        let (prefix, bits, tier, color, url) =
            IrcService::parse_cheermote("mathox1Cheer250", Some(&set)).unwrap();
        assert_eq!(prefix, "mathox1cheer");
        assert_eq!(bits, 250);
        assert_eq!(tier, "100");
        assert_eq!(color, "#9c3ee8");
        assert_eq!(url, "https://example.test/100.gif");
    }

    // Longest-match applies to the fetched map too, and non-cheer words with a
    // known-prefix start must stay text.
    #[test]
    fn channel_set_longest_match_and_rejects() {
        let mut set: CheermoteSet = HashMap::new();
        set.insert("cheer".to_string(), tiers(&[(1, "#979797")]));
        set.insert("cheerwhal".to_string(), tiers(&[(1, "#979797")]));
        let (prefix, ..) = IrcService::parse_cheermote("cheerwhal5", Some(&set)).unwrap();
        assert_eq!(prefix, "cheerwhal");
        assert!(IrcService::parse_cheermote("cheerleader", Some(&set)).is_none());
        assert!(IrcService::parse_cheermote("cheer0", Some(&set)).is_none());
        assert!(IrcService::parse_cheermote("cheer", Some(&set)).is_none());
        assert!(IrcService::parse_cheermote("100", Some(&set)).is_none());
    }

    // Raw Helix `bits/cheermotes` shape -> parse map: prefixes lowercase,
    // tiers ascending, 2x dark animated art preferred, static-only tiers kept
    // via fallback instead of dropped.
    #[test]
    fn helix_response_converts_to_parse_map() {
        let json: serde_json::Value = serde_json::from_str(
            r##"{"data":[
                {"prefix":"mathox1Cheer","tiers":[
                    {"min_bits":100,"id":"100","color":"#9c3ee8","can_cheer":true,
                     "images":{"dark":{"animated":{"1":"https://cdn.test/m/100/1.gif","2":"https://cdn.test/m/100/2.gif"},
                               "static":{"1":"https://cdn.test/m/100/1.png"}}}},
                    {"min_bits":1,"id":"1","color":"#979797","can_cheer":true,
                     "images":{"dark":{"static":{"1":"https://cdn.test/m/1/1.png","2":"https://cdn.test/m/1/2.png"}}}}
                ]},
                {"prefix":"NoArt","tiers":[
                    {"min_bits":1,"id":"1","color":"#979797","can_cheer":true,"images":{"dark":{}}}
                ]}
            ]}"##,
        )
        .unwrap();
        let set = IrcService::cheermote_set_from_helix(&json);
        let tiers = set.get("mathox1cheer").expect("prefix lowercased");
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].min_bits, 1, "tiers sorted ascending");
        assert_eq!(tiers[0].url, "https://cdn.test/m/1/2.png", "static fallback");
        assert_eq!(tiers[1].url, "https://cdn.test/m/100/2.gif", "2x animated");
        assert!(!set.contains_key("noart"), "art-less prefix dropped");
    }

    // LIVE sweep against Mathox's real channel (broadcaster 194431028) via the
    // deployed streamnook.app cheermotes endpoint — the same Helix data the new
    // Rust fetch pulls, already production-verified. Every real prefix in the
    // channel (globals + channel_custom) at several amounts must round-trip
    // through the real parse_cheermote, and the resolved tier must agree with
    // the endpoint's own tier thresholds. Network: run explicitly with
    // `cargo test live_mathox -- --ignored`.
    #[test]
    #[ignore]
    fn live_mathox_channel_cheermotes_resolve() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let body: serde_json::Value = rt.block_on(async {
            crate::services::http::client()
                .get("https://streamnook.app/api/twitch/cheermotes?room=194431028")
                .send()
                .await
                .expect("endpoint reachable")
                .json()
                .await
                .expect("json body")
        });
        let site = body
            .get("cheermotes")
            .and_then(|c| c.as_object())
            .expect("cheermotes map");
        assert!(!site.is_empty(), "endpoint returned no cheermotes");
        assert!(
            site.contains_key("mathox1cheer"),
            "Mathox's channel_custom prefix missing from live data"
        );

        // Site shape ({minBits,color,url}) -> the Rust parse map shape.
        let mut set: CheermoteSet = HashMap::new();
        for (prefix, tiers) in site {
            let parsed: Vec<CheermoteTier> = tiers
                .as_array()
                .unwrap()
                .iter()
                .map(|t| CheermoteTier {
                    min_bits: t["minBits"].as_u64().unwrap() as u32,
                    color: t["color"].as_str().unwrap().to_string(),
                    url: t["url"].as_str().unwrap().to_string(),
                })
                .collect();
            set.insert(prefix.clone(), parsed);
        }

        let mut words = 0;
        for (prefix, tiers) in &set {
            for amount in [1u32, 47, 100, 999, 1000, 5000, 10000, 25000] {
                // Real chatters type mixed case; build it like one would.
                let word = format!("{}{}", prefix.to_uppercase(), amount);
                let (got_prefix, got_bits, _tier, got_color, got_url) =
                    IrcService::parse_cheermote(&word, Some(&set))
                        .unwrap_or_else(|| panic!("{word} failed to resolve"));
                words += 1;
                // Longest-match may legitimately resolve to a LONGER nested
                // prefix (cheerwhal over cheer) but never to a shorter one.
                assert!(
                    got_prefix.len() >= prefix.len(),
                    "{word} resolved to shorter prefix {got_prefix}"
                );
                // Prefix + amount must reassemble the word exactly.
                assert_eq!(
                    format!("{got_prefix}{got_bits}"),
                    word.to_lowercase(),
                    "{word} split corrupted"
                );
                // Tier agreement with the endpoint's own thresholds.
                let expect = set[&got_prefix]
                    .iter()
                    .rev()
                    .find(|t| t.min_bits <= amount)
                    .unwrap_or(&set[&got_prefix][0]);
                assert_eq!(got_color, expect.color, "{word} wrong tier color");
                assert_eq!(got_url, expect.url, "{word} wrong tier art");
                assert!(
                    got_url.starts_with("https://"),
                    "{word} art not an absolute URL"
                );
            }
        }
        println!(
            "live sweep: {} prefixes x amounts = {} words, all resolved",
            set.len(),
            words
        );
    }
}

// ---------------------------------------------------------------------------
// Debounced dictionary write after a live 7TV delta.
//
// A bot adding ten emotes in a row is ten dispatches in a few seconds; writing
// the 1.5 MB dictionary after each would be ten writes for one outcome. Each
// delta bumps a per-channel generation and schedules a write 2 s out; only the
// task still holding the latest generation writes, from the set as it is THEN.
// Memory is already current, so a lost write costs only the disk-first seed of
// the next join, which that join's refresh corrects.
static DICTIONARY_WRITE_GEN: OnceLock<std::sync::Mutex<HashMap<String, u64>>> = OnceLock::new();

fn schedule_dictionary_write(key: String, user_id: String) {
    let gens = DICTIONARY_WRITE_GEN.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let my_gen = {
        let Ok(mut g) = gens.lock() else {
            return;
        };
        let e = g.entry(key.clone()).or_insert(0);
        *e += 1;
        *e
    };
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let current = gens.lock().ok().and_then(|g| g.get(&key).copied());
        if current != Some(my_gen) {
            return; // a newer delta rescheduled the write
        }
        let snapshot = get_channel_emotes().lock().await.get(&key).cloned();
        if let Some(set) = snapshot {
            crate::services::emote_set_cache::save_force(&user_id, &set);
        }
    });
}

// ---------------------------------------------------------------------------
// Deferred emote-refresh gate.
//
// Handing the chat socket back before the provider refresh finishes is what made
// chat open in ~150ms instead of seconds. It also removed the thing that had been
// bounding those fetches: the refresh used to run INSIDE `start`'s global start
// lock, so exactly one could ever be in flight. Spawned and ungated, a burst of
// channel switches (or one MultiChat window opening several tabs, or a reconnect
// re-joining every channel) starts an unbounded number of concurrent
// BTTV+FFZ+7TV fetches, each building and holding a multi-megabyte EmoteSet, and
// with no dedupe the SAME channel can have several running at once.
//
// That is the identical stampede the 7TV entitlement lane already had to fix.
// Two bounds, for the two different ways it grows: the in-flight set collapses
// duplicates per channel, and the semaphore caps how many distinct channels can
// be fetching at all. Four is well clear of any real grid or MultiChat layout
// while keeping the worst case a handful of buffers rather than dozens.
const EMOTE_REFRESH_CONCURRENCY: usize = 4;

static EMOTE_REFRESH_INFLIGHT: OnceLock<
    std::sync::Mutex<std::collections::HashSet<String>>,
> = OnceLock::new();
static EMOTE_REFRESH_PERMITS: OnceLock<std::sync::Arc<tokio::sync::Semaphore>> = OnceLock::new();

fn emote_refresh_permits() -> std::sync::Arc<tokio::sync::Semaphore> {
    EMOTE_REFRESH_PERMITS
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(EMOTE_REFRESH_CONCURRENCY)))
        .clone()
}

/// Claims the refresh slot for `channel`, or returns None if one is already
/// running. The returned guard releases the slot on drop, including on panic.
fn try_begin_emote_refresh(channel: &str) -> Option<EmoteRefreshGuard> {
    let set = EMOTE_REFRESH_INFLIGHT
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    let mut guard = set.lock().ok()?;
    if !guard.insert(channel.to_string()) {
        return None;
    }
    Some(EmoteRefreshGuard(channel.to_string()))
}

struct EmoteRefreshGuard(String);

impl Drop for EmoteRefreshGuard {
    fn drop(&mut self) {
        if let Some(set) = EMOTE_REFRESH_INFLIGHT.get() {
            if let Ok(mut guard) = set.lock() {
                guard.remove(&self.0);
            }
        }
    }
}
