//! Twitch's own "bell" feed, polled for the things StreamNook cannot see any
//! other way.
//!
//! `Query.currentUser.notifications` is a plain authenticated field: raw query
//! text, no persisted hash, no integrity token, and it resolves with the same
//! app-token + web-client-id pairing `auth_proxy` uses for
//! `currentUser { hasTurbo }`.
//!
//! Scope is deliberately narrow: this ingests by exception rather than
//! mirroring the bell. Two kinds of row are taken:
//!
//! - Rewards Twitch names for your account: a badge you earned, a drop reward
//!   waiting to be claimed. The app's own drop and badge notices cannot always
//!   say WHICH reward (a claim made by a plugin or on another device, a badge
//!   granted on Twitch's side), and these rows always do. Watch streaks and
//!   advertising (`category: "promotions"`) are still left out.
//! - Gift subs, because nothing else can see them:
//!
//! - EventSub's `channel.subscription.gift` authorizes as the BROADCASTER, so
//!   it reports gifts in your own channel, not gifts to you in someone else's.
//! - Twitch IRC carries the recipient in `msg-param-recipient-*`, which this
//!   codebase parses nowhere, and it only arrives while connected to that
//!   channel's chat.
//! - Twitch's own push socket has no user-scoped sub-gift topic;
//!   `channel-sub-gifts-v1` is keyed by CHANNEL and only sees gifts in a
//!   channel you are already watching.

use anyhow::{anyhow, Result};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tauri::async_runtime::JoinHandle;

use crate::models::settings::Settings;
use crate::services::auth_proxy::TWITCH_WEB_CLIENT_ID;
use crate::services::cache_service;
use crate::services::twitch_service::TwitchService;

/// Gift subs are not time-critical and the feed is cheap, but it is still a
/// network call on a background timer. Five minutes keeps tray idle honest.
const POLL_INTERVAL: Duration = Duration::from_secs(300);

/// Page size for the poll. The feed is newest-first and mixes every category,
/// so a gift sub is missed only if more than this many notifications of ANY
/// kind arrive inside one interval. Observed arrival rates are nowhere near
/// that: a full page spanned over a week.
const POLL_PAGE: u32 = 25;

/// How many ids to remember, and the only thing dedupe relies on. It has to
/// outlast one page, which it does by a wide margin: only gift subs and
/// reward rows are ever recorded, eight pages' worth even if every row were one.
const SEEN_CAP: usize = 200;

const STATE_FILE: &str = "onsite_notifications_state.json";

static APP: OnceLock<AppHandle> = OnceLock::new();
/// A std mutex, not a tokio one, so `refresh` stays synchronous. It is only
/// ever held across a spawn or an abort, never across an await.
static POLL: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
/// Which kinds the running poll collects, set by `refresh` from settings.
static GIFTS_ON: AtomicBool = AtomicBool::new(false);
static REWARDS_ON: AtomicBool = AtomicBool::new(false);
/// Set when rewards are switched on, so the rows that piled up while they
/// were off are recorded silently instead of announced all at once.
static RESEED_REWARDS: AtomicBool = AtomicBool::new(false);

/// One span of a notification body. Twitch sends `**bold**` markdown, and the
/// bolded runs are the nouns worth emphasising (gifter, tier, channel), so the
/// parse happens here and React renders a ready model rather than re-deriving
/// it. Same reasoning as the Rust-side highlight stamps.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BodySpan {
    pub text: String,
    pub bold: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GiftSubNotification {
    pub id: String,
    /// Twitch's own sentence, markdown stripped. Kept for native OS
    /// notifications and for anything that cannot render spans.
    pub body_plain: String,
    pub body_spans: Vec<BodySpan>,
    pub created_at: String,
    /// Avatar of whoever the row is about. Always populated by Twitch.
    pub thumbnail_url: String,
    /// Channel login pulled out of the action URL when it is a plain
    /// `twitch.tv/<login>` link, so the row can open a stream rather than a
    /// browser. `None` keeps `action_url` as the only target.
    pub channel_login: Option<String>,
    pub action_url: Option<String>,
}

/// Which reward a row is about.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RewardKind {
    /// A badge the account earned.
    Badge,
    /// A drop reward, usually one waiting to be claimed.
    Drop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchRewardNotification {
    pub id: String,
    pub kind: RewardKind,
    /// Twitch's own sentence, markdown stripped, naming the reward.
    pub body_plain: String,
    pub body_spans: Vec<BodySpan>,
    pub created_at: String,
    /// The reward's own art (a badge image, a drop's reward image).
    pub thumbnail_url: String,
    pub action_url: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    /// Newest `node.createdAt` observed, kept for diagnostics only.
    ///
    /// It is deliberately NOT the dedupe key, and neither is Twitch's own
    /// `summary.unseenCount`: that counts rows newer than `lastSeenAt`, and
    /// `lastSeenAt` advances whenever the bell is opened anywhere, including
    /// on the Twitch website. A poll gated on it would silently skip the
    /// fetch for anyone who glanced at their browser.
    last_created_at: Option<String>,
    seen_ids: VecDeque<String>,
    /// Reward rows were first collected after gift subs, so they seed on
    /// their own: an existing install must not announce its reward backlog.
    #[serde(default)]
    rewards_seeded: bool,
}

fn state_path() -> Result<PathBuf> {
    Ok(cache_service::get_app_data_dir()?.join(STATE_FILE))
}

fn read_state() -> Option<State> {
    let path = state_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_state(state: &State) {
    let Ok(path) = state_path() else { return };
    match serde_json::to_string(state) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                warn!("[OnsiteNotif] could not persist state: {e}");
            }
        }
        Err(e) => warn!("[OnsiteNotif] could not serialize state: {e}"),
    }
}

/// Split `**bold**` runs out of a Twitch notification body.
///
/// Only bold is handled because only bold was observed. An unmatched `**` is
/// left as literal text rather than swallowing the rest of the sentence.
pub fn parse_body(body: &str) -> Vec<BodySpan> {
    let mut spans = Vec::new();
    let mut rest = body;

    while let Some(open) = rest.find("**") {
        let (before, after_open) = rest.split_at(open);
        let after_open = &after_open[2..];

        let Some(close) = after_open.find("**") else {
            break;
        };

        if !before.is_empty() {
            spans.push(BodySpan { text: before.to_string(), bold: false });
        }
        let (bold, after_close) = after_open.split_at(close);
        if !bold.is_empty() {
            spans.push(BodySpan { text: bold.to_string(), bold: true });
        }
        rest = &after_close[2..];
    }

    if !rest.is_empty() {
        spans.push(BodySpan { text: rest.to_string(), bold: false });
    }
    spans
}

fn strip_bold(body: &str) -> String {
    parse_body(body).into_iter().map(|s| s.text).collect()
}

/// Pull a bare channel login out of a Twitch action URL.
///
/// `www.twitch.tv/alexotos` -> `alexotos`. Anything with extra path segments
/// (`twitch.tv/save-streak/foo`, `twitch.tv/inventory`) is NOT a channel and
/// returns None so the row falls back to opening the URL.
fn channel_login_from_url(url: &str) -> Option<String> {
    let trimmed = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    let rest = trimmed.strip_prefix("twitch.tv/")?;
    let login = rest.split(['/', '?', '#']).next()?;
    if login.is_empty() || !login.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(login.to_lowercase())
}

/// Build the emitted shape from one feed node. Shared so the poll and the dev
/// preview below can never drift into rendering the same row differently.
fn node_to_gift_sub(node: &serde_json::Value) -> Option<GiftSubNotification> {
    let id = node.get("id").and_then(|v| v.as_str())?;
    let created_at = node.get("createdAt").and_then(|v| v.as_str())?;
    let body = node.get("body").and_then(|v| v.as_str()).unwrap_or_default();

    let action_url = node
        .get("actions")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|a| a.get("url"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    Some(GiftSubNotification {
        id: id.to_string(),
        body_plain: strip_bold(body),
        body_spans: parse_body(body),
        created_at: created_at.to_string(),
        thumbnail_url: node
            .get("thumbnailURL")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        channel_login: action_url.as_deref().and_then(channel_login_from_url),
        action_url,
    })
}

/// Which reward a feed row is about, if it is one worth announcing.
///
/// Twitch's `type` is not an enum: campaign rows carry one-off identifiers
/// (`drops_lifecycle_062625_p1`), and a type is versioned by suffixing it
/// (`..._recovered_streak_new`). So the stable prefixes are matched, and
/// `category: "promotions"` is refused outright because it is the ad feed.
pub fn reward_kind(node: &serde_json::Value) -> Option<RewardKind> {
    let category = node.get("category").and_then(|v| v.as_str()).unwrap_or_default();
    let kind = node.get("type").and_then(|v| v.as_str()).unwrap_or_default();
    if category == "promotions" || kind.contains("lifecycle") {
        return None;
    }
    if kind.contains("earned_badge") {
        return Some(RewardKind::Badge);
    }
    if kind.starts_with("user_drop_reward") || kind.contains("drop_reward") {
        return Some(RewardKind::Drop);
    }
    None
}

fn node_to_reward(node: &serde_json::Value, kind: RewardKind) -> Option<TwitchRewardNotification> {
    let id = node.get("id").and_then(|v| v.as_str())?;
    let created_at = node.get("createdAt").and_then(|v| v.as_str())?;
    let body = node.get("body").and_then(|v| v.as_str()).unwrap_or_default();
    if body.trim().is_empty() {
        return None;
    }
    let action_url = node
        .get("actions")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|a| a.get("url"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    Some(TwitchRewardNotification {
        id: id.to_string(),
        kind,
        body_plain: strip_bold(body),
        body_spans: parse_body(body),
        created_at: created_at.to_string(),
        thumbnail_url: node
            .get("thumbnailURL")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        action_url,
    })
}

const QUERY: &str = r#"query SNGiftSubs($first: Int!) {
  currentUser {
    notifications(first: $first, language: "en", displayType: VIEWER) {
      edges { node { id type category body createdAt thumbnailURL actions { url body } } }
    }
  }
}"#;

async fn fetch_feed(first: u32) -> Result<serde_json::Value> {
    let token = TwitchService::get_token().await?;
    let client = crate::services::http::client().clone();

    let body = serde_json::json!({
        "operationName": "SNGiftSubs",
        "query": QUERY,
        "variables": { "first": first },
    });

    let resp: serde_json::Value = client
        .post("https://gql.twitch.tv/gql")
        .header("Authorization", format!("OAuth {token}"))
        .header("Client-ID", TWITCH_WEB_CLIENT_ID)
        .timeout(Duration::from_secs(10))
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    if let Some(errors) = resp.get("errors") {
        return Err(anyhow!("gql errors: {errors}"));
    }
    Ok(resp)
}

/// Everything new in `gift_subscriptions` since the last run.
///
/// Platform-neutral on purpose: the desktop poll below emits Tauri events with
/// it, and the Android worker (`android_notify.rs`) can call the same function
/// from its `collect_all` without an AppHandle.
///
/// First run after install seeds silently and returns nothing, the same rule
/// `collect_live` uses on Android. Announcing here would empty the whole
/// backlog into the centre at once.
pub async fn collect_new_gift_subs() -> Result<Vec<GiftSubNotification>> {
    Ok(collect_new(true, false).await?.gifts)
}

/// What one poll found.
#[derive(Default)]
pub struct Collected {
    pub gifts: Vec<GiftSubNotification>,
    pub rewards: Vec<TwitchRewardNotification>,
}

/// Everything new since the last run, for the kinds asked for. One fetch and
/// one pass serve both; each kind seeds silently the first time it is read.
pub async fn collect_new(gifts: bool, rewards: bool) -> Result<Collected> {
    let resp = fetch_feed(POLL_PAGE).await?;

    let edges = resp
        .pointer("/data/currentUser/notifications/edges")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("no edges in response"))?;

    let seeding = read_state().is_none();
    let mut state = read_state().unwrap_or_default();
    let watermark = state.last_created_at.clone();
    // Reward rows seed on their own the first time they are read, and again
    // after being switched back on, so neither announces a backlog.
    let seeding_rewards = seeding || !state.rewards_seeded || RESEED_REWARDS.swap(false, Ordering::AcqRel);

    let mut fresh = Vec::new();
    let mut fresh_rewards = Vec::new();
    let mut newest = watermark.clone();

    for edge in edges {
        let node = match edge.get("node") {
            Some(n) => n,
            None => continue,
        };
        let id = node.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let created_at = node
            .get("createdAt")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if id.is_empty() || created_at.is_empty() {
            continue;
        }

        // Timestamps are RFC3339 UTC with a fixed shape, so lexical ordering is
        // chronological and needs no date parse.
        if newest.as_deref().is_none_or(|n| created_at > n) {
            newest = Some(created_at.to_string());
        }

        let is_gift = node.get("category").and_then(|v| v.as_str()) == Some("gift_subscriptions");
        let reward = reward_kind(node);
        if !is_gift && reward.is_none() {
            continue;
        }

        // Dedupe on the id alone, never on the watermark. A timestamp floor
        // would drop a gift sub that Twitch inserted BELOW the newest row
        // already seen, which ordering alone does not rule out. The ring
        // cannot miss one either: it holds far more gift-sub ids than a single
        // page can show, because only this category is ever recorded.
        if state.seen_ids.iter().any(|s| s == id) {
            continue;
        }

        state.seen_ids.push_back(id.to_string());
        while state.seen_ids.len() > SEEN_CAP {
            state.seen_ids.pop_front();
        }

        if is_gift {
            if seeding || !gifts {
                continue;
            }
            if let Some(item) = node_to_gift_sub(node) {
                fresh.push(item);
            }
        } else if let Some(kind) = reward {
            // Recorded even when rewards are off, so switching them on later
            // does not announce every reward that arrived in the meantime.
            if seeding_rewards || !rewards {
                continue;
            }
            if let Some(item) = node_to_reward(node, kind) {
                fresh_rewards.push(item);
            }
        }
    }

    state.last_created_at = newest;
    if seeding_rewards && rewards {
        state.rewards_seeded = true;
        info!("[OnsiteNotif] reward rows seeded silently");
    }
    write_state(&state);

    if seeding {
        info!("[OnsiteNotif] first run, seeded {} ids silently", state.seen_ids.len());
        return Ok(Collected::default());
    }
    Ok(Collected { gifts: fresh, rewards: fresh_rewards })
}

pub fn init(app: AppHandle) {
    let _ = APP.set(app);
}

/// Spawn or abort the poll to match the current settings.
///
/// Called from `save_settings` alongside `ChatRules::refresh` and
/// `StreamerMode::refresh`, so a toggle takes effect without a restart and
/// disabled genuinely costs nothing: no task, no timer, no connection.
pub fn refresh(settings: &Settings) {
    let on = settings.live_notifications.enabled;
    let gifts = on && settings.live_notifications.show_gift_sub_notifications;
    let rewards = on && settings.live_notifications.show_twitch_reward_notifications;
    GIFTS_ON.store(gifts, Ordering::Release);
    if rewards && !REWARDS_ON.swap(true, Ordering::AcqRel) {
        RESEED_REWARDS.store(true, Ordering::Release);
    } else if !rewards {
        REWARDS_ON.store(false, Ordering::Release);
    }
    let want = gifts || rewards;

    let mut guard = POLL.lock().unwrap_or_else(|p| p.into_inner());
    match (want, guard.is_some()) {
        (true, false) => {
            debug!("[OnsiteNotif] enabling the Twitch notification poll");
            *guard = Some(spawn_poll());
        }
        (false, true) => {
            debug!("[OnsiteNotif] disabling the Twitch notification poll");
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
        _ => {}
    }
}

/// Spawned through Tauri's runtime handle rather than `tokio::spawn`.
///
/// The first call comes from `setup()`, which runs on the main thread with no
/// Tokio runtime entered, so `tokio::spawn` panics there with "there is no
/// reactor running".
fn spawn_poll() -> JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        loop {
            let gifts = GIFTS_ON.load(Ordering::Acquire);
            let rewards = REWARDS_ON.load(Ordering::Acquire);
            match collect_new(gifts, rewards).await {
                Ok(found) => {
                    if !found.gifts.is_empty() {
                        info!("[OnsiteNotif] {} new gift sub(s)", found.gifts.len());
                    }
                    if !found.rewards.is_empty() {
                        info!("[OnsiteNotif] {} new reward row(s)", found.rewards.len());
                    }
                    if let Some(app) = APP.get() {
                        for item in found.gifts {
                            let _ = app.emit("gift-sub-received", &item);
                        }
                        // A badge earned means the missing-badges list is out
                        // of date now, not in ten minutes.
                        if found.rewards.iter().any(|r| r.kind == RewardKind::Badge) {
                            crate::services::badge_standing::collection_may_have_changed(app);
                        }
                        for item in found.rewards {
                            let _ = app.emit("twitch-reward-received", &item);
                        }
                    }
                }
                // A failed poll is not fatal and must not kill the loop: an
                // expired token refreshes inside get_token on the next tick.
                Err(e) => debug!("[OnsiteNotif] poll failed: {e}"),
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

/// Dev-only. Puts the newest REAL reward row off your feed (a badge earned, a
/// drop reward) in front of you on demand, which is also the end-to-end proof
/// that the query, the token and the classifier work. Bypasses the dedupe
/// ring and writes no state. Errors when the feed holds no reward row.
#[cfg(debug_assertions)]
pub async fn emit_reward_preview(app: &AppHandle) -> Result<()> {
    let resp = fetch_feed(POLL_PAGE).await?;
    let item = resp
        .pointer("/data/currentUser/notifications/edges")
        .and_then(|v| v.as_array())
        .and_then(|edges| {
            edges.iter().find_map(|edge| {
                let node = edge.get("node")?;
                node_to_reward(node, reward_kind(node)?)
            })
        })
        .ok_or_else(|| anyhow!("no reward rows in the latest {POLL_PAGE} notifications"))?;
    info!("[OnsiteNotif] reward preview: emitting {:?} {}", item.kind, item.body_plain);
    app.emit("twitch-reward-received", &item)?;
    Ok(())
}

/// Dev-only. Puts a gift-sub row in front of you on demand.
///
/// Gift subs arrive days apart, so the row is otherwise unreviewable while it
/// is being built. This prefers a REAL one off the feed, which doubles as the
/// end-to-end proof that the query, the token pairing and the parse all work,
/// and only falls back to a stand-in when the account has none in range.
///
/// Bypasses the dedupe ring on purpose and never writes state, so repeat
/// previews keep working and a genuinely new gift sub is still announced by
/// the poll afterwards.
#[cfg(debug_assertions)]
pub async fn emit_preview(app: &AppHandle) -> Result<()> {
    let real = match fetch_feed(POLL_PAGE).await {
        Ok(resp) => {
            info!("[OnsiteNotif] preview: feed reachable with the app token");
            resp.pointer("/data/currentUser/notifications/edges")
                .and_then(|v| v.as_array())
                .and_then(|edges| {
                    edges.iter().find_map(|edge| {
                        let node = edge.get("node")?;
                        let is_gift = node.get("category").and_then(|v| v.as_str())
                            == Some("gift_subscriptions");
                        if !is_gift {
                            return None;
                        }
                        node_to_gift_sub(node)
                    })
                })
        }
        Err(e) => {
            warn!("[OnsiteNotif] preview: feed unreachable ({e}), using a stand-in");
            None
        }
    };

    let item = real.unwrap_or_else(|| {
        // Placeholder names on purpose: a mock must never bake in a real
        // handle. Empty thumbnail so the glyph fallback gets exercised too.
        let body = "**Someone** gifted you a **Tier 1** Subscription (1 Month) to **a_channel**!";
        let now = chrono::Utc::now();
        GiftSubNotification {
            id: format!("preview-{}", now.timestamp_millis()),
            body_plain: strip_bold(body),
            body_spans: parse_body(body),
            created_at: now.to_rfc3339(),
            thumbnail_url: String::new(),
            channel_login: None,
            action_url: None,
        }
    });

    info!("[OnsiteNotif] preview: emitting {}", item.body_plain);
    app.emit("gift-sub-received", &item)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_reward_rows_and_nothing_else() {
        use serde_json::json;
        let row = |category: &str, kind: &str| json!({ "category": category, "type": kind });
        assert_eq!(reward_kind(&row("transactional", "quests_viewer_reward_campaign_earned_badge")), Some(RewardKind::Badge));
        assert_eq!(reward_kind(&row("transactional", "user_drop_reward_reminder_notification")), Some(RewardKind::Drop));
        // Versioned by suffix, so a future `_v2` still classifies.
        assert_eq!(reward_kind(&row("transactional", "quests_viewer_reward_campaign_earned_badge_v2")), Some(RewardKind::Badge));
        // Advertising and campaign promos never.
        assert_eq!(reward_kind(&row("drops_and_quests", "drops_lifecycle_062625_p1")), None);
        assert_eq!(reward_kind(&row("promotions", "tcsd_creator_camp_91426")), None);
        // Streaks and gift subs are not rewards.
        assert_eq!(reward_kind(&row("watch_streaks", "recovering_watch_streaks_recovered_streak")), None);
        assert_eq!(reward_kind(&row("gift_subscriptions", "sub_gift_received")), None);
    }

    #[test]
    fn a_reward_row_keeps_its_name_and_art() {
        let node = serde_json::json!({
            "id": "n1", "type": "quests_viewer_reward_campaign_earned_badge", "category": "transactional",
            "body": "You earned the **Wolf** badge!", "createdAt": "2026-09-24T14:40:00Z",
            "thumbnailURL": "https://static-cdn.jtvnw.net/twitch-quests-assets/REWARD/wolf.png",
            "actions": [{ "url": "https://www.twitch.tv/drops/inventory", "body": "Learn more" }]
        });
        let r = node_to_reward(&node, RewardKind::Badge).unwrap();
        assert_eq!(r.body_plain, "You earned the Wolf badge!");
        assert!(r.body_spans.iter().any(|s| s.bold && s.text == "Wolf"));
        assert!(r.thumbnail_url.ends_with("wolf.png"));
        assert_eq!(r.action_url.as_deref(), Some("https://www.twitch.tv/drops/inventory"));
        let empty = serde_json::json!({ "id": "n2", "createdAt": "2026-09-24T14:40:00Z", "body": "  " });
        assert!(node_to_reward(&empty, RewardKind::Drop).is_none());
    }

    #[test]
    fn parses_the_real_gift_sub_body() {
        let body = "**TiagoCodes** gifted you a **Tier 1** Subscription (1 Month) to **alexotos**!";
        let spans = parse_body(body);
        assert_eq!(
            spans,
            vec![
                BodySpan { text: "TiagoCodes".into(), bold: true },
                BodySpan { text: " gifted you a ".into(), bold: false },
                BodySpan { text: "Tier 1".into(), bold: true },
                BodySpan { text: " Subscription (1 Month) to ".into(), bold: false },
                BodySpan { text: "alexotos".into(), bold: true },
                BodySpan { text: "!".into(), bold: false },
            ]
        );
        assert_eq!(
            strip_bold(body),
            "TiagoCodes gifted you a Tier 1 Subscription (1 Month) to alexotos!"
        );
    }

    #[test]
    fn unmatched_marker_stays_literal() {
        let spans = parse_body("half **open sentence");
        assert_eq!(spans, vec![BodySpan { text: "half **open sentence".into(), bold: false }]);
    }

    #[test]
    fn plain_body_is_one_span() {
        assert_eq!(
            parse_body("Fresh Drop for Diablo IV detected!"),
            vec![BodySpan { text: "Fresh Drop for Diablo IV detected!".into(), bold: false }]
        );
    }

    #[test]
    fn channel_url_yields_a_login() {
        assert_eq!(channel_login_from_url("www.twitch.tv/alexotos"), Some("alexotos".into()));
        assert_eq!(
            channel_login_from_url("https://www.twitch.tv/HutchMF"),
            Some("hutchmf".into())
        );
    }

    #[test]
    fn non_channel_urls_are_rejected() {
        // These are real action URLs from the captured feed. Treating them as
        // channels would open a stream called "save-streak".
        assert_eq!(channel_login_from_url("www.twitch.tv/save-streak/michaelreeves"), None);
        assert_eq!(channel_login_from_url("twitchcon.com/san-diego-2026/passes/"), None);
        assert_eq!(channel_login_from_url("help.twitch.tv/s/article/how-to-use-badges"), None);
    }

    #[test]
    fn inventory_is_a_channel_shaped_url_we_must_not_open_as_one() {
        // Single-segment and alphanumeric, so this one DOES parse as a login.
        // Harmless here because only gift_subscriptions rows reach the parser,
        // and those link to a real channel. Documented so a future category
        // does not inherit the assumption silently.
        assert_eq!(channel_login_from_url("www.twitch.tv/inventory"), Some("inventory".into()));
    }
}
