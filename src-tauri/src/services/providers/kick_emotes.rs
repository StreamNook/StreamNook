//! 7TV emotes for Kick channels.
//!
//! 7TV supports Kick natively: `https://7tv.io/v3/users/kick/{kick_user_id}`
//! identifies the channel's active emote set (older payloads inlined the full
//! set; newer ones only carry `emote_set_id`, so the set is fetched from
//! `https://7tv.io/v3/emote-sets/{id}`), and `https://7tv.io/v3/emote-sets/global`
//! is the shared global set. We fetch both, build a per-slug `name -> emote`
//! map, and the Kick chat parser bakes matching words into emote segments —
//! parity with the Twitch 7TV path.
//!
//! (BTTV/FFZ don't support Kick, so this is 7TV-only by design.)

use crate::services::emote_service;
use crate::services::emote_service::{Emote, EmoteProvider, EmoteSet};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct KickEmote {
    pub id: String,
    pub url: String,
    pub zero_width: bool,
}

struct ChannelEmotes {
    map: HashMap<String, KickEmote>, // emote name -> emote
    fetched_at: Instant,
}

static STORE: OnceLock<Mutex<HashMap<String, ChannelEmotes>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<String, ChannelEmotes>> {
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// One of Kick's OWN emotes (a channel sub emote, a Global emote, or an Emoji),
/// as reported by the resolver webview. `set` is the group label Kick gives the
/// set (the channel name for sub emotes, "Global", "Emojis") and drives the
/// picker's section headers. The image is `files.kick.com/emotes/{id}/fullsize`.
///
/// Doubles as the wire type the `report_kick_chatroom` command deserializes, so
/// the JS reports `{ id, name, set }` directly.
#[derive(Clone, serde::Deserialize)]
pub struct KickNativeEmoteEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub set: String,
}

// Per-slug native emotes, captured from kick.com/emotes/{slug} during resolve.
// Unlike the 7TV set above, this endpoint is Cloudflare-gated, so the resolver
// webview pushes the data in (store_native) rather than us fetching it directly.
static NATIVE: OnceLock<Mutex<HashMap<String, Vec<KickNativeEmoteEntry>>>> = OnceLock::new();

fn native_store() -> &'static Mutex<HashMap<String, Vec<KickNativeEmoteEntry>>> {
    NATIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a channel's native Kick emotes (reported by the resolver webview).
/// Replaces any prior set for the slug. Skips an empty report so a later failed
/// re-resolve can't wipe a good set.
pub fn store_native(slug: &str, emotes: Vec<KickNativeEmoteEntry>) {
    if emotes.is_empty() {
        return;
    }
    if let Ok(mut s) = native_store().lock() {
        s.insert(slug.to_lowercase(), emotes);
    }
}

/// Rewrite bare Kick emote names in an outgoing message into the
/// `[emote:{id}:{name}]` token Kick's own client sends.
///
/// Kick only renders a native emote when the message carries that token; a bare
/// name arrives as plain text for every other viewer. The emote picker inserts
/// bare names (a raw token in the composer would be unreadable), so the
/// substitution happens here, on the way out.
///
/// 7TV names are deliberately left alone: they are matched by NAME on the
/// receiving side, so tokenizing them would break them.
pub fn inline_native_emotes(slug: &str, text: &str) -> String {
    let Ok(store) = native_store().lock() else {
        return text.to_string();
    };
    let Some(emotes) = store.get(&slug.to_lowercase()) else {
        return text.to_string();
    };
    if emotes.is_empty() {
        return text.to_string();
    }

    // Split on whitespace but keep it, so the message reads back exactly as typed.
    let mut out = String::with_capacity(text.len());
    for part in text.split_inclusive(char::is_whitespace) {
        let trailing_len = part.len() - part.trim_end().len();
        let (word, tail) = part.split_at(part.len() - trailing_len);
        // Never touch a word that already looks tokenized.
        if word.is_empty() || word.contains('[') || word.contains(']') {
            out.push_str(part);
            continue;
        }
        // Kick emote names are case-sensitive, so only an exact match qualifies.
        match emotes.iter().find(|e| e.name == word) {
            Some(e) => out.push_str(&format!("[emote:{}:{}]", e.id, e.name)),
            None => out.push_str(word),
        }
        out.push_str(tail);
    }
    out
}

// Re-fetch a channel's 7TV set at most this often (it changes rarely).
/// 7TV's user endpoint is SLOW for large sets — measured 57s for xqc's 2.4MB
/// (1000 emotes), and 71s for the equivalent Twitch document. The previous 6s
/// budget could only ever fetch small channels, so big ones silently fell back
/// to globals. This runs in a background task, so waiting is free.
const SEVENTV_TIMEOUT: Duration = Duration::from_secs(90);

/// How soon to retry after a channel-set fetch failed (vs the full TTL).
const RETRY_AFTER: Duration = Duration::from_secs(30);
const TTL: Duration = Duration::from_secs(10 * 60);

/// Look up a single word against a channel's 7TV emotes. Cheap (one uncontended
/// lock); the Kick parser calls it per word.
pub fn lookup(slug: &str, word: &str) -> Option<KickEmote> {
    let s = store().lock().ok()?;
    s.get(&slug.to_lowercase())?.map.get(word).cloned()
}

/// The channel's emotes (Kick native sets + 7TV) as an `EmoteSet` for the
/// frontend emote picker — parity with Twitch's `fetch_channel_emotes`. Waits for
/// the channel resolve to populate its caches (see `wait_for_resolve`), then fills
/// the 7TV slot (channel set + globals) and the Kick slot (native sets).
pub async fn channel_emote_set(slug: &str) -> EmoteSet {
    // A picker/add emote fetch can fire right after a Kick channel is added,
    // racing ahead of the multi-second, Cloudflare-clearing resolve that populates
    // the meta + native-emote caches. Without this wait the fetch reads empty
    // caches and the frontend caches an EMPTY set for good (the "second Kick
    // channel shows 0 emotes" bug). channel_meta appears the moment the resolver
    // reports back, and the native emotes are stored just before it, so waiting on
    // meta guarantees both are ready.
    let meta = wait_for_resolve(slug).await;
    if let Some(uid) = meta.as_ref().and_then(|m| m.user_id) {
        refresh(slug, uid).await;
    }
    let display = meta.and_then(|m| m.username);

    let mut set = EmoteSet::new();
    if let Ok(s) = store().lock() {
        if let Some(c) = s.get(&slug.to_lowercase()) {
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
    // Kick's own native emotes (channel sub set + Global + Emojis). The set label
    // rides along in `emote_type` so the picker groups them under section headers.
    if let Ok(s) = native_store().lock() {
        if let Some(list) = s.get(&slug.to_lowercase()) {
            set.kick = list
                .iter()
                .map(|e| Emote {
                    id: e.id.clone(),
                    name: e.name.clone(),
                    url: format!("https://files.kick.com/emotes/{}/fullsize", e.id),
                    provider: EmoteProvider::Kick,
                    is_zero_width: Some(false),
                    local_url: None,
                    emote_type: Some(kick_set_label(&e.set, slug, display.as_deref())),
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

/// Wait for a Kick channel to finish resolving before building its emote set, so a
/// picker/add fetch issued right after the channel is added doesn't read empty
/// caches and cache an empty set. Async-sleeps, never blocks a thread.
///
/// Two phases: first wait for the channel chrome (`channel_meta`, ~10s cap) — that
/// is the moment the resolve landed — then, since native emotes are reported on a
/// separate, slightly later round-trip, give them a brief window (~4s) to arrive.
/// Real channels always carry the Global + Emoji sets, so phase 2 normally returns
/// the instant they land.
async fn wait_for_resolve(slug: &str) -> Option<super::kick::KickChannelMeta> {
    let mut meta = None;
    for _ in 0..50 {
        if let Some(m) = super::kick::channel_meta(slug) {
            meta = Some(m);
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    if meta.is_some() {
        for _ in 0..20 {
            if native_present(slug) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    meta
}

/// Whether the channel's native emotes have been stored (the separate, post-chrome
/// resolver report has landed).
fn native_present(slug: &str) -> bool {
    native_store()
        .lock()
        .map(|s| s.contains_key(&slug.to_lowercase()))
        .unwrap_or(false)
}

/// The picker section header for a Kick emote set. Kick names the Global/Emoji
/// sets ("Global"/"Emojis") but gives the channel set only its numeric channel id,
/// so swap a numeric (or empty) label for the channel's display name.
fn kick_set_label(raw: &str, slug: &str, display: Option<&str>) -> String {
    if raw.is_empty() || raw.chars().all(|c| c.is_ascii_digit()) {
        return display
            .map(String::from)
            .unwrap_or_else(|| if raw.is_empty() { slug.to_string() } else { raw.to_string() });
    }
    raw.to_string()
}

/// Fetch + cache a Kick channel's 7TV emotes (channel set over globals). Safe to
/// call repeatedly — it self-throttles to the TTL. Spawned during channel resolve
/// once the numeric Kick `user_id` is known.
pub async fn refresh(slug: &str, user_id: u64) {
    let slug = slug.to_lowercase();
    {
        if let Ok(s) = store().lock() {
            if let Some(c) = s.get(&slug) {
                if c.fetched_at.elapsed() < TTL {
                    return;
                }
            }
        }
    }

    let client = reqwest::Client::new();
    let mut map: HashMap<String, KickEmote> = HashMap::new();
    // Globals first so the channel set overrides on name collisions.
    fetch_into(&client, "https://7tv.io/v3/emote-sets/global", "/emotes", &mut map).await;
    let chan_url = format!("https://7tv.io/v3/users/kick/{user_id}");
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
        if s.get(&slug).is_some_and(|c| c.map.len() > map.len()) {
            log::debug!(
                "[Kick] 7TV refresh for {} returned {} vs {} cached; keeping the cached set",
                slug,
                map.len(),
                s.get(&slug).map(|c| c.map.len()).unwrap_or(0)
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
            "[Kick] 7TV channel set for {} did not resolve; serving {} global(s) and retrying shortly",
            slug,
            count
        );
        Instant::now()
            .checked_sub(TTL - RETRY_AFTER)
            .unwrap_or_else(Instant::now)
    };
    if let Ok(mut s) = store().lock() {
        s.insert(
            slug.clone(),
            ChannelEmotes {
                map,
                fetched_at: stamp,
            },
        );
    }
    log::info!("[Kick] 7TV emotes for {slug} (user {user_id}): {count} loaded");
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

fn collect_emotes(v: &Value, pointer: &str, map: &mut HashMap<String, KickEmote>) {
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
            KickEmote {
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
    map: &mut HashMap<String, KickEmote>,
) {
    if let Some(v) = fetch_json(client, url).await {
        collect_emotes(&v, pointer, map);
    }
}

/// Fetch a channel's native Kick emotes over plain HTTP.
///
/// `kick.com/emotes/{slug}` was long treated as reachable only from a browser
/// page context, but the edge is keying on Chrome's client hints rather than on
/// being a real browser — send those and it answers normally. Returns None on
/// any failure so the caller can fall back to the webview resolver.
pub async fn fetch_native_via_http(slug: &str) -> Option<Vec<KickNativeEmoteEntry>> {
    let url = format!("https://kick.com/emotes/{}", slug);
    let resp = crate::services::providers::kick::browser_get(&url, slug).await?;
    let sets: Value = resp.json().await.ok()?;
    let arr = sets.as_array()?.clone();

    let mut out = Vec::new();
    for set in arr {
        let label = set
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Emotes".to_string());
        if let Some(emotes) = set.get("emotes").and_then(|v| v.as_array()) {
            for e in emotes {
                let (Some(id), Some(name)) = (e.get("id"), e.get("name").and_then(|v| v.as_str()))
                else {
                    continue;
                };
                let id = id
                    .as_str()
                    .map(|s| s.to_string())
                    .or_else(|| id.as_u64().map(|n| n.to_string()));
                if let Some(id) = id {
                    out.push(KickNativeEmoteEntry {
                        id,
                        name: name.to_string(),
                        set: label.clone(),
                    });
                }
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(slug: &str) {
        store_native(
            slug,
            vec![
                KickNativeEmoteEntry {
                    id: "37225".into(),
                    name: "KEKW".into(),
                    set: "Global".into(),
                },
                KickNativeEmoteEntry {
                    id: "42".into(),
                    name: "kekw".into(),
                    set: "Global".into(),
                },
            ],
        );
    }

    #[test]
    fn tokenizes_exact_native_names_only() {
        seed("chan_exact");
        // Exact match is tokenized; the differently-cased entry keeps its own id,
        // proving the lookup is case-sensitive rather than first-wins.
        assert_eq!(
            inline_native_emotes("chan_exact", "KEKW"),
            "[emote:37225:KEKW]"
        );
        assert_eq!(inline_native_emotes("chan_exact", "kekw"), "[emote:42:kekw]");
        // A name that only differs by case from both entries is left alone.
        assert_eq!(inline_native_emotes("chan_exact", "KeKw"), "KeKw");
        // Substrings inside a larger word must not be rewritten.
        assert_eq!(inline_native_emotes("chan_exact", "KEKWow"), "KEKWow");
    }

    #[test]
    fn preserves_surrounding_text_and_whitespace() {
        seed("chan_ws");
        assert_eq!(
            inline_native_emotes("chan_ws", "hello  KEKW\tworld\n"),
            "hello  [emote:37225:KEKW]\tworld\n"
        );
    }

    #[test]
    fn leaves_already_tokenized_and_unknown_channels_untouched() {
        seed("chan_tok");
        // An existing token must not be re-wrapped.
        assert_eq!(
            inline_native_emotes("chan_tok", "[emote:37225:KEKW]"),
            "[emote:37225:KEKW]"
        );
        // No stored set for the slug: pass the text through verbatim.
        assert_eq!(inline_native_emotes("never_resolved", "KEKW"), "KEKW");
    }
}

/// Drop a channel's cached 7TV set so the next `refresh` re-fetches instead of
/// returning early on the TTL. Used when 7TV pushes an emote-set update.
pub fn invalidate(slug: &str) {
    if let Ok(mut s) = store().lock() {
        s.remove(&slug.to_lowercase());
    }
}
