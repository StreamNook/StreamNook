//! Live line (live or not, category, viewers, title) for every chat tab
//! strip: the chat dock's chats, the phones' joined chats, MultiChat's tabs
//! and the MultiNook chat switcher. One map by `make_key(provider, login)`,
//! emitted as `chat-dock-live` to every window. A key with no row has not been
//! answered yet: no dot, and no "Offline" either.
//!
//! Kick, YouTube and TikTok have nothing like Twitch's batched poll for an
//! arbitrary channel, so this polls them with each platform's own `live_check`
//! (Kick batches 50 slugs per call; YouTube and TikTok are per channel), on the
//! same cadences the favourites poller uses, and emits only when something
//! changed. A strip's Twitch channels ride `channel_state`'s one batched viewer
//! poll instead: they are watched there live-only, and its answers land here
//! through `note_twitch`. (Docked Twitch chats are watched by the dock itself
//! and read channel state directly.)
//!
//! The dock and the phones' chats are read straight from their own lists, so
//! nothing has to register for them: docking or closing a chat wakes this
//! (`wake`). Other strips declare what they show with `set_tab_live_channels`,
//! one set per strip, ended when its window is destroyed. With nothing to show
//! it does nothing, and it rests while no window is on screen.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::sync::Notify;

use crate::models::provider_stream::ProviderStream;
use crate::rt::AppHandle;
use crate::services::providers::key::{is_provider_id, make_key, normalize_channel, same_channel};
use crate::services::providers::registry;

pub const EVENT: &str = "chat-dock-live";

/// What a chat tab's row shows.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct DockLive {
    pub live: bool,
    pub viewer_count: Option<u32>,
    pub category: Option<String>,
    pub title: Option<String>,
}

fn period(provider: &str) -> Duration {
    match provider {
        "kick" => Duration::from_secs(60),
        _ => Duration::from_secs(180),
    }
}

#[derive(Default)]
struct State {
    /// By `make_key(provider, login)`.
    live: HashMap<String, DockLive>,
    checked: HashMap<String, Instant>,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));
static WAKE: Lazy<Notify> = Lazy::new(Notify::new);

/// A channel a tab strip shows, as the page declares it.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct TabChannel {
    pub provider: String,
    pub login: String,
    #[serde(default)]
    pub channel_id: Option<String>,
}

/// The most channels one strip may declare.
const MAX_STRIP_CHANNELS: usize = 64;

/// What each strip outside the dock shows, by `<window label>/<strip>`, with
/// the order its declaration arrived in: an older one that lands late (a
/// Twitch id lookup in between) never undoes a newer one. A strip that ends
/// keeps an empty set for the same reason.
static STRIPS: Lazy<Mutex<HashMap<String, (u64, Vec<TabChannel>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static STRIP_SEQ: AtomicU64 = AtomicU64::new(0);
/// The Twitch channels the strips hold a live-only `channel_state` watch on,
/// login to id.
static TWITCH_HELD: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));
/// Twitch ids looked up for strips that sent none, by login.
static TWITCH_IDS: Lazy<Mutex<HashMap<String, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static EMIT_PENDING: AtomicBool = AtomicBool::new(false);

/// The current rows, for a window that is starting up.
#[tauri::command]
pub fn get_chat_dock_live() -> HashMap<String, DockLive> {
    STATE.lock().map(|s| s.live.clone()).unwrap_or_default()
}

/// The dock changed: check what is new now rather than at the next tick.
pub fn wake() {
    WAKE.notify_one();
}

/// Sends the map once for a burst of changes: a viewer poll answers for every
/// held channel in one pass, and every window re-reads the whole map on each
/// send.
fn emit_soon(app: &AppHandle) {
    if EMIT_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        EMIT_PENDING.store(false, Ordering::Release);
        let _ = app.emit(EVENT, get_chat_dock_live());
    });
}

/// Docked chats (desktop), joined phone chats and the tab strips' channels,
/// per non-Twitch platform, as `(provider, logins)`.
fn docked_by_provider() -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    #[cfg(desktop)]
    let chats = crate::commands::chat_dock::get_chat_dock().chats.into_iter().map(|c| (c.provider, c.login));
    #[cfg(mobile)]
    let chats = crate::services::chat_tabs::joined().into_iter().map(|c| (c.provider, c.login));
    for (provider, login) in chats {
        if provider != "twitch" {
            let logins = out.entry(provider).or_default();
            if !logins.contains(&login) {
                logins.push(login);
            }
        }
    }
    if let Ok(strips) = STRIPS.lock() {
        add_strip_channels(&mut out, &strips);
    }
    out
}

/// The strips' Kick / YouTube / TikTok channels, added to `out` once each.
fn add_strip_channels(out: &mut HashMap<String, Vec<String>>, strips: &HashMap<String, (u64, Vec<TabChannel>)>) {
    for c in strips.values().flat_map(|(_, cs)| cs) {
        if c.provider == "twitch" {
            continue;
        }
        let logins = out.entry(c.provider.clone()).or_default();
        if !logins.iter().any(|l| same_channel(&c.provider, l, &c.login)) {
            logins.push(c.login.clone());
        }
    }
}

/// The Twitch channels the strips show, login to id. One with no id is left
/// out: the batched poll reads by id.
fn twitch_wanted(strips: &HashMap<String, (u64, Vec<TabChannel>)>) -> HashMap<String, String> {
    strips
        .values()
        .flat_map(|(_, cs)| cs)
        .filter(|c| c.provider == "twitch")
        .filter_map(|c| {
            let id = c.channel_id.as_deref().unwrap_or("").trim();
            (!id.is_empty()).then(|| (normalize_channel("twitch", &c.login), id.to_string()))
        })
        .collect()
}

/// What to start and stop watching to go from `held` to `wanted`.
fn twitch_diff(held: &HashMap<String, String>, wanted: &HashMap<String, String>) -> (Vec<(String, String)>, Vec<String>) {
    let added = wanted
        .iter()
        .filter(|(l, _)| !held.contains_key(*l))
        .map(|(l, id)| (l.clone(), id.clone()))
        .collect();
    let removed = held.keys().filter(|l| !wanted.contains_key(*l)).cloned().collect();
    (added, removed)
}

/// Whether a declaration numbered `seq` may replace what `strip` holds.
fn accepts(strips: &HashMap<String, (u64, Vec<TabChannel>)>, strip: &str, seq: u64) -> bool {
    strips.get(strip).is_none_or(|(newest, _)| *newest <= seq)
}

/// A Twitch channel's row as the strips show it: offline carries nothing.
fn twitch_row(live: bool, viewer_count: Option<u64>, category: Option<String>, title: Option<String>) -> DockLive {
    if !live {
        return DockLive::default();
    }
    DockLive {
        live: true,
        viewer_count: viewer_count.map(|v| u32::try_from(v).unwrap_or(u32::MAX)),
        category,
        title,
    }
}

/// Writes a held Twitch channel's row; true when it changed.
fn set_twitch_row(login: &str, live: bool, viewer_count: Option<u64>, category: Option<String>, title: Option<String>) -> bool {
    let login = normalize_channel("twitch", login);
    if !TWITCH_HELD.lock().is_ok_and(|h| h.contains_key(&login)) {
        return false;
    }
    let row = twitch_row(live, viewer_count, category, title);
    let key = make_key("twitch", &login);
    let Ok(mut s) = STATE.lock() else { return false };
    if s.live.get(&key) == Some(&row) {
        return false;
    }
    s.live.insert(key, row);
    true
}

/// Drops a Twitch row no strip holds any more; true when one went.
fn drop_twitch_row(login: &str) -> bool {
    if TWITCH_HELD.lock().map_or(true, |h| h.contains_key(login)) {
        return false;
    }
    STATE.lock().is_ok_and(|mut s| s.live.remove(&make_key("twitch", login)).is_some())
}

/// `channel_state`'s viewer poll answered for a Twitch channel. A strip
/// holding it gets the row; any other channel is not shown here.
pub fn note_twitch(app: &AppHandle, login: &str, viewer_count: Option<u64>, category: Option<String>, title: Option<String>) {
    if set_twitch_row(login, viewer_count.is_some(), viewer_count, category, title) {
        emit_soon(app);
    }
}

/// A Twitch id for a strip channel that came without one, looked up once.
async fn twitch_id(login: &str) -> Option<String> {
    let login = normalize_channel("twitch", login);
    let cached = TWITCH_IDS.lock().ok().and_then(|m| m.get(&login).cloned());
    if let Some(id) = cached {
        return Some(id);
    }
    let id = crate::services::twitch_service::TwitchService::get_user_by_login(&login).await.ok()?.id;
    if let Ok(mut m) = TWITCH_IDS.lock() {
        m.insert(login, id.clone());
    }
    Some(id)
}

/// A tab strip outside the dock (MultiChat's tabs, the MultiNook chat
/// switcher) shows `channels`; an empty list ends it. Replaces what the same
/// strip declared before.
#[tauri::command]
pub async fn set_tab_live_channels(
    window: crate::rt::Window,
    app: AppHandle,
    surface: String,
    channels: Vec<TabChannel>,
) -> Result<(), String> {
    let channels: Vec<TabChannel> = channels
        .into_iter()
        .filter(|c| is_provider_id(&c.provider) && !c.login.trim().is_empty())
        .take(MAX_STRIP_CHANNELS)
        .collect();
    let seq = STRIP_SEQ.fetch_add(1, Ordering::Relaxed);
    set_strip(&app, format!("{}/{}", window.label(), surface), channels, seq).await;
    Ok(())
}

/// A window was destroyed and its page ran no cleanup: end its strips.
pub fn release_window(app: &AppHandle, label: &str) {
    let prefix = format!("{label}/");
    let strips: Vec<String> = STRIPS
        .lock()
        .map(|s| s.keys().filter(|k| k.starts_with(&prefix)).cloned().collect())
        .unwrap_or_default();
    if strips.is_empty() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        for strip in strips {
            let seq = STRIP_SEQ.fetch_add(1, Ordering::Relaxed);
            set_strip(&app, strip, Vec::new(), seq).await;
        }
        if let Ok(mut s) = STRIPS.lock() {
            s.retain(|k, (_, cs)| !(k.starts_with(&prefix) && cs.is_empty()));
        }
    });
}

async fn set_strip(app: &AppHandle, strip: String, mut channels: Vec<TabChannel>, seq: u64) {
    // A Twitch channel the page has no id for (a Go Live seed, an older saved
    // source): look it up, since the batched poll reads by id.
    for c in channels.iter_mut() {
        if c.provider == "twitch" && c.channel_id.as_deref().is_none_or(|id| id.trim().is_empty()) {
            c.channel_id = twitch_id(&c.login).await;
        }
    }
    let (added, removed) = {
        let Ok(mut strips) = STRIPS.lock() else { return };
        if !accepts(&strips, &strip, seq) {
            return;
        }
        strips.insert(strip, (seq, channels));
        let wanted = twitch_wanted(&strips);
        let Ok(mut held) = TWITCH_HELD.lock() else { return };
        let diff = twitch_diff(&held, &wanted);
        *held = wanted;
        diff
    };
    let mut changed = false;
    for login in &removed {
        crate::services::channel_state::unwatch(login, true).await;
        changed |= drop_twitch_row(login);
    }
    if !added.is_empty() {
        crate::services::channel_state::watch_live_many(&added).await;
        for (login, _) in &added {
            let live = crate::services::channel_state::is_live(login).await;
            let state = crate::services::channel_state::get(login).await;
            if let (Some(live), Some(state)) = (live, state) {
                changed |= set_twitch_row(login, live, state.viewer_count, state.game_name, state.title);
            }
        }
    }
    // New Kick / YouTube / TikTok channels are checked now, not at the next tick.
    wake();
    if changed {
        emit_soon(app);
    }
}

/// A row per docked channel from a complete check: the platform's answer when
/// it gave one, offline when it left the channel out.
fn rows_for(provider: &str, logins: &[String], found: &[ProviderStream]) -> Vec<(String, DockLive)> {
    logins
        .iter()
        .map(|login| {
            let hit = found
                .iter()
                .find(|r| same_channel(provider, &r.user_login, login) || r.key == make_key(provider, login));
            let row = match hit {
                Some(r) if r.is_live => DockLive {
                    live: true,
                    viewer_count: Some(r.viewer_count),
                    category: Some(r.game_name.clone()).filter(|s| !s.is_empty()),
                    title: Some(r.title.clone()).filter(|s| !s.is_empty()),
                },
                _ => DockLive::default(),
            };
            (make_key(provider, login), row)
        })
        .collect()
}

async fn check(app: &AppHandle) {
    let docked = docked_by_provider();
    let now = Instant::now();
    let mut changed = false;
    {
        // Forget channels nothing shows any more.
        let mut keep: Vec<String> = docked
            .iter()
            .flat_map(|(p, ls)| ls.iter().map(move |l| make_key(p, l)))
            .collect();
        // The strips' Twitch rows stay while a strip holds them.
        if let Ok(held) = TWITCH_HELD.lock() {
            keep.extend(held.keys().map(|l| make_key("twitch", l)));
        }
        if let Ok(mut s) = STATE.lock() {
            let before = s.live.len();
            s.live.retain(|k, _| keep.contains(k));
            changed |= s.live.len() != before;
        }
    }
    for (provider, logins) in &docked {
        // Due when its cadence has passed or a docked channel was never checked.
        let due = STATE.lock().map(|s| {
            let fresh = logins.iter().any(|l| !s.live.contains_key(&make_key(provider, l)));
            fresh || s.checked.get(provider).is_none_or(|t| now.duration_since(*t) >= period(provider))
        });
        if !due.unwrap_or(false) {
            continue;
        }
        let Some(src) = registry().await.get_source(provider) else { continue };
        if !src.caps().live_check {
            continue;
        }
        match src.live_check(logins).await {
            Ok(found) => {
                let rows = rows_for(provider, logins, &found);
                #[cfg(mobile)]
                for (k, row) in &rows {
                    crate::commands::chat_tabs::note_live(k, row.live, row.viewer_count.map(u64::from));
                }
                if let Ok(mut s) = STATE.lock() {
                    s.checked.insert(provider.clone(), now);
                    for (k, row) in rows {
                        if s.live.get(&k) != Some(&row) {
                            s.live.insert(k, row);
                            changed = true;
                        }
                    }
                }
            }
            Err(e) => debug!("[ChatDockLive] {provider} live_check failed: {e}"),
        }
    }
    if changed {
        // Every window: MultiChat windows draw these dots too.
        emit_soon(app);
    }
}

/// Start the poller. Called once from the setup hook.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            if !crate::services::window_visibility::all_hidden() {
                check(&app).await;
            }
            // Wake for a dock change, or look again in a minute (the shortest
            // cadence; each platform is only re-checked when its own is due).
            let _ = tokio::time::timeout(Duration::from_secs(60), WAKE.notified()).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(login: &str, live: bool, game: &str) -> ProviderStream {
        let mut r: ProviderStream = serde_json::from_value(serde_json::json!({
            "provider": "kick",
            "key": format!("kick:{login}"),
            "user_login": login,
            "user_name": login,
            "is_live": false,
            "watch_url": "",
        }))
        .expect("a minimal provider row");
        r.is_live = live;
        r.game_name = game.into();
        r.viewer_count = 120;
        r
    }

    #[test]
    fn a_live_row_carries_its_category_and_a_missing_one_reads_offline() {
        let rows = rows_for("kick", &["xqc".into(), "gone".into()], &[row("XQC", true, "Slots")]);
        let xqc = &rows.iter().find(|(k, _)| k == "kick:xqc").unwrap().1;
        assert!(xqc.live);
        assert_eq!(xqc.category.as_deref(), Some("Slots"));
        assert_eq!(xqc.viewer_count, Some(120));
        let gone = &rows.iter().find(|(k, _)| k == "kick:gone").unwrap().1;
        assert_eq!(gone, &DockLive::default());
    }

    #[test]
    fn an_offline_answer_has_no_category() {
        let rows = rows_for("kick", &["sleepy".into()], &[row("sleepy", false, "Slots")]);
        assert_eq!(rows[0].1, DockLive::default());
    }

    fn strip(rows: &[(&str, &str, Option<&str>)]) -> (u64, Vec<TabChannel>) {
        let channels = rows
            .iter()
            .map(|(p, l, id)| TabChannel { provider: p.to_string(), login: l.to_string(), channel_id: id.map(String::from) })
            .collect();
        (0, channels)
    }

    #[test]
    fn strips_hold_each_twitch_channel_once_and_skip_one_with_no_id() {
        let mut strips = HashMap::new();
        strips.insert(
            "main/multinook-a".to_string(),
            strip(&[("twitch", "Kathi", Some("1")), ("twitch", "nobody", None), ("kick", "kathi", None)]),
        );
        strips.insert("multichat-1/multichat-b".to_string(), strip(&[("twitch", "kathi", Some("1"))]));
        let wanted = twitch_wanted(&strips);
        assert_eq!(wanted.len(), 1);
        assert_eq!(wanted.get("kathi").map(String::as_str), Some("1"));
    }

    #[test]
    fn a_twitch_channel_is_released_only_when_no_strip_shows_it() {
        let held: HashMap<String, String> = [("a".to_string(), "1".to_string()), ("b".to_string(), "2".to_string())].into();
        let wanted: HashMap<String, String> = [("b".to_string(), "2".to_string()), ("c".to_string(), "3".to_string())].into();
        let (added, removed) = twitch_diff(&held, &wanted);
        assert_eq!(added, vec![("c".to_string(), "3".to_string())]);
        assert_eq!(removed, vec!["a".to_string()]);
    }

    #[test]
    fn strip_channels_on_other_platforms_join_the_check_once() {
        let mut out: HashMap<String, Vec<String>> = HashMap::new();
        out.insert("kick".into(), vec!["xqc".into()]);
        let mut strips = HashMap::new();
        strips.insert(
            "w/s".to_string(),
            strip(&[("kick", "XQC", None), ("youtube", "@Amane", None), ("twitch", "xqc", Some("9"))]),
        );
        add_strip_channels(&mut out, &strips);
        assert_eq!(out.get("kick"), Some(&vec!["xqc".to_string()]));
        assert_eq!(out.get("youtube"), Some(&vec!["@Amane".to_string()]));
        assert!(!out.contains_key("twitch"));
    }

    #[test]
    fn an_offline_twitch_row_carries_nothing() {
        assert_eq!(twitch_row(false, Some(5), Some("Chess".into()), None), DockLive::default());
        assert_eq!(twitch_row(true, Some(5), None, None).viewer_count, Some(5));
    }

    #[test]
    fn a_late_older_declaration_never_undoes_a_newer_one() {
        let mut strips = HashMap::new();
        strips.insert("w/s".to_string(), (5, Vec::new()));
        assert!(!accepts(&strips, "w/s", 4));
        assert!(accepts(&strips, "w/s", 6));
        assert!(accepts(&strips, "w/other", 0));
    }
}
