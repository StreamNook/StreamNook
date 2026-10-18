//! The stream open in the main window, as a session Rust owns.
//!
//! What used to run inside the frontend's `startStream` and
//! `handleStreamOffline`, on webview timers that throttle when the window is
//! hidden: the EventSub subscription for the watched channel, the hype-train
//! poll, Discord presence, raid redirects, the offline confirmation and the
//! choice of what to switch to, and the live check standing in for
//! `stream.offline` on other platforms. The watched channel's hype train is
//! shown through services/hype_train_watch.rs, like every other surface's. The frontend keeps the player and the
//! chat panes; it tells Rust what it is watching and acts on what Rust decides.
//!
//! Events (all scoped to the current session):
//! - `watch-session://redirect`      a raid to follow, with the target row
//! - `watch-session://offline`       the watched stream looks offline; the view
//!                                   asks `watch_session_resolve_offline`
//! - `watch-session://went-live`     an offline room's channel went live and
//!                                   is playable: `{ stream }`
//!
//! An offline room (`WatchTarget::offline`) is a session too: the view shows a
//! channel's chat while it is not streaming, and the session waits for it to
//! go live, through EventSub `stream.online` when signed in and the batched
//! `channel_state` viewer poll always (which also covers signed-out viewers).
//!
//! The watch EventSub socket (services/eventsub_service.rs) is aimed only from
//! here: at MultiNook's focused Twitch tile while the grid is open
//! (`set_grid_focus`), else at the watched Twitch channel.
//! - `watch-session://stream-update` fresh title / category / viewers
//! - `watch-session://resolving`     an offline confirmation started / ended
//! - `watch-session://reconnecting`  a dropped streamer is being waited out:
//!                                   `{ login, waiting }`
//! - `watch-session://reload`        the same stream came back under a fresh
//!                                   manifest; reload it in place: `{ login }`

use crate::commands::eventsub::EventSubServiceState;
use crate::models::settings::{AppState, AutoSwitchMode};
use crate::models::stream::TwitchStream;
use crate::services::eventsub_service::{ChannelUpdateEvent, RaidEvent, StreamOnlineEvent};
use crate::services::hype_train_watch;
use crate::services::twitch_service::TwitchService;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use crate::rt::AppHandle;
use tauri::{Emitter, Manager};

pub const REDIRECT_EVENT: &str = "watch-session://redirect";
pub const OFFLINE_EVENT: &str = "watch-session://offline";
pub const WENT_LIVE_EVENT: &str = "watch-session://went-live";
pub const STREAM_UPDATE_EVENT: &str = "watch-session://stream-update";
pub const RESOLVING_EVENT: &str = "watch-session://resolving";
pub const RECONNECTING_EVENT: &str = "watch-session://reconnecting";
pub const RELOAD_EVENT: &str = "watch-session://reload";
/// An offline room's "N waiting": `{ login, count }`, about once a minute.
pub const WAITING_EVENT: &str = "watch-session://waiting";
const WAITING_POLL: Duration = Duration::from_secs(60);

/// Helix keeps listing a dead stream for a while after `stream.offline`, so a
/// fast double-check reads "still online" for a genuinely ended stream. Poll
/// for up to ~35 s: offline twice in a row confirms; still live when the window
/// closes means an encoder blip the streamer recovered from.
const VERIFY_ATTEMPTS: usize = 8;
const VERIFY_INTERVAL: Duration = Duration::from_secs(5);
const VERIFY_CONSECUTIVE_OFFLINE: usize = 2;
/// A raid redirect wins over an auto-switch triggered just after it.
const RAID_COOLDOWN_MS: i64 = 15_000;
const SWITCH_CANDIDATES: u32 = 10;

/// Stands in for `stream.offline` on platforms without EventSub.
const PROVIDER_POLL: Duration = Duration::from_secs(60);
/// Two misses in a row, so one flaky answer cannot eject the viewer.
const PROVIDER_OFFLINE_STRIKES: u32 = 2;
/// How long a YouTube broadcast that is held open (the streamer dropped) is
/// waited out before the viewer is told it did not come back, and how often it
/// is re-read meanwhile. youtube.com waits as long as the broadcast stays open;
/// this only bounds a broadcast YouTube never closes.
const RECONNECT_WAIT: Duration = Duration::from_secs(10 * 60);
const RECONNECT_POLL: Duration = Duration::from_secs(10);

/// What the view is watching, as it knows it at start.
#[derive(Debug, Clone, Deserialize)]
pub struct WatchTarget {
    /// "twitch", "kick", "youtube", "tiktok".
    pub provider: String,
    pub login: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub game_id: Option<String>,
    #[serde(default)]
    pub game_name: String,
    /// RFC 3339 start of the broadcast, empty when unknown.
    #[serde(default)]
    pub started_at: String,
    /// Whether a Twitch account is signed in (EventSub needs one).
    #[serde(default)]
    pub authenticated: bool,
    /// An offline room: the channel is not streaming and the view shows its
    /// chat. The session waits for it to go live instead of for it to end.
    #[serde(default)]
    pub offline: bool,
}

struct Session {
    seq: u64,
    target: WatchTarget,
    tasks: Vec<tauri::async_runtime::JoinHandle<()>>,
    last_raid_ms: i64,
    resolving: bool,
    /// An offline room already announced its channel going live.
    live_sent: bool,
}

/// How long an offline room waits for a channel that announced `stream.online`
/// to be listed live (and so playable): Twitch can announce before the stream
/// is watchable, and starting it then fails into chat-only.
const LIVE_CONFIRM_ATTEMPTS: usize = 8;
const LIVE_CONFIRM_INTERVAL: Duration = Duration::from_secs(4);

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static SEQ: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn with_session<R>(f: impl FnOnce(&mut Option<Session>) -> R) -> R {
    let mut guard = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// The watched Twitch channel and its broadcast start, if a Twitch stream plays.
pub fn current_twitch_watch() -> Option<(String, String)> {
    with_session(|s| {
        s.as_ref()
            .filter(|s| s.target.provider == "twitch" && !s.target.offline)
            .map(|s| (s.target.login.to_lowercase(), s.target.started_at.clone()))
    })
}

/// What the main window is watching, if anything. An offline room plays
/// nothing, so it is not a target.
pub fn current_target() -> Option<WatchTarget> {
    with_session(|s| s.as_ref().filter(|s| !s.target.offline).map(|s| s.target.clone()))
}

/// The Twitch broadcaster id of the session (live or an offline room), if any.
pub fn twitch_channel_id() -> Option<String> {
    with_session(|s| {
        s.as_ref()
            .filter(|s| s.target.provider == "twitch" && !s.target.user_id.is_empty())
            .map(|s| s.target.user_id.clone())
    })
}

/// The channel whose offline room is open, if one is.
pub fn offline_room_login() -> Option<String> {
    with_session(|s| s.as_ref().filter(|s| s.target.offline).map(|s| s.target.login.to_lowercase()))
}

/// The session's target, when `seq` is still the current session.
fn current(seq: u64) -> Option<WatchTarget> {
    with_session(|s| s.as_ref().filter(|s| s.seq == seq).map(|s| s.target.clone()))
}

fn is_current_broadcaster(broadcaster_id: &str, login: &str) -> Option<u64> {
    with_session(|s| {
        s.as_ref()
            .filter(|s| {
                s.target.provider == "twitch"
                    && ((!broadcaster_id.is_empty() && s.target.user_id == broadcaster_id)
                        || s.target.login.eq_ignore_ascii_case(login))
            })
            .map(|s| s.seq)
    })
}

fn provider_label(provider: &str) -> &'static str {
    match provider {
        "kick" => "Kick",
        "youtube" => "YouTube",
        "tiktok" => "TikTok",
        _ => "Twitch",
    }
}

fn watch_url(target: &WatchTarget) -> String {
    if target.provider == "twitch" {
        format!("https://twitch.tv/{}", target.login)
    } else {
        crate::services::providers::watch_urls::watch_url(&target.provider, &target.login)
    }
}

async fn publish_presence(app: &AppHandle, target: &WatchTarget) {
    let state = app.state::<AppState>();
    let enabled = state.settings.lock().map(|s| s.discord_rpc_enabled).unwrap_or(false);
    if !enabled {
        return;
    }
    let name = if target.user_name.is_empty() { &target.login } else { &target.user_name };
    if target.offline {
        let started = u64::try_from(now_ms()).unwrap_or_default();
        let small = format!("{}_logo", target.provider);
        if let Err(e) = discord::watching(&format!("In {name}'s chat"), "Offline", &small, started, "", &watch_url(target), &state).await {
            log::debug!("[WatchSession] Discord presence not updated (Discord may not be running): {e}");
        }
        return;
    }
    let activity = if target.title.is_empty() {
        format!("Live on {}", provider_label(&target.provider))
    } else {
        target.title.clone()
    };
    let small = format!("{}_logo", target.provider);
    let started = u64::try_from(now_ms()).unwrap_or_default();
    if let Err(e) = discord::watching(&format!("Watching {name}"), &activity, &small, started, &target.game_name, &watch_url(target), &state).await {
        log::debug!("[WatchSession] Discord presence not updated (Discord may not be running): {e}");
    }
}

/// Discord Rich Presence exists only on desktop; on the phone these do nothing.
mod discord {
    use crate::models::settings::AppState;

    #[cfg(desktop)]
    pub async fn watching(
        details: &str,
        activity: &str,
        small_image: &str,
        started: u64,
        game: &str,
        url: &str,
        state: &AppState,
    ) -> Result<(), String> {
        crate::services::discord_service::DiscordService::update_presence(
            details, activity, "icon_256x256", small_image, started, game, url, state,
        )
        .await
        .map_err(|e| e.to_string())
    }

    #[cfg(desktop)]
    pub async fn idle(state: &AppState) -> Result<(), String> {
        crate::services::discord_service::DiscordService::set_idle_presence(state)
            .await
            .map_err(|e| e.to_string())
    }

    #[cfg(not(desktop))]
    pub async fn watching(
        _details: &str,
        _activity: &str,
        _small_image: &str,
        _started: u64,
        _game: &str,
        _url: &str,
        _state: &AppState,
    ) -> Result<(), String> {
        Ok(())
    }

    #[cfg(not(desktop))]
    pub async fn idle(_state: &AppState) -> Result<(), String> {
        Ok(())
    }
}

// ---- Lifecycle ----------------------------------------------------------------

/// Begin (or replace) the session for what the main window now plays.
pub async fn start(app: AppHandle, target: WatchTarget) {
    // Whatever opened this session (an offline room, a provider stream) wins
    // over a Twitch start still resolving.
    crate::services::watch_start::supersede();
    let seq = SEQ.fetch_add(1, Ordering::AcqRel) + 1;
    let previous = with_session(|s| {
        // A raid's cooldown outlives the raided session: the channel just left
        // still reports offline, and that must not auto-switch away from the
        // raid target.
        let last_raid_ms = s.as_ref().map(|p| p.last_raid_ms).unwrap_or(0);
        s.replace(Session {
            seq,
            target: target.clone(),
            tasks: Vec::new(),
            last_raid_ms,
            resolving: false,
            live_sent: false,
        })
    });
    if let Some(prev) = previous {
        release(prev);
    }
    // StreamNook watch time counts a live channel on screen; an offline room
    // shows chat only, so it clears the main source instead.
    crate::services::watch_minutes::set_source(
        crate::services::watch_minutes::MAIN,
        (!target.offline).then(|| crate::services::watch_minutes::WatchedChannel {
            platform: target.provider.clone(),
            channel_id: target.user_id.clone(),
            login: target.login.clone(),
            name: if target.user_name.is_empty() { target.login.clone() } else { target.user_name.clone() },
        }),
    );
    // While a Cast session is up, the TV follows what the view switched to.
    // An offline room plays nothing, so the TV keeps what it has.
    #[cfg(target_os = "android")]
    if !target.offline {
        crate::services::cast_session::on_watch_target(&app, &target);
    }

    // This channel's EventSub, unless MultiNook's focused tile holds the
    // socket. A Twitch subscription must not keep firing over a Kick stream.
    aim_watch_socket(&app).await;

    let mut tasks = Vec::new();
    if target.provider == "twitch" {
        if target.offline {
            // The batched viewer poll notices the go-live too, signed in or
            // not; a channel that is also docked costs no extra request.
            if !target.user_id.is_empty() {
                let known = crate::services::channel_state::watch(&target.login, &target.user_id, true).await;
                // Another surface already watches it and knows it is live: no
                // edge will come, so look now.
                if known.started_at.is_some() {
                    let (app, target) = (app.clone(), target.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Some(row) = live_row(&target).await {
                            announce_live(&app, seq, row);
                        }
                    });
                }
            }
            tasks.push(tauri::async_runtime::spawn(poll_waiting(app.clone(), seq)));
        } else {
            hype_train_watch::watch(
                &app,
                hype_train_watch::INTERNAL_OWNER,
                &target.login,
                Some(target.user_id.clone()),
                &target.user_name,
            );
        }
    } else if !target.offline {
        // Offline rooms are Twitch-only.
        tasks.push(tauri::async_runtime::spawn(poll_provider_live(app.clone(), seq)));
        if target.provider == "youtube" {
            tasks.push(tauri::async_runtime::spawn(watch_youtube_broadcast(app.clone(), seq)));
        }
    }
    publish_presence(&app, &target).await;

    let orphaned = with_session(|s| match s.as_mut().filter(|s| s.seq == seq) {
        Some(session) => {
            session.tasks = tasks;
            None
        }
        None => Some(tasks),
    });
    // A newer start won while this one was connecting: its tasks are not ours.
    for task in orphaned.into_iter().flatten() {
        task.abort();
    }
}

/// End the session. `preserve_backend` hands the channel to MultiNook: the
/// watch socket stays on it until the grid declares its focus
/// (`set_grid_focus`), and the grid publishes its own presence.
pub async fn stop(app: AppHandle, preserve_backend: bool) {
    crate::services::watch_start::supersede();
    let previous = with_session(|s| s.take());
    crate::services::watch_minutes::set_source(crate::services::watch_minutes::MAIN, None);
    let Some(prev) = previous else { return };
    release(prev);
    if preserve_backend {
        return;
    }
    // Down, unless MultiNook's focused tile holds the socket.
    aim_watch_socket(&app).await;
    let state = app.state::<AppState>();
    if let Err(e) = discord::idle(&state).await {
        log::debug!("[WatchSession] Could not set idle Discord presence: {e}");
    }
}

/// Publish Discord presence for what is being watched, or idle when nothing
/// is (Discord was just switched on in settings).
pub async fn refresh_presence(app: AppHandle) {
    let target = with_session(|s| s.as_ref().map(|s| s.target.clone()));
    match target {
        Some(target) => publish_presence(&app, &target).await,
        None => {
            let state = app.state::<AppState>();
            let _ = discord::idle(&state).await;
        }
    }
}

/// Stop what a finished session was running.
fn release(session: Session) {
    for task in session.tasks {
        task.abort();
    }
    if session.target.provider != "twitch" {
        return;
    }
    if !session.target.offline {
        hype_train_watch::unwatch(hype_train_watch::INTERNAL_OWNER, &session.target.login);
    } else if !session.target.user_id.is_empty() {
        let login = session.target.login;
        tauri::async_runtime::spawn(async move {
            crate::services::channel_state::unwatch(&login, true).await;
        });
    }
}

// ---- The watch socket and MultiNook's focus ----------------------------------------

/// The MultiNook tile whose chat the grid shows, as the grid declares it.
#[derive(Debug, Clone, Deserialize)]
pub struct GridFocus {
    /// "twitch", "kick", "youtube", "tiktok".
    pub provider: String,
    pub login: String,
    /// Empty while the tile's channel id is not known yet.
    #[serde(default)]
    pub user_id: String,
}

/// A Twitch channel the grid's focus settled on.
#[derive(Debug, Clone, PartialEq)]
struct FocusedChannel {
    id: String,
    login: String,
}

/// MultiNook while it is open. `twitch` is the last Twitch tile the focus
/// settled on: a Kick, YouTube or TikTok tile has no Twitch events, so
/// focusing one leaves the socket, drops and the predictions set where they
/// were.
#[derive(Default)]
struct Grid {
    twitch: Option<FocusedChannel>,
}

static GRID: Mutex<Option<Grid>> = Mutex::new(None);
/// Bumped by every Twitch focus and by the grid closing, so a focus still
/// settling gives way to a newer one.
static GRID_SEQ: AtomicU64 = AtomicU64::new(0);
/// Clicking through tiles moves the focus several times a second; the socket
/// follows the tile it settles on.
const GRID_SETTLE: Duration = Duration::from_millis(250);

/// Every aim runs under this lock, so a session start and a grid focus never
/// replace each other's connection halfway through its subscribes.
static SOCKET_AIM: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn with_grid<R>(f: impl FnOnce(&mut Option<Grid>) -> R) -> R {
    let mut guard = GRID.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// Whose broadcaster the watch socket follows.
#[derive(Debug, PartialEq)]
enum SocketTarget {
    /// MultiNook's focused Twitch tile.
    Grid(String),
    /// The watched Twitch channel (an offline room too: it waits on
    /// `stream.online`).
    Session(String),
}

/// The grid's focused Twitch tile wins while the grid is open: it is what is
/// on screen, over any solo stream kept behind the grid. EventSub needs a
/// signed-in user either way.
fn socket_target(grid_focus: Option<&str>, signed_in: bool, session: Option<&WatchTarget>) -> Option<SocketTarget> {
    if let Some(id) = grid_focus {
        return signed_in.then(|| SocketTarget::Grid(id.to_string()));
    }
    session
        .filter(|t| t.provider == "twitch" && t.authenticated && !t.user_id.is_empty())
        .map(|t| SocketTarget::Session(t.user_id.clone()))
}

/// Point the watch socket at what it should follow now. A socket that already
/// follows the right broadcaster is left alone, connected or waiting out a
/// reconnect: it keeps its subscriptions and heals itself. One that stopped
/// (a sign-out ends it) no longer counts as following anything.
async fn aim_watch_socket(app: &AppHandle) {
    let _serial = SOCKET_AIM.lock().await;
    let grid_focus = with_grid(|g| g.as_ref().and_then(|g| g.twitch.as_ref()).map(|c| c.id.clone()));
    let signed_in = grid_focus.is_some() && TwitchService::get_token().await.is_ok();
    let session = with_session(|s| s.as_ref().map(|s| s.target.clone()));
    let target = socket_target(grid_focus.as_deref(), signed_in, session.as_ref());
    let (wanted, from) = match target {
        Some(SocketTarget::Grid(id)) => (Some(id), "MultiNook's focused tile"),
        Some(SocketTarget::Session(id)) => (Some(id), "the watched channel"),
        None => (None, ""),
    };
    let eventsub = app.state::<EventSubServiceState>().0.clone();
    let service = eventsub.read().await;
    if service.following().await == wanted {
        return;
    }
    match wanted {
        Some(id) => {
            log::info!("[WatchSession] watch socket follows {from} ({id})");
            if let Err(e) = service.connect_and_listen(id, app.clone()).await {
                log::warn!("[WatchSession] EventSub could not connect: {e}");
            }
        }
        None => {
            log::info!("[WatchSession] watch socket closed: nothing to follow");
            service.disconnect().await;
        }
    }
}

/// Record a Twitch focus that settled. `Some(moved)` while the grid is open,
/// `None` once it has closed.
fn settle_focus(grid: &mut Option<Grid>, channel: FocusedChannel) -> Option<bool> {
    let grid = grid.as_mut()?;
    let moved = grid.twitch.as_ref() != Some(&channel);
    grid.twitch = Some(channel);
    Some(moved)
}

/// MultiNook's chat moved to `focus`, or the grid closed or emptied (`None`).
/// The watch socket, drops monitoring and the predictions set follow the
/// grid's focused Twitch tile; closing hands the socket back to the watched
/// stream, or takes it down. Only the main window's grid declares a focus: a
/// popout is a chat-only window, not a viewing context, and must not pull the
/// socket off the stream on screen.
pub fn set_grid_focus(app: AppHandle, focus: Option<GridFocus>) {
    let Some(focus) = focus else {
        let seq = GRID_SEQ.fetch_add(1, Ordering::AcqRel) + 1;
        with_grid(|g| *g = None);
        // Settles too: a grid that remounts declares its tile again at once,
        // and the socket should not drop and come back for it.
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(GRID_SETTLE).await;
            if GRID_SEQ.load(Ordering::Acquire) == seq {
                aim_watch_socket(&app).await;
            }
        });
        return;
    };
    with_grid(|g| {
        g.get_or_insert_with(Grid::default);
    });
    let login = focus.login.trim().to_lowercase();
    if focus.provider != "twitch" || login.is_empty() {
        return;
    }
    let seq = GRID_SEQ.fetch_add(1, Ordering::AcqRel) + 1;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(GRID_SETTLE).await;
        if GRID_SEQ.load(Ordering::Acquire) != seq {
            return;
        }
        // The grid unregisters a tile from the predictions set by the id it
        // knows, so only an id it gave is registered there.
        let declared = !focus.user_id.trim().is_empty();
        let id = match focus.user_id.trim() {
            "" => TwitchService::get_user_by_login(&login).await.ok().map(|u| u.id),
            id => Some(id.to_string()),
        };
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            log::debug!("[WatchSession] MultiNook focus {login} has no channel id");
            return;
        };
        let channel = FocusedChannel { id, login };
        let moved = with_grid(|g| {
            if GRID_SEQ.load(Ordering::Acquire) != seq {
                return None;
            }
            settle_focus(g, channel.clone())
        });
        let Some(moved) = moved else { return };
        aim_watch_socket(&app).await;
        if moved {
            crate::commands::drops::monitor_watched_channel(&app, channel.id.clone(), channel.login.clone()).await;
        }
        if moved && declared {
            let ws = {
                let state = app.state::<AppState>();
                let background = state.background_service.lock().await;
                background.websocket_service.clone()
            };
            ws.lock().await.register_active_channel(&channel.id).await;
        }
    });
}

// ---- EventSub, as it concerns the watched channel ----------------------------------

pub fn on_raid(app: &AppHandle, raid: &RaidEvent) {
    let Some(seq) = is_current_broadcaster(&raid.from_broadcaster_user_id, &raid.from_broadcaster_user_login)
    else {
        return;
    };
    let follow = app
        .state::<AppState>()
        .settings
        .lock()
        .map(|s| s.auto_switch.auto_redirect_on_raid)
        .unwrap_or(true);
    if !follow {
        return;
    }
    with_session(|s| {
        if let Some(session) = s.as_mut().filter(|s| s.seq == seq) {
            session.last_raid_ms = now_ms();
        }
    });
    // The TV follows the raid itself rather than waiting for the view: a locked
    // phone's page can be slow to act on the redirect below.
    #[cfg(target_os = "android")]
    crate::services::cast_session::on_raid(
        app,
        &raid.to_broadcaster_user_login,
        &raid.to_broadcaster_user_name,
    );
    // A floor, not an answer: the raid knows ids and the raiding party's size,
    // so everything else is left blank for the view's start to backfill from the
    // target's live row. A blank reads as "not known yet"; a wrong value would be
    // rendered as fact (a fabricated start time counts uptime from zero).
    let _ = app.emit(
        REDIRECT_EVENT,
        json!({
            "reason": "raid",
            "target": {
                "id": "",
                "user_id": raid.to_broadcaster_user_id,
                "user_login": raid.to_broadcaster_user_login,
                "user_name": if raid.to_broadcaster_user_name.is_empty() {
                    &raid.to_broadcaster_user_login
                } else {
                    &raid.to_broadcaster_user_name
                },
                "title": "",
                "viewer_count": raid.viewers.max(0),
                "game_name": "",
                "thumbnail_url": "",
                "profile_image_url": "",
                "started_at": "",
            },
        }),
    );
}

pub fn on_offline(app: &AppHandle, broadcaster_id: &str, login: &str) {
    if is_current_broadcaster(broadcaster_id, login).is_some() {
        let _ = app.emit(OFFLINE_EVENT, json!({ "provider": "twitch", "login": login }));
    }
}

/// `stream.online` for the watched channel. Only an offline room acts on it,
/// and only once the stream is listed live: Twitch can announce a stream
/// before it is watchable.
pub fn on_online(app: &AppHandle, online: &StreamOnlineEvent) {
    let Some(seq) = is_current_broadcaster(&online.broadcaster_user_id, &online.broadcaster_user_login) else {
        return;
    };
    let Some(target) = current(seq).filter(|t| t.offline) else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for attempt in 0..LIVE_CONFIRM_ATTEMPTS {
            if attempt > 0 {
                tokio::time::sleep(LIVE_CONFIRM_INTERVAL).await;
            }
            if current(seq).is_none() {
                return;
            }
            if let Some(row) = live_row(&target).await {
                announce_live(&app, seq, row);
                return;
            }
        }
        log::debug!("[WatchSession] {} announced online but was never listed live", target.login);
    });
}

/// The batched viewer poll saw a channel go live (or found it live on its
/// first look). An offline room on that channel starts it.
pub fn on_channel_live(app: &AppHandle, row: TwitchStream) {
    let Some(seq) = is_current_broadcaster(&row.user_id, &row.user_login) else { return };
    if current(seq).is_some_and(|t| t.offline) {
        announce_live(app, seq, row);
    }
}

/// The room's channel as a live row, or `None` while it is offline (or the
/// check failed: a failed check is not evidence either way).
async fn live_row(target: &WatchTarget) -> Option<TwitchStream> {
    let rows = if target.user_id.is_empty() {
        TwitchService::get_live_public(&[], &[target.login.clone()]).await
    } else {
        TwitchService::get_live_public(&[target.user_id.clone()], &[]).await
    };
    match rows {
        Ok(rows) => rows.into_iter().next(),
        Err(e) => {
            log::debug!("[WatchSession] live check for {} failed: {e}", target.login);
            None
        }
    }
}

/// Tell the view an offline room's channel is live, once per session.
fn announce_live(app: &AppHandle, seq: u64, row: TwitchStream) {
    let first = with_session(|s| match s.as_mut().filter(|s| s.seq == seq && s.target.offline) {
        Some(session) if !session.live_sent => {
            session.live_sent = true;
            true
        }
        _ => false,
    });
    if !first {
        return;
    }
    let _ = app.emit(WENT_LIVE_EVENT, json!({ "stream": row }));
}

pub fn on_channel_update(app: &AppHandle, update: &ChannelUpdateEvent) {
    let Some(seq) = is_current_broadcaster(&update.broadcaster_user_id, &update.broadcaster_user_login)
    else {
        return;
    };
    let target = with_session(|s| {
        let session = s.as_mut().filter(|s| s.seq == seq)?;
        session.target.title = update.title.clone();
        session.target.game_name = update.category_name.clone();
        session.target.game_id = Some(update.category_id.clone()).filter(|id| !id.is_empty());
        Some(session.target.clone())
    });
    let _ = app.emit(
        STREAM_UPDATE_EVENT,
        json!({
            "title": update.title,
            "game_name": update.category_name,
            "game_id": update.category_id,
        }),
    );
    if let Some(target) = target {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { publish_presence(&app, &target).await });
    }
}

// ---- Polls -----------------------------------------------------------------------

/// How many are waiting in an offline room's chat, refreshed while the room is
/// open (the room's own lookup gave the first count). Skipped while no window
/// is on screen; ends with the session.
async fn poll_waiting(app: AppHandle, seq: u64) {
    loop {
        tokio::time::sleep(WAITING_POLL).await;
        let Some(target) = current(seq).filter(|t| t.offline) else { return };
        if crate::services::window_visibility::all_hidden() {
            continue;
        }
        if let Ok(count) = TwitchService::get_chatter_count(&target.login).await {
            if current(seq).is_none() {
                return;
            }
            let _ = app.emit(WAITING_EVENT, json!({ "login": target.login, "count": count }));
        }
    }
}

async fn poll_provider_live(app: AppHandle, seq: u64) {
    let mut strikes = 0;
    loop {
        tokio::time::sleep(PROVIDER_POLL).await;
        let Some(target) = current(seq) else { return };
        let rows = crate::commands::provider_browse::provider_live_check(
            target.provider.clone(),
            vec![target.login.clone()],
        )
        .await;
        if current(seq).is_none() {
            return;
        }
        match rows {
            Ok(rows) => match rows.first() {
                Some(row) if row.is_live => {
                    strikes = 0;
                    let _ = app.emit(
                        STREAM_UPDATE_EVENT,
                        json!({
                            "viewer_count": row.viewer_count,
                            "title": row.title,
                            "game_name": row.game_name,
                        }),
                    );
                }
                _ => {
                    strikes += 1;
                    if strikes >= PROVIDER_OFFLINE_STRIKES {
                        // A trigger, not a verdict: the view asks
                        // `resolve_offline`, which waits out a dropped streamer.
                        // A stream that really ended stops the session, and this
                        // task with it.
                        strikes = 0;
                        let _ = app.emit(
                            OFFLINE_EVENT,
                            json!({ "provider": target.provider, "login": target.login }),
                        );
                    }
                }
            },
            // A failed check is not evidence the stream ended.
            Err(e) => log::debug!("[WatchSession] {} live check failed: {e}", target.provider),
        }
    }
}

// ---- Went offline: confirm, then decide ---------------------------------------------

/// What the view should do about a stream that looked offline.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum OfflineDecision {
    /// Nothing to do (no session, a raid just redirected, auto-switch off, a
    /// confirmation already running, or a newer session started meanwhile).
    Nothing { why: String },
    /// Helix kept reporting the stream live: a blip.
    StillLive,
    /// The stream is live and serving: reload it in place (the player was
    /// stuck on a manifest the streamer reconnected out of).
    Resume,
    /// Stop the video, keep the chat (the user prefers offline chat).
    OfflineChat,
    /// Tell the user why nothing can be done; keep what is on screen.
    Notify { notice: Notice },
    /// Stop everything; `notice`, when set, is shown to the user.
    Stop { notice: Option<Notice> },
    /// Stop everything and open this stream.
    SwitchTo { stream: TwitchStream, notice: Option<Notice> },
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Notice {
    pub message: String,
    pub error: bool,
}

fn notice(show: bool, message: String, error: bool) -> Option<Notice> {
    show.then_some(Notice { message, error })
}

/// The auto-switch target from the candidate list: the busiest stream that is
/// not the one that just ended.
fn pick_target(mut streams: Vec<TwitchStream>, ended_login: &str, sort_by_viewers: bool) -> Option<TwitchStream> {
    streams.retain(|s| !s.user_login.eq_ignore_ascii_case(ended_login));
    if sort_by_viewers {
        streams.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
    }
    streams.into_iter().next()
}

/// What made the view ask whether the stream ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OfflineTrigger {
    /// Something said the stream ended (EventSub, a live poll, a chat
    /// watchdog, a failed restart).
    #[default]
    Signal,
    /// The player stalled. A stream still live after it is reloaded in place:
    /// leaving the player on the manifest it stalled on would freeze it.
    Player,
}

/// Confirm the watched stream is really offline, then decide what the view
/// does next. Safe to call from every trigger (EventSub, player errors, a
/// stale chat, a live poll): concurrent calls resolve once.
pub async fn resolve_offline(app: AppHandle, trigger: OfflineTrigger) -> OfflineDecision {
    let state = app.state::<AppState>();
    let auto = state.settings.lock().map(|s| s.auto_switch.clone()).ok();
    let Some(auto) = auto else {
        return OfflineDecision::Nothing { why: "settings unavailable".into() };
    };
    let started = with_session(|s| {
        let session = s.as_mut()?;
        if session.target.offline {
            return Some(Err("an offline room has no stream to end"));
        }
        if session.resolving {
            return Some(Err("already resolving"));
        }
        let twitch = session.target.provider == "twitch";
        if twitch && session.last_raid_ms > 0 && now_ms() - session.last_raid_ms < RAID_COOLDOWN_MS {
            return Some(Err("a raid redirect just happened"));
        }
        // Auto-switch decides what follows an ENDED Twitch stream. A stalled
        // player still needs to learn the stream is live, so it is checked.
        if twitch && !auto.enabled && trigger != OfflineTrigger::Player {
            return Some(Err("auto-switch is off"));
        }
        session.resolving = true;
        Some(Ok((session.seq, session.target.clone())))
    });
    let (seq, target) = match started {
        None => return OfflineDecision::Nothing { why: "no session".into() },
        Some(Err(why)) => return OfflineDecision::Nothing { why: why.into() },
        Some(Ok(started)) => started,
    };
    let _ = app.emit(RESOLVING_EVENT, true);
    let decision = if target.provider == "twitch" {
        decide_offline(&state, &target, &auto, seq, trigger).await
    } else {
        decide_provider_offline(&app, &target, seq).await
    };
    with_session(|s| {
        if let Some(session) = s.as_mut().filter(|s| s.seq == seq) {
            session.resolving = false;
        }
    });
    let _ = app.emit(RESOLVING_EVENT, false);
    if current(seq).is_none() {
        return OfflineDecision::Nothing { why: "a newer stream started".into() };
    }
    decision
}

async fn decide_offline(
    state: &AppState,
    target: &WatchTarget,
    auto: &crate::models::settings::AutoSwitchSettings,
    seq: u64,
    trigger: OfflineTrigger,
) -> OfflineDecision {
    let login = target.login.as_str();
    let mut consecutive_offline = 0;
    for attempt in 0..VERIFY_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(VERIFY_INTERVAL).await;
        }
        if current(seq).is_none() {
            return OfflineDecision::Nothing { why: "a newer stream started".into() };
        }
        match TwitchService::check_stream_online(login).await {
            Ok(Some(_)) => consecutive_offline = 0,
            // An unanswerable check is not "offline".
            Err(_) => {}
            Ok(None) => {
                consecutive_offline += 1;
                if consecutive_offline >= VERIFY_CONSECUTIVE_OFFLINE {
                    break;
                }
            }
        }
    }
    if consecutive_offline < VERIFY_CONSECUTIVE_OFFLINE {
        return match trigger {
            OfflineTrigger::Player => OfflineDecision::Resume,
            OfflineTrigger::Signal => OfflineDecision::StillLive,
        };
    }
    // Watched to its end: the live bookmark files nothing for it. Only this
    // confirmed answer counts; a bare `stream.offline` also fires on a
    // reconnect the stream comes back from.
    crate::services::live_bookmark::note_ended(login);
    if !auto.enabled {
        return OfflineDecision::Nothing { why: "auto-switch is off".into() };
    }
    if auto.stay_in_offline_chat {
        return OfflineDecision::OfflineChat;
    }

    let show = auto.show_notification;
    let current_game = with_session(|s| {
        s.as_ref()
            .filter(|s| s.seq == seq)
            .map(|s| (s.target.game_id.clone(), s.target.game_name.clone()))
    })
    .unwrap_or((target.game_id.clone(), target.game_name.clone()));

    let candidates = match auto.mode {
        AutoSwitchMode::SameCategory => {
            let (game_id, game_name) = current_game;
            let game_id = game_id.filter(|id| !id.is_empty());
            if game_id.is_none() && game_name.is_empty() {
                // Nothing is stopped on this path, exactly as before.
                let message = format!("{login} went offline. Unable to find similar streams.");
                return match notice(show, message, false) {
                    Some(notice) => OfflineDecision::Notify { notice },
                    None => OfflineDecision::Nothing { why: format!("{login} has no category") },
                };
            }
            let found = match &game_id {
                Some(id) => {
                    TwitchService::get_streams_by_game_id(state, id, Some(login), None, SWITCH_CANDIDATES).await
                }
                None => {
                    TwitchService::get_streams_by_game_name(state, &game_name, Some(login), None, SWITCH_CANDIDATES)
                        .await
                }
            };
            match pick_target(found.map(|(s, _)| s).unwrap_or_default(), login, false) {
                Some(stream) => stream,
                None => {
                    return OfflineDecision::Stop {
                        notice: notice(
                            show,
                            format!("{login} went offline. No other {game_name} streams available."),
                            false,
                        ),
                    }
                }
            }
        }
        AutoSwitchMode::FollowedStreams => match TwitchService::get_followed_streams(state).await {
            Ok(followed) => match pick_target(followed, login, true) {
                Some(stream) => stream,
                None => {
                    return OfflineDecision::Stop {
                        notice: notice(show, format!("{login} went offline. No other followed streams are live."), false),
                    }
                }
            },
            Err(e) => {
                log::error!("[AutoSwitch] Error fetching followed streams: {e}");
                return OfflineDecision::Stop {
                    notice: notice(show, format!("{login} went offline. Unable to load followed streams."), true),
                };
            }
        },
    };
    let message = format!("{login} went offline. Switching to {}...", candidates.user_name);
    OfflineDecision::SwitchTo { notice: notice(show, message, false), stream: candidates }
}

// ---- Other platforms: a dropped streamer is waited out ---------------------------

fn display_name(target: &WatchTarget) -> &str {
    if target.user_name.is_empty() { &target.login } else { &target.user_name }
}

/// A provider stream that ended: the view leaves it and says why.
fn provider_ended(target: &WatchTarget, message: String) -> OfflineDecision {
    log::info!("[WatchSession] {} {}: {}", target.provider, target.login, message);
    OfflineDecision::Stop { notice: Some(Notice { message, error: false }) }
}

/// The video a YouTube watch plays: recorded by the resolve, or the login
/// itself when the stream was opened by its video id.
fn youtube_video_of(login: &str) -> Option<String> {
    use crate::services::providers::youtube_media;
    youtube_media::served_broadcast(login).map(|(video, _)| video).or_else(|| {
        let is_video_id = login.len() == 11
            && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        is_video_id.then(|| login.to_string())
    })
}

/// Confirm a Kick / YouTube / TikTok stream really ended. A brief drop on the
/// streamer's side must keep the viewer on the stream, the way the platform's
/// own player does, and only an ending leaves it.
async fn decide_provider_offline(app: &AppHandle, target: &WatchTarget, seq: u64) -> OfflineDecision {
    if target.provider == "youtube" {
        if let Some(video_id) = youtube_video_of(&target.login) {
            return wait_out_youtube(app, target, &video_id, seq).await;
        }
    }
    let mut consecutive_offline = 0;
    for attempt in 0..VERIFY_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(VERIFY_INTERVAL).await;
        }
        if current(seq).is_none() {
            return OfflineDecision::Nothing { why: "a newer stream started".into() };
        }
        let rows = crate::commands::provider_browse::provider_live_check(
            target.provider.clone(),
            vec![target.login.clone()],
        )
        .await;
        match rows {
            Ok(rows) if rows.first().is_some_and(|r| r.is_live) => return OfflineDecision::Resume,
            Ok(_) => {
                consecutive_offline += 1;
                if consecutive_offline >= VERIFY_CONSECUTIVE_OFFLINE {
                    return provider_ended(target, format!("{} went offline", display_name(target)));
                }
            }
            // A failed check is not evidence the stream ended.
            Err(e) => log::debug!("[WatchSession] {} live check failed: {e}", target.provider),
        }
    }
    OfflineDecision::Resume
}

/// Wait out a YouTube broadcast the player lost. YouTube holds the broadcast
/// open while the streamer reconnects and serves it again on the SAME video id
/// under a new `broadcastId`; resume once its manifest grows again, leave only
/// once the broadcast is over.
async fn wait_out_youtube(app: &AppHandle, target: &WatchTarget, video_id: &str, seq: u64) -> OfflineDecision {
    use crate::services::providers::youtube_media::{self, Broadcast};
    let deadline = std::time::Instant::now() + RECONNECT_WAIT;
    let mut waiting = false;
    let decision = loop {
        if current(seq).is_none() {
            break OfflineDecision::Nothing { why: "a newer stream started".into() };
        }
        match youtube_media::broadcast_state(video_id).await {
            Some(Broadcast::Ended) => {
                break provider_ended(target, format!("{} went offline", display_name(target)));
            }
            Some(Broadcast::Live { manifest, .. }) if youtube_media::manifest_growing(&manifest).await => {
                youtube_media::forget_master(video_id);
                log::info!("[WatchSession] youtube {} is serving again; resuming", target.login);
                break OfflineDecision::Resume;
            }
            // Held open, not growing yet, or no client answered: wait.
            _ => {}
        }
        if std::time::Instant::now() >= deadline {
            break provider_ended(target, format!("{}'s stream did not come back", display_name(target)));
        }
        if !waiting {
            waiting = true;
            log::info!("[WatchSession] youtube {} dropped; waiting for it to reconnect", target.login);
            let _ = app.emit(RECONNECTING_EVENT, json!({ "login": target.login, "waiting": true }));
        }
        tokio::time::sleep(RECONNECT_POLL).await;
    };
    if waiting {
        let _ = app.emit(RECONNECTING_EVENT, json!({ "login": target.login, "waiting": false }));
    }
    decision
}

fn is_resolving(seq: u64) -> bool {
    with_session(|s| s.as_ref().is_some_and(|s| s.seq == seq && s.resolving))
}

/// What youtube.com does while a broadcast plays: re-read it every
/// `pollDelayMs` and reload when its `broadcastId` moves on, which is how a
/// streamer's reconnect shows up (same video id, new manifest). Without this
/// the player sits on the old manifest until it stalls.
async fn watch_youtube_broadcast(app: AppHandle, seq: u64) {
    use crate::services::providers::youtube_media::{self, Broadcast};
    let mut pause = Duration::from_secs(30);
    // The broadcast seen first, for a stream whose resolve recorded none.
    let mut seen: Option<(String, String)> = None;
    loop {
        tokio::time::sleep(pause).await;
        let Some(target) = current(seq) else { return };
        // The offline check owns the stream while it runs.
        if is_resolving(seq) {
            continue;
        }
        let Some((video_id, served)) = youtube_media::served_broadcast(&target.login) else { continue };
        let Some(Broadcast::Live { broadcast_id, poll, .. }) = youtube_media::broadcast_state(&video_id).await else {
            continue;
        };
        pause = poll;
        if broadcast_id.is_empty() || current(seq).is_none() || is_resolving(seq) {
            continue;
        }
        let playing = if !served.is_empty() {
            served
        } else {
            match &seen {
                Some((video, id)) if *video == video_id => id.clone(),
                _ => {
                    seen = Some((video_id, broadcast_id));
                    continue;
                }
            }
        };
        if playing == broadcast_id {
            continue;
        }
        log::info!(
            "[WatchSession] youtube {} reconnected (broadcast {} -> {}); reloading",
            target.login,
            playing,
            broadcast_id
        );
        youtube_media::forget_master(&video_id);
        seen = Some((video_id, broadcast_id));
        let _ = app.emit(RELOAD_EVENT, json!({ "provider": target.provider, "login": target.login }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(login: &str, viewers: u32) -> TwitchStream {
        serde_json::from_value(json!({
            "id": "", "user_id": "", "user_name": login, "user_login": login, "title": "",
            "viewer_count": viewers, "game_name": "", "thumbnail_url": "", "started_at": "",
        }))
        .expect("minimal stream row")
    }

    #[test]
    fn the_ended_stream_is_never_the_target() {
        let picked = pick_target(vec![stream("Ended", 900), stream("next", 10)], "ended", false);
        assert_eq!(picked.map(|s| s.user_login), Some("next".to_string()));
        assert!(pick_target(vec![stream("ended", 5)], "ended", false).is_none());
    }

    #[test]
    fn followed_candidates_go_to_the_busiest() {
        let picked = pick_target(vec![stream("a", 10), stream("b", 300), stream("c", 40)], "x", true);
        assert_eq!(picked.map(|s| s.user_login), Some("b".to_string()));
    }

    #[test]
    fn category_candidates_keep_the_api_order() {
        // Helix already returns a category's streams busiest first.
        let picked = pick_target(vec![stream("first", 10), stream("second", 300)], "x", false);
        assert_eq!(picked.map(|s| s.user_login), Some("first".to_string()));
    }

    #[test]
    fn only_the_watched_twitch_channel_counts() {
        with_session(|s| {
            *s = Some(Session {
                seq: 42,
                target: WatchTarget {
                    provider: "twitch".into(),
                    login: "chan".into(),
                    user_id: "123".into(),
                    user_name: "Chan".into(),
                    title: String::new(),
                    game_id: None,
                    game_name: String::new(),
                    started_at: String::new(),
                    authenticated: true,
                    offline: false,
                },
                tasks: Vec::new(),
                last_raid_ms: 0,
                resolving: false,
                live_sent: false,
            })
        });
        assert_eq!(is_current_broadcaster("123", ""), Some(42));
        assert_eq!(is_current_broadcaster("", "CHAN"), Some(42));
        assert_eq!(is_current_broadcaster("999", "other"), None);
        assert!(offline_room_login().is_none());
        assert!(current_target().is_some());
        with_session(|s| s.as_mut().unwrap().target.offline = true);
        // An offline room still hears its channel, but plays nothing.
        assert_eq!(is_current_broadcaster("123", ""), Some(42));
        assert_eq!(offline_room_login().as_deref(), Some("chan"));
        assert!(current_target().is_none());
        assert!(current_twitch_watch().is_none());
        with_session(|s| *s = None);
        assert_eq!(is_current_broadcaster("123", "chan"), None);
    }

    fn target(provider: &str, user_id: &str, authenticated: bool) -> WatchTarget {
        serde_json::from_value(json!({
            "provider": provider, "login": "chan", "user_id": user_id, "authenticated": authenticated,
        }))
        .unwrap()
    }

    #[test]
    fn the_grid_focus_holds_the_socket_over_the_watched_channel() {
        let solo = target("twitch", "1", true);
        assert_eq!(socket_target(Some("2"), true, Some(&solo)), Some(SocketTarget::Grid("2".into())));
        // No focused Twitch tile (a closed grid, or only Kick focused): the
        // watched channel has it.
        assert_eq!(socket_target(None, false, Some(&solo)), Some(SocketTarget::Session("1".into())));
        // Nothing to subscribe with when signed out.
        assert_eq!(socket_target(Some("2"), false, Some(&solo)), None);
        assert_eq!(socket_target(None, false, Some(&target("twitch", "1", false))), None);
        assert_eq!(socket_target(None, false, Some(&target("twitch", "", true))), None);
        assert_eq!(socket_target(None, false, Some(&target("kick", "1", true))), None);
        assert_eq!(socket_target(None, false, None), None);
    }

    #[test]
    fn a_settled_focus_moves_only_on_a_new_channel_of_an_open_grid() {
        let chan = |id: &str| FocusedChannel { id: id.into(), login: format!("c{id}") };
        let mut grid = None;
        assert_eq!(settle_focus(&mut grid, chan("1")), None, "a closed grid takes no focus");
        grid = Some(Grid::default());
        assert_eq!(settle_focus(&mut grid, chan("1")), Some(true));
        assert_eq!(settle_focus(&mut grid, chan("1")), Some(false));
        assert_eq!(settle_focus(&mut grid, chan("2")), Some(true));
        assert_eq!(grid.unwrap().twitch, Some(chan("2")));
    }

    #[test]
    fn an_old_view_starts_a_live_session() {
        let target: WatchTarget = serde_json::from_value(json!({ "provider": "twitch", "login": "chan" })).unwrap();
        assert!(!target.offline, "a target without the flag is a live stream");
    }

    #[test]
    fn decisions_serialize_as_the_view_reads_them() {
        let d = OfflineDecision::Stop { notice: Some(Notice { message: "m".into(), error: false }) };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            json!({ "action": "stop", "notice": { "message": "m", "error": false } })
        );
        assert_eq!(serde_json::to_value(OfflineDecision::StillLive).unwrap(), json!({ "action": "still_live" }));
        assert_eq!(serde_json::to_value(OfflineDecision::Resume).unwrap(), json!({ "action": "resume" }));
    }

    #[test]
    fn a_trigger_reads_as_the_view_sends_it() {
        assert_eq!(serde_json::from_value::<OfflineTrigger>(json!("player")).unwrap(), OfflineTrigger::Player);
        assert_eq!(serde_json::from_value::<OfflineTrigger>(json!("signal")).unwrap(), OfflineTrigger::Signal);
        assert_eq!(OfflineTrigger::default(), OfflineTrigger::Signal);
    }
}
