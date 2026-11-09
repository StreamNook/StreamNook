//! Starting and stopping a stream in the main watch view, as one call.
//!
//! A Twitch start resolves the stream (the relay), completes the row
//! (Following's row, else the caller's seed with the gaps filled by one lookup,
//! never a made-up start time), aims drops and channel points at the channel,
//! and hands it to the watch session. A start on another platform (Kick,
//! YouTube, TikTok) resolves its watch URL and completes the row from the
//! platform's channel facts instead, lets drops and points go of any Twitch
//! channel before, and starts the session's provider live check. A newer
//! start, a stop, or any other session start (an offline room) supersedes a
//! start still resolving: the superseded one touches nothing and says so.
//!
//! Chat stays the caller's: the desktop and Android webview warm the channel
//! with `start_chat` for its window before calling here, and the iPhone opens
//! the stream's room in its chat tabs first.

use crate::commands::streaming::{self, StartError, StreamStartResult};
use crate::models::provider_stream::ProviderStream;
use crate::models::settings::AppState;
use crate::models::stream::TwitchStream;
use crate::services::offline_room::OfflineRoom;
use crate::services::providers::key::{make_key, normalize_channel};
use crate::services::providers::watch_urls::watch_url;
use crate::rt::AppHandle;
use crate::services::twitch_service::TwitchService;
use crate::services::unified_discover::DiscoverRow;
use crate::services::watch_session::{self, WatchTarget};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{Manager, State};

static TICKET: AtomicU64 = AtomicU64::new(0);
/// The latest ticket belongs to a stop, so a relay a superseded start brought
/// up is nobody's and goes down.
static STOPPED: AtomicBool = AtomicBool::new(true);
/// The ticket of the start still resolving, or 0: closing MultiNook leaves
/// earning to it.
static IN_FLIGHT: AtomicU64 = AtomicU64::new(0);
/// Every move of earning (drops and points) by a start or by closing the grid
/// runs under this, so neither replaces the other's halfway.
static EARNING_AIM: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Closing the grid found a start in flight and left earning to it. If that
/// start then fails or is superseded it never aims earning, so once it ends,
/// however it ends, the grid's exit is decided again.
static GRID_EXIT_DEFERRED: AtomicBool = AtomicBool::new(false);

/// Held by a start until it finishes, however it finishes.
struct PendingStart(u64);

impl Drop for PendingStart {
    fn drop(&mut self) {
        let mine = IN_FLIGHT
            .compare_exchange(self.0, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        // A newer start still in flight inherits the deferred exit instead.
        if mine && GRID_EXIT_DEFERRED.swap(false, Ordering::AcqRel) {
            if let Some(app) = crate::services::watch_drops::app() {
                tauri::async_runtime::spawn(async move { earning_after_grid(&app).await });
            }
        }
    }
}

/// Something newer than any start in flight began.
pub fn supersede() {
    TICKET.fetch_add(1, Ordering::AcqRel);
}

fn take_ticket() -> u64 {
    STOPPED.store(false, Ordering::Release);
    let ticket = TICKET.fetch_add(1, Ordering::AcqRel) + 1;
    IN_FLIGHT.store(ticket, Ordering::Release);
    ticket
}

fn current(ticket: u64) -> bool {
    TICKET.load(Ordering::Acquire) == ticket
}

/// `ticket` is still the latest, and from now on it is not, in one step. A
/// start takes the watch session this way, so a newer start that took a ticket
/// during its awaits is never superseded by an older one.
pub(crate) fn claim(ticket: u64) -> bool {
    claim_in(&TICKET, ticket)
}

fn claim_in(counter: &AtomicU64, ticket: u64) -> bool {
    counter.compare_exchange(ticket, ticket + 1, Ordering::AcqRel, Ordering::Acquire).is_ok()
}

/// What the caller already knows of the stream: a list row, a raid's ids, a
/// notification's login. Any field may be missing.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct WatchSeed {
    pub id: Option<String>,
    pub user_id: Option<String>,
    pub user_name: Option<String>,
    pub title: Option<String>,
    pub viewer_count: Option<u32>,
    pub game_id: Option<String>,
    pub game_name: Option<String>,
    pub thumbnail_url: Option<String>,
    pub started_at: Option<String>,
    pub broadcaster_type: Option<String>,
    pub profile_image_url: Option<String>,
    pub is_live: Option<bool>,
    pub tags: Option<Vec<String>>,
    pub language: Option<String>,
    pub drops_earnable: Option<bool>,
}

/// A finished start. `superseded` means a newer start or a stop won while this
/// one resolved: nothing was touched and the rest is absent.
#[derive(Debug, Clone, Serialize)]
pub struct WatchStart {
    pub superseded: bool,
    /// A Twitch row, or another platform's (`provider`, `key`, `watch_url`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<DiscoverRow>,
    /// The relay's answer; absent when the caller said the relay already serves
    /// the stream (a MultiNook tile promoted to the main view).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<StreamStartResult>,
}

impl WatchStart {
    fn superseded() -> Self {
        WatchStart { superseded: true, row: None, stream: None }
    }
}

fn filled(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// The seed with gaps a lookup must fill: no id, title, category, picture or
/// start time.
fn has_gaps(seed: &WatchSeed) -> bool {
    filled(&seed.user_id).is_none()
        || filled(&seed.title).is_none()
        || filled(&seed.game_name).is_none()
        || filled(&seed.profile_image_url).is_none()
        || filled(&seed.started_at).is_none()
}

/// Seed first, lookup second, field by field. The viewer count is the live
/// row's (a seed's is a raiding party or a search placeholder) and so is the
/// start time; a start is never invented, since empty shows no uptime.
pub fn merge_row(login: &str, seed: &WatchSeed, resolved: Option<&TwitchStream>) -> TwitchStream {
    let pick = |mine: &Option<String>, theirs: Option<&str>| -> String {
        filled(mine).map(str::to_string).or_else(|| theirs.filter(|v| !v.is_empty()).map(str::to_string)).unwrap_or_default()
    };
    let r = resolved;
    let title = pick(&seed.title, r.map(|r| r.title.as_str()));
    TwitchStream {
        id: pick(&seed.id, r.map(|r| r.id.as_str())),
        user_id: pick(&seed.user_id, r.map(|r| r.user_id.as_str())),
        user_name: {
            let name = pick(&seed.user_name, r.map(|r| r.user_name.as_str()));
            if name.is_empty() { login.to_string() } else { name }
        },
        user_login: login.to_lowercase(),
        title: if title.is_empty() { format!("Watching {login}") } else { title },
        viewer_count: r.map(|r| r.viewer_count).or(seed.viewer_count).unwrap_or(0),
        game_id: pick(&seed.game_id, r.map(|r| r.game_id.as_str())),
        game_name: pick(&seed.game_name, r.map(|r| r.game_name.as_str())),
        thumbnail_url: pick(&seed.thumbnail_url, r.map(|r| r.thumbnail_url.as_str())),
        started_at: r
            .map(|r| r.started_at.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| filled(&seed.started_at).map(str::to_string))
            .unwrap_or_default(),
        broadcaster_type: seed.broadcaster_type.clone().or_else(|| r.and_then(|r| r.broadcaster_type.clone())),
        profile_image_url: filled(&seed.profile_image_url)
            .map(str::to_string)
            .or_else(|| r.and_then(|r| r.profile_image_url.clone()).filter(|v| !v.is_empty())),
        is_live: r.and_then(|r| r.is_live).or(seed.is_live),
        tags: seed.tags.clone().or_else(|| r.and_then(|r| r.tags.clone())),
        language: seed.language.clone().or_else(|| r.and_then(|r| r.language.clone())),
        drops_earnable: seed.drops_earnable.or(r.map(|r| r.drops_earnable)).unwrap_or(false),
    }
}

/// The row the watch view shows: Following's own row when it lists the
/// channel, else the seed completed by one `resolve_stream_for_login`.
async fn assemble_row(login: &str, seed: &WatchSeed) -> TwitchStream {
    if let Some(row) = crate::services::home_snapshot::followed_live_row(login).await {
        return row;
    }
    let resolved = if has_gaps(seed) {
        match TwitchService::resolve_stream_for_login(login).await {
            Ok(row) => Some(row),
            Err(e) => {
                log::warn!("[WatchStart] could not resolve the row for {login}: {e}");
                None
            }
        }
    } else {
        None
    };
    merge_row(login, seed, resolved.as_ref())
}

/// Start watching a live channel. `provider` names its platform (Twitch when
/// absent); `login` is the Twitch login or the platform's channel (a Kick slug).
/// `quality` defaults to the saved one; `relay_ready` skips the relay (it
/// already serves this channel). `view` is the picture a Twitch start wants
/// (SN-40: "landscape" | "vertical", absent for the remembered pick).
pub async fn start(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: Option<String>,
    login: String,
    seed: Option<WatchSeed>,
    quality: Option<String>,
    relay_ready: bool,
    view: Option<String>,
) -> Result<WatchStart, StartError> {
    let provider = provider.map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty() && p != "twitch");
    if let Some(provider) = provider {
        return start_provider(app, state, provider, login, seed, quality, relay_ready).await;
    }
    let login = login.trim().trim_start_matches('#').to_lowercase();
    if login.is_empty() {
        return Err(StartError::Message("No channel to watch".to_string()));
    }
    let ticket = take_ticket();
    let pending = PendingStart(ticket);
    let seed = seed.unwrap_or_default();
    let quality = quality
        .filter(|q| !q.trim().is_empty())
        .unwrap_or_else(|| state.settings.lock().unwrap_or_else(|e| e.into_inner()).quality.clone());

    // The row resolves alongside the relay, so a seed with gaps costs no time.
    let relay = async {
        if relay_ready {
            None
        } else {
            Some(streaming::start_stream(format!("https://twitch.tv/{login}"), quality, view, state).await)
        }
    };
    let (started, row) = tokio::join!(relay, assemble_row(&login, &seed));
    let stream = match started {
        None => None,
        Some(started) => {
            if !current(ticket) {
                // A stop came after: this relay is nobody's. A newer start owns
                // the relay already (one solo relay), so it is left alone.
                if started.is_ok() && STOPPED.load(Ordering::Acquire) {
                    let _ = streaming::stop_stream().await;
                }
                log::debug!("[WatchStart] {login}: superseded while resolving");
                return Ok(WatchStart::superseded());
            }
            Some(started?)
        }
    };
    if !current(ticket) {
        return Ok(WatchStart::superseded());
    }
    // Earning and the watch session follow the answer, so the first frame
    // never waits on them.
    let (app_bg, row_bg, login_bg) = (app.clone(), row.clone(), login.clone());
    tauri::async_runtime::spawn(async move {
        follow(app_bg, ticket, login_bg, row_bg).await;
        drop(pending);
    });
    log::info!("[WatchStart] {login}: watching");
    Ok(WatchStart { superseded: false, row: Some(DiscoverRow::Twitch(row)), stream })
}

/// The row a stream on another platform shows: the platform's channel facts
/// where it has them, the caller's seed where it does not. Never a made-up
/// start time; the viewer count is the platform's live one.
pub fn provider_row(provider: &str, channel: &str, seed: &WatchSeed, meta: Option<&ProviderStream>) -> ProviderStream {
    let pick = |theirs: Option<&str>, mine: &Option<String>| -> String {
        theirs
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .or_else(|| filled(mine).map(str::to_string))
            .unwrap_or_default()
    };
    let m = meta;
    let name = pick(m.map(|m| m.user_name.as_str()), &seed.user_name);
    ProviderStream {
        provider: provider.to_string(),
        key: make_key(provider, channel),
        id: pick(m.map(|m| m.id.as_str()), &seed.id),
        user_id: pick(m.map(|m| m.user_id.as_str()), &seed.user_id),
        user_login: channel.to_string(),
        user_name: if name.is_empty() { channel.to_string() } else { name },
        title: pick(m.map(|m| m.title.as_str()), &seed.title),
        viewer_count: m.map(|m| m.viewer_count).filter(|v| *v > 0).or(seed.viewer_count).unwrap_or(0),
        game_id: pick(m.map(|m| m.game_id.as_str()), &seed.game_id),
        game_name: pick(m.map(|m| m.game_name.as_str()), &seed.game_name),
        category_thumbnail: m.and_then(|m| m.category_thumbnail.clone()),
        thumbnail_url: pick(m.map(|m| m.thumbnail_url.as_str()), &seed.thumbnail_url),
        started_at: pick(m.map(|m| m.started_at.as_str()), &seed.started_at),
        profile_image_url: m
            .and_then(|m| m.profile_image_url.clone())
            .filter(|v| !v.is_empty())
            .or_else(|| filled(&seed.profile_image_url).map(str::to_string)),
        // The relay resolved it, so it is live whatever a stale seed said.
        is_live: true,
        watch_url: watch_url(provider, channel),
        tags: m.and_then(|m| m.tags.clone()).or_else(|| seed.tags.clone()),
    }
}

/// Start watching a channel on another platform: the relay on its watch URL
/// and the row from the platform's channel facts, side by side. Drops and
/// channel points are Twitch's, so they let go of any Twitch channel before;
/// the watch session then runs its provider live check. Chat stays the
/// caller's, as on Twitch.
async fn start_provider(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: String,
    channel: String,
    seed: Option<WatchSeed>,
    quality: Option<String>,
    relay_ready: bool,
) -> Result<WatchStart, StartError> {
    let channel = normalize_channel(&provider, channel.trim().trim_start_matches('@'));
    if channel.is_empty() {
        return Err(StartError::Message("No channel to watch".to_string()));
    }
    let ticket = take_ticket();
    let pending = PendingStart(ticket);
    let seed = seed.unwrap_or_default();
    let quality = quality
        .filter(|q| !q.trim().is_empty())
        .unwrap_or_else(|| state.settings.lock().unwrap_or_else(|e| e.into_inner()).quality.clone());
    let url = watch_url(&provider, &channel);
    let relay = async {
        if relay_ready {
            None
        } else {
            // Another platform has one picture: no view to ask for.
            Some(streaming::start_stream(url.clone(), quality, None, state.clone()).await)
        }
    };
    let meta = async {
        match crate::commands::provider_browse::provider_channel_meta(provider.clone(), channel.clone()).await {
            Ok(meta) => Some(meta),
            Err(e) => {
                // Best effort: a resolved stream never fails to play over it.
                log::warn!("[WatchStart] {provider}:{channel}: no channel facts: {e}");
                None
            }
        }
    };
    let (started, meta) = tokio::join!(relay, meta);
    let stream = match started {
        None => None,
        Some(started) => {
            if !current(ticket) {
                if started.is_ok() && STOPPED.load(Ordering::Acquire) {
                    let _ = streaming::stop_stream().await;
                }
                log::debug!("[WatchStart] {provider}:{channel}: superseded while resolving");
                return Ok(WatchStart::superseded());
            }
            Some(started?)
        }
    };
    if !current(ticket) {
        return Ok(WatchStart::superseded());
    }
    let row = provider_row(&provider, &channel, &seed, meta.as_ref());
    let (app_bg, row_bg) = (app.clone(), row.clone());
    tauri::async_runtime::spawn(async move {
        follow_provider(app_bg, ticket, row_bg).await;
        drop(pending);
    });
    log::info!("[WatchStart] {provider}:{channel}: watching");
    Ok(WatchStart { superseded: false, row: Some(DiscoverRow::Provider(row)), stream })
}

/// Drops and points let go of the Twitch channel before (a Kick stream earns
/// neither), then the watch session takes the stream, unless a newer start or
/// a stop came meanwhile.
async fn follow_provider(app: AppHandle, ticket: u64, row: ProviderStream) {
    let state = app.state::<AppState>();
    {
        let _aim = EARNING_AIM.lock().await;
        if !current(ticket) {
            return;
        }
        let previous = watch_session::twitch_channel_id();
        let _ = crate::commands::drops::stop_drops_monitoring(state.clone()).await;
        if let Some(id) = previous {
            let _ = streaming::unregister_active_channel(id, state.clone()).await;
        }
    }
    if !current(ticket) {
        return;
    }
    if !watch_session::start_current(
        app.clone(),
        WatchTarget {
            provider: row.provider.clone(),
            login: row.user_login.clone(),
            user_id: row.user_id.clone(),
            user_name: row.user_name.clone(),
            title: row.title.clone(),
            game_id: (!row.game_id.is_empty()).then(|| row.game_id.clone()),
            game_name: row.game_name.clone(),
            started_at: row.started_at.clone(),
            // EventSub is Twitch's; a provider session never subscribes.
            authenticated: false,
            offline: false,
            avatar_url: row.profile_image_url.clone().unwrap_or_default(),
            category_art: row.category_thumbnail.clone().unwrap_or_default(),
        },
        ticket,
    )
    .await
    {
        return;
    }
    log::info!("[WatchStart] {}: the watch session follows it", row.key);
}

/// Drops and channel points follow the channel on screen, then the watch
/// session takes it, unless a newer start or a stop came meanwhile. With no
/// broadcaster id the heartbeat cannot move, so it stops rather than keep
/// crediting the channel before.
async fn follow(app: AppHandle, ticket: u64, login: String, row: TwitchStream) {
    let state = app.state::<AppState>();
    {
        // Under the lock closing MultiNook re-aims under, so a grid closing
        // while this start resolves never clears what it set.
        let _aim = EARNING_AIM.lock().await;
        if !current(ticket) {
            return;
        }
        if row.user_id.is_empty() {
            log::warn!("[WatchStart] no broadcaster id for {login}; drops and points monitoring off for this session");
            let _ = crate::commands::drops::stop_drops_monitoring(state.clone()).await;
        } else {
            crate::commands::drops::monitor_watched_channel(&app, row.user_id.clone(), login.clone()).await;
            let _ = streaming::register_active_channel(row.user_id.clone(), state.clone()).await;
        }
    }
    if !current(ticket) {
        return;
    }
    let authenticated = state.twitch_auth.get_token().await.is_ok();
    if !watch_session::start_current(
        app.clone(),
        WatchTarget {
            provider: "twitch".to_string(),
            login: login.clone(),
            user_id: row.user_id.clone(),
            user_name: row.user_name.clone(),
            title: row.title.clone(),
            game_id: (!row.game_id.is_empty()).then(|| row.game_id.clone()),
            game_name: row.game_name.clone(),
            started_at: row.started_at.clone(),
            authenticated,
            offline: false,
            avatar_url: row.profile_image_url.clone().unwrap_or_default(),
            category_art: String::new(),
        },
        ticket,
    )
    .await
    {
        return;
    }
    // Playback resolved, so the channel is live even where Helix does not list
    // it yet: the chat's live-only rewards follow at once.
    crate::services::channel_state::note_live(&row).await;
    log::info!("[WatchStart] {login}: drops, points and the watch session follow it");
    // Can this broadcast be rewound, and where did the viewer leave its recording: the
    // cached lookup also files the archive for the live bookmark. Every app shares it here;
    // the desktop still reads the answer for its Rewind button (cached, so no second fetch).
    // The take above claimed `ticket`, which leaves the counter one past it
    // while this start is still the latest.
    if current(ticket + 1) {
        let _ = streaming::get_live_rewind_info(login.clone()).await;
    }
}

/// An offline room opened: the room's facts, or the live row when the card that
/// opened it was stale (the caller starts the stream instead), or nothing when
/// a newer start or a stop won meanwhile.
#[derive(Debug, Clone, Serialize)]
pub struct RoomOpen {
    pub superseded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub room: Option<OfflineRoom>,
    /// The channel is live after all: watch it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live: Option<TwitchStream>,
    /// The row the view shows for the room (the room's facts first, the seed's second).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<TwitchStream>,
}

/// The view's row for an offline room: what the room knows first, then the seed.
pub fn room_row(login: &str, room: Option<&OfflineRoom>, seed: &WatchSeed) -> TwitchStream {
    let pick = |mine: Option<&String>, seeded: &Option<String>| -> String {
        mine.filter(|v| !v.is_empty()).cloned().or_else(|| filled(seeded).map(str::to_string)).unwrap_or_default()
    };
    let name = pick(room.map(|r| &r.display_name), &seed.user_name);
    TwitchStream {
        id: String::new(),
        user_id: pick(room.map(|r| &r.user_id), &seed.user_id),
        user_name: if name.is_empty() { login.to_string() } else { name },
        user_login: login.to_string(),
        title: pick(room.and_then(|r| r.last_title.as_ref()), &seed.title),
        viewer_count: 0,
        game_id: String::new(),
        game_name: pick(room.and_then(|r| r.last_category.as_ref()), &seed.game_name),
        thumbnail_url: String::new(),
        started_at: String::new(),
        broadcaster_type: seed.broadcaster_type.clone(),
        profile_image_url: room
            .and_then(|r| r.avatar_url.clone())
            .or_else(|| filled(&seed.profile_image_url).map(str::to_string)),
        is_live: Some(false),
        tags: None,
        language: None,
        drops_earnable: false,
    }
}

/// Open a Twitch channel's offline room in the main view: its facts in one
/// answer, whatever played stops, and the watch session waits for the channel
/// to go live (not for `chat_only`, a live channel whose playback failed). A
/// stale card for a live channel answers with the live row instead.
pub async fn open_room(app: AppHandle, login: String, seed: Option<WatchSeed>, chat_only: bool) -> RoomOpen {
    let login = login.trim().trim_start_matches('#').to_lowercase();
    let ticket = take_ticket();
    let _pending = PendingStart(ticket);
    let seed = seed.unwrap_or_default();
    let mut room = match crate::services::offline_room::open(&login).await {
        Ok(room) => Some(room),
        Err(e) => {
            log::warn!("[WatchStart] offline room for {login}: {e}");
            None
        }
    };
    // Where the viewer left the latest broadcast, when resuming is on.
    if let Some(r) = room.as_mut().filter(|r| r.live.is_none() || chat_only) {
        let resume = app
            .state::<AppState>()
            .settings
            .lock()
            .map(|s| s.video_player.resume_vod_playback)
            .unwrap_or(false);
        if resume {
            crate::services::offline_room::attach_resume(r).await;
        }
    }
    if !current(ticket) {
        return RoomOpen { superseded: true, room: None, live: None, row: None };
    }
    if !chat_only {
        if let Some(mut live) = room.as_ref().and_then(|r| r.live.clone()) {
            if live.profile_image_url.as_deref().unwrap_or("").is_empty() {
                live.profile_image_url = room.as_ref().and_then(|r| r.avatar_url.clone());
            }
            return RoomOpen { superseded: false, room, live: Some(live), row: None };
        }
    }
    // Whatever the main view played (an ended stream, a recording) stops, and a
    // room earns nothing: drops and points let go of the channel before.
    let previous = watch_session::twitch_channel_id();
    let _ = streaming::stop_stream().await;
    {
        let state = app.state::<AppState>();
        let _ = crate::commands::drops::stop_drops_monitoring(state.clone()).await;
        if let Some(id) = previous {
            let _ = streaming::unregister_active_channel(id, state.clone()).await;
        }
    }
    let row = room_row(&login, room.as_ref(), &seed);
    if !current(ticket) {
        return RoomOpen { superseded: true, room: None, live: None, row: None };
    }
    if !chat_only {
        let authenticated = app.state::<AppState>().twitch_auth.get_token().await.is_ok();
        if !watch_session::start_current(
            app,
            WatchTarget {
                provider: "twitch".to_string(),
                login: login.clone(),
                user_id: row.user_id.clone(),
                user_name: row.user_name.clone(),
                title: row.title.clone(),
                game_id: None,
                game_name: row.game_name.clone(),
                started_at: String::new(),
                authenticated,
                offline: true,
                avatar_url: row.profile_image_url.clone().unwrap_or_default(),
                category_art: String::new(),
            },
            ticket,
        )
        .await
        {
            return RoomOpen { superseded: true, room: None, live: None, row: None };
        }
    } else {
        // Playback failed and only the chat opened: what the view watched
        // before is over, unless a start that took its ticket since owns it.
        watch_session::end_session(app, move || current(ticket)).await;
    }
    log::info!("[WatchStart] {login}: offline room open");
    RoomOpen { superseded: false, room, live: None, row: Some(row) }
}

/// Stop watching: the relay goes down, the watch session ends, and unless
/// MultiNook takes the channel over (`preserve_backend`), drops and channel
/// points stop following it. Any start still resolving is superseded.
pub async fn stop(app: AppHandle, state: State<'_, AppState>, preserve_backend: bool) {
    STOPPED.store(true, Ordering::Release);
    supersede();
    let channel_id = watch_session::twitch_channel_id();
    if let Err(e) = streaming::stop_stream().await {
        log::warn!("[WatchStart] stop_stream failed: {e}");
    }
    watch_session::stop(app.clone(), preserve_backend).await;
    if preserve_backend {
        return;
    }
    // A stop the viewer makes ends their sleep timer too (the timer's own stop finds it off).
    crate::services::sleep_timer::cancel(&app);
    let _ = crate::commands::drops::stop_drops_monitoring(state.clone()).await;
    if let Some(id) = channel_id {
        let _ = streaming::unregister_active_channel(id, state.clone()).await;
    }
}

/// Where earning goes once MultiNook's grid has closed or emptied.
#[derive(Debug, PartialEq)]
enum AfterGrid {
    /// A start in flight (or a grid open again) decides it.
    Skip,
    /// Back to the solo Twitch stream behind the grid: (id, login).
    Follow(String, String),
    /// Nothing behind it earns: drops, points and the socket stop.
    Stop,
}

fn after_grid(start_in_flight: bool, grid_open: bool, session: Option<&WatchTarget>) -> AfterGrid {
    if start_in_flight || grid_open {
        return AfterGrid::Skip;
    }
    match session {
        Some(t) if t.provider == "twitch" && !t.offline && !t.user_id.is_empty() => {
            AfterGrid::Follow(t.user_id.clone(), t.login.clone())
        }
        _ => AfterGrid::Stop,
    }
}

/// MultiNook closed or emptied: earning leaves the tile it last focused, for
/// the solo stream behind the grid, or stops with nothing behind it. A start
/// in flight aims earning itself, so this leaves it alone.
pub async fn earning_after_grid(app: &AppHandle) {
    let _aim = EARNING_AIM.lock().await;
    let state = app.state::<AppState>();
    loop {
        let in_flight = IN_FLIGHT.load(Ordering::Acquire) != 0;
        let grid_open = watch_session::grid_open();
        match after_grid(in_flight, grid_open, watch_session::current_target().as_ref()) {
            AfterGrid::Skip => {
                if in_flight && !grid_open {
                    // The start that ends next decides this exit again.
                    GRID_EXIT_DEFERRED.store(true, Ordering::Release);
                    // It may have ended between the check and the mark: decide now.
                    if IN_FLIGHT.load(Ordering::Acquire) == 0 && GRID_EXIT_DEFERRED.swap(false, Ordering::AcqRel) {
                        continue;
                    }
                }
            }
            AfterGrid::Follow(id, login) => crate::commands::drops::monitor_watched_channel(app, id, login).await,
            AfterGrid::Stop => {
                let _ = crate::commands::drops::stop_drops_monitoring(state.clone()).await;
            }
        }
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_wins_once_and_only_for_the_latest_ticket() {
        let counter = AtomicU64::new(5);
        assert!(!claim_in(&counter, 4), "an older ticket");
        assert!(claim_in(&counter, 5));
        assert!(!claim_in(&counter, 5), "taken once");
        assert_eq!(counter.load(Ordering::Acquire), 6);
    }

    fn resolved() -> TwitchStream {
        TwitchStream {
            id: "s1".into(),
            user_id: "42".into(),
            user_name: "Caster".into(),
            user_login: "caster".into(),
            title: "Live title".into(),
            viewer_count: 900,
            game_id: "g1".into(),
            game_name: "Game".into(),
            thumbnail_url: "thumb".into(),
            started_at: "2026-10-16T10:00:00Z".into(),
            broadcaster_type: Some("partner".into()),
            profile_image_url: Some("pic".into()),
            is_live: Some(true),
            tags: None,
            language: Some("en".into()),
            drops_earnable: false,
        }
    }

    #[test]
    fn seed_fields_win_and_the_lookup_fills_gaps() {
        let seed = WatchSeed { title: Some("Raid seed title".into()), viewer_count: Some(12), ..Default::default() };
        let row = merge_row("Caster", &seed, Some(&resolved()));
        assert_eq!(row.title, "Raid seed title");
        assert_eq!(row.user_id, "42");
        assert_eq!(row.viewer_count, 900, "the live count beats a raiding party's size");
        assert_eq!(row.user_login, "caster");
        assert_eq!(row.profile_image_url.as_deref(), Some("pic"));
    }

    #[test]
    fn a_failed_lookup_never_invents_a_start_or_a_title() {
        let row = merge_row("caster", &WatchSeed::default(), None);
        assert_eq!(row.started_at, "");
        assert_eq!(row.title, "Watching caster");
        assert_eq!(row.user_name, "caster");
        assert_eq!(row.viewer_count, 0);
    }

    #[test]
    fn blank_seed_fields_count_as_gaps() {
        let seed = WatchSeed {
            user_id: Some("42".into()),
            title: Some("  ".into()),
            game_name: Some("Game".into()),
            profile_image_url: Some("pic".into()),
            started_at: Some("2026-10-16T10:00:00Z".into()),
            ..Default::default()
        };
        assert!(has_gaps(&seed));
        let full = WatchSeed { title: Some("t".into()), ..seed };
        assert!(!has_gaps(&full));
    }

    #[test]
    fn a_room_row_takes_the_rooms_facts_then_the_seed() {
        let seed = WatchSeed { title: Some("seed title".into()), user_id: Some("7".into()), ..Default::default() };
        let row = room_row("caster", None, &seed);
        assert_eq!(row.title, "seed title");
        assert_eq!(row.user_id, "7");
        assert_eq!(row.user_name, "caster");
        assert_eq!(row.is_live, Some(false));
        assert_eq!(row.started_at, "", "an offline row never has a start");
    }

    fn kick_meta() -> ProviderStream {
        ProviderStream {
            provider: "kick".into(),
            key: "kick:xqc".into(),
            id: "99".into(),
            user_id: "676".into(),
            user_login: "xqc".into(),
            user_name: "xQc".into(),
            title: "Kick title".into(),
            viewer_count: 4200,
            game_id: "15".into(),
            game_name: "Just Chatting".into(),
            category_thumbnail: None,
            thumbnail_url: "https://images.kick.com/thumb.webp".into(),
            started_at: "2026-10-16T10:00:00Z".into(),
            profile_image_url: Some("https://files.kick.com/pic.webp".into()),
            is_live: true,
            watch_url: "https://kick.com/xqc".into(),
            tags: None,
        }
    }

    #[test]
    fn a_kick_row_takes_the_platforms_facts_and_keeps_its_own_key_space() {
        let seed = WatchSeed { title: Some("card title".into()), viewer_count: Some(10), ..Default::default() };
        let row = provider_row("kick", "xqc", &seed, Some(&kick_meta()));
        assert_eq!(row.provider, "kick");
        assert_eq!(row.key, "kick:xqc");
        assert_eq!(row.watch_url, "https://kick.com/xqc");
        assert_eq!(row.title, "Kick title", "the platform's live title beats the card's");
        assert_eq!(row.viewer_count, 4200);
        assert_eq!(row.user_id, "676");
        assert!(row.is_live);
        let json = serde_json::to_value(DiscoverRow::Provider(row)).unwrap();
        assert_eq!(json["provider"], "kick", "the shells read the platform off the row");
    }

    #[test]
    fn a_kick_row_without_facts_falls_back_to_the_seed_and_never_invents_a_start() {
        let seed = WatchSeed { user_name: Some("xQc".into()), viewer_count: Some(10), ..Default::default() };
        let row = provider_row("kick", "xqc", &seed, None);
        assert_eq!(row.user_name, "xQc");
        assert_eq!(row.viewer_count, 10);
        assert_eq!(row.started_at, "");
        assert_eq!(row.title, "");
        let bare = provider_row("kick", "xqc", &WatchSeed::default(), None);
        assert_eq!(bare.user_name, "xqc");
    }

    #[test]
    fn a_twitch_row_serializes_as_it_did() {
        let json = serde_json::to_value(DiscoverRow::Twitch(resolved())).unwrap();
        assert!(json.get("provider").is_none());
        assert_eq!(json["user_login"], "caster");
    }

    #[test]
    fn a_stop_supersedes_a_start_in_flight() {
        let ticket = take_ticket();
        assert!(current(ticket));
        supersede();
        assert!(!current(ticket));
    }

    fn session(provider: &str, user_id: &str, offline: bool) -> WatchTarget {
        serde_json::from_value(serde_json::json!({
            "provider": provider, "login": "caster", "user_id": user_id, "offline": offline,
        }))
        .unwrap()
    }

    #[test]
    fn closing_the_grid_goes_back_to_the_solo_stream_or_stops() {
        let solo = session("twitch", "42", false);
        assert_eq!(after_grid(false, false, Some(&solo)), AfterGrid::Follow("42".into(), "caster".into()));
        assert_eq!(after_grid(false, false, None), AfterGrid::Stop);
        assert_eq!(after_grid(false, false, Some(&session("kick", "42", false))), AfterGrid::Stop);
        assert_eq!(after_grid(false, false, Some(&session("twitch", "42", true))), AfterGrid::Stop, "an offline room earns nothing");
        assert_eq!(after_grid(false, false, Some(&session("twitch", "", false))), AfterGrid::Stop);
    }

    #[test]
    fn a_start_in_flight_or_an_open_grid_decides_instead() {
        let solo = session("twitch", "42", false);
        assert_eq!(after_grid(true, false, Some(&solo)), AfterGrid::Skip);
        assert_eq!(after_grid(false, true, None), AfterGrid::Skip);
    }

    /// The tests below share the start marks with each other.
    static MARKS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn a_finished_start_clears_only_its_own_mark() {
        let _marks = MARKS.lock().unwrap_or_else(|e| e.into_inner());
        IN_FLIGHT.store(7, Ordering::Release);
        drop(PendingStart(6));
        assert_eq!(IN_FLIGHT.load(Ordering::Acquire), 7, "an older start leaves a newer one's mark");
        drop(PendingStart(7));
        assert_eq!(IN_FLIGHT.load(Ordering::Acquire), 0);
    }

    #[test]
    fn a_grid_exit_left_to_a_start_is_decided_again_when_that_start_ends() {
        let _marks = MARKS.lock().unwrap_or_else(|e| e.into_inner());
        IN_FLIGHT.store(9, Ordering::Release);
        GRID_EXIT_DEFERRED.store(true, Ordering::Release);
        // An older start ending is not the one the exit waited on.
        drop(PendingStart(8));
        assert!(GRID_EXIT_DEFERRED.load(Ordering::Acquire), "kept for the start still in flight");
        // The start it waited on ends (aimed, failed or superseded): the exit is taken up again.
        drop(PendingStart(9));
        assert!(!GRID_EXIT_DEFERRED.load(Ordering::Acquire));
        assert_eq!(IN_FLIGHT.load(Ordering::Acquire), 0);
    }
}
