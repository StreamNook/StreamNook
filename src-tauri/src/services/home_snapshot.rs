//! Rust-owned Home snapshot: the data the Home grid and the Sidebar render,
//! kept warm by Rust on its own cadence and handed to any window in one call.
//!
//! Before this, three JavaScript timers re-fetched the same followed list
//! (Sidebar every 3 min, Home on every mount after a 300 ms delay, the live
//! notification loop every 60 s in Rust and throwing the result away), the
//! offline roster and its last-broadcast times lived in Home component state
//! and were refetched on every reopen, and hype trains were polled from
//! JavaScript every 30 s. Reopening Home therefore meant two network round
//! trips before the grid was current.
//!
//! Now one poll per section runs here:
//!
//! - followed live: every 60 s while signed in; feeds the live-notification
//!   diff as well, so the one Helix call serves notifications, Sidebar and Home.
//! - offline follows + last broadcasts: every 10 min while signed in, and on
//!   demand when a Home mounts with a stale section.
//! - recommended page 1: every 5 min, only while a Home is mounted.
//! - hype trains: every 30 s while any window exists, for the channels on
//!   screen (followed live + recommended + whatever a Home reports through
//!   `set_extra_channels`: category and search results).
//! - watch streaks: hourly, for the followed-live channels.
//! - drops: active campaigns plus the inventory's active game names, hourly
//!   while any window exists and on mount when stale.
//! - recommended paging: `load_more_recommended` appends the next page to
//!   the same section, so the list stays canonical here.
//!
//! Each section carries its fetch time. A section is emitted to the windows
//! (`home-snapshot`, tagged by section) only when its content changed, so a
//! quiet minute costs nothing on the IPC side. `get_home_snapshot` returns
//! everything at once for a mounting Home; `refresh_home_section` is the
//! manual pull (sidebar close, palette command) with a 15 s floor per section.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::{debug, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

use crate::commands::hype_train::{get_bulk_hype_train_status, HypeTrainBulkStatus};
use crate::commands::watch_streak::get_watch_streaks_batch;
use crate::models::drops::{CampaignStatus, DropCampaign};
use crate::models::settings::AppState;
use crate::models::stream::TwitchStream;
use crate::services::live_notification_service::LiveNotificationService;
use crate::services::twitch_service::TwitchService;

/// Event name for section updates. Payload: `HomeUpdate`.
pub const EVENT: &str = "home-snapshot";

const FOLLOWED_PERIOD: Duration = Duration::from_secs(60);
const OFFLINE_PERIOD: Duration = Duration::from_secs(600);
const RECOMMENDED_PERIOD: Duration = Duration::from_secs(300);
const HYPE_PERIOD: Duration = Duration::from_secs(30);
const STREAKS_PERIOD: Duration = Duration::from_secs(3600);
const DROPS_PERIOD: Duration = Duration::from_secs(3600);
/// Floor between two manual refreshes of the same section.
const MIN_MANUAL_GAP: Duration = Duration::from_secs(15);
/// Recommended page size Home shows before "load more".
const RECOMMENDED_LIMIT: u32 = 20;
/// Follow list page size for the offline roster (Home showed 100 before).
const OFFLINE_LIMIT: u32 = 100;

#[derive(Serialize, Clone, Default)]
pub struct HomeSnapshot {
    pub followed_live: Vec<TwitchStream>,
    pub followed_live_at: Option<u64>,
    pub offline_follows: Vec<TwitchStream>,
    pub last_broadcasts: HashMap<String, Option<String>>,
    pub offline_at: Option<u64>,
    pub recommended: Vec<TwitchStream>,
    pub recommended_cursor: Option<String>,
    pub recommended_at: Option<u64>,
    pub hype_trains: Vec<HypeTrainBulkStatus>,
    pub hype_at: Option<u64>,
    /// channel_id -> current watch streak (only channels with a streak > 0).
    pub watch_streaks: HashMap<String, u32>,
    pub streaks_at: Option<u64>,
    /// Every active campaign (cards and category tiles key on game id/name).
    pub drops_campaigns: Vec<DropCampaign>,
    /// Lower-cased game names of campaigns the account is actively in.
    pub drops_active_game_names: Vec<String>,
    pub drops_at: Option<u64>,
}

/// One changed section, as emitted on `EVENT`.
#[derive(Serialize, Clone)]
#[serde(tag = "section", rename_all = "snake_case")]
pub enum HomeUpdate {
    FollowedLive {
        streams: Vec<TwitchStream>,
        at: u64,
    },
    Offline {
        channels: Vec<TwitchStream>,
        last_broadcasts: HashMap<String, Option<String>>,
        at: u64,
    },
    Recommended {
        streams: Vec<TwitchStream>,
        cursor: Option<String>,
        at: u64,
    },
    HypeTrains {
        statuses: Vec<HypeTrainBulkStatus>,
        at: u64,
    },
    WatchStreaks {
        streaks: HashMap<String, u32>,
        at: u64,
    },
    Drops {
        campaigns: Vec<DropCampaign>,
        active_game_names: Vec<String>,
        at: u64,
    },
}

struct Inner {
    app: AppHandle,
    notifications: Arc<LiveNotificationService>,
    snap: RwLock<HomeSnapshot>,
    /// Mounted Home components across all windows. Recommended polling and
    /// the on-mount stale refresh key off this.
    home_mounted: AtomicUsize,
    last_manual: Mutex<HashMap<&'static str, Instant>>,
    /// Channel ids a Home has on screen beyond followed + recommended
    /// (category and search results), included in the hype poll.
    extra_hype_ids: RwLock<Vec<String>>,
}

static SERVICE: OnceLock<Arc<Inner>> = OnceLock::new();

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn app_state(app: &AppHandle) -> Option<AppState> {
    app.try_state::<AppState>().map(|s| s.inner().clone())
}

async fn signed_in() -> bool {
    TwitchService::get_token().await.is_ok()
}

/// Never loaded, or loaded longer ago than `period`.
fn is_stale(at: Option<u64>, period: Duration) -> bool {
    at.map_or(true, |t| now_secs().saturating_sub(t) >= period.as_secs())
}

/// Content equality through the serialized form: sections are a few hundred
/// KB at most and this runs once a minute, so it is cheaper to reason about
/// than a hand-written comparator that misses a field.
fn same<T: Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_string(a).ok() == serde_json::to_string(b).ok()
}

fn emit(app: &AppHandle, update: HomeUpdate) {
    if let Err(e) = app.emit(EVENT, &update) {
        debug!("[HomeSnapshot] emit failed: {e}");
    }
}

/// Start the pollers. Called once from the setup hook.
pub fn start(app: AppHandle, notifications: Arc<LiveNotificationService>) {
    let inner = Arc::new(Inner {
        app,
        notifications,
        snap: RwLock::new(HomeSnapshot::default()),
        home_mounted: AtomicUsize::new(0),
        last_manual: Mutex::new(HashMap::new()),
        extra_hype_ids: RwLock::new(Vec::new()),
    });
    if SERVICE.set(inner.clone()).is_err() {
        return;
    }

    // Followed live: the one poll that also drives live notifications.
    let followed = inner.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(5)).await;
        loop {
            refresh_followed(&followed).await;
            tokio::time::sleep(FOLLOWED_PERIOD).await;
        }
    });

    // Offline roster. The FIRST load is chained off the first successful
    // followed poll (see refresh_followed), so Home gets it one round trip
    // after the live list instead of waiting on a timer that may fire before
    // sign-in completes. This loop only keeps it fresh afterwards; while the
    // followed poll has not succeeded yet it re-checks often rather than
    // sleeping a whole period.
    let offline = inner.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let (signed, stale) = {
                let s = offline.snap.read().await;
                (s.followed_live_at.is_some(), is_stale(s.offline_at, OFFLINE_PERIOD))
            };
            if signed && stale {
                refresh_offline(&offline).await;
            }
            tokio::time::sleep(if signed { OFFLINE_PERIOD } else { Duration::from_secs(15) }).await;
        }
    });

    // Recommended: only while a Home is mounted somewhere.
    let recommended = inner.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        loop {
            if recommended.home_mounted.load(Ordering::Relaxed) > 0 {
                refresh_recommended(&recommended).await;
            }
            tokio::time::sleep(RECOMMENDED_PERIOD).await;
        }
    });

    // Hype trains: while any window exists (cards are visible in Sidebar too).
    let hype = inner.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            if !hype.app.webview_windows().is_empty() {
                refresh_hype(&hype).await;
            }
            tokio::time::sleep(HYPE_PERIOD).await;
        }
    });

    // Drops: hourly while any window exists, once signed in. First load is
    // chained off the first followed poll like the offline roster.
    let drops = inner;
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            let (signed, stale) = {
                let s = drops.snap.read().await;
                (s.followed_live_at.is_some(), is_stale(s.drops_at, DROPS_PERIOD))
            };
            if signed && stale && !drops.app.webview_windows().is_empty() {
                refresh_drops(&drops).await;
            }
            tokio::time::sleep(if signed { DROPS_PERIOD } else { Duration::from_secs(15) }).await;
        }
    });
}

/// Channel ids a Home has on screen beyond the followed and recommended
/// lists (category grid, search results). Refreshes hype right away when the
/// set gained ids the last poll did not cover.
pub async fn set_extra_channels(ids: Vec<String>) {
    let Some(inner) = SERVICE.get() else { return };
    let gained = {
        let mut extra = inner.extra_hype_ids.write().await;
        let gained = ids.iter().any(|id| !extra.contains(id));
        *extra = ids;
        gained
    };
    if gained {
        let _ = refresh("hype_trains", None, None).await;
    }
}

/// Append the next recommended page to the section (the cursor lives here).
pub async fn load_more_recommended() -> Result<(), String> {
    let inner = SERVICE.get().ok_or("home snapshot not started")?;
    let cursor = inner.snap.read().await.recommended_cursor.clone();
    let Some(cursor) = cursor else { return Ok(()) };
    let state = app_state(&inner.app).ok_or("no app state")?;
    let (languages, personalized) = discovery_prefs(&state);
    let (streams, next) = TwitchService::get_recommended_streams_paginated(
        &state,
        Some(cursor),
        RECOMMENDED_LIMIT,
        languages,
        personalized,
    )
    .await
    .map_err(|e| e.to_string())?;
    let at = now_secs();
    let (merged, next) = {
        let mut s = inner.snap.write().await;
        let followed: HashSet<String> = s.followed_live.iter().map(|x| x.user_id.clone()).collect();
        let mut seen: HashSet<String> = s.recommended.iter().map(|x| x.user_id.clone()).collect();
        for st in streams {
            if !followed.contains(&st.user_id) && seen.insert(st.user_id.clone()) {
                s.recommended.push(st);
            }
        }
        s.recommended_cursor = next.clone();
        s.recommended_at = Some(at);
        (s.recommended.clone(), next)
    };
    emit(&inner.app, HomeUpdate::Recommended { streams: merged, cursor: next, at });
    Ok(())
}

fn discovery_prefs(state: &AppState) -> (Vec<String>, bool) {
    let Ok(settings) = state.settings.lock() else { return (Vec::new(), false) };
    let languages = settings
        .extra
        .get("discovery_languages")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default();
    let personalized = settings
        .extra
        .get("discovery_personalized")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    (languages, personalized)
}

async fn refresh_streaks(inner: &Inner) {
    let ids: Vec<String> = inner
        .snap
        .read()
        .await
        .followed_live
        .iter()
        .map(|s| s.user_id.clone())
        .collect();
    if ids.is_empty() {
        return;
    }
    let streaks: HashMap<String, u32> = match get_watch_streaks_batch(ids).await {
        Ok(map) => map
            .into_iter()
            .filter(|(_, v)| v.streak_count > 0)
            .map(|(k, v)| (k, v.streak_count.max(0) as u32))
            .collect(),
        Err(e) => {
            debug!("[HomeSnapshot] watch streaks: {e}");
            return;
        }
    };
    let at = now_secs();
    let changed = {
        let mut s = inner.snap.write().await;
        let changed = !same(&s.watch_streaks, &streaks);
        s.watch_streaks = streaks.clone();
        s.streaks_at = Some(at);
        changed
    };
    if changed {
        emit(&inner.app, HomeUpdate::WatchStreaks { streaks, at });
    }
}

async fn refresh_drops(inner: &Inner) {
    let Some(state) = app_state(&inner.app) else { return };
    let (campaigns, active_game_names) = {
        let drops = state.drops_service.lock().await;
        let campaigns = match drops.get_all_active_campaigns_cached().await {
            Ok(c) => c,
            Err(e) => {
                debug!("[HomeSnapshot] drop campaigns: {e}");
                return;
            }
        };
        // The inventory names the campaigns the account is actively in; the
        // Sidebar indicator keyed on those. Best effort: an inventory failure
        // keeps the campaign list and an empty active set.
        let mut names: Vec<String> = match drops.fetch_inventory().await {
            Ok(inv) => inv
                .items
                .iter()
                .filter(|item| matches!(item.status, CampaignStatus::Active))
                .filter(|item| !item.campaign.game_name.is_empty())
                .map(|item| item.campaign.game_name.to_lowercase())
                .collect(),
            Err(e) => {
                debug!("[HomeSnapshot] drops inventory: {e}");
                Vec::new()
            }
        };
        names.sort();
        names.dedup();
        (campaigns, names)
    };
    let at = now_secs();
    let changed = {
        let mut s = inner.snap.write().await;
        let changed = !same(&s.drops_campaigns, &campaigns) || s.drops_active_game_names != active_game_names;
        s.drops_campaigns = campaigns.clone();
        s.drops_active_game_names = active_game_names.clone();
        s.drops_at = Some(at);
        changed
    };
    if changed {
        emit(
            &inner.app,
            HomeUpdate::Drops {
                campaigns,
                active_game_names,
                at,
            },
        );
    }
}

/// The whole snapshot, for a mounting Home.
pub async fn snapshot() -> HomeSnapshot {
    match SERVICE.get() {
        Some(inner) => inner.snap.read().await.clone(),
        None => HomeSnapshot::default(),
    }
}

/// A Home mounted (`true`) or unmounted (`false`). On mount, sections that
/// are stale or empty refresh right away so the grid is current within a
/// round trip instead of waiting for their next tick.
pub async fn set_home_mounted(mounted: bool) {
    let Some(inner) = SERVICE.get() else { return };
    if mounted {
        inner.home_mounted.fetch_add(1, Ordering::Relaxed);
        let (offline_stale, recommended_stale, drops_stale) = {
            let s = inner.snap.read().await;
            (
                is_stale(s.offline_at, OFFLINE_PERIOD),
                is_stale(s.recommended_at, RECOMMENDED_PERIOD),
                is_stale(s.drops_at, DROPS_PERIOD),
            )
        };
        let inner = inner.clone();
        tauri::async_runtime::spawn(async move {
            if recommended_stale {
                refresh_recommended(&inner).await;
            }
            let signed = inner.snap.read().await.followed_live_at.is_some();
            if offline_stale && signed {
                refresh_offline(&inner).await;
            }
            if drops_stale && signed {
                refresh_drops(&inner).await;
            }
        });
    } else {
        let _ = inner
            .home_mounted
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| Some(n.saturating_sub(1)));
    }
}

/// Manual refresh of one section, floored at `MIN_MANUAL_GAP` per section.
pub async fn refresh(
    section: &str,
    languages: Option<Vec<String>>,
    personalized: Option<bool>,
) -> Result<(), String> {
    let inner = SERVICE.get().ok_or("home snapshot not started")?;
    let key: &'static str = match section {
        "followed_live" => "followed_live",
        "offline" => "offline",
        "recommended" => "recommended",
        "hype_trains" => "hype_trains",
        "watch_streaks" => "watch_streaks",
        "drops" => "drops",
        other => return Err(format!("unknown home section: {other}")),
    };
    {
        let mut last = inner.last_manual.lock().await;
        if last.get(key).is_some_and(|t| t.elapsed() < MIN_MANUAL_GAP) {
            return Ok(());
        }
        last.insert(key, Instant::now());
    }
    match key {
        "followed_live" => refresh_followed(inner).await,
        "offline" => refresh_offline(inner).await,
        "recommended" => refresh_recommended_with(inner, languages, personalized).await,
        "watch_streaks" => refresh_streaks(inner).await,
        "drops" => refresh_drops(inner).await,
        _ => refresh_hype(inner).await,
    }
    Ok(())
}

async fn refresh_followed(inner: &Inner) {
    let Some(state) = app_state(&inner.app) else { return };
    if !signed_in().await {
        // Signed out: drop whatever the last account left behind so the next
        // Home does not render a stranger's follows, and tell the windows.
        let had_data = {
            let mut s = inner.snap.write().await;
            let had = !s.followed_live.is_empty() || !s.offline_follows.is_empty();
            *s = HomeSnapshot::default();
            had
        };
        if had_data {
            let at = now_secs();
            emit(&inner.app, HomeUpdate::FollowedLive { streams: Vec::new(), at });
            emit(
                &inner.app,
                HomeUpdate::Offline {
                    channels: Vec::new(),
                    last_broadcasts: HashMap::new(),
                    at,
                },
            );
        }
        return;
    }
    match TwitchService::get_followed_streams(&state).await {
        Ok(streams) => {
            let at = now_secs();
            let changed = {
                let mut s = inner.snap.write().await;
                let changed = !same(&s.followed_live, &streams);
                s.followed_live = streams.clone();
                s.followed_live_at = Some(at);
                changed
            };
            if changed {
                emit(
                    &inner.app,
                    HomeUpdate::FollowedLive {
                        streams: streams.clone(),
                        at,
                    },
                );
            }
            inner
                .notifications
                .observe(&inner.app, &state, &streams)
                .await;
            let (streaks_stale, offline_never, drops_never) = {
                let s = inner.snap.read().await;
                (
                    is_stale(s.streaks_at, STREAKS_PERIOD),
                    s.offline_at.is_none(),
                    s.drops_at.is_none(),
                )
            };
            // The sections gated on sign-in load right behind the first
            // successful followed poll, so a Home that mounted at launch (before
            // this poll could run) is not left with a spinner until the offline
            // and drops timers happen to line up with a signed-in state.
            if offline_never {
                refresh_offline(inner).await;
            }
            if drops_never && !inner.app.webview_windows().is_empty() {
                refresh_drops(inner).await;
            }
            if streaks_stale {
                refresh_streaks(inner).await;
            }
        }
        Err(e) => debug!("[HomeSnapshot] followed streams: {e}"),
    }
}

async fn refresh_offline(inner: &Inner) {
    let live_ids: HashSet<String> = inner
        .snap
        .read()
        .await
        .followed_live
        .iter()
        .map(|s| s.user_id.clone())
        .collect();
    let channels = match TwitchService::get_all_followed_channels(OFFLINE_LIMIT, None).await {
        Ok((channels, _cursor)) => channels,
        Err(e) => {
            debug!("[HomeSnapshot] followed channels: {e}");
            return;
        }
    };
    let offline: Vec<TwitchStream> = channels
        .into_iter()
        .filter(|c| !live_ids.contains(&c.user_id))
        .collect();
    let ids: Vec<String> = offline.iter().map(|c| c.user_id.clone()).collect();
    let last_broadcasts = if ids.is_empty() {
        HashMap::new()
    } else {
        match TwitchService::get_offline_last_broadcasts(ids).await {
            Ok(map) => map,
            Err(e) => {
                warn!("[HomeSnapshot] last broadcasts: {e}");
                HashMap::new()
            }
        }
    };
    let at = now_secs();
    let changed = {
        let mut s = inner.snap.write().await;
        let changed = !same(&s.offline_follows, &offline) || !same(&s.last_broadcasts, &last_broadcasts);
        s.offline_follows = offline.clone();
        s.last_broadcasts = last_broadcasts.clone();
        s.offline_at = Some(at);
        changed
    };
    if changed {
        emit(
            &inner.app,
            HomeUpdate::Offline {
                channels: offline,
                last_broadcasts,
                at,
            },
        );
    }
}

async fn refresh_recommended(inner: &Inner) {
    refresh_recommended_with(inner, None, None).await
}

/// `languages` / `personalized` override the stored discovery preferences:
/// the settings dialog calls the manual refresh before its debounced save
/// reaches Rust, so the caller passes what it just chose.
async fn refresh_recommended_with(
    inner: &Inner,
    languages: Option<Vec<String>>,
    personalized: Option<bool>,
) {
    let Some(state) = app_state(&inner.app) else { return };
    // Discovery preferences are frontend-shaped settings that ride the serde
    // catch-all; read them the way the store did when it made this call.
    let (stored_languages, stored_personalized) = discovery_prefs(&state);
    let languages = languages.unwrap_or(stored_languages);
    let personalized = personalized.unwrap_or(stored_personalized);
    let (streams, cursor) = match TwitchService::get_recommended_streams_paginated(
        &state,
        None,
        RECOMMENDED_LIMIT,
        languages,
        personalized,
    )
    .await
    {
        Ok(page) => page,
        Err(e) => {
            debug!("[HomeSnapshot] recommended: {e}");
            return;
        }
    };
    let followed_ids: HashSet<String> = inner
        .snap
        .read()
        .await
        .followed_live
        .iter()
        .map(|s| s.user_id.clone())
        .collect();
    let streams: Vec<TwitchStream> = streams
        .into_iter()
        .filter(|s| !followed_ids.contains(&s.user_id))
        .collect();
    let at = now_secs();
    let changed = {
        let mut s = inner.snap.write().await;
        let changed = !same(&s.recommended, &streams) || s.recommended_cursor != cursor;
        s.recommended = streams.clone();
        s.recommended_cursor = cursor.clone();
        s.recommended_at = Some(at);
        changed
    };
    if changed {
        emit(&inner.app, HomeUpdate::Recommended { streams, cursor, at });
    }
}

async fn refresh_hype(inner: &Inner) {
    let ids: Vec<String> = {
        let s = inner.snap.read().await;
        let extra = inner.extra_hype_ids.read().await;
        let mut seen = HashSet::new();
        s.followed_live
            .iter()
            .chain(s.recommended.iter())
            .map(|st| st.user_id.clone())
            .chain(extra.iter().cloned())
            .filter(|id| seen.insert(id.clone()))
            .collect()
    };
    if ids.is_empty() {
        return;
    }
    let statuses: Vec<HypeTrainBulkStatus> = match get_bulk_hype_train_status(ids).await {
        Ok(all) => all.into_iter().filter(|h| h.is_active).collect(),
        Err(e) => {
            debug!("[HomeSnapshot] hype trains: {e}");
            return;
        }
    };
    let at = now_secs();
    let changed = {
        let mut s = inner.snap.write().await;
        let changed = !same(&s.hype_trains, &statuses);
        s.hype_trains = statuses.clone();
        s.hype_at = Some(at);
        changed
    };
    if changed {
        emit(&inner.app, HomeUpdate::HypeTrains { statuses, at });
    }
}
