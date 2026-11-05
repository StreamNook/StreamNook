// BetterTTV Pro loyalty-badge resolver.
//
// BTTV has two unrelated badge systems. The contributor badges (Developer,
// Translator, Emote Approver, Support Volunteer) come from the cached/badges
// REST feed and are handled in badge_service.rs. The BTTV *Pro* loyalty badge
// (what a Pro subscriber enables) is NOT in any REST endpoint -- the per-user
// cached/users/twitch/{id} has no badge field. It is delivered only over BTTV's
// live-update WebSocket.
//
// Trick: sending `broadcast_me` for a user id makes the server immediately reply
// with that user's `lookup_user` event, which carries { pro, glow, badge:{ url,
// startedAt } }. This works as a general on-demand lookup for ANY user in ANY
// channel (no auth, no special headers). Crucially, NON-Pro users get no reply
// at all, so a lookup either resolves fast (Pro) or times out (everyone else).
// We therefore resolve on demand, off the profile card's critical path, and
// cache the answer (including the negative one) so the vast majority of non-Pro
// users aren't re-queried on every profile open. A socket that cannot be asked
// is a failure, not a "not Pro", and is never cached.

use crate::services::shared_lookup::{Lookup, SharedLookup};
use futures_util::{SinkExt, StreamExt};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const BTTV_WS_URL: &str = "wss://sockets.betterttv.net/ws";
// Pro replies arrive ~150ms after broadcast; non-Pro users never reply, so this
// is mostly the ceiling we wait before concluding "not Pro". Generous enough to
// also absorb the TLS handshake on a cold connection.
const LOOKUP_TIMEOUT_MS: u64 = 2500;
// Re-check a user at most this often. Covers new Pro subs / lapses without
// hammering the socket for the (vast) majority of non-Pro users.
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BttvProBadge {
    pub url: String,
    pub started_at: Option<String>,
    pub glow: bool,
}

/// Every looked-up answer, positive and negative, one read per user at a time.
/// A non-Pro user is only known after the socket's 2.5 s of silence, so the read
/// runs detached and caches that answer even when a caller stopped waiting.
static LOOKUPS: LazyLock<SharedLookup<Option<BttvProBadge>>> = LazyLock::new(|| SharedLookup::new(2000));

/// The longest `resolve_bttv_pro_badge` waits: the socket's own deadline and a
/// second for the connection and the hand-off, never less than that deadline.
const RESOLVE_BUDGET: Duration = Duration::from_millis(LOOKUP_TIMEOUT_MS + 1000);

/// Forget every looked-up badge; the next profile open asks again.
pub async fn clear_cache() {
    LOOKUPS.clear();
}

async fn lookup(user_id: &str, budget: Duration) -> Lookup<Option<BttvProBadge>> {
    let id = user_id.to_string();
    LOOKUPS.lookup(user_id, CACHE_TTL, budget, move || async move { lookup_over_socket(&id).await }).await
}

/// Accumulate every distinct Pro badge image we ever see, from ANY user, so
/// the BetterTTV tab can show the full set of discovered loyalty designs (not
/// just the signed-in user's own). Persisted so it grows across sessions.
fn note(badge: &Option<BttvProBadge>) {
    if let Some(b) = badge {
        record_discovered_badge(&b.url);
    }
}

/// Resolve a user's BTTV Pro loyalty badge: `Ok(None)` when they don't have
/// Pro (the socket took the question and stayed silent, which is how BTTV says
/// so), `Err` when the socket could not be asked at all (no connection, a failed
/// send, closed before answering). Answers are cached with a TTL, negative ones
/// too; a failure is not, so the next open asks again.
pub async fn resolve_bttv_pro_badge(user_id: &str) -> Result<Option<BttvProBadge>, String> {
    if user_id.is_empty() {
        return Ok(None);
    }
    match lookup(user_id, RESOLVE_BUDGET).await {
        Lookup::Fresh(badge) | Lookup::Stale(badge) => {
            note(&badge);
            Ok(badge)
        }
        Lookup::Failed(e) => Err(e),
        Lookup::Pending => Err("BTTV socket: no answer in time".to_string()),
    }
}

/// The same lookup for a surface that cannot wait the full socket deadline (the
/// profile sheet, the You header): `Some(answer)` (`Some(None)` is a real "not
/// Pro"), or `None` when nothing is known yet. The read goes on in the
/// background and the next ask finds it.
pub async fn pro_badge_within(user_id: &str, budget: Duration) -> Option<Option<BttvProBadge>> {
    if user_id.is_empty() {
        return Some(None);
    }
    let badge = lookup(user_id, budget).await.value()?;
    note(&badge);
    Some(badge)
}

// ============================================================================
// DISCOVERED PRO BADGES (cross-user gallery, persisted to disk)
// ============================================================================
//
// There is no public catalog of BTTV Pro loyalty tiers, so we build one
// organically: every distinct badge image URL returned by a lookup (for the
// signed-in user OR any profile opened) is recorded here and persisted to
// `bttv_discovered_pro_badges.json` in the app data dir. The BetterTTV tab
// renders one tile per discovered URL regardless of ownership.

struct DiscoveredStore {
    urls: Vec<String>,
    loaded: bool,
}

impl DiscoveredStore {
    fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Some(path) = discovered_path() {
            if let Ok(txt) = std::fs::read_to_string(&path) {
                if let Ok(urls) = serde_json::from_str::<Vec<String>>(&txt) {
                    self.urls = urls;
                }
            }
        }
    }

    fn save(&self) {
        if let Some(path) = discovered_path() {
            if let Ok(txt) = serde_json::to_string(&self.urls) {
                let _ = std::fs::write(&path, txt);
            }
        }
    }
}

static DISCOVERED: Lazy<std::sync::Mutex<DiscoveredStore>> = Lazy::new(|| {
    std::sync::Mutex::new(DiscoveredStore {
        urls: Vec::new(),
        loaded: false,
    })
});

fn discovered_path() -> Option<std::path::PathBuf> {
    crate::services::cache_service::get_app_data_dir()
        .ok()
        .map(|d| d.join("bttv_discovered_pro_badges.json"))
}

fn record_discovered_badge(url: &str) {
    if url.is_empty() {
        return;
    }
    if let Ok(mut store) = DISCOVERED.lock() {
        store.ensure_loaded();
        if !store.urls.iter().any(|u| u == url) {
            store.urls.push(url.to_string());
            store.save();
        }
    }
}

/// All distinct BTTV Pro badge image URLs discovered across every user lookup so
/// far (persisted). The BetterTTV tab renders one tile per URL, regardless of
/// whether the signed-in user owns it.
pub fn get_discovered_bttv_pro_badges() -> Vec<String> {
    match DISCOVERED.lock() {
        Ok(mut store) => {
            store.ensure_loaded();
            store.urls.clone()
        }
        Err(_) => Vec::new(),
    }
}

/// The shared list every StreamNook profile lookup adds to (streamnook.app
/// keeps it, since BetterTTV publishes no catalogue).
const SHARED_PRO_BADGES_URL: &str = "https://streamnook.app/api/v1/bttv-pro/badges";
const SHARED_PRO_BADGES_TTL: Duration = Duration::from_secs(600);

static SHARED_PRO_BADGES: Lazy<std::sync::Mutex<Option<(std::time::Instant, Vec<String>)>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

/// The design id in a Pro badge URL (`.../badges/pro/<id>.png|webp`).
fn pro_design_id(url: &str) -> Option<&str> {
    let rest = url.split("/badges/pro/").nth(1)?;
    let id = rest.split('.').next()?;
    (id.len() == 36).then_some(id)
}

/// Design ids from the shared list, reused for a few minutes; empty when
/// streamnook.app cannot be reached (the tab then shows this install's own).
async fn shared_pro_design_ids() -> Vec<String> {
    if let Ok(guard) = SHARED_PRO_BADGES.lock() {
        if let Some((at, ids)) = guard.as_ref() {
            if at.elapsed() < SHARED_PRO_BADGES_TTL {
                return ids.clone();
            }
        }
    }
    let fetched = async {
        let request = crate::services::http::client().get(SHARED_PRO_BADGES_URL).timeout(Duration::from_secs(6));
        let body: serde_json::Value = crate::services::sn_transport::send(request).await.ok()?.json().await.ok()?;
        Some(
            body.get("badges")?
                .as_array()?
                .iter()
                .filter_map(|b| b.get("id")?.as_str().map(str::to_string))
                .collect::<Vec<_>>(),
        )
    }
    .await;
    match fetched {
        Some(ids) => {
            if let Ok(mut guard) = SHARED_PRO_BADGES.lock() {
                *guard = Some((std::time::Instant::now(), ids.clone()));
            }
            ids
        }
        None => Vec::new(),
    }
}

/// Every Pro design this install has seen, then every other one on the shared
/// list, one URL per design. Shared designs use the `.png` form the socket
/// returns, so the tab's "you own this" check (an exact URL match) still holds.
pub async fn all_known_bttv_pro_badges() -> Vec<String> {
    let mut urls = get_discovered_bttv_pro_badges();
    let mut seen: std::collections::HashSet<String> =
        urls.iter().filter_map(|u| pro_design_id(u).map(str::to_string)).collect();
    for id in shared_pro_design_ids().await {
        if seen.insert(id.clone()) {
            urls.push(format!("https://cdn.betterttv.net/badges/pro/{id}.png"));
        }
    }
    urls
}

/// How one socket lookup ended, before the timeout decides what silence means.
#[derive(Debug, PartialEq)]
enum LookupEnd {
    /// BTTV answered for this user (`None`: not Pro).
    Answered(Option<BttvProBadge>),
    /// The socket could not be asked, or closed before answering.
    Failed(&'static str),
}

/// What a lookup tells the caller. Silence after the question went out is how
/// BTTV says "not Pro"; silence before it went out (a connection still opening
/// when the time ran out) says nothing, so it is a failure.
fn lookup_result(ended: Option<LookupEnd>, asked: bool) -> Result<Option<BttvProBadge>, String> {
    match ended {
        Some(LookupEnd::Answered(badge)) => Ok(badge),
        Some(LookupEnd::Failed(why)) => Err(format!("BTTV socket: {why}")),
        None if asked => Ok(None),
        None => Err("BTTV socket: no connection in time".to_string()),
    }
}

async fn lookup_over_socket(user_id: &str) -> Result<Option<BttvProBadge>, String> {
    let asked = std::sync::atomic::AtomicBool::new(false);
    let lookup = async {
        let Ok((ws, _)) = connect_async(BTTV_WS_URL).await else {
            return LookupEnd::Failed("could not connect");
        };
        let (mut write, mut read) = ws.split();

        // join_channel then broadcast_me. broadcast_me makes BTTV reply with the
        // broadcasted user's lookup_user, so the user need not be live or in any
        // particular channel -- we just use their own channel id as the context.
        let channel = format!("twitch:{}", user_id);
        let join = json!({ "name": "join_channel", "data": { "name": channel } });
        let broadcast = json!({
            "name": "broadcast_me",
            "data": { "provider": "twitch", "providerId": user_id, "channel": channel }
        });
        if write.send(Message::text(join.to_string())).await.is_err()
            || write.send(Message::text(broadcast.to_string())).await.is_err()
        {
            return LookupEnd::Failed("could not send the lookup");
        }
        asked.store(true, std::sync::atomic::Ordering::SeqCst);

        while let Some(Ok(msg)) = read.next().await {
            let txt = match msg {
                Message::Text(t) => t,
                Message::Close(_) => break,
                _ => continue, // ping/pong/binary
            };
            let v: Value = match serde_json::from_str(&txt) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("name").and_then(|n| n.as_str()) != Some("lookup_user") {
                continue;
            }
            let data = match v.get("data") {
                Some(d) => d,
                None => continue,
            };
            // Only the reply for the user we asked about counts.
            if data.get("providerId").and_then(|p| p.as_str()) != Some(user_id) {
                continue;
            }
            return LookupEnd::Answered(pro_badge_from(data));
        }
        LookupEnd::Failed("closed before answering")
    };

    // Either the user is Pro (fast reply) or we wait out the timeout (not Pro,
    // when the question went out; a failure when it never did).
    let ended = timeout(Duration::from_millis(LOOKUP_TIMEOUT_MS), lookup).await.ok();
    lookup_result(ended, asked.load(std::sync::atomic::Ordering::SeqCst))
}

/// The Pro badge in a `lookup_user` reply's data, or `None` for a user without Pro.
fn pro_badge_from(data: &Value) -> Option<BttvProBadge> {
    if data.get("pro").and_then(|p| p.as_bool()) != Some(true) {
        return None;
    }
    let badge = data.get("badge");
    let url = badge.and_then(|b| b.get("url")).and_then(|u| u.as_str())?.to_string();
    Some(BttvProBadge {
        url,
        started_at: badge.and_then(|b| b.get("startedAt")).and_then(|s| s.as_str()).map(String::from),
        glow: data.get("glow").and_then(|g| g.as_bool()).unwrap_or(false),
    })
}

/// Resident BTTV Pro lookups (try-lock). Diagnostics for the resource line.
pub fn cache_len() -> Option<usize> {
    Some(LOOKUPS.len())
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    fn pro() -> BttvProBadge {
        BttvProBadge { url: "https://cdn.betterttv.net/badges/pro/x.webp".into(), started_at: None, glow: false }
    }

    #[test]
    fn silence_after_the_question_is_not_pro_and_silence_before_it_is_a_failure() {
        assert_eq!(lookup_result(None, true), Ok(None));
        assert!(lookup_result(None, false).is_err());
    }

    #[test]
    fn an_answer_is_kept_and_a_broken_socket_is_a_failure() {
        assert_eq!(lookup_result(Some(LookupEnd::Answered(Some(pro()))), true), Ok(Some(pro())));
        assert_eq!(lookup_result(Some(LookupEnd::Answered(None)), true), Ok(None));
        assert!(lookup_result(Some(LookupEnd::Failed("could not connect")), false).is_err());
        assert!(lookup_result(Some(LookupEnd::Failed("closed before answering")), true).is_err());
    }

    #[test]
    fn a_reply_reads_pro_and_its_badge() {
        let data = json!({ "providerId": "5", "pro": true, "glow": true, "badge": { "url": "https://cdn.betterttv.net/badges/pro/x.webp", "startedAt": "2024" } });
        let badge = pro_badge_from(&data).unwrap();
        assert!(badge.glow);
        assert_eq!(badge.started_at.as_deref(), Some("2024"));
        assert_eq!(pro_badge_from(&json!({ "pro": false })), None);
        assert_eq!(pro_badge_from(&json!({ "pro": true })), None);
    }

    #[tokio::test]
    async fn an_empty_id_is_not_pro_and_asks_nothing() {
        assert_eq!(resolve_bttv_pro_badge("").await, Ok(None));
        assert_eq!(pro_badge_within("", Duration::from_millis(10)).await, Some(None));
    }

    #[tokio::test]
    async fn a_held_answer_serves_both_doors_and_clear_cache_forgets_it() {
        // A negative answer is an answer: nobody asks the socket again within the TTL.
        LOOKUPS.prime("test-bttv-held", None);
        assert_eq!(resolve_bttv_pro_badge("test-bttv-held").await, Ok(None));
        assert_eq!(pro_badge_within("test-bttv-held", Duration::from_millis(10)).await, Some(None));
        assert!(cache_len().is_some_and(|n| n >= 1));
        clear_cache().await;
        assert_eq!(LOOKUPS.peek("test-bttv-held"), None);
    }
}
