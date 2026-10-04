//! Live line (live or not, category, viewers, title) for the chat dock's
//! chats on platforms other than Twitch.
//!
//! Twitch docked chats ride `channel_state`'s one batched viewer poll. Kick,
//! YouTube and TikTok have nothing equivalent for an arbitrary channel, so
//! their rows in the chat list had no category at all. This polls them with
//! each platform's own `live_check` (Kick batches 50 slugs per call; YouTube
//! and TikTok are per channel), on the same cadences the favourites poller
//! uses, and emits `chat-dock-live` only when something changed.
//!
//! It reads the dock straight from `commands::chat_dock`, so nothing has to
//! register: docking or closing a chat wakes it (`wake`), and with no such chat
//! docked it does nothing. It also rests while no window is on screen, since
//! only the chat list shows this.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use log::debug;
use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::Emitter;
use tokio::sync::Notify;

use crate::models::provider_stream::ProviderStream;
use crate::rt::AppHandle;
use crate::services::providers::key::{make_key, same_channel};
use crate::services::providers::registry;

pub const EVENT: &str = "chat-dock-live";

/// What a docked chat's row shows.
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

/// The current rows, for a window that is starting up.
#[tauri::command]
pub fn get_chat_dock_live() -> HashMap<String, DockLive> {
    STATE.lock().map(|s| s.live.clone()).unwrap_or_default()
}

/// The dock changed: check what is new now rather than at the next tick.
pub fn wake() {
    WAKE.notify_one();
}

/// Docked chats per non-Twitch platform, as `(provider, logins)`.
fn docked_by_provider() -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    for c in crate::commands::chat_dock::get_chat_dock().chats {
        if c.provider != "twitch" {
            out.entry(c.provider).or_default().push(c.login);
        }
    }
    out
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
        // Forget channels no longer docked.
        let keep: Vec<String> = docked
            .iter()
            .flat_map(|(p, ls)| ls.iter().map(move |l| make_key(p, l)))
            .collect();
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
        let _ = app.emit_to("main", EVENT, get_chat_dock_live());
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
}
