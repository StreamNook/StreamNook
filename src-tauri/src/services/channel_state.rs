//! Rust-owned per-channel chat state: the live broadcast (viewer count, start,
//! title, category), channel points (balance,
//! custom name and icon, an available bonus claim) and pinned messages, for
//! every Twitch channel some window has a chat open on, plus the wallet's bits
//! face (bits on here, the viewer's balance, the gem) and the last rise in
//! either currency, which the wallet button shows as a "+N".
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
//!
//! The viewer's relationship to the channel (follow, subscription, the badge
//! their next month wears) is read once when a chat opens, again every ten
//! minutes, and set at once by the viewer's own follow, unfollow or sub, so a
//! chat menu opens with it already known and never fills in on screen.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use log::debug;
use serde::Serialize;
use crate::rt::AppHandle;
use tauri::{Emitter, Manager};
use tokio::sync::RwLock;

use crate::models::settings::AppState;
use crate::models::stream::TwitchStream;
use crate::services::channel_relationship::{self, Relationship};
use crate::services::collaboration::{self, Collaboration};
use crate::services::shared_chat::{self, SharedChat};
use crate::services::twitch_service::TwitchService;

pub const EVENT: &str = "channel-state";

const VIEWERS_PERIOD: Duration = Duration::from_secs(60);
const POINTS_PERIOD: Duration = Duration::from_secs(60);
const PINNED_PERIOD: Duration = Duration::from_secs(30);
/// Follows and subs change through this app (set at once) or rarely elsewhere.
const RELATIONSHIP_PERIOD: Duration = Duration::from_secs(10 * 60);

/// A rise in one of the viewer's balances here, for the wallet's "+N".
#[derive(Serialize, Clone, PartialEq, Debug)]
pub struct WalletGain {
    /// Grows with every gain, so two equal amounts in a row still show twice.
    pub seq: u64,
    pub amount: i64,
    /// Why, in the viewer's words ("for watching"); None when Twitch did not
    /// say (a rise only the poll saw).
    pub reason: Option<String>,
    /// A bonus chest, for viewers who only want the "+N" for chests.
    pub chest: bool,
}

static GAIN_SEQ: AtomicU64 = AtomicU64::new(0);

/// A new gain; `code` is Twitch's reason code when it gave one.
fn gain(amount: i64, code: Option<&str>) -> WalletGain {
    WalletGain {
        seq: GAIN_SEQ.fetch_add(1, Ordering::Relaxed) + 1,
        amount,
        reason: code.and_then(gain_reason),
        chest: code.is_some_and(|c| c.eq_ignore_ascii_case("claim")),
    }
}

/// Twitch's `point_gain.reason_code`, worded for the hover card. Codes not
/// worded here stay unnamed rather than guessed.
pub fn gain_reason(code: &str) -> Option<String> {
    let code = code.to_ascii_uppercase();
    let words = match code.as_str() {
        "WATCH" => "for watching",
        "CLAIM" => "from the bonus chest",
        "WATCH_STREAK" => "for your watch streak",
        "RAID" => "for joining a raid",
        c if c.contains("PREDICTION") => "from a prediction",
        _ => return None,
    };
    Some(words.to_string())
}

/// The rise a poll saw that nothing reported: only against a known earlier
/// balance, never on a first read, never for a fall.
fn poll_rise(before: Option<i64>, now: Option<i64>) -> Option<i64> {
    match (before, now) {
        (Some(b), Some(n)) if n > b => Some(n - b),
        _ => None,
    }
}

#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct ChannelPoints {
    /// False when the channel has points off or the account cannot earn here.
    pub enabled: bool,
    pub balance: Option<i64>,
    pub name: Option<String>,
    pub icon_url: Option<String>,
    /// A bonus chest the viewer can claim right now.
    pub available_claim_id: Option<String>,
    /// The last rise in the balance, from any source.
    pub last_gain: Option<WalletGain>,
}

/// What the wallet button needs about bits on a channel. The Bits tab itself
/// reads `get_bits_state` when it opens.
#[derive(Serialize, Clone, Default, PartialEq, Debug)]
pub struct ChannelBitsFace {
    /// The channel has bits on.
    pub enabled: bool,
    /// The smallest single cheer here.
    pub pick_min: u32,
    /// The viewer's balance; None when the twitch.tv session could not be read.
    pub balance: Option<i64>,
    /// Twitch's still Cheer gem for that balance; None until the channel's
    /// cheermotes are known.
    pub gem_url: Option<String>,
    /// The last rise in the balance (a Get Bits purchase).
    pub last_gain: Option<WalletGain>,
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
    pub bits: Option<ChannelBitsFace>,
    pub bits_at: Option<u64>,
    pub pinned: Vec<serde_json::Value>,
    pub pinned_at: Option<u64>,
    pub collab: Option<Collaboration>,
    /// Outside a Stream Together group, the Twitch Shared Chat session the
    /// channel's chat is in (the channel first, then its live partners), so
    /// the chat header can say whose chatters these are. Read with `collab`.
    pub shared_chat: Option<SharedChat>,
    pub collab_at: Option<u64>,
    /// The signed-in viewer's follow and subscription here; `None` until the
    /// first answer.
    pub relationship: Option<Relationship>,
    pub relationship_at: Option<u64>,
    /// Numbers each pinned fetch as it starts. Fetches overlap (the poll, a
    /// refresh after a pin, the follow-up checks), and only the newest one to
    /// start may write, so a slow answer from before a pin can never put the
    /// replaced pin back on screen.
    #[serde(skip)]
    pinned_seq: u64,
    /// Polls in a row a live channel was missing from Helix's answer. One miss
    /// is often a glitch, so a live channel only reads offline after two.
    #[serde(skip)]
    offline_misses: u8,
    /// Whether the viewer poll has answered for this channel yet. The first
    /// answer is always emitted: an offline channel's state equals the empty
    /// start, and without it the view could never tell offline from unknown.
    #[serde(skip)]
    answered: bool,
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
    Bits {
        login: String,
        bits: Option<ChannelBitsFace>,
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
        shared_chat: Option<SharedChat>,
        at: u64,
    },
    Relationship {
        login: String,
        relationship: Option<Relationship>,
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
    let pinned = inner.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(PINNED_PERIOD).await;
            for login in fully_watched_logins(&pinned).await {
                refresh_pinned(&pinned, &login).await;
            }
        }
    });
    let relationships = inner;
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(RELATIONSHIP_PERIOD).await;
            for login in fully_watched_logins(&relationships).await {
                refresh_relationship(&relationships, &login, true).await;
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
    if first_full {
        // On its own task: three lookups that the other sections need not wait for.
        let inner = inner.clone();
        let l = login.clone();
        tauri::async_runtime::spawn(async move { refresh_relationship(&inner, &l, true).await });
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
                refresh_bits(&inner, &l).await;
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

/// Whether a watched channel is live: `None` until the viewer poll has
/// answered for it, or when nobody watches it.
pub async fn is_live(login: &str) -> Option<bool> {
    let inner = SERVICE.get()?;
    let state = inner.state.read().await;
    let s = state.get(&login.to_lowercase())?;
    s.answered.then_some(s.viewer_count.is_some())
}

/// Manual refresh of one section (`viewers`, `points`, `pinned`, `bits`,
/// `relationship`) for a watched channel: after a pin, a claim, a spend.
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
        "bits" => refresh_bits(inner, &login).await,
        "relationship" => refresh_relationship(inner, &login, true).await,
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
    let ids: Vec<String> = by_login.values().filter(|id| !id.is_empty()).cloned().collect();
    if ids.is_empty() {
        return;
    }
    let streams = match TwitchService::get_streams_by_user_ids(&ids).await {
        Ok(streams) => streams,
        // Signed out (Helix needs an account): the public GQL answers the same.
        Err(e) => match TwitchService::get_live_public(&ids, &[]).await {
            Ok(streams) => streams,
            Err(public) => {
                debug!("[ChannelState] viewers: {e}; public: {public}");
                return;
            }
        },
    };
    let rows: HashMap<String, TwitchStream> = streams.into_iter().map(|s| (s.user_id.clone(), s)).collect();
    let live: HashMap<String, Live> = rows
        .iter()
        .map(|(id, s)| {
            let some = |v: &String| Some(v.clone()).filter(|v| !v.is_empty());
            let live = Live {
                viewer_count: Some(s.viewer_count as u64),
                started_at: some(&s.started_at),
                title: some(&s.title),
                game_name: some(&s.game_name),
            };
            (id.clone(), live)
        })
        .collect();
    let at = now_secs();
    let mut changed: Vec<(String, Live)> = Vec::new();
    let mut went_live: Vec<TwitchStream> = Vec::new();
    // Channels whose live/offline answer changed, or arrived for the first time.
    let mut live_flipped: Vec<String> = Vec::new();
    // Live channels and the start of the stream they are in.
    let mut streams_seen: Vec<(String, String)> = Vec::new();
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
                if before != now || !s.answered {
                    changed.push((login.clone(), now.clone()));
                }
                if !s.answered || before.viewer_count.is_some() != now.viewer_count.is_some() {
                    live_flipped.push(login.clone());
                }
                if let Some(start) = &now.started_at {
                    streams_seen.push((login.clone(), start.clone()));
                }
                if before.started_at.is_none() && now.started_at.is_some() {
                    if let Some(row) = rows.get(id) {
                        went_live.push(row.clone());
                    }
                }
                s.answered = true;
                s.viewer_count = now.viewer_count;
                s.started_at = now.started_at;
                s.title = now.title;
                s.game_name = now.game_name;
                s.viewers_at = Some(at);
            }
        }
    }
    // Rewards with a per-stream limit open and close with the stream; a viewer
    // count moving is no reason to judge them again.
    for login in live_flipped {
        crate::services::channel_rewards::note_live_changed(&login);
    }
    // A new stream starts every per-stream reward count again.
    for (login, start) in streams_seen {
        crate::services::channel_rewards::note_stream_seen(&login, &start);
    }
    for (login, l) in changed {
        #[cfg(mobile)]
        crate::commands::chat_tabs::note_live(&login, l.viewer_count.is_some(), l.viewer_count);
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
    // An offline room waiting on one of these channels starts its stream.
    for row in went_live {
        crate::services::watch_session::on_channel_live(&inner.app, row);
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
    // Outside a group (a group already shares its chat), the Shared Chat
    // session: cached per channel and signed-in only, so a failed or missing
    // answer keeps what the channel last had.
    let solo: Vec<String> = live_logins
        .iter()
        .filter(|l| matches!(fresh.get(*l), Some(None)) || !fresh.contains_key(*l))
        .filter_map(|l| watched.get(l).cloned())
        .collect();
    let mut sessions = if solo.is_empty() { HashMap::new() } else { shared_chat::fetch(&solo).await };
    let at = now_secs();
    let mut changed: Vec<(String, Option<Collaboration>, Option<SharedChat>)> = Vec::new();
    {
        let mut state = inner.state.write().await;
        for (login, collab) in fresh {
            if let Some(s) = state.get_mut(&login) {
                let shared = if collab.is_some() || !live_logins.contains(&login) {
                    None
                } else {
                    match watched.get(&login).and_then(|id| sessions.remove(id)) {
                        Some(answer) => answer,
                        None => s.shared_chat.clone(),
                    }
                };
                if s.collab != collab || s.shared_chat != shared {
                    changed.push((login.clone(), collab.clone(), shared.clone()));
                }
                s.collab = collab;
                s.shared_chat = shared;
                s.collab_at = Some(at);
            }
        }
    }
    for (login, collab, shared_chat) in changed {
        emit(&inner.app, ChannelUpdate::Collab { login, collab, shared_chat, at });
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
        last_gain: None,
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
    let Some(mut points) = parse_points(&raw) else {
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
        // A rise nothing reported (the socket is down, or Twitch sent no
        // earned message for it) is still a gain; one the socket or a claim
        // already applied reads equal here and is not counted twice.
        let before = s.points.as_ref().filter(|p| p.enabled);
        points.last_gain = before.and_then(|p| p.last_gain.clone());
        if let Some(rise) = poll_rise(before.and_then(|p| p.balance), points.balance) {
            points.last_gain = Some(gain(rise, None));
        }
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

/// Read the viewer's relationship to a fully watched channel. `merge` keeps
/// what a failed lookup could not answer (see `channel_relationship::merge`);
/// after an account change the new read replaces everything.
async fn refresh_relationship(inner: &Inner, login: &str, merge: bool) {
    let channel_id = match inner.watched.read().await.get(login) {
        Some(w) if w.full_refs > 0 && !w.channel_id.is_empty() => w.channel_id.clone(),
        _ => return,
    };
    let (read, subs_answered) = channel_relationship::read_twitch(&channel_id, login).await;
    let held = if merge {
        inner.state.read().await.get(login).and_then(|s| s.relationship.clone())
    } else {
        None
    };
    store_relationship(inner, login, channel_relationship::merge(held.as_ref(), read, subs_answered)).await;
}

/// Keep `relationship` for `login` and emit it if it changed (the first answer
/// always differs from the empty start).
async fn store_relationship(inner: &Inner, login: &str, relationship: Relationship) {
    let at = now_secs();
    let changed = {
        let mut state = inner.state.write().await;
        let Some(s) = state.get_mut(login) else { return };
        let changed = s.relationship.as_ref() != Some(&relationship);
        s.relationship = Some(relationship.clone());
        s.relationship_at = Some(at);
        changed
    };
    if changed {
        emit(
            &inner.app,
            ChannelUpdate::Relationship { login: login.to_string(), relationship: Some(relationship), at },
        );
    }
}

/// Change the held relationship for a channel id some window has fully open.
async fn adjust_relationship(channel_id: &str, change: impl FnOnce(&mut Relationship)) {
    let Some(inner) = SERVICE.get() else { return };
    let Some(login) = login_for_id(inner, channel_id).await else { return };
    let mut relationship = inner.state.read().await.get(&login).and_then(|s| s.relationship.clone()).unwrap_or_default();
    change(&mut relationship);
    store_relationship(inner, &login, relationship).await;
}

/// The viewer followed or unfollowed `channel_id` from this app. Twitch's
/// answer to the write is the truth, so every chat menu shows it at once.
pub async fn own_follow_changed(channel_id: &str, following: bool) {
    adjust_relationship(channel_id, |r| r.following = Some(following)).await;
}

/// The viewer subscribed in `channel_id` (their own sub, a resub or upgrade,
/// or a gift to them). The months and badge follow on the next read.
pub async fn own_sub_seen(channel_id: &str) {
    adjust_relationship(channel_id, |r| {
        r.subscribed = true;
        r.has_sub_history = false;
    })
    .await;
}

/// The signed-in account changed or signed out: every open chat's
/// relationship belongs to the old one, so read each again from scratch.
pub fn account_changed() {
    let Some(inner) = SERVICE.get() else { return };
    let inner = inner.clone();
    tauri::async_runtime::spawn(async move {
        for login in fully_watched_logins(&inner).await {
            refresh_relationship(&inner, &login, false).await;
        }
    });
}

/// The watched login for a channel id, when some window has it fully open.
async fn login_for_id(inner: &Inner, channel_id: &str) -> Option<String> {
    inner
        .watched
        .read()
        .await
        .iter()
        .find(|(_, w)| w.channel_id == channel_id && w.full_refs > 0)
        .map(|(login, _)| login.clone())
}

/// Points rose on `channel_id`: Twitch's earned message or a chest claim's
/// answer. Sets the balance and, when it is higher than the one already
/// known, records the gain, so the same rise reported twice (socket and
/// claim, then the poll) shows once.
pub async fn points_gained(channel_id: &str, amount: i64, reason_code: &str, balance: Option<i64>) {
    let Some(inner) = SERVICE.get() else { return };
    let Some(login) = login_for_id(inner, channel_id).await else { return };
    if amount <= 0 {
        return;
    }
    let balance = balance.filter(|b| *b > 0);
    let at = now_secs();
    let update = {
        let mut state = inner.state.write().await;
        let Some(s) = state.get_mut(&login) else { return };
        let Some(points) = s.points.as_mut().filter(|p| p.enabled) else { return };
        if let (Some(known), Some(b)) = (points.balance, balance) {
            if b <= known {
                return;
            }
        }
        if balance.is_some() {
            points.balance = balance;
        }
        points.last_gain = Some(gain(amount, Some(reason_code)));
        s.points_at = Some(at);
        s.points.clone()
    };
    emit(&inner.app, ChannelUpdate::Points { login, points: update, at });
}

/// Points were spent on `channel_id` (a redemption, a prediction bet): take
/// the new balance so the next poll does not misread the difference.
pub async fn points_spent(channel_id: &str, balance: i64) {
    let Some(inner) = SERVICE.get() else { return };
    let Some(login) = login_for_id(inner, channel_id).await else { return };
    let at = now_secs();
    let update = {
        let mut state = inner.state.write().await;
        let Some(s) = state.get_mut(&login) else { return };
        let Some(points) = s.points.as_mut().filter(|p| p.enabled) else { return };
        if balance < 0 || points.balance == Some(balance) {
            return;
        }
        points.balance = Some(balance);
        s.points_at = Some(at);
        s.points.clone()
    };
    emit(&inner.app, ChannelUpdate::Points { login, points: update, at });
}

/// The wallet's bits face for one channel: bits on here (public, cached), the
/// viewer's balance (read once per account), the gem for it.
async fn refresh_bits(inner: &Inner, login: &str) {
    let channel_id = match inner.watched.read().await.get(login) {
        Some(w) => w.channel_id.clone(),
        None => return,
    };
    let ch = match crate::services::bits::channel_bits(login).await {
        Ok(ch) => ch,
        Err(e) => {
            debug!("[ChannelState] bits for {login}: {e}");
            return;
        }
    };
    let mut face = ChannelBitsFace { enabled: ch.enabled, pick_min: ch.smallest_single(), ..Default::default() };
    if ch.enabled {
        if crate::services::irc_service::channel_cheermote_set(login).is_none() && !channel_id.is_empty() {
            crate::services::irc_service::IrcService::fetch_and_store_cheermotes(login.to_string(), channel_id).await;
        }
        if let Some(state) = inner.app.try_state::<AppState>() {
            face.balance = crate::services::bits::known_balance(&state.twitch_auth).await;
        }
        face.gem_url = crate::services::bits::still_gem(login, face.balance);
    }
    store_bits(inner, login, face).await;
}

/// Keep `face` for `login` (carrying its last gain) and emit it if it changed.
async fn store_bits(inner: &Inner, login: &str, mut face: ChannelBitsFace) {
    let at = now_secs();
    let changed = {
        let mut state = inner.state.write().await;
        let Some(s) = state.get_mut(login) else { return };
        if face.last_gain.is_none() {
            face.last_gain = s.bits.as_ref().and_then(|b| b.last_gain.clone());
        }
        let changed = s.bits.as_ref() != Some(&face);
        s.bits = Some(face.clone());
        s.bits_at = Some(at);
        changed
    };
    if changed {
        emit(&inner.app, ChannelUpdate::Bits { login: login.to_string(), bits: Some(face), at });
    }
}

/// The viewer's bits balance changed (a cheer's answer, a Get Bits purchase):
/// every watched channel with bits on shows the new number, and a rise
/// becomes that channel's "+N".
pub async fn bits_balance_changed(balance: i64, rise: Option<i64>) {
    let Some(inner) = SERVICE.get() else { return };
    let faces: Vec<(String, ChannelBitsFace)> = inner
        .state
        .read()
        .await
        .iter()
        .filter_map(|(login, s)| s.bits.clone().filter(|b| b.enabled).map(|b| (login.clone(), b)))
        .collect();
    for (login, mut face) in faces {
        face.balance = Some(balance);
        face.gem_url = crate::services::bits::still_gem(&login, face.balance);
        if let Some(r) = rise.filter(|r| *r > 0) {
            face.last_gain = Some(gain(r, None));
        }
        store_bits(inner, &login, face).await;
    }
}

/// The account changed: read every open channel's bits face again for the
/// new viewer.
pub async fn refresh_all_bits() {
    let Some(inner) = SERVICE.get() else { return };
    for login in fully_watched_logins(inner).await {
        refresh_bits(inner, &login).await;
    }
}

async fn refresh_pinned(inner: &Inner, login: &str) {
    let channel_id = match inner.watched.read().await.get(login) {
        Some(w) => w.channel_id.clone(),
        None => return,
    };
    let seq = {
        let mut state = inner.state.write().await;
        let Some(s) = state.get_mut(login) else { return };
        s.pinned_seq += 1;
        s.pinned_seq
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
        if s.pinned_seq != seq {
            // A newer fetch started while this one was in flight; its answer wins.
            return;
        }
        let changed = s.pinned != pinned;
        s.pinned = pinned.clone();
        s.pinned_at = Some(at);
        changed
    };
    if changed {
        emit(&inner.app, ChannelUpdate::Pinned { login: login.to_string(), pinned, at });
    }
}

/// What a pin action should leave on screen.
#[derive(Clone, Debug, PartialEq)]
pub enum PinChange {
    /// This chat message is now the pin.
    Pinned(String),
    /// This chat message (or, with `None`, whatever was pinned) is gone.
    Unpinned(Option<String>),
}

/// Waits between follow-up reads after a pin action, in milliseconds. Twitch
/// answers the Helix pin before its pinned-chat read reflects it, and a pin
/// that replaces another can take several seconds to show there. The checks
/// stop as soon as the read agrees with the action.
const PIN_SETTLE_STEPS_MS: [u64; 7] = [0, 700, 1300, 2000, 3000, 4000, 6000];

fn message_id_of(pin: &serde_json::Value) -> &str {
    pin.get("message_id").and_then(|v| v.as_str()).unwrap_or("")
}

/// Whether a pinned-chat read already shows `change`. A new pin must be the
/// one on top, since the banner shows the first entry.
fn pin_settled(pinned: &[serde_json::Value], change: &PinChange) -> bool {
    match change {
        PinChange::Pinned(id) => pinned.first().is_some_and(|p| message_id_of(p) == id),
        PinChange::Unpinned(Some(id)) => !pinned.iter().any(|p| message_id_of(p) == id),
        PinChange::Unpinned(None) => pinned.is_empty(),
    }
}

/// After a pin or unpin by this app, re-read the channel's pins until they
/// show the change (or the steps run out), so every window watching it updates
/// within a moment of Twitch reflecting the action instead of at the next poll.
pub fn settle_pin(channel_id: &str, change: PinChange) {
    let Some(inner) = SERVICE.get().cloned() else { return };
    let channel_id = channel_id.to_string();
    tauri::async_runtime::spawn(async move {
        for step in PIN_SETTLE_STEPS_MS {
            tokio::time::sleep(Duration::from_millis(step)).await;
            let logins: Vec<String> = inner
                .watched
                .read()
                .await
                .iter()
                .filter(|(_, w)| w.full_refs > 0 && w.channel_id == channel_id)
                .map(|(login, _)| login.clone())
                .collect();
            if logins.is_empty() {
                return;
            }
            let mut settled = true;
            for login in &logins {
                refresh_pinned(&inner, login).await;
                let state = inner.state.read().await;
                if !state.get(login).is_some_and(|s| pin_settled(&s.pinned, &change)) {
                    settled = false;
                }
            }
            if settled {
                return;
            }
        }
        debug!("[ChannelState] pins for {channel_id} still do not show {change:?}");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poll_rise_needs_a_known_earlier_balance() {
        assert_eq!(poll_rise(Some(100), Some(110)), Some(10));
        assert_eq!(poll_rise(None, Some(110)), None);
        assert_eq!(poll_rise(Some(110), Some(100)), None);
        assert_eq!(poll_rise(Some(110), Some(110)), None);
        assert_eq!(poll_rise(Some(110), None), None);
    }

    #[test]
    fn known_reasons_are_worded_and_unknown_ones_stay_unnamed() {
        assert_eq!(gain_reason("WATCH").as_deref(), Some("for watching"));
        assert_eq!(gain_reason("claim").as_deref(), Some("from the bonus chest"));
        assert_eq!(gain_reason("WATCH_STREAK").as_deref(), Some("for your watch streak"));
        assert_eq!(gain_reason("PREDICTION_WIN").as_deref(), Some("from a prediction"));
        assert_eq!(gain_reason("unknown"), None);
    }

    #[test]
    fn every_gain_gets_a_new_number() {
        let a = gain(10, None);
        let b = gain(10, None);
        assert!(b.seq > a.seq);
        assert!(gain(50, Some("CLAIM")).chest);
        assert!(!gain(10, Some("WATCH")).chest);
    }
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
    fn a_replacing_pin_settles_only_once_it_is_on_top() {
        let old = json!({"message_id": "old"});
        let new = json!({"message_id": "new"});
        let pinned = PinChange::Pinned("new".into());
        assert!(!pin_settled(&[old.clone()], &pinned), "the replaced pin is still shown");
        assert!(!pin_settled(&[old.clone(), new.clone()], &pinned), "the new pin is not on top");
        assert!(pin_settled(&[new.clone(), old.clone()], &pinned));
        assert!(!pin_settled(&[], &pinned));

        let unpin_new = PinChange::Unpinned(Some("new".into()));
        assert!(!pin_settled(&[new.clone()], &unpin_new));
        assert!(pin_settled(&[old.clone()], &unpin_new));
        assert!(pin_settled(&[], &unpin_new));

        let unpin_any = PinChange::Unpinned(None);
        assert!(!pin_settled(&[old], &unpin_any));
        assert!(pin_settled(&[], &unpin_any));
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
