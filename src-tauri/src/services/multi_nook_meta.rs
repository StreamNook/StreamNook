//! Live state, title and category for MultiNook's Twitch tiles: one Helix
//! batch for the whole grid every minute, and an event only when something
//! changed. `live` drives the tiles: an offline tile starts its stream when its
//! channel goes live, and a playing tile whose channel ended shows the offline
//! card (a live channel reads offline only after two missed polls, so one
//! glitch never stops a playing tile).
//!
//! The grid registers its Twitch channels (`set_multi_nook_meta_channels`),
//! the way it registers them for raids; a change of channels refreshes at once
//! and an empty list ends the poll. Each channel's account (id, display name,
//! avatar, partner or affiliate status) is looked up once, in parallel, and
//! kept: it does not change during a session, and it repairs a saved grid
//! whose avatars have expired. The poll skips while no window is on screen:
//! only the grid shows this.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use log::debug;
use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::Emitter;
use tokio::sync::Notify;

use crate::rt::AppHandle;
use crate::services::twitch_service::TwitchService;

pub const EVENT: &str = "multi-nook://meta";
const PERIOD: Duration = Duration::from_secs(60);

/// What a tile shows about its channel. `live` false clears the title; the
/// category is kept as the last one known.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct TileMeta {
    pub login: String,
    pub live: bool,
    pub title: Option<String>,
    pub game_name: Option<String>,
    pub broadcaster_type: Option<String>,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    pub profile_image_url: Option<String>,
}

/// A channel's account, looked up once per session.
#[derive(Clone, Debug, PartialEq, Default)]
struct Account {
    id: String,
    display_name: String,
    avatar: Option<String>,
    broadcaster_type: String,
}

#[derive(Default)]
struct State {
    logins: Vec<String>,
    last: HashMap<String, TileMeta>,
    accounts: HashMap<String, Account>,
    /// Polls in a row a live channel was missing from the batch.
    misses: HashMap<String, u8>,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));
static WAKE: Lazy<Notify> = Lazy::new(Notify::new);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// The grid's Twitch channels. Refreshes at once when they change.
pub fn set_channels(app: &AppHandle, logins: Vec<String>) {
    let mut logins: Vec<String> = logins
        .into_iter()
        .map(|l| l.trim().to_ascii_lowercase())
        .filter(|l| !l.is_empty())
        .collect();
    logins.sort();
    logins.dedup();
    {
        let Ok(mut s) = STATE.lock() else { return };
        if s.logins == logins {
            return;
        }
        // Forget channels that left, so a channel added back is reported.
        s.last.retain(|k, _| logins.contains(k));
        s.misses.retain(|k, _| logins.contains(k));
        s.logins = logins;
    }
    if RUNNING.swap(true, Ordering::AcqRel) {
        WAKE.notify_one();
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            refresh(&app).await;
            tokio::select! {
                _ = tokio::time::sleep(PERIOD) => {}
                _ = WAKE.notified() => {}
            }
            let empty = STATE.lock().map(|s| s.logins.is_empty()).unwrap_or(true);
            if empty {
                RUNNING.store(false, Ordering::Release);
                // Channels registered between the check and the store would
                // otherwise go unpolled.
                let again = STATE.lock().map(|s| !s.logins.is_empty()).unwrap_or(false);
                if again && !RUNNING.swap(true, Ordering::AcqRel) {
                    continue;
                }
                break;
            }
        }
    });
}

async fn refresh(app: &AppHandle) {
    if crate::services::window_visibility::all_hidden() {
        return;
    }
    let (logins, unknown_types) = {
        let Ok(s) = STATE.lock() else { return };
        let unknown: Vec<String> = s.logins.iter().filter(|l| !s.accounts.contains_key(*l)).cloned().collect();
        (s.logins.clone(), unknown)
    };
    if logins.is_empty() {
        return;
    }

    let live = match TwitchService::check_streams_online(&logins).await {
        Ok(streams) => streams,
        Err(e) => {
            debug!("[MultiNookMeta] streams: {e}");
            return;
        }
    };
    let looked_up = futures::future::join_all(unknown_types.into_iter().map(|login| async move {
        let user = TwitchService::get_user_by_login(&login).await.ok()?;
        Some((
            login,
            Account {
                id: user.id,
                display_name: user.display_name,
                avatar: user.profile_image_url.filter(|u| !u.is_empty()),
                broadcaster_type: user.broadcaster_type.unwrap_or_default(),
            },
        ))
    }))
    .await;

    let changed = {
        let Ok(mut s) = STATE.lock() else { return };
        s.accounts.extend(looked_up.into_iter().flatten());
        let built = build(&logins, &live, &s.accounts, &s.last);
        let State { last, misses, .. } = &mut *s;
        let now = hold_live(built, last, misses);
        let changed = changed_entries(&s.last, &now);
        for m in &changed {
            s.last.insert(m.login.clone(), m.clone());
        }
        changed
    };
    if !changed.is_empty() {
        let _ = app.emit(EVENT, &changed);
    }
}

/// Each registered channel's meta from the live batch. A channel absent from
/// the batch is offline: no title, its last known category kept.
fn build(
    logins: &[String],
    live: &[crate::models::stream::TwitchStream],
    accounts: &HashMap<String, Account>,
    last: &HashMap<String, TileMeta>,
) -> Vec<TileMeta> {
    let by_login: HashMap<String, &crate::models::stream::TwitchStream> =
        live.iter().map(|s| (s.user_login.to_ascii_lowercase(), s)).collect();
    let some = |v: &str| Some(v.to_string()).filter(|v| !v.is_empty());
    logins
        .iter()
        .map(|login| {
            let account = accounts.get(login);
            let mut meta = match by_login.get(login) {
                Some(s) => TileMeta {
                    login: login.clone(),
                    live: true,
                    title: some(&s.title),
                    game_name: some(&s.game_name).or_else(|| last.get(login).and_then(|m| m.game_name.clone())),
                    ..Default::default()
                },
                None => TileMeta {
                    login: login.clone(),
                    live: false,
                    title: None,
                    game_name: last.get(login).and_then(|m| m.game_name.clone()),
                    ..Default::default()
                },
            };
            if let Some(a) = account {
                meta.broadcaster_type = Some(a.broadcaster_type.clone());
                meta.user_id = some(&a.id);
                meta.display_name = some(&a.display_name);
                meta.profile_image_url = a.avatar.clone();
            }
            meta
        })
        .collect()
}

/// Keep a live channel live through ONE poll that left it out: a single
/// missing row is often a glitch, and reading it as offline would stop a
/// playing tile. The second miss in a row is believed.
fn hold_live(now: Vec<TileMeta>, last: &HashMap<String, TileMeta>, misses: &mut HashMap<String, u8>) -> Vec<TileMeta> {
    now.into_iter()
        .map(|m| {
            let was_live = last.get(&m.login).is_some_and(|l| l.live);
            if m.live || !was_live {
                misses.remove(&m.login);
                return m;
            }
            let count = misses.entry(m.login.clone()).or_insert(0);
            *count += 1;
            if *count >= 2 {
                misses.remove(&m.login);
                m
            } else {
                last.get(&m.login).cloned().unwrap_or(m)
            }
        })
        .collect()
}

/// The entries that differ from what was last sent.
fn changed_entries(last: &HashMap<String, TileMeta>, now: &[TileMeta]) -> Vec<TileMeta> {
    now.iter().filter(|m| last.get(&m.login) != Some(*m)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stream::TwitchStream;

    fn stream(login: &str, title: &str, game: &str) -> TwitchStream {
        let mut s: TwitchStream = serde_json::from_value(serde_json::json!({
            "id": "1", "user_id": "1", "user_name": login, "user_login": login,
            "title": title, "viewer_count": 1, "game_id": "1", "game_name": game,
            "thumbnail_url": "", "started_at": ""
        }))
        .expect("a minimal stream row");
        s.user_login = login.to_string();
        s
    }

    #[test]
    fn live_channels_carry_title_and_category_offline_ones_keep_the_category() {
        let logins = vec!["a".to_string(), "b".to_string()];
        let mut last = HashMap::new();
        last.insert("b".into(), TileMeta { login: "b".into(), live: true, title: Some("old".into()), game_name: Some("Chess".into()), ..Default::default() });
        let mut accounts = HashMap::new();
        accounts.insert(
            "a".to_string(),
            Account { id: "11".into(), display_name: "A".into(), avatar: Some("https://img/a.png".into()), broadcaster_type: "partner".into() },
        );
        let now = build(&logins, &[stream("A", "hi", "Tetris")], &accounts, &last);
        assert_eq!(
            now[0],
            TileMeta {
                login: "a".into(),
                live: true,
                title: Some("hi".into()),
                game_name: Some("Tetris".into()),
                broadcaster_type: Some("partner".into()),
                user_id: Some("11".into()),
                display_name: Some("A".into()),
                profile_image_url: Some("https://img/a.png".into()),
            }
        );
        assert_eq!(now[1], TileMeta { login: "b".into(), live: false, title: None, game_name: Some("Chess".into()), ..Default::default() });
    }

    #[test]
    fn a_live_tile_reads_offline_only_after_two_missed_polls() {
        let live = TileMeta { login: "a".into(), live: true, title: Some("hi".into()), ..Default::default() };
        let gone = TileMeta { login: "a".into(), live: false, ..Default::default() };
        let mut last = HashMap::new();
        last.insert("a".to_string(), live.clone());
        let mut misses = HashMap::new();
        assert_eq!(hold_live(vec![gone.clone()], &last, &mut misses), vec![live.clone()], "one miss is held");
        assert_eq!(hold_live(vec![gone.clone()], &last, &mut misses), vec![gone.clone()], "the second is believed");
        assert!(misses.is_empty());
        assert_eq!(hold_live(vec![live.clone()], &last, &mut misses), vec![live.clone()]);
        let mut offline_last = HashMap::new();
        offline_last.insert("a".to_string(), gone.clone());
        assert_eq!(hold_live(vec![gone.clone()], &offline_last, &mut misses), vec![gone], "offline stays offline");
    }

    #[test]
    fn only_changes_are_sent() {
        let a = TileMeta { login: "a".into(), live: true, title: Some("hi".into()), ..Default::default() };
        let mut last = HashMap::new();
        last.insert("a".to_string(), a.clone());
        assert!(changed_entries(&last, &[a.clone()]).is_empty());
        let retitled = TileMeta { title: Some("new".into()), ..a };
        assert_eq!(changed_entries(&last, &[retitled.clone()]), vec![retitled]);
    }
}
