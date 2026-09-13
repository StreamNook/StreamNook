use anyhow::Result;
use log::{debug, error, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

const CLIENT_ID: &str = env!("TWITCH_APP_CLIENT_ID");

// 7TV API circuit breaker. The 7TV API (the endpoint that lists a channel's
// emotes) has been getting overloaded; when it starts failing we stop hammering
// it. Exhausting retries on a 7TV request opens the circuit for a cooldown so
// subsequent 7TV calls fail fast (no waiting on 10s timeouts) instead of grinding
// a bulk prefetch scan to a halt; a success closes it immediately. Shared by the
// live picker and the AFK prefetch.
static SEVENTV_CIRCUIT_OPEN_UNTIL: AtomicU64 = AtomicU64::new(0); // unix secs; 0 = closed
const SEVENTV_CIRCUIT_COOLDOWN_SECS: u64 = 60;

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// True if the 7TV API circuit is currently open (recent repeated failures). The
/// prefetch reads this after scanning to know the emote counts are incomplete.
pub fn seventv_circuit_open() -> bool {
    unix_now_secs() < SEVENTV_CIRCUIT_OPEN_UNTIL.load(Ordering::Relaxed)
}

/// Open the 7TV circuit for the cooldown. Only a real outage signal (connect
/// failure, 5xx, or the small global fetch failing) may call this: a slow body
/// on a large channel document is not an outage, and opening the circuit on one
/// took every other 7TV fetch down for a minute each time (2026-09-07, kathi).
fn open_seventv_circuit() {
    SEVENTV_CIRCUIT_OPEN_UNTIL.store(
        unix_now_secs() + SEVENTV_CIRCUIT_COOLDOWN_SECS,
        Ordering::Relaxed,
    );
}

/// Budget for one channel document (a user's connection document or an emote
/// set). Sized for what those are: kathi's set is a 14 MB document 7TV takes
/// 5.6 to 8 s to produce (measured 2026-09-07). Nothing on a critical path
/// waits on this since 2026-08-29 (chat is disk-first and the refresh is
/// spawned), so it can afford Chatterino7's 20 to 25 s rather than the 4 s cap
/// that guards the small calls.
const SEVENTV_DOCUMENT_TIMEOUT: Duration = Duration::from_secs(25);

// Channel-independent 7TV data (the global set) cached process-wide, so a
// channel join or a prefetch-scan worker does not re-fetch it.
// Serve-stale-on-error: when 7TV is down an expired copy still beats an empty
// picker.
//
// Trending deliberately does NOT live here. It is a discovery list, not an
// emote layer: merged into a channel dictionary and deduped by id it replaces
// the channel's own alias with the trending row for the same emote, and makes
// chat render emotes nobody else in the room can see.
const SEVENTV_SHARED_TTL: Duration = Duration::from_secs(3600);

type SharedEmoteCache = RwLock<Option<(Instant, Vec<Emote>)>>;

static SEVENTV_GLOBALS_CACHE: OnceLock<SharedEmoteCache> = OnceLock::new();

fn seventv_globals_cache() -> &'static SharedEmoteCache {
    SEVENTV_GLOBALS_CACHE.get_or_init(|| RwLock::new(None))
}

/// The last known 7TV global set, any age (empty if never fetched). Delta
/// application needs it to restore a global whose name a channel row stopped
/// shadowing; a stale copy is fine for that, globals change rarely.
pub(crate) async fn seventv_globals_snapshot() -> Vec<Emote> {
    shared_cache_any(seventv_globals_cache())
        .await
        .unwrap_or_default()
}

/// Fresh hit -> Some(clone). Stale/empty -> None (caller fetches, then stores).
async fn shared_cache_fresh(cache: &'static SharedEmoteCache) -> Option<Vec<Emote>> {
    let guard = cache.read().await;
    match guard.as_ref() {
        Some((at, v)) if at.elapsed() < SEVENTV_SHARED_TTL => Some(v.clone()),
        _ => None,
    }
}

/// Stale fallback for a failed fetch (any age beats nothing).
async fn shared_cache_any(cache: &'static SharedEmoteCache) -> Option<Vec<Emote>> {
    cache.read().await.as_ref().map(|(_, v)| v.clone())
}

async fn shared_cache_store(cache: &'static SharedEmoteCache, v: Vec<Emote>) {
    *cache.write().await = Some((Instant::now(), v));
}

// The two 7TV ids the EventAPI subscribes with per channel (active emote set
// id, 7TV user id), captured from the channel document the emote fetch already
// parsed. Replaces a cache of the whole document (14 MB on a large channel,
// held for 60 s) whose only reader wanted these two strings. A miss falls back
// to one small v4 GQL lookup in the EventAPI, never to the document.
const SEVENTV_IDS_TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, Default)]
pub(crate) struct SeventvIds {
    pub emote_set_id: Option<String>,
    /// The 7TV user id (`/user/id` in the channel document), NOT the platform
    /// id at the document root. The presence endpoint takes this one; the
    /// platform id gets a 400 (measured 2026-09-07).
    pub user_id: Option<String>,
}

type SeventvIdCache = RwLock<HashMap<String, (Instant, SeventvIds)>>;

static SEVENTV_IDS: OnceLock<SeventvIdCache> = OnceLock::new();

fn seventv_ids() -> &'static SeventvIdCache {
    SEVENTV_IDS.get_or_init(|| RwLock::new(HashMap::new()))
}

pub(crate) async fn seventv_ids_cached(channel_id: &str) -> Option<SeventvIds> {
    let map = seventv_ids().read().await;
    map.get(channel_id)
        .filter(|(at, _)| at.elapsed() < SEVENTV_IDS_TTL)
        .map(|(_, ids)| ids.clone())
}

pub(crate) async fn seventv_ids_store(channel_id: &str, ids: SeventvIds) {
    let mut map = seventv_ids().write().await;
    map.retain(|_, v| v.0.elapsed() < SEVENTV_IDS_TTL);
    map.insert(channel_id.to_string(), (Instant::now(), ids));
}

/// Fetch a 7TV personal emote set by id and return only the emotes a 7TV
/// subscriber is approved to use in any channel. The 7TV EventAPI binds a user
/// to one of these sets via an EMOTE_SET entitlement; an emote counts as
/// personal-use only when its `data.state` carries "PERSONAL" (others are still
/// pending approval), so we filter on that exactly as the official client does.
/// One direct REST read by set id, parsed once into ready-to-render emotes.
pub async fn fetch_personal_emote_set(set_id: &str) -> Vec<Emote> {
    let mut out = Vec::new();
    let url = format!("https://7tv.io/v3/emote-sets/{}", set_id);
    let resp = match crate::services::http::client().get(&url).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return out,
    };
    let json: serde_json::Value = match resp.json().await {
        Ok(j) => j,
        Err(_) => return out,
    };
    let Some(arr) = json.get("emotes").and_then(|v| v.as_array()) else {
        return out;
    };
    for active in arr {
        let data = active.get("data").unwrap_or(active);

        let is_personal = data
            .get("state")
            .and_then(|v| v.as_array())
            .map(|st| st.iter().any(|s| s.as_str() == Some("PERSONAL")))
            .unwrap_or(false);
        if !is_personal {
            continue;
        }

        let id = data
            .get("id")
            .or_else(|| active.get("id"))
            .and_then(|v| v.as_str());
        let name = active.get("name").and_then(|v| v.as_str());
        if let (Some(id), Some(name)) = (id, name) {
            let flags = data
                .get("flags")
                .or_else(|| active.get("flags"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let width = data
                .pointer("/host/files/0/width")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let owner_name = data
                .pointer("/owner/display_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            out.push(Emote {
                id: id.to_string(),
                name: name.to_string(),
                url: format!("https://cdn.7tv.app/emote/{}/1x.avif", id),
                provider: EmoteProvider::SevenTV,
                is_zero_width: Some((flags & 256) == 256),
                local_url: None,
                emote_type: None,
                owner_id: None,
                width,
                owner_name,
                modifier_flags: None,
                ffz_sub_only: None,
            });
        }
    }
    out
}

/// Active emote-set id from a `/v3/users/:platform/:id` payload. Reads the
/// root `emote_set_id` (the field that survives 7TV's removal of the inline
/// `emote_set` object), falling back to `/emote_set/id` for legacy payloads.
/// Shared by every consumer of that endpoint so the detection cannot drift.
pub(crate) fn seventv_active_set_id(json: &serde_json::Value) -> Option<String> {
    json.get("emote_set_id")
        .and_then(|v| v.as_str())
        .or_else(|| json.pointer("/emote_set/id").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// The channel owner's 7TV user id from a `/v3/users/:platform/:id` payload.
/// The document root is the CONNECTION, whose `id` is the platform id; the 7TV
/// user id sits under `user.id`. (Measured 2026-09-07: the presence endpoint
/// answers 400 to the platform id and 200 to this one.)
pub(crate) fn seventv_user_id_from_payload(json: &serde_json::Value) -> Option<String> {
    json.pointer("/user/id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Compose a channel's 7TV dictionary: every row of the channel's own set, in
/// set order, then the globals whose names the channel did not take.
///
/// Keyed by NAME and nothing else. 7TV keys a set entry by (emote id, alias),
/// so one emote legitimately appears under two names in one set, and deduping
/// by id destroys the second alias. A channel row shadows a global of the same
/// name, which is what 7TV's own client renders.
///
/// A repeated NAME within the channel set also happens, from legacy rows
/// predating 7TV's uniqueness rule. A name-keyed dictionary holds one row per
/// name and the LAST row wins: it is the newer add, it matches what the delta
/// path produces when an add takes an existing name (so initial and patched
/// compositions agree), and it is how 7TV's own client builds its maps. The
/// winner keeps the first occurrence's position so picker order stays stable.
pub(crate) fn compose_seventv(channel: Vec<Emote>, globals: &[Emote]) -> Vec<Emote> {
    let mut slot: HashMap<String, usize> = HashMap::with_capacity(channel.len() + globals.len());
    let mut out: Vec<Emote> = Vec::with_capacity(channel.len() + globals.len());
    for e in channel {
        match slot.get(&e.name) {
            Some(&i) => out[i] = e,
            None => {
                slot.insert(e.name.clone(), out.len());
                out.push(e);
            }
        }
    }
    for g in globals {
        if !slot.contains_key(&g.name) {
            slot.insert(g.name.clone(), out.len());
            out.push(g.clone());
        }
    }
    out
}

/// One change to a channel's 7TV set, as the EventAPI dispatches it.
#[derive(Debug, Clone, Default)]
pub(crate) struct SeventvSetDelta {
    /// `pushed`: full rows, already parsed.
    pub added: Vec<Emote>,
    /// `pulled`: (emote id, alias) of each removed row.
    pub removed: Vec<(String, String)>,
    /// `updated`: (emote id, old alias, new row). Covers renames and flag
    /// changes; the new row carries whatever changed.
    pub updated: Vec<(String, String, Emote)>,
}

/// A removed row, by the two things that identify it in a name-keyed set.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub(crate) struct RemovedRow {
    pub id: String,
    pub name: String,
}

/// What a delta actually changed in the COMPOSED dictionary: rows to drop and
/// rows to add, including any global a removal or rename stopped shadowing.
/// Windows apply this blindly; the precedence lives here, once.
#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct SeventvComposedDelta {
    pub added: Vec<Emote>,
    pub removed: Vec<RemovedRow>,
}

/// Apply a set delta to a composed dictionary in place, keeping the invariant
/// that it always equals `compose_seventv(channel, globals)`. Returns the rows
/// that changed so every other copy can be patched identically.
pub(crate) fn apply_seventv_delta(
    seven_tv: &mut Vec<Emote>,
    delta: &SeventvSetDelta,
    globals: &[Emote],
) -> SeventvComposedDelta {
    let mut out = SeventvComposedDelta::default();

    // Drop every row carrying `name` (and, when given, only rows with that id,
    // so a stale removal can never take out an emote that since took the name).
    fn drop_name(
        rows: &mut Vec<Emote>,
        name: &str,
        only_id: Option<&str>,
        out: &mut SeventvComposedDelta,
    ) {
        let mut i = 0;
        while i < rows.len() {
            let hit = rows[i].name == name && only_id.is_none_or(|id| rows[i].id == id);
            if hit {
                let gone = rows.remove(i);
                out.removed.push(RemovedRow {
                    id: gone.id,
                    name: gone.name,
                });
            } else {
                i += 1;
            }
        }
    }

    // A name the channel stopped using goes back to the global that carries it.
    fn restore_global(
        rows: &mut Vec<Emote>,
        name: &str,
        globals: &[Emote],
        out: &mut SeventvComposedDelta,
    ) {
        if rows.iter().any(|r| r.name == name) {
            return;
        }
        if let Some(g) = globals.iter().find(|g| g.name == name) {
            rows.push(g.clone());
            out.added.push(g.clone());
        }
    }

    for (id, old_name, row) in &delta.updated {
        drop_name(seven_tv, old_name, Some(id), &mut out);
        drop_name(seven_tv, &row.name, None, &mut out);
        seven_tv.push(row.clone());
        out.added.push(row.clone());
        if old_name != &row.name {
            restore_global(seven_tv, old_name, globals, &mut out);
        }
    }
    for (id, name) in &delta.removed {
        drop_name(seven_tv, name, Some(id), &mut out);
        restore_global(seven_tv, name, globals, &mut out);
    }
    for row in &delta.added {
        drop_name(seven_tv, &row.name, None, &mut out);
        seven_tv.push(row.clone());
        out.added.push(row.clone());
    }
    out
}

/// Parse 7TV "active emote" objects (name at the root, full emote under `data`)
/// into ready-to-render Emotes. Both channel-set sources carry this shape: the
/// inline `emote_set.emotes` array (pre-change user payloads) and the
/// `/v3/emote-sets/{id}` endpoint (the post-change fetch).
pub(crate) fn parse_seventv_active_emotes(items: &[serde_json::Value]) -> Vec<Emote> {
    let mut out = Vec::new();
    for active_emote in items {
        let emote_data = active_emote.get("data").unwrap_or(active_emote);
        let emote_id = emote_data
            .get("id")
            .or_else(|| active_emote.get("id"))
            .and_then(|v| v.as_str());
        let name = active_emote.get("name").and_then(|v| v.as_str());
        if let (Some(id), Some(name)) = (emote_id, name) {
            let flags = emote_data
                .get("flags")
                .or_else(|| active_emote.get("flags"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let width = emote_data
                .pointer("/host/files/0/width")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
            let owner_name = emote_data
                .pointer("/owner/display_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            out.push(Emote {
                id: id.to_string(),
                name: name.to_string(),
                url: format!("https://cdn.7tv.app/emote/{}/1x.avif", id),
                provider: EmoteProvider::SevenTV,
                is_zero_width: Some((flags & 256) == 256),
                local_url: None,
                emote_type: None,
                owner_id: None,
                width,
                owner_name,
                modifier_flags: None,
                ffz_sub_only: None,
            });
        }
    }
    out
}

// Modifier-emote flag bits carried by `Emote::modifier_flags` and by
// `MessageSegment::Emote`. Bits 0..17 are FrankerFaceZ's own, in FFZ's
// authoritative declaration order. Bits 18+ are ours, for BetterTTV effects
// that have no FFZ equivalent. Codes whose effect is genuinely identical
// share a bit rather than getting a duplicate.
//
// The renderer mirrors this table in `src/utils/emoteModifiers.ts`; the two
// are hand-kept in sync, which is what the exact-value unit tests below guard.
const MOD_HIDDEN: u32 = 1; // modifier draws no art of its own
const MOD_FLIP_X: u32 = 1 << 1;
const MOD_FLIP_Y: u32 = 1 << 2;
const MOD_CURSED: u32 = 1 << 14;
const MOD_NO_SPACE: u32 = 1 << 17; // eats the space before the target
/// Marker: this modifier attaches to the emote AFTER it, not before it.
/// BetterTTV modifiers are prefixes; FrankerFaceZ modifiers are suffixes.
const MOD_PREFIX: u32 = 1 << 18;
const MOD_BTTV_WIDE: u32 = 1 << 19;
const MOD_BTTV_ROTATE_L: u32 = 1 << 20;
const MOD_BTTV_ROTATE_R: u32 = 1 << 21;
const MOD_BTTV_PARTY: u32 = 1 << 22;
const MOD_BTTV_SHAKE: u32 = 1 << 23;

/// BetterTTV emote modifiers, mirroring `EMOTE_MODIFIERS` in BetterTTV's own
/// client. Every one is hidden and attaches forward. A `modifier: true` emote
/// whose code is NOT in this table renders as an ordinary emote, which is what
/// BetterTTV does with a code missing from its map.
const BTTV_MODIFIER_FLAGS: &[(&str, u32)] = &[
    ("w!", MOD_HIDDEN | MOD_PREFIX | MOD_BTTV_WIDE),
    ("h!", MOD_HIDDEN | MOD_PREFIX | MOD_FLIP_X),
    ("v!", MOD_HIDDEN | MOD_PREFIX | MOD_FLIP_Y),
    ("l!", MOD_HIDDEN | MOD_PREFIX | MOD_BTTV_ROTATE_L),
    ("r!", MOD_HIDDEN | MOD_PREFIX | MOD_BTTV_ROTATE_R),
    ("c!", MOD_HIDDEN | MOD_PREFIX | MOD_CURSED),
    ("p!", MOD_HIDDEN | MOD_PREFIX | MOD_BTTV_PARTY),
    ("s!", MOD_HIDDEN | MOD_PREFIX | MOD_BTTV_SHAKE),
    ("z!", MOD_HIDDEN | MOD_PREFIX | MOD_NO_SPACE),
];

/// BetterTTV overlay ("zero-width") emotes. The API reports `modifier: false`
/// for all of them, so there is nothing to detect: BetterTTV's client hardcodes
/// these same ids in CSS and we mirror the list. Six are seasonal and drop out
/// of the global set outside winter, so the list stays complete year-round.
const BTTV_OVERLAY_EMOTE_IDS: &[&str] = &[
    "5e76d338d6581c3724c0f0b2", // cvHazmat
    "5e76d399d6581c3724c0f0b8", // cvMask
    "5849c9a4f52be01a7ee5f79d", // IceCold
    "567b5b520e984428652809b6", // SoSnowy
    "58487cc6f52be01a7ee5f205", // SantaHat
    "5849c9c8f52be01a7ee5f79e", // TopHat
    "567b5c080e984428652809ba", // CandyCane
    "567b5dc00e984428652809bd", // ReinDeer
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emote {
    pub id: String,
    pub name: String,
    pub url: String,
    pub provider: EmoteProvider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_zero_width: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_url: Option<String>,
    /// Type of emote: "globals", "subscriptions", "bitstier", "follower", "channelpoints", etc.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emote_type: Option<String>,
    /// Owner/broadcaster ID for subscription emotes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    /// Owner/author display name for emote attribution
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// Emote width in pixels (for aspect ratio sorting - wide emotes > 32)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    /// Modifier bitmask (see the MOD_* constants above). Some(_) marks the
    /// emote as a modifier: it attaches to the preceding emote by default, or
    /// to the following one when MOD_PREFIX is set (BetterTTV's `w!` and
    /// friends). Some(0) is a legacy FFZ overlay modifier with no effect bits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modifier_flags: Option<u32>,
    /// FFZ effect emote that only FFZ subscribers may compose with (set not in
    /// the API's default_sets). Rendering is never gated on this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ffz_sub_only: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::upper_case_acronyms)] // BTTV and FFZ are established acronyms (BetterTTV, FrankerFaceZ)
pub enum EmoteProvider {
    Twitch,
    BTTV,
    #[serde(rename = "7tv")]
    SevenTV,
    FFZ,
    // Kick's own native emotes (channel sub set + Global + Emojis), served from
    // files.kick.com. Only ever populated for Kick channels.
    Kick,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmoteSet {
    pub twitch: Vec<Emote>,
    pub bttv: Vec<Emote>,
    #[serde(rename = "7tv")]
    pub seven_tv: Vec<Emote>,
    pub ffz: Vec<Emote>,
    // Kick native emotes (empty for Twitch channels). Defaulted so older cached
    // payloads / the Twitch path deserialize without it.
    #[serde(default)]
    pub kick: Vec<Emote>,
    /// Whether the 7TV rows are this channel's real dictionary (the channel
    /// document answered, or the channel is simply not on 7TV). False means
    /// they are a fallback (globals only, or a disk copy) because the fetch
    /// failed, so a picker should keep retrying. Stored files are only ever
    /// written from authoritative fetches, hence the default.
    #[serde(default = "default_true")]
    pub seven_tv_ok: bool,
}

fn default_true() -> bool {
    true
}

impl EmoteSet {
    pub fn new() -> Self {
        Self {
            twitch: Vec::new(),
            bttv: Vec::new(),
            seven_tv: Vec::new(),
            ffz: Vec::new(),
            kick: Vec::new(),
            seven_tv_ok: true,
        }
    }

    pub fn total_count(&self) -> usize {
        self.twitch.len() + self.bttv.len() + self.seven_tv.len() + self.ffz.len() + self.kick.len()
    }
}

/// Outcome of fetching one 7TV document. See `get_seventv_document`.
enum DocFetch {
    Ok(reqwest::Response),
    /// A definitive "does not exist" (4xx other than 429): the channel is not
    /// on 7TV, or the set was deleted. Globals-only is the correct answer.
    NotFound,
    /// Timeout, connect failure, 5xx, 429, or an open circuit. The result must
    /// not be treated as this channel's set.
    Failed,
}

#[derive(Debug, Clone)]
struct CachedEmoteSet {
    set: EmoteSet,
    timestamp: SystemTime,
    /// Whether the 7TV CHANNEL-specific fetch definitively succeeded for this
    /// set. Carried through the cache so a cache hit reports the same authority
    /// as the original fetch (a globals-only set from a failed channel fetch must
    /// not later look authoritative just because it was cached).
    seven_tv_ok: bool,
}

// Clone is cheap (two Arcs, a pooled client handle, a Duration) and lets
// callers snapshot the service out of a lock before a network fetch instead of
// holding the lock across the await.
#[derive(Clone)]
pub struct EmoteService {
    // Memory cache: channel_id -> EmoteSet. LRU-bounded: entries were only
    // ever removed by a 7TV invalidation or an explicit clear, so a session
    // hopping channels retained every visited set (~0.5-1MB each) for process
    // life. 32 channels comfortably covers MultiNook + hopping.
    cache: Arc<RwLock<lru::LruCache<String, CachedEmoteSet>>>,
    // HTTP client with connection pooling
    client: reqwest::Client,
    // Cache duration (5 minutes like the TS version)
    cache_duration: Duration,
    // Cached authorized user ID to prevent rate-limiting on /validate
    cached_user_id: Arc<RwLock<Option<String>>>,
}

/// The `broadcaster_id` to send to Twitch's user-emotes endpoint, if any.
///
/// Helix rejects a non-numeric id outright, so anything else is dropped rather
/// than sent and logged as an error.
fn twitch_broadcaster_id<'a>(is_twitch: bool, channel_id: Option<&'a str>) -> Option<&'a str> {
    let id = channel_id?;
    if !is_twitch || id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(id)
}

impl EmoteService {
    /// Number of channel emote sets resident in the LRU (try-read; `None`
    /// while a refresh holds the lock). Diagnostics for the resource line.
    pub fn cache_len(&self) -> Option<usize> {
        self.cache.try_read().ok().map(|c| c.len())
    }


    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(32).expect("nonzero"),
            ))),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .gzip(true)
                .build()
                .unwrap_or_default(),
            cache_duration: Duration::from_secs(5 * 60), // 5 minutes
            cached_user_id: Arc::new(RwLock::new(None)),
        }
    }

    /// Fetch all emotes for a channel (or global) with concurrent requests.
    /// Thin wrapper over [`fetch_channel_emotes_checked`] for callers that don't
    /// need to know whether the 7TV channel set specifically came back.
    pub async fn fetch_channel_emotes(
        &self,
        channel_name: Option<String>,
        channel_id: Option<String>,
        access_token: Option<String>,
        provider: Option<String>,
    ) -> Result<EmoteSet> {
        Ok(self
            .fetch_channel_emotes_checked(channel_name, channel_id, access_token, provider)
            .await?
            .0)
    }

    /// Like [`fetch_channel_emotes`] but also reports whether the 7TV
    /// channel-specific fetch definitively succeeded. The chat layer uses this to
    /// decide whether a result is authoritative: `false` means 7TV's channel
    /// endpoint failed (timeout / tripped circuit breaker), so the 7TV array is
    /// globals-only and must NOT overwrite a healthy stored set. `true` includes
    /// the legitimate "channel isn't on 7TV" case (a clean 404), where a
    /// globals-only result is the correct answer for that channel.
    pub async fn fetch_channel_emotes_checked(
        &self,
        channel_name: Option<String>,
        channel_id: Option<String>,
        access_token: Option<String>,
        // Which platform `channel_id` belongs to. None means Twitch, so every
        // existing caller is unchanged.
        provider: Option<String>,
    ) -> Result<(EmoteSet, bool)> {
        let cache_key = channel_id.clone().unwrap_or_else(|| "global".to_string());

        // Check memory cache first
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.peek(&cache_key) {
                if let Ok(elapsed) = cached.timestamp.elapsed() {
                    if elapsed < self.cache_duration {
                        debug!("[EmoteService] Memory cache hit for {}", cache_key);
                        return Ok((cached.set.clone(), cached.seven_tv_ok));
                    }
                }
            }
        }

        let is_twitch = provider.as_deref().unwrap_or("twitch") == "twitch";
        debug!(
            "[EmoteService] Fetching emotes concurrently for channel: {:?}, ID: {:?}, provider: {:?}",
            channel_name, channel_id, provider
        );

        // Fetch all emote providers concurrently using tokio::join!
        // Include Twitch user emotes if we have an access token
        let (bttv_result, seven_tv_result, ffz_result, twitch_result) = tokio::join!(
            self.fetch_bttv_emotes(channel_name.clone(), channel_id.clone()),
            self.fetch_7tv_emotes(channel_name.clone(), channel_id.clone()),
            self.fetch_ffz_emotes(channel_name.clone()),
            // Twitch's `chat/emotes/user` takes a NUMERIC Twitch broadcaster_id, so a
            // YouTube UC id makes it 400 ("value must be numeric"). The id only means
            // anything for follower emotes on a Twitch channel, so it is omitted
            // elsewhere: the call still returns the user's own global and subscription
            // emotes, which is the correct result.
            //
            // Check the VALUE, not just the `provider` flag. `provider` is optional and
            // defaults to Twitch, so a caller that forgets it while holding a non-Twitch
            // id would silently reintroduce the 400. A shape the id can never
            // legally have is not worth sending under any provider.
            self.fetch_user_twitch_emotes(
                access_token.as_deref(),
                twitch_broadcaster_id(is_twitch, channel_id.as_deref()),
            )
        );

        // Collect results (log errors but continue with available emotes)
        let bttv_emotes = match bttv_result {
            Ok(emotes) => emotes,
            Err(e) => {
                error!("[EmoteService] BTTV fetch error: {}", e);
                Vec::new()
            }
        };

        let (seven_tv_emotes, seven_tv_ok) = match seven_tv_result {
            Ok((emotes, channel_ok)) => (emotes, channel_ok),
            Err(e) => {
                error!("[EmoteService] 7TV fetch error: {}", e);
                (Vec::new(), false)
            }
        };

        let ffz_emotes = match ffz_result {
            Ok(emotes) => emotes,
            Err(e) => {
                error!("[EmoteService] FFZ fetch error: {}", e);
                Vec::new()
            }
        };

        let has_twitch_error = twitch_result.is_err();
        let twitch_emotes = match twitch_result {
            Ok(emotes) => emotes,
            Err(e) => {
                error!("[EmoteService] Twitch user emotes fetch error: {}", e);
                // Fallback to hardcoded global emotes
                Self::get_global_twitch_emotes()
            }
        };

        // Build emote set
        let mut emote_set = EmoteSet {
            twitch: twitch_emotes,
            bttv: bttv_emotes,
            seven_tv: seven_tv_emotes,
            ffz: ffz_emotes,
            kick: Vec::new(),
            seven_tv_ok,
        };

        debug!(
            "[EmoteService] Fetched emotes: Twitch={}, BTTV={}, 7TV={}, FFZ={}",
            emote_set.twitch.len(),
            emote_set.bttv.len(),
            emote_set.seven_tv.len(),
            emote_set.ffz.len()
        );

        // Never hand back a worse 7TV set than the one already held. A
        // deficient result (timed-out channel document, tripped circuit) used to
        // replace whatever was cached, and the picker read it within
        // milliseconds of the EventAPI emit: kathi's 6,000 emotes became the
        // 287-row globals block (2026-09-07). The chat path already kept its
        // disk-seeded set in this case; the cache the picker reads now gets the
        // same rule. The returned bool still says the FETCH was not
        // authoritative, so the chat path installs and saves nothing.
        if !seven_tv_ok {
            let good = {
                let cache = self.cache.read().await;
                cache
                    .peek(&cache_key)
                    .filter(|c| c.seven_tv_ok)
                    .map(|c| c.set.clone())
            };
            if let Some(good) = good {
                debug!(
                    "[EmoteService] 7TV channel fetch deficient for {}; keeping the cached authoritative set ({} 7TV)",
                    cache_key,
                    good.seven_tv.len()
                );
                return Ok((good, false));
            }
            // Nothing good in memory: the disk dictionary (written only from
            // authoritative fetches) is the next best thing, for the picker
            // exactly as for chat. Its 7TV rows ride with THIS fetch's other
            // providers, marked not authoritative so the picker keeps retrying
            // until 7TV answers.
            if let Some(disk) = channel_id
                .as_deref()
                .and_then(crate::services::emote_set_cache::load)
            {
                if !disk.seven_tv.is_empty() {
                    debug!(
                        "[EmoteService] 7TV channel fetch deficient for {}; serving the disk dictionary ({} 7TV)",
                        cache_key,
                        disk.seven_tv.len()
                    );
                    emote_set.seven_tv = disk.seven_tv;
                }
            }
        }

        // Update memory cache
        {
            let mut cache = self.cache.write().await;
            // Back-date the timestamp so a DEGRADED fetch expires fast (~10s)
            // instead of being pinned for the full 5 min. Degraded means: Twitch
            // failing, OR 7TV returning nothing, OR the 7TV CHANNEL fetch failing
            // (seven_tv_ok == false) so the set is globals-only. That last case is
            // the key one: a globals-only set is not empty (44 globals), so without
            // this it would look healthy and pin for 5 min, serving a channel its
            // own emotes as plain text until the TTL expired. A short TTL lets it
            // re-fetch and self-heal once 7TV recovers, on both the chat and picker
            // paths. A channel genuinely not on 7TV has seven_tv_ok == true (clean
            // 404), so it is correctly NOT treated as degraded.
            let degraded = has_twitch_error || emote_set.seven_tv.is_empty() || !seven_tv_ok;
            let timestamp = if degraded {
                SystemTime::now()
                    .checked_sub(Duration::from_secs(290))
                    .unwrap_or(SystemTime::now())
            } else {
                SystemTime::now()
            };

            cache.put(
                cache_key,
                CachedEmoteSet {
                    set: emote_set.clone(),
                    timestamp,
                    seven_tv_ok,
                },
            );
        }

        Ok((emote_set, seven_tv_ok))
    }

    /// Drop a channel's cached emote set so the next fetch re-pulls fresh from
    /// the providers. Used when the 7TV EventAPI reports an emote set change, so
    /// a freshly opened window (which reads through this cache) does not serve a
    /// stale set until the 5 minute TTL expires.
    pub async fn invalidate_channel(&self, channel_id: &str) {
        self.cache.write().await.pop(channel_id);
    }

    /// Patch the cached set for `channel_id` with a live 7TV delta, so a window
    /// fetching after an EventAPI change gets the changed set without anyone
    /// re-downloading the channel document. No-op when the channel is not
    /// cached (the next fetch is fresh anyway).
    pub async fn apply_seventv_delta_cached(
        &self,
        channel_id: &str,
        delta: &SeventvSetDelta,
        globals: &[Emote],
    ) -> Option<SeventvComposedDelta> {
        let mut cache = self.cache.write().await;
        let entry = cache.get_mut(channel_id)?;
        Some(apply_seventv_delta(&mut entry.set.seven_tv, delta, globals))
    }

    /// Fetch one 7TV channel document (a user's connection document or an emote
    /// set) under [`SEVENTV_DOCUMENT_TIMEOUT`], keeping the two failure shapes
    /// apart: "this does not exist" is an answer, "the fetch failed" is not.
    ///
    /// A slow body is NOT a 7TV outage, so a timeout here never opens the
    /// circuit; only a connect failure or a 5xx does. This is also why it does
    /// not share [`get_with_retry`]: that path's 4 s single attempt exists to
    /// keep the small calls (globals, personal sets) from ever stalling a join,
    /// and it failed a 14 MB document by construction.
    async fn get_seventv_document(&self, url: &str) -> DocFetch {
        if seventv_circuit_open() {
            return DocFetch::Failed;
        }
        match self
            .client
            .get(url)
            .timeout(SEVENTV_DOCUMENT_TIMEOUT)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                SEVENTV_CIRCUIT_OPEN_UNTIL.store(0, Ordering::Relaxed);
                DocFetch::Ok(resp)
            }
            Ok(resp) => {
                let s = resp.status();
                if s.is_server_error() {
                    open_seventv_circuit();
                    DocFetch::Failed
                } else if s == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    DocFetch::Failed
                } else {
                    DocFetch::NotFound
                }
            }
            Err(e) => {
                if e.is_connect() {
                    open_seventv_circuit();
                }
                warn!("[EmoteService] 7TV document fetch failed ({}): {}", url, e);
                DocFetch::Failed
            }
        }
    }

    /// Get emote by name from cached emote set
    pub async fn get_emote_by_name(
        &self,
        channel_id: Option<String>,
        emote_name: &str,
    ) -> Option<Emote> {
        let cache_key = channel_id.unwrap_or_else(|| "global".to_string());

        let cache = self.cache.read().await;
        if let Some(cached) = cache.peek(&cache_key) {
            // Search in priority order: 7TV > FFZ > BTTV > Twitch
            for emote in &cached.set.seven_tv {
                if emote.name == emote_name {
                    return Some(emote.clone());
                }
            }
            for emote in &cached.set.ffz {
                if emote.name == emote_name {
                    return Some(emote.clone());
                }
            }
            for emote in &cached.set.bttv {
                if emote.name == emote_name {
                    return Some(emote.clone());
                }
            }
            for emote in &cached.set.twitch {
                if emote.name == emote_name {
                    return Some(emote.clone());
                }
            }
        }

        None
    }

    /// Clear the memory cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
        debug!("[EmoteService] Memory cache cleared");
    }

    /// GET with retry + a 7TV circuit breaker. Retries transient failures
    /// (network errors, 5xx, 429) with exponential backoff. A non-retryable 4xx
    /// (e.g. 404 = channel not on 7TV) returns None immediately WITHOUT opening
    /// the circuit. For 7TV URLs: if the circuit is open it fails fast; exhausting
    /// retries opens it; a success closes it.
    async fn get_with_retry(&self, url: &str, attempts: u32) -> Option<reqwest::Response> {
        let is_seventv = url.contains("7tv.");
        if is_seventv && seventv_circuit_open() {
            return None;
        }
        // 7TV is the only provider prone to multi-second stalls, and it sits on the
        // chat-load critical path: parse_historical_messages awaits the channel emote
        // fetch, whose join waits for ALL providers. A down 7TV at the shared 10s
        // client timeout x 3 attempts pinned that await ~31s (measured: a 41s
        // parse_historical that left chat blank on join). Cap 7TV hard — a short
        // per-request timeout and a SINGLE attempt — so a slow/down 7TV fails in ~4s
        // and immediately trips the circuit breaker, which then fast-skips 7TV on
        // every subsequent fetch for the cooldown (so only the first join after a 7TV
        // outage pays anything). Healthy 7TV answers in ~1s, well under the 4s budget;
        // the per-request retry resilience it gives up is covered by the circuit's
        // cooldown re-probe. Other providers keep their full timeout + retry budget.
        let attempts = if is_seventv { 1 } else { attempts.max(1) };
        let mut backoff_ms = 300u64;
        for attempt in 0..attempts {
            let mut req = self.client.get(url);
            if is_seventv {
                req = req.timeout(Duration::from_secs(4));
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    if is_seventv {
                        SEVENTV_CIRCUIT_OPEN_UNTIL.store(0, Ordering::Relaxed);
                    }
                    return Some(resp);
                }
                Ok(resp) => {
                    let s = resp.status();
                    // 4xx other than 429 are final and not a provider outage.
                    if !(s.is_server_error() || s == reqwest::StatusCode::TOO_MANY_REQUESTS) {
                        return None;
                    }
                }
                Err(_) => {} // network / timeout — retry
            }
            if attempt + 1 < attempts {
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                backoff_ms = (backoff_ms * 2).min(2000);
            }
        }
        if is_seventv {
            SEVENTV_CIRCUIT_OPEN_UNTIL.store(
                unix_now_secs() + SEVENTV_CIRCUIT_COOLDOWN_SECS,
                Ordering::Relaxed,
            );
        }
        None
    }

    /// Cheap liveness probe for the 7TV API. Used by the prefetch before scanning
    /// so it can warn that the count is incomplete when 7TV is down, instead of
    /// reporting a confident total that silently omits most 7TV emotes. Going
    /// through get_with_retry means a failure also trips the circuit breaker, so
    /// the scan that follows fails fast on 7TV rather than grinding.
    pub async fn seventv_api_healthy(&self) -> bool {
        self.get_with_retry("https://7tv.io/v3/emote-sets/global", 2)
            .await
            .is_some()
    }

    /// Fetch user-specific Twitch emotes using the Helix API
    /// Returns all emotes the user has access to: globals, subscriptions, drops, bits, etc.
    async fn fetch_user_twitch_emotes(
        &self,
        access_token: Option<&str>,
        broadcaster_id: Option<&str>,
    ) -> Result<Vec<Emote>> {
        let token = match access_token {
            Some(t) if !t.is_empty() => t,
            _ => {
                debug!("[EmoteService] No access token provided, using global emotes fallback");
                return Ok(Self::get_global_twitch_emotes());
            }
        };

        // Get user ID from cache, or fetch it via /validate
        let user_id = {
            let mut cache_write = self.cached_user_id.write().await;
            if let Some(id) = cache_write.clone() {
                id
            } else {
                // Fetch the user ID from the token validation endpoint
                let validate_response = self
                    .client
                    .get("https://id.twitch.tv/oauth2/validate")
                    .header("Authorization", format!("OAuth {}", token))
                    .send()
                    .await?;

                if !validate_response.status().is_success() {
                    return Err(anyhow::anyhow!("Token validation failed"));
                }

                let validate_data: serde_json::Value = validate_response.json().await?;
                let fetched_id = validate_data["user_id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("No user_id in token validation response"))?
                    .to_string();

                *cache_write = Some(fetched_id.clone());
                fetched_id
            }
        };

        debug!(
            "[EmoteService] Fetching Twitch emotes for user_id: {}",
            user_id
        );

        // Build the API URL with pagination support
        let mut all_emotes: Vec<Emote> = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut url = format!(
                "https://api.twitch.tv/helix/chat/emotes/user?user_id={}",
                user_id
            );

            // Include broadcaster_id for follower emotes if provided
            if let Some(bid) = broadcaster_id {
                url.push_str(&format!("&broadcaster_id={}", bid));
            }

            // Add pagination cursor if we have one
            if let Some(ref c) = cursor {
                url.push_str(&format!("&after={}", c));
            }

            let response = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", token))
                .header("Client-Id", CLIENT_ID)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                error!(
                    "[EmoteService] Twitch emotes API error {}: {}",
                    status, error_text
                );

                // Return fallback on error
                if all_emotes.is_empty() {
                    return Ok(Self::get_global_twitch_emotes());
                } else {
                    break;
                }
            }

            let data: serde_json::Value = response.json().await?;

            // Parse emotes from response
            if let Some(emotes_array) = data["data"].as_array() {
                for emote_data in emotes_array {
                    if let (Some(id), Some(name)) =
                        (emote_data["id"].as_str(), emote_data["name"].as_str())
                    {
                        // Capture emote type and owner for categorization
                        let emote_type = emote_data["emote_type"].as_str().map(|s| s.to_string());
                        let owner_id = emote_data["owner_id"].as_str().map(|s| s.to_string());

                        all_emotes.push(Emote {
                            id: id.to_string(),
                            name: name.to_string(),
                            // 2.0 (56 px) covers a 28 px chat row up to 2x DPR. 3.0 (112 px)
                            // decoded 16x the pixels a 1x row needs, and the renderer keeps
                            // every distinct emote it has shown in its image cache, which
                            // was the largest part of the 2 MB/min drift measured on
                            // 2026-09-06. Hover previews build their own 4.0 URL.
                            url: format!(
                                "https://static-cdn.jtvnw.net/emoticons/v2/{}/default/dark/2.0",
                                id
                            ),
                            provider: EmoteProvider::Twitch,
                            is_zero_width: None,
                            local_url: None,
                            emote_type,
                            owner_id,
                            width: None,
                            owner_name: None,
                            modifier_flags: None,
                            ffz_sub_only: None,
                        });
                    }
                }
            }

            // Check for pagination cursor
            cursor = data["pagination"]["cursor"].as_str().map(|s| s.to_string());

            if cursor.is_none() {
                break;
            }
        }

        debug!(
            "[EmoteService] Fetched {} Twitch user emotes",
            all_emotes.len()
        );

        // If we got no emotes (e.g., new account), return hardcoded globals
        if all_emotes.is_empty() {
            return Ok(Self::get_global_twitch_emotes());
        }

        Ok(all_emotes)
    }

    // Private helper methods for fetching from each provider

    /// Parse one BetterTTV emote. A `modifier: true` emote whose code is in
    /// BTTV_MODIFIER_FLAGS rides the zero-width grouping machinery (the same
    /// mapping FFZ modifiers use) and carries its flag bitmask, including
    /// MOD_PREFIX so the renderer attaches it to the FOLLOWING emote. Overlay
    /// emotes are matched by id because BetterTTV's API does not flag them.
    fn parse_bttv_emote(item: &serde_json::Value) -> Option<Emote> {
        let id = item.get("id").and_then(|v| v.as_str())?;
        let code = item.get("code").and_then(|v| v.as_str())?;

        let modifier_flags = if item
            .get("modifier")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            BTTV_MODIFIER_FLAGS
                .iter()
                .find(|(c, _)| *c == code)
                .map(|(_, flags)| *flags)
        } else {
            None
        };
        let is_overlay = BTTV_OVERLAY_EMOTE_IDS.contains(&id);

        Some(Emote {
            id: id.to_string(),
            name: code.to_string(),
            url: format!("https://cdn.betterttv.net/emote/{}/1x", id),
            provider: EmoteProvider::BTTV,
            is_zero_width: if modifier_flags.is_some() || is_overlay {
                Some(true)
            } else {
                None
            },
            local_url: None,
            emote_type: None,
            owner_id: None,
            width: None,
            owner_name: None,
            modifier_flags,
            ffz_sub_only: None,
        })
    }

    async fn fetch_bttv_emotes(
        &self,
        _channel_name: Option<String>,
        channel_id: Option<String>,
    ) -> Result<Vec<Emote>> {
        let mut emotes = Vec::new();

        // Fetch global BTTV emotes
        match self
            .client
            .get("https://api.betterttv.net/3/cached/emotes/global")
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                if let Ok(json) = response.json::<serde_json::Value>().await {
                    if let Some(array) = json.as_array() {
                        emotes.extend(array.iter().filter_map(Self::parse_bttv_emote));
                    }
                }
            }
            Ok(_) => error!("[EmoteService] BTTV global: non-success status"),
            Err(e) => error!("[EmoteService] BTTV global request failed: {}", e),
        }

        // Fetch channel-specific BTTV emotes
        if let Some(channel_id) = channel_id {
            match self
                .client
                .get(format!(
                    "https://api.betterttv.net/3/cached/users/twitch/{}",
                    channel_id
                ))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    if let Ok(json) = response.json::<serde_json::Value>().await {
                        // Channel emotes, then shared emotes. Neither can be a
                        // modifier (BetterTTV only sets `modifier` on globals),
                        // but both can be overlay emotes, so both go through
                        // the same parser.
                        for key in ["channelEmotes", "sharedEmotes"] {
                            if let Some(list) = json.get(key).and_then(|v| v.as_array()) {
                                emotes.extend(list.iter().filter_map(Self::parse_bttv_emote));
                            }
                        }
                    }
                }
                Ok(_) => {} // Channel not found or error - not critical
                Err(e) => error!("[EmoteService] BTTV channel request failed: {}", e),
            }
        }

        Ok(emotes)
    }

    async fn fetch_7tv_emotes(
        &self,
        _channel_name: Option<String>,
        channel_id: Option<String>,
    ) -> Result<(Vec<Emote>, bool)> {
        // True once we have a definitive answer for the channel's 7TV set: a 200
        // that parsed, a clean 404 (channel simply isn't on 7TV), or no channel
        // requested at all. Stays false only when the channel fetch failed
        // (timeout / 5xx / tripped circuit), meaning the result is globals-only
        // and the caller must not treat it as this channel's real set.
        let mut channel_ok = channel_id.is_none();

        // Global 7TV emotes (v3). Channel-independent, cached process-wide.
        let globals: Vec<Emote> =
            if let Some(cached) = shared_cache_fresh(seventv_globals_cache()).await {
                cached
            } else {
                let mut globals: Vec<Emote> = Vec::new();
                match self
                    .get_with_retry("https://7tv.io/v3/emote-sets/global", 3)
                    .await
                {
                    Some(response) => {
                        if let Ok(json) = response.json::<serde_json::Value>().await {
                            if let Some(items) = json.get("emotes").and_then(|v| v.as_array()) {
                                globals = parse_seventv_active_emotes(items);
                            }
                        }
                    }
                    None => error!("[EmoteService] 7TV global unavailable (after retries)"),
                }
                // One rule for every failure shape: fall back to any stale copy.
                if globals.is_empty() {
                    shared_cache_any(seventv_globals_cache())
                        .await
                        .unwrap_or_default()
                } else {
                    shared_cache_store(seventv_globals_cache(), globals.clone()).await;
                    globals
                }
            };

        // The channel's own active set, under the channel-document budget.
        let mut channel: Vec<Emote> = Vec::new();
        if let Some(channel_id) = channel_id {
            match self
                .get_seventv_document(&format!("https://7tv.io/v3/users/twitch/{}", channel_id))
                .await
            {
                DocFetch::Ok(response) => {
                    // A 200 is a definitive answer for this channel only once its
                    // body has parsed. A body that did not arrive or did not parse
                    // is a failed fetch, not "no channel emotes": marking it ok
                    // before parsing wrote a globals-only set to disk as the
                    // authoritative dictionary (2026-09-05, ohnepixel: 295 cached
                    // of 950 live, every channel emote rendered as text).
                    let parsed = response.json::<serde_json::Value>().await;
                    if let Err(e) = &parsed {
                        warn!(
                            "[EmoteService] 7TV user payload for {} did not parse: {}",
                            channel_id, e
                        );
                    }
                    channel_ok = parsed.is_ok();
                    if let Ok(json) = parsed {
                        // Hand the EventAPI the two ids it subscribes with, so it
                        // never re-downloads this document to find them.
                        seventv_ids_store(
                            &channel_id,
                            SeventvIds {
                                emote_set_id: seventv_active_set_id(&json),
                                user_id: seventv_user_id_from_payload(&json),
                            },
                        )
                        .await;
                        if let Some(items) =
                            json.pointer("/emote_set/emotes").and_then(|v| v.as_array())
                        {
                            // Inline set: authoritative, even if empty.
                            channel = parse_seventv_active_emotes(items);
                        } else if let Some(set_id) = seventv_active_set_id(&json) {
                            // `emote_set` omitted: fetch the set by id.
                            match self
                                .get_seventv_document(&format!(
                                    "https://7tv.io/v3/emote-sets/{}",
                                    set_id
                                ))
                                .await
                            {
                                DocFetch::Ok(set_resp) => {
                                    match set_resp.json::<serde_json::Value>().await {
                                        Ok(set_json) => {
                                            if let Some(items) =
                                                set_json.get("emotes").and_then(|v| v.as_array())
                                            {
                                                channel = parse_seventv_active_emotes(items);
                                            }
                                        }
                                        Err(e) => {
                                            warn!(
                                                "[EmoteService] 7TV emote set {} did not parse: {}",
                                                set_id, e
                                            );
                                            channel_ok = false;
                                        }
                                    }
                                }
                                // The set was deleted: on 7TV, no active set.
                                // The user document already parsed, so this
                                // stays a definitive answer.
                                DocFetch::NotFound => channel_ok = true,
                                DocFetch::Failed => channel_ok = false,
                            }
                        }
                        // else: on 7TV but no active set, globals-only is the
                        // real answer.
                    }
                }
                // Not on 7TV: globals-only is correct, and definitive. This
                // MUST be marked authoritative: channel_ok starts false for a
                // requested channel, and leaving it there would make every
                // channel that simply is not on 7TV look like a failed fetch
                // (chat keeps a stale seed, the picker retries forever). The
                // live dictionary test caught exactly that on 2026-09-07.
                DocFetch::NotFound => channel_ok = true,
                DocFetch::Failed => channel_ok = false,
            }
        }

        Ok((compose_seventv(channel, &globals), channel_ok))
    }

    /// Pick the best CDN URL for an FFZ emoticon.
    ///
    /// Animated FFZ emotes expose a separate `animated` object (WebP) alongside
    /// the static `urls` (PNG). Prefer the animated 1x variant when present so
    /// animated emotes actually move, falling back to the static 1x URL, then a
    /// constructed default.
    fn ffz_emote_url(item: &serde_json::Value, id: i64) -> String {
        item.pointer("/animated/1")
            .and_then(|v| v.as_str())
            .or_else(|| item.pointer("/urls/1").and_then(|v| v.as_str()))
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("https://cdn.frankerfacez.com/emote/{}/1", id))
    }

    /// Parse one FFZ emoticon into an Emote. A `modifier: true` emoticon is an
    /// FFZ modifier (attaches to the preceding emote): it rides the zero-width
    /// grouping machinery (the same mapping BTTV's modifier bool uses) and
    /// carries its `modifier_flags` bitmask for the renderer. `sub_only` marks
    /// emotes from non-default global sets (FFZ subscriber effect perks).
    fn parse_ffz_emoticon(item: &serde_json::Value, sub_only: bool) -> Option<Emote> {
        let id = item.get("id").and_then(|v| v.as_i64())?;
        let name = item.get("name").and_then(|v| v.as_str())?;
        let is_modifier = item
            .get("modifier")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let modifier_flags = if is_modifier {
            // Some(0) is a legacy overlay modifier with no effect bits.
            Some(
                item.get("modifier_flags")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
            )
        } else {
            None
        };
        Some(Emote {
            id: id.to_string(),
            name: name.to_string(),
            url: Self::ffz_emote_url(item, id),
            provider: EmoteProvider::FFZ,
            is_zero_width: if is_modifier { Some(true) } else { None },
            local_url: None,
            emote_type: None,
            owner_id: None,
            width: None,
            owner_name: None,
            modifier_flags,
            ffz_sub_only: if sub_only { Some(true) } else { None },
        })
    }

    async fn fetch_ffz_emotes(&self, channel_name: Option<String>) -> Result<Vec<Emote>> {
        let mut emotes = Vec::new();

        // Fetch global FFZ emotes
        match self
            .client
            .get("https://api.frankerfacez.com/v1/set/global")
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                if let Ok(json) = response.json::<serde_json::Value>().await {
                    // Sets outside `default_sets` (e.g. the Subwoofer Emote
                    // Effects set) are FFZ-subscriber perks: rendered for
                    // everyone, offered in the picker only to subscribers.
                    let default_sets: Vec<i64> = json
                        .get("default_sets")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|v| v.as_i64()).collect())
                        .unwrap_or_default();
                    if let Some(sets) = json.get("sets").and_then(|v| v.as_object()) {
                        for (set_id, set_data) in sets {
                            let sub_only = set_id
                                .parse::<i64>()
                                .map(|id| !default_sets.contains(&id))
                                .unwrap_or(false);
                            if let Some(emoticons) =
                                set_data.get("emoticons").and_then(|v| v.as_array())
                            {
                                for item in emoticons {
                                    if let Some(emote) = Self::parse_ffz_emoticon(item, sub_only) {
                                        emotes.push(emote);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(_) => error!("[EmoteService] FFZ global: non-success status"),
            Err(e) => error!("[EmoteService] FFZ global request failed: {}", e),
        }

        // Fetch channel-specific FFZ emotes
        if let Some(channel_name) = channel_name {
            match self
                .client
                .get(format!(
                    "https://api.frankerfacez.com/v1/room/{}",
                    channel_name
                ))
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    if let Ok(json) = response.json::<serde_json::Value>().await {
                        if let Some(sets) = json.get("sets").and_then(|v| v.as_object()) {
                            for (_set_id, set_data) in sets {
                                if let Some(emoticons) =
                                    set_data.get("emoticons").and_then(|v| v.as_array())
                                {
                                    for item in emoticons {
                                        if let Some(emote) = Self::parse_ffz_emoticon(item, false)
                                        {
                                            emotes.push(emote);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(_) => {} // Channel not found - not critical
                Err(e) => error!("[EmoteService] FFZ channel request failed: {}", e),
            }
        }

        Ok(emotes)
    }

    fn get_global_twitch_emotes() -> Vec<Emote> {
        vec![
            Emote {
                id: "25".to_string(),
                name: "Kappa".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/25/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "354".to_string(),
                name: "4Head".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/354/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "425618".to_string(),
                name: "LUL".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/425618/default/dark/3.0"
                    .to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "305954156".to_string(),
                name: "Pog".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/305954156/default/dark/3.0"
                    .to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "88".to_string(),
                name: "PogChamp".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/88/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "81273".to_string(),
                name: "BibleThump".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/81273/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "81248".to_string(),
                name: "Kreygasm".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/81248/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "81249".to_string(),
                name: "ResidentSleeper".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/81249/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "81274".to_string(),
                name: "FailFish".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/81274/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "81997".to_string(),
                name: "NotLikeThis".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/81997/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "166266".to_string(),
                name: "CoolCat".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/166266/default/dark/3.0"
                    .to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "191762".to_string(),
                name: "CoolStoryBob".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/191762/default/dark/3.0"
                    .to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "196892".to_string(),
                name: "SeemsGood".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/196892/default/dark/3.0"
                    .to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "245".to_string(),
                name: "KappaHD".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/245/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
            Emote {
                id: "1902".to_string(),
                name: "Keepo".to_string(),
                url: "https://static-cdn.jtvnw.net/emoticons/v2/1902/default/dark/3.0".to_string(),
                provider: EmoteProvider::Twitch,
                is_zero_width: None,
                local_url: None,
                emote_type: None,
                owner_id: None,
                width: None,
                owner_name: None,
                modifier_flags: None,
                ffz_sub_only: None,
            },
        ]
    }
}

impl Default for EmoteService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_active_emotes_with_nested_data() {
        let items = vec![serde_json::json!({
            "id": "active-id",
            "name": "PogChamp",
            "data": {
                "id": "data-id",
                "flags": 256,
                "host": { "files": [{ "width": 32 }] },
                "owner": { "display_name": "someone" }
            }
        })];
        let out = parse_seventv_active_emotes(&items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "data-id");
        assert_eq!(out[0].name, "PogChamp");
        assert_eq!(out[0].is_zero_width, Some(true));
        assert_eq!(out[0].width, Some(32));
        assert_eq!(out[0].owner_name.as_deref(), Some("someone"));
    }

    #[test]
    fn parses_active_emotes_flat_fallback() {
        let items = vec![serde_json::json!({
            "id": "flat-id",
            "name": "Kappa",
            "flags": 0
        })];
        let out = parse_seventv_active_emotes(&items);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "flat-id");
        assert_eq!(out[0].is_zero_width, Some(false));
        assert_eq!(out[0].width, None);
        assert_eq!(out[0].owner_name, None);
    }

    #[test]
    fn set_id_from_post_change_payload() {
        let json = serde_json::json!({ "emote_set": null, "emote_set_id": "01ABC" });
        assert_eq!(seventv_active_set_id(&json).as_deref(), Some("01ABC"));
    }

    #[test]
    fn set_id_legacy_fallback() {
        let json = serde_json::json!({ "emote_set": { "id": "01DEF", "emotes": [] } });
        assert_eq!(seventv_active_set_id(&json).as_deref(), Some("01DEF"));
    }

    #[test]
    fn set_id_absent_or_empty() {
        assert_eq!(seventv_active_set_id(&serde_json::json!({})), None);
        assert_eq!(
            seventv_active_set_id(&serde_json::json!({ "emote_set_id": "" })),
            None
        );
    }

    #[test]
    fn user_id_comes_from_the_nested_user_not_the_connection_root() {
        // The root `id` is the PLATFORM id; the presence endpoint 400s on it.
        let json = serde_json::json!({
            "id": "71092938",
            "emote_set_id": "01FE9DRF000009TR6M9N941CYW",
            "user": { "id": "01FE9DRF000009TR6M9N941CYW" }
        });
        assert_eq!(
            seventv_user_id_from_payload(&json).as_deref(),
            Some("01FE9DRF000009TR6M9N941CYW")
        );
        assert_eq!(seventv_user_id_from_payload(&serde_json::json!({ "id": "1" })), None);
    }

    fn stv(id: &str, name: &str) -> Emote {
        Emote {
            id: id.to_string(),
            name: name.to_string(),
            url: format!("https://cdn.7tv.app/emote/{}/1x.avif", id),
            provider: EmoteProvider::SevenTV,
            is_zero_width: Some(false),
            local_url: None,
            emote_type: None,
            owner_id: None,
            width: None,
            owner_name: None,
            modifier_flags: None,
            ffz_sub_only: None,
        }
    }

    fn names(rows: &[Emote]) -> Vec<(&str, &str)> {
        rows.iter().map(|e| (e.id.as_str(), e.name.as_str())).collect()
    }

    // The seam the 2026-09-07 report sat on: a channel alias for an emote that
    // is also a global must survive, under the channel's name. Positive control:
    // revert compose_seventv to a dedupe-by-id and this fails.
    #[test]
    fn compose_channel_alias_beats_global_by_name_and_keeps_the_global_id() {
        let channel = vec![stv("X", "Cinema")];
        let globals = vec![stv("X", "7Cinema")];
        let out = compose_seventv(channel, &globals);
        // Same emote id twice is fine: two names, two words in chat.
        assert_eq!(names(&out), vec![("X", "Cinema"), ("X", "7Cinema")]);
    }

    #[test]
    fn compose_keeps_two_aliases_of_one_emote() {
        // kathi carried 44 of these pairs; dedupe-by-id kept only the first.
        let channel = vec![stv("X", "shutup"), stv("X", "shadup")];
        let out = compose_seventv(channel, &[]);
        assert_eq!(names(&out), vec![("X", "shutup"), ("X", "shadup")]);
    }

    #[test]
    fn compose_channel_shadows_global_with_the_same_name() {
        let channel = vec![stv("X", "Pog")];
        let globals = vec![stv("G", "Pog"), stv("G2", "Kappa")];
        let out = compose_seventv(channel, &globals);
        assert_eq!(names(&out), vec![("X", "Pog"), ("G2", "Kappa")]);
    }

    #[test]
    fn compose_repeated_name_within_the_channel_resolves_to_the_last_row() {
        // Legacy sets repeat names (kathi: MEGALUL from 2021-03 and again from
        // 2021-05). The newer add wins, in the first row's position, and the
        // result matches what a delta add of the same name would produce.
        let channel = vec![stv("A", "dup"), stv("C", "other"), stv("B", "dup")];
        let out = compose_seventv(channel, &[]);
        assert_eq!(names(&out), vec![("B", "dup"), ("C", "other")]);

        let mut patched = compose_seventv(vec![stv("A", "dup"), stv("C", "other")], &[]);
        let add = SeventvSetDelta {
            added: vec![stv("B", "dup")],
            ..Default::default()
        };
        apply_seventv_delta(&mut patched, &add, &[]);
        let mut a: Vec<(&str, &str)> = names(&out);
        let mut b: Vec<(&str, &str)> = names(&patched);
        a.sort();
        b.sort();
        assert_eq!(a, b);
    }

    #[test]
    fn delta_add_shadows_global_and_remove_restores_it() {
        let globals = vec![stv("G", "Pog")];
        let mut set = compose_seventv(vec![], &globals);
        assert_eq!(names(&set), vec![("G", "Pog")]);

        let add = SeventvSetDelta {
            added: vec![stv("X", "Pog")],
            ..Default::default()
        };
        let out = apply_seventv_delta(&mut set, &add, &globals);
        assert_eq!(names(&set), vec![("X", "Pog")]);
        assert_eq!(out.removed, vec![RemovedRow { id: "G".into(), name: "Pog".into() }]);
        assert_eq!(names(&out.added), vec![("X", "Pog")]);

        let remove = SeventvSetDelta {
            removed: vec![("X".into(), "Pog".into())],
            ..Default::default()
        };
        let out = apply_seventv_delta(&mut set, &remove, &globals);
        assert_eq!(names(&set), vec![("G", "Pog")]);
        assert_eq!(out.removed, vec![RemovedRow { id: "X".into(), name: "Pog".into() }]);
        assert_eq!(names(&out.added), vec![("G", "Pog")]);
        // The invariant: the patched set equals a fresh composition.
        assert_eq!(names(&set), names(&compose_seventv(vec![], &globals)));
    }

    #[test]
    fn delta_rename_frees_the_old_name_and_restores_its_global() {
        let globals = vec![stv("G", "Pog")];
        let mut set = compose_seventv(vec![stv("X", "Pog")], &globals);
        let rename = SeventvSetDelta {
            updated: vec![("X".into(), "Pog".into(), stv("X", "Pog2"))],
            ..Default::default()
        };
        apply_seventv_delta(&mut set, &rename, &globals);
        assert_eq!(names(&set), vec![("X", "Pog2"), ("G", "Pog")]);
        assert_eq!(
            names(&set),
            names(&compose_seventv(vec![stv("X", "Pog2")], &globals))
        );
    }

    #[test]
    fn delta_second_alias_add_keeps_the_first_alias() {
        let mut set = compose_seventv(vec![stv("X", "shutup")], &[]);
        let add = SeventvSetDelta {
            added: vec![stv("X", "shadup")],
            ..Default::default()
        };
        let out = apply_seventv_delta(&mut set, &add, &[]);
        assert_eq!(names(&set), vec![("X", "shutup"), ("X", "shadup")]);
        assert!(out.removed.is_empty());
    }

    #[test]
    fn delta_stale_removal_never_takes_a_row_that_since_took_the_name() {
        // Channel removed X/"Pog" long ago and later added Y/"Pog"; a late
        // replay of the first removal must not delete Y.
        let mut set = compose_seventv(vec![stv("Y", "Pog")], &[]);
        let remove = SeventvSetDelta {
            removed: vec![("X".into(), "Pog".into())],
            ..Default::default()
        };
        let out = apply_seventv_delta(&mut set, &remove, &[]);
        assert_eq!(names(&set), vec![("Y", "Pog")]);
        assert!(out.removed.is_empty() && out.added.is_empty());
    }

    /// Live proof that the composed dictionary is complete for a real channel:
    /// every (id, alias) row 7TV serves is present, and every extra row is a
    /// global. Network-bound and slow (a 14 MB document, twice), so ignored by
    /// default; run it as the acceptance check for the dictionary:
    ///
    ///   cargo test --no-default-features live_channel_dictionary -- --ignored --nocapture
    ///
    /// SEVENTV_LIVE_CHANNEL_ID overrides the channel. The default is kathi:
    /// about 6,200 rows with 44 duplicate-alias pairs, the case that broke.
    #[test]
    #[ignore = "network: fetches a real channel from 7TV"]
    fn live_channel_dictionary_is_complete() {
        let channel_id = std::env::var("SEVENTV_LIVE_CHANNEL_ID")
            .unwrap_or_else(|_| "418422047".to_string());
        tauri::async_runtime::block_on(async {
            let svc = EmoteService::new();
            let started = Instant::now();
            let (rows, ok) = svc
                .fetch_7tv_emotes(None, Some(channel_id.clone()))
                .await
                .expect("fetch_7tv_emotes");
            let fetch_ms = started.elapsed().as_millis();
            assert!(ok, "channel document fetch was not authoritative");

            // Independent truth: the raw document, read with a plain client.
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("client");
            let truth: serde_json::Value = http
                .get(format!("https://7tv.io/v3/users/twitch/{channel_id}"))
                .send()
                .await
                .expect("truth fetch")
                .json()
                .await
                .expect("truth json");
            let live: Vec<(String, String)> = truth
                .pointer("/emote_set/emotes")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|e| {
                            let id = e.pointer("/data/id").or_else(|| e.get("id"))?.as_str()?;
                            let name = e.get("name")?.as_str()?;
                            Some((id.to_string(), name.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let globals: serde_json::Value = http
                .get("https://7tv.io/v3/emote-sets/global")
                .send()
                .await
                .expect("globals fetch")
                .json()
                .await
                .expect("globals json");
            let global_names: std::collections::HashSet<String> = globals
                .get("emotes")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| e.get("name")?.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            // Coverage is by NAME: a name-keyed dictionary holds one row per
            // name, so a set that repeats a name (legacy rows) is complete when
            // every distinct name is present and each repeated name resolves to
            // its LAST row, as the delta path and 7TV's own client do.
            let ours_by_name: HashMap<&str, &Emote> =
                rows.iter().map(|e| (e.name.as_str(), e)).collect();
            let mut last_by_name: HashMap<&str, &str> = HashMap::new();
            let mut name_counts: HashMap<&str, usize> = HashMap::new();
            let mut id_counts: HashMap<&str, usize> = HashMap::new();
            for (id, name) in &live {
                last_by_name.insert(name.as_str(), id.as_str());
                *name_counts.entry(name.as_str()).or_insert(0) += 1;
                *id_counts.entry(id.as_str()).or_insert(0) += 1;
            }
            let repeated_names = name_counts.values().filter(|n| **n > 1).count();
            let dup_alias_ids = id_counts.values().filter(|n| **n > 1).count();
            let missing: Vec<&str> = last_by_name
                .keys()
                .copied()
                .filter(|n| !ours_by_name.contains_key(n))
                .collect();
            let wrong_winner: Vec<(&str, &str, &str)> = last_by_name
                .iter()
                .filter_map(|(n, want)| {
                    let got = ours_by_name.get(n)?;
                    (got.id != *want).then_some((*n, *want, got.id.as_str()))
                })
                .collect();
            let extra: Vec<&Emote> = rows
                .iter()
                .filter(|e| {
                    !last_by_name.contains_key(e.name.as_str()) && !global_names.contains(&e.name)
                })
                .collect();
            eprintln!(
                "channel {channel_id}: fetched in {fetch_ms} ms; live rows {}, distinct names {}, composed rows {}, duplicate-alias ids {}, repeated names {}, missing names {}, wrong winners {}, extra {}",
                live.len(),
                last_by_name.len(),
                rows.len(),
                dup_alias_ids,
                repeated_names,
                missing.len(),
                wrong_winner.len(),
                extra.len()
            );
            assert!(
                missing.is_empty(),
                "live names missing from the dictionary: {:?}",
                missing.iter().take(10).collect::<Vec<_>>()
            );
            assert!(
                wrong_winner.is_empty(),
                "repeated names not resolved to their last row (name, want, got): {:?}",
                wrong_winner.iter().take(10).collect::<Vec<_>>()
            );
            assert!(
                extra.is_empty(),
                "rows that are neither channel nor global: {:?}",
                extra.iter().map(|e| &e.name).take(10).collect::<Vec<_>>()
            );
        });
    }

    #[test]
    fn ffz_modifier_emoticon_parses_flags_and_rides_zero_width() {
        let item = serde_json::json!({
            "id": 720508,
            "name": "ffzX",
            "modifier": true,
            "modifier_flags": 3,
            "urls": { "1": "https://cdn.frankerfacez.com/emote/720508/1" }
        });
        let e = EmoteService::parse_ffz_emoticon(&item, false).unwrap();
        assert_eq!(e.is_zero_width, Some(true));
        assert_eq!(e.modifier_flags, Some(3));
        assert_eq!(e.ffz_sub_only, None);
    }

    #[test]
    fn ffz_sub_only_set_marks_emote() {
        let item = serde_json::json!({
            "id": 720510,
            "name": "ffzRainbow",
            "modifier": true,
            "modifier_flags": 2049
        });
        let e = EmoteService::parse_ffz_emoticon(&item, true).unwrap();
        assert_eq!(e.ffz_sub_only, Some(true));
        assert_eq!(e.modifier_flags, Some(2049));
    }

    #[test]
    fn ffz_plain_emoticon_has_no_modifier_fields() {
        let item = serde_json::json!({
            "id": 1,
            "name": "PlainEmote",
            "modifier": false,
            "modifier_flags": 0
        });
        let e = EmoteService::parse_ffz_emoticon(&item, false).unwrap();
        assert_eq!(e.is_zero_width, None);
        assert_eq!(e.modifier_flags, None);
        assert_eq!(e.ffz_sub_only, None);
    }

    // The renderer hand-mirrors this bit table in TypeScript, so these assert
    // literal integers rather than the constants: a bit reassigned on one side
    // and not the other has to fail here, not in a live chat.
    #[test]
    fn bttv_prefix_modifier_parses_flags_and_rides_zero_width() {
        let item = serde_json::json!({
            "id": "64e3b31920cb0d25d950a9f9",
            "code": "w!",
            "imageType": "png",
            "animated": false,
            "modifier": true
        });
        let e = EmoteService::parse_bttv_emote(&item).unwrap();
        assert_eq!(e.is_zero_width, Some(true));
        // Hidden | Prefix | BttvWide
        assert_eq!(e.modifier_flags, Some(786433));
    }

    #[test]
    fn bttv_modifiers_reusing_ffz_bits_match_them_exactly() {
        for (code, expected) in [
            ("h!", 1 | (1 << 18) | (1 << 1)),  // Hidden | Prefix | FlipX
            ("c!", 1 | (1 << 18) | (1 << 14)), // Hidden | Prefix | Cursed
            ("z!", 1 | (1 << 18) | (1 << 17)), // Hidden | Prefix | NoSpace
        ] {
            let item = serde_json::json!({ "id": "x", "code": code, "modifier": true });
            let e = EmoteService::parse_bttv_emote(&item).unwrap();
            assert_eq!(e.modifier_flags, Some(expected), "code {}", code);
        }
    }

    #[test]
    fn bttv_overlay_emote_is_zero_width_without_flags() {
        let item = serde_json::json!({
            "id": "5e76d399d6581c3724c0f0b8",
            "code": "cvMask",
            "modifier": false
        });
        let e = EmoteService::parse_bttv_emote(&item).unwrap();
        assert_eq!(e.is_zero_width, Some(true));
        assert_eq!(e.modifier_flags, None);
    }

    #[test]
    fn bttv_unknown_modifier_code_renders_as_a_plain_emote() {
        // BetterTTV renders a modifier missing from its own map as ordinary
        // art rather than guessing an effect, and so do we.
        let item = serde_json::json!({ "id": "abc", "code": "q!", "modifier": true });
        let e = EmoteService::parse_bttv_emote(&item).unwrap();
        assert_eq!(e.is_zero_width, None);
        assert_eq!(e.modifier_flags, None);
    }

    #[test]
    fn bttv_plain_emote_has_no_modifier_fields() {
        let item = serde_json::json!({ "id": "def", "code": "Kappa", "modifier": false });
        let e = EmoteService::parse_bttv_emote(&item).unwrap();
        assert_eq!(e.is_zero_width, None);
        assert_eq!(e.modifier_flags, None);
        assert_eq!(e.ffz_sub_only, None);
    }
}

#[cfg(test)]
mod broadcaster_id_tests {
    use super::twitch_broadcaster_id;

    #[test]
    fn keeps_a_numeric_twitch_id() {
        assert_eq!(twitch_broadcaster_id(true, Some("71092938")), Some("71092938"));
    }

    #[test]
    fn drops_ids_helix_would_reject() {
        // A YouTube channel id under a caller that forgot to pass `provider`:
        // the flag says Twitch, the value says otherwise, and the value wins.
        assert_eq!(twitch_broadcaster_id(true, Some("UChNWxrTlmh4IRSevon1X93g")), None);
        assert_eq!(twitch_broadcaster_id(true, Some("")), None);
        assert_eq!(twitch_broadcaster_id(true, None), None);
        // Correctly-flagged non-Twitch stays dropped even when it looks numeric.
        assert_eq!(twitch_broadcaster_id(false, Some("12345")), None);
    }
}
