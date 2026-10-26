//! Channel avatars for stream cards, across every platform, shared by every
//! window and kept across restarts.
//!
//! A stream row does not reliably carry its channel's avatar, and where it comes
//! from differs per platform: Twitch's takes a Helix `users` lookup (100 ids a
//! call), YouTube's and Kick's their own per-channel resolvers. A page asks for
//! the rows it is drawing and gets whatever is cached at once. The rest are
//! looked up here, every platform at once, and broadcast to every window as
//! `channel-avatars` the moment each platform answers. A page that moved on
//! before an answer arrived loses nothing: the answer is cached, and the next
//! page to ask gets it immediately.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use crate::rt::AppHandle;
use tauri::Emitter;

/// Avatars change rarely; one a few days stale costs nothing next to fetching
/// every face again on each launch.
const TTL_SECS: u64 = 7 * 86_400;
/// A channel that resolved to no avatar is not asked about again for this long.
const MISS_TTL: Duration = Duration::from_secs(60 * 60);
/// Non-Twitch channels looked up per platform per request. Each is a real
/// request upstream, so the rest wait for a later request as more cards render.
const PER_REQUEST: usize = 24;
const EVENT: &str = "channel-avatars";
const FILE: &str = "channel_avatars.json";
const SAVE_DELAY: Duration = Duration::from_secs(2);

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    url: String,
    /// Unix seconds.
    at: u64,
}

#[derive(Default)]
struct Cache {
    loaded: bool,
    known: HashMap<String, Entry>,
    misses: HashMap<String, Instant>,
    /// Being looked up now, so two pages asking at once cost one lookup.
    in_flight: HashSet<String>,
    save_pending: bool,
}

static CACHE: Lazy<Mutex<Cache>> = Lazy::new(|| Mutex::new(Cache::default()));
/// Woken after every lookup resolves (found or not), for `wait_for`.
static LANDED: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// One channel a page wants a face for: its platform and the id that
/// platform's avatar lookup takes (Twitch and YouTube ids, Kick slugs).
#[derive(Deserialize)]
pub struct AvatarRequest {
    pub provider: String,
    pub id: String,
}

/// An entry from the page's old avatar store, handed over once.
#[derive(Deserialize)]
pub struct LegacyEntry {
    url: String,
    /// Unix milliseconds.
    t: u64,
}

/// The cache key, in each platform's own id space so a Kick slug can never
/// shadow a Twitch id. The page keys its rows the same way.
fn key(provider: &str, id: &str) -> String {
    format!("{}:{}", provider, id.to_lowercase())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn usable(url: &str) -> bool {
    url.starts_with("https://") && url.len() < 2048
}

fn path() -> Option<std::path::PathBuf> {
    crate::services::twitch_service::get_app_data_dir()
        .ok()
        .map(|d| d.join(FILE))
}

fn ensure_loaded(cache: &mut Cache) {
    if cache.loaded {
        return;
    }
    cache.loaded = true;
    let Some(raw) = path().and_then(|p| std::fs::read(p).ok()) else {
        return;
    };
    let Ok(stored) = serde_json::from_slice::<HashMap<String, Entry>>(&raw) else {
        return;
    };
    let now = unix_now();
    cache.known = stored
        .into_iter()
        .filter(|(_, e)| now.saturating_sub(e.at) < TTL_SECS && usable(&e.url))
        .collect();
}

/// Write the cache a moment after the last change, once per burst of answers.
fn schedule_save(cache: &mut Cache) {
    if cache.save_pending {
        return;
    }
    cache.save_pending = true;
    tauri::async_runtime::spawn(async {
        tokio::time::sleep(SAVE_DELAY).await;
        let json = {
            let Ok(mut cache) = CACHE.lock() else { return };
            cache.save_pending = false;
            serde_json::to_vec(&cache.known)
        };
        if let (Some(p), Ok(json)) = (path(), json) {
            if let Err(e) = std::fs::write(p, json) {
                log::debug!("[Avatars] could not save the cache: {}", e);
            }
        }
    });
}

/// Take over the page's old avatar store, keeping what is still fresh.
pub fn import_legacy(entries: HashMap<String, LegacyEntry>) {
    let Ok(mut cache) = CACHE.lock() else { return };
    ensure_loaded(&mut cache);
    let now = unix_now();
    let mut added = false;
    for (k, e) in entries {
        let at = e.t / 1000;
        if !usable(&e.url) || now.saturating_sub(at) >= TTL_SECS || !k.contains(':') {
            continue;
        }
        cache.known.entry(k.to_lowercase()).or_insert_with(|| {
            added = true;
            Entry { url: e.url, at }
        });
    }
    if added {
        schedule_save(&mut cache);
    }
}

/// Avatars for `requests`, keyed as `provider:id`: whatever is cached, now.
/// The rest are looked up in the background, each platform at once, and
/// broadcast to every window as `channel-avatars`.
pub fn request(app: &AppHandle, requests: Vec<AvatarRequest>) -> HashMap<String, String> {
    let mut hits = HashMap::new();
    let mut wanted: HashMap<String, Vec<String>> = HashMap::new();
    {
        let Ok(mut cache) = CACHE.lock() else {
            return hits;
        };
        ensure_loaded(&mut cache);
        let now = unix_now();
        for r in requests {
            if r.id.is_empty() {
                continue;
            }
            let k = key(&r.provider, &r.id);
            if let Some(e) = cache.known.get(&k) {
                if now.saturating_sub(e.at) < TTL_SECS {
                    hits.insert(k, e.url.clone());
                    continue;
                }
            }
            if cache.in_flight.contains(&k) || cache.misses.get(&k).is_some_and(|t| t.elapsed() < MISS_TTL) {
                continue;
            }
            let ids = wanted.entry(r.provider.clone()).or_default();
            // Twitch answers 100 per call, so it takes the whole list.
            if r.provider != "twitch" && ids.len() >= PER_REQUEST {
                continue;
            }
            cache.in_flight.insert(k);
            ids.push(r.id);
        }
    }
    for (provider, ids) in wanted {
        let app = app.clone();
        tauri::async_runtime::spawn(async move { look_up(&app, &provider, ids).await });
    }
    hits
}

/// Avatars for `requests` like `request`, but a lookup that lands within
/// `timeout` is in the answer too (a notification about to go out waits a
/// moment for a face rather than going without). Returns as soon as nothing
/// it asked for is still being looked up.
pub async fn wait_for(app: &AppHandle, requests: Vec<AvatarRequest>, timeout: Duration) -> HashMap<String, String> {
    let keys: Vec<String> = requests.iter().filter(|r| !r.id.is_empty()).map(|r| key(&r.provider, &r.id)).collect();
    let mut found = request(app, requests);
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        // Registered before the cache is read again, so a lookup that lands
        // in between still wakes this.
        let landed = LANDED.notified();
        tokio::pin!(landed);
        landed.as_mut().enable();
        if !settle(&keys, &mut found) {
            return found;
        }
        if tokio::time::timeout_at(deadline, landed).await.is_err() {
            return found;
        }
    }
}

/// Copies what the cache holds now for `keys` into `found`. Whether any of
/// them is still being looked up.
fn settle(keys: &[String], found: &mut HashMap<String, String>) -> bool {
    let Ok(cache) = CACHE.lock() else { return false };
    let now = unix_now();
    let mut waiting = false;
    for k in keys {
        if found.contains_key(k) {
            continue;
        }
        match cache.known.get(k) {
            Some(e) if now.saturating_sub(e.at) < TTL_SECS => {
                found.insert(k.clone(), e.url.clone());
            }
            _ => waiting |= cache.in_flight.contains(k),
        }
    }
    waiting
}

async fn look_up(app: &AppHandle, provider: &str, ids: Vec<String>) {
    let found: HashMap<String, String> = match provider {
        "twitch" => crate::services::twitch_service::TwitchService::users_by_ids(&ids)
            .await
            .into_iter()
            .filter_map(|(id, (_, _, avatar))| avatar.map(|a| (id, a)))
            .collect(),
        "youtube" => crate::services::providers::youtube_media::channel_avatars(&ids).await,
        // Kick addresses channels by SLUG, which is what its lookup takes.
        "kick" => crate::services::providers::kick::channel_avatars(&ids).await,
        _ => HashMap::new(),
    };
    let found: HashMap<String, String> = found
        .into_iter()
        .filter(|(_, url)| usable(url))
        .map(|(id, url)| (id.to_lowercase(), url))
        .collect();

    let mut landed = HashMap::new();
    {
        let Ok(mut cache) = CACHE.lock() else { return };
        let now = unix_now();
        for id in &ids {
            let k = key(provider, id);
            cache.in_flight.remove(&k);
            match found.get(&id.to_lowercase()) {
                Some(url) => {
                    cache.misses.remove(&k);
                    cache.known.insert(k.clone(), Entry { url: url.clone(), at: now });
                    landed.insert(k, url.clone());
                }
                None => {
                    cache.misses.insert(k, Instant::now());
                }
            }
        }
        if !landed.is_empty() {
            schedule_save(&mut cache);
        }
    }
    LANDED.notify_waiters();
    if !landed.is_empty() {
        let _ = app.emit(EVENT, &landed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_per_platform_and_case_blind() {
        assert_eq!(key("youtube", "UCabc"), "youtube:ucabc");
        assert_ne!(key("kick", "123"), key("twitch", "123"));
    }

    #[test]
    fn settle_takes_cached_faces_and_waits_only_for_lookups_running() {
        let (hit, busy, gone) = ("twitch:settle-hit", "twitch:settle-busy", "twitch:settle-gone");
        {
            let mut cache = CACHE.lock().unwrap();
            cache.known.insert(hit.into(), Entry { url: "https://static-cdn.jtvnw.net/h.png".into(), at: unix_now() });
            cache.in_flight.insert(busy.into());
        }
        let keys = [hit.to_string(), busy.to_string(), gone.to_string()];
        let mut found = HashMap::new();
        assert!(settle(&keys, &mut found), "a lookup is still running");
        assert_eq!(found.get(hit).map(String::as_str), Some("https://static-cdn.jtvnw.net/h.png"));
        CACHE.lock().unwrap().in_flight.remove(busy);
        assert!(!settle(&keys, &mut found), "nothing left to wait for: a miss never holds a notification");
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn only_https_urls_are_kept() {
        assert!(usable("https://static-cdn.jtvnw.net/a.png"));
        assert!(!usable("http://example.com/a.png"));
        assert!(!usable("javascript:alert(1)"));
    }
}
