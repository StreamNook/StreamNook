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
//! - `watch-session://went-live`     the watched channel just went live
//! - `watch-session://stream-update` fresh title / category / viewers
//! - `watch-session://resolving`     an offline confirmation started / ended

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
use tauri::{AppHandle, Emitter, Manager};

pub const REDIRECT_EVENT: &str = "watch-session://redirect";
pub const OFFLINE_EVENT: &str = "watch-session://offline";
pub const WENT_LIVE_EVENT: &str = "watch-session://went-live";
pub const STREAM_UPDATE_EVENT: &str = "watch-session://stream-update";
pub const RESOLVING_EVENT: &str = "watch-session://resolving";

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
}

struct Session {
    seq: u64,
    target: WatchTarget,
    tasks: Vec<tauri::async_runtime::JoinHandle<()>>,
    last_raid_ms: i64,
    resolving: bool,
}

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
            .filter(|s| s.target.provider == "twitch")
            .map(|s| (s.target.login.to_lowercase(), s.target.started_at.clone()))
    })
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
    let activity = if target.title.is_empty() {
        format!("Live on {}", provider_label(&target.provider))
    } else {
        target.title.clone()
    };
    let small = format!("{}_logo", target.provider);
    let started = u64::try_from(now_ms()).unwrap_or_default();
    if let Err(e) = crate::services::discord_service::DiscordService::update_presence(
        &format!("Watching {name}"),
        &activity,
        "icon_256x256",
        &small,
        started,
        &target.game_name,
        &watch_url(target),
        &state,
    )
    .await
    {
        log::debug!("[WatchSession] Discord presence not updated (Discord may not be running): {e}");
    }
}

// ---- Lifecycle ----------------------------------------------------------------

/// Begin (or replace) the session for what the main window now plays.
pub async fn start(app: AppHandle, target: WatchTarget) {
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
        })
    });
    let previous_was_twitch = previous.as_ref().is_some_and(|p| p.target.provider == "twitch");
    if let Some(prev) = previous {
        release(prev);
    }

    let mut tasks = Vec::new();
    if target.provider == "twitch" {
        let eventsub = app.state::<EventSubServiceState>().0.clone();
        let service = eventsub.read().await;
        if target.authenticated && !target.user_id.is_empty() {
            if let Err(e) = service.connect_and_listen(target.user_id.clone(), app.clone()).await {
                log::warn!("[WatchSession] EventSub could not connect: {e}");
            }
        } else {
            service.disconnect().await;
        }
        drop(service);
        hype_train_watch::watch(
            &app,
            hype_train_watch::INTERNAL_OWNER,
            &target.login,
            Some(target.user_id.clone()),
            &target.user_name,
        );
    } else {
        // A Twitch subscription must not keep firing over a Kick stream.
        if previous_was_twitch {
            app.state::<EventSubServiceState>().0.read().await.disconnect().await;
        }
        tasks.push(tauri::async_runtime::spawn(poll_provider_live(app.clone(), seq)));
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

/// End the session. `preserve_backend` hands the channel to MultiNook, which
/// keeps its EventSub subscription and publishes its own presence.
pub async fn stop(app: AppHandle, preserve_backend: bool) {
    let previous = with_session(|s| s.take());
    let Some(prev) = previous else { return };
    let was_twitch = prev.target.provider == "twitch";
    release(prev);
    if preserve_backend {
        return;
    }
    if was_twitch {
        app.state::<EventSubServiceState>().0.read().await.disconnect().await;
    }
    let state = app.state::<AppState>();
    if let Err(e) = crate::services::discord_service::DiscordService::set_idle_presence(&state).await {
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
            let _ = crate::services::discord_service::DiscordService::set_idle_presence(&state).await;
        }
    }
}

/// Stop what a finished session was running.
fn release(session: Session) {
    for task in session.tasks {
        task.abort();
    }
    if session.target.provider == "twitch" {
        hype_train_watch::unwatch(hype_train_watch::INTERNAL_OWNER, &session.target.login);
    }
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

pub fn on_online(app: &AppHandle, online: &StreamOnlineEvent) {
    if is_current_broadcaster(&online.broadcaster_user_id, &online.broadcaster_user_login).is_some() {
        let _ = app.emit(WENT_LIVE_EVENT, online);
    }
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
                        let _ = app.emit(
                            OFFLINE_EVENT,
                            json!({ "provider": target.provider, "login": target.login }),
                        );
                        return;
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

/// Confirm the watched Twitch stream is really offline, then decide what the
/// view does next. Safe to call from every trigger (EventSub, player errors,
/// a stale chat): concurrent calls resolve once.
pub async fn resolve_offline(app: AppHandle) -> OfflineDecision {
    let state = app.state::<AppState>();
    let auto = state.settings.lock().map(|s| s.auto_switch.clone()).ok();
    let Some(auto) = auto else {
        return OfflineDecision::Nothing { why: "settings unavailable".into() };
    };
    let started = with_session(|s| {
        let session = s.as_mut().filter(|s| s.target.provider == "twitch")?;
        if session.resolving {
            return Some(Err("already resolving"));
        }
        if session.last_raid_ms > 0 && now_ms() - session.last_raid_ms < RAID_COOLDOWN_MS {
            return Some(Err("a raid redirect just happened"));
        }
        if !auto.enabled {
            return Some(Err("auto-switch is off"));
        }
        session.resolving = true;
        Some(Ok((session.seq, session.target.clone())))
    });
    let (seq, target) = match started {
        None => return OfflineDecision::Nothing { why: "no Twitch session".into() },
        Some(Err(why)) => return OfflineDecision::Nothing { why: why.into() },
        Some(Ok(started)) => started,
    };
    let _ = app.emit(RESOLVING_EVENT, true);
    let decision = decide_offline(&state, &target, &auto, seq).await;
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
        return OfflineDecision::StillLive;
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
                },
                tasks: Vec::new(),
                last_raid_ms: 0,
                resolving: false,
            })
        });
        assert_eq!(is_current_broadcaster("123", ""), Some(42));
        assert_eq!(is_current_broadcaster("", "CHAN"), Some(42));
        assert_eq!(is_current_broadcaster("999", "other"), None);
        with_session(|s| *s = None);
        assert_eq!(is_current_broadcaster("123", "chan"), None);
    }

    #[test]
    fn decisions_serialize_as_the_view_reads_them() {
        let d = OfflineDecision::Stop { notice: Some(Notice { message: "m".into(), error: false }) };
        assert_eq!(
            serde_json::to_value(&d).unwrap(),
            json!({ "action": "stop", "notice": { "message": "m", "error": false } })
        );
        assert_eq!(serde_json::to_value(OfflineDecision::StillLive).unwrap(), json!({ "action": "still_live" }));
    }
}
