//! Rust-owned per-channel chat state: the live broadcast (viewer count, start,
//! title, category), channel points (balance,
//! custom name and icon, an available bonus claim) and pinned messages, for
//! every Twitch channel some window has a chat open on.
//!
//! Before this, every mounted ChatWidget ran three JavaScript timers per
//! channel (viewers 60 s, points 60 s, pinned 30 s), so two windows on one
//! channel polled it twice, and the viewer count was fetched from the WebView
//! straight to Helix with credentials the page asked Rust for. Now windows
//! register a watch (`watch_channel_state`, refcounted), Rust polls each
//! section on its own cadence and emits `channel-state` only when the
//! content changed; the viewer poll is one Helix call for every watched
//! channel at once. Nothing polls for a channel nobody is looking at.
//!
//! The same tick reads Shared Viewership (`collab`, see `services::collaboration`):
//! who a live channel is streaming with and the combined count, batched for
//! every watched live channel. It sleeps while no window is on screen.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::debug;
use serde::Serialize;
use crate::rt::AppHandle;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

use crate::models::settings::AppState;
use crate::services::collaboration::{self, Collaboration};
use crate::services::twitch_service::TwitchService;

pub const EVENT: &str = "channel-state";

const VIEWERS_PERIOD: Duration = Duration::from_secs(60);
const POINTS_PERIOD: Duration = Duration::from_secs(60);
const PINNED_PERIOD: Duration = Duration::from_secs(30);

#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct ChannelPoints {
    /// False when the channel has points off or the account cannot earn here.
    pub enabled: bool,
    pub balance: Option<i64>,
    pub name: Option<String>,
    pub icon_url: Option<String>,
    /// A bonus chest the viewer can claim right now.
    pub available_claim_id: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct ChannelState {
    pub login: String,
    pub channel_id: String,
    pub viewer_count: Option<u64>,
    /// When the current broadcast began (Helix RFC 3339), from the same poll
    /// as the viewer count. `None` while offline.
    pub started_at: Option<String>,
    /// The live broadcast's title and category, from the same poll. `None`
    /// while offline.
    pub title: Option<String>,
    pub game_name: Option<String>,
    pub viewers_at: Option<u64>,
    pub points: Option<ChannelPoints>,
    pub points_at: Option<u64>,
    pub pinned: Vec<serde_json::Value>,
    pub pinned_at: Option<u64>,
    pub collab: Option<Collaboration>,
    pub collab_at: Option<u64>,
    /// Polls in a row a live channel was missing from Helix's answer. One miss
    /// is often a glitch, so a live channel only reads offline after two.
    #[serde(skip)]
    offline_misses: u8,
}

#[derive(Serialize, Clone)]
#[serde(tag = "section", rename_all = "snake_case")]
pub enum ChannelUpdate {
    Viewers {
        login: String,
        viewer_count: Option<u64>,
        started_at: Option<String>,
        title: Option<String>,
        game_name: Option<String>,
        at: u64,
    },
    Points {
        login: String,
        points: Option<ChannelPoints>,
        at: u64,
    },
    Pinned {
        login: String,
        pinned: Vec<serde_json::Value>,
        at: u64,
    },
    Collab {
        login: String,
        collab: Option<Collaboration>,
        at: u64,
    },
}

struct Watched {
    channel_id: String,
    refs: usize,
    /// How many of `refs` want points and pinned messages too. A live-only
    /// watch (a docked chat's row in the chat list) rides the batched viewer
    /// poll alone, so twenty docked chats add no per-channel requests.
    full_refs: usize,
}

struct Inner {
    app: AppHandle,
    watched: RwLock<HashMap<String, Watched>>,
    state: RwLock<HashMap<String, ChannelState>>,
}

static SERVICE: OnceLock<Arc<Inner>> = OnceLock::new();

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn emit(app: &AppHandle, update: ChannelUpdate) {
    if let Err(e) = app.emit(EVENT, &update) {
        debug!("[ChannelState] emit failed: {e}");
    }
}

/// Start the pollers. Called once from the setup hook.
pub fn start(app: AppHandle) {
    let inner = Arc::new(Inner {
        app,
        watched: RwLock::new(HashMap::new()),
        state: RwLock::new(HashMap::new()),
    });
    if SERVICE.set(inner.clone()).is_err() {
        return;
    }
    let viewers = inner.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(VIEWERS_PERIOD).await;
            refresh_viewers(&viewers).await;
        }
    });
    let points = inner.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(POINTS_PERIOD).await;
            for login in fully_watched_logins(&points).await {
                refresh_points(&points, &login).await;
            }
        }
    });
    let pinned = inner;
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(PINNED_PERIOD).await;
            for login in fully_watched_logins(&pinned).await {
                refresh_pinned(&pinned, &login).await;
            }
        }
    });
}

async fn fully_watched_logins(inner: &Inner) -> Vec<String> {
    inner.watched.read().await.iter().filter(|(_, w)| w.full_refs > 0).map(|(k, _)| k.clone()).collect()
}

/// A window started showing chat for `login`. The first watcher triggers an
/// immediate refresh of every section it wants; the current state is returned
/// so the caller can paint without waiting for the events. `live_only` asks
/// for the live section (viewers, start, title, category) and nothing else.
pub async fn watch(login: &str, channel_id: &str, live_only: bool) -> ChannelState {
    let login = login.to_lowercase();
    let Some(inner) = SERVICE.get() else {
        return ChannelState { login, channel_id: channel_id.to_string(), ..Default::default() };
    };
    let full = usize::from(!live_only);
    let (first, first_full) = {
        let mut watched = inner.watched.write().await;
        match watched.get_mut(&login) {
            Some(w) => {
                w.refs += 1;
                w.full_refs += full;
                (false, full == 1 && w.full_refs == 1)
            }
            None => {
                watched.insert(
                    login.clone(),
                    Watched { channel_id: channel_id.to_string(), refs: 1, full_refs: full },
                );
                (true, full == 1)
            }
        }
    };
    if first {
        inner.state.write().await.insert(
            login.clone(),
            ChannelState { login: login.clone(), channel_id: channel_id.to_string(), ..Default::default() },
        );
    }
    if first || first_full {
        let inner = inner.clone();
        let l = login.clone();
        tauri::async_runtime::spawn(async move {
            if first {
                refresh_viewers(&inner).await;
            }
            if first_full {
                refresh_points(&inner, &l).await;
                refresh_pinned(&inner, &l).await;
            }
        });
    }
    inner.state.read().await.get(&login).cloned().unwrap_or_default()
}

/// A window stopped showing chat for `login`. The last watcher drops the state.
/// `live_only` matches the watch it ends.
pub async fn unwatch(login: &str, live_only: bool) {
    let login = login.to_lowercase();
    let Some(inner) = SERVICE.get() else { return };
    let gone = {
        let mut watched = inner.watched.write().await;
        match watched.get_mut(&login) {
            Some(w) if w.refs > 1 => {
                w.refs -= 1;
                if !live_only {
                    w.full_refs = w.full_refs.saturating_sub(1);
                }
                false
            }
            Some(_) => {
                watched.remove(&login);
                true
            }
            None => false,
        }
    };
    if gone {
        inner.state.write().await.remove(&login);
    }
}

/// Current state for a channel, or `None` if nobody watches it.
pub async fn get(login: &str) -> Option<ChannelState> {
    let inner = SERVICE.get()?;
    inner.state.read().await.get(&login.to_lowercase()).cloned()
}

/// Manual refresh of one section (`viewers`, `points`, `pinned`) for a
/// watched channel: after a pin, a claim, a spend.
pub async fn refresh(login: &str, section: &str) -> Result<(), String> {
    let inner = SERVICE.get().ok_or("channel state not started")?;
    let login = login.to_lowercase();
    if !inner.watched.read().await.contains_key(&login) {
        return Ok(());
    }
    match section {
        "viewers" => refresh_viewers(inner).await,
        "points" => refresh_points(inner, &login).await,
        "pinned" => refresh_pinned(inner, &login).await,
        other => return Err(format!("unknown channel section: {other}")),
    }
    Ok(())
}

/// One channel's live broadcast as the viewer poll read it; all `None` while
/// offline.
#[derive(Clone, Default, PartialEq)]
struct Live {
    viewer_count: Option<u64>,
    started_at: Option<String>,
    title: Option<String>,
    game_name: Option<String>,
}

/// Whether to keep showing a channel as live although this poll missed it,
/// and the miss count to carry. A live channel reads offline only after two
/// polls in a row leave it out.
fn hold_live(was_live: bool, live_now: bool, misses: u8) -> (bool, u8) {
    if !live_now && was_live && misses == 0 {
        (true, 1)
    } else {
        (false, 0)
    }
}

async fn refresh_viewers(inner: &Inner) {
    let by_login: HashMap<String, String> = inner
        .watched
        .read()
        .await
        .iter()
        .map(|(login, w)| (login.clone(), w.channel_id.clone()))
        .collect();
    if by_login.is_empty() {
        return;
    }
    let ids: Vec<String> = by_login.values().cloned().collect();
    let live: HashMap<String, Live> = match TwitchService::get_streams_by_user_ids(&ids).await {
        Ok(streams) => streams
            .into_iter()
            .map(|s| {
                let some = |v: String| Some(v).filter(|v| !v.is_empty());
                let live = Live {
                    viewer_count: Some(s.viewer_count as u64),
                    started_at: some(s.started_at),
                    title: some(s.title),
                    game_name: some(s.game_name),
                };
                (s.user_id, live)
            })
            .collect(),
        Err(e) => {
            debug!("[ChannelState] viewers: {e}");
            return;
        }
    };
    let at = now_secs();
    let mut changed: Vec<(String, Live)> = Vec::new();
    {
        let mut state = inner.state.write().await;
        for (login, id) in &by_login {
            let now = live.get(id).cloned().unwrap_or_default();
            if let Some(s) = state.get_mut(login) {
                let (hold, misses) = hold_live(s.viewer_count.is_some(), now.viewer_count.is_some(), s.offline_misses);
                s.offline_misses = misses;
                if hold {
                    s.viewers_at = Some(at);
                    continue;
                }
                let before = Live {
                    viewer_count: s.viewer_count,
                    started_at: s.started_at.clone(),
                    title: s.title.clone(),
                    game_name: s.game_name.clone(),
                };
                if before != now {
                    changed.push((login.clone(), now.clone()));
                }
                s.viewer_count = now.viewer_count;
                s.started_at = now.started_at;
                s.title = now.title;
                s.game_name = now.game_name;
                s.viewers_at = Some(at);
            }
        }
    }
    for (login, l) in changed {
        emit(
            &inner.app,
            ChannelUpdate::Viewers {
                login,
                viewer_count: l.viewer_count,
                started_at: l.started_at,
                title: l.title,
                game_name: l.game_name,
                at,
            },
        );
    }
    let live_logins: Vec<String> = by_login
        .iter()
        .filter(|(_, id)| live.contains_key(*id))
        .map(|(login, _)| login.clone())
        .collect();
    refresh_collab(inner, &by_login, &live_logins).await;
}

/// Shared Viewership for every watched live channel, in batched requests.
/// Skipped while no window is on screen, since only the UI reads it; the first
/// tick after a window returns catches up. A channel that went offline drops
/// its group; a failed batch keeps what its channels last had.
async fn refresh_collab(inner: &Inner, watched: &HashMap<String, String>, live_logins: &[String]) {
    if crate::services::window_visibility::all_hidden() {
        return;
    }
    let mut fresh: HashMap<String, Option<Collaboration>> = watched
        .keys()
        .filter(|login| !live_logins.contains(login))
        .map(|login| (login.clone(), None))
        .collect();
    let ids: Vec<String> = live_logins.iter().filter_map(|l| watched.get(l).cloned()).collect();
    let mut found = collaboration::fetch(&ids).await;
    for login in live_logins {
        if let Some(collab) = watched.get(login).and_then(|id| found.remove(id)) {
            fresh.insert(login.clone(), collab);
        }
    }
    let at = now_secs();
    let mut changed: Vec<(String, Option<Collaboration>)> = Vec::new();
    {
        let mut state = inner.state.write().await;
        for (login, collab) in fresh {
            if let Some(s) = state.get_mut(&login) {
                if s.collab != collab {
                    changed.push((login.clone(), collab.clone()));
                }
                s.collab = collab;
                s.collab_at = Some(at);
            }
        }
    }
    for (login, collab) in changed {
        emit(&inner.app, ChannelUpdate::Collab { login, collab, at });
    }
}

/// Parse the community-points GQL answer the way the widget used to: the
/// channel node lives under `community.channel` or `user.channel`; a null
/// `self.communityPoints` means points are off (or the account cannot earn).
fn parse_points(v: &serde_json::Value) -> Option<ChannelPoints> {
    let data = v.get("data")?;
    let channel = data
        .pointer("/community/channel")
        .or_else(|| data.pointer("/user/channel"))?;
    let community = channel.pointer("/self/communityPoints");
    let Some(community) = community.filter(|c| !c.is_null()) else {
        return Some(ChannelPoints { enabled: false, ..Default::default() });
    };
    let balance = community
        .get("balance")
        .and_then(|b| b.as_i64())
        .or_else(|| v.get("balance").and_then(|b| b.as_i64()));
    let settings = channel.get("communityPointsSettings");
    Some(ChannelPoints {
        enabled: true,
        balance,
        name: settings
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str())
            .filter(|n| !n.is_empty())
            .map(String::from),
        icon_url: settings
            .and_then(|s| s.pointer("/image/url"))
            .and_then(|u| u.as_str())
            .map(String::from),
        available_claim_id: community
            .pointer("/availableClaim/id")
            .and_then(|i| i.as_str())
            .map(String::from),
    })
}

async fn refresh_points(inner: &Inner, login: &str) {
    let channel_id = match inner.watched.read().await.get(login) {
        Some(w) => w.channel_id.clone(),
        None => return,
    };
    let raw = match crate::commands::drops::get_channel_points_for_channel(login.to_string()).await {
        Ok(v) => v,
        Err(e) => {
            debug!("[ChannelState] points for {login}: {e}");
            return;
        }
    };
    let Some(points) = parse_points(&raw) else {
        debug!("[ChannelState] points for {login}: unrecognised payload shape");
        return;
    };
    // Keep the drops ledger current from here, the way the widget did after
    // every successful poll.
    if let (Some(balance), Some(state)) = (points.balance, inner.app.try_state::<AppState>()) {
        let drops = state.drops_service.lock().await;
        let _ = drops
            .update_channel_points_balance(&channel_id, login, balance.clamp(0, i32::MAX as i64) as i32)
            .await;
    }
    // The minute fallback for a chest the socket did not push.
    if let Some(claim_id) = points.available_claim_id.clone() {
        crate::services::watched_chest::offer(&inner.app, channel_id.clone(), claim_id);
    }
    let at = now_secs();
    let changed = {
        let mut state = inner.state.write().await;
        let Some(s) = state.get_mut(login) else { return };
        let changed = s.points.as_ref() != Some(&points);
        s.points = Some(points.clone());
        s.points_at = Some(at);
        changed
    };
    if changed {
        emit(
            &inner.app,
            ChannelUpdate::Points { login: login.to_string(), points: Some(points), at },
        );
    }
}

async fn refresh_pinned(inner: &Inner, login: &str) {
    let channel_id = match inner.watched.read().await.get(login) {
        Some(w) => w.channel_id.clone(),
        None => return,
    };
    let pinned = match TwitchService::get_pinned_chat_messages(&channel_id).await {
        Ok(p) => p,
        Err(e) => {
            debug!("[ChannelState] pinned for {login}: {e}");
            return;
        }
    };
    let at = now_secs();
    let changed = {
        let mut state = inner.state.write().await;
        let Some(s) = state.get_mut(login) else { return };
        let changed = s.pinned != pinned;
        s.pinned = pinned.clone();
        s.pinned_at = Some(at);
        changed
    };
    if changed {
        emit(&inner.app, ChannelUpdate::Pinned { login: login.to_string(), pinned, at });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_live_channel_reads_offline_only_after_two_misses() {
        assert_eq!(hold_live(true, false, 0), (true, 1), "one miss is held");
        assert_eq!(hold_live(true, false, 1), (false, 0), "the second miss goes offline");
        assert_eq!(hold_live(true, true, 1), (false, 0), "back in the answer resets");
        assert_eq!(hold_live(false, false, 0), (false, 0), "an offline channel stays offline");
        assert_eq!(hold_live(false, true, 0), (false, 0), "going live shows at once");
    }

    #[test]
    fn parses_community_shape() {
        let v = json!({"data": {"community": {"channel": {
            "self": {"communityPoints": {"balance": 1234, "availableClaim": {"id": "c1"}}},
            "communityPointsSettings": {"name": "Nooks", "image": {"url": "https://x/y.png"}}
        }}}});
        let p = parse_points(&v).unwrap();
        assert!(p.enabled);
        assert_eq!(p.balance, Some(1234));
        assert_eq!(p.name.as_deref(), Some("Nooks"));
        assert_eq!(p.icon_url.as_deref(), Some("https://x/y.png"));
        assert_eq!(p.available_claim_id.as_deref(), Some("c1"));
    }

    #[test]
    fn parses_user_shape_and_disabled() {
        let v = json!({"data": {"user": {"channel": {"self": {"communityPoints": null}}}}});
        let p = parse_points(&v).unwrap();
        assert!(!p.enabled);
        assert_eq!(p.balance, None);
        assert!(parse_points(&json!({"data": {}})).is_none());
    }
}
