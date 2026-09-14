//! 7TV emotes for YouTube channels.
//!
//! 7TV calls the YouTube platform **GOOGLE**, so the route is
//! `https://7tv.io/v3/users/google/{channel_id}`. VERIFIED against the live API:
//! `/users/youtube/…` answers 400 "invalid platform", `/users/google/…` answers
//! 200 or 404, and the v4 GraphQL `Platform` enum is exactly
//! `TWITCH, DISCORD, GOOGLE, KICK`. Do not "correct" google back to youtube.
//! identifies the channel's active emote set (older payloads inlined the full
//! set; newer ones only carry `emote_set_id`, so the set is fetched from
//! `https://7tv.io/v3/emote-sets/{id}`), and `https://7tv.io/v3/emote-sets/global`
//! is the shared global set. We fetch both, build a per-channel `name -> emote`
//! map, and the YouTube chat parser bakes matching words into emote segments.
//!
//! This mirrors `kick_emotes` deliberately, including its scope decision: BTTV
//! and FFZ have no YouTube support, so this is 7TV-only.
//!
//! The baking has to happen HERE, in Rust, because the frontend renders provider
//! `segments` verbatim — there is no name-to-emote substitution on that side to
//! lean on. YouTube's OWN emoji are a separate thing entirely and already work
//! (`store_channel_emojis` scrapes them from the live_chat page).

use crate::services::emote_service;
use crate::services::emote_service::{Emote, EmoteProvider, EmoteSet};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 7TV's user endpoint is SLOW for large sets — measured 57s for xqc's 2.4MB
/// (1000 emotes), and 71s for the equivalent Twitch document. The previous 6s
/// budget could only ever fetch small channels, so big ones silently fell back
/// to globals. This runs in a background task, so waiting is free.
const SEVENTV_TIMEOUT: Duration = Duration::from_secs(90);

/// How soon to retry after a channel-set fetch failed (vs the full TTL).
const RETRY_AFTER: Duration = Duration::from_secs(30);
const TTL: Duration = Duration::from_secs(600);

#[derive(Clone)]
pub struct YouTubeEmote {
    pub id: String,
    pub url: String,
    pub zero_width: bool,
}

struct ChannelEmotes {
    map: HashMap<String, YouTubeEmote>, // emote name -> emote
    fetched_at: Instant,
}

static STORE: OnceLock<Mutex<HashMap<String, ChannelEmotes>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<String, ChannelEmotes>> {
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The store key for a channel: its UC id.
///
/// NOT the identifier the chat happens to be addressed by. A YouTube chat is
/// usually opened by VIDEO id, and a video id is per-BROADCAST — keying on it
/// meant every new stream from the same channel re-fetched the same emote set
/// (a ~60s, multi-megabyte request), and the same channel opened by `@handle`
/// versus video id occupied two unrelated entries. 7TV emotes belong to the
/// channel, so the channel is what they are filed under.
///
/// Falls back to the identifier when the meta is not resolved yet, which is
/// still internally consistent because every caller resolves the same way.
fn store_key(identifier: &str) -> String {
    super::youtube::channel_meta(identifier)
        .and_then(|m| m.user_id)
        .unwrap_or_else(|| identifier.to_string())
        .to_lowercase()
}

/// The emote for an exact word in this channel's set, if any.
pub fn lookup(identifier: &str, word: &str) -> Option<YouTubeEmote> {
    let s = store().lock().ok()?;
    s.get(&store_key(identifier))?.map.get(word).cloned()
}

/// Whether this channel has any 7TV emotes cached — lets the parser skip the
/// per-word scan entirely for the overwhelmingly common case of a channel whose
/// community does not use 7TV.
pub fn has_emotes(identifier: &str) -> bool {
    let key = store_key(identifier);
    store()
        .lock()
        .map(|s| s.get(&key).is_some_and(|c| !c.map.is_empty()))
        .unwrap_or(false)
}

/// The channel's 7TV emotes as an `EmoteSet` for the frontend emote picker.
///
/// ASYNC, and it waits: the picker fetch fires as chat connects, while the 7TV
/// refresh spawned at connect is still in flight. Answering "no emotes" in that
/// window is not harmless — the FRONTEND caches whatever it gets, per channel,
/// so one early answer left the 7TV tab empty for the rest of the session.
/// `kick_emotes` guards this with `wait_for_resolve`; this is the same guard.
pub async fn channel_emote_set(identifier: &str) -> EmoteSet {
    wait_for_refresh(identifier).await;
    build_set(identifier)
}

/// Wait until this channel's 7TV set has been stored, or give up.
///
/// `refresh` ALWAYS inserts an entry — even an empty map for a channel with no
/// 7TV emotes — so key presence is a real readiness signal that terminates
/// rather than a proxy that can hang forever.
async fn wait_for_refresh(identifier: &str) {
    for _ in 0..40 {
        // Recomputed each tick on purpose: the key comes from the channel meta,
        // and if this runs before the resolve lands, store_key falls back to the
        // identifier — which is NOT what refresh files under. Latching that early
        // value would wait out the whole budget on a key that can never appear.
        let key = store_key(identifier);
        if store().lock().map(|s| s.contains_key(&key)).unwrap_or(false) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    log::debug!(
        "[YouTube] 7TV set for '{}' never arrived; serving empty",
        identifier
    );
}

fn build_set(identifier: &str) -> EmoteSet {
    let key = store_key(identifier);
    let mut set = EmoteSet::new();
    if let Ok(s) = store().lock() {
        if let Some(c) = s.get(&key) {
            set.seven_tv = c
                .map
                .iter()
                .map(|(name, e)| Emote {
                    id: e.id.clone(),
                    name: name.clone(),
                    url: e.url.clone(),
                    provider: EmoteProvider::SevenTV,
                    is_zero_width: Some(e.zero_width),
                    local_url: None,
                    emote_type: None,
                    owner_id: None,
                    owner_name: None,
                    modifier_flags: None,
                    ffz_sub_only: None,
                    width: None,
                })
                .collect();
        }
    }
    set
}

/// Fetch + cache a YouTube channel's 7TV emotes (channel set over globals). Safe
/// to call repeatedly — it self-throttles to the TTL. Spawned when a chat
/// connection resolves and the channel's UC id is known.
pub async fn refresh(identifier: &str, channel_id: &str) {
    // The channel id IS the key (see store_key); `identifier` is only for logs.
    let key = channel_id.to_lowercase();
    {
        if let Ok(s) = store().lock() {
            if let Some(c) = s.get(&key) {
                if c.fetched_at.elapsed() < TTL {
                    return;
                }
            }
        }
    }

    let client = reqwest::Client::new();
    let mut map: HashMap<String, YouTubeEmote> = HashMap::new();
    // Globals first so the channel set overrides on name collisions.
    fetch_into(&client, "https://7tv.io/v3/emote-sets/global", "/emotes", &mut map).await;
    let chan_url = format!("https://7tv.io/v3/users/google/{channel_id}");
    // Whether the CHANNEL half resolved. A failed fetch and a channel with no
    // 7TV set both leave the map holding globals only, but they need opposite
    // treatment: the first should be retried soon, the second is the final answer.
    let mut channel_ok = false;
    let fetched = fetch_user(&client, &chan_url).await;
    if let Ok(none_or_user) = &fetched {
        // Reached 7TV and got a definitive answer, even if that answer is
        // "this channel has no set".
        channel_ok = true;
        let _ = none_or_user;
    }
    if let Ok(Some(user)) = fetched {
        if user
            .pointer("/emote_set/emotes")
            .and_then(|e| e.as_array())
            .is_some()
        {
            // Inline set (pre-change payload).
            collect_emotes(&user, "/emote_set/emotes", &mut map);
        } else if let Some(set_id) = emote_service::seventv_active_set_id(&user) {
            // Post-change payload: fetch the active set by id.
            let set_url = format!("https://7tv.io/v3/emote-sets/{set_id}");
            fetch_into(&client, &set_url, "/emotes", &mut map).await;
        }
    }

    // A channel-set fetch that failed leaves ONLY the globals in `map`. Writing
    // that would clobber a good set with a worse one — observed live: xqc's 999
    // emotes were replaced by the 45 globals when one of three concurrent
    // refreshes lost its channel fetch. Same discipline `store_native` already
    // uses: never let a partial result overwrite a fuller one.
    if let Ok(s) = store().lock() {
        if s.get(&key).is_some_and(|c| c.map.len() > map.len()) {
            log::debug!(
                "[YouTube] 7TV refresh for {} returned {} vs {} cached; keeping the cached set",
                key,
                map.len(),
                s.get(&key).map(|c| c.map.len()).unwrap_or(0)
            );
            return;
        }
    }
    let count = map.len();
    // A partial result is still worth serving (globals beat nothing), but it must
    // not sit for the whole TTL: backdating `fetched_at` makes the next call retry
    // in RETRY_AFTER instead of ten minutes. Seen live on xqc, whose 999-emote set
    // failed to fetch three times in a row and would otherwise have left that chat
    // with 45 emotes until the TTL lapsed.
    let stamp = if channel_ok {
        Instant::now()
    } else {
        log::warn!(
            "[YouTube] 7TV channel set for {} did not resolve; serving {} global(s) and retrying shortly",
            key,
            count
        );
        Instant::now()
            .checked_sub(TTL - RETRY_AFTER)
            .unwrap_or_else(Instant::now)
    };
    if let Ok(mut s) = store().lock() {
        s.insert(
            key.clone(),
            ChannelEmotes {
                map,
                fetched_at: stamp,
            },
        );
    }
    log::info!("[YouTube] 7TV emotes for channel {channel_id} (as {identifier}): {count} loaded");
}

/// Fetch a 7TV USER document, keeping the distinction `fetch_json` throws away.
///
/// A channel that simply is not on 7TV answers 404, which is a FINAL answer and
/// must be cached like any other; a timeout or a 5xx is transient and should be
/// retried shortly. Collapsing both to `None` meant either caching "no emotes"
/// for ten minutes after a blip, or retrying forever for a channel that will
/// never have a set.
async fn fetch_user(client: &reqwest::Client, url: &str) -> Result<Option<Value>, ()> {
    let resp = match client.get(url).timeout(SEVENTV_TIMEOUT).send().await {
        Ok(r) => r,
        Err(_) => return Err(()),
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None); // not on 7TV: final
    }
    if !resp.status().is_success() {
        return Err(()); // 5xx / rate limit: transient
    }
    resp.json().await.map(Some).map_err(|_| ())
}

async fn fetch_json(client: &reqwest::Client, url: &str) -> Option<Value> {
    let resp = client
        .get(url)
        .timeout(SEVENTV_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

fn collect_emotes(v: &Value, pointer: &str, map: &mut HashMap<String, YouTubeEmote>) {
    let Some(emotes) = v.pointer(pointer).and_then(|e| e.as_array()) else {
        return;
    };
    for e in emotes {
        let (Some(name), Some(id)) = (
            e.get("name").and_then(|x| x.as_str()),
            e.get("id").and_then(|x| x.as_str()),
        ) else {
            continue;
        };
        // Zero-width flag (bit 256) lives on the emote data; fall back to the
        // active-emote flags.
        let flags = e
            .pointer("/data/flags")
            .and_then(|x| x.as_i64())
            .or_else(|| e.get("flags").and_then(|x| x.as_i64()))
            .unwrap_or(0);
        map.insert(
            name.to_string(),
            YouTubeEmote {
                id: id.to_string(),
                url: format!("https://cdn.7tv.app/emote/{id}/2x.webp"),
                zero_width: (flags & 256) == 256,
            },
        );
    }
}

async fn fetch_into(
    client: &reqwest::Client,
    url: &str,
    pointer: &str,
    map: &mut HashMap<String, YouTubeEmote>,
) {
    if let Some(v) = fetch_json(client, url).await {
        collect_emotes(&v, pointer, map);
    }
}

/// Drop a channel's cached 7TV set so the next `refresh` re-fetches instead of
/// returning early on the TTL. Used when 7TV pushes an emote-set update.
pub fn invalidate(identifier: &str) {
    let key = store_key(identifier);
    if let Ok(mut s) = store().lock() {
        s.remove(&key);
    }
}
