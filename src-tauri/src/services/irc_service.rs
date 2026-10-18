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
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::OnceLock;
use tokio::sync::{broadcast, Mutex};
use crate::services::chat_bridge_route::{BridgeFrame, SocketRoute};
use warp::Filter;

pub struct IrcService;

static WS_SERVER_HANDLE: OnceLock<Mutex<Option<tokio::task::JoinHandle<()>>>> = OnceLock::new();
// Serializes bring-up of the local WS bridge so concurrent first-acquires (MultiChat
// restoring N channels at boot) don't each spin up a separate server + broadcaster,
// which leaves the frontend attached to a dead or message-less socket.
static BRIDGE_BRINGUP_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static CURRENT_CHANNELS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
static MESSAGE_BROADCASTER: OnceLock<Mutex<Option<Arc<broadcast::Sender<BridgeFrame>>>>> =
    OnceLock::new();
static MESSAGE_QUEUE: OnceLock<Mutex<VecDeque<BridgeFrame>>> = OnceLock::new();
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
/// How far back a newly arrived personal set repaints its owner's rows, and
/// how many of each channel's newest rows that looks through.
const PERSONAL_REPAINT_WINDOW_MS: i64 = 10 * 60 * 1000;
const PERSONAL_REPAINT_SCAN: usize = 300;

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
// Wall-clock twin of the last read, used only to spot suspension: across a
// sleep the wall clock jumps ahead of the monotonic one.
static LAST_IRC_READ_WALL_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// Ring of recent connection-lifecycle events, pullable from a packaged build
// (which has no stderr) via the get_chat_lifecycle_log command.
static LIFECYCLE_LOG: OnceLock<std::sync::Mutex<VecDeque<String>>> = OnceLock::new();
const LIFECYCLE_LOG_CAP: usize = 100;

// We PING only when the link has been quiet for IRC_PING_IDLE_MS (a busy chat
// proves itself), checked every IRC_PING_CHECK. The server answers, so a
// healthy link never goes IRC_READ_TIMEOUT without a completed read, and a
// network that silently cuts the connection costs about 30 s of chat, which
// the reconnect backfill then fills. Reference clients ping every 5s, so
// Twitch has ample tolerance for this cadence.
const IRC_PING_CHECK: std::time::Duration = std::time::Duration::from_secs(5);
const IRC_PING_IDLE_MS: u64 = 15_000;
const IRC_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const HANDSHAKE_STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
// Read timeout plus slack: past this the frontend must be allowed to see
// silence so its own watchdog can act.
const HEARTBEAT_SUPPRESS_AFTER_MS: u64 = 45_000;
// A session that ends with nothing read for this long went silent before it
// ended: at least one idle PING went unanswered. That is evidence against the
// transport (see irc_transport's stall rule), unless the machine slept.
const STALL_READ_AGE_MS: u64 = 20_000;
// How far the read clock may lag the timer or the wall clock before the gap is
// put down to suspension (laptop sleep, a phone freezing the app) instead.
const SUSPEND_SLACK_MS: u64 = 5_000;
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
// Every JOIN this process writes, whoever writes it (handshake burst, paced
// batches, user-driven ensure_joined, the unconfirmed re-issue), counts against
// one rolling window, a little inside Twitch's 20 per 10 s. Opening many chats
// at once (a restored chat dock) used to write every JOIN immediately.
const JOIN_WINDOW_MAX: usize = 18;
const JOIN_WINDOW_MS: u64 = 10_500;

/// The JOINs written in the last `JOIN_WINDOW_MS`, by send time.
#[derive(Default)]
struct JoinWindow {
    sent: std::collections::VecDeque<u64>,
}

impl JoinWindow {
    fn prune(&mut self, now: u64) {
        while self.sent.front().is_some_and(|&t| now.saturating_sub(t) >= JOIN_WINDOW_MS) {
            self.sent.pop_front();
        }
    }

    /// Claim a slot at `now`, or say how long until one frees.
    fn claim(&mut self, now: u64) -> Result<(), u64> {
        self.prune(now);
        if self.sent.len() < JOIN_WINDOW_MAX {
            self.sent.push_back(now);
            return Ok(());
        }
        let oldest = *self.sent.front().unwrap_or(&now);
        Err((oldest + JOIN_WINDOW_MS).saturating_sub(now).max(1))
    }

    /// Count JOINs written by a path that paces itself.
    fn note(&mut self, now: u64, n: usize) {
        self.prune(now);
        self.sent.extend(std::iter::repeat_n(now, n));
    }
}

static JOIN_WINDOW: once_cell::sync::Lazy<std::sync::Mutex<JoinWindow>> =
    once_cell::sync::Lazy::new(Default::default);

/// Wait for room in the JOIN window, then claim it.
async fn take_join_slot() {
    loop {
        let wait = match JOIN_WINDOW.lock() {
            Ok(mut w) => match w.claim(mono_ms()) {
                Ok(()) => return,
                Err(ms) => ms,
            },
            Err(_) => return,
        };
        tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
    }
}

fn note_joins_sent(n: usize) {
    if let Ok(mut w) = JOIN_WINDOW.lock() {
        w.note(mono_ms(), n);
    }
}
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
        // A reward that asks for text reaches chat with only its id.
        crate::services::channel_rewards::note_chat_joined(key);
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

fn wall_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mark_irc_read() {
    LAST_IRC_READ_ELAPSED_MS.store(mono_ms(), std::sync::atomic::Ordering::Relaxed);
    LAST_IRC_READ_WALL_MS.store(wall_ms(), std::sync::atomic::Ordering::Relaxed);
}

fn irc_read_age_ms() -> u64 {
    mono_ms().saturating_sub(LAST_IRC_READ_ELAPSED_MS.load(std::sync::atomic::Ordering::Relaxed))
}

fn irc_read_wall_age_ms() -> u64 {
    wall_ms().saturating_sub(LAST_IRC_READ_WALL_MS.load(std::sync::atomic::Ordering::Relaxed))
}

/// Did a signed-in session go silent before it ended, with the machine awake?
/// `read_age_ms` is the monotonic time since the last read, `wall_age_ms` the
/// wall-clock time. A timer can only see more silence than the read timeout if
/// it fired late, and the wall clock only runs ahead of the monotonic one
/// across a sleep: either way the gap is suspension, not the network.
fn is_silent_session_end(reason: &str, read_age_ms: u64, wall_age_ms: u64) -> bool {
    if matches!(reason, "server RECONNECT" | "handler stall") {
        return false;
    }
    if read_age_ms < STALL_READ_AGE_MS {
        return false;
    }
    let timeout_ms = IRC_READ_TIMEOUT.as_millis() as u64;
    if read_age_ms > timeout_ms + SUSPEND_SLACK_MS {
        return false;
    }
    wall_age_ms <= read_age_ms + SUSPEND_SLACK_MS
}

/// Development-only fault injection for the stall path:
/// `STREAMNOOK_IRC_FAULT=deafen:<secs>[:irc|websocket]` makes every session
/// (on that transport, or any) throw away everything it reads from `<secs>`
/// after sign-in, the way a network that blackholes a connection would.
/// Compiled out of release builds.
fn fault_deafen_after(transport: IrcTransport) -> Option<std::time::Duration> {
    #[cfg(debug_assertions)]
    {
        let spec = std::env::var("STREAMNOOK_IRC_FAULT").ok()?;
        parse_deafen_fault(&spec, transport)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = transport;
        None
    }
}

#[cfg(debug_assertions)]
fn parse_deafen_fault(spec: &str, transport: IrcTransport) -> Option<std::time::Duration> {
    let mut parts = spec.trim().split(':');
    if parts.next()? != "deafen" {
        return None;
    }
    let secs: u64 = parts.next()?.parse().ok()?;
    match parts.next() {
        None => {}
        Some(t) if t == transport.label() => {}
        Some(_) => return None,
    }
    Some(std::time::Duration::from_secs(secs))
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

/// How an established session ended.
struct SessionEnd {
    reason: &'static str,
    transport: IrcTransport,
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

/// Moves on at every full stop. Aborting the bridge's server task stops it
/// accepting, but each page socket already upgraded runs in a task of its own
/// with its own handle on the old broadcaster, so it stayed open with nothing
/// feeding it: the page saw a healthy socket, never reconnected, and its chat
/// went silent until a watchdog noticed. Each socket watches this instead and
/// closes when it moves past the value its server was built at.
static BRIDGE_CLOSE: OnceLock<tokio::sync::watch::Sender<u64>> = OnceLock::new();

fn bridge_close() -> &'static tokio::sync::watch::Sender<u64> {
    BRIDGE_CLOSE.get_or_init(|| tokio::sync::watch::channel(0).0)
}

/// The value a bridge server built now is born at.
fn bridge_generation() -> u64 {
    *bridge_close().borrow()
}

/// Closes every page socket the bridge serves now (code 1012, "service
/// restart"), so each page reconnects at once to whatever bridge comes next.
fn close_page_sockets() {
    bridge_close().send_modify(|g| *g = g.wrapping_add(1));
}

fn get_current_channels() -> &'static Mutex<HashSet<String>> {
    CURRENT_CHANNELS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn get_message_broadcaster() -> &'static Mutex<Option<Arc<broadcast::Sender<BridgeFrame>>>> {
    MESSAGE_BROADCASTER.get_or_init(|| Mutex::new(None))
}

fn get_message_queue() -> &'static Mutex<VecDeque<BridgeFrame>> {
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
async fn send_to_bridge(msg: impl Into<BridgeFrame>, queue_on_fail: bool) -> bool {
    let msg: BridgeFrame = msg.into();
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

/// Send `channel`'s room again, worded for the account as it stands now, when
/// something other than a ROOMSTATE changed what the modes mean for it (a new
/// role, a follow). Also refreshes the replay cache for late-mounting panes.
pub(crate) async fn reship_room_state(channel: &str) {
    let Some(fields) = crate::services::chat_send_limiter::room_state_fields(channel) else {
        return;
    };
    let mut frame = serde_json::Map::new();
    frame.insert("type".into(), serde_json::json!("ROOMSTATE"));
    frame.insert("channel".into(), serde_json::json!(channel));
    frame.extend(fields);
    let frame = serde_json::Value::Object(frame).to_string();
    get_room_state_cache().lock().await.insert(channel.to_string(), frame.clone());
    send_to_bridge(BridgeFrame::channel(channel, frame), false).await;
}

/// Send one chat-stream frame to `channel`'s rooms (a held AutoMod message,
/// say), held for the next attach when no room is listening.
pub(crate) async fn ship_channel_frame(channel: &str, frame: String) {
    send_to_bridge(BridgeFrame::channel(channel, frame), true).await;
}

/// Look up the account's own follow of a followers-only room (one Helix call),
/// word the room again with it, and keep a "you can chat in N minutes" line
/// current until the wait is over.
async fn learn_own_follow(channel: String, room_id: String) {
    use crate::services::follow_lookup::{own_follow, FollowAnswer};
    let answer = match own_follow(&room_id).await {
        FollowAnswer::Following(at) => chrono::DateTime::parse_from_rfc3339(&at)
            .ok()
            .map(|t| Some(t.with_timezone(&chrono::Utc))),
        FollowAnswer::NotFollowing => Some(None),
        FollowAnswer::Unknown => None,
    };
    let gen = crate::services::chat_send_limiter::note_own_follow(&channel, answer);
    reship_room_state(&channel).await;
    run_follow_countdown(channel, gen).await;
}

/// Word the room again each time the followers-only wait crosses a minute,
/// until it ends. Stops as soon as a newer follow answer, a role change or the
/// mode going off makes it moot. Nothing runs when there is no wait.
pub(crate) async fn run_follow_countdown(channel: String, gen: u64) {
    while let Some(wait) = crate::services::chat_send_limiter::own_follow_tick(&channel, gen) {
        tokio::time::sleep(wait).await;
        reship_room_state(&channel).await;
    }
}

/// The account followed or unfollowed `room_id` from inside the app: word every
/// open room of that channel again.
pub async fn own_follow_changed(room_id: &str, following: bool) {
    for (channel, gen) in crate::services::chat_send_limiter::note_own_follow_by_room_id(room_id, following) {
        reship_room_state(&channel).await;
        tauri::async_runtime::spawn(run_follow_countdown(channel, gen));
    }
}

/// Chat messages published while nothing was subscribed to the bus.
///
/// `broadcast::send` discards when there are no receivers, and a provider's
/// join backlog is published before the frontend WebSocket client attaches.
/// Bounded, and drained by the first client to attach. The frontend dedupes
/// by message id, so a replayed row cannot double up one that arrived live.
static PENDING_MESSAGES: OnceLock<Mutex<Vec<BridgeFrame>>> = OnceLock::new();
/// Enough for a full join backlog with headroom; past this the oldest go, because
/// a buffer that grows without a listener is a leak, not a feature.
const PENDING_MESSAGES_MAX: usize = 200;

fn get_pending_messages() -> &'static Mutex<Vec<BridgeFrame>> {
    PENDING_MESSAGES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Hold a message that had no subscriber, for replay when one attaches.
pub async fn hold_undelivered_message(frame: BridgeFrame) {
    let mut pending = get_pending_messages().lock().await;
    if pending.len() >= PENDING_MESSAGES_MAX {
        let overflow = pending.len() + 1 - PENDING_MESSAGES_MAX;
        pending.drain(0..overflow);
    }
    pending.push(frame);
}

/// Take everything held, leaving the buffer empty.
pub async fn take_undelivered_messages() -> Vec<BridgeFrame> {
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

/// Hand a window that claims an already-JOINed channel the room as last known.
/// Twitch sends ROOMSTATE only on the JOIN itself and on a change, so a chat
/// opened on a channel the connection already holds (another window's, or one
/// kept joined) would otherwise never learn its modes. A fresh JOIN has
/// nothing cached yet; its ROOMSTATE comes from the server.
async fn replay_room_state(key: &str) {
    let frame = get_room_state_cache().lock().await.get(key).cloned();
    if let Some(frame) = frame {
        send_to_bridge(BridgeFrame::channel(key, frame), false).await;
    }
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

/// Run `f` over a joined channel's emote set without cloning it. `None` when
/// the channel is not joined or its set has not landed yet. This set lives
/// exactly as long as the chat does and takes live 7TV changes, which is why
/// the composer's emote matching reads it rather than the picker's LRU.
pub async fn with_channel_emotes<R>(channel: &str, f: impl FnOnce(&EmoteSet) -> R) -> Option<R> {
    let map = get_channel_emotes().lock().await;
    map.get(&channel.to_lowercase()).map(f)
}

/// Run `f` over a user's 7TV personal emotes (name -> emote), which work in
/// every channel. `None` until 7TV has reported that user's personal set; ours
/// arrives through the presence we post on each channel we join.
pub fn with_personal_emotes<R>(twitch_id: &str, f: impl FnOnce(&HashMap<String, Emote>) -> R) -> Option<R> {
    let set = {
        let cache = get_personal_emotes().read().ok()?;
        cache.peek(twitch_id).map(|(_, map)| map.clone())?
    };
    Some(f(&set))
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
/// tier). Nothing in here ever replaces a span Twitch tagged as an emote.
pub(crate) struct EmoteLookup {
    /// name -> emote, inserted bttv, ffz, seven_tv in order (later insert
    /// wins), preserving the word tier's 7TV > FFZ > BTTV priority.
    by_name: HashMap<String, Emote>,
    /// The channel's own Twitch emotes. A live message never reads these (its
    /// Twitch emotes come from the IRC tag's positions, and matching names
    /// would turn a word into an emote its sender cannot use); they are for
    /// text that arrives without positions, like a reply's quoted parent.
    natives: Vec<Emote>,
}

impl EmoteLookup {
    pub(crate) fn build(set: &EmoteSet) -> Arc<Self> {
        let mut by_name =
            HashMap::with_capacity(set.bttv.len() + set.ffz.len() + set.seven_tv.len());
        for e in set.bttv.iter().chain(&set.ffz).chain(&set.seven_tv) {
            by_name.insert(e.name.clone(), e.clone());
        }
        Arc::new(Self { by_name, natives: set.twitch.clone() })
    }

    /// The third-party emote a word names. Only words Twitch did not tag reach
    /// this: a tagged span is always the Twitch emote.
    pub(crate) fn get(&self, name: &str) -> Option<&Emote> {
        self.by_name.get(name)
    }

    /// How many third-party names it knows.
    pub(crate) fn len(&self) -> usize {
        self.by_name.len()
    }
}

static CHANNEL_PARSE_LOOKUP: OnceLock<std::sync::RwLock<HashMap<String, Arc<EmoteLookup>>>> =
    OnceLock::new();

fn channel_parse_lookup() -> &'static std::sync::RwLock<HashMap<String, Arc<EmoteLookup>>> {
    CHANNEL_PARSE_LOOKUP.get_or_init(|| std::sync::RwLock::new(HashMap::new()))
}

/// Join each run of whitespace-only Text segments into one, keeping its exact
/// text. The renderer attaches a zero-width emote to the emote before it by
/// looking back past one spacer, so a doubled space (a composer autocomplete
/// leaves a trailing one) split into two spacers orphaned the overlay.
/// Chatterino and 7TV stack across any whitespace; one spacer per run matches
/// them, and a run of spaces renders the same either way.
fn merge_space_runs(segments: &mut Vec<MessageSegment>) {
    let is_space = |s: &MessageSegment| matches!(s, MessageSegment::Text { content } if !content.is_empty() && content.trim().is_empty());
    let mut out: Vec<MessageSegment> = Vec::with_capacity(segments.len());
    for seg in segments.drain(..) {
        if is_space(&seg) {
            if let (Some(MessageSegment::Text { content: prev }), MessageSegment::Text { content }) = (out.last_mut(), &seg) {
                if !prev.is_empty() && prev.trim().is_empty() {
                    prev.push_str(content);
                    continue;
                }
            }
        }
        out.push(seg);
    }
    *segments = out;
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
    crate::services::shared_chat_emotes::release_home(key);
}

/// What a CLEARMSG line says: one message deleted by a moderator.
struct ClearMsg {
    channel: Option<String>,
    target_msg_id: String,
    login: String,
    text: Option<String>,
}

impl ClearMsg {
    fn frame(&self) -> Value {
        json!({
            "type": "CLEARMSG",
            "channel": self.channel,
            "target_msg_id": self.target_msg_id,
            "login": self.login,
            "message": self.text
        })
    }
}

/// What a CLEARCHAT line says: a timeout or ban of one user, or a full clear.
struct ClearChat {
    channel: Option<String>,
    target_user_id: Option<String>,
    target_user: Option<String>,
    ban_duration_secs: Option<u64>,
}

impl ClearChat {
    fn frame(&self) -> Value {
        json!({
            "type": "CLEARCHAT",
            "channel": self.channel,
            "target_user_id": self.target_user_id,
            "target_user": self.target_user,
            "ban_duration": self.ban_duration_secs
        })
    }
}

impl IrcService {
    /// Format: `@login=<user>;room-id=<room>;target-msg-id=<msg-id>;tmi-sent-ts=<ts> :tmi.twitch.tv CLEARMSG #<channel> :<message>`
    fn parse_clearmsg(line: &str) -> Option<ClearMsg> {
        let target_msg_id = Self::extract_tag_value(line, "target-msg-id")?;
        Some(ClearMsg {
            channel: extract_channel_from_irc_line(line),
            target_msg_id,
            login: Self::extract_tag_value(line, "login").unwrap_or_default(),
            // The deleted message text is the trailing param after the last " :".
            text: line.rfind(" :").map(|idx| line[idx + 2..].trim().to_string()),
        })
    }

    /// Format: `@ban-duration=<sec>;room-id=<room>;target-user-id=<id>;tmi-sent-ts=<ts> :tmi.twitch.tv CLEARCHAT #<channel> :<user>`,
    /// or `:tmi.twitch.tv CLEARCHAT #<channel>` for a full clear.
    fn parse_clearchat(line: &str) -> ClearChat {
        ClearChat {
            channel: extract_channel_from_irc_line(line),
            target_user_id: Self::extract_tag_value(line, "target-user-id"),
            // The target username is the trailing param. A full clear has no
            // trailing " :", so an earlier inline colon is never read as one.
            target_user: line.rfind(" :").map(|idx| line[idx + 2..].trim().to_string()),
            ban_duration_secs: Self::extract_tag_value(line, "ban-duration").map(|d| d.parse::<u64>().unwrap_or(0)),
        }
    }

    /// The ROOMSTATE frame for a line, merged with what the room already said
    /// so the page and the replay cache never hold a partial frame.
    fn roomstate_frame(line: &str, channel: Option<&str>) -> String {
        // Forward room state to frontend — only include tags actually present
        // Twitch sends FULL roomstate on join, PARTIAL on setting changes
        let mut room_state = serde_json::Map::new();
        room_state.insert("type".into(), serde_json::json!("ROOMSTATE"));
        if let Some(ch) = channel {
            room_state.insert("channel".into(), serde_json::json!(ch));
        }

        if let Some(v) = Self::extract_tag_value(line, "followers-only") {
            if let Ok(n) = v.parse::<i64>() {
                room_state.insert("followers_only".into(), serde_json::json!(n));
            }
        }
        if let Some(v) = Self::extract_tag_value(line, "slow") {
            if let Ok(n) = v.parse::<u64>() {
                room_state.insert("slow".into(), serde_json::json!(n));
            }
        }
        if let Some(v) = Self::extract_tag_value(line, "subs-only") {
            if let Ok(n) = v.parse::<u8>() {
                room_state.insert("subs_only".into(), serde_json::json!(n == 1));
                if let Some(ch) = channel {
                    crate::services::channel_rewards::note_subs_only(ch, n == 1);
                }
            }
        }
        if let Some(v) = Self::extract_tag_value(line, "emote-only") {
            if let Ok(n) = v.parse::<u8>() {
                room_state.insert("emote_only".into(), serde_json::json!(n == 1));
            }
        }
        if let Some(v) = Self::extract_tag_value(line, "r9k") {
            if let Ok(n) = v.parse::<u8>() {
                room_state.insert("r9k".into(), serde_json::json!(n == 1));
            }
        }

        if let Some(ch) = channel {
            let tag_u8 = |name: &str| {
                Self::extract_tag_value(line, name)
                    .and_then(|v| v.parse::<u8>().ok())
                    .map(|n| n == 1)
            };
            crate::services::chat_send_limiter::note_room_state(
                ch,
                crate::services::chat_send_limiter::RoomModes {
                    slow_seconds: Self::extract_tag_value(line, "slow").and_then(|v| v.parse().ok()),
                    followers_only_minutes: Self::extract_tag_value(line, "followers-only")
                        .and_then(|v| v.parse().ok()),
                    subs_only: tag_u8("subs-only"),
                    emote_only: tag_u8("emote-only"),
                    r9k: tag_u8("r9k"),
                },
            );
            if let Some(room_id) = Self::extract_tag_value(line, "room-id") {
                crate::services::chat_send_limiter::note_room_id(ch, &room_id);
            }
            // Ship the merged room, with its modes in words, so the page
            // and the replay cache never hold a partial frame.
            if let Some(fields) = crate::services::chat_send_limiter::room_state_fields(ch) {
                room_state.extend(fields);
            }
            // A followers-only room: learn whether the account already
            // follows long enough, then word the room again.
            if let Some(room_id) = crate::services::chat_send_limiter::own_follow_wanted(ch) {
                tauri::async_runtime::spawn(learn_own_follow(ch.to_string(), room_id));
            }
        }
        serde_json::Value::Object(room_state).to_string()
    }
}

/// An emote's size at 1x in CSS pixels, where it is known before the image
/// loads: Twitch emotes are square at 28; a 7TV emote's 1x file size is in
/// the room's table.
pub(crate) fn emote_size_hint(channel: &str, code: &str, url: &str) -> Option<(u32, u32)> {
    if url.contains("static-cdn.jtvnw.net/emoticons/") {
        return Some((28, 28));
    }
    let lookup = channel_parse_lookup().read().ok()?.get(channel).cloned()?;
    let emote = lookup.get(code)?;
    let width = emote.width?;
    match emote.provider {
        crate::services::emote_service::EmoteProvider::SevenTV => Some((width, emote.height.unwrap_or(32))),
        _ => None,
    }
}

/// Sends one non-row frame to the windows reading `channel`.
pub(crate) async fn post_channel_frame(channel: &str, frame: String) {
    send_to_bridge(BridgeFrame::channel(channel, frame), false).await;
}

/// A non-row frame for every room (a member's look changing everywhere at once).
pub(crate) async fn post_frame_to_all(frame: String) {
    send_to_bridge(BridgeFrame::all(frame), false).await;
}

fn clear_parse_lookups() {
    if let Ok(mut guard) = channel_parse_lookup().write() {
        guard.clear();
    }
    crate::services::shared_chat_emotes::release_all();
}

/// The message body of a `:prefix COMMAND #channel :text` line, or "". The
/// colon is optional when the body is one word (Twitch and the history mirror
/// both leave it out), which is how a one-emote message arrives.
pub(crate) fn irc_trailing_text(raw: &str) -> &str {
    let rest = if raw.starts_with('@') { raw.split_once(' ').map(|(_, r)| r).unwrap_or("") } else { raw };
    let text = rest.splitn(4, ' ').nth(3).unwrap_or("");
    text.strip_prefix(':').unwrap_or(text)
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
            no_cheermotes: false,
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
    /// Text nobody cheered with (a reward title) never turns a word like
    /// `Cheer100` into bits; without a channel set the global prefixes
    /// would still match.
    no_cheermotes: bool,
}

/// One cheermote tier from Helix: bits threshold, hex color, animated dark art,
/// plus what the bits picker needs about its cheermote.
#[derive(Debug, Clone, Default)]
pub struct CheermoteTier {
    pub min_bits: u32,
    pub color: String,
    pub url: String,
    /// The cheermote's prefix as Twitch writes it ("Cheer", "Corgo"); the set
    /// is keyed by the lowercase form.
    pub prefix: String,
    /// The signed-in viewer can cheer with this tier. False for a tier Twitch
    /// marks `can_cheer: false` and for every tier of a `display_only`
    /// cheermote: those still render when someone with access used them.
    pub can_cheer: bool,
    /// Twitch's display order for the cheermote.
    pub order: u32,
    /// The channel's own cheermote rather than a global one.
    pub custom: bool,
}

/// The channel's cached cheermote set (`key` = lowercase login), if Helix has
/// answered for it since the channel was joined.
pub fn channel_cheermote_set(key: &str) -> Option<Arc<CheermoteSet>> {
    get_channel_cheermotes().read().ok()?.get(key).cloned()
}

/// A word that is a cheer in `set`: the longest prefix followed only by
/// digits, with a nonzero amount. Returns the matched tiers and the amount.
pub fn match_cheermote<'a>(word: &str, set: &'a CheermoteSet) -> Option<(&'a str, u32, &'a [CheermoteTier])> {
    if !word.ends_with(|c: char| c.is_ascii_digit()) || !word.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let word_lower = word.to_lowercase();
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
    (bits > 0).then_some((prefix, bits, tiers.as_slice()))
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

/// Channel -> labels of the windows holding it. A channel stays JOINed while
/// any window holds it.
type ChannelClaims = HashMap<String, HashSet<String>>;

/// Records `window` as holding `channel`. True when it did not hold it yet.
fn claim_channel(claims: &mut ChannelClaims, channel: &str, window: &str) -> bool {
    claims
        .entry(channel.to_string())
        .or_default()
        .insert(window.to_string())
}

/// Drops `window`'s hold on `channel` and returns how many windows still hold
/// it. At zero no window holds the channel and it may PART.
fn release_channel_claim(claims: &mut ChannelClaims, channel: &str, window: &str) -> usize {
    let Some(holders) = claims.get_mut(channel) else {
        return 0;
    };
    holders.remove(window);
    let left = holders.len();
    if left == 0 {
        claims.remove(channel);
    }
    left
}

/// Drops every hold `window` has, except on `keep`, and returns the channels
/// no window holds any more.
fn release_window_holds(claims: &mut ChannelClaims, window: &str, keep: Option<&str>) -> Vec<String> {
    let mut emptied = Vec::new();
    claims.retain(|channel, holders| {
        if keep == Some(channel.as_str()) {
            return true;
        }
        holders.remove(window);
        if holders.is_empty() {
            emptied.push(channel.clone());
            false
        } else {
            true
        }
    });
    emptied
}

/// Whether any window other than `window` holds a channel.
fn held_by_another_window(claims: &ChannelClaims, window: &str) -> bool {
    claims
        .values()
        .any(|holders| holders.iter().any(|w| w != window))
}

/// Whether `window` holds any channel.
fn held_by(claims: &ChannelClaims, window: &str) -> bool {
    claims.values().any(|holders| holders.contains(window))
}

#[cfg(test)]
mod channel_claim_tests {
    use super::*;

    #[test]
    fn a_channel_stays_held_while_any_window_holds_it() {
        let mut claims = ChannelClaims::new();
        assert!(claim_channel(&mut claims, "caseoh_", "overlay-caseoh_"));
        assert!(claim_channel(&mut claims, "caseoh_", "main"));
        // Main lets go while the floating chat still shows it: no PART.
        assert_eq!(release_channel_claim(&mut claims, "caseoh_", "main"), 1);
        assert!(claims.contains_key("caseoh_"));
        // The floating chat lets go: nobody holds it, so it PARTs.
        assert_eq!(release_channel_claim(&mut claims, "caseoh_", "overlay-caseoh_"), 0);
        assert!(claims.is_empty());
    }

    #[test]
    fn a_window_holds_a_channel_once_however_often_it_claims() {
        let mut claims = ChannelClaims::new();
        assert!(claim_channel(&mut claims, "xqc", "multichat-a"));
        assert!(!claim_channel(&mut claims, "xqc", "multichat-a"));
        assert_eq!(release_channel_claim(&mut claims, "xqc", "multichat-a"), 0);
        assert!(claims.is_empty());
    }

    #[test]
    fn a_claim_after_a_release_holds_the_channel_again() {
        // A remount: claim, release, claim again. The second claim holds the
        // channel, so another window letting go does not PART it.
        let mut claims = ChannelClaims::new();
        claim_channel(&mut claims, "caseoh_", "overlay-caseoh_");
        assert_eq!(release_channel_claim(&mut claims, "caseoh_", "overlay-caseoh_"), 0);
        assert!(claim_channel(&mut claims, "caseoh_", "overlay-caseoh_"));
        assert_eq!(release_channel_claim(&mut claims, "caseoh_", "main"), 1);
    }

    #[test]
    fn releasing_a_channel_nobody_holds_is_zero() {
        let mut claims = ChannelClaims::new();
        assert_eq!(release_channel_claim(&mut claims, "shroud", "main"), 0);
        assert!(claims.is_empty());
    }

    #[test]
    fn a_closed_window_frees_only_what_no_other_window_holds() {
        let mut claims = ChannelClaims::new();
        claim_channel(&mut claims, "caseoh_", "main");
        claim_channel(&mut claims, "xqc", "main");
        claim_channel(&mut claims, "caseoh_", "overlay-caseoh_");
        claim_channel(&mut claims, "shroud", "multichat-a");
        let mut freed = release_window_holds(&mut claims, "main", None);
        freed.sort();
        assert_eq!(freed, vec!["xqc".to_string()]);
        assert_eq!(claims.get("caseoh_").map(|h| h.len()), Some(1));
        assert!(claims.contains_key("shroud"));
    }

    #[test]
    fn a_window_sweep_keeps_the_channel_being_claimed() {
        let mut claims = ChannelClaims::new();
        claim_channel(&mut claims, "old", "overlay-x");
        claim_channel(&mut claims, "new", "overlay-x");
        assert_eq!(release_window_holds(&mut claims, "overlay-x", Some("new")), vec!["old".to_string()]);
        assert!(claims.get("new").is_some_and(|h| h.contains("overlay-x")));
    }

    #[test]
    fn stopping_chat_sees_other_windows_holds() {
        let mut claims = ChannelClaims::new();
        assert!(!held_by(&claims, "main"));
        claim_channel(&mut claims, "xqc", "main");
        assert!(held_by(&claims, "main"));
        assert!(!held_by_another_window(&claims, "main"));
        claim_channel(&mut claims, "caseoh_", "overlay-caseoh_");
        assert!(held_by_another_window(&claims, "main"));
        assert!(held_by_another_window(&claims, "overlay-caseoh_"));
        release_channel_claim(&mut claims, "caseoh_", "overlay-caseoh_");
        assert!(!held_by_another_window(&claims, "main"));
        assert!(!held_by(&claims, "overlay-caseoh_"));
    }
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

/// The command of a raw IRC line (`PRIVMSG`, `USERNOTICE`, ...): the first
/// token after the optional tags (`@...`) and prefix (`:...`). Read by token,
/// so a tag value or message text naming a command never matches.
fn irc_command(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    if rest.starts_with('@') {
        rest = rest.split_once(' ')?.1.trim_start();
    }
    if rest.starts_with(':') {
        rest = rest.split_once(' ')?.1.trim_start();
    }
    rest.split(' ').next().filter(|c| !c.is_empty())
}

/// A USERNOTICE with neither the chatter's text nor a system message renders
/// as a blank row (an `onetapgiftredeemed` with no text, for one); live chat
/// and history both leave it out.
fn usernotice_has_visible_content(msg: &ChatMessage) -> bool {
    !msg.content.is_empty() || msg.metadata.system_message.as_ref().is_some_and(|s| !s.is_empty())
}

/// One tag's raw value from an IRC line's tag block.
fn line_tag<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let tags = line.strip_prefix('@')?.split_once(' ')?.0;
    tags.split(';').find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
}

/// The trailing parameter of an IRC line (the text after the first " :" that
/// follows the command), or None when the line has none.
fn line_trailing(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();
    if rest.starts_with('@') {
        rest = rest.split_once(' ')?.1.trim_start();
    }
    if rest.starts_with(':') {
        rest = rest.split_once(' ')?.1;
    }
    rest.split_once(" :").map(|(_, t)| t.trim_end())
}

/// The NOTICEs a room sends everyone when a moderator changes a chat mode.
/// Every other NOTICE answers one chatter's own action (a refused send, a
/// command result), so a backlog never carries one worth showing.
fn is_room_mode_notice(msg_id: &str) -> bool {
    matches!(
        msg_id,
        "followers_on"
            | "followers_on_zero"
            | "followers_off"
            | "slow_on"
            | "slow_off"
            | "subs_on"
            | "subs_off"
            | "emote_only_on"
            | "emote_only_off"
            | "r9k_on"
            | "r9k_off"
    )
}

/// A room-mode NOTICE from the backlog as the row live chat shows for it: a
/// "System" row signed Twitch, carrying the notice text ("This room is now in
/// followers-only mode."). None for any other NOTICE.
fn history_notice_row(raw: &str) -> Option<ChatMessage> {
    let msg_id = line_tag(raw, "msg-id")?;
    if !is_room_mode_notice(msg_id) {
        return None;
    }
    let text = line_trailing(raw).filter(|t| !t.is_empty())?.to_string();
    let channel = extract_channel_from_irc_line(raw).unwrap_or_default();
    let timestamp = line_tag(raw, "tmi-sent-ts")
        .or_else(|| line_tag(raw, "rm-received-ts"))
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis().to_string());
    let (formatted_timestamp, formatted_timestamp_with_seconds) = IrcService::format_timestamp(&timestamp);
    // Stable across parses of the same line, so a later re-parse of the
    // backlog names the same row.
    let id = format!("notice-{channel}-{msg_id}-{timestamp}");
    let tags: HashMap<String, String> = [
        ("id", id.as_str()),
        ("user-id", "tw-system"),
        ("msg-id", msg_id),
        ("historical", "1"),
        ("tmi-sent-ts", timestamp.as_str()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    Some(ChatMessage {
        id,
        user_id: "tw-system".to_string(),
        username: "System".to_string(),
        display_name: "Twitch".to_string(),
        color: Some("#9147ff".to_string()),
        badges: vec![Badge {
            name: "staff".to_string(),
            version: "1".to_string(),
            image_url_1x: None,
            image_url_2x: None,
            image_url_4x: None,
            title: None,
            description: None,
        }],
        timestamp,
        content: text.clone(),
        provider: "twitch".to_string(),
        channel,
        emotes: Vec::new(),
        tags,
        layout: LayoutResult::default(),
        segments: vec![MessageSegment::Text { content: text }],
        metadata: MessageMetadata {
            formatted_timestamp,
            formatted_timestamp_with_seconds,
            ..Default::default()
        },
    })
}

/// A backlog CLEARMSG removes the one row it deleted, and a CLEARCHAT naming a
/// user removes that user's rows before it. The mirror is asked to leave
/// moderated rows out already (`hide_moderated_messages`); this keeps the
/// result the same when a line slips through. A whole-chat CLEARCHAT removes
/// nothing, as live chat marks nothing for one.
fn apply_history_clear(rows: &mut Vec<ChatMessage>, raw: &str) {
    match irc_command(raw) {
        Some("CLEARMSG") => {
            if let Some(target) = line_tag(raw, "target-msg-id").filter(|t| !t.is_empty()) {
                rows.retain(|m| m.id != target);
            }
        }
        Some("CLEARCHAT") => {
            if let Some(user) = line_tag(raw, "target-user-id").filter(|t| !t.is_empty()) {
                rows.retain(|m| m.user_id != user);
            }
        }
        _ => {}
    }
}

/// What a backlog row looks like to a re-stamp: its segments and its quoted
/// parent's, which are the parts an emote table changes.
fn history_row_signature(msg: &ChatMessage) -> String {
    serde_json::to_string(&(
        &msg.segments,
        msg.metadata.reply_info.as_ref().and_then(|r| r.parent_segments.as_ref()),
    ))
    .unwrap_or_default()
}

/// The rows of a second parse of a backlog whose segments differ from the
/// first parse (`before`: id to signature), as the page patches them in
/// place: `{ id, segments, parent_segments }`. Rows the first parse did not
/// have are left out; the page never showed them.
fn history_segment_updates(before: &HashMap<String, String>, again: &[ChatMessage]) -> Vec<Value> {
    again
        .iter()
        .filter(|m| before.get(&m.id).is_some_and(|sig| *sig != history_row_signature(m)))
        .map(|m| {
            let mut row = json!({
                "id": m.id,
                "segments": m.segments,
                "parent_segments": m.metadata.reply_info.as_ref().and_then(|r| r.parent_segments.as_ref()),
            });
            // A native renderer draws from the view, so the patch carries the new one.
            if let Some(view) = &m.metadata.view {
                row["view"] = json!(view);
            }
            row
        })
        .collect()
}

/// Announces each channel whose emote table was put in place (the saved
/// dictionary or a provider refresh), for work waiting on one.
static EMOTE_TABLE_INSTALLED: OnceLock<broadcast::Sender<String>> = OnceLock::new();

fn emote_table_installed() -> &'static broadcast::Sender<String> {
    EMOTE_TABLE_INSTALLED.get_or_init(|| broadcast::channel(64).0)
}

fn note_emote_table_installed(key: &str) {
    let _ = emote_table_installed().send(key.to_string());
}

/// Wait until `channel`'s emote table is in place, up to `budget`. True when
/// it is.
pub async fn wait_for_channel_emotes(channel: &str, budget: std::time::Duration) -> bool {
    let key = channel.to_lowercase();
    // Subscribe before looking, so a table landing in between is still heard.
    let mut installed = emote_table_installed().subscribe();
    if with_channel_emotes(&key, |_| ()).await.is_some() {
        return true;
    }
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        match tokio::time::timeout_at(deadline, installed.recv()).await {
            Ok(Ok(k)) if k == key => return true,
            Ok(Ok(_)) => {}
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                if with_channel_emotes(&key, |_| ()).await.is_some() {
                    return true;
                }
            }
            Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => {
                return with_channel_emotes(&key, |_| ()).await.is_some();
            }
        }
    }
}

/// How long a backlog parsed without its channel's emote table waits for the
/// table before giving up on re-stamping. The channel document fetch has a
/// 25 s budget.
const HISTORY_RESTAMP_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// A reward that asks for text reaches chat as an ordinary message tagged only
/// with its id. Name it: `sn-redeemed-title` and `sn-redeemed-cost` give the
/// row "Redeemed <title>" and the cost. These are not the `sn-reward-*` tags of
/// a no-text redemption row, which the stream overlay draws as an event card
/// with no message.
fn stamp_reward_name(tags: &mut HashMap<String, String>, raw: &str) {
    let Some(reward_id) = tags.get("custom-reward-id").filter(|id| !id.is_empty()).cloned() else { return };
    let Some(channel) = extract_channel_from_irc_line(raw) else { return };
    let Some(name) = crate::services::channel_rewards::reward_name(&channel, &reward_id) else { return };
    tags.insert("sn-redeemed-title".into(), name.title);
    if name.cost > 0 {
        tags.insert("sn-redeemed-cost".into(), name.cost.to_string());
    }
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
        crate::services::channel_rewards::forget_all_rooms();
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
                    irc_transport::note_connect_failed();
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
            // Read before any await below, while they still describe how the
            // session ended.
            let read_age_ms = irc_read_age_ms();
            let read_wall_age_ms = irc_read_wall_age_ms();

            // Between sessions: fail sends/JOINs fast instead of writing into
            // a dead socket, retire this session's keepalive tasks, and drop
            // its JOIN-ack state — the next session re-JOINs and re-confirms
            // everything in CURRENT_CHANNELS from scratch.
            *get_irc_writer().lock().await = None;
            abort_keepalive_tasks().await;
            tracker_clear().await;

            let delay = match outcome {
                Ok(SessionEnd { reason, transport }) => {
                    // An Ok return proves IRC_CONNECTED went out this attempt
                    // (irc_session's contract), so this loss is a real one.
                    EVER_ESTABLISHED.store(true, std::sync::atomic::Ordering::Relaxed);
                    send_to_bridge("IRC_RECONNECTING".to_string(), false).await;
                    crate::services::chat_outage::lost();
                    let delay = if session_lived_ms < SESSION_FLAP_THRESHOLD_MS {
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
                            "session ended: {} ({}s since the last read); reconnecting in {}s",
                            reason,
                            read_age_ms / 1000,
                            d.as_secs()
                        ));
                        d
                    };
                    if is_silent_session_end(reason, read_age_ms, read_wall_age_ms) {
                        irc_transport::note_session_deaf(transport);
                    }
                    delay
                }
                Err(SessionError::Transient(e)) => {
                    irc_transport::note_connect_failed();
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
    ) -> std::result::Result<SessionEnd, SessionError> {
        debug!("[IRC Chat] Connecting to Twitch IRC...");

        let connect_started = std::time::Instant::now();
        let connected = irc_transport::connect_transport()
            .await
            .map_err(|reason| SessionError::Transient(anyhow::anyhow!("{}", reason)))?;
        let transport = connected.transport;
        let other_failed = connected.other_failed;
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
        record_lifecycle(&format!(
            "authenticated via {} in {}ms{}",
            transport.describe(),
            connect_started.elapsed().as_millis(),
            other_failed
                .as_deref()
                .map(|e| format!(" ({} failed: {})", transport.other().describe(), e))
                .unwrap_or_default()
        ));
        let authenticated_at = tokio::time::Instant::now();

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
            note_joins_sent(channels.len());
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
                        note_joins_sent(chunk.len());
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
        crate::services::chat_outage::restored().await;

        // Flush queued messages (dropped if no receiver is attached yet; the
        // WS handshake also drains this queue when a client connects).
        let queued: Vec<BridgeFrame> = {
            let mut queue = get_message_queue().lock().await;
            queue.drain(..).collect()
        };
        if !queued.is_empty() {
            debug!("[IRC Chat] Flushing {} queued messages", queued.len());
            for msg in queued {
                send_to_bridge(msg, false).await;
            }
        }

        // Start ping task to keep IRC connection alive. It PINGs only once the
        // link has been quiet for IRC_PING_IDLE_MS, and every PING elicits a
        // PONG read, so the read timeout bounds dead-connection detection.
        let writer_clone = writer.clone();
        let ping_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(IRC_PING_CHECK);
            let mut last_ping_ms: Option<u64> = None;
            loop {
                interval.tick().await;
                let now = mono_ms();
                let pinged_recently =
                    last_ping_ms.is_some_and(|t| now.saturating_sub(t) < IRC_PING_IDLE_MS);
                if irc_read_age_ms() < IRC_PING_IDLE_MS || pinged_recently {
                    continue;
                }
                last_ping_ms = Some(now);
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
                        take_join_slot().await;
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

        // Listen for messages. The deadline is the half-open detector: we PING
        // a quiet link and the server answers, so IRC_READ_TIMEOUT without a
        // completed read means the socket is dead even if the OS never reports
        // it. It runs from the last ACCEPTED read, so the fault injection's
        // discarded lines count as silence, exactly like a blackhole.
        let deafen_after = fault_deafen_after(transport);
        let mut deafened = false;
        let mut last_accepted = tokio::time::Instant::now();
        loop {
            line.clear();
            let deadline = last_accepted + IRC_READ_TIMEOUT;
            let deaf = deafen_after.is_some_and(|d| authenticated_at.elapsed() >= d);
            let end_reason: Option<&'static str> =
                match tokio::time::timeout_at(deadline, reader.read_line(&mut line)).await {
                    Err(_) => {
                        warn!(
                            "[IRC Chat] no IRC traffic for {}s, connection presumed dead",
                            IRC_READ_TIMEOUT.as_secs()
                        );
                        Some("read liveness timeout")
                    }
                    Ok(Ok(0)) => Some("closed by server"),
                    Ok(Ok(_)) if deaf => {
                        if !deafened {
                            deafened = true;
                            record_lifecycle(&format!(
                                "fault injection: discarding everything read on {} from now on",
                                transport.describe()
                            ));
                        }
                        None
                    }
                    Ok(Ok(_)) => {
                        last_accepted = tokio::time::Instant::now();
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
                return Ok(SessionEnd { reason, transport });
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
                if !verdict.drop {
                    crate::services::mention_ping::on_live_message(
                        &chat_msg,
                        verdict.mentioned,
                        verdict.reply_to_me,
                    );
                    crate::services::chat_row_view::stamp(&mut chat_msg);
                    #[cfg(desktop)]
                    crate::services::mention_notify::on_live_message(
                        &chat_msg,
                        verdict.mentioned,
                        verdict.reply_to_me,
                    );
                }

                // Deliver to the frontend FIRST (wire order is the only order
                // the UI needs), then hand the slow side effects (history LRU,
                // chat logger, plugins) to the ordered lane so they can never
                // block the read loop.
                if !verdict.drop {
                    if let Ok(json_msg) = serde_json::to_string(&chat_msg) {
                        send_to_bridge(
                            BridgeFrame::row(
                                &chat_msg.channel,
                                json_msg,
                                chat_msg.metadata.is_mentioned || chat_msg.metadata.is_reply_to_me,
                            ),
                            true,
                        )
                        .await;
                    }
                    crate::services::reminder_service::on_message(&chat_msg);
                }
                enqueue_side_effect(MessageSideEffects {
                    history_key: history_key_for(&chat_msg),
                    msg: chat_msg,
                    add_history: true,
                });
            } else {
                // Fallback to sending raw string if parsing fails
                let text = enhanced_message.to_string();
                let ch = extract_channel_from_irc_line(&text);
                send_to_bridge(BridgeFrame::maybe_channel(ch.as_deref(), text), true).await;
            }
        } else if trimmed.contains("USERNOTICE") {
            // Subscription, resub, gift sub, etc.
            // The viewer's own sub, or a gift to them, shows in every chat menu at once.
            if let Some((room, subscriber)) = crate::services::channel_relationship::sub_notice(trimmed) {
                let mine = crate::services::account_store::AccountStore::primary()
                    .is_some_and(|me| me.login.eq_ignore_ascii_case(&subscriber));
                if mine {
                    tauri::async_runtime::spawn(async move {
                        crate::services::channel_state::own_sub_seen(&room).await;
                    });
                }
            }
            // Parse USERNOTICE messages - layout will be measured by frontend
            if let Some(mut chat_msg) = Self::parse_usernotice(trimmed) {
                // Skip USERNOTICE messages with no visible content
                // These render as blank/ghost messages (e.g., "onetapgiftredeemed" with no text)
                if !usernotice_has_visible_content(&chat_msg) {
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
                if !verdict.drop {
                    crate::services::chat_row_view::stamp(&mut chat_msg);
                }

                // Frontend first, side effects on the ordered lane (no history:
                // USERNOTICE never fed the profile-card history).
                if !verdict.drop {
                    if let Ok(json_msg) = serde_json::to_string(&chat_msg) {
                        send_to_bridge(
                            BridgeFrame::row(
                                &chat_msg.channel,
                                json_msg,
                                chat_msg.metadata.is_mentioned || chat_msg.metadata.is_reply_to_me,
                            ),
                            true,
                        )
                        .await;
                    }
                }
                enqueue_side_effect(MessageSideEffects {
                    history_key: history_key_for(&chat_msg),
                    msg: chat_msg,
                    add_history: false,
                });
            } else {
                // Fallback to raw string if parsing fails
                send_to_bridge(
                    BridgeFrame::maybe_channel(extract_channel_from_irc_line(trimmed).as_deref(), trimmed.to_string()),
                    true,
                )
                .await;
            }
        } else if trimmed.contains("ROOMSTATE") {
            // Room state updates (slow mode, sub-only, etc.)
            debug!("[IRC Chat] Room state update: {}", trimmed);

            // Extract channel so the synthetic message is routable and the cache
            // is keyed per channel.
            let channel_name = extract_channel_from_irc_line(trimmed);

            let room_state_str = Self::roomstate_frame(trimmed, channel_name.as_deref());

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

            send_to_bridge(BridgeFrame::maybe_channel(channel_name.as_deref(), room_state_str), false).await;

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
                // In chat order, so rows built or repainted from it match everyone else's.
                let badges = crate::models::chat_layout::order_twitch_badge_tag(&badges);
                debug!(
                    "[IRC Chat] Caching user badges from USERSTATE for {:?}: {}",
                    channel_name, badges
                );
                if let Some(ref ch) = channel_name {
                    crate::services::channel_rewards::note_viewer_badges(ch, &badges);
                    // A new role (or subscribing) changes what the room's
                    // modes mean for the account: resend the room in words.
                    if crate::services::chat_send_limiter::note_own_badges(ch, &badges) {
                        reship_room_state(ch).await;
                    }
                    get_user_badges_cache()
                        .lock()
                        .await
                        .insert(ch.clone(), badges.clone());
                }

                // Send badges to frontend tagged with the channel they apply to.
                // Format: USER_BADGES:#<channel>:<badges>. The leading '#' lets
                // the frontend parser locate the channel-prefix segment unambiguously.
                // A line with no channel is GLOBALUSERSTATE (this branch matches it
                // too): account-wide badges only, never a channel's subscriber or
                // founder badge. Forwarded untagged, the page applied it to its
                // only open channel and repainted the user's own rows without
                // their channel badges, so it is not forwarded at all.
                if let Some(ch) = &channel_name {
                    send_to_bridge(BridgeFrame::channel(ch, format!("USER_BADGES:#{}:{}", ch, badges)), false).await;
                }
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
                    send_to_bridge(BridgeFrame::maybe_channel(channel_name.as_deref(), color_message), false).await;
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

            if let Some(clear) = Self::parse_clearmsg(trimmed) {
                if let Some(ch) = &clear.channel {
                    ChatLoggerService::log_deleted_message(ch, &clear.login, clear.text.as_deref());
                    ChatHistory::mark_deleted(ch, &clear.target_msg_id);
                }
                // Send deletion event to frontend, tagged with channel for routing
                send_to_bridge(BridgeFrame::maybe_channel(clear.channel.as_deref(), clear.frame().to_string()), false).await;
            }
        } else if trimmed.contains("CLEARCHAT") {
            // User timed out/banned (clear all their messages) or chat cleared
            // Format: @ban-duration=<sec>;room-id=<room>;target-user-id=<id>;tmi-sent-ts=<ts> :tmi.twitch.tv CLEARCHAT #<channel> :<user>
            // Or for full chat clear: :tmi.twitch.tv CLEARCHAT #<channel>
            debug!("[IRC Chat] Chat clear/timeout: {}", trimmed);

            let clear = Self::parse_clearchat(trimmed);
            if let Some(ch) = &clear.channel {
                match &clear.target_user {
                    Some(user) => ChatLoggerService::log_timeout(ch, user, clear.ban_duration_secs),
                    None => ChatLoggerService::log_chat_cleared(ch),
                }
                if let Some(uid) = &clear.target_user_id {
                    ChatHistory::mark_user_cleared(ch, uid);
                }
            }
            send_to_bridge(BridgeFrame::maybe_channel(clear.channel.as_deref(), clear.frame().to_string()), false).await;
        } else if trimmed.contains("NOTICE") {
            // System notices — forward to frontend for user-facing handling
            debug!("[IRC Chat] Notice: {}", trimmed);

            // Extract the msg-id tag (e.g. "msg_followersonly", "msg_subsonly")
            // Present when twitch.tv/tags capability is active (requested at connect)
            let msg_id = Self::extract_tag_value(trimmed, "msg-id");

            // A running /spam or /pyramid there hears refusals and bans.
            if let (Some(id), Some(ch)) = (msg_id.as_deref(), extract_channel_from_irc_line(trimmed)) {
                crate::services::chat_burst::note_notice(&ch, id);
                // A send refused because of a warning locks the box behind
                // its Acknowledge button.
                if crate::services::chat_warning::is_warned_refusal(Some(id), None) {
                    tokio::spawn(async move {
                        crate::services::chat_warning::on_refusal(&ch, Some("msg_warned"), None).await;
                    });
                }
            }

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
            send_to_bridge(
                BridgeFrame::maybe_channel(extract_channel_from_irc_line(trimmed).as_deref(), notice_event.to_string()),
                false,
            )
            .await;
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
                                                // Who each partner is, ahead of their first row.
                                                crate::services::shared_chat_source::warm(
                                                    partner_ids.iter().filter(|id| id.as_str() != room_id).cloned().collect(),
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
    pub(crate) async fn fetch_and_store_cheermotes(key: String, broadcaster_id: String) {
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
            let kind = entry.get("type").and_then(|t| t.as_str()).unwrap_or_default();
            let display_only = kind == "display_only";
            let custom = kind == "channel_custom";
            let order = entry.get("order").and_then(|o| o.as_u64()).unwrap_or(u64::from(u32::MAX)) as u32;
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
                    let can_cheer = !display_only && t.get("can_cheer").and_then(|c| c.as_bool()).unwrap_or(false);
                    Some(CheermoteTier {
                        min_bits,
                        color,
                        url,
                        prefix: prefix.to_string(),
                        can_cheer,
                        order,
                        custom,
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
        tx: Arc<broadcast::Sender<BridgeFrame>>,
        born: u64,
    ) {
        let (mut local_tx, mut local_rx) = local_socket.split();
        let mut rx = tx.subscribe();
        // Resolves once a full stop has come after this socket's server was
        // built, even one that landed before this task started.
        let mut closing = bridge_close().subscribe();
        let stopped = async move {
            let _ = closing.wait_for(|g| *g != born).await;
        };
        tokio::pin!(stopped);

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
                let _ = local_tx.send(warp::ws::Message::text(msg.text.to_string())).await;
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
                if local_tx.send(warp::ws::Message::text(msg.text.to_string())).await.is_err() {
                    debug!("[WS] Client disconnected while sending queued messages");
                    return;
                }
            }
        }
        drop(queue);

        // Forward frames to this window by its route (services::chat_bridge_route):
        // until the window says which channels it holds it gets everything; after
        // that only its own channels, with the ones it holds back waiting here in
        // order and a per-second count sent instead.
        //
        // A lagged receiver is a recoverable miss: log it and keep draining. Only
        // `Closed`, a closed client, or a failed send ends the handler.
        let mut route = SocketRoute::default();
        let mut ticks = tokio::time::interval(std::time::Duration::from_secs(1));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = &mut stopped => {
                    let _ = local_tx
                        .send(warp::ws::Message::close_with(1012u16, "chat service stopped"))
                        .await;
                    debug!("[WS] Closed a page socket: the chat service stopped");
                    break;
                }
                frame = rx.recv() => match frame {
                    Ok(f) => {
                        if let Some(text) = route.on_frame(&f) {
                            if local_tx.send(warp::ws::Message::text(text.to_string())).await.is_err() {
                                debug!("[WS] Client disconnected");
                                break;
                            }
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
                },
                line = local_rx.next() => match line {
                    Some(Ok(m)) => {
                        if m.is_close() {
                            break;
                        }
                        if let Ok(text) = m.to_str() {
                            for out in route.on_line(text) {
                                if local_tx.send(warp::ws::Message::text(out.to_string())).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    Some(Err(_)) | None => break,
                },
                _ = ticks.tick() => {
                    for out in route.take_ticks() {
                        if local_tx.send(warp::ws::Message::text(out.to_string())).await.is_err() {
                            return;
                        }
                    }
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
            Self::record_own_irc_line(&channel, message).await;
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
        let newly_claimed = claim_channel(&mut *get_channel_consumers().lock().await, &key, window);

        if !newly_claimed {
            debug!(
                "[IRC Chat] join_channel({}): window {} already a consumer, reusing JOIN",
                key, window
            );
            // Still run the JOIN health probe: for a confirmed channel this is
            // two set lookups, but for a channel whose JOIN the server silently
            // dropped it re-issues the JOIN — which is what turns the user's
            // refresh into a real recovery instead of an IRC no-op.
            Self::ensure_joined(&key).await?;
            replay_room_state(&key).await;
            return Ok(());
        }

        // Make sure the channel is actually JOINed (no-op when another window
        // got there first). On failure, give back the claim so a later retry
        // can JOIN cleanly instead of seeing an existing consumer.
        if let Err(e) = Self::ensure_joined(&key).await {
            release_channel_claim(&mut *get_channel_consumers().lock().await, &key, window);
            return Err(e);
        }
        replay_room_state(&key).await;

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
                take_join_slot().await;
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
        let remaining = release_channel_claim(&mut *get_channel_consumers().lock().await, &key, window);

        if remaining > 0 {
            debug!(
                "[IRC Chat] leave_channel({}): {} consumer(s) remain, keeping JOIN",
                key, remaining
            );
            return Ok(());
        }

        // Named in the lifecycle log, so a chat that went quiet in one window
        // shows which window's release took the channel away.
        record_lifecycle(&format!("PART #{} (window {} let go, no window holds it)", key, window));
        Self::part_channel(&key).await?;
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
        let emptied = release_window_holds(&mut *get_channel_consumers().lock().await, window, keep);

        for channel in emptied {
            record_lifecycle(&format!(
                "PART #{} (window {} closed or restarted, no window holds it)",
                channel, window
            ));
            if let Err(e) = Self::part_channel(&channel).await {
                log::warn!("[IRC Chat] stale-claim PART for {} failed: {}", channel, e);
            }
        }
    }

    /// Whether a window other than `window` holds a Twitch channel, so the
    /// shared connection still carries someone else's chat.
    pub async fn held_by_other_windows(window: &str) -> bool {
        held_by_another_window(&*get_channel_consumers().lock().await, window)
    }

    /// Whether `window` itself holds a Twitch channel.
    pub async fn held_by_window(window: &str) -> bool {
        held_by(&*get_channel_consumers().lock().await, window)
    }

    /// Every Twitch channel the connection is in, sorted.
    pub async fn current_channel_list() -> Vec<String> {
        let mut list: Vec<String> = get_current_channels().lock().await.iter().cloned().collect();
        list.sort();
        list
    }

    /// The Twitch channels `window` holds, sorted.
    pub async fn channels_held_by(window: &str) -> Vec<String> {
        let claims = get_channel_consumers().lock().await;
        let mut held: Vec<String> =
            claims.iter().filter(|(_, holders)| holders.contains(window)).map(|(ch, _)| ch.clone()).collect();
        held.sort();
        held
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
            record_lifecycle(&format!("PART #{} (warm-up reap, no window holds it)", channel));
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
        crate::services::channel_rewards::forget_room(key);
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
                        note_emote_table_installed(&key);
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
                    get_channel_emotes().lock().await.insert(key.clone(), emote_set);
                    note_emote_table_installed(&key);
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
        let map: Arc<HashMap<String, Emote>> =
            Arc::new(emotes.into_iter().map(|e| (e.name.clone(), e)).collect());
        if let Ok(mut g) = get_personal_emotes().write() {
            g.put(twitch_id.clone(), (set_id, map.clone()));
        }
        if has_any {
            PERSONAL_EMOTES_PRESENT.store(true, std::sync::atomic::Ordering::Relaxed);
            Self::repaint_personal_rows(&twitch_id, &map).await;
        }
    }

    /// A personal set usually lands a second or so after its owner is first
    /// seen, so the messages they sent in that gap were parsed without it and
    /// show their emotes as plain words. Name those rows to every window
    /// (a `PERSONAL_EMOTES` bridge frame per channel) so each re-applies the set
    /// in place through `apply_personal_emotes`. Only rows whose text holds one
    /// of the set's names are named, which is usually none.
    async fn repaint_personal_rows(twitch_id: &str, set: &HashMap<String, Emote>) {
        let since_ms = chrono::Utc::now().timestamp_millis() - PERSONAL_REPAINT_WINDOW_MS;
        let rows = ChatHistory::recent_by_user(twitch_id, since_ms, PERSONAL_REPAINT_SCAN);
        for (channel, message_ids) in Self::rows_naming_personal_emotes(rows, set) {
            let frame = serde_json::json!({
                "type": "PERSONAL_EMOTES",
                "channel": channel,
                "user_id": twitch_id,
                "message_ids": message_ids,
            });
            send_to_bridge(BridgeFrame::channel(&channel, frame.to_string()), false).await;
        }
    }

    /// The ids, per channel, of the rows whose text holds one of the set's
    /// names as a whole word (the same split the parser uses).
    /// A reply's quoted parent as segments, tokenized with the channel's sets
    /// like a row (its Twitch emotes matched by name, since the parent arrives
    /// without positions). The parent's sender is not this message's, so no
    /// personal emotes; and no cheermotes, a quote does not spend bits.
    fn quoted_segments(body: &str, snapshots: &ParseSnapshots) -> Vec<MessageSegment> {
        let natives = snapshots
            .channel
            .as_deref()
            .map(|lookup| native_emote_positions(body, &lookup.natives))
            .unwrap_or_default();
        let ctx = ParseCtx {
            channel: snapshots.channel.as_deref(),
            no_cheermotes: true,
            ..Default::default()
        };
        Self::parse_message_segments(body, &natives, &ctx)
    }

    fn rows_naming_personal_emotes(
        rows: Vec<(String, String, String)>,
        set: &HashMap<String, Emote>,
    ) -> HashMap<String, Vec<String>> {
        let mut by_channel: HashMap<String, Vec<String>> = HashMap::new();
        for (channel, id, text) in rows {
            // A personal set belongs to a TWITCH id. The history ring also holds
            // other platforms' rows (keyed `provider:channel`), whose user ids are
            // a different number space: a Kick chatter sharing the number would
            // have been painted with a Twitch stranger's emotes.
            if channel.contains(':') {
                continue;
            }
            if text.split(' ').any(|word| set.contains_key(word)) {
                by_channel.entry(channel).or_default().push(id);
            }
        }
        by_channel
    }

    /// `segments` with the sender's 7TV personal emotes applied: every text
    /// segment that is exactly one of their names becomes that emote, built as
    /// the parser builds it. `None` when nothing changed or no set is held.
    pub fn apply_personal_emotes(twitch_id: &str, segments: &[MessageSegment]) -> Option<Vec<MessageSegment>> {
        with_personal_emotes(twitch_id, |set| {
            let mut changed = false;
            let out: Vec<MessageSegment> = segments
                .iter()
                .map(|segment| match segment {
                    MessageSegment::Text { content } => match set.get(content.as_str()) {
                        Some(emote) => {
                            changed = true;
                            MessageSegment::Emote {
                                content: content.clone(),
                                emote_id: Some(emote.id.clone()),
                                emote_url: emote.url.clone(),
                                is_zero_width: emote.is_zero_width,
                                modifier_flags: emote.modifier_flags,
                                is_personal: Some(true),
                            }
                        }
                        None => segment.clone(),
                    },
                    _ => segment.clone(),
                })
                .collect();
            changed.then_some(out)
        })
        .flatten()
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
    /// `partner`: the table of the Shared Chat room the row was sent in, which
    /// replaces the home channel's (see `shared_chat_emotes`).
    fn gather_parse_snapshots(channel: &str, sender_id: &str, partner: Option<Arc<EmoteLookup>>) -> ParseSnapshots {
        let channel_lookup = partner.or_else(|| channel_parse_lookup().read().ok().and_then(|g| g.get(channel).cloned()));
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

            // Add the Twitch emote - bounds already validated above
            let emote_name = &content[start_byte..end_byte_exclusive];

            // A Twitch chat GIF: the span is a bracketed description, not an
            // emote code, so it takes no text parsing.
            if emote.gif {
                segments.push(MessageSegment::Gif {
                    content: emote_name.to_string(),
                    gif_id: emote.id.clone(),
                    gif_url: emote.url.clone(),
                });
                last_char_index = emote.end + 1;
                continue;
            }

            // A span Twitch marked is Twitch's emote, always. A third-party
            // emote with the same name (a channel's 7TV `xqcL`) never replaces
            // it; third-party names only match words Twitch left untagged, in
            // parse_text_segment. Sub emotes are commonly re-uploaded to 7TV
            // under their Twitch names, so an override here put someone else's
            // art on a sub emote the sender actually used.
            segments.push(MessageSegment::Emote {
                content: emote_name.to_string(),
                emote_id: Some(emote.id.clone()),
                emote_url: emote.url.clone(),
                is_zero_width: None,
                modifier_flags: None,
                is_personal: None,
            });

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
            } else if let Some((prefix, bits, tier, color, cheermote_url)) = if ctx.no_cheermotes {
                None
            } else {
                Self::parse_cheermote(word, ctx.cheermotes)
            } {
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

        merge_space_runs(&mut segments);
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
            let (prefix, bits, tiers) = match_cheermote(word, set)?;
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

    /// The login in a PRIVMSG's `:login!login@login.tmi.twitch.tv` prefix.
    /// Read from the prefix itself, after the tags: tag values (a reply body,
    /// a system message) may contain `!` too.
    fn privmsg_prefix_login(raw: &str) -> Option<String> {
        let after_tags = if raw.starts_with('@') {
            &raw[raw.find(' ')? + 1..]
        } else {
            raw
        };
        let prefix = after_tags.strip_prefix(':')?.split(' ').next()?;
        let login = prefix.split('!').next()?;
        if login.is_empty() || login.contains('.') {
            // A server prefix (`tmi.twitch.tv`) names no chatter.
            return None;
        }
        Some(login.to_lowercase())
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

        // `username` is the LOGIN, as on every other row type: rules, plugins,
        // moderation and mentions all treat it as one. The display name (which
        // can be localized and differ from the login beyond case) is its own
        // field below. A PRIVMSG carries the login in its prefix.
        let username = Self::privmsg_prefix_login(raw)
            .or_else(|| tag_map.get("login").map(|s| s.to_string()))
            .or_else(|| tag_map.get("display-name").map(|s| s.to_lowercase()))
            .filter(|s| !s.is_empty())
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

        let mut tags_owned: HashMap<String, String> = tag_map
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        stamp_reward_name(&mut tags_owned, raw);

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

        let mut badges: Vec<Badge> = badges_str
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
        crate::services::badge_service::add_ffz_bot_badge(&user_id, &mut badges);
        crate::models::chat_layout::order_twitch_badges(&mut badges);
        // A Shared Chat partner row's channel badges are its own room's.
        if let Some(room) = crate::services::shared_chat_emotes::partner_room(
            tag_map.get("source-room-id").copied(),
            tag_map.get("room-id").copied(),
        ) {
            crate::services::shared_chat_emotes::wear_room_badges(room, &mut badges);
        }

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
                                let url = native_emote_url(id);
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
                // Filled once the channel's parse snapshot is in hand, below.
                parent_segments: None,
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
                parent_shown_name: String::new(),
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
        // A row from a Shared Chat partner uses that room's emotes, not ours.
        let partner = crate::services::shared_chat_emotes::partner_room(
            tag_map.get("source-room-id").copied(),
            tag_map.get("room-id").copied(),
        )
        .and_then(|room| crate::services::shared_chat_emotes::lookup_for(&privmsg_channel, room, raw));
        let snapshots = Self::gather_parse_snapshots(&privmsg_channel, &user_id, partner);
        // Cheermotes draw only in a message that carries bits, as on Twitch: a
        // "Cheer100" that spent nothing (typed as text, or the composer's own
        // copy before the echo) is plain text.
        let mut ctx = snapshots.ctx();
        ctx.no_cheermotes = !tag_map.get("bits").and_then(|b| b.parse::<u32>().ok()).is_some_and(|b| b > 0);
        let segments = Self::parse_message_segments(&content_for_segments, &emotes_adjusted, &ctx);
        let reply_info = reply_info.map(|mut reply| {
            if !reply.parent_msg_body.is_empty() {
                reply.parent_segments = Some(Self::quoted_segments(&reply.parent_msg_body, &snapshots));
            }
            reply
        });

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

        let mut badges: Vec<Badge> = badges_str
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
        crate::services::badge_service::add_ffz_bot_badge(&user_id, &mut badges);
        crate::models::chat_layout::order_twitch_badges(&mut badges);
        // A Shared Chat partner row's channel badges are its own room's.
        if let Some(room) = crate::services::shared_chat_emotes::partner_room(
            tag_map.get("source-room-id").copied(),
            tag_map.get("room-id").copied(),
        ) {
            crate::services::shared_chat_emotes::wear_room_badges(room, &mut badges);
        }

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
                                let url = native_emote_url(id);
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
            let partner = crate::services::shared_chat_emotes::partner_room(
                tag_map.get("source-room-id").copied(),
                tag_map.get("room-id").copied(),
            )
            .and_then(|room| crate::services::shared_chat_emotes::lookup_for(&usernotice_channel, room, raw));
            let snapshots = Self::gather_parse_snapshots(&usernotice_channel, &user_id, partner);
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
            // The backlog holds more than PRIVMSGs, and each command is read
            // as the live path reads it. USERNOTICEs (announcements, subs,
            // raids) have their own parser: the PRIVMSG one reads only text
            // after "PRIVMSG", so an announcement came back with an empty
            // body. A room-mode NOTICE is the "Twitch" row live chat shows.
            // CLEARMSG / CLEARCHAT remove what they moderated. ROOMSTATE and
            // anything else describe state, not rows (the JOIN brings the
            // current state), and were showing as empty rows named "unknown".
            let parsed = match irc_command(&raw) {
                Some("PRIVMSG") => Self::parse_privmsg(&raw),
                Some("USERNOTICE") => Self::parse_usernotice(&raw).filter(usernotice_has_visible_content),
                Some("NOTICE") => history_notice_row(&raw),
                Some("CLEARMSG") | Some("CLEARCHAT") => {
                    apply_history_clear(&mut results, &raw);
                    None
                }
                _ => None,
            };
            if let Some(mut chat_msg) = parsed {
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
                crate::services::chat_row_view::stamp(&mut chat_msg);
                results.push(chat_msg);
            }
        }

        results
    }

    /// A backlog parsed before its channel had an emote table (a first visit,
    /// with no saved dictionary yet) shows the channel's 7TV/BTTV/FFZ emotes
    /// as plain words, and a row parsed as text stays text. When the table
    /// lands, the same lines are parsed again and the rows whose segments
    /// changed go to every window showing the channel as one
    /// `HISTORY_SEGMENTS` frame, which patches them in place. The backlog
    /// itself never waits on the providers.
    /// Rows from a Shared Chat partner parsed before its set landed, parsed
    /// again with it; the ones whose emotes change are patched in place in each
    /// home chat (`HISTORY_SEGMENTS`, segments and view), so nothing on screen
    /// moves and nothing waited for the set.
    pub(crate) async fn restamp_partner_rows(room_id: &str, partner: Option<&EmoteLookup>, rows: Vec<(String, String)>) {
        let mut by_home: HashMap<String, Vec<Value>> = HashMap::new();
        for (home, raw) in rows {
            let home_lookup = channel_parse_lookup().read().ok().and_then(|g| g.get(&home).cloned());
            let emotes_change = partner.is_some_and(|p| {
                crate::services::shared_chat_emotes::words_change(irc_trailing_text(&raw), p, home_lookup.as_deref())
            });
            if !emotes_change && !crate::services::shared_chat_emotes::badges_change(&raw, room_id) {
                continue;
            }
            let parsed = match irc_command(&raw) {
                Some("PRIVMSG") => Self::parse_privmsg(&raw),
                Some("USERNOTICE") => Self::parse_usernotice(&raw),
                _ => None,
            };
            let Some(mut msg) = parsed else { continue };
            crate::services::chat_row_view::stamp(&mut msg);
            let mut row = json!({
                "id": msg.id,
                "segments": msg.segments,
                "parent_segments": msg.metadata.reply_info.as_ref().and_then(|r| r.parent_segments.as_ref()),
                "badges": msg.badges,
            });
            if let Some(view) = &msg.metadata.view {
                row["view"] = json!(view);
            }
            by_home.entry(home).or_default().push(row);
        }
        for (home, rows) in by_home {
            debug!("[SharedChat] {home}: {} partner rows now wear their room's emotes and badges", rows.len());
            let frame = json!({ "type": "HISTORY_SEGMENTS", "channel": home, "rows": rows });
            send_to_bridge(BridgeFrame::channel(&home, frame.to_string()), false).await;
        }
    }

    #[cfg(test)]
    pub(crate) fn install_parse_lookup_for_test(key: &str, set: &EmoteSet) {
        rebuild_parse_lookup(key, set);
    }

    pub fn spawn_history_restamp(channel: &str, raw: Vec<String>, first: &[ChatMessage]) {
        if raw.is_empty() || first.is_empty() {
            return;
        }
        let key = channel.trim().trim_start_matches('#').to_lowercase();
        let before: HashMap<String, String> =
            first.iter().map(|m| (m.id.clone(), history_row_signature(m))).collect();
        tokio::spawn(async move {
            if !wait_for_channel_emotes(&key, HISTORY_RESTAMP_WAIT).await {
                debug!("[ChatHistory] {key}: emote table never landed, backlog left as parsed");
                return;
            }
            let again = Self::parse_historical_messages(raw).await;
            let rows = history_segment_updates(&before, &again);
            if rows.is_empty() {
                return;
            }
            debug!("[ChatHistory] {key}: re-stamped {} backlog rows with the channel's emotes", rows.len());
            let frame = json!({ "type": "HISTORY_SEGMENTS", "channel": key, "rows": rows });
            send_to_bridge(BridgeFrame::channel(&key, frame.to_string()), false).await;
        });
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

        // Stop WS server. This stops it accepting; the page sockets it already
        // serves are closed at the end, once everything below is cleared.
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
        crate::services::channel_rewards::forget_all_rooms();
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

        // Last, so a page that reconnects at once finds everything cleared.
        close_page_sockets();
        record_lifecycle("chat service stopped: IRC and the bridge are down, page sockets closed");

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
        crate::services::channel_rewards::forget_all_rooms();
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
        let (tx, _rx) = broadcast::channel::<BridgeFrame>(1000);
        let tx = Arc::new(tx);

        let tx_for_warp = tx.clone();
        let born = bridge_generation();
        let local_ws = warp::ws().map(move |ws: warp::ws::Ws| {
            let tx_clone = tx_for_warp.clone();
            ws.on_upgrade(move |socket| Self::handle_local_ws(socket, tx_clone, born))
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

    /// Serve the bridge on a fresh listener, keeping its broadcaster. iOS can
    /// reclaim a suspended app's listening socket while the server task keeps
    /// running on it, so `ensure_local_ws_bridge` still reports it alive and
    /// nothing answers. The broadcaster stays, so publishers and the IRC
    /// session are untouched; the port is the old one when it is free again,
    /// a new one otherwise, and the shell reconnects to whichever this returns.
    pub async fn rebind_local_ws_bridge() -> Result<u16> {
        let guard = get_bridge_bringup_lock().lock().await;
        let Some(tx) = get_message_broadcaster().lock().await.clone() else {
            drop(guard);
            return Self::ensure_local_ws_bridge().await;
        };
        if let Some(old) = get_ws_server_handle().lock().await.take() {
            old.abort();
        }
        let previous = *get_ws_port().lock().await;
        let tx_for_warp = tx.clone();
        let born = bridge_generation();
        let local_ws = warp::ws().map(move |ws: warp::ws::Ws| {
            let tx_clone = tx_for_warp.clone();
            ws.on_upgrade(move |socket| Self::handle_local_ws(socket, tx_clone, born))
        });
        let probe = match previous {
            Some(p) => match tokio::net::TcpListener::bind(("127.0.0.1", p)).await {
                Ok(listener) => listener,
                Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
            },
            None => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
        };
        let port = probe.local_addr()?.port();
        drop(probe);
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let handle = tokio::spawn(async move {
            warp::serve(local_ws).run(addr).await;
        });
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
        *get_ws_server_handle().lock().await = Some(handle);
        *get_ws_port().lock().await = Some(port);
        record_lifecycle("WS bridge listener re-bound after a resume");
        Ok(port)
    }

    /// The shared broadcast sender for the local WS bridge, if it is up. Provider
    /// adapters serialize a `ChatMessage`/`ActivityEvent` frame and `send` it here
    /// to reach the frontend over the same socket the Twitch path uses.
    pub async fn broadcaster() -> Option<Arc<broadcast::Sender<BridgeFrame>>> {
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

    /// Write one line, as typed, to the primary account's IRC connection
    /// (`/raw`). The caller has already checked it is a single, safe line.
    pub async fn send_raw_line(line: &str) -> Result<()> {
        let writer = get_irc_writer()
            .lock()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Chat is not connected"))?;
        let mut w = writer.lock().await;
        w.send_line(&format!("{}\r\n", line)).await?;
        record_lifecycle(&format!("raw line sent: {}", line.split_whitespace().next().unwrap_or("")));
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

/// A message the user is sending, as the composer knows it at send time.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OwnMessage {
    pub channel: String,
    pub text: String,
    /// The provisional id the row carries until Helix returns the real one.
    pub local_id: String,
    pub sender_id: String,
    pub sender_login: String,
    pub sender_display_name: String,
    #[serde(default)]
    pub color: String,
    /// `name/version,...`, as USERSTATE reported them for this channel.
    #[serde(default)]
    pub badges: String,
    #[serde(default)]
    pub room_id: String,
    #[serde(default)]
    pub reply_to: Option<OwnReplyParent>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct OwnReplyParent {
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub body: String,
}

/// IRCv3 tag-value escaping.
fn escape_tag_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\:"),
            ' ' => out.push_str("\\s"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

/// The CDN art for a Twitch-native emote id, as every received row carries it.
fn native_emote_url(id: &str) -> String {
    format!("https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/3.0", id)
}

/// Each word of `text` that names one of the channel's Twitch emotes, by
/// inclusive codepoint range in text order: the positions Twitch would send in
/// the `emotes` tag. Twitch emotes resolve from positions, never by name.
fn native_emote_positions(text: &str, twitch: &[Emote]) -> Vec<EmotePos> {
    let by_name: HashMap<&str, &str> = twitch.iter().map(|e| (e.name.as_str(), e.id.as_str())).collect();
    let mut out = Vec::new();
    let mut pos = 0usize;
    for word in text.split(' ') {
        let len = word.chars().count();
        if let Some(id) = by_name.get(word).copied() {
            if len > 0 {
                out.push(EmotePos {
                    id: id.to_string(),
                    start: pos,
                    end: pos + len - 1,
                    url: native_emote_url(id),
                    gif: false,
                });
            }
        }
        pos += len + 1;
    }
    out
}

/// The `emotes` tag Twitch would attach to `text`: the native emote positions
/// grouped by emote id in first-seen order. A sent row built without it would
/// show its Twitch emotes as text until the echo arrives.
/// `text` wrapped as the IRC ACTION a `/me` becomes.
fn irc_action(text: &str) -> String {
    format!("\u{1}ACTION {text}\u{1}")
}

fn own_emotes_tag(text: &str, twitch: &[Emote]) -> String {
    let mut order: Vec<&str> = Vec::new();
    let mut ranges: HashMap<&str, Vec<String>> = HashMap::new();
    let positions = native_emote_positions(text, twitch);
    for p in &positions {
        if !ranges.contains_key(p.id.as_str()) {
            order.push(&p.id);
        }
        ranges.entry(&p.id).or_default().push(format!("{}-{}", p.start, p.end));
    }
    order
        .iter()
        .map(|id| format!("{}:{}", id, ranges[id].join(",")))
        .collect::<Vec<_>>()
        .join("/")
}

/// A channel-points redemption that posts nothing to chat on its own (a reward
/// with no text input), shown as a chat row authored by the redeemer whose
/// body is the reward title.
pub struct RedemptionRow {
    /// The channel's bare login.
    pub channel: String,
    /// The channel's Twitch id. It rides the row as `room-id`, which every
    /// renderer reads to draw badges from the channel's own set: without it a
    /// subscriber badge falls back to Twitch's global art (the default star)
    /// instead of the channel's custom one.
    pub room_id: String,
    /// Twitch's redemption id; the row id derives from it so the same
    /// redemption reported twice is one row.
    pub redemption_id: String,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub reward_id: String,
    pub reward_title: String,
    pub reward_cost: i64,
    pub image_url: String,
    pub background_color: String,
    /// The channel's own points icon, when it has one.
    pub points_icon_url: Option<String>,
    /// `name/version,...` the redeemer last showed in this chat.
    pub badges: String,
    pub color: String,
}

/// The row for a redemption, in the shape every received row has. The tags
/// drive the renderers: `custom-reward-id` gives the chat row its redemption
/// highlight, `sn-reward-*` and `sn-points-icon` its reward name, art and
/// cost, and the stream overlay draws any row with `sn-reward-title` as a
/// channel points event.
fn redemption_message(r: &RedemptionRow, segments: Vec<MessageSegment>, timestamp_ms: i64) -> ChatMessage {
    let id = if r.redemption_id.is_empty() {
        format!("redeem-{}", uuid::Uuid::new_v4())
    } else {
        format!("redeem-{}", r.redemption_id)
    };
    let login = if r.user_login.is_empty() { &r.user_name } else { &r.user_login };
    let name = if r.user_name.is_empty() { login } else { &r.user_name };
    let reward_id = if r.reward_id.is_empty() { "sn-redemption" } else { &r.reward_id };

    let mut tags: HashMap<String, String> = HashMap::new();
    tags.insert("user-id".into(), r.user_id.clone());
    if !r.room_id.is_empty() {
        tags.insert("room-id".into(), r.room_id.clone());
    }
    tags.insert("id".into(), id.clone());
    tags.insert("display-name".into(), name.clone());
    tags.insert("custom-reward-id".into(), reward_id.to_string());
    tags.insert("sn-reward-title".into(), r.reward_title.clone());
    if !r.image_url.is_empty() {
        tags.insert("sn-reward-image".into(), r.image_url.clone());
    }
    if !r.background_color.is_empty() {
        tags.insert("sn-reward-bg".into(), r.background_color.clone());
    }
    if r.reward_cost > 0 {
        tags.insert("sn-reward-cost".into(), r.reward_cost.to_string());
    }
    if let Some(icon) = r.points_icon_url.as_deref().filter(|s| !s.is_empty()) {
        tags.insert("sn-points-icon".into(), icon.to_string());
    }

    let mut badges: Vec<Badge> = r
        .badges
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|b| {
            let mut p = b.split('/');
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
    crate::models::chat_layout::order_twitch_badges(&mut badges);

    let timestamp = timestamp_ms.to_string();
    let (formatted_timestamp, formatted_timestamp_with_seconds) =
        IrcService::format_timestamp(&timestamp);

    let mut msg = ChatMessage {
        id,
        user_id: r.user_id.clone(),
        username: login.clone(),
        display_name: name.clone(),
        color: Some(if r.color.is_empty() { "#9147ff".to_string() } else { r.color.clone() }),
        badges,
        timestamp,
        content: r.reward_title.clone(),
        provider: "twitch".to_string(),
        channel: r.channel.clone(),
        emotes: Vec::new(),
        tags,
        layout: LayoutResult::default(),
        segments,
        metadata: MessageMetadata {
            formatted_timestamp,
            formatted_timestamp_with_seconds,
            ..Default::default()
        },
    };
    // Redemption rows skip the rule engine, so they settle their name here.
    crate::services::chatter_name::stamp(&mut msg);
    msg
}

/// The badges a sent row carries: the channel's USERSTATE badges from the
/// connection's cache when the sender is the connected account and the cache
/// has them, else whatever the page passed.
fn own_row_badges(page: &str, cached: Option<&str>, is_primary: bool) -> String {
    match cached {
        Some(c) if is_primary && !c.is_empty() => c.to_string(),
        _ => page.to_string(),
    }
}

fn own_message_line(m: &OwnMessage, emotes_tag: &str, timestamp_ms: i64) -> String {
    let channel = m.channel.trim_start_matches('#').to_lowercase();
    let reply = match &m.reply_to {
        Some(r) if !r.user_id.is_empty() || !r.login.is_empty() => format!(
            "reply-parent-msg-id={};reply-parent-user-id={};reply-parent-user-login={};reply-parent-display-name={};reply-parent-msg-body={};",
            escape_tag_value(&r.id),
            escape_tag_value(&r.user_id),
            escape_tag_value(&r.login),
            escape_tag_value(&r.display_name),
            escape_tag_value(&r.body),
        ),
        Some(r) => format!("reply-parent-msg-id={};", escape_tag_value(&r.id)),
        None => String::new(),
    };
    let login = m.sender_login.to_lowercase();
    format!(
        "@badge-info=;badges={};color={};display-name={};emotes={};first-msg=0;flags=;id={};mod=0;{}returning-chatter=0;room-id={};subscriber=0;tmi-sent-ts={};turbo=0;user-id={};user-type= :{login}!{login}@{login}.tmi.twitch.tv PRIVMSG #{channel} :{}",
        escape_tag_value(&m.badges),
        escape_tag_value(&m.color),
        escape_tag_value(&m.sender_display_name),
        emotes_tag,
        escape_tag_value(&m.local_id),
        reply,
        escape_tag_value(&m.room_id),
        timestamp_ms,
        escape_tag_value(&m.sender_id),
        m.text,
    )
}

impl IrcService {
    /// Whether this Twitch channel is joined (or being joined) on the IRC
    /// connection.
    pub async fn is_joined(login: &str) -> bool {
        get_current_channels().lock().await.contains(&login.to_lowercase())
    }

    /// The row for a message the user is sending, built exactly as a received
    /// one is (segments, emotes, reply info, rule stamps), so the composer
    /// shows it at once and the echo later upgrades it in place by id. Nothing
    /// is recorded: the echo is what goes into history.
    pub async fn build_own_message(mut m: OwnMessage) -> Option<ChatMessage> {
        let key = m.channel.trim_start_matches('#').to_lowercase();
        // A reply to a row still carrying its local id is sent as a plain
        // message (ChatService::send_message), so the row shows none either.
        m.reply_to = m.reply_to.filter(|r| !r.id.starts_with("local-"));
        // The IRC-connected account's badges for this channel, as USERSTATE
        // reported them on JOIN. Messages go out over Helix, which triggers no
        // USERSTATE, so a page that missed the JOIN one (its slice did not exist
        // yet when the replay arrived) passed none and every row it sent stayed
        // bare. The cache here always saw it. A secondary account is not the
        // connected user, so its rows keep what the page passed.
        let is_primary = ChatRules::own_identity()
            .map(|(login, id)| {
                (!id.is_empty() && id == m.sender_id) || login == m.sender_login.to_lowercase()
            })
            .unwrap_or(false);
        let cached = get_user_badges_cache().lock().await.get(&key).cloned();
        m.badges = own_row_badges(&m.badges, cached.as_deref(), is_primary);
        // A `/me` goes out as an IRC ACTION, so the row is one too. Emote
        // positions count from the action's own words, as Twitch's do.
        let action_body = m.text.strip_prefix("/me ").map(str::to_string);
        let emotes_tag = {
            let map = get_channel_emotes().lock().await;
            let body = action_body.as_deref().unwrap_or(&m.text);
            map.get(&key).map(|set| own_emotes_tag(body, &set.twitch)).unwrap_or_default()
        };
        if let Some(body) = action_body {
            m.text = irc_action(&body);
        }
        let line = own_message_line(&m, &emotes_tag, chrono::Utc::now().timestamp_millis());
        let mut msg = Self::parse_privmsg(&line)?;
        msg.layout = LayoutResult {
            height: 60.0,
            width: 0.0,
            has_reply: msg.metadata.reply_info.is_some(),
            is_first_message: false,
        };
        // Stamps only (mentions, highlights); your own message is never dropped,
        // and never recorded: the echo is the record.
        let rules = ChatRules::snapshot();
        let _ = ChatRules::evaluate_unrecorded(&mut msg, &rules);
        crate::services::chat_row_view::stamp(&mut msg);
        Some(msg)
    }

    /// A line sent over this IRC connection gets no echo, so it goes into the
    /// history ring from here (`/logs`, search). A Helix send is recorded when
    /// its echo arrives, and the composer's own copy never is.
    async fn record_own_irc_line(channel: &str, message: &str) {
        let Some((login, user_id)) = get_own_identity().lock().await.clone() else {
            return;
        };
        let key = channel.trim_start_matches('#').to_lowercase();
        let badges = get_user_badges_cache().lock().await.get(&key).cloned().unwrap_or_default();
        let text = match message.strip_prefix("/me ") {
            Some(rest) => irc_action(rest),
            None => message.to_string(),
        };
        let own = OwnMessage {
            channel: key,
            text,
            local_id: format!("irc-{}", uuid::Uuid::new_v4().simple()),
            sender_id: user_id,
            sender_login: login.clone(),
            sender_display_name: login,
            color: String::new(),
            badges,
            room_id: String::new(),
            reply_to: None,
        };
        let line = own_message_line(&own, "", chrono::Utc::now().timestamp_millis());
        if let Some(mut msg) = Self::parse_privmsg(&line) {
            let rules = ChatRules::snapshot();
            let _ = ChatRules::evaluate(&mut msg, &rules);
        }
    }

    /// Deliver a redemption row to the windows holding its channel, over the
    /// bridge every received row takes, so it is routed, held for a parked
    /// chat and deduped by id like any other. The title tokenizes against the
    /// channel's emote table here, which a hidden chat keeps while its page
    /// lets its own copy go. Nothing is delivered for a chat that is not open.
    pub async fn publish_redemption_row(mut r: RedemptionRow) {
        let key = r.channel.trim_start_matches('#').to_lowercase();
        if key.is_empty() || r.reward_title.is_empty() || !Self::is_joined(&key).await {
            return;
        }
        r.channel = key;
        // Your own redemption before you have chatted here has no remembered
        // look; wear the badges USERSTATE reported for the channel, which the
        // page also takes from any row you author.
        if r.badges.is_empty() {
            let is_primary = ChatRules::own_identity()
                .map(|(_, id)| !id.is_empty() && id == r.user_id)
                .unwrap_or(false);
            if is_primary {
                if let Some(cached) = get_user_badges_cache().lock().await.get(&r.channel) {
                    r.badges = cached.clone();
                }
            }
        }
        let natives = {
            let map = get_channel_emotes().lock().await;
            map.get(&r.channel)
                .map(|set| native_emote_positions(&r.reward_title, &set.twitch))
                .unwrap_or_default()
        };
        // The broadcaster wrote the title, so the redeemer's personal emotes
        // and cheermotes have no part in it.
        let snapshots = Self::gather_parse_snapshots(&r.channel, "", None);
        let ctx = ParseCtx {
            channel: snapshots.channel.as_deref(),
            no_cheermotes: true,
            ..Default::default()
        };
        let segments = Self::parse_message_segments(&r.reward_title, &natives, &ctx);
        let msg = redemption_message(&r, segments, chrono::Utc::now().timestamp_millis());
        if let Ok(json_msg) = serde_json::to_string(&msg) {
            send_to_bridge(BridgeFrame::row(&msg.channel, json_msg, false), true).await;
        }
    }
}

#[cfg(test)]
mod own_message_tests {
    use super::*;

    fn twitch_emote(id: &str, name: &str) -> Emote {
        serde_json::from_value(json!({ "id": id, "name": name, "url": "", "provider": "twitch" }))
            .expect("minimal emote")
    }

    fn own(text: &str, reply_to: Option<OwnReplyParent>) -> OwnMessage {
        OwnMessage {
            channel: "#Chan".into(),
            text: text.into(),
            local_id: "local-1".into(),
            sender_id: "42".into(),
            sender_login: "Me".into(),
            sender_display_name: "Me Me".into(),
            color: "#ff0000".into(),
            badges: "subscriber/12".into(),
            room_id: "7".into(),
            reply_to,
        }
    }

    #[test]
    fn a_sent_row_takes_the_cached_channel_badges_for_the_connected_account() {
        // The page missed USERSTATE and passed nothing: the cache fills it.
        assert_eq!(own_row_badges("", Some("subscriber/3012,premium/1"), true), "subscriber/3012,premium/1");
        // The cache is newer than what the page held.
        assert_eq!(own_row_badges("premium/1", Some("subscriber/3012,premium/1"), true), "subscriber/3012,premium/1");
        // Nothing cached yet: keep the page's value.
        assert_eq!(own_row_badges("vip/1", None, true), "vip/1");
        assert_eq!(own_row_badges("vip/1", Some(""), true), "vip/1");
        // A secondary account is not the connected user: never the cache.
        assert_eq!(own_row_badges("", Some("subscriber/3012"), false), "");
    }

    #[test]
    fn emote_tags_match_what_twitch_would_send() {
        let set = vec![twitch_emote("25", "Kappa"), twitch_emote("88", "PogChamp")];
        assert_eq!(own_emotes_tag("Kappa hi Kappa PogChamp", &set), "25:0-4,9-13/88:15-22");
        assert_eq!(own_emotes_tag("héllo Kappa", &set), "25:6-10");
        assert_eq!(own_emotes_tag("no emotes here", &set), "");
    }

    #[test]
    fn an_own_reply_carries_the_senders_id_not_the_parents() {
        let parent = OwnReplyParent {
            id: "p1".into(),
            user_id: "99".into(),
            login: "them".into(),
            display_name: "Them".into(),
            body: "hello there; friend".into(),
        };
        let line = own_message_line(&own("@them hi", Some(parent)), "", 1000);
        let msg = IrcService::parse_privmsg(&line).expect("parses");
        assert_eq!(msg.user_id, "42");
        assert_eq!(msg.id, "local-1");
        // The redundant leading @mention is stripped, as on the echo.
        assert_eq!(msg.content, "hi");
        let reply = msg.metadata.reply_info.expect("reply info");
        assert_eq!(reply.parent_user_id, "99");
    }

    #[test]
    fn tag_values_are_escaped() {
        assert_eq!(escape_tag_value("a b;c\\d"), "a\\sb\\:c\\\\d");
        let line = own_message_line(&own("hi", None), "", 1000);
        assert!(line.contains("display-name=Me\\sMe;"));
        assert!(line.ends_with("PRIVMSG #chan :hi"));
    }

    const LOCALIZED_PRIVMSG: &str = "@badge-info=;badges=subscriber/3;color=#FF69B4;display-name=もちパンダ;emotes=;id=a1;reply-parent-msg-body=wow!\\sok;room-id=1;tmi-sent-ts=1000;user-id=700700 :mochi_panda!mochi_panda@mochi_panda.tmi.twitch.tv PRIVMSG #chan :こんにちは";
    const LOGIN_ONLY_WATCH_STREAK: &str = "@badge-info=;badges=subscriber/3;color=#FF69B4;display-name=mochi_panda;emotes=;id=a2;login=mochi_panda;msg-id=viewermilestone;msg-param-category=watch-streak;msg-param-copoReward=350;msg-param-value=3;room-id=1;system-msg=mochi_panda\\swatched\\s3\\sconsecutive\\sstreams\\sand\\ssparked\\sa\\swatch\\sstreak!;tmi-sent-ts=2000;user-id=700700 :tmi.twitch.tv USERNOTICE #chan :yay";

    #[test]
    fn a_text_reward_message_is_named_from_the_channels_rewards() {
        crate::services::channel_rewards::note_reward_name("rewardnamechan", "rw-1", "Билет на Розыгрыш Ножа", 4999);
        let line = |reward: &str| {
            format!("@badges=;color=;display-name=Bob;emotes=;id=r{reward};custom-reward-id={reward};room-id=1;tmi-sent-ts=1;user-id=2 :bob!bob@bob.tmi.twitch.tv PRIVMSG #rewardnamechan :https://example.com/trade")
        };
        let named = IrcService::parse_privmsg(&line("rw-1")).expect("parses");
        assert_eq!(named.tags["sn-redeemed-title"], "Билет на Розыгрыш Ножа");
        assert_eq!(named.tags["sn-redeemed-cost"], "4999");
        assert!(!named.tags.contains_key("sn-reward-title"), "not drawn as a no-text redemption");
        let unknown = IrcService::parse_privmsg(&line("rw-unknown")).expect("parses");
        assert!(!unknown.tags.contains_key("sn-redeemed-title"));
        let plain = IrcService::parse_privmsg(LOCALIZED_PRIVMSG).expect("parses");
        assert!(!plain.tags.contains_key("sn-redeemed-title"));
    }

    #[test]
    fn a_privmsg_username_is_the_login_not_the_display_name() {
        let msg = IrcService::parse_privmsg(LOCALIZED_PRIVMSG).expect("parses");
        assert_eq!(msg.username, "mochi_panda");
        assert_eq!(msg.display_name, "もちパンダ");
        assert_eq!(IrcService::privmsg_prefix_login(":tmi.twitch.tv USERNOTICE #c"), None);
    }

    #[test]
    fn a_localized_chatter_shows_one_name_on_message_and_watch_streak_rows() {
        let rules = ChatRules::snapshot();
        let mut privmsg = IrcService::parse_privmsg(LOCALIZED_PRIVMSG).expect("parses");
        let _ = ChatRules::evaluate(&mut privmsg, &rules);
        let mut streak = IrcService::parse_usernotice(LOGIN_ONLY_WATCH_STREAK).expect("parses");
        let _ = ChatRules::evaluate(&mut streak, &rules);
        assert_eq!(privmsg.metadata.shown_name, "もちパンダ (mochi_panda)");
        assert_eq!(streak.metadata.shown_name, privmsg.metadata.shown_name);
        assert_eq!(streak.username, privmsg.username);
    }

    fn redemption(title: &str) -> RedemptionRow {
        RedemptionRow {
            channel: "chan".into(),
            room_id: "7".into(),
            redemption_id: "r-1".into(),
            user_id: "42".into(),
            user_login: "viewer".into(),
            user_name: "Viewer".into(),
            reward_id: "reward-9".into(),
            reward_title: title.into(),
            reward_cost: 5000,
            image_url: "https://img/4x.png".into(),
            background_color: "#00C7AC".into(),
            points_icon_url: Some("https://img/points.png".into()),
            badges: "vip/1,subscriber/12".into(),
            color: String::new(),
        }
    }

    #[test]
    fn a_redemption_title_tokenizes_against_the_channel_table() {
        let mut set = EmoteSet::new();
        set.twitch.push(twitch_emote("25", "Kappa"));
        set.seven_tv.push(
            serde_json::from_value(json!({ "id": "7a", "name": "COPIUM", "url": "https://7tv/1x", "provider": "7tv" }))
                .expect("minimal emote"),
        );
        let lookup = EmoteLookup::build(&set);
        let ctx = ParseCtx { channel: Some(&lookup), no_cheermotes: true, ..Default::default() };
        let title = "COPIUM Kappa copium Cheer100";
        let natives = native_emote_positions(title, &set.twitch);
        let segs = IrcService::parse_message_segments(title, &natives, &ctx);
        let kinds: Vec<String> = segs
            .iter()
            .map(|s| match s {
                MessageSegment::Emote { content, emote_id, .. } => format!("emote:{content}:{}", emote_id.as_deref().unwrap_or("")),
                MessageSegment::Text { content } => format!("text:{content}"),
                other => format!("{other:?}"),
            })
            .collect();
        // Exact case, as in chat; and no word becomes bits.
        assert_eq!(
            kinds,
            ["emote:COPIUM:7a", "text: ", "emote:Kappa:25", "text: ", "text:copium", "text: ", "text:Cheer100"]
        );
    }

    #[test]
    fn a_redemption_row_carries_the_reward_tags_and_a_stable_id() {
        let msg = redemption_message(&redemption("Hydrate"), Vec::new(), 1000);
        assert_eq!(msg.id, "redeem-r-1");
        assert_eq!(msg.tags["id"], "redeem-r-1");
        assert_eq!(msg.tags["custom-reward-id"], "reward-9");
        assert_eq!(msg.tags["sn-reward-title"], "Hydrate");
        assert_eq!(msg.tags["sn-reward-image"], "https://img/4x.png");
        assert_eq!(msg.tags["sn-reward-bg"], "#00C7AC");
        assert_eq!(msg.tags["sn-reward-cost"], "5000");
        assert_eq!(msg.tags["sn-points-icon"], "https://img/points.png");
        assert_eq!(msg.username, "viewer");
        assert_eq!(msg.display_name, "Viewer");
        assert_eq!(msg.content, "Hydrate");
        assert_eq!(msg.color.as_deref(), Some("#9147ff"));
        assert_eq!(msg.provider, "twitch");
        let badges: Vec<&str> = msg.badges.iter().map(|b| b.name.as_str()).collect();
        assert!(badges.contains(&"vip") && badges.contains(&"subscriber"));

        let mut bare = redemption("Hydrate");
        bare.room_id.clear();
        assert!(!redemption_message(&bare, Vec::new(), 1000).tags.contains_key("room-id"));
        let mut bare = redemption("Hydrate");
        bare.reward_id.clear();
        bare.reward_cost = 0;
        bare.points_icon_url = None;
        bare.image_url.clear();
        let msg = redemption_message(&bare, Vec::new(), 1000);
        assert_eq!(msg.tags["custom-reward-id"], "sn-redemption");
        for absent in ["sn-reward-cost", "sn-points-icon", "sn-reward-image"] {
            assert!(!msg.tags.contains_key(absent), "{absent}");
        }
    }

    #[test]
    fn a_redemption_row_names_its_channel_so_badges_resolve_against_the_channel_set() {
        // The renderers look a Twitch badge up in the set of the row's
        // `room-id`; a row without one draws a subscriber badge from the global
        // set, the default star, instead of the channel's custom art.
        let msg = redemption_message(&redemption("Hydrate"), Vec::new(), 1000);
        assert_eq!(msg.tags.get("room-id").map(String::as_str), Some("7"));
        let sub = msg.badges.iter().find(|b| b.name == "subscriber").expect("subscriber badge");
        assert_eq!(sub.version, "12");
        // The page reads the row's tags as a plain object, so the id must survive JSON.
        let json: Value = serde_json::to_value(&msg).expect("serializes");
        assert_eq!(json["tags"]["room-id"], "7");
    }
}

#[cfg(test)]
mod personal_emote_tests {
    use super::*;

    fn emote(id: &str, name: &str) -> Emote {
        Emote {
            id: id.to_string(),
            name: name.to_string(),
            url: format!("https://cdn.7tv.app/emote/{id}/1x.avif"),
            provider: crate::services::emote_service::EmoteProvider::SevenTV,
            is_zero_width: Some(false),
            local_url: None,
            emote_type: None,
            owner_id: None,
            owner_name: None,
            original_name: None,
            owner_ref: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
        }
    }

    fn text(s: &str) -> MessageSegment {
        MessageSegment::Text { content: s.to_string() }
    }

    #[test]
    fn a_quoted_parent_gets_the_channels_emotes() {
        // A reply's parent arrives as plain text with no positions: its Twitch
        // emotes are matched by name against the channel's own, its 7TV words
        // against the channel's set, like the parent's own row.
        let mut native = emote("25", "Kappa");
        native.provider = crate::services::emote_service::EmoteProvider::Twitch;
        let mut set = EmoteSet::new();
        set.twitch = vec![native];
        set.seven_tv = vec![emote("7a", "catJAM")];
        let snapshots = ParseSnapshots { channel: Some(EmoteLookup::build(&set)), personal: None, cheermotes: None };
        let segs = IrcService::quoted_segments("hi Kappa catJAM", &snapshots);
        let emotes: Vec<(&str, Option<&str>)> = segs
            .iter()
            .filter_map(|s| match s {
                MessageSegment::Emote { content, emote_id, .. } => Some((content.as_str(), emote_id.as_deref())),
                _ => None,
            })
            .collect();
        assert_eq!(emotes, vec![("Kappa", Some("25")), ("catJAM", Some("7a"))]);
        // No channel table yet: the quote reads as words, never as nothing.
        let empty = ParseSnapshots { channel: None, personal: None, cheermotes: None };
        assert!(IrcService::quoted_segments("hi Kappa", &empty)
            .iter()
            .all(|s| matches!(s, MessageSegment::Text { .. })));
    }

    #[test]
    fn only_rows_naming_a_personal_emote_are_repainted() {
        let set: HashMap<String, Emote> = [("cuh".to_string(), emote("e1", "cuh"))].into_iter().collect();
        let rows = vec![
            ("xqc".to_string(), "m1".to_string(), "hello cuh".to_string()),
            ("xqc".to_string(), "m2".to_string(), "cuhh not it".to_string()),
            ("forsen".to_string(), "m3".to_string(), "cuh".to_string()),
            // Another platform's row: its user id is not a Twitch id.
            ("kick:xqc".to_string(), "m4".to_string(), "cuh".to_string()),
        ];
        let named = IrcService::rows_naming_personal_emotes(rows, &set);
        assert_eq!(named.get("xqc"), Some(&vec!["m1".to_string()]));
        assert_eq!(named.get("forsen"), Some(&vec!["m3".to_string()]));
        assert!(named.get("kick:xqc").is_none());
    }

    #[test]
    fn applying_a_personal_set_swaps_exact_words_only() {
        let owner = "personal-test-owner";
        let set: HashMap<String, Emote> = [("cuh".to_string(), emote("e1", "cuh"))].into_iter().collect();
        get_personal_emotes().write().unwrap().put(owner.to_string(), ("set".to_string(), Arc::new(set)));

        let segments = vec![text("hello"), text(" "), text("cuh"), text(" "), text("cuhh")];
        let painted = IrcService::apply_personal_emotes(owner, &segments).expect("one word changes");
        match &painted[2] {
            MessageSegment::Emote { content, emote_id, is_personal, .. } => {
                assert_eq!(content, "cuh");
                assert_eq!(emote_id.as_deref(), Some("e1"));
                assert_eq!(*is_personal, Some(true));
            }
            other => panic!("expected an emote, got {other:?}"),
        }
        assert!(matches!(&painted[4], MessageSegment::Text { content } if content == "cuhh"));

        // Nothing to change, or nobody's set: no repaint.
        assert!(IrcService::apply_personal_emotes(owner, &[text("hello")]).is_none());
        assert!(IrcService::apply_personal_emotes("someone-else", &segments).is_none());
        get_personal_emotes().write().unwrap().pop(owner);
    }
}

#[cfg(test)]
mod join_window_tests {
    use super::{JoinWindow, JOIN_WINDOW_MAX, JOIN_WINDOW_MS};

    #[test]
    fn a_full_window_waits_for_its_oldest_join() {
        let mut w = JoinWindow::default();
        for i in 0..JOIN_WINDOW_MAX as u64 {
            assert!(w.claim(1_000 + i).is_ok());
        }
        assert_eq!(w.claim(2_000), Err(1_000 + JOIN_WINDOW_MS - 2_000));
        assert!(w.claim(1_000 + JOIN_WINDOW_MS).is_ok(), "the oldest has aged out");
    }

    #[test]
    fn self_paced_joins_count_too() {
        let mut w = JoinWindow::default();
        w.note(0, 15);
        for _ in 0..(JOIN_WINDOW_MAX - 15) {
            assert!(w.claim(10).is_ok());
        }
        assert!(w.claim(10).is_err());
    }
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
        let ctx = ParseCtx::default();
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
    fn silent_session_end_needs_real_silence_while_awake() {
        // A network that cuts the connection mid-session: it ends by our read
        // timeout (or a server FIN) about 30 s after the last read.
        let timeout = "read liveness timeout";
        assert!(is_silent_session_end(timeout, 30_000, 30_200));
        assert!(is_silent_session_end("closed by server", 25_000, 25_000));
        assert!(is_silent_session_end("read error", 20_000, 20_000));
        // A healthy idle link never gets this far without an answered PING.
        assert!(!is_silent_session_end("closed by server", 12_000, 12_000));
        // Twitch asked us to reconnect, or our own handler stalled: not the
        // transport's fault however long the silence.
        assert!(!is_silent_session_end("server RECONNECT", 30_000, 30_000));
        assert!(!is_silent_session_end("handler stall", 30_000, 30_000));
        // The timer fired late: the process was frozen or the machine slept.
        assert!(!is_silent_session_end(timeout, 600_000, 600_000));
        // The wall clock ran ahead of the monotonic one: a sleep the monotonic
        // clock did not count.
        assert!(!is_silent_session_end("read error", 21_000, 900_000));
    }

    #[test]
    fn liveness_constants_keep_their_order() {
        let timeout = IRC_READ_TIMEOUT.as_millis() as u64;
        // An idle PING goes out (worst case one check tick late) with time
        // left for the PONG before the read deadline.
        assert!(IRC_PING_IDLE_MS + IRC_PING_CHECK.as_millis() as u64 + 5_000 <= timeout);
        // A session counted silent has missed at least one PING.
        assert!(STALL_READ_AGE_MS > IRC_PING_IDLE_MS && STALL_READ_AGE_MS < timeout);
        // The heartbeat must never vouch past the read deadline.
        assert!(HEARTBEAT_SUPPRESS_AFTER_MS > timeout);
    }

    #[test]
    fn deafen_fault_spec_parses() {
        use IrcTransport::{Irc, WebSocket as Ws};
        let s = std::time::Duration::from_secs;
        assert_eq!(parse_deafen_fault("deafen:40", Irc), Some(s(40)));
        assert_eq!(parse_deafen_fault("deafen:40:websocket", Ws), Some(s(40)));
        assert_eq!(parse_deafen_fault("deafen:40:websocket", Irc), None);
        assert_eq!(parse_deafen_fault("deafen:soon", Irc), None);
        assert_eq!(parse_deafen_fault("mute:40", Irc), None);
        assert_eq!(parse_deafen_fault("", Irc), None);
        assert_eq!(parse_deafen_fault("deafen:40:irc", Irc), Some(s(40)));
    }

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
        // "a  b" -> a, one spacer holding both spaces, b
        assert_eq!(segs.len(), 3);
        assert!(matches!(&segs[1], MessageSegment::Text { content } if content == "  "));
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
    fn emote_lookup_priority() {
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
            original_name: None,
            owner_ref: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
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
        assert_eq!(lookup.get("BttvOnly").unwrap().id, "b2");
        assert!(lookup.get("Missing").is_none());
    }

    fn tiers(spec: &[(u32, &str)]) -> Vec<CheermoteTier> {
        spec.iter()
            .map(|(min_bits, color)| CheermoteTier {
                min_bits: *min_bits,
                color: color.to_string(),
                url: format!("https://example.test/{}.gif", min_bits),
                ..Default::default()
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
                    ..Default::default()
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

/// A span Twitch tagged as an emote is Twitch's emote, whatever a third-party
/// set calls the same name; third-party names only match untagged words.
#[cfg(test)]
mod native_emote_priority_tests {
    use super::*;

    const XQCL: &str = "1035663";
    /// A 7TV upload named `xqcL` (someone else's art under the sub emote's name).
    const SEVENTV_XQCL: &str = "01F1136BTG0008S1RA006Y72TN";

    /// A real Fossabot line from xQc's chat, tags trimmed to the ones parsing
    /// reads. Its `emotes` tag marks all four `xqcL` as Twitch emote 1035663.
    const FOSSABOT_XQCL: &str = "@badges=moderator/1,subscriber/3090,bot-badge/1;color=#1976D2;display-name=Fossabot;emotes=1035663:0-3,57-60,92-95,135-138;id=420c56bf-3fca-4bcd-949f-06227180a962;room-id=71092938;tmi-sent-ts=1791703315881;user-id=237719657 :fossabot!fossabot@fossabot.tmi.twitch.tv PRIVMSG #{CHAN} :xqcL boxtoasterpog is staying in the jungle using Prime! xqcL Thanks so much for re-subbing xqcL And continuing to support the stream! xqcL";

    fn seventv(id: &str, name: &str, zero_width: bool) -> Emote {
        serde_json::from_value(json!({
            "id": id,
            "name": name,
            "url": format!("https://cdn.7tv.app/emote/{id}/1x.avif"),
            "provider": "7tv",
            "is_zero_width": zero_width,
        }))
        .expect("minimal emote")
    }

    /// A channel whose 7TV set holds an `xqcL` and a zero-width `RainTime`,
    /// registered under its own key so no other test's channel is touched.
    fn channel_with_seventv_xqcl(chan: &str) {
        let mut set = EmoteSet::new();
        set.seven_tv.push(seventv(SEVENTV_XQCL, "xqcL", false));
        set.seven_tv.push(seventv("01GB2S3ZQ8000BNRXQQ0VKQ0E3", "RainTime", true));
        rebuild_parse_lookup(chan, &set);
    }

    fn line(chan: &str, tags: &str, body: &str) -> String {
        format!("@{tags};id=x;room-id=71092938;tmi-sent-ts=1;user-id=237719657 :fossabot!fossabot@fossabot.tmi.twitch.tv PRIVMSG #{chan} :{body}")
    }

    /// Emote segments as (text, id, zero-width).
    fn emotes(segments: &[MessageSegment]) -> Vec<(String, String, bool)> {
        segments
            .iter()
            .filter_map(|s| match s {
                MessageSegment::Emote { content, emote_id, is_zero_width, .. } => Some((
                    content.clone(),
                    emote_id.clone().unwrap_or_default(),
                    is_zero_width.unwrap_or(false),
                )),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn twitch_history_rows_get_twitch_and_seventv_emotes() {
        let chan = "twitch_history_7tv";
        channel_with_seventv_xqcl(chan);
        let answer = serde_json::json!({ "data": { "channel": { "id": "71092938", "recentChatMessages": [{
            "id": "h1", "deletedAt": null, "sentAt": "2026-10-13T17:45:08Z",
            "content": { "text": "xqcL RainTime gg", "fragments": [
                { "text": "xqcL", "content": { "emoteID": "1035663" } },
                { "text": " RainTime gg", "content": null }
            ] },
            "parentMessage": null,
            "sender": { "id": "237719657", "login": "fossabot", "displayName": "Fossabot" },
            "senderBadges": [], "senderChatColor": null, "sourceChannel": null
        }] } } });
        let lines = crate::services::twitch_chat_history::lines_from(&answer, chan).unwrap();
        let rows = IrcService::parse_historical_messages(lines).await;
        assert_eq!(rows.len(), 1);
        let found = emotes(&rows[0].segments);
        assert_eq!(found.len(), 2, "{:?}", rows[0].segments);
        assert_eq!(found[0], ("xqcL".to_string(), "1035663".to_string(), false), "Twitch's emote from its fragment");
        assert_eq!(found[1].0, "RainTime", "the 7TV word from the channel's table");
    }

    #[test]
    fn a_tagged_sub_emote_beats_a_same_name_seventv_channel_emote() {
        let chan = "native_priority_fossabot";
        channel_with_seventv_xqcl(chan);
        let msg = IrcService::parse_privmsg(&FOSSABOT_XQCL.replace("{CHAN}", chan)).expect("parses");
        let found = emotes(&msg.segments);
        assert_eq!(found.len(), 4, "{:?}", msg.segments);
        for (content, id, _) in &found {
            assert_eq!(content, "xqcL");
            assert_eq!(id, XQCL);
        }
        for seg in &msg.segments {
            if let MessageSegment::Emote { emote_url, .. } = seg {
                assert!(emote_url.contains("static-cdn.jtvnw.net/emoticons/v2/1035663/"), "{emote_url}");
            }
        }
    }

    #[test]
    fn an_untagged_word_still_matches_seventv() {
        let chan = "native_priority_untagged";
        channel_with_seventv_xqcl(chan);
        // Only the first xqcL is tagged; a sender without the sub types the second.
        let msg = IrcService::parse_privmsg(&line(chan, "emotes=1035663:0-3", "xqcL hi xqcL")).expect("parses");
        assert_eq!(
            emotes(&msg.segments),
            [("xqcL".to_string(), XQCL.to_string(), false), ("xqcL".to_string(), SEVENTV_XQCL.to_string(), false)]
        );
        // No tag at all: the word is the 7TV emote.
        let msg = IrcService::parse_privmsg(&line(chan, "emotes=", "xqcL")).expect("parses");
        assert_eq!(emotes(&msg.segments), [("xqcL".to_string(), SEVENTV_XQCL.to_string(), false)]);
    }

    #[test]
    fn multibyte_text_before_the_emote_keeps_the_codepoint_offsets() {
        let chan = "native_priority_multibyte";
        channel_with_seventv_xqcl(chan);
        // Twitch counts code points: "héllo" is 5, the emoji 1 (2 in UTF-16,
        // 4 in UTF-8), "日本" 2, plus three spaces, so xqcL sits at 11-14.
        let body = "héllo 😀 日本 xqcL ok";
        assert_eq!(body.chars().position(|c| c == 'x'), Some(11));
        let msg = IrcService::parse_privmsg(&line(chan, "emotes=1035663:11-14", body)).expect("parses");
        assert_eq!(emotes(&msg.segments), [("xqcL".to_string(), XQCL.to_string(), false)]);
        let text: String = msg
            .segments
            .iter()
            .map(|s| match s {
                MessageSegment::Text { content } => content.clone(),
                MessageSegment::Emoji { content, .. } => content.clone(),
                MessageSegment::Emote { content, .. } => content.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(text, body);
    }

    #[test]
    fn a_zero_width_seventv_emote_still_overlays_a_native_emote() {
        let chan = "native_priority_zero_width";
        channel_with_seventv_xqcl(chan);
        for body in ["xqcL RainTime", "xqcL  RainTime"] {
            let msg = IrcService::parse_privmsg(&line(chan, "emotes=1035663:0-3", body)).expect("parses");
            let segs = &msg.segments;
            assert_eq!(segs.len(), 3, "{body:?}: {segs:?}");
            assert!(matches!(&segs[0], MessageSegment::Emote { emote_id: Some(id), .. } if id == XQCL));
            // One spacer between them, which is what the renderer looks back past.
            assert!(matches!(&segs[1], MessageSegment::Text { content } if content.trim().is_empty()));
            assert!(matches!(
                &segs[2],
                MessageSegment::Emote { content, is_zero_width: Some(true), .. } if content == "RainTime"
            ));
        }
    }
}

#[cfg(test)]
mod history_usernotice_tests {
    use super::*;

    // Lines as the recent-messages mirror returns them (tags trimmed).
    const ANNOUNCEMENT: &str = "@badge-info=;badges=moderator/1;color=#1E90FF;display-name=ModUser;emotes=;flags=;historical=1;id=hist-ann-1;login=moduser;mod=1;msg-id=announcement;msg-param-color=PRIMARY;rm-received-ts=1700000000100;room-id=11;subscriber=0;system-msg=;tmi-sent-ts=1700000000000;user-id=42;user-type=mod :tmi.twitch.tv USERNOTICE #chan :drops are live now";
    const RESUB: &str = "@badge-info=subscriber/14;badges=subscriber/12;color=;display-name=Fan;emotes=;flags=;historical=1;id=hist-resub-1;login=fan;msg-id=resub;msg-param-cumulative-months=14;msg-param-sub-plan=1000;room-id=11;system-msg=Fan\\ssubscribed\\sat\\sTier\\s1.\\sThey've\\ssubscribed\\sfor\\s14\\smonths!;tmi-sent-ts=1700000001000;user-id=43 :tmi.twitch.tv USERNOTICE #chan :still here";
    const RAID: &str = "@badge-info=;badges=;color=;display-name=Raider;emotes=;flags=;historical=1;id=hist-raid-1;login=raider;msg-id=raid;msg-param-viewerCount=25;room-id=11;system-msg=25\\sraiders\\sfrom\\sRaider\\shave\\sjoined!;tmi-sent-ts=1700000002000;user-id=44 :tmi.twitch.tv USERNOTICE #chan";
    const BLANK: &str = "@badge-info=;badges=;display-name=Ghost;emotes=;historical=1;id=hist-blank-1;login=ghost;msg-id=onetapgiftredeemed;room-id=11;tmi-sent-ts=1700000003000;user-id=45 :tmi.twitch.tv USERNOTICE #chan";
    const PRIVMSG: &str = "@badge-info=;badges=;color=;display-name=Bob;emotes=;historical=1;id=hist-msg-1;room-id=11;tmi-sent-ts=1700000004000;user-id=46 :bob!bob@bob.tmi.twitch.tv PRIVMSG #chan :USERNOTICE is just a word here";

    #[test]
    fn the_command_is_read_by_token() {
        assert_eq!(irc_command(ANNOUNCEMENT), Some("USERNOTICE"));
        assert_eq!(irc_command(RAID), Some("USERNOTICE"));
        assert_eq!(irc_command(PRIVMSG), Some("PRIVMSG"));
        assert_eq!(irc_command(":tmi.twitch.tv CLEARCHAT #chan"), Some("CLEARCHAT"));
        assert_eq!(irc_command("PING :tmi.twitch.tv"), Some("PING"));
        assert_eq!(irc_command(""), None);
    }

    #[tokio::test]
    async fn history_usernotices_keep_their_text_like_live_ones() {
        let rows = IrcService::parse_historical_messages(
            [ANNOUNCEMENT, RESUB, RAID, BLANK, PRIVMSG].iter().map(|s| s.to_string()).collect(),
        )
        .await;
        let row = |id: &str| rows.iter().find(|m| m.id == id).unwrap_or_else(|| panic!("{id} missing"));

        let ann = row("hist-ann-1");
        assert_eq!(ann.content, "drops are live now", "the announcement keeps its body");
        assert_eq!(ann.metadata.msg_type.as_deref(), Some("announcement"));
        assert_eq!(ann.username, "moduser");
        assert!(ann.metadata.from_backfill);

        let resub = row("hist-resub-1");
        assert_eq!(resub.content, "still here");
        assert_eq!(resub.metadata.msg_type.as_deref(), Some("resub"));
        assert!(resub.metadata.system_message.as_deref().is_some_and(|s| s.contains("14 months")));

        let raid = row("hist-raid-1");
        assert!(raid.content.is_empty());
        assert_eq!(raid.metadata.system_message.as_deref(), Some("25 raiders from Raider have joined!"));

        // Nothing to show: left out, as live chat leaves it out.
        assert!(rows.iter().all(|m| m.id != "hist-blank-1"));

        // A PRIVMSG that merely mentions the word is still a PRIVMSG.
        assert_eq!(row("hist-msg-1").content, "USERNOTICE is just a word here");
    }
}

/// The backlog after a moderator toggled followers-only and slow mode, plus a
/// deletion and a timeout, as the recent-messages mirror hands it over.
#[cfg(test)]
mod history_notice_tests {
    use super::*;

    const HELLO: &str = "@badge-info=;badges=;color=;display-name=Bob;emotes=;historical=1;id=hist-msg-1;room-id=11;tmi-sent-ts=1700000004000;user-id=46 :bob!bob@bob.tmi.twitch.tv PRIVMSG #chan :hello";
    const FOLLOWERS_ON: &str = "@historical=1;msg-id=followers_on;rm-received-ts=1700000005000;room-id=11 :tmi.twitch.tv NOTICE #chan :This room is now in followers-only mode.";
    const ROOMSTATE_FOLLOWERS: &str = "@emote-only=0;followers-only=0;historical=1;r9k=0;rm-received-ts=1700000005001;room-id=11;slow=0;subs-only=0 :tmi.twitch.tv ROOMSTATE #chan";
    const SLOW_ON: &str = "@historical=1;msg-id=slow_on;rm-received-ts=1700000006000;room-id=11 :tmi.twitch.tv NOTICE #chan :This room is now in slow mode. You may send messages every 30 seconds.";
    const ROOMSTATE_SLOW: &str = "@historical=1;rm-received-ts=1700000006001;room-id=11;slow=30 :tmi.twitch.tv ROOMSTATE #chan";
    const FOLLOWERS_OFF: &str = "@historical=1;msg-id=followers_off;rm-received-ts=1700000007000;room-id=11 :tmi.twitch.tv NOTICE #chan :This room is no longer in followers-only mode.";
    const RATELIMIT: &str = "@historical=1;msg-id=msg_ratelimit;rm-received-ts=1700000007500 :tmi.twitch.tv NOTICE #chan :Your message was not sent because you are sending messages too quickly.";
    const DELETED: &str = "@badge-info=;badges=;color=;display-name=Bob;emotes=;historical=1;id=hist-msg-2;room-id=11;tmi-sent-ts=1700000008000;user-id=46 :bob!bob@bob.tmi.twitch.tv PRIVMSG #chan :bad words";
    const CLEARMSG: &str = "@historical=1;login=bob;room-id=;target-msg-id=hist-msg-2;tmi-sent-ts=1700000008500 :tmi.twitch.tv CLEARMSG #chan :bad words";
    const CARL_BEFORE: &str = "@badge-info=;badges=;color=;display-name=Carl;emotes=;historical=1;id=hist-msg-3;room-id=11;tmi-sent-ts=1700000009000;user-id=47 :carl!carl@carl.tmi.twitch.tv PRIVMSG #chan :spam";
    const CLEARCHAT: &str = "@ban-duration=600;historical=1;room-id=11;target-user-id=47;tmi-sent-ts=1700000009500 :tmi.twitch.tv CLEARCHAT #chan :carl";
    const CARL_AFTER: &str = "@badge-info=;badges=;color=;display-name=Carl;emotes=;historical=1;id=hist-msg-4;room-id=11;tmi-sent-ts=1700000010000;user-id=47 :carl!carl@carl.tmi.twitch.tv PRIVMSG #chan :sorry";
    const CLEAR_ALL: &str = "@historical=1;room-id=11;tmi-sent-ts=1700000010500 :tmi.twitch.tv CLEARCHAT #chan";

    #[tokio::test]
    async fn mode_changes_show_as_the_live_twitch_row_and_state_lines_do_not_show() {
        let rows = IrcService::parse_historical_messages(
            [
                HELLO,
                FOLLOWERS_ON,
                ROOMSTATE_FOLLOWERS,
                SLOW_ON,
                ROOMSTATE_SLOW,
                FOLLOWERS_OFF,
                RATELIMIT,
                DELETED,
                CLEARMSG,
                CARL_BEFORE,
                CLEARCHAT,
                CARL_AFTER,
                CLEAR_ALL,
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        )
        .await;

        // The bug: every NOTICE and ROOMSTATE came back as a nameless row.
        assert!(rows.iter().all(|m| m.username != "unknown"), "{rows:?}");

        let ids: Vec<&str> = rows.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "hist-msg-1",
                "notice-chan-followers_on-1700000005000",
                "notice-chan-slow_on-1700000006000",
                "notice-chan-followers_off-1700000007000",
                // hist-msg-2 went with its CLEARMSG, hist-msg-3 with Carl's
                // timeout; what Carl said after it stays, and a whole-chat
                // clear removes nothing (live chat marks nothing for one).
                "hist-msg-4",
            ]
        );

        let notice = &rows[1];
        assert_eq!(notice.content, "This room is now in followers-only mode.");
        assert_eq!(notice.username, "System");
        assert_eq!(notice.display_name, "Twitch");
        assert_eq!(notice.user_id, "tw-system");
        assert_eq!(notice.channel, "chan");
        assert_eq!(notice.timestamp, "1700000005000");
        assert!(notice.metadata.from_backfill);
        assert!(matches!(
            notice.segments.as_slice(),
            [MessageSegment::Text { content }] if content == "This room is now in followers-only mode."
        ));
        assert_eq!(rows[2].content, "This room is now in slow mode. You may send messages every 30 seconds.");
    }

    #[test]
    fn notice_rows_keep_their_id_across_parses() {
        let a = history_notice_row(FOLLOWERS_ON).unwrap();
        let b = history_notice_row(FOLLOWERS_ON).unwrap();
        assert_eq!(a.id, b.id);
        assert!(history_notice_row(RATELIMIT).is_none());
        assert!(history_notice_row(ROOMSTATE_SLOW).is_none());
    }

    #[test]
    fn a_reparse_with_the_table_names_only_the_rows_it_changed() {
        let chan = "history_restamp_chan";
        let line = |id: &str, body: &str| {
            format!("@display-name=Dee;emotes=;historical=1;id={id};room-id=12;tmi-sent-ts=1700000000000;user-id=48 :dee!dee@dee.tmi.twitch.tv PRIVMSG #{chan} :{body}")
        };
        let lines = vec![line("r-1", "catJAM vibes"), line("r-2", "no emotes here")];
        let parse = |lines: &[String]| -> Vec<ChatMessage> {
            lines.iter().filter_map(|l| IrcService::parse_privmsg(l)).collect()
        };

        // First visit: no table yet, so the emote is a word.
        let first = parse(&lines);
        let before: HashMap<String, String> =
            first.iter().map(|m| (m.id.clone(), history_row_signature(m))).collect();
        assert!(history_segment_updates(&before, &first).is_empty());

        // The channel's table lands.
        let mut set = EmoteSet::new();
        set.seven_tv.push(
            serde_json::from_value(json!({
                "id": "01GB2S3ZQ8000BNRXQQ0VKQ0E4",
                "name": "catJAM",
                "url": "https://cdn.7tv.app/emote/01GB2S3ZQ8000BNRXQQ0VKQ0E4/1x.avif",
                "provider": "7tv",
            }))
            .expect("minimal emote"),
        );
        rebuild_parse_lookup(chan, &set);

        let again = parse(&lines);
        let updates = history_segment_updates(&before, &again);
        assert_eq!(updates.len(), 1, "{updates:?}");
        assert_eq!(updates[0]["id"], "r-1");
        let first_segment = &updates[0]["segments"][0];
        assert_eq!(first_segment["type"], "emote");
        assert_eq!(first_segment["content"], "catJAM");
        drop_parse_lookup(chan);
    }

    #[tokio::test]
    async fn a_waiter_hears_the_table_land_and_gives_up_without_one() {
        let key = "history_wait_chan";
        let waiter = tokio::spawn(wait_for_channel_emotes(key, std::time::Duration::from_secs(5)));
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        get_channel_emotes().lock().await.insert(key.to_string(), EmoteSet::new());
        note_emote_table_installed(key);
        assert!(waiter.await.unwrap());
        // Already in place: answered at once.
        assert!(wait_for_channel_emotes(key, std::time::Duration::from_millis(1)).await);
        get_channel_emotes().lock().await.remove(key);

        assert!(!wait_for_channel_emotes("history_wait_none", std::time::Duration::from_millis(30)).await);
    }
}

/// Chat frames for the iPhone app's replay fixtures, made by the same parse and
/// stamp path the socket uses, from real emote, badge and paint data and raw
/// IRC lines in `ios/Tests/Fixtures/chat-inputs.json`. Always checks the frames;
/// with `SN_WRITE_CHAT_FIXTURES=<path>` it also writes them there.
#[cfg(test)]
mod chat_fixture_tests {
    use super::*;
    use crate::models::chat_layout::ViewSpan;

    const INPUTS: &str = include_str!("../../../ios/Tests/Fixtures/chat-inputs.json");

    fn frames() -> (Value, Vec<String>) {
        let inputs: Value = serde_json::from_str(INPUTS).expect("chat inputs parse");
        let channel = inputs["channel"].as_str().unwrap().to_string();
        let set: EmoteSet = serde_json::from_value(inputs["emotes"].clone()).expect("emote set");
        rebuild_parse_lookup(&channel, &set);
        // The invite's card as Discord's public invite read answers it.
        crate::services::link_preview_cache::remember_for_test(
            "https://discord.gg/discord-developers",
            crate::commands::link_preview::LinkPreview {
                url: "https://discord.gg/discord-developers".into(),
                kind: "discord".into(),
                title: Some("Discord Developers".into()),
                image: Some(
                    "https://cdn.discordapp.com/icons/613425648685547541/a_1d18823294be0ccfbcdec090c8ffcc0d.gif?size=128".into(),
                ),
                site_name: Some("Discord".into()),
                author: Some("discord.gg/discord-developers".into()),
                online_count: Some(70761),
                member_count: Some(308559),
                ..Default::default()
            },
        );
        crate::services::chat_row_view::note_twitch_badges(
            None,
            inputs["twitch_badges"].as_array().unwrap().iter().map(|b| {
                (
                    b["set_id"].as_str().unwrap(),
                    b["version"].as_str().unwrap(),
                    b["url"].as_str().unwrap(),
                    b["title"].as_str().unwrap(),
                )
            }),
        );
        for (user_id, worn) in inputs["cosmetics"].as_object().unwrap() {
            let mark = |v: &Value| {
                let mut v = v.clone();
                v["selected"] = json!(true);
                v
            };
            crate::services::seventv_cosmetics_resolver::remember_for_test(
                user_id,
                crate::services::seventv_cosmetics_resolver::UserCosmetics {
                    paints: worn["paint"].as_object().map(|_| vec![mark(&worn["paint"])]).unwrap_or_default(),
                    badges: worn["badge"].as_object().map(|_| vec![mark(&worn["badge"])]).unwrap_or_default(),
                    ..Default::default()
                },
            );
        }
        ChatRules::set_own_identity(
            inputs["own"]["login"].as_str().unwrap(),
            inputs["own"]["user_id"].as_str().unwrap(),
        );
        let rules = crate::services::chat_rules::CompiledRules::defaults_for_test();

        let mut out = vec!["IRC_CONNECTED".to_string()];
        for line in inputs["lines"].as_array().unwrap() {
            let line = line.as_str().unwrap();
            let frame = match irc_command(line) {
                Some("PRIVMSG") | Some("USERNOTICE") => {
                    let parsed = if irc_command(line) == Some("PRIVMSG") {
                        IrcService::parse_privmsg(line)
                    } else {
                        IrcService::parse_usernotice(line).filter(usernotice_has_visible_content)
                    };
                    let mut msg = parsed.unwrap_or_else(|| panic!("line did not parse: {line}"));
                    assert!(!ChatRules::evaluate_unrecorded(&mut msg, &rules).drop);
                    crate::services::chat_row_view::stamp_view(&mut msg);
                    serde_json::to_string(&msg).unwrap()
                }
                Some("ROOMSTATE") => IrcService::roomstate_frame(line, Some(&channel)),
                Some("CLEARMSG") => IrcService::parse_clearmsg(line).unwrap().frame().to_string(),
                Some("CLEARCHAT") => IrcService::parse_clearchat(line).frame().to_string(),
                other => panic!("no fixture path for {other:?}"),
            };
            out.push(frame);
        }
        out.push("HEARTBEAT".to_string());
        (inputs, out)
    }

    fn row<'a>(frames: &'a [Value], id: &str) -> &'a Value {
        frames.iter().find(|f| f["id"] == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn fixture_frames_carry_the_native_view() {
        let (inputs, frames) = frames();
        let parsed: Vec<Value> = frames.iter().filter_map(|f| serde_json::from_str(f).ok()).collect();
        assert_eq!(frames.len(), inputs["lines"].as_array().unwrap().len() + 2);

        // Overlays ride their base emote.
        let pet = row(&parsed, "fx-0003");
        let spans: Vec<ViewSpan> = serde_json::from_value(pet["metadata"]["view"]["spans"].clone()).unwrap();
        assert!(matches!(&spans[0], ViewSpan::Emote { emote, overlays, .. }
            if emote.code == "PepePls" && overlays.len() == 1 && overlays[0].code == "PETPET" && emote.width == Some(22)));

        // A mention of the viewer is stamped by the rule engine.
        assert_eq!(row(&parsed, "fx-0007")["metadata"]["is_mentioned"], json!(true));

        // A Discord invite carries its server card, read before the row was sent.
        let invite = &row(&parsed, "fx-discord")["metadata"]["view"]["previews"][0];
        assert_eq!(invite["url"], json!("https://discord.gg/discord-developers"));
        assert_eq!(invite["preview"]["kind"], json!("discord"));
        assert_eq!(invite["preview"]["member_count"], json!(308559));

        // Paints, Twitch badge art and the 7TV badge for a chatter the cache knows.
        let owl = row(&parsed, "fx-0001");
        assert_eq!(owl["metadata"]["view"]["paint"]["kind"], json!("linear"));
        let badges = owl["metadata"]["view"]["badges"].as_array().unwrap();
        assert_eq!(badges.len(), 3);
        assert_eq!(badges[2]["provider"], json!("7tv"));

        // A navy name reads light on the dark theme.
        let navy = row(&parsed, "fx-clap")["metadata"]["view"]["name_color"].as_str().unwrap().to_string();
        assert_ne!(navy.to_lowercase(), "#000080");

        // Room state, deletions and timeouts are frames of their own.
        assert!(frames.iter().any(|f| f.contains("\"type\":\"ROOMSTATE\"") && f.contains("\"modes\"")));
        assert!(frames.iter().any(|f| f.contains("\"type\":\"CLEARMSG\"")));
        assert!(frames.iter().any(|f| f.contains("\"type\":\"CLEARCHAT\"") && f.contains("\"ban_duration\":600")));

        if let Ok(path) = std::env::var("SN_WRITE_CHAT_FIXTURES") {
            let mut design = serde_json::to_value(crate::models::settings::ChatDesignSettings::first_run()).unwrap();
            // The phone's first-run text size and spacing (this test runs on a desktop build).
            design["font_size"] = json!(14);
            design["message_spacing"] = json!(8);
            let fixture = json!({
                "channel": inputs["channel"],
                "room_id": inputs["room_id"],
                "own": inputs["own"],
                "chat_design": design,
                "emotes": inputs["emotes"],
                // What `command_catalog` answers a viewer on a phone.
                "commands": crate::services::command_catalog::catalog(true, false, false, &[], ""),
                "frames": frames,
            });
            std::fs::write(&path, serde_json::to_string_pretty(&fixture).unwrap() + "\n").expect("write fixtures");
        }
    }
}

#[cfg(test)]
mod bridge_close_tests {
    use super::{bridge_generation, close_page_sockets, IrcService};
    use futures_util::StreamExt;
    use std::time::Duration;
    use tokio_tungstenite::tungstenite::Message;

    /// The close code a page socket ends with, or `None` when it ended without one.
    async fn close_code(
        socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    ) -> Option<u16> {
        while let Some(frame) = socket.next().await {
            match frame {
                Ok(Message::Close(c)) => return c.map(|f| u16::from(f.code)),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
        None
    }

    // Drives the same close `IrcService::stop` ends with, without the rest of
    // its teardown: that clears process-wide caches other tests here read.
    #[tokio::test]
    async fn a_full_stop_closes_the_page_socket_it_left_open() {
        let port = IrcService::ensure_local_ws_bridge().await.expect("bridge up");
        let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/"))
            .await
            .expect("page socket connects");

        let before = bridge_generation();
        close_page_sockets();
        assert_ne!(bridge_generation(), before);

        let code = tokio::time::timeout(Duration::from_secs(5), close_code(&mut socket))
            .await
            .expect("the page socket closes within five seconds");
        assert_eq!(code, Some(1012), "a service-restart close, so the page reconnects");
    }
}
