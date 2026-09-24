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
//! - collaborations (Shared Viewership): no timer of their own. They are
//!   fetched right behind every list's viewer counts (followed, recommended,
//!   the next recommended page, a Home's category and search results) and
//!   sent AHEAD of that list, so a card paints with its group instead of
//!   gaining it a poll later. Skipped while no window is on screen; a Home
//!   mount catches up. See `services::collaboration`.
//! - watch streaks: hourly, for the followed-live channels.
//! - drops: active campaigns plus the inventory's active game names, hourly
//!   while any window exists and on mount when stale.
//! - continue watching: **no poll loop at all.** The row is derived from the
//!   local VOD watch-position store, which only changes when the viewer
//!   watches something, so it is rebuilt on Home mount, on a trailing-edge
//!   notify from `report_vod_position` / `clear_vod_progress` while a Home is
//!   actually on screen, and on manual refresh. Hourly, and only while
//!   mounted, one batched GQL call reconciles the stored entries against
//!   Twitch (deleted VODs, final lengths, real thumbnails for a VOD watched
//!   while it was still recording).
//! - recommended paging: `load_more_recommended` appends the next page to
//!   the same section, so the list stays canonical here.
//! - Discover lists (`unified_discover`): Home's Discover grid on the unified
//!   view, and the Sidebar's second section for whatever scope it shows, both
//!   finished here from one cache of the other platforms' directories. Each
//!   directory is fetched on its own task and the lists are emitted as each one
//!   lands, and rebuilt when anything they leave out (follows, live favourites,
//!   the favourite list) changes. A unified Home refetches directories on a
//!   60 s check once 5 min old, never while the main window is hidden or
//!   minimized; the Sidebar alone never polls, it refetches when it is shown a
//!   scope, when it closes, and when the window comes back to the front.
//!
//! What is on screen is claimed per window and per page (`WindowClaims`), not
//! counted: a page that dies without its React cleanup (a destroyed window, a
//! reload) must not leave the polls that follow Home running in the tray.
//!
//! Each section carries its fetch time. A section is emitted to the windows
//! (`home-snapshot`, tagged by section) only when its content changed, so a
//! quiet minute costs nothing on the IPC side. `get_home_snapshot` returns
//! everything at once for a mounting Home; `refresh_home_section` is the
//! manual pull (sidebar close, palette command) with a 15 s floor per section.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::{debug, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, RwLock};

use crate::commands::hype_train::{get_bulk_hype_train_status, HypeTrainBulkStatus};
use crate::commands::watch_streak::get_watch_streaks_batch;
use crate::models::drops::{CampaignStatus, DropCampaign};
use crate::models::provider_stream::ProviderCategory;
use crate::models::settings::{AppState, FavoriteChannel, ProviderFollow};
use crate::models::stream::TwitchStream;
use crate::services::collaboration::{self, Collaboration};
use crate::services::live_notification_service::LiveNotificationService;
use crate::services::providers::key::PROVIDER_IDS;
use crate::services::providers::registry;
use crate::services::providers::source::StreamSource;
use crate::services::twitch_service::TwitchService;
use crate::services::unified_discover::{self, DiscoverRow, Surface, UnifiedDiscover, View};
use crate::services::unified_following::{self, Following};
use crate::services::{favorite_live_service, provider_categories, provider_live_service};

/// Event name for section updates. Payload: `HomeUpdate`.
pub const EVENT: &str = "home-snapshot";

const FOLLOWED_PERIOD: Duration = Duration::from_secs(60);
const OFFLINE_PERIOD: Duration = Duration::from_secs(600);
const RECOMMENDED_PERIOD: Duration = Duration::from_secs(300);
const HYPE_PERIOD: Duration = Duration::from_secs(30);
/// A Home mount refetches collaborations older than this.
const COLLAB_MOUNT_STALE: Duration = Duration::from_secs(60);
/// The longest a list waits for its collaborations before it is sent without
/// them. The next pass fills them in.
const COLLAB_WAIT: Duration = Duration::from_secs(3);
const STREAKS_PERIOD: Duration = Duration::from_secs(3600);
const DROPS_PERIOD: Duration = Duration::from_secs(3600);
/// Floor between two manual refreshes of the same section.
const MIN_MANUAL_GAP: Duration = Duration::from_secs(15);
/// Recommended page size Home shows before "load more".
const RECOMMENDED_LIMIT: u32 = 20;
/// Follow list page size for the offline roster (Home showed 100 before).
const OFFLINE_LIMIT: u32 = 100;
/// Cards the Continue Watching row carries, and the hydrate batch size.
const CONTINUE_LIMIT: usize = 24;
/// How often the row is reconciled against Twitch.
const CONTINUE_HYDRATE_PERIOD: Duration = Duration::from_secs(3600);
/// A card with no avatar yet (a VOD first watched since the last hydrate)
/// pulls the next hydrate forward to this floor, so the new card fills in
/// on the next Home mount instead of up to an hour later.
const CONTINUE_HYDRATE_RETRY: Duration = Duration::from_secs(60);
/// Rebuild the row at most this often while Home is mounted and the viewer is
/// watching (`report_vod_position` fires every 5 s).
const CONTINUE_COALESCE: Duration = Duration::from_secs(10);
/// A mount rebuilds the row if it is older than this.
const CONTINUE_MOUNT_STALE: Duration = Duration::from_secs(30);
/// How often the unified Discover directories are checked while a unified
/// Home is on screen. Each is refetched once `unified_discover::PERIOD_SECS`
/// old, so this bounds how late that runs.
const DISCOVER_TICK: Duration = Duration::from_secs(60);
/// A directory fetch that has not answered by now is abandoned, so a platform
/// that hangs cannot hold its claim (and block every later fetch) forever.
const DIRECTORY_TIMEOUT: Duration = Duration::from_secs(30);
/// Categories each other platform contributes to the unified Categories tab.
const OTHER_CATEGORIES_PER_PROVIDER: u32 = 20;

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
    /// Twitch channel id -> its Shared Viewership group, only for channels
    /// that are in one.
    pub collaborations: HashMap<String, Collaboration>,
    pub collab_at: Option<u64>,
    /// channel_id -> current watch streak (only channels with a streak > 0).
    pub watch_streaks: HashMap<String, u32>,
    pub streaks_at: Option<u64>,
    /// Every active campaign (cards and category tiles key on game id/name).
    pub drops_campaigns: Vec<DropCampaign>,
    /// Lower-cased game names of campaigns the account is actively in.
    pub drops_active_game_names: Vec<String>,
    pub drops_at: Option<u64>,
    /// Unfinished VODs the viewer opened on purpose, newest watch first.
    pub continue_watching: Vec<ContinueWatchingItem>,
    pub continue_watching_at: Option<u64>,
    /// Home's Discover tab on the unified view, finished: Twitch's picks and
    /// every other platform's directory as one ranked list, without what the
    /// Following tab or the Favourites section already shows. Built when the
    /// snapshot is read (see `snapshot`), never stored here.
    pub unified_discover: Vec<DiscoverRow>,
    pub unified_discover_at: Option<u64>,
    /// Your channels across every platform, finished: live favourites, the
    /// other live follows and the offline roster (see `unified_following`).
    /// Built when the snapshot is read, like `unified_discover`.
    pub following: Following,
    pub following_at: Option<u64>,
    /// The unified Categories tab's "On other platforms" row: every other
    /// platform's own categories, from `provider_categories`' cache.
    pub other_categories: Vec<ProviderCategory>,
    pub other_categories_at: Option<u64>,
}

/// One card in Home's Continue Watching row. Built entirely from the local
/// watch-position store, which already keeps the channel, title and thumbnail,
/// so the row paints with no Twitch call and survives a cold start.
///
/// No `PartialEq`: change detection goes through `same()`, which compares
/// serialized forms, so a derive here would be dead code.
#[derive(Serialize, Clone)]
pub struct ContinueWatchingItem {
    pub video_id: String,
    pub channel_login: String,
    pub channel_name: String,
    pub title: String,
    pub thumbnail_url: String,
    pub position_secs: f64,
    pub duration_secs: f64,
    /// Empty until the first hydrate after the VOD was recorded.
    pub profile_image_url: String,
    pub partner: bool,
    /// Category of the broadcast; empty until the hydrate has seen it.
    pub game_name: String,
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
    Collaborations {
        collabs: HashMap<String, Collaboration>,
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
    ContinueWatching {
        items: Vec<ContinueWatchingItem>,
        at: u64,
    },
    UnifiedDiscover {
        streams: Vec<DiscoverRow>,
        at: u64,
    },
    /// The Sidebar's second section, built for `scope` (`"all"`, or one
    /// provider id). The page shows it only under that scope.
    SidebarDiscover {
        scope: String,
        streams: Vec<DiscoverRow>,
        at: u64,
    },
    /// Your channels across every platform: see `HomeSnapshot::following`.
    Following {
        favorites: Vec<DiscoverRow>,
        live: Vec<DiscoverRow>,
        offline: Vec<DiscoverRow>,
        at: u64,
    },
    /// The unified Categories tab's "On other platforms" row.
    OtherCategories {
        categories: Vec<ProviderCategory>,
        at: u64,
    },
}

/// What one window's surfaces have claimed, as one JS context (page load) of
/// it.
#[derive(Default, Debug, PartialEq)]
struct WindowClaims {
    /// The page these claims belong to. A reload runs no React cleanup, so the
    /// page before never says its surfaces left: a claim from a new context
    /// replaces that page's claims instead.
    context: String,
    /// Mounted Homes.
    mounted: usize,
    /// How many of `mounted` show every platform at once.
    unified: usize,
    /// The scope the window's Sidebar shows its second section for, while it
    /// shows one.
    sidebar: Option<String>,
}

impl WindowClaims {
    /// `window`'s claims for the page `context`, replacing a dead page's. `None`
    /// for a release from a page that has since been replaced, which must
    /// change nothing, so the order two in-flight calls land in cannot matter.
    fn for_page<'a>(
        claims: &'a mut HashMap<String, WindowClaims>,
        window: &str,
        context: &str,
        releasing: bool,
    ) -> Option<&'a mut WindowClaims> {
        let entry = claims.entry(window.to_string()).or_default();
        if entry.context != context {
            if releasing {
                return None;
            }
            *entry = WindowClaims {
                context: context.to_string(),
                ..WindowClaims::default()
            };
        }
        Some(entry)
    }
}

/// Record a Home arriving (`mounted`) or leaving in `window`, under the page
/// `context`.
fn claim_home(
    claims: &mut HashMap<String, WindowClaims>,
    window: &str,
    context: &str,
    mounted: bool,
    unified: bool,
) {
    let Some(entry) = WindowClaims::for_page(claims, window, context, !mounted) else {
        return;
    };
    if mounted {
        entry.mounted += 1;
        entry.unified += usize::from(unified);
    } else {
        entry.mounted = entry.mounted.saturating_sub(1);
        entry.unified = entry.unified.saturating_sub(usize::from(unified));
    }
}

/// Record the scope the Sidebar in `window` shows its second section for, or
/// that it shows none (`None`: disabled, or the section switched off).
fn claim_sidebar(
    claims: &mut HashMap<String, WindowClaims>,
    window: &str,
    context: &str,
    scope: Option<&str>,
) {
    let Some(entry) = WindowClaims::for_page(claims, window, context, scope.is_none()) else {
        return;
    };
    entry.sidebar = scope.map(str::to_string);
}

struct Inner {
    app: AppHandle,
    notifications: Arc<LiveNotificationService>,
    snap: RwLock<HomeSnapshot>,
    /// What each window's Home and Sidebar have claimed. Recommended polling,
    /// the on-mount stale refresh and the Discover lists key off these.
    ///
    /// Per window and per page rather than counters, because a page that goes
    /// away without its React cleanup never says it left. Go Live, and closing
    /// to the tray under "Always", DESTROY the main window with its Home still
    /// mounted (`release_window` drops those claims), and a reload abandons
    /// them (`WindowClaims::for_page` drops those). A bare counter stayed above
    /// zero after either and kept the recommended poll running in the tray.
    claims: std::sync::Mutex<HashMap<String, WindowClaims>>,
    last_manual: Mutex<HashMap<&'static str, Instant>>,
    /// Channel ids a Home has on screen beyond followed + recommended
    /// (category and search results), included in the hype poll.
    extra_hype_ids: RwLock<Vec<String>>,
    /// When the Continue Watching row was last reconciled against Twitch.
    /// Internal timing, deliberately not on the React-facing snapshot, and
    /// stamped only on success so a failed hydrate retries.
    continue_hydrated: Mutex<Option<Instant>>,
    /// The Discover lists: each platform's directory and each surface's list as
    /// last emitted. Held for a whole rebuild, which is what keeps two rebuilds
    /// from emitting out of order. Deliberately outside `snap`, so the
    /// signed-out reset of that struct leaves the other platforms' anonymous
    /// directories alone.
    discover: Mutex<UnifiedDiscover>,
    /// The Following lists and the "On other platforms" row as last emitted.
    /// Each is held for a whole rebuild, like `discover`, so two rebuilds
    /// cannot emit out of order.
    following: Mutex<Option<Following>>,
    other_categories: Mutex<Option<Vec<ProviderCategory>>>,
}

impl Inner {
    fn claims(&self) -> std::sync::MutexGuard<'_, HashMap<String, WindowClaims>> {
        self.claims.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A Home is mounted in some window.
    fn home_mounted(&self) -> bool {
        self.claims().values().any(|w| w.mounted > 0)
    }

    /// A mounted Home shows every platform at once, so Home's Discover list is
    /// wanted, and with it the periodic directory refresh.
    fn unified_mounted(&self) -> bool {
        self.claims().values().any(|w| w.unified > 0)
    }

    /// The scope a Sidebar shows its second section for. There is one Sidebar,
    /// in the main window; were there more, the first window by label wins.
    fn sidebar_scope(&self) -> Option<String> {
        self.claims()
            .iter()
            .filter_map(|(window, c)| c.sidebar.as_ref().map(|scope| (window, scope)))
            .min_by_key(|(window, _)| *window)
            .map(|(_, scope)| scope.clone())
    }

    /// Some surface shows one of the Discover lists.
    fn discover_wanted(&self) -> bool {
        self.unified_mounted() || self.sidebar_scope().is_some()
    }
}

static SERVICE: OnceLock<Arc<Inner>> = OnceLock::new();
/// A Continue Watching rebuild is already scheduled. Trailing edge, so the
/// final position report of a session always lands.
static CONTINUE_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// A unified Discover rebuild is already scheduled, and will see any change
/// that lands before it starts.
static DISCOVER_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
        claims: std::sync::Mutex::new(HashMap::new()),
        last_manual: Mutex::new(HashMap::new()),
        extra_hype_ids: RwLock::new(Vec::new()),
        continue_hydrated: Mutex::new(None),
        discover: Mutex::new(UnifiedDiscover::default()),
        following: Mutex::new(None),
        other_categories: Mutex::new(None),
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
            if recommended.home_mounted() {
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

    // Unified Discover directories: only while a Home shows every platform and
    // its window is actually up. The first fetch comes from the mount itself,
    // and bringing the window back refetches through `note_main_window_focused`.
    let discover = inner.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(DISCOVER_TICK).await;
            if discover.unified_mounted() && main_window_shown(&discover.app) {
                refresh_other_categories(&discover).await;
                refresh_directories(&discover, unified_discover::PERIOD_SECS).await;
            }
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
/// lists (category grid, search results). Refreshes hype trains and
/// collaborations right away when the set gained ids the last poll did not
/// cover.
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
        refresh_collaborations(inner).await;
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
    collaborations_before_list(inner).await;
    emit(&inner.app, HomeUpdate::Recommended { streams: merged, cursor: next, at });
    // The next page ranks into the unified list too.
    rebuild_discover(inner).await;
    Ok(())
}

/// A window was destroyed. Its page ran no React cleanup, so drop whatever its
/// Home and Sidebar still claim: Go Live, and closing to the tray under
/// "Always", destroy the main window with both mounted.
pub fn release_window(window: &str) {
    let Some(inner) = SERVICE.get() else { return };
    let released = inner.claims().remove(window).is_some();
    if released {
        debug!("[HomeSnapshot] released the claims of destroyed window {window}");
    }
}

/// The Sidebar in `window` shows its second section for `scope` (`"all"`, or
/// one provider id), or shows none (`None`). A scope is always sent its list,
/// even when its rows match the last one sent, since this may be a page that
/// has none yet; then the directories it reads are fetched if due.
pub async fn set_sidebar(window: &str, context: &str, scope: Option<&str>) {
    let Some(inner) = SERVICE.get() else { return };
    claim_sidebar(&mut inner.claims(), window, context, scope);
    if scope.is_none() {
        return;
    }
    inner.discover.lock().await.forget(Surface::Sidebar);
    let inner = inner.clone();
    tauri::async_runtime::spawn(async move {
        rebuild_discover(&inner).await;
        refresh_directories(&inner, unified_discover::MOUNT_STALE_SECS).await;
    });
}

/// The main window came to the front. Nothing is fetched while it is minimized
/// or in the tray, so the directories the Discover lists read that went stale
/// in the meantime are refetched now, not on the next tick.
pub fn note_main_window_focused() {
    let Some(inner) = SERVICE.get() else { return };
    if !inner.discover_wanted() {
        return;
    }
    let inner = inner.clone();
    tauri::async_runtime::spawn(async move {
        refresh_other_categories(&inner).await;
        refresh_directories(&inner, unified_discover::MOUNT_STALE_SECS).await;
    });
}

/// Something the Following lists read, or the Discover lists leave out,
/// changed outside this module: a platform's live follows, the live
/// favourites, the favourite list or the follow list. The Following lists are
/// rebuilt every time, since the Sidebar shows them whenever it is up and a
/// build is a few hundred rows in memory. The Discover lists only while some
/// surface shows one, since the next one to arrive rebuilds anyway.
pub fn note_discover_inputs_changed() {
    let Some(inner) = SERVICE.get() else { return };
    // Already scheduled: that rebuild will see this change too.
    if DISCOVER_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let inner = inner.clone();
    tauri::async_runtime::spawn(async move {
        // Cleared BEFORE the rebuild, so a change arriving during it schedules
        // the next one instead of being swallowed.
        DISCOVER_PENDING.store(false, Ordering::Release);
        rebuild_following(&inner).await;
        if inner.discover_wanted() {
            rebuild_discover(&inner).await;
        }
    });
}

/// The main window, the only one with a Home, is up: neither hidden to the
/// tray nor minimized, with its Home still mounted and nobody looking at it.
/// A destroyed main window is not up either.
fn main_window_shown(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|window| {
        window.is_visible().unwrap_or(true) && !window.is_minimized().unwrap_or(false)
    })
}

/// `Settings.favorite_streamers`, copied out from under the lock.
fn favorite_ids(app: &AppHandle) -> Vec<String> {
    let Some(state) = app_state(app) else { return Vec::new() };
    let ids = state
        .settings
        .lock()
        .map(|s| s.favorite_streamers.clone())
        .unwrap_or_default();
    ids
}

/// `Settings.provider_follows` and the favourite identities, copied out from
/// under the lock.
fn follow_settings(app: &AppHandle) -> (Vec<ProviderFollow>, Vec<FavoriteChannel>) {
    let Some(state) = app_state(app) else { return (Vec::new(), Vec::new()) };
    let lists = state
        .settings
        .lock()
        .map(|s| (s.provider_follows.clone(), s.favorite_channels.clone()))
        .unwrap_or_default();
    lists
}

/// Rebuild the Following lists and emit them if they changed. Every input is
/// read from its owner on every call, the same ones the Discover lists
/// subtract, so the two never disagree about a channel.
async fn rebuild_following(inner: &Inner) {
    let mut last = inner.following.lock().await;
    let provider_followed_live = provider_live_service::snapshot().await;
    let favorites_live = favorite_live_service::snapshot().await;
    let favorite_ids = favorite_ids(&inner.app);
    let (provider_follows, favorite_channels) = follow_settings(&inner.app);
    let following = {
        let s = inner.snap.read().await;
        unified_following::build(&unified_following::Inputs {
            followed_live: &s.followed_live,
            followed_offline: &s.offline_follows,
            provider_followed_live: &provider_followed_live,
            favorites_live: &favorites_live,
            provider_follows: &provider_follows,
            favorite_ids: &favorite_ids,
            favorite_channels: &favorite_channels,
        })
    };
    if last.as_ref() == Some(&following) {
        return;
    }
    *last = Some(following.clone());
    let Following { favorites, live, offline } = following;
    emit(&inner.app, HomeUpdate::Following { favorites, live, offline, at: now_secs() });
}

/// The "On other platforms" row as it stands: every other platform's cached
/// categories, in platform order.
fn other_categories_now() -> Vec<ProviderCategory> {
    PROVIDER_IDS
        .iter()
        .filter(|provider| **provider != "twitch")
        .flat_map(|provider| provider_categories::cached(provider, OTHER_CATEGORIES_PER_PROVIDER))
        .collect()
}

/// Emit the "On other platforms" row if it changed.
async fn rebuild_other_categories(inner: &Inner) {
    let mut last = inner.other_categories.lock().await;
    let row = other_categories_now();
    if last.as_ref().is_some_and(|sent| same(sent, &row)) {
        return;
    }
    *last = Some(row.clone());
    emit(&inner.app, HomeUpdate::OtherCategories { categories: row, at: now_secs() });
}

/// Fetch every platform's categories that are due, each on its own task, and
/// emit the row as each lands. Only while a Home shows every platform.
async fn refresh_other_categories(inner: &Arc<Inner>) {
    if !inner.unified_mounted() {
        return;
    }
    for (provider, _) in registry().await.sources() {
        if !provider_categories::is_stale(provider) {
            continue;
        }
        let inner = inner.clone();
        tauri::async_runtime::spawn(async move {
            match provider_categories::get(provider, OTHER_CATEGORIES_PER_PROVIDER).await {
                Ok(_) => rebuild_other_categories(&inner).await,
                // A platform with no category taxonomy answers with an error at once.
                Err(e) => debug!("[HomeSnapshot] {provider} categories: {e}"),
            }
        });
    }
}

/// Rebuild the Discover list of every surface showing one, and emit each that
/// changed. Every input is read from its owner on every call, so nothing here
/// holds a copy that could drift from what the Following tab and the
/// Favourites section show.
async fn rebuild_discover(inner: &Inner) {
    let home = inner.unified_mounted();
    let sidebar = inner.sidebar_scope();
    if !home && sidebar.is_none() {
        return;
    }
    let mut discover = inner.discover.lock().await;
    let provider_followed_live = provider_live_service::snapshot().await;
    let favorites_live = favorite_live_service::snapshot().await;
    let favorite_ids = favorite_ids(&inner.app);
    let (home_list, sidebar_list) = {
        let s = inner.snap.read().await;
        let inputs = unified_discover::Inputs {
            recommended: &s.recommended,
            followed_live: &s.followed_live,
            provider_followed_live: &provider_followed_live,
            favorites_live: &favorites_live,
            favorite_ids: &favorite_ids,
        };
        let home_list = if home {
            discover.rebuild(Surface::Home, View::home(), &inputs)
        } else {
            None
        };
        let sidebar_list = sidebar.as_deref().and_then(|scope| {
            discover
                .rebuild(Surface::Sidebar, View::sidebar(scope), &inputs)
                .map(|streams| (scope.to_string(), streams))
        });
        (home_list, sidebar_list)
    };
    let at = now_secs();
    if let Some(streams) = home_list {
        emit(&inner.app, HomeUpdate::UnifiedDiscover { streams, at });
    }
    if let Some((scope, streams)) = sidebar_list {
        emit(&inner.app, HomeUpdate::SidebarDiscover { scope, streams, at });
    }
}

/// Fetch every directory a surface on screen reads that is due, each on its
/// own task, so each lands (and is emitted) as soon as its platform answers
/// and none waits for the slowest. `max_age` is how old a directory may be
/// before it is due.
async fn refresh_directories(inner: &Arc<Inner>, max_age: u64) {
    let home = inner.unified_mounted();
    let sidebar = inner.sidebar_scope();
    let wanted = |provider: &str| {
        (home && View::home().reads(provider))
            || sidebar
                .as_deref()
                .is_some_and(|scope| View::sidebar(scope).reads(provider))
    };
    let now = now_secs();
    let due: Vec<(&'static str, Arc<dyn StreamSource>)> = {
        let mut discover = inner.discover.lock().await;
        registry()
            .await
            .sources()
            .filter(|(provider, source)| source.caps().directory && wanted(provider))
            .filter(|(provider, _)| discover.begin_fetch(provider, now, max_age))
            .collect()
    };
    for (provider, source) in due {
        let inner = inner.clone();
        tauri::async_runtime::spawn(async move {
            let page = tokio::time::timeout(
                DIRECTORY_TIMEOUT,
                source.directory(None, None, unified_discover::PER_PROVIDER),
            )
            .await;
            let rows = match page {
                Ok(Ok(page)) => Some(page.streams),
                Ok(Err(e)) => {
                    warn!("[HomeSnapshot] {provider} directory for Discover failed: {e}");
                    None
                }
                Err(_) => {
                    warn!(
                        "[HomeSnapshot] {provider} directory for Discover gave no answer in {} s",
                        DIRECTORY_TIMEOUT.as_secs()
                    );
                    None
                }
            };
            inner.discover.lock().await.finish_fetch(provider, rows, now_secs());
            rebuild_discover(&inner).await;
        });
    }
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

/// Rebuild Home's Continue Watching row from the local watch-position store.
/// No network, no Twitch call.
async fn refresh_continue_watching(inner: &Inner) {
    // The row exists to resume. With "Resume VODs where you left off" off, a
    // card promising "Resume at 1:23:45" would start from the top instead, so
    // the row stays empty rather than lying.
    //
    // Read on ONE line: `settings` is a std Mutex and its guard must not be
    // alive across the await below (same shape as streaming.rs's resume read).
    let enabled = match app_state(&inner.app) {
        Some(state) => state.settings.lock().unwrap().video_player.resume_vod_playback,
        None => true,
    };
    let items: Vec<ContinueWatchingItem> = if enabled {
        let rows = tokio::task::spawn_blocking(|| {
            crate::services::vod_progress_service::recent(CONTINUE_LIMIT)
        })
        .await
        .unwrap_or_default();
        rows.into_iter()
            .map(|p| ContinueWatchingItem {
                video_id: p.video_id,
                // A card should never show a bare lowercase login when a
                // display name is known.
                channel_name: if p.channel_name.is_empty() {
                    p.channel_login.clone()
                } else {
                    p.channel_name
                },
                channel_login: p.channel_login,
                title: p.title,
                thumbnail_url: p.thumbnail_url,
                position_secs: p.position_secs,
                duration_secs: p.duration_secs,
                profile_image_url: p.profile_image_url,
                partner: p.partner,
                game_name: p.game_name,
            })
            .collect()
    } else {
        Vec::new()
    };
    let at = now_secs();
    let changed = {
        let mut s = inner.snap.write().await;
        let changed = !same(&s.continue_watching, &items);
        s.continue_watching = items.clone();
        s.continue_watching_at = Some(at);
        changed
    };
    if changed {
        emit(&inner.app, HomeUpdate::ContinueWatching { items, at });
    }
}

/// Reconcile the stored watch positions against Twitch in one batched call:
/// drop VODs Twitch has deleted (the store keeps entries for 90 days, Twitch
/// keeps VODs for 7-60), pick up final lengths, and replace the processing
/// placeholder thumbnail a VOD watched mid-broadcast was left with.
async fn hydrate_continue_watching(inner: &Inner) {
    let (ids, missing_avatar) = {
        let s = inner.snap.read().await;
        let ids: Vec<String> = s.continue_watching.iter().map(|i| i.video_id.clone()).collect();
        // Avatar or category still blank: the store predates the hydrate that
        // fills them, or the VOD was recorded since the last one.
        let missing = s
            .continue_watching
            .iter()
            .any(|i| i.profile_image_url.is_empty() || i.game_name.is_empty());
        (ids, missing)
    };
    if ids.is_empty() {
        return;
    }
    {
        let period = if missing_avatar {
            CONTINUE_HYDRATE_RETRY
        } else {
            CONTINUE_HYDRATE_PERIOD
        };
        let hydrated = inner.continue_hydrated.lock().await;
        if hydrated.is_some_and(|t| t.elapsed() < period) {
            return;
        }
    }
    let (updates, gone) = match TwitchService::get_videos_by_ids(&ids).await {
        Ok(pair) => pair,
        Err(e) => {
            // Not stamped, so the next mount or manual refresh retries.
            debug!("[HomeSnapshot] continue-watching hydrate: {e}");
            return;
        }
    };
    *inner.continue_hydrated.lock().await = Some(Instant::now());
    let changed = tokio::task::spawn_blocking(move || {
        crate::services::vod_progress_service::reconcile(&updates, &gone)
    })
    .await
    .unwrap_or(false);
    if changed {
        refresh_continue_watching(inner).await;
    }
}

/// A watch position changed. Rebuilds the row on the TRAILING edge so the last
/// report of a session always lands (a leading-edge debounce drops it, leaving
/// a finished VOD on the row), and only while a Home is on screen: during
/// playback Home is usually unmounted and this returns immediately.
pub fn note_progress_changed() {
    let Some(inner) = SERVICE.get() else { return };
    if !inner.home_mounted() {
        return;
    }
    // Already scheduled: that rebuild will see this change too.
    if CONTINUE_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let inner = inner.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CONTINUE_COALESCE).await;
        // Cleared BEFORE the rebuild, so a change arriving during it schedules
        // the next one instead of being swallowed.
        CONTINUE_PENDING.store(false, Ordering::Release);
        refresh_continue_watching(&inner).await;
    });
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
///
/// The unified Discover list is built here rather than read back: a window
/// hydrates once, possibly before its event listener is registered, so it must
/// start from the list as it stands now, not from the last one emitted.
/// Building records nothing, so the next rebuild still emits any change.
pub async fn snapshot() -> HomeSnapshot {
    let Some(inner) = SERVICE.get() else { return HomeSnapshot::default() };
    let provider_followed_live = provider_live_service::snapshot().await;
    let favorites_live = favorite_live_service::snapshot().await;
    let favorite_ids = favorite_ids(&inner.app);
    let discover = inner.discover.lock().await;
    let mut snap = inner.snap.read().await.clone();
    let unified = discover.build(
        &unified_discover::Inputs {
            recommended: &snap.recommended,
            followed_live: &snap.followed_live,
            provider_followed_live: &provider_followed_live,
            favorites_live: &favorites_live,
            favorite_ids: &favorite_ids,
        },
        View::home(),
    );
    snap.unified_discover = unified;
    snap.unified_discover_at = Some(now_secs());
    drop(discover);
    let (provider_follows, favorite_channels) = follow_settings(&inner.app);
    snap.following = unified_following::build(&unified_following::Inputs {
        followed_live: &snap.followed_live,
        followed_offline: &snap.offline_follows,
        provider_followed_live: &provider_followed_live,
        favorites_live: &favorites_live,
        provider_follows: &provider_follows,
        favorite_ids: &favorite_ids,
        favorite_channels: &favorite_channels,
    });
    snap.following_at = Some(now_secs());
    snap.other_categories = other_categories_now();
    snap.other_categories_at = Some(now_secs());
    snap
}

/// A Home in `window` mounted (`true`) or unmounted (`false`), from the page
/// `context`, and whether it shows every platform (`unified`). On mount,
/// sections that are stale or empty refresh right away so the grid is current
/// within a round trip instead of waiting for their next tick. A unified mount
/// also rebuilds the Discover list from what Rust already holds, which needs no
/// network, then fetches the directories that are due, each landing on its own.
pub async fn set_home_mounted(window: &str, context: &str, mounted: bool, unified: bool) {
    let Some(inner) = SERVICE.get() else { return };
    claim_home(&mut inner.claims(), window, context, mounted, unified);
    if mounted {
        if unified {
            let inner = inner.clone();
            tauri::async_runtime::spawn(async move {
                rebuild_discover(&inner).await;
                refresh_other_categories(&inner).await;
                refresh_directories(&inner, unified_discover::MOUNT_STALE_SECS).await;
            });
        }
        let (offline_stale, recommended_stale, drops_stale, continue_stale, collab_stale) = {
            let s = inner.snap.read().await;
            (
                is_stale(s.offline_at, OFFLINE_PERIOD),
                is_stale(s.recommended_at, RECOMMENDED_PERIOD),
                is_stale(s.drops_at, DROPS_PERIOD),
                is_stale(s.continue_watching_at, CONTINUE_MOUNT_STALE),
                is_stale(s.collab_at, COLLAB_MOUNT_STALE),
            )
        };
        let inner = inner.clone();
        tauri::async_runtime::spawn(async move {
            // First: the row is local-only, so it paints before any network.
            if continue_stale {
                refresh_continue_watching(&inner).await;
            }
            // A stale recommended refresh brings collaborations with it.
            if recommended_stale {
                refresh_recommended(&inner).await;
            } else if collab_stale {
                refresh_collaborations(&inner).await;
            }
            let signed = inner.snap.read().await.followed_live_at.is_some();
            if offline_stale && signed {
                refresh_offline(&inner).await;
            }
            if drops_stale && signed {
                refresh_drops(&inner).await;
            }
            // Last: reconciling the row against Twitch is the least urgent
            // work here, and it needs the rebuild above to have run.
            hydrate_continue_watching(&inner).await;
        });
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
        "collaborations" => "collaborations",
        "watch_streaks" => "watch_streaks",
        "drops" => "drops",
        "continue_watching" => "continue_watching",
        "discover" => "discover",
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
        "collaborations" => refresh_collaborations(inner).await,
        // Explicit arm required: the catch-all below silently refreshes hype
        // trains instead, with no error, for any key added above but not here.
        "continue_watching" => {
            refresh_continue_watching(inner).await;
            hydrate_continue_watching(inner).await;
        }
        // The directories the Discover lists on screen read, where due. The
        // lists follow as each one lands.
        "discover" => refresh_directories(inner, unified_discover::MOUNT_STALE_SECS).await,
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
            // The last account's picks and follows just went with it.
            rebuild_discover(inner).await;
            rebuild_following(inner).await;
        }
        // The reset above zeroed every field, including Continue Watching.
        // That row is per-account INCLUDING the signed-out `anon` store, so a
        // sign-out changes WHICH row applies rather than deleting the concept;
        // rebuild it against the new owner. Without this the row blanks on
        // every 60 s poll while signed out, and the `had_data` guard above
        // means the windows are never even told.
        refresh_continue_watching(inner).await;
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
            collaborations_before_list(inner).await;
            if changed {
                emit(
                    &inner.app,
                    HomeUpdate::FollowedLive {
                        streams: streams.clone(),
                        at,
                    },
                );
                // The unified list leaves out whoever the Following tab shows.
                rebuild_discover(inner).await;
                rebuild_following(inner).await;
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
        rebuild_following(inner).await;
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
    collaborations_before_list(inner).await;
    if changed {
        emit(&inner.app, HomeUpdate::Recommended { streams, cursor, at });
        rebuild_discover(inner).await;
    }
}

/// Twitch channel ids with a card on screen: followed live, recommended and
/// whatever a Home reported through `set_extra_channels`.
async fn card_channel_ids(inner: &Inner) -> Vec<String> {
    let s = inner.snap.read().await;
    let extra = inner.extra_hype_ids.read().await;
    let mut seen = HashSet::new();
    s.followed_live
        .iter()
        .chain(s.recommended.iter())
        .map(|st| st.user_id.clone())
        .chain(extra.iter().cloned())
        .filter(|id| !id.is_empty() && seen.insert(id.clone()))
        .collect()
}

/// Collaborations for the cards, right behind a list's viewer counts and ahead
/// of the list itself, so the windows hold a card's group before they draw the
/// card. Bounded by `COLLAB_WAIT`: a slow answer must never hold back the list
/// (or the live notifications behind the followed one); it is dropped and the
/// next pass fills in.
async fn collaborations_before_list(inner: &Inner) {
    if tokio::time::timeout(COLLAB_WAIT, refresh_collaborations(inner)).await.is_err() {
        debug!("[HomeSnapshot] collaborations: no answer in {COLLAB_WAIT:?}, sending the list without");
    }
}

/// Shared Viewership for every card on screen. Skipped while no window is on
/// screen; a Home mount catches up. A channel whose batch failed keeps the
/// group it had.
async fn refresh_collaborations(inner: &Inner) {
    if inner.app.webview_windows().is_empty() || crate::services::window_visibility::all_hidden() {
        return;
    }
    let ids = card_channel_ids(inner).await;
    let found = collaboration::fetch(&ids).await;
    let at = now_secs();
    let (changed, collabs) = {
        let mut s = inner.snap.write().await;
        let mut next: HashMap<String, Collaboration> = HashMap::new();
        for id in &ids {
            match found.get(id) {
                Some(Some(c)) => {
                    next.insert(id.clone(), c.clone());
                }
                Some(None) => {}
                None => {
                    if let Some(c) = s.collaborations.get(id) {
                        next.insert(id.clone(), c.clone());
                    }
                }
            }
        }
        let changed = next != s.collaborations;
        s.collaborations = next.clone();
        s.collab_at = Some(at);
        (changed, next)
    };
    if changed {
        emit(&inner.app, HomeUpdate::Collaborations { collabs, at });
    }
}

async fn refresh_hype(inner: &Inner) {
    let ids = card_channel_ids(inner).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// (a Home is mounted, a unified Home is mounted)
    fn up(homes: &HashMap<String, WindowClaims>) -> (bool, bool) {
        (
            homes.values().any(|w| w.mounted > 0),
            homes.values().any(|w| w.unified > 0),
        )
    }

    #[test]
    fn a_view_switch_lands_the_same_in_either_order() {
        // Switching view re-runs the effect: unmount as the old view, mount as
        // the new one. Two async commands, so they can land either way round.
        for unmount_first in [true, false] {
            let mut homes = HashMap::new();
            claim_home(&mut homes, "main", "page", true, true);
            let mut calls = vec![(false, true), (true, false)];
            if !unmount_first {
                calls.reverse();
            }
            for (mounted, unified) in calls {
                claim_home(&mut homes, "main", "page", mounted, unified);
            }
            assert_eq!(up(&homes), (true, false), "unmount first: {unmount_first}");
        }
    }

    #[test]
    fn a_reload_replaces_the_claims_its_old_page_never_released() {
        let mut homes = HashMap::new();
        claim_home(&mut homes, "main", "page-1", true, true);
        // Reloaded, now on the Twitch view. Page 1's Home never unmounted.
        claim_home(&mut homes, "main", "page-2", true, false);
        assert_eq!(up(&homes), (true, false));
        // A late unmount from the dead page changes nothing.
        claim_home(&mut homes, "main", "page-1", false, true);
        assert_eq!(up(&homes), (true, false));
        claim_home(&mut homes, "main", "page-2", false, false);
        assert_eq!(up(&homes), (false, false));
    }

    #[test]
    fn a_reload_drops_the_old_page_s_sidebar_claim_too() {
        let mut claims = HashMap::new();
        claim_home(&mut claims, "main", "page-1", true, true);
        claim_sidebar(&mut claims, "main", "page-1", Some("all"));
        // Reloaded: the new page's Sidebar arrives first, scoped to Kick, and
        // the old page's Home and Sidebar go with it.
        claim_sidebar(&mut claims, "main", "page-2", Some("kick"));
        assert_eq!(up(&claims), (false, false));
        assert_eq!(claims["main"].sidebar.as_deref(), Some("kick"));
        // The dead page's Sidebar letting go changes nothing.
        claim_sidebar(&mut claims, "main", "page-1", None);
        assert_eq!(claims["main"].sidebar.as_deref(), Some("kick"));
        // The live page's does.
        claim_sidebar(&mut claims, "main", "page-2", None);
        assert_eq!(claims["main"].sidebar, None);
    }

    #[test]
    fn go_live_and_back_leaves_no_claim_behind() {
        let mut homes = HashMap::new();
        claim_home(&mut homes, "main", "page-1", true, true);
        // Go Live destroys the main window; `release_window` drops its entry.
        homes.remove("main");
        assert_eq!(up(&homes), (false, false));
        // The window is recreated, and its Home leaves again normally.
        claim_home(&mut homes, "main", "page-2", true, true);
        claim_home(&mut homes, "main", "page-2", false, true);
        assert_eq!(up(&homes), (false, false));
    }
}
