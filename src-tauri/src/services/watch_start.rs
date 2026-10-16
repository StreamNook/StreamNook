//! Starting and stopping a Twitch stream in the main watch view, as one call.
//!
//! A start resolves the stream (the relay), completes the row (Following's row,
//! else the caller's seed with the gaps filled by one lookup, never a made-up
//! start time), aims drops and channel points at the channel, and hands it to
//! the watch session. A newer start, a stop, or any other session start (an
//! offline room, a provider stream) supersedes a start still resolving: the
//! superseded one touches nothing and says so.
//!
//! Chat stays the caller's: the desktop and Android webview warm the channel
//! with `start_chat` for its window before calling here, and the iPhone opens
//! the stream's room in its chat tabs first.

use crate::commands::streaming::{self, StartError, StreamStartResult};
use crate::models::settings::AppState;
use crate::models::stream::TwitchStream;
use crate::rt::AppHandle;
use crate::services::twitch_service::TwitchService;
use crate::services::watch_session::{self, WatchTarget};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::State;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row: Option<TwitchStream>,
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

/// Start watching a live Twitch channel. `quality` defaults to the saved one;
/// `relay_ready` skips the relay (it already serves this channel).
pub async fn start(
    app: AppHandle,
    state: State<'_, AppState>,
    login: String,
    seed: Option<WatchSeed>,
    quality: Option<String>,
    relay_ready: bool,
) -> Result<WatchStart, StartError> {
    let login = login.trim().trim_start_matches('#').to_lowercase();
    if login.is_empty() {
        return Err(StartError::Message("No channel to watch".to_string()));
    }
    let ticket = take_ticket();
    let seed = seed.unwrap_or_default();
    let quality = quality
        .filter(|q| !q.trim().is_empty())
        .unwrap_or_else(|| state.settings.lock().unwrap_or_else(|e| e.into_inner()).quality.clone());

    let stream = if relay_ready {
        None
    } else {
        let started = streaming::start_stream(format!("https://twitch.tv/{login}"), quality, state).await;
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
    };

    let row = assemble_row(&login, &seed).await;
    if !current(ticket) {
        return Ok(WatchStart::superseded());
    }

    // Drops and channel points follow the channel on screen. With no
    // broadcaster id the heartbeat cannot move, so it stops rather than keep
    // crediting the channel before.
    if row.user_id.is_empty() {
        log::warn!("[WatchStart] no broadcaster id for {login}; drops and points monitoring off for this session");
        let _ = crate::commands::drops::stop_drops_monitoring(state).await;
    } else {
        crate::commands::drops::monitor_watched_channel(&app, row.user_id.clone(), login.clone()).await;
        let _ = streaming::register_active_channel(row.user_id.clone(), state).await;
    }
    if !current(ticket) {
        return Ok(WatchStart::superseded());
    }

    let authenticated = state.twitch_auth.get_token().await.is_ok();
    watch_session::start(
        app,
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
    log::info!("[WatchStart] {login}: watching");
    Ok(WatchStart { superseded: false, row: Some(row), stream })
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
    let _ = crate::commands::drops::stop_drops_monitoring(state).await;
    if let Some(id) = channel_id {
        let _ = streaming::unregister_active_channel(id, state).await;
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
    fn a_stop_supersedes_a_start_in_flight() {
        let ticket = take_ticket();
        assert!(current(ticket));
        supersede();
        assert!(!current(ticket));
    }
}
