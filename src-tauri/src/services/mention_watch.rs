//! Mentions while you're away, the app's half (every platform).
//!
//! With Notifications > Mentions > Mentions while you're away on, the push
//! server (`notify.streamnook.app`) is told what the shared mention rule needs
//! for this account: whether a bare login and the display name count, whether
//! to push, and the users and chats to leave out. Its mention watcher reads
//! the chat of every followed channel while it is live and keeps what it finds
//! for 7 days. This pulls those into the inbox at once on launch, sign-in and
//! when a phone app comes back to the front, when a push arrives, and every few
//! minutes while the app runs; `mention_inbox::merge_remote` files them without
//! a sound, a notification or an arrival card (the pushes already came while
//! the viewer was away), deduped against what this app caught itself.
//!
//! Turning it off deletes the server's rows for the account at once. A server
//! that no longer knows the account (14 days without a pull) answers
//! `enrolled: false`, and the next pull opts back in on its own.
//!
//! Offline chats: the watcher reads a followed channel while it is live, and an
//! offline one only while the viewer holds a lease on it. Opening a Twitch chat
//! (any app reports the chats on screen through `mention_inbox::set_visible_rooms`)
//! takes or renews that chat's 14-day lease (`lease_rooms`), at most once per
//! channel per `LEASE_EVERY`, so an offline chat the viewer keeps visiting stays
//! watched.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::models::settings::Settings;
use crate::services::mention_inbox::{MentionEntry, MentionKind};

const SERVER: &str = "https://notify.streamnook.app";
/// Kept per entry, in characters (the inbox's own limit).
const MAX_TEXT: usize = 500;
/// Logins sent as ids, at most this many per list.
const MAX_IDS: usize = 200;

/// What the server is told, from the settings. Lists lowercased and sorted, so
/// two reads of the same settings compare equal.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    /// Mentions while you're away.
    pub away_alerts: bool,
    pub bare_login: bool,
    pub display_name: bool,
    /// Whether the server pushes: notifications on and Mentions and replies on.
    pub push: bool,
    /// Chat filters' hidden users, by login.
    pub hidden_logins: Vec<String>,
    /// Twitch chats muted for mentions, by login.
    pub muted_logins: Vec<String>,
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.retain(|s| !s.is_empty());
    v.sort();
    v.dedup();
    v.truncate(MAX_IDS);
    v
}

impl Inputs {
    pub fn from_settings(settings: &Settings) -> Self {
        let hidden = settings
            .extra
            .get("chat_filters")
            .and_then(|f| f.get("hidden_users"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|n| n.trim().trim_start_matches('@').to_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        // Only Twitch chats are read by the watcher: a room with a provider
        // prefix (`kick:slug`) has nothing to leave out there.
        let muted = settings
            .mentions
            .muted_channels
            .iter()
            .map(|r| r.trim().to_lowercase())
            .filter(|r| !r.contains(':'))
            .collect();
        Self {
            away_alerts: settings.mentions.away_alerts,
            bare_login: settings.mentions.match_bare_login,
            display_name: settings.mentions.match_display_name,
            push: settings.live_notifications.enabled && settings.live_notifications.show_mention_notifications,
            hidden_logins: sorted(hidden),
            muted_logins: sorted(muted),
        }
    }
}

/// How often one channel's lease is renewed from here; the server takes it no
/// more often either.
pub const LEASE_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// The on-screen rooms whose lease is due: Twitch chats only (a bare login),
/// each once, none renewed within `LEASE_EVERY`. Stamps the ones it returns.
pub fn due_leases(
    rooms: &[String],
    stamps: &mut std::collections::HashMap<String, std::time::Instant>,
    now: std::time::Instant,
) -> Vec<String> {
    let mut out = Vec::new();
    for room in rooms {
        let login = room.trim().trim_start_matches('#').to_lowercase();
        let twitch = !login.is_empty() && login.len() <= 25 && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !twitch || out.contains(&login) {
            continue;
        }
        if stamps.get(&login).is_some_and(|at| now.duration_since(*at) < LEASE_EVERY) {
            continue;
        }
        stamps.insert(login.clone(), now);
        out.push(login);
    }
    out
}

/// The `/mentions/optin` body, the lists already resolved to Twitch ids.
pub fn optin_body(inputs: &Inputs, hidden_ids: &[String], muted_ids: &[String]) -> Value {
    json!({
        "match_bare_login": inputs.bare_login,
        "match_display_name": inputs.display_name,
        "push": inputs.push,
        "hidden_user_ids": hidden_ids,
        "muted_channel_ids": muted_ids,
    })
}

/// One stored mention as `/mentions/pending` returns it.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct RemoteMention {
    pub id: String,
    #[serde(default)]
    pub platform: String,
    pub room: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    pub kind: String,
    pub sender_id: String,
    pub sender_login: String,
    #[serde(default)]
    pub sender_name: String,
    #[serde(default)]
    pub color: String,
    pub text: String,
    #[serde(default)]
    pub reply_parent_id: Option<String>,
    pub at_ms: i64,
}

impl RemoteMention {
    /// The inbox entry, or None for anything the inbox cannot hold (another
    /// platform, an unknown kind, no id).
    pub fn into_entry(self) -> Option<MentionEntry> {
        let platform = if self.platform.is_empty() { "twitch".to_string() } else { self.platform.to_lowercase() };
        if platform != "twitch" || self.id.is_empty() {
            return None;
        }
        let kind = match self.kind.as_str() {
            "mention" => MentionKind::Mention,
            "reply" => MentionKind::Reply,
            _ => return None,
        };
        let room = self.room.trim().trim_start_matches('#').to_lowercase();
        if room.is_empty() {
            return None;
        }
        let color = (!self.color.is_empty()).then_some(self.color.as_str());
        Some(MentionEntry {
            id: self.id,
            watch_url: crate::services::providers::watch_urls::watch_url("twitch", &room),
            open_key: room.clone(),
            channel_login: room.clone(),
            room,
            platform,
            channel_id: self.channel_id.filter(|c| !c.is_empty()),
            sender_name: if self.sender_name.is_empty() { self.sender_login.clone() } else { self.sender_name },
            sender_id: self.sender_id,
            sender_login: self.sender_login.to_lowercase(),
            sender_color: Some(crate::services::chat_row_view::readable_color(color)),
            text: self.text.chars().take(MAX_TEXT).collect(),
            parts: Vec::new(),
            kind,
            reply_parent_id: self.reply_parent_id.filter(|p| !p.is_empty()),
            at_ms: self.at_ms,
            read: false,
            removed: false,
        })
    }
}

/// One `/mentions/pending` answer.
#[derive(Deserialize, Debug)]
pub struct Pending {
    pub enrolled: bool,
    #[serde(default)]
    pub entries: Vec<Value>,
    #[serde(default)]
    pub next: i64,
    #[serde(default)]
    pub more: bool,
}

impl Pending {
    /// The entries the inbox can hold; a malformed one is left out, not fatal.
    pub fn mentions(&self) -> Vec<MentionEntry> {
        self.entries
            .iter()
            .filter_map(|v| serde_json::from_value::<RemoteMention>(v.clone()).ok())
            .filter_map(RemoteMention::into_entry)
            .collect()
    }
}

/// Whether a pushed mention may show as a notification here: the switches the
/// app's own notices follow, the chat not muted, streamer mode off, and not one
/// this app already caught (or whose chat is on screen). `settings` is the
/// saved settings file, so this works in a process Android started for the
/// push alone.
pub fn claim_push(settings: &Value, id: &str, room: &str) -> bool {
    let notif = |name: &str| {
        settings
            .get("live_notifications")
            .and_then(|n| n.get(name))
            .and_then(Value::as_bool)
            .unwrap_or(true)
    };
    if !notif("enabled") || !notif("show_mention_notifications") {
        return false;
    }
    let room = room.trim().to_lowercase();
    let muted = settings
        .get("mentions")
        .and_then(|m| m.get("muted_channels"))
        .and_then(Value::as_array)
        .is_some_and(|a| a.iter().filter_map(Value::as_str).any(|r| r.trim().eq_ignore_ascii_case(&room)));
    if muted || crate::services::streamer_mode::StreamerMode::is_active() {
        return false;
    }
    !crate::services::mention_inbox::caught_here(id, &room)
}

/// The running app's side: the server calls, the debounce and the pulls.
pub mod client {
    use super::*;
    use crate::services::twitch_service::TwitchService;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// How long a run of settings changes settles before the one opt-in.
    const DEBOUNCE: Duration = Duration::from_secs(3);
    /// While the app runs with the switch on, a pull this often, so an app
    /// left open (the desktop) still collects what the watcher found.
    const PULL_EVERY: Duration = Duration::from_secs(5 * 60);
    /// A resume or a push pulls at most this often.
    const PULL_FLOOR: Duration = Duration::from_secs(20);
    /// Pages one pull reads at most (200 each).
    const MAX_PAGES: usize = 5;

    static GENERATION: AtomicU64 = AtomicU64::new(0);
    static INPUTS: Mutex<Option<Inputs>> = Mutex::new(None);
    static PULLING: AtomicBool = AtomicBool::new(false);
    static LOOP_STARTED: AtomicBool = AtomicBool::new(false);
    static LAST_PULL: Mutex<Option<Instant>> = Mutex::new(None);
    /// Twitch logins already resolved to ids.
    static IDS: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);
    /// When each chat's lease was last taken from here.
    static LEASED: Mutex<Option<HashMap<String, Instant>>> = Mutex::new(None);

    fn inputs() -> Inputs {
        INPUTS.lock().ok().and_then(|i| i.clone()).unwrap_or_default()
    }

    /// The settings as loaded at launch: with the switch on, the inbox pulls
    /// at once and the server is told (which also counts as seeing the account).
    pub fn seed(settings: &Settings) {
        let next = Inputs::from_settings(settings);
        let on = next.away_alerts;
        if let Ok(mut i) = INPUTS.lock() {
            *i = Some(next);
        }
        ensure_loop();
        if on {
            pull_soon();
            schedule();
        }
    }

    /// After every settings save: tell the server when anything it holds moved
    /// (turning the switch off opts out at once).
    pub fn refresh(settings: &Settings) {
        let next = Inputs::from_settings(settings);
        let (changed, was_on) = match INPUTS.lock() {
            Ok(mut i) => {
                let was_on = i.as_ref().is_some_and(|p| p.away_alerts);
                let changed = i.as_ref() != Some(&next);
                *i = Some(next.clone());
                (changed, was_on)
            }
            Err(_) => (false, false),
        };
        if !changed {
            return;
        }
        if next.away_alerts {
            schedule();
        } else if was_on {
            GENERATION.fetch_add(1, Ordering::SeqCst);
            tauri::async_runtime::spawn(async {
                match post("/mentions/optout", &json!({})).await {
                    Ok(_) => log::info!("[MentionWatch] opted out"),
                    Err(e) => log::warn!("[MentionWatch] opt-out failed: {e}"),
                }
            });
        }
    }

    /// A different account signed in (or the same one again): its own pull at
    /// once, then its own opt-in.
    pub fn on_account_changed() {
        if inputs().away_alerts {
            pull_soon();
            schedule();
        }
    }

    /// The opt-in, then a pull, once things settle.
    pub fn schedule() {
        let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(DEBOUNCE).await;
            if GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            match sync().await {
                Ok(()) => {
                    if let Err(e) = pull().await {
                        log::warn!("[MentionWatch] pull failed: {e}");
                    }
                }
                Err(e) => log::warn!("[MentionWatch] opt-in failed: {e}"),
            }
        });
    }

    /// Launch, sign-in, a phone app back in front, or a push: pull now, unless
    /// one succeeded a moment ago.
    pub fn pull_soon() {
        if !inputs().away_alerts {
            return;
        }
        let recent = LAST_PULL.lock().ok().and_then(|l| *l).is_some_and(|t| t.elapsed() < PULL_FLOOR);
        if recent {
            return;
        }
        tauri::async_runtime::spawn(async {
            if let Err(e) = pull().await {
                log::warn!("[MentionWatch] pull failed: {e}");
            }
        });
    }

    fn ensure_loop() {
        if LOOP_STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        tauri::async_runtime::spawn(async {
            loop {
                tokio::time::sleep(PULL_EVERY).await;
                if inputs().away_alerts {
                    if let Err(e) = pull().await {
                        log::debug!("[MentionWatch] timed pull failed: {e}");
                    }
                }
            }
        });
    }

    async fn request(method: reqwest::Method, path: &str, body: Option<&Value>) -> Result<reqwest::Response, String> {
        let token = TwitchService::get_token().await.map_err(|e| e.to_string())?;
        let mut req = crate::services::http::client()
            .clone()
            .request(method, format!("{SERVER}{path}"))
            .header(reqwest::header::AUTHORIZATION, format!("OAuth {token}"));
        if let Some(body) = body {
            req = req.json(body);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("{path} failed: {status}"));
        }
        Ok(resp)
    }

    async fn post(path: &str, body: &Value) -> Result<Value, String> {
        let resp = request(reqwest::Method::POST, path, Some(body)).await?;
        resp.json::<Value>().await.map_err(|e| e.to_string())
    }

    /// Twitch ids for these logins, from the cache or Twitch; a login Twitch
    /// does not know is left out.
    async fn ids_for(logins: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for login in logins {
            let known = IDS.lock().ok().and_then(|m| m.as_ref().and_then(|m| m.get(login).cloned()));
            let id = match known {
                Some(id) => id,
                None => match TwitchService::get_user_by_login(login).await {
                    Ok(user) => {
                        if let Ok(mut m) = IDS.lock() {
                            m.get_or_insert_with(HashMap::new).insert(login.clone(), user.id.clone());
                        }
                        user.id
                    }
                    Err(_) => continue,
                },
            };
            if !id.is_empty() {
                out.push(id);
            }
        }
        out
    }

    /// Tells the server this account's rule now (opt-in is also how it learns
    /// the account is still in use).
    async fn sync() -> Result<(), String> {
        let inputs = inputs();
        if !inputs.away_alerts {
            return Ok(());
        }
        let hidden = ids_for(&inputs.hidden_logins).await;
        let muted = ids_for(&inputs.muted_logins).await;
        post("/mentions/optin", &optin_body(&inputs, &hidden, &muted)).await?;
        log::info!("[MentionWatch] opted in");
        Ok(())
    }

    /// Pulls what the server holds past this account's cursor into the inbox.
    /// Answers how many entries were new. One pull at a time.
    pub async fn pull() -> Result<u32, String> {
        if !inputs().away_alerts {
            return Ok(0);
        }
        if PULLING.swap(true, Ordering::SeqCst) {
            return Ok(0);
        }
        let out = pull_pages().await;
        // Only a pull that worked holds the next one off: one tried before
        // sign-in must not delay the pull sign-in starts.
        if out.is_ok() {
            if let Ok(mut l) = LAST_PULL.lock() {
                *l = Some(Instant::now());
            }
        }
        PULLING.store(false, Ordering::SeqCst);
        out
    }

    async fn pull_pages() -> Result<u32, String> {
        let mut cursor = crate::services::mention_inbox::remote_cursor().ok_or("not signed in")?;
        let mut filed = 0u32;
        let mut reenrolled = false;
        for _ in 0..MAX_PAGES {
            let resp = request(reqwest::Method::GET, &format!("/mentions/pending?since={cursor}"), None).await?;
            let page: Pending = resp.json().await.map_err(|e| e.to_string())?;
            if !page.enrolled {
                // The server let the account go (14 days unseen) while the
                // switch stayed on: opt back in, once.
                if reenrolled {
                    break;
                }
                reenrolled = true;
                sync().await?;
                continue;
            }
            filed += crate::services::mention_inbox::merge_remote(page.mentions(), page.next).unwrap_or(0) as u32;
            cursor = cursor.max(page.next);
            if !page.more {
                break;
            }
        }
        if filed > 0 {
            log::info!("[MentionWatch] pulled {filed} new");
        }
        Ok(filed)
    }

    /// The chats on screen now (room keys, from every app): each Twitch one takes
    /// or renews its lease, so it stays watched while offline. Only with the
    /// switch on; a failed call forgets its stamp, so the next open tries again.
    pub fn lease_rooms(rooms: &[String]) {
        if !inputs().away_alerts {
            return;
        }
        let due = match LEASED.lock() {
            Ok(mut stamps) => due_leases(rooms, stamps.get_or_insert_with(HashMap::new), Instant::now()),
            Err(_) => return,
        };
        if due.is_empty() {
            return;
        }
        tauri::async_runtime::spawn(async move {
            for login in due {
                let result = match ids_for(std::slice::from_ref(&login)).await.into_iter().next() {
                    Some(id) => post("/mentions/lease", &json!({ "channel_id": id, "login": login })).await.map(|_| ()),
                    None => Err("no Twitch id".to_string()),
                };
                if let Err(e) = result {
                    log::debug!("[MentionWatch] lease for {login} failed: {e}");
                    if let Ok(mut stamps) = LEASED.lock() {
                        if let Some(map) = stamps.as_mut() {
                            map.remove(&login);
                        }
                    }
                }
            }
        });
    }

    /// A push arrived while this app runs: pull it (and anything else) in.
    pub fn on_push_arrived() {
        if crate::services::providers::app_handle().is_some() {
            pull_soon();
        }
    }

    /// The saved settings file, as `claim_push` reads it.
    pub fn saved_settings() -> Value {
        crate::services::cache_service::get_app_data_dir()
            .ok()
            .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(f: impl FnOnce(&mut Settings)) -> Settings {
        let mut s = Settings::default();
        f(&mut s);
        s
    }

    #[test]
    fn off_by_default() {
        assert!(!Inputs::from_settings(&Settings::default()).away_alerts);
    }

    #[test]
    fn the_rule_follows_the_mention_settings_and_lists_compare_by_content() {
        let a = settings(|s| {
            s.mentions.away_alerts = true;
            s.mentions.match_bare_login = false;
            s.mentions.muted_channels = vec!["Forsen".into(), "kick:xqc".into(), "forsen".into()];
            s.extra.insert("chat_filters".into(), json!({ "hidden_users": ["@Spammer", "bot", ""] }));
        });
        let inputs = Inputs::from_settings(&a);
        assert!(inputs.away_alerts && !inputs.bare_login && inputs.display_name);
        assert_eq!(inputs.muted_logins, vec!["forsen".to_string()]);
        assert_eq!(inputs.hidden_logins, vec!["bot".to_string(), "spammer".to_string()]);
        let b = settings(|s| {
            s.mentions.away_alerts = true;
            s.mentions.match_bare_login = false;
            s.mentions.muted_channels = vec!["forsen".into()];
            s.extra.insert("chat_filters".into(), json!({ "hidden_users": ["spammer", "BOT"] }));
        });
        assert_eq!(Inputs::from_settings(&b), inputs);
    }

    #[test]
    fn push_follows_the_notification_switches() {
        let on = settings(|s| s.mentions.away_alerts = true);
        assert!(Inputs::from_settings(&on).push);
        let quiet = settings(|s| {
            s.mentions.away_alerts = true;
            s.live_notifications.show_mention_notifications = false;
        });
        assert!(!Inputs::from_settings(&quiet).push);
        let master_off = settings(|s| {
            s.mentions.away_alerts = true;
            s.live_notifications.enabled = false;
        });
        assert!(!Inputs::from_settings(&master_off).push);
    }

    #[test]
    fn the_optin_body_is_the_servers_contract() {
        let inputs = Inputs { away_alerts: true, bare_login: true, display_name: false, push: true, ..Default::default() };
        assert_eq!(
            optin_body(&inputs, &["9".into()], &["500".into()]),
            json!({
                "match_bare_login": true,
                "match_display_name": false,
                "push": true,
                "hidden_user_ids": ["9"],
                "muted_channel_ids": ["500"],
            })
        );
    }

    fn pending_json() -> Value {
        json!({
            "enrolled": true,
            "next": 1792424190000i64,
            "more": false,
            "entries": [
                {
                    "id": "a1", "platform": "twitch", "room": "BigStreamer", "channel_id": "500",
                    "kind": "reply", "sender_id": "777", "sender_login": "Viewer", "sender_name": "Viewer",
                    "color": "#FF0000", "text": "thanks @alice", "reply_parent_id": "p9", "at_ms": 1792424183000i64
                },
                { "id": "a2", "platform": "kick", "room": "x", "kind": "mention", "sender_id": "1", "sender_login": "y", "text": "", "at_ms": 1 },
                { "id": "a3", "platform": "twitch", "room": "x", "kind": "word", "sender_id": "1", "sender_login": "y", "text": "", "at_ms": 1 },
                { "broken": true }
            ]
        })
    }

    #[test]
    fn a_pending_page_files_only_what_the_inbox_can_hold() {
        let page: Pending = serde_json::from_value(pending_json()).unwrap();
        let mentions = page.mentions();
        assert_eq!(mentions.len(), 1);
        let e = &mentions[0];
        assert_eq!((e.id.as_str(), e.room.as_str(), e.open_key.as_str()), ("a1", "bigstreamer", "bigstreamer"));
        assert_eq!(e.kind, MentionKind::Reply);
        assert_eq!(e.reply_parent_id.as_deref(), Some("p9"));
        assert_eq!(e.channel_id.as_deref(), Some("500"));
        assert_eq!(e.sender_login, "viewer");
        assert!(!e.read && !e.removed && e.parts.is_empty());
        assert!(e.watch_url.contains("bigstreamer"));
    }

    #[test]
    fn a_lease_is_taken_for_twitch_chats_only_once_per_six_hours() {
        use std::collections::HashMap;
        use std::time::{Duration, Instant};
        let mut stamps = HashMap::new();
        let t0 = Instant::now();
        let rooms: Vec<String> = ["Hamlin", "#hamlin", "kick:someone", "youtube:@x", "", "other_one"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(due_leases(&rooms, &mut stamps, t0), vec!["hamlin".to_string(), "other_one".to_string()]);
        assert!(due_leases(&rooms, &mut stamps, t0 + Duration::from_secs(60 * 60)).is_empty());
        assert_eq!(
            due_leases(&["hamlin".to_string()], &mut stamps, t0 + LEASE_EVERY + Duration::from_secs(1)),
            vec!["hamlin".to_string()]
        );
    }

    #[test]
    fn an_unenrolled_answer_reads() {
        let page: Pending = serde_json::from_value(json!({ "enrolled": false, "entries": [], "next": 0, "more": false })).unwrap();
        assert!(!page.enrolled && page.mentions().is_empty());
    }

    #[test]
    fn a_push_shows_only_with_the_switches_on_and_the_chat_not_muted() {
        let on = json!({ "live_notifications": { "enabled": true, "show_mention_notifications": true }, "mentions": { "muted_channels": [] } });
        assert!(claim_push(&on, "m1", "bigstreamer"));
        assert!(claim_push(&Value::Null, "m1", "bigstreamer"), "never-saved switches read as on");
        let quiet = json!({ "live_notifications": { "enabled": true, "show_mention_notifications": false } });
        assert!(!claim_push(&quiet, "m1", "bigstreamer"));
        let master = json!({ "live_notifications": { "enabled": false } });
        assert!(!claim_push(&master, "m1", "bigstreamer"));
        let muted = json!({ "mentions": { "muted_channels": ["BigStreamer"] } });
        assert!(!claim_push(&muted, "m1", "bigstreamer"));
    }
}
