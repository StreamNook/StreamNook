//! Rust-owned per-channel chat state: viewer count, channel points (balance,
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

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::debug;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::RwLock;

use crate::models::settings::AppState;
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
    pub viewers_at: Option<u64>,
    pub points: Option<ChannelPoints>,
    pub points_at: Option<u64>,
    pub pinned: Vec<serde_json::Value>,
    pub pinned_at: Option<u64>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "section", rename_all = "snake_case")]
pub enum ChannelUpdate {
    Viewers {
        login: String,
        viewer_count: Option<u64>,
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
}

struct Watched {
    channel_id: String,
    refs: usize,
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
            for login in watched_logins(&points).await {
                refresh_points(&points, &login).await;
            }
        }
    });
    let pinned = inner;
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(PINNED_PERIOD).await;
            for login in watched_logins(&pinned).await {
                refresh_pinned(&pinned, &login).await;
            }
        }
    });
}

async fn watched_logins(inner: &Inner) -> Vec<String> {
    inner.watched.read().await.keys().cloned().collect()
}

/// A window started showing chat for `login`. The first watcher triggers an
/// immediate refresh of every section; the current state is returned so the
/// caller can paint without waiting for the events.
pub async fn watch(login: &str, channel_id: &str) -> ChannelState {
    let login = login.to_lowercase();
    let Some(inner) = SERVICE.get() else {
        return ChannelState { login, channel_id: channel_id.to_string(), ..Default::default() };
    };
    let first = {
        let mut watched = inner.watched.write().await;
        match watched.get_mut(&login) {
            Some(w) => {
                w.refs += 1;
                false
            }
            None => {
                watched.insert(
                    login.clone(),
                    Watched { channel_id: channel_id.to_string(), refs: 1 },
                );
                true
            }
        }
    };
    if first {
        inner.state.write().await.insert(
            login.clone(),
            ChannelState { login: login.clone(), channel_id: channel_id.to_string(), ..Default::default() },
        );
        let inner = inner.clone();
        let l = login.clone();
        tauri::async_runtime::spawn(async move {
            refresh_viewers(&inner).await;
            refresh_points(&inner, &l).await;
            refresh_pinned(&inner, &l).await;
        });
    }
    inner.state.read().await.get(&login).cloned().unwrap_or_default()
}

/// A window stopped showing chat for `login`. The last watcher drops the state.
pub async fn unwatch(login: &str) {
    let login = login.to_lowercase();
    let Some(inner) = SERVICE.get() else { return };
    let gone = {
        let mut watched = inner.watched.write().await;
        match watched.get_mut(&login) {
            Some(w) if w.refs > 1 => {
                w.refs -= 1;
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
    let live: HashMap<String, u64> = match TwitchService::get_streams_by_user_ids(&ids).await {
        Ok(streams) => streams
            .into_iter()
            .map(|s| (s.user_id, s.viewer_count as u64))
            .collect(),
        Err(e) => {
            debug!("[ChannelState] viewers: {e}");
            return;
        }
    };
    let at = now_secs();
    let mut changed: Vec<(String, Option<u64>)> = Vec::new();
    {
        let mut state = inner.state.write().await;
        for (login, id) in &by_login {
            let count = live.get(id).copied();
            if let Some(s) = state.get_mut(login) {
                if s.viewer_count != count {
                    changed.push((login.clone(), count));
                }
                s.viewer_count = count;
                s.viewers_at = Some(at);
            }
        }
    }
    for (login, viewer_count) in changed {
        emit(&inner.app, ChannelUpdate::Viewers { login, viewer_count, at });
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
