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
// users aren't re-queried on every profile open.

use futures_util::{SinkExt, StreamExt};
use lru::LruCache;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BttvProBadge {
    pub url: String,
    pub started_at: Option<String>,
    pub glow: bool,
}

struct CacheEntry {
    badge: Option<BttvProBadge>,
    fetched_at: Instant,
}

static CACHE: Lazy<Mutex<LruCache<String, CacheEntry>>> =
    Lazy::new(|| Mutex::new(LruCache::new(NonZeroUsize::new(2000).unwrap())));

/// Resolve a user's BTTV Pro loyalty badge, or `None` if they don't have Pro
/// (or the socket didn't answer in time). Cached with a TTL; negative answers
/// are cached too so non-Pro users aren't re-queried on every profile open.
pub async fn resolve_bttv_pro_badge(user_id: &str) -> Option<BttvProBadge> {
    if user_id.is_empty() {
        return None;
    }

    // Fast path: fresh cache hit (positive or negative).
    let cached_hit = {
        let mut cache = CACHE.lock().await;
        match cache.get(user_id) {
            Some(entry) if entry.fetched_at.elapsed() < CACHE_TTL => Some(entry.badge.clone()),
            _ => None,
        }
    };

    let badge = match cached_hit {
        Some(b) => b,
        None => {
            let b = lookup_over_socket(user_id).await;
            let mut cache = CACHE.lock().await;
            cache.put(
                user_id.to_string(),
                CacheEntry {
                    badge: b.clone(),
                    fetched_at: Instant::now(),
                },
            );
            b
        }
    };

    // Accumulate every distinct Pro badge image we ever see, from ANY user, so
    // the BetterTTV tab can show the full set of discovered loyalty designs (not
    // just the signed-in user's own). Persisted so it grows across sessions.
    if let Some(ref b) = badge {
        record_discovered_badge(&b.url);
    }

    badge
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

async fn lookup_over_socket(user_id: &str) -> Option<BttvProBadge> {
    let lookup = async {
        let (ws, _) = connect_async(BTTV_WS_URL).await.ok()?;
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
        write.send(Message::text(join.to_string())).await.ok()?;
        write
            .send(Message::text(broadcast.to_string()))
            .await
            .ok()?;

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
            if data.get("pro").and_then(|p| p.as_bool()) != Some(true) {
                return None;
            }
            let badge = data.get("badge");
            let url = badge
                .and_then(|b| b.get("url"))
                .and_then(|u| u.as_str())?
                .to_string();
            return Some(BttvProBadge {
                url,
                started_at: badge
                    .and_then(|b| b.get("startedAt"))
                    .and_then(|s| s.as_str())
                    .map(String::from),
                glow: data.get("glow").and_then(|g| g.as_bool()).unwrap_or(false),
            });
        }
        None
    };

    // Either the user is Pro (fast reply) or we wait out the timeout (not Pro /
    // socket unavailable). A timeout/connection failure is a clean "no badge".
    timeout(Duration::from_millis(LOOKUP_TIMEOUT_MS), lookup)
        .await
        .unwrap_or(None)
}

/// Resident BTTV Pro lookups (try-lock). Diagnostics for the resource line.
pub fn cache_len() -> Option<usize> {
    CACHE.try_lock().ok().map(|c| c.len())
}
