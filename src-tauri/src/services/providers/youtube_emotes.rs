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

use super::table_repair::{self, HeldRows, Install, Retrying, Standing};
use crate::models::chat_layout::ChatMessage;
use crate::services::emote_service;
use crate::services::emote_service::{Emote, EmoteProvider, EmoteSet};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// 7TV's user endpoint is SLOW for large sets — measured 57s for xqc's 2.4MB
/// (1000 emotes), and 71s for the equivalent Twitch document. The previous 6s
/// budget could only ever fetch small channels, so big ones silently fell back
/// to globals. This runs in a background task, so waiting is free.
const SEVENTV_TIMEOUT: Duration = Duration::from_secs(90);

/// A partial table is fetched again by its retry loop (`table_repair::DELAYS`),
/// not by this.
const TTL: Duration = Duration::from_secs(600);

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct YouTubeEmote {
    pub id: String,
    pub url: String,
    pub zero_width: bool,
    /// The 7TV uploader, for the hover credit. Defaulted: stored maps predate it.
    #[serde(default)]
    pub owner_name: Option<String>,
    /// The emote's own name when the set shows it under an alias.
    #[serde(default)]
    pub original_name: Option<String>,
    /// The uploader's 7TV user id, for a link to their profile.
    #[serde(default)]
    pub owner_ref: Option<String>,
}

struct ChannelEmotes {
    map: HashMap<String, YouTubeEmote>, // emote name -> emote
    fetched_at: Instant,
    standing: Standing,
}

static STORE: OnceLock<Mutex<HashMap<String, ChannelEmotes>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<String, ChannelEmotes>> {
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Channel (store key) -> the chats (by identifier) showing it, so a retry loop
/// knows when to stop. One channel can be open by video id and by handle.
static IN_USE: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();
/// One retry loop per channel while its 7TV table is partial.
static RETRYING: Retrying = Retrying::new();
/// Rows parsed before the channel's whole 7TV table landed.
static HELD: HeldRows = HeldRows::new();

fn in_use_map() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    IN_USE.get_or_init(Default::default)
}

fn in_use(key: &str) -> bool {
    in_use_map().lock().map(|m| m.get(key).is_some_and(|ids| !ids.is_empty())).unwrap_or(false)
}

/// The chat opened as `identifier` was left: a channel no chat shows any more
/// stops its retries and lets its held rows go.
pub fn release(identifier: &str) {
    let identifier = identifier.to_lowercase();
    let mut gone = Vec::new();
    if let Ok(mut m) = in_use_map().lock() {
        m.retain(|key, ids| {
            ids.remove(&identifier);
            if ids.is_empty() {
                gone.push(key.clone());
                return false;
            }
            true
        });
    }
    for key in gone {
        HELD.drop_key(&key);
    }
}

/// Keeps a published row for a re-bake when its channel's 7TV table is not
/// whole yet (still loading, partial, or only the saved copy), so the row's
/// 7TV words are patched in once the whole table lands.
pub fn hold_row(msg: &ChatMessage) {
    let key = store_key(&super::key::parse_key(&msg.channel).channel);
    let whole = store()
        .lock()
        .map(|s| s.get(&key).is_some_and(|c| c.standing == Standing::Whole))
        .unwrap_or(true);
    if !whole {
        HELD.hold(&key, msg);
    }
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

/// Run `f` over a channel's 7TV emotes (name -> emote) without copying them.
/// `None` until the channel's 7TV refresh has stored an entry.
pub fn with_seventv<R>(identifier: &str, f: impl FnOnce(&HashMap<String, YouTubeEmote>) -> R) -> Option<R> {
    let key = store_key(identifier);
    let s = store().lock().ok()?;
    s.get(&key).map(|c| f(&c.map))
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
                    owner_name: e.owner_name.clone(),
                    original_name: e.original_name.clone(),
                    owner_ref: e.owner_ref.clone(),
                    modifier_flags: None,
                    ffz_sub_only: None,
                    animated: None,
                    width: None,
                    height: None,
                })
                .collect();
        }
    }
    set
}

//// Fetch + cache a YouTube channel's 7TV emotes (channel set over globals). Safe
/// to call repeatedly — it self-throttles to the TTL. Spawned when a chat
/// connection resolves and the channel's UC id is known.
pub async fn refresh(identifier: &str, channel_id: &str) {
    refresh_inner(identifier, channel_id, Mode::Normal).await;
}

/// Refetch right away after a live 7TV edit, skipping the TTL. The cached set
/// keeps serving while the fetch runs (dropping it first left every 7TV word
/// in this chat as text for as long as the fetch took, up to 90 s), and it is
/// replaced, smaller or not, only by a whole answer: a removal must land,
/// a failed fetch must not wipe the set.
pub async fn refresh_after_edit(identifier: &str, channel_id: &str) {
    refresh_inner(identifier, channel_id, Mode::LiveEdit).await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Chat connect: throttled to the TTL.
    Normal,
    /// A live 7TV edit: right away.
    LiveEdit,
    /// The background retry of a partial table: right away, and it starts no
    /// other loop.
    Retry,
}

/// One fetch of the channel's table and what it did with it. True when the
/// answer was whole.
async fn refresh_inner(identifier: &str, channel_id: &str, mode: Mode) -> bool {
    // The channel id IS the key (see store_key); `identifier` is only for logs
    // and for knowing which chats hold the channel open.
    let key = channel_id.to_lowercase();
    if mode == Mode::Normal {
        if let Ok(mut s) = in_use_map().lock() {
            s.entry(key.clone()).or_default().insert(identifier.to_lowercase());
        }
    }
    // No set in memory yet (first use this run): seed the saved one so this
    // chat parses 7TV words from its first message, marked stale so the fetch
    // below still runs. Only whole answers are ever saved.
    if let Ok(mut s) = store().lock() {
        if !s.contains_key(&key) {
            if let Some(map) = crate::services::emote_set_cache::load_provider::<HashMap<String, YouTubeEmote>>("youtube7tv", &key) {
                log::debug!("[YouTube] seeded {} 7TV emote(s) for {} from disk", map.len(), key);
                let stale = Instant::now().checked_sub(TTL).unwrap_or_else(Instant::now);
                s.insert(key.clone(), ChannelEmotes { map, fetched_at: stale, standing: Standing::Saved });
            }
        }
    }
    if mode == Mode::Normal {
        if let Ok(s) = store().lock() {
            if let Some(c) = s.get(&key) {
                if c.fetched_at.elapsed() < TTL {
                    return c.standing == Standing::Whole;
                }
            }
        }
    }

    let client = crate::services::http::client_unbounded();
    let (map, whole) = fetch_table(&client, channel_id).await;
    let count = map.len();

    let install = install_table(&key, map, whole);
    if install == (Install::Replace { save: true }) {
        if let Ok(s) = store().lock() {
            if let Some(c) = s.get(&key) {
                crate::services::emote_set_cache::save_provider("youtube7tv", &key, &c.map);
            }
        }
    }

    if install != Install::Keep {
        log::info!(
            "[YouTube] 7TV emotes for channel {channel_id} (as {identifier}): {count} loaded{}",
            if whole { "" } else { " (partial)" }
        );
        repair_rows(&key, whole).await;
    }
    if !whole {
        log::warn!("[YouTube] 7TV channel set for {} did not fully resolve; retrying in the background", key);
        if mode != Mode::Retry {
            spawn_retries(identifier.to_string(), channel_id.to_string());
        }
    }
    whole
}

/// Puts a fetched table in place, or keeps the held one (`table_repair::decide`).
/// A partial table is used only while nothing better is held; whatever is
/// kept stops counting toward the TTL, since its retry loop fetches it again.
fn install_table(key: &str, map: HashMap<String, YouTubeEmote>, whole: bool) -> Install {
    let Ok(mut s) = store().lock() else { return Install::Keep };
    let count = map.len();
    let held = s.get(key).map(|c| (c.standing, c.map.len()));
    let install = table_repair::decide(held, whole, count);
    match install {
        Install::Replace { .. } => {
            let standing = if whole { Standing::Whole } else { Standing::Partial };
            s.insert(key.to_string(), ChannelEmotes { map, fetched_at: Instant::now(), standing });
        }
        Install::Keep => {
            if let Some(c) = s.get_mut(key) {
                c.fetched_at = Instant::now();
            }
            log::debug!(
                "[YouTube] 7TV refresh for {} was partial ({} emotes); keeping the held set ({})",
                key,
                count,
                held.map(|(_, len)| len).unwrap_or(0)
            );
        }
    }
    install
}

/// Fetches the channel's 7TV table: the globals with the channel's set over
/// them. Whole only when every part answered; a 404 for the channel means it
/// is not on 7TV, which is a whole answer (the globals alone).
async fn fetch_table(client: &reqwest::Client, channel_id: &str) -> (HashMap<String, YouTubeEmote>, bool) {
    let mut map: HashMap<String, YouTubeEmote> = HashMap::new();
    // Globals first so the channel set overrides on name collisions.
    let globals_ok = match table_repair::get_json(client, "https://7tv.io/v3/emote-sets/global", SEVENTV_TIMEOUT).await {
        Ok(Some(v)) => {
            collect_emotes(&v, "/emotes", &mut map);
            true
        }
        _ => false,
    };
    let chan_url = format!("https://7tv.io/v3/users/google/{channel_id}");
    let channel_ok = match table_repair::get_json(client, &chan_url, SEVENTV_TIMEOUT).await {
        Err(()) => false,
        // Not on 7TV: final.
        Ok(None) => true,
        Ok(Some(user)) => {
            if user.pointer("/emote_set/emotes").and_then(|e| e.as_array()).is_some() {
                // Inline set (pre-change payload).
                collect_emotes(&user, "/emote_set/emotes", &mut map);
                true
            } else if let Some(set_id) = emote_service::seventv_active_set_id(&user) {
                // Post-change payload: fetch the active set by id.
                let set_url = format!("https://7tv.io/v3/emote-sets/{set_id}");
                match table_repair::get_json(client, &set_url, SEVENTV_TIMEOUT).await {
                    Ok(Some(v)) => {
                        collect_emotes(&v, "/emotes", &mut map);
                        true
                    }
                    _ => false,
                }
            } else {
                // On 7TV with no active set.
                true
            }
        }
    };
    (map, globals_ok && channel_ok)
}

/// Fetches a partial table again in the background while a chat shows it.
fn spawn_retries(identifier: String, channel_id: String) {
    let key = channel_id.to_lowercase();
    let open = key.clone();
    table_repair::spawn(&RETRYING, key, move || in_use(&open), move || {
        let identifier = identifier.clone();
        let channel_id = channel_id.clone();
        async move { refresh_inner(&identifier, &channel_id, Mode::Retry).await }
    });
}

/// Bakes the held rows again with the channel's current table and patches the
/// ones that changed.
async fn repair_rows(key: &str, whole: bool) {
    if HELD.len(key) == 0 {
        return;
    }
    let table = store().lock().ok().and_then(|s| s.get(key).map(|c| c.map.clone()));
    let Some(table) = table else { return };
    let patches = HELD.repair(key, whole, |segments| table_repair::bake(segments, |word| baked(&table, word)));
    table_repair::send_patches(patches).await;
}

fn baked(table: &HashMap<String, YouTubeEmote>, word: &str) -> Option<table_repair::Baked> {
    table.get(word).map(|e| table_repair::Baked { id: e.id.clone(), url: e.url.clone(), zero_width: e.zero_width })
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
                owner_name: e
                    .pointer("/data/owner/display_name")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
                original_name: e
                    .pointer("/data/name")
                    .and_then(|x| x.as_str())
                    .filter(|own| !own.is_empty() && *own != name)
                    .map(str::to_string),
                owner_ref: e.pointer("/data/owner/id").and_then(|x| x.as_str()).map(str::to_string),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat_layout::MessageSegment;

    fn table(names: &[(&str, &str)]) -> HashMap<String, YouTubeEmote> {
        names
            .iter()
            .map(|(name, id)| {
                (
                    name.to_string(),
                    YouTubeEmote {
                        id: id.to_string(),
                        url: format!("https://cdn.7tv.app/emote/{id}/2x.webp"),
                        zero_width: false,
                        owner_name: None,
                        original_name: None,
                        owner_ref: None,
                    },
                )
            })
            .collect()
    }

    fn standing(key: &str) -> Option<(Standing, usize)> {
        store().lock().unwrap().get(key).map(|c| (c.standing, c.map.len()))
    }

    /// A row as the YouTube parser leaves it, with a YouTube emoji after the text.
    fn row(id: &str, channel: &str, body: &str) -> ChatMessage {
        let mut msg: ChatMessage = serde_json::from_value(serde_json::json!({
            "id": id,
            "user_id": "1",
            "username": "a",
            "display_name": "A",
            "color": null,
            "badges": [],
            "timestamp": "",
            "content": body,
            "tags": {},
            "provider": "youtube",
            "channel": format!("youtube:{channel}"),
        }))
        .expect("a row");
        msg.segments = vec![
            MessageSegment::Text { content: format!("{body} ") },
            MessageSegment::Emote {
                content: ":yt:".into(),
                emote_id: None,
                emote_url: "https://yt3.ggpht.com/emoji".into(),
                is_zero_width: None,
                modifier_flags: None,
                is_personal: None,
            },
        ];
        msg
    }

    #[test]
    fn a_partial_table_never_replaces_the_saved_copy() {
        let key = "ucrule1saved";
        store().lock().unwrap().insert(
            key.to_string(),
            ChannelEmotes {
                map: table(&[("OMEGALUL", "chan1"), ("EZ", "glob1")]),
                fetched_at: Instant::now(),
                standing: Standing::Saved,
            },
        );
        assert_eq!(install_table(key, table(&[("EZ", "glob1")]), false), Install::Keep);
        assert_eq!(standing(key), Some((Standing::Saved, 2)));
        assert_eq!(install_table(key, table(&[("EZ", "glob1")]), true), Install::Replace { save: true });
        assert_eq!(standing(key), Some((Standing::Whole, 1)));
    }

    #[test]
    fn a_partial_table_serves_only_when_nothing_is_held_and_is_never_saved() {
        let key = "ucrule1empty";
        assert_eq!(install_table(key, table(&[("EZ", "glob1")]), false), Install::Replace { save: false });
        assert_eq!(standing(key), Some((Standing::Partial, 1)));
        assert!(has_emotes(key), "the globals serve while nothing better is held");
        assert_eq!(install_table(key, table(&[("EZ", "glob1")]), false), Install::Keep);
        assert_eq!(
            install_table(key, table(&[("EZ", "glob1"), ("OMEGALUL", "chan1")]), true),
            Install::Replace { save: true }
        );
        assert_eq!(standing(key), Some((Standing::Whole, 2)));
    }

    #[test]
    fn rows_parsed_before_the_whole_table_are_held_and_repaired_once_it_lands() {
        let key = "ucrule4";
        install_table(key, table(&[("EZ", "glob1")]), false);
        hold_row(&row("m1", "UCrule4", "look OMEGALUL"));
        assert_eq!(HELD.len(key), 1, "held while the table is partial");

        install_table(key, table(&[("EZ", "glob1"), ("OMEGALUL", "chan1")]), true);
        let whole = store().lock().unwrap().get(key).map(|c| c.map.clone()).unwrap();
        let patches = HELD.repair(key, true, |s| table_repair::bake(s, |w| baked(&whole, w)));
        let rows = patches.get("youtube:UCrule4").expect("patched in the chat it was shown in");
        assert_eq!(rows.len(), 1);
        let ids: Vec<Option<&str>> = rows[0]["segments"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["type"] == "emote")
            .map(|s| s["emote_id"].as_str())
            .collect();
        assert_eq!(ids, vec![Some("chan1"), None], "the 7TV word baked, the YouTube emoji kept");
        assert_eq!(HELD.len(key), 0);

        hold_row(&row("m2", "UCrule4", "OMEGALUL"));
        assert_eq!(HELD.len(key), 0, "a whole table needs no repair");
    }

    #[test]
    fn a_channel_stays_in_use_until_every_chat_showing_it_is_left() {
        let key = "ucrule3";
        in_use_map().lock().unwrap().entry(key.to_string()).or_default().extend(["videoid1".to_string(), "@handle".to_string()]);
        hold_row(&row("m1", "UCrule3", "OMEGALUL"));
        assert_eq!(HELD.len(key), 1, "no table yet: held");
        release("VIDEOID1");
        assert!(in_use(key), "still open by handle");
        assert_eq!(HELD.len(key), 1);
        release("@handle");
        assert!(!in_use(key), "the retry loop sees no chat shows it");
        assert_eq!(HELD.len(key), 0);
    }
}
