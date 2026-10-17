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

/// Something newer than any start in flight began.
pub fn supersede() {
    TICKET.fetch_add(1, Ordering::AcqRel);
}

fn take_ticket() -> u64 {
    STOPPED.store(false, Ordering::Release);
    TICKET.fetch_add(1, Ordering::AcqRel) + 1
}

fn current(ticket: u64) -> bool {
    TICKET.load(Ordering::Acquire) == ticket
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
/// already serves this channel).
pub async fn start(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: Option<String>,
    login: String,
    seed: Option<WatchSeed>,
    quality: Option<String>,
    relay_ready: bool,
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
    let seed = seed.unwrap_or_default();
    let quality = quality
        .filter(|q| !q.trim().is_empty())
        .unwrap_or_else(|| state.settings.lock().unwrap_or_else(|e| e.into_inner()).quality.clone());

    // The row resolves alongside the relay, so a seed with gaps costs no time.
    let relay = async {
        if relay_ready {
            None
        } else {
            Some(streaming::start_stream(format!("https://twitch.tv/{login}"), quality, state).await)
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
    tauri::async_runtime::spawn(async move { follow(app_bg, ticket, login_bg, row_bg).await });
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
    let seed = seed.unwrap_or_default();
    let quality = quality
        .filter(|q| !q.trim().is_empty())
        .unwrap_or_else(|| state.settings.lock().unwrap_or_else(|e| e.into_inner()).quality.clone());
    let url = watch_url(&provider, &channel);
    let relay = async {
        if relay_ready {
            None
        } else {
            Some(streaming::start_stream(url.clone(), quality, state.clone()).await)
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
    tauri::async_runtime::spawn(async move { follow_provider(app_bg, ticket, row_bg).await });
    log::info!("[WatchStart] {provider}:{channel}: watching");
    Ok(WatchStart { superseded: false, row: Some(DiscoverRow::Provider(row)), stream })
}

/// Drops and points let go of the Twitch channel before (a Kick stream earns
/// neither), then the watch session takes the stream, unless a newer start or
/// a stop came meanwhile.
async fn follow_provider(app: AppHandle, ticket: u64, row: ProviderStream) {
    let state = app.state::<AppState>();
    let previous = watch_session::twitch_channel_id();
    let _ = crate::commands::drops::stop_drops_monitoring(state.clone()).await;
    if let Some(id) = previous {
        let _ = streaming::unregister_active_channel(id, state.clone()).await;
    }
    if !current(ticket) {
        return;
    }
    watch_session::start(
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
        },
    )
    .await;
    log::info!("[WatchStart] {}: the watch session follows it", row.key);
}

/// Drops and channel points follow the channel on screen, then the watch
/// session takes it, unless a newer start or a stop came meanwhile. With no
/// broadcaster id the heartbeat cannot move, so it stops rather than keep
/// crediting the channel before.
async fn follow(app: AppHandle, ticket: u64, login: String, row: TwitchStream) {
    let state = app.state::<AppState>();
    if row.user_id.is_empty() {
        log::warn!("[WatchStart] no broadcaster id for {login}; drops and points monitoring off for this session");
        let _ = crate::commands::drops::stop_drops_monitoring(state.clone()).await;
    } else {
        crate::commands::drops::monitor_watched_channel(&app, row.user_id.clone(), login.clone()).await;
        let _ = streaming::register_active_channel(row.user_id.clone(), state.clone()).await;
    }
    if !current(ticket) {
        return;
    }
    let authenticated = state.twitch_auth.get_token().await.is_ok();
    watch_session::start(
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
        },
    )
    .await;
    log::info!("[WatchStart] {login}: drops, points and the watch session follow it");
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
        watch_session::start(
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
            },
        )
        .await;
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
    watch_session::stop(app, preserve_backend).await;
    if preserve_backend {
        return;
    }
    let _ = crate::commands::drops::stop_drops_monitoring(state.clone()).await;
    if let Some(id) = channel_id {
        let _ = streaming::unregister_active_channel(id, state.clone()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
