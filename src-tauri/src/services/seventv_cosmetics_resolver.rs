//! Which 7TV paint and badge each chatter is wearing, resolved once for every
//! window.
//!
//! Every window used to run its own copy of this: its own batch queue, its own
//! 4,000-user cache and its own parse of the ~1 MB definitions catalog, so a
//! main window plus two MultiChat popouts asked 7TV three times about the same
//! chatters. Requests from all windows now land in one queue, share one cache,
//! and resolve against one catalog.
//!
//! The shape of a lookup:
//! - Definitions (every paint and badge 7TV has) are a small, shared, public
//!   set, fetched once and kept on disk for a day. Measured 2026-08-29 at 1013
//!   paints + 127 badges, ~1 MB, one request.
//! - Per chatter we ask only which cosmetic ids they are WEARING. A chatter's
//!   whole inventory would carry a definition for everything they own, and that
//!   weight is what used to pin the batch at five users per query.
//! - Chatters arriving within BATCH_WINDOW are multiplexed into one aliased
//!   GraphQL query. Live chat delivers one message at a time, so without the
//!   window every chatter paid a round-trip of their own.

use futures::future::{BoxFuture, FutureExt, Shared};
use futures::stream::{self, StreamExt};
use lru::LruCache;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Notify};

/// 7TV rejects a query past ~400 complexity with "Query is too complex." and
/// the WHOLE batch comes back null. Re-measured against live 7TV on 2026-08-29
/// with the active-only selection: 40 passed, 45 was rejected. 30 keeps a
/// quarter of that headroom. Do not raise without re-measuring, and do not put
/// definitions back into the per-user selection without lowering it again.
const BATCH_MAX_SIZE: usize = 30;
/// In-flight chunks at once: 150 users in flight is plenty, and polite to 7TV
/// on a cold-start burst (a hype-train channel join can queue 40+ chunks).
const MAX_PARALLEL_CHUNKS: usize = 5;
/// How long ids accumulate before a drain. Cosmetics paint onto a row that has
/// already rendered, so this is invisible to the reader.
const BATCH_WINDOW: Duration = Duration::from_millis(150);
/// One full parallel wave is already worth sending without waiting.
const BATCH_DRAIN_THRESHOLD: usize = MAX_PARALLEL_CHUNKS * BATCH_MAX_SIZE;

const USER_TTL: Duration = Duration::from_secs(5 * 60);
/// A hard failure (network, 5xx, rejected batch) is retried soon, so a 7TV blip
/// cannot strand a real user without their paint for the full TTL.
const HARD_FAIL_TTL: Duration = Duration::from_secs(30);
/// Bounded because the TTL decides FRESHNESS, not residency: an expired entry
/// is refetched on read anyway, so evicting a cold one discards nothing a read
/// would have used.
#[cfg(mobile)]
const USER_CACHE_CAPACITY: usize = 1500;
#[cfg(not(mobile))]
const USER_CACHE_CAPACITY: usize = 4000;

const CATALOG_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const CATALOG_FILE: &str = "seventv_cosmetic_catalog.json";

/// Opening a profile card asks for the same inventory twice at once (the card's
/// Rust profile and its cosmetics section); one request answers both, and a
/// reopen within this window reuses it.
const INVENTORY_TTL: Duration = Duration::from_secs(60);

const GQL_ATTEMPTS: usize = 6;
const GQL_RETRY_DELAY: Duration = Duration::from_millis(500);

const PAINT_FIELDS: &str = "{ id name description data { layers { id ty { ... on PaintLayerTypeImage { __typename images { __typename url mime size scale width height frameCount } } ... on PaintLayerTypeRadialGradient { __typename repeating shape stops { at color { __typename hex r g b a } } } ... on PaintLayerTypeLinearGradient { __typename angle repeating stops { __typename at color { __typename hex r g b a } } } ... on PaintLayerTypeSingleColor { __typename color { __typename hex r g b a } } } opacity } shadows { __typename offsetX offsetY blur color { __typename hex r g b a } } } }";
const BADGE_FIELDS: &str = "{ id name description images { url mime scale frameCount } }";
/// What chat needs: which cosmetics a user is WEARING. Definitions come from
/// the catalog.
const ACTIVE_ONLY_SELECTION: &str = "{ id style { activePaint { id } activeBadge { id description } } }";

/// A chatter's cosmetics in the shape the renderer reads: the worn paint and
/// badge (each `selected: true`) and their 7TV account id. An inventory lookup
/// fills the lists with everything owned instead.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct UserCosmetics {
    pub paints: Vec<Value>,
    pub badges: Vec<Value>,
    #[serde(rename = "seventvUserId", skip_serializing_if = "Option::is_none")]
    pub seventv_user_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CosmeticsLookup {
    pub data: UserCosmetics,
    /// True when 7TV never gave a real answer, as opposed to "this user has no
    /// cosmetics". Callers short-TTL their own caches on it.
    #[serde(rename = "hardFail")]
    pub hard_fail: bool,
}

// ---- Catalog ----------------------------------------------------------------

#[derive(Default)]
struct Catalog {
    paints: HashMap<String, Value>,
    badges: HashMap<String, Value>,
    fetched_at_ms: i64,
}

static CATALOG: RwLock<Option<Catalog>> = RwLock::new(None);
/// Single-flight for the first load, so a burst of drains fetches it once.
static CATALOG_LOAD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static CATALOG_REFRESHING: AtomicBool = AtomicBool::new(false);

fn catalog_path() -> Option<std::path::PathBuf> {
    crate::services::cache_service::get_app_data_dir()
        .ok()
        .map(|d| d.join(CATALOG_FILE))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn catalog_from(paints: &[Value], badges: &[Value], fetched_at_ms: i64) -> Catalog {
    let by_id = |items: &[Value]| {
        items
            .iter()
            .filter_map(|v| Some((v.get("id")?.as_str()?.to_string(), v.clone())))
            .collect::<HashMap<_, _>>()
    };
    Catalog { paints: by_id(paints), badges: by_id(badges), fetched_at_ms }
}

fn seed_catalog_from_disk() -> bool {
    let Some(raw) = catalog_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return false;
    };
    let Ok(stored) = serde_json::from_str::<Value>(&raw) else { return false };
    let paints = stored.get("paints").and_then(Value::as_array).cloned().unwrap_or_default();
    if paints.is_empty() {
        return false;
    }
    let badges = stored.get("badges").and_then(Value::as_array).cloned().unwrap_or_default();
    let at = stored.get("at").and_then(Value::as_i64).unwrap_or(0);
    if let Ok(mut slot) = CATALOG.write() {
        *slot = Some(catalog_from(&paints, &badges, at));
    }
    true
}

fn catalog_fresh() -> bool {
    CATALOG
        .read()
        .ok()
        .and_then(|c| c.as_ref().map(|c| now_ms() - c.fetched_at_ms < CATALOG_TTL_MS))
        .unwrap_or(false)
}

async fn fetch_catalog() {
    let query = format!("{{ paints {{ paints {PAINT_FIELDS} }} badges {{ badges {BADGE_FIELDS} }} }}");
    let Some(data) = request_gql(&query).await else {
        log::warn!("[7TV] cosmetic catalog fetch failed");
        return;
    };
    let paints = data.pointer("/paints/paints").and_then(Value::as_array).cloned().unwrap_or_default();
    // A degraded response must never empty a catalog we already have: a chatter
    // resolving against nothing renders with no paint at all.
    if paints.is_empty() {
        return;
    }
    let badges = data.pointer("/badges/badges").and_then(Value::as_array).cloned().unwrap_or_default();
    let at = now_ms();
    // Loud on purpose: once per launch, or once a day. A repeat means an id we
    // keep failing to find is driving a refetch loop.
    log::info!("[7TV] cosmetic catalog fetched: {} paints, {} badges", paints.len(), badges.len());
    if let Ok(mut slot) = CATALOG.write() {
        let keep_badges = badges.is_empty();
        let mut next = catalog_from(&paints, &badges, at);
        if keep_badges {
            if let Some(old) = slot.take() {
                next.badges = old.badges;
            }
        }
        *slot = Some(next);
    }
    if let Some(path) = catalog_path() {
        let body = json!({ "at": at, "paints": paints, "badges": badges }).to_string();
        let _ = tokio::task::spawn_blocking(move || std::fs::write(path, body)).await;
    }
}

/// Definitions in hand before any id is resolved. Free on a disk-seeded launch.
async fn ensure_catalog() {
    if catalog_fresh() {
        return;
    }
    let _guard = CATALOG_LOAD.lock().await;
    let empty = CATALOG.read().map(|c| c.is_none()).unwrap_or(true);
    if empty && seed_catalog_from_disk() && catalog_fresh() {
        return;
    }
    if !catalog_fresh() {
        fetch_catalog().await;
    }
}

/// An id we have never seen means 7TV shipped a cosmetic since our snapshot.
fn refresh_catalog_for_unknown(id: &str) {
    if CATALOG_REFRESHING.swap(true, Ordering::AcqRel) {
        return;
    }
    log::info!("[7TV] cosmetic id {id} missing from catalog; refreshing");
    tauri::async_runtime::spawn(async {
        fetch_catalog().await;
        CATALOG_REFRESHING.store(false, Ordering::Release);
    });
}

// ---- GraphQL ------------------------------------------------------------------

/// The query's `data`, or None once every attempt failed (network error, an
/// errors array, or a message instead of data).
async fn request_gql(query: &str) -> Option<Value> {
    for attempt in 0..GQL_ATTEMPTS {
        match crate::commands::seventv::seventv_graphql(query.to_string()).await {
            Ok(resp) if resp.errors.is_none() && resp.message.is_none() => return resp.data,
            Ok(resp) => log::warn!(
                "[7TV-diag] gql error (attempt {attempt}): {:?}",
                resp.message.or_else(|| resp.errors.map(|e| format!("{e:?}")))
            ),
            Err(e) => log::debug!("[7TV] gql request failed (attempt {attempt}): {e}"),
        }
        if attempt + 1 < GQL_ATTEMPTS {
            tokio::time::sleep(GQL_RETRY_DELAY).await;
        }
    }
    None
}

/// 7TV's platform name and id for a cosmetics id. Twitch ids are bare numbers;
/// other platforms are namespaced (`kick:123`, `youtube:UC...`). 7TV calls
/// YouTube GOOGLE: sending YOUTUBE fails the whole query.
fn platform_of(id: &str) -> (&'static str, &str) {
    if let Some(rest) = id.strip_prefix("kick:") {
        ("KICK", rest)
    } else if let Some(rest) = id.strip_prefix("youtube:") {
        ("GOOGLE", rest)
    } else {
        ("TWITCH", id)
    }
}

/// GraphQL aliases must match /[_A-Za-z][_0-9A-Za-z]*/.
fn alias_of(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    format!("u_{safe}")
}

fn user_query(ids: &[String], selection: &str) -> String {
    let parts: Vec<String> = ids
        .iter()
        .map(|id| {
            let (platform, platform_id) = platform_of(id);
            let platform_id = platform_id.replace('\\', "").replace('"', "");
            format!(
                "{}: users {{ userByConnection(platform: {platform}, platformId: \"{platform_id}\") {selection} }}",
                alias_of(id)
            )
        })
        .collect();
    format!("{{ {} }}", parts.join(" "))
}

// ---- Turning a 7TV user into cosmetics ------------------------------------------

fn selected(mut definition: Value) -> Value {
    if let Some(obj) = definition.as_object_mut() {
        obj.insert("selected".into(), Value::Bool(true));
    }
    definition
}

/// A chatter's worn cosmetics, from their active ids plus the catalog. With no
/// definition the badge still renders from its CDN path, so an unknown badge
/// shows art rather than disappearing while the catalog refreshes.
fn worn_cosmetics(user: &Value) -> UserCosmetics {
    let mut out = UserCosmetics {
        seventv_user_id: user.get("id").and_then(Value::as_str).map(String::from),
        ..Default::default()
    };
    let catalog = CATALOG.read().ok();
    let catalog = catalog.as_ref().and_then(|c| c.as_ref());
    if let Some(paint_id) = user.pointer("/style/activePaint/id").and_then(Value::as_str) {
        match catalog.and_then(|c| c.paints.get(paint_id)) {
            Some(def) => out.paints.push(selected(def.clone())),
            None => refresh_catalog_for_unknown(paint_id),
        }
    }
    if let Some(badge) = user.pointer("/style/activeBadge").filter(|b| !b.is_null()) {
        if let Some(badge_id) = badge.get("id").and_then(Value::as_str) {
            let def = catalog.and_then(|c| c.badges.get(badge_id)).cloned();
            if def.is_none() {
                refresh_catalog_for_unknown(badge_id);
            }
            let def = def.unwrap_or_else(|| {
                json!({
                    "id": badge_id,
                    "name": "",
                    "description": badge.get("description").and_then(Value::as_str).unwrap_or_default(),
                    "images": [],
                })
            });
            out.badges.push(selected(def));
        }
    }
    out
}

/// Everything a user owns, from the full inventory selection, with the worn
/// ones marked selected.
fn owned_cosmetics(user: &Value) -> UserCosmetics {
    let active_paint = user.pointer("/style/activePaint/id").and_then(Value::as_str);
    let active_badge = user.pointer("/style/activeBadge/id").and_then(Value::as_str);
    let collect = |list: &str, field: &str, active: Option<&str>| -> Vec<Value> {
        user.pointer(list)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.pointer(&format!("/to/{field}")).filter(|v| !v.is_null()).cloned())
            .map(|def| {
                let is_active = def.get("id").and_then(Value::as_str) == active && active.is_some();
                if is_active { selected(def) } else { def }
            })
            .collect()
    };
    UserCosmetics {
        paints: collect("/inventory/paints", "paint", active_paint),
        badges: collect("/inventory/badges", "badge", active_badge),
        seventv_user_id: user.get("id").and_then(Value::as_str).map(String::from),
    }
}

// ---- Cache and batching -------------------------------------------------------

struct Cached {
    at: Instant,
    hard_fail: bool,
    data: UserCosmetics,
}

static USERS: Mutex<Option<LruCache<String, Cached>>> = Mutex::new(None);

fn cached(id: &str) -> Option<CosmeticsLookup> {
    let mut guard = USERS.lock().ok()?;
    let entry = guard.as_mut()?.get(id)?;
    let ttl = if entry.hard_fail { HARD_FAIL_TTL } else { USER_TTL };
    (entry.at.elapsed() < ttl).then(|| CosmeticsLookup { data: entry.data.clone(), hard_fail: entry.hard_fail })
}

fn remember(id: &str, lookup: &CosmeticsLookup) {
    if let Ok(mut guard) = USERS.lock() {
        guard
            .get_or_insert_with(|| {
                LruCache::new(NonZeroUsize::new(USER_CACHE_CAPACITY).expect("non-zero capacity"))
            })
            .put(
                id.to_string(),
                Cached { at: Instant::now(), hard_fail: lookup.hard_fail, data: lookup.data.clone() },
            );
    }
}

/// None = hard failure for that user's chunk.
type Waiter = oneshot::Sender<Option<UserCosmetics>>;

#[derive(Default)]
struct Queue {
    waiters: HashMap<String, Vec<Waiter>>,
    scheduled: bool,
}

static QUEUE: Mutex<Option<Queue>> = Mutex::new(None);
static DRAIN_NOW: Notify = Notify::const_new();

fn with_queue<R>(f: impl FnOnce(&mut Queue) -> R) -> R {
    let mut guard = QUEUE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Queue::default))
}

/// Queue one id and say whether a drain needs starting, and whether now.
fn enqueue(id: &str, waiter: Waiter) -> (bool, bool) {
    with_queue(|q| {
        q.waiters.entry(id.to_string()).or_default().push(waiter);
        let immediate = q.waiters.len() >= BATCH_DRAIN_THRESHOLD;
        let start = !q.scheduled;
        q.scheduled = true;
        (start, immediate)
    })
}

fn take_queue() -> HashMap<String, Vec<Waiter>> {
    with_queue(|q| {
        q.scheduled = false;
        std::mem::take(&mut q.waiters)
    })
}

fn chunks_of(ids: Vec<String>) -> Vec<Vec<String>> {
    ids.chunks(BATCH_MAX_SIZE).map(<[String]>::to_vec).collect()
}

/// Resolve one chunk: Some(result per id), or None for the whole chunk when 7TV
/// gave no data (a hard failure, never "these users have nothing").
async fn resolve_chunk(chunk: &[String]) -> Option<HashMap<String, UserCosmetics>> {
    let data = request_gql(&user_query(chunk, ACTIVE_ONLY_SELECTION)).await?;
    let mut resolved = 0;
    let out = chunk
        .iter()
        .map(|id| {
            let user = data.get(alias_of(id)).and_then(|u| u.get("userByConnection")).filter(|u| !u.is_null());
            if user.is_some() {
                resolved += 1;
            }
            (id.clone(), user.map(worn_cosmetics).unwrap_or_default())
        })
        .collect();
    log::debug!("[7TV-diag] chunk resolved {resolved}/{} user(s) with a 7TV connection", chunk.len());
    Some(out)
}

async fn drain() {
    ensure_catalog().await;
    let mut waiters = take_queue();
    if waiters.is_empty() {
        return;
    }
    let ids: Vec<String> = waiters.keys().cloned().collect();
    let results: Vec<(Vec<String>, Option<HashMap<String, UserCosmetics>>)> = stream::iter(chunks_of(ids))
        .map(|chunk| async move {
            let resolved = resolve_chunk(&chunk).await;
            (chunk, resolved)
        })
        .buffer_unordered(MAX_PARALLEL_CHUNKS)
        .collect()
        .await;
    for (chunk, resolved) in results {
        for id in chunk {
            let answer = resolved.as_ref().map(|r| r.get(&id).cloned().unwrap_or_default());
            for waiter in waiters.remove(&id).unwrap_or_default() {
                let _ = waiter.send(answer.clone());
            }
        }
    }
}

fn start_drain(immediate: bool) {
    tauri::async_runtime::spawn(async move {
        if !immediate {
            tokio::select! {
                _ = tokio::time::sleep(BATCH_WINDOW) => {}
                _ = DRAIN_NOW.notified() => {}
            }
        }
        drain().await;
    });
}

/// What one chatter is wearing. Cached for every window; concurrent asks for
/// the same chatter share one lookup.
pub async fn resolve(id: String) -> CosmeticsLookup {
    if let Some(hit) = cached(&id) {
        return hit;
    }
    let (tx, rx) = oneshot::channel();
    match enqueue(&id, tx) {
        (true, immediate) => start_drain(immediate),
        (false, true) => DRAIN_NOW.notify_one(),
        (false, false) => {}
    }
    let lookup = match rx.await.ok().flatten() {
        Some(data) => CosmeticsLookup { data, hard_fail: false },
        None => CosmeticsLookup { data: UserCosmetics::default(), hard_fail: true },
    };
    remember(&id, &lookup);
    lookup
}

/// Drop one chatter from the cache so the next lookup asks 7TV again.
pub fn invalidate(id: &str) {
    if let Ok(mut guard) = USERS.lock() {
        if let Some(lru) = guard.as_mut() {
            lru.pop(id);
        }
    }
    if let Ok(mut guard) = INVENTORIES.lock() {
        if let Some(map) = guard.as_mut() {
            map.remove(id);
        }
    }
}

pub fn clear() {
    if let Ok(mut guard) = USERS.lock() {
        *guard = None;
    }
    if let Ok(mut guard) = INVENTORIES.lock() {
        *guard = None;
    }
}

/// Everything an account OWNS, and the 7TV avatar it has set.
#[derive(Debug, Clone, Default)]
pub struct Inventory {
    pub cosmetics: UserCosmetics,
    /// `style.activeProfilePicture.images`; empty when no 7TV avatar is set.
    pub avatar_images: Vec<Value>,
}

type InventoryFlight = Shared<BoxFuture<'static, Option<Inventory>>>;

static INVENTORIES: Mutex<Option<HashMap<String, (Instant, InventoryFlight)>>> = Mutex::new(None);

async fn fetch_inventory(id: String) -> Option<Inventory> {
    let selection = format!(
        "{{ id style {{ activePaint {{ id }} activeBadge {{ id description }} activeProfilePicture {{ images {{ url mime scale frameCount }} }} }} inventory {{ paints {{ to {{ paint {PAINT_FIELDS} }} }} badges {{ to {{ badge {BADGE_FIELDS} }} }} }} }}"
    );
    let data = request_gql(&user_query(std::slice::from_ref(&id), &selection)).await?;
    let user = data.get(alias_of(&id))?.get("userByConnection").filter(|u| !u.is_null())?;
    Some(Inventory {
        cosmetics: owned_cosmetics(user),
        avatar_images: user
            .pointer("/style/activeProfilePicture/images")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    })
}

/// Everything an account OWNS. Only the profile card, the cosmetics picker and
/// the attainables view need this, one account at a time, so the heavy query is
/// affordable exactly there and nowhere else. Concurrent asks share one request.
pub async fn owned(id: &str) -> Option<Inventory> {
    let flight = {
        let mut guard = INVENTORIES.lock().unwrap_or_else(|e| e.into_inner());
        let map = guard.get_or_insert_with(HashMap::new);
        map.retain(|_, (at, _)| at.elapsed() < INVENTORY_TTL);
        map.entry(id.to_string())
            .or_insert_with(|| (Instant::now(), fetch_inventory(id.to_string()).boxed().shared()))
            .1
            .clone()
    };
    let answer = flight.await;
    if answer.is_none() {
        // A failure is not an answer to keep: the next open asks again.
        if let Ok(mut guard) = INVENTORIES.lock() {
            if let Some(map) = guard.as_mut() {
                map.remove(id);
            }
        }
    }
    answer
}

/// The owned cosmetics alone, as the picker reads them.
pub async fn inventory(id: &str) -> Option<UserCosmetics> {
    owned(id).await.map(|i| i.cosmetics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize, from: usize) -> Vec<String> {
        (from..from + n).map(|i| i.to_string()).collect()
    }

    #[test]
    fn the_per_user_query_asks_only_what_is_worn() {
        let q = user_query(&["111".to_string()], ACTIVE_ONLY_SELECTION);
        assert!(!q.contains("inventory"));
        assert!(q.contains("activePaint") && q.contains("activeBadge"));
        assert!(q.contains("u_111: users { userByConnection(platform: TWITCH, platformId: \"111\")"));
    }

    #[test]
    fn a_25_user_burst_fits_one_query_and_65_is_chunked_at_30() {
        assert_eq!(chunks_of(ids(25, 1000)).len(), 1);
        let chunks = chunks_of(ids(65, 2000));
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.len() <= BATCH_MAX_SIZE));
        assert_eq!(chunks.concat().last().map(String::as_str), Some("2064"));
    }

    #[test]
    fn platforms_and_aliases_follow_7tv_rules() {
        assert_eq!(platform_of("kick:123"), ("KICK", "123"));
        assert_eq!(platform_of("youtube:UCabc"), ("GOOGLE", "UCabc"));
        assert_eq!(platform_of("42"), ("TWITCH", "42"));
        assert_eq!(alias_of("kick:12-3"), "u_kick_12_3");
    }

    #[test]
    fn chatters_queued_apart_share_one_drain() {
        let (a, _ra) = oneshot::channel();
        let (b, _rb) = oneshot::channel();
        let (c, _rc) = oneshot::channel();
        // The first ask starts a drain; a second chatter and a repeat of the
        // first ride it instead of starting their own.
        assert_eq!(enqueue("111", a), (true, false));
        assert_eq!(enqueue("222", b), (false, false));
        assert_eq!(enqueue("111", c), (false, false));
        let taken = take_queue();
        assert_eq!(taken.len(), 2);
        assert_eq!(taken["111"].len(), 2);
        // Once drained, the next ask starts a fresh one.
        let (d, _rd) = oneshot::channel();
        assert_eq!(enqueue("333", d), (true, false));
        take_queue();
    }

    #[test]
    fn worn_cosmetics_come_from_the_catalog_and_are_marked_selected() {
        if let Ok(mut slot) = CATALOG.write() {
            *slot = Some(catalog_from(
                &[json!({ "id": "p1", "name": "Sunset" })],
                &[json!({ "id": "b1", "name": "Sub", "images": [] })],
                now_ms(),
            ));
        }
        let user = json!({ "id": "7tv-user", "style": {
            "activePaint": { "id": "p1" }, "activeBadge": { "id": "b1", "description": "d" } } });
        let worn = worn_cosmetics(&user);
        assert_eq!(worn.seventv_user_id.as_deref(), Some("7tv-user"));
        assert_eq!(worn.paints, vec![json!({ "id": "p1", "name": "Sunset", "selected": true })]);
        assert_eq!(worn.badges[0]["selected"], true);
        // The shared definition itself is not marked.
        assert!(CATALOG.read().unwrap().as_ref().unwrap().paints["p1"].get("selected").is_none());
    }

    #[test]
    fn an_owned_list_marks_only_the_worn_ones() {
        let user = json!({ "id": "u", "style": { "activePaint": { "id": "p2" }, "activeBadge": null },
            "inventory": {
                "paints": [ { "to": { "paint": { "id": "p1" } } }, { "to": { "paint": { "id": "p2" } } } ],
                "badges": [ { "to": { "badge": { "id": "b1" } } }, { "to": { "badge": null } } ] } });
        let owned = owned_cosmetics(&user);
        assert_eq!(owned.paints.len(), 2);
        assert!(owned.paints[0].get("selected").is_none());
        assert_eq!(owned.paints[1]["selected"], true);
        assert_eq!(owned.badges.len(), 1);
        assert!(owned.badges[0].get("selected").is_none());
    }
}
