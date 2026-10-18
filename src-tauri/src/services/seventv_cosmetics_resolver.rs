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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Notify};

/// 7TV rejects a query past ~400 complexity with "Query is too complex." and
/// the WHOLE batch comes back null. The analyzer reports 13 per user for the
/// selection below (12 before the profile picture id was added), so 25 users
/// is ~325. Do not raise without re-measuring, and do not put definitions back
/// into the per-user selection without lowering it again: the picture's image
/// list alone would take it to 18 per user, 450 for a full batch.
const BATCH_MAX_SIZE: usize = 25;
/// The picture lookup's selection measures 10 per user, so a full batch of it is
/// ~250.
const PICTURE_BATCH_MAX_SIZE: usize = 25;
/// Most ids one `profile_pictures` call resolves; a list surface never shows more.
const PICTURE_LOOKUP_MAX: usize = 200;
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
/// What chat needs: which cosmetics a user is WEARING (definitions come from
/// the catalog), plus their roles and personal emote set id, so a subscriber's
/// personal emotes are known even when 7TV never announced them. The profile
/// picture comes as an id only; its images are looked up once per picture id.
const ACTIVE_ONLY_SELECTION: &str = "{ id style { activePaint { id } activeBadge { id description } activeProfilePictureId } roleIds personalEmoteSet { id } }";
/// A worn profile picture's images, for the users whose picture id is new to us.
const PICTURE_SELECTION: &str = "{ style { activeProfilePicture { id images { url mime scale frameCount } } } }";

/// 7TV's Subscriber role. Roles are public where entitlements are not, so this
/// is the public sign that a chatter's personal set is live.
const SUBSCRIBER_ROLE_ID: &str = "01F37R3RFR0000K96678WEQT01";

/// A chatter's cosmetics in the shape the renderer reads: the worn paint and
/// badge (each `selected: true`) and their 7TV account id. An inventory lookup
/// fills the lists with everything owned instead.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct UserCosmetics {
    pub paints: Vec<Value>,
    pub badges: Vec<Value>,
    #[serde(rename = "seventvUserId", skip_serializing_if = "Option::is_none")]
    pub seventv_user_id: Option<String>,
    /// The profile picture this user set on 7TV, if any.
    #[serde(rename = "profilePicture", skip_serializing_if = "Option::is_none")]
    pub profile_picture: Option<ProfilePicture>,
    /// The worn picture's id, kept until its images are filled in.
    #[serde(skip)]
    pub(crate) picture_id: Option<String>,
    /// A Twitch user's live 7TV personal emote set, handed to the personal
    /// emote lane when this person is looked up as a chatter.
    #[serde(skip)]
    pub(crate) personal_set: Option<String>,
}

/// One rendition of a profile picture. 7TV's 1x is 32 px square.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PictureImage {
    pub url: String,
    pub scale: u8,
}

/// A 7TV profile picture, reduced to what a renderer chooses between: one
/// still and (when it moves) one animated rendition per scale, smallest first,
/// each in the best format 7TV offers at that scale (webp, then avif, png, gif).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProfilePicture {
    pub id: String,
    /// Single-frame renditions. For a moving picture these are 7TV's `_static`
    /// first frames, which is what a dense list should draw.
    pub still: Vec<PictureImage>,
    /// Multi-frame renditions; empty when the picture does not move.
    pub animated: Vec<PictureImage>,
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
    // A copy of 7TV's public catalog: with a separate cache dir (iOS) it goes
    // there, so it is never backed up.
    if crate::services::app_paths::cache_base().is_some() {
        return crate::services::cache_service::get_cache_dir().ok().map(|d| d.join(CATALOG_FILE));
    }
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
    out.picture_id = user.pointer("/style/activeProfilePictureId").and_then(Value::as_str).map(String::from);
    out.profile_picture = out.picture_id.as_deref().and_then(known_picture).flatten();
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
    let picture = user.pointer("/style/activeProfilePicture").and_then(profile_picture_from);
    UserCosmetics {
        paints: collect("/inventory/paints", "paint", active_paint),
        badges: collect("/inventory/badges", "badge", active_badge),
        seventv_user_id: user.get("id").and_then(Value::as_str).map(String::from),
        picture_id: picture.as_ref().map(|p| p.id.clone()),
        profile_picture: picture,
        personal_set: None,
    }
}

// ---- Profile pictures -----------------------------------------------------------

/// Best format first: every renderer here decodes webp, and it is the smallest
/// of the animated formats 7TV offers.
fn format_rank(mime: &str) -> u8 {
    match mime {
        m if m.contains("webp") => 0,
        m if m.contains("avif") => 1,
        m if m.contains("png") => 2,
        m if m.contains("gif") => 3,
        _ => 4,
    }
}

/// `style.activeProfilePicture` reduced to a still and an animated rendition
/// per scale. None when 7TV gave no picture or no usable image.
fn profile_picture_from(picture: &Value) -> Option<ProfilePicture> {
    let id = picture.get("id")?.as_str()?.to_string();
    // (animated, scale) -> (format rank, url)
    let mut best: HashMap<(bool, u8), (u8, String)> = HashMap::new();
    for image in picture.get("images").and_then(Value::as_array).into_iter().flatten() {
        let Some(url) = image.get("url").and_then(Value::as_str).filter(|u| !u.is_empty()) else { continue };
        let url = match url.strip_prefix("//") {
            Some(rest) => format!("https://{rest}"),
            None => url.to_string(),
        };
        let scale = image.get("scale").and_then(Value::as_u64).unwrap_or(1).clamp(1, 4) as u8;
        let animated = image.get("frameCount").and_then(Value::as_u64).unwrap_or(1) > 1;
        let rank = format_rank(image.get("mime").and_then(Value::as_str).unwrap_or_default());
        let slot = best.entry((animated, scale)).or_insert((u8::MAX, String::new()));
        if rank < slot.0 {
            *slot = (rank, url);
        }
    }
    let renditions = |animated: bool| -> Vec<PictureImage> {
        let mut list: Vec<PictureImage> = best
            .iter()
            .filter(|((a, _), _)| *a == animated)
            .map(|((_, scale), (_, url))| PictureImage { url: url.clone(), scale: *scale })
            .collect();
        list.sort_by_key(|i| i.scale);
        list
    };
    let (still, animated) = (renditions(false), renditions(true));
    if still.is_empty() && animated.is_empty() {
        return None;
    }
    Some(ProfilePicture { id, still, animated })
}

/// Pictures by picture id. A picture id names one upload, which 7TV never
/// changes, so an entry stays valid until evicted; a user who picks another
/// picture arrives with another id.
static PICTURES: Mutex<Option<LruCache<String, Option<ProfilePicture>>>> = Mutex::new(None);

/// Some(None) = 7TV said this id has no picture; None = not asked yet.
fn known_picture(picture_id: &str) -> Option<Option<ProfilePicture>> {
    PICTURES.lock().ok()?.as_mut()?.get(picture_id).cloned()
}

fn remember_picture(picture_id: &str, picture: Option<ProfilePicture>) {
    if let Ok(mut guard) = PICTURES.lock() {
        guard
            .get_or_insert_with(|| {
                LruCache::new(NonZeroUsize::new(USER_CACHE_CAPACITY).expect("non-zero capacity"))
            })
            .put(picture_id.to_string(), picture);
    }
}

/// Whether the viewer shows 7TV profile pictures (Settings > Chat > Profile
/// pictures). On the platform source no picture image is ever looked up; the
/// worn picture id still rides the cosmetics query, which costs nothing extra.
fn pictures_shown() -> bool {
    crate::services::providers::setting(|s| s.cosmetics.shows_seventv_pictures()).unwrap_or(true)
}

/// Emitted with `{ "<cosmetics id>": ProfilePicture }` when worn pictures'
/// images arrive after the cosmetics answer that named them, so every window
/// can draw them without asking again.
pub const PICTURES_EVENT: &str = "seventv-profile-pictures";

type PictureRead = Shared<BoxFuture<'static, ()>>;

/// Picture lookups in flight, by picture id, so a list surface waiting on a
/// picture joins the lookup a chat batch already started.
static PICTURE_READS: Mutex<Option<HashMap<String, PictureRead>>> = Mutex::new(None);

/// A worn picture whose images are not known yet: (cosmetics id, picture id).
fn unfilled(id: &str, user: &UserCosmetics) -> Option<(String, String)> {
    let picture_id = user.picture_id.as_ref()?;
    (user.profile_picture.is_none() && known_picture(picture_id).is_none())
        .then(|| (id.to_string(), picture_id.clone()))
}

/// Look up the images of worn pictures we have not seen yet, joining lookups
/// already running for the same pictures, and return once all have answered.
/// Each lookup runs on its own task, so it finishes whoever stops waiting. A
/// failed lookup caches nothing, and the user's cached cosmetics then live only
/// `HARD_FAIL_TTL`, so the next resolve asks again.
async fn fill_pictures(pairs: Vec<(String, String)>) {
    let mut waits: Vec<PictureRead> = Vec::new();
    {
        let mut guard = PICTURE_READS.lock().unwrap_or_else(|e| e.into_inner());
        let reads = guard.get_or_insert_with(HashMap::new);
        let mut fresh: Vec<(String, String)> = Vec::new();
        for (id, picture_id) in pairs {
            if let Some(read) = reads.get(&picture_id) {
                waits.push(read.clone());
            } else if !fresh.iter().any(|(_, p)| *p == picture_id) {
                fresh.push((id, picture_id));
            }
        }
        if !fresh.is_empty() {
            let picture_ids: Vec<String> = fresh.iter().map(|(_, p)| p.clone()).collect();
            let done = picture_ids.clone();
            let job = read_pictures(fresh);
            let read: PictureRead = async move {
                job.await;
                if let Ok(mut guard) = PICTURE_READS.lock() {
                    if let Some(reads) = guard.as_mut() {
                        for p in &done {
                            reads.remove(p);
                        }
                    }
                }
            }
            .boxed()
            .shared();
            for p in picture_ids {
                reads.insert(p, read.clone());
            }
            waits.push(read.clone());
            tauri::async_runtime::spawn(read);
        }
    }
    futures::future::join_all(waits).await;
}

/// One picture lookup: ask 7TV for the worn picture of each user, remember the
/// images by picture id, and tell every window about the pictures found.
async fn read_pictures(pairs: Vec<(String, String)>) {
    let wanted: HashMap<String, String> = pairs.iter().cloned().collect();
    let ids: Vec<String> = pairs.into_iter().map(|(id, _)| id).collect();
    let answers: Vec<(Vec<String>, Option<Value>)> =
        stream::iter(ids.chunks(PICTURE_BATCH_MAX_SIZE).map(<[String]>::to_vec))
            .map(|chunk| async move {
                let data = request_gql(&user_query(&chunk, PICTURE_SELECTION)).await;
                (chunk, data)
            })
            .buffer_unordered(MAX_PARALLEL_CHUNKS)
            .collect()
            .await;
    let mut found: HashMap<String, ProfilePicture> = HashMap::new();
    for (chunk, data) in answers {
        let Some(data) = data else { continue };
        for id in chunk {
            let picture = data
                .get(alias_of(&id))
                .and_then(|u| u.pointer("/userByConnection/style/activeProfilePicture"))
                .and_then(profile_picture_from);
            if let Some(picture) = &picture {
                remember_picture(&picture.id, Some(picture.clone()));
            }
            // The id we asked about has nothing behind it (removed between the
            // two queries, or an upload without images).
            if let Some(asked) = wanted.get(&id) {
                match &picture {
                    Some(p) if p.id == *asked => {
                        found.insert(id.clone(), p.clone());
                    }
                    _ => remember_picture(asked, None),
                }
            }
        }
    }
    if !found.is_empty() {
        if let Some(app) = crate::services::providers::app_handle() {
            use tauri::Emitter;
            let _ = app.emit(PICTURES_EVENT, &found);
        }
    }
}

/// The personal emote set a chatter may use, when 7TV shows them as an active
/// subscriber. The EventAPI entitlement stays the authoritative signal; this
/// catches sets it never announced, such as a subscriber chatting from a client
/// that posts no 7TV presence, or one who arrived before we subscribed.
fn usable_personal_set(user: &Value) -> Option<&str> {
    let subscribed = user
        .get("roleIds")
        .and_then(Value::as_array)?
        .iter()
        .any(|role| role.as_str() == Some(SUBSCRIBER_ROLE_ID));
    if !subscribed {
        return None;
    }
    user.pointer("/personalEmoteSet/id").and_then(Value::as_str)
}

// ---- Cache and batching -------------------------------------------------------

struct Cached {
    at: Instant,
    hard_fail: bool,
    data: UserCosmetics,
    /// The personal emote lane has been told about this user's set. A list
    /// surface's lookup leaves it false, so the first chat lookup still tells it.
    personal_noted: bool,
}

static USERS: Mutex<Option<LruCache<String, Cached>>> = Mutex::new(None);

/// A cached lookup and whether this chat lookup is the one to tell the
/// personal emote lane. A worn picture whose images arrived since is attached
/// here; one whose images never arrived keeps the entry only `HARD_FAIL_TTL`,
/// so the next resolve asks again.
fn cached(id: &str, chatter: bool) -> Option<(CosmeticsLookup, bool)> {
    // Read before the cache lock: a picture nobody shows is never missing.
    let pictures_wanted = pictures_shown();
    let mut guard = USERS.lock().ok()?;
    let entry = guard.as_mut()?.get_mut(id)?;
    let mut picture_missing = false;
    if entry.data.profile_picture.is_none() {
        if let Some(picture_id) = entry.data.picture_id.as_deref() {
            match known_picture(picture_id) {
                Some(known) => entry.data.profile_picture = known,
                None => picture_missing = pictures_wanted,
            }
        }
    }
    let ttl = if entry.hard_fail || picture_missing { HARD_FAIL_TTL } else { USER_TTL };
    if entry.at.elapsed() >= ttl {
        return None;
    }
    let note = chatter && !entry.personal_noted;
    if note {
        entry.personal_noted = true;
    }
    Some((CosmeticsLookup { data: entry.data.clone(), hard_fail: entry.hard_fail }, note))
}

fn remember(id: &str, lookup: &CosmeticsLookup, personal_noted: bool) {
    if let Ok(mut guard) = USERS.lock() {
        guard
            .get_or_insert_with(|| {
                LruCache::new(NonZeroUsize::new(USER_CACHE_CAPACITY).expect("non-zero capacity"))
            })
            .put(
                id.to_string(),
                Cached { at: Instant::now(), hard_fail: lookup.hard_fail, data: lookup.data.clone(), personal_noted },
            );
    }
}

/// Hand a chatter's personal emote set to the personal emote lane. Personal
/// emotes are a Twitch feature; the bare id is the Twitch one.
fn note_personal_set(id: &str, data: &UserCosmetics) {
    if let (("TWITCH", twitch_id), Some(set_id)) = (platform_of(id), data.personal_set.as_deref()) {
        crate::services::seventv_eventapi::personal_set_seen(twitch_id, set_id);
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
            let mut cosmetics = user.map(worn_cosmetics).unwrap_or_default();
            // Personal emotes are a Twitch feature; the bare id is the Twitch
            // one. The set is handed on only when a chat lookup asked (`resolve`).
            if platform_of(id).0 == "TWITCH" {
                cosmetics.personal_set = user.and_then(usable_personal_set).map(String::from);
            }
            (id.clone(), cosmetics)
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
    // Worn pictures whose images are new to us. Paints and badges are answered
    // first; the images follow on their own task and reach the windows through
    // PICTURES_EVENT and the cache.
    let unfilled_pictures: Vec<(String, String)> = results
        .iter()
        .filter_map(|(_, r)| r.as_ref())
        .flatten()
        .filter_map(|(id, cosmetics)| unfilled(id, cosmetics))
        .collect();
    for (chunk, resolved) in results {
        for id in chunk {
            let answer = resolved.as_ref().map(|r| r.get(&id).cloned().unwrap_or_default());
            for waiter in waiters.remove(&id).unwrap_or_default() {
                let _ = waiter.send(answer.clone());
            }
        }
    }
    if !unfilled_pictures.is_empty() && pictures_shown() {
        tauri::async_runtime::spawn(fill_pictures(unfilled_pictures));
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

/// Puts a chatter's cosmetics in the cache as if 7TV had answered.
#[cfg(test)]
pub(crate) fn remember_for_test(id: &str, data: UserCosmetics) {
    remember(id, &CosmeticsLookup { data, hard_fail: false }, true);
}

/// What a chatter is wearing when the cache already knows, without asking
/// 7TV. A failed lookup reads as unknown so the caller asks again.
pub fn peek(id: &str) -> Option<UserCosmetics> {
    let data = {
        let guard = USERS.lock().ok()?;
        let entry = guard.as_ref()?.peek(id)?;
        let ttl = if entry.hard_fail { HARD_FAIL_TTL } else { USER_TTL };
        (!entry.hard_fail && entry.at.elapsed() < ttl).then(|| entry.data.clone())?
    };
    Some(with_pick(id, data, false))
}

// ---- The viewer's own picks ----------------------------------------------------

/// How long a paint or badge the viewer just chose wins over 7TV's answers.
/// 7TV's reads lag its own writes by seconds, so a re-read straight after a
/// change would bring the previous pick back.
const PICK_HOLD: Duration = Duration::from_secs(120);

/// A pick the viewer made: `Some(None)` took the item off.
#[derive(Clone, Debug)]
struct Pick {
    paint: Option<Option<String>>,
    badge: Option<Option<String>>,
    at: Instant,
}

static PICKS: Mutex<Option<HashMap<String, Pick>>> = Mutex::new(None);

/// Records the paint or badge the viewer just chose for their own account, so
/// every surface (their chat rows, header, the picker) shows it at once and a
/// lagging 7TV read cannot undo it. `None` leaves that slot as 7TV says.
pub fn note_own_pick(id: &str, paint: Option<Option<&str>>, badge: Option<Option<&str>>) {
    if let Ok(mut guard) = PICKS.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        let pick = map.entry(id.to_string()).or_insert(Pick { paint: None, badge: None, at: Instant::now() });
        if let Some(paint) = paint {
            pick.paint = Some(paint.map(String::from));
        }
        if let Some(badge) = badge {
            pick.badge = Some(badge.map(String::from));
        }
        pick.at = Instant::now();
    }
}

fn current_pick(id: &str) -> Option<Pick> {
    let mut guard = PICKS.lock().ok()?;
    let map = guard.as_mut()?;
    map.retain(|_, p| p.at.elapsed() < PICK_HOLD);
    map.get(id).cloned()
}

/// An answer with the viewer's recent pick laid over it. An inventory keeps
/// every item and moves the `selected` mark; a worn answer becomes the picked
/// definition alone (or nothing).
fn with_pick(id: &str, mut data: UserCosmetics, inventory: bool) -> UserCosmetics {
    let Some(pick) = current_pick(id) else { return data };
    let catalog = CATALOG.read().ok();
    let catalog = catalog.as_ref().and_then(|c| c.as_ref());
    let apply = |list: &mut Vec<Value>, chosen: &Option<String>, defs: Option<&HashMap<String, Value>>| {
        if inventory {
            for item in list.iter_mut() {
                let on = chosen.as_deref().is_some_and(|c| item.get("id").and_then(Value::as_str) == Some(c));
                if let Some(obj) = item.as_object_mut() {
                    if on {
                        obj.insert("selected".into(), Value::Bool(true));
                    } else {
                        obj.remove("selected");
                    }
                }
            }
        } else {
            let def = chosen.as_deref().and_then(|c| {
                list.iter().find(|v| v.get("id").and_then(Value::as_str) == Some(c)).cloned().or_else(|| defs?.get(c).cloned())
            });
            *list = def.map(selected).into_iter().collect();
        }
    };
    if let Some(paint) = &pick.paint {
        apply(&mut data.paints, paint, catalog.map(|c| &c.paints));
    }
    if let Some(badge) = &pick.badge {
        apply(&mut data.badges, badge, catalog.map(|c| &c.badges));
    }
    data
}

/// What one chatter is wearing. Cached for every window; concurrent asks for
/// the same chatter share one lookup.
pub async fn resolve(id: String) -> CosmeticsLookup {
    resolve_as(id, true).await
}

/// `chatter`: the person is in a chat this app shows, so their personal emote
/// set is fetched. A list surface (a streamer's avatar) asks with false and
/// gets the same batch and cache without that.
async fn resolve_as(id: String, chatter: bool) -> CosmeticsLookup {
    if let Some((hit, note)) = cached(&id, chatter) {
        if note {
            note_personal_set(&id, &hit.data);
        }
        return CosmeticsLookup { data: with_pick(&id, hit.data, false), hard_fail: hit.hard_fail };
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
    remember(&id, &lookup, chatter);
    if chatter {
        note_personal_set(&id, &lookup.data);
    }
    CosmeticsLookup { data: with_pick(&id, lookup.data, false), hard_fail: lookup.hard_fail }
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

/// Drop every cached profile picture; the next lookup asks 7TV again.
pub fn clear_pictures() {
    if let Ok(mut guard) = PICTURES.lock() {
        *guard = None;
    }
}

/// Everything an account OWNS, and the 7TV profile picture it has set (on
/// `cosmetics.profile_picture`).
#[derive(Debug, Clone, Default)]
pub struct Inventory {
    pub cosmetics: UserCosmetics,
}

type InventoryFlight = Shared<BoxFuture<'static, Option<Inventory>>>;

static INVENTORIES: Mutex<Option<HashMap<String, (Instant, InventoryFlight)>>> = Mutex::new(None);

async fn fetch_inventory(id: String) -> Option<Inventory> {
    let selection = format!(
        "{{ id style {{ activePaint {{ id }} activeBadge {{ id description }} activeProfilePicture {{ id images {{ url mime scale frameCount }} }} }} inventory {{ paints {{ to {{ paint {PAINT_FIELDS} }} }} badges {{ to {{ badge {BADGE_FIELDS} }} }} }} }}"
    );
    let data = request_gql(&user_query(std::slice::from_ref(&id), &selection)).await?;
    let user = data.get(alias_of(&id))?.get("userByConnection").filter(|u| !u.is_null())?;
    let cosmetics = owned_cosmetics(user);
    // The card's lookup already carries the images; the chat path reuses them.
    if let Some(picture) = &cosmetics.profile_picture {
        remember_picture(&picture.id, Some(picture.clone()));
    }
    Some(Inventory { cosmetics })
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

/// Whether 7TV can know this cosmetics id: a bare Twitch id, `kick:<id>` or
/// `youtube:UC...`. Anything else (TikTok, a login) would be sent to 7TV as a
/// Twitch id and could only match the wrong person or no one.
fn has_seventv_platform(id: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit());
    if let Some(rest) = id.strip_prefix("kick:") {
        digits(rest)
    } else if let Some(rest) = id.strip_prefix("youtube:") {
        rest.len() == 24
            && rest.starts_with("UC")
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    } else {
        digits(id)
    }
}

/// The cosmetics id 7TV knows a channel's owner by, from a stream row's
/// platform and user id: the bare Twitch id, `kick:<id>` or `youtube:UC...`.
/// None where 7TV has no such platform or the id is not the account id 7TV
/// keys on. Twin of the page's `sevenTvIdForStream`.
pub fn cosmetics_id(provider: &str, user_id: &str) -> Option<String> {
    let id = match provider {
        "" | "twitch" => user_id.to_string(),
        "kick" | "youtube" => format!("{provider}:{user_id}"),
        _ => return None,
    };
    has_seventv_platform(&id).then_some(id)
}

/// The 7TV profile picture of each id, through the same batch, cache and
/// lifetime as everything else a chatter wears, so a list of streamers costs
/// one IPC call and at most a few 7TV queries. Ids 7TV cannot know are left
/// out, and so are ids whose lookup failed (the caller asks again later);
/// `None` means 7TV answered and there is no picture.
///
/// These are streamers on a list, not chatters, so no personal emote set is
/// fetched for them. A worn picture whose images are not known yet is waited
/// on here (joining the lookup a chat batch may have started), because a list
/// surface keeps whatever this answers. Answers nothing, and asks 7TV
/// nothing, while the viewer shows platform pictures.
pub async fn profile_pictures(ids: Vec<String>) -> HashMap<String, Option<ProfilePicture>> {
    if !pictures_shown() {
        return HashMap::new();
    }
    let mut ids: Vec<String> = ids.into_iter().filter(|id| has_seventv_platform(id)).collect();
    ids.sort_unstable();
    ids.dedup();
    ids.truncate(PICTURE_LOOKUP_MAX);
    let lookups: Vec<(String, CosmeticsLookup)> = futures::future::join_all(ids.into_iter().map(|id| async move {
        let lookup = resolve_as(id.clone(), false).await;
        (id, lookup)
    }))
    .await
    .into_iter()
    .filter(|(_, lookup)| !lookup.hard_fail)
    .collect();
    let waiting: Vec<(String, String)> = lookups.iter().filter_map(|(id, l)| unfilled(id, &l.data)).collect();
    if !waiting.is_empty() {
        fill_pictures(waiting).await;
    }
    lookups
        .into_iter()
        .filter_map(|(id, lookup)| match (lookup.data.profile_picture, lookup.data.picture_id) {
            (Some(picture), _) => Some((id, Some(picture))),
            (None, None) => Some((id, None)),
            // Still unknown means the lookup failed: left out, asked again later.
            (None, Some(picture_id)) => known_picture(&picture_id).map(|known| (id, known)),
        })
        .collect()
}

/// The owned cosmetics alone, as the picker reads them, with the viewer's
/// recent pick marked.
pub async fn inventory(id: &str) -> Option<UserCosmetics> {
    owned(id).await.map(|i| with_pick(id, i.cosmetics, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize, from: usize) -> Vec<String> {
        (from..from + n).map(|i| i.to_string()).collect()
    }

    #[test]
    fn a_recent_own_pick_wins_over_a_lagging_answer() {
        let paints = vec![json!({ "id": "p1", "name": "One", "selected": true }), json!({ "id": "p2", "name": "Two" })];
        note_own_pick("pick-test", Some(Some("p2")), None);
        let inv = with_pick("pick-test", UserCosmetics { paints: paints.clone(), ..Default::default() }, true);
        assert!(inv.paints[0].get("selected").is_none());
        assert_eq!(inv.paints[1]["selected"], true);
        let worn = with_pick("pick-test", UserCosmetics { paints: paints.clone(), ..Default::default() }, false);
        assert_eq!(worn.paints.len(), 1);
        assert_eq!(worn.paints[0]["id"], "p2");
        note_own_pick("pick-test", Some(None), None);
        let off = with_pick("pick-test", UserCosmetics { paints, ..Default::default() }, false);
        assert!(off.paints.is_empty());
    }

    #[test]
    fn the_per_user_query_asks_only_what_is_worn() {
        let q = user_query(&["111".to_string()], ACTIVE_ONLY_SELECTION);
        assert!(!q.contains("inventory"));
        assert!(q.contains("activePaint") && q.contains("activeBadge"));
        assert!(q.contains("u_111: users { userByConnection(platform: TWITCH, platformId: \"111\")"));
        assert!(q.contains("roleIds personalEmoteSet { id }"));
    }

    #[test]
    fn a_personal_set_counts_only_for_an_active_subscriber() {
        let subscriber = json!({
            "roleIds": ["01G68MMQFR0007J6GNM9E2M0TM", SUBSCRIBER_ROLE_ID],
            "personalEmoteSet": { "id": "set1" }
        });
        assert_eq!(usable_personal_set(&subscriber), Some("set1"));
        // Every account has a personal set object; without the role it is not live.
        let lapsed = json!({ "roleIds": ["01G68MMQFR0007J6GNM9E2M0TM"], "personalEmoteSet": { "id": "set2" } });
        assert_eq!(usable_personal_set(&lapsed), None);
        let no_set = json!({ "roleIds": [SUBSCRIBER_ROLE_ID], "personalEmoteSet": null });
        assert_eq!(usable_personal_set(&no_set), None);
    }

    #[test]
    fn a_25_user_burst_fits_one_query_and_65_is_chunked_at_the_cap() {
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

    fn fixture() -> Value {
        serde_json::from_str(include_str!("fixtures/seventv_profile_pictures.json")).expect("fixture parses")
    }

    fn fixture_picture(data: &Value, alias: &str) -> Option<ProfilePicture> {
        data.pointer(&format!("/pictures/{alias}/userByConnection/style/activeProfilePicture"))
            .and_then(profile_picture_from)
    }

    #[test]
    fn the_per_user_query_asks_for_the_worn_picture_id_only() {
        let q = user_query(&["111".to_string()], ACTIVE_ONLY_SELECTION);
        assert!(q.contains("activeProfilePictureId"));
        assert!(!q.contains("images"), "image lists would push a full batch past 7TV's complexity cap");
        let q = user_query(&["kick:5".to_string()], PICTURE_SELECTION);
        assert!(q.contains("platform: KICK") && q.contains("activeProfilePicture { id images"));
    }

    #[test]
    fn an_animated_picture_keeps_a_still_and_a_moving_webp_per_scale() {
        let pic = fixture_picture(&fixture(), "u_64282485").expect("animated picture parses");
        assert_eq!(pic.id, "01JJCJQ9HDR7HH3PH3APTW9SS7");
        assert_eq!(pic.still.iter().map(|i| i.scale).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert_eq!(pic.animated.iter().map(|i| i.scale).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
        assert!(pic.still.iter().all(|i| i.url.ends_with("_static.webp")), "{:?}", pic.still);
        assert!(pic.animated.iter().all(|i| i.url.ends_with("x.webp")));
        assert_eq!(
            pic.animated[0].url,
            "https://cdn.7tv.app/user/01FF3DRSGG000FF5VVCKV49EA1/profile-picture/01JJCJQ9HDR7HH3PH3APTW9SS7/1x.webp"
        );
    }

    #[test]
    fn a_still_picture_has_no_animated_renditions() {
        let pic = fixture_picture(&fixture(), "u_129272570").expect("still picture parses");
        assert_eq!(pic.still.len(), 4);
        assert!(pic.animated.is_empty());
        assert!(pic.still.iter().all(|i| i.url.ends_with(".webp")));
    }

    #[test]
    fn no_picture_and_uploads_without_webp_are_handled() {
        assert!(fixture_picture(&fixture(), "u_71092938").is_none());
        // A gif upload falls back to gif, and a protocol-relative url is made absolute.
        let gif = json!({ "id": "g", "images": [
            { "url": "//cdn.7tv.app/g/1x.gif", "mime": "image/gif", "scale": 1, "frameCount": 9 },
            { "url": "//cdn.7tv.app/g/1x_static.png", "mime": "image/png", "scale": 1, "frameCount": 1 } ] });
        let pic = profile_picture_from(&gif).unwrap();
        assert_eq!(pic.animated[0].url, "https://cdn.7tv.app/g/1x.gif");
        assert_eq!(pic.still[0].url, "https://cdn.7tv.app/g/1x_static.png");
        assert!(profile_picture_from(&json!({ "id": "e", "images": [] })).is_none());
    }

    #[test]
    fn the_worn_answer_carries_the_picture_id_and_a_known_picture() {
        let data = fixture();
        let worn = worn_cosmetics(data.pointer("/worn/u_64282485/userByConnection").unwrap());
        assert_eq!(worn.picture_id.as_deref(), Some("01JJCJQ9HDR7HH3PH3APTW9SS7"));
        let none = worn_cosmetics(data.pointer("/worn/u_71092938/userByConnection").unwrap());
        assert!(none.picture_id.is_none() && none.profile_picture.is_none());

        // Once a picture's images are known, a worn answer attaches them, and
        // only the picture (never the bookkeeping id) reaches the wire.
        remember_picture("01KPNFT8JDMS68AD6GYCHGA3Z9", fixture_picture(&data, "u_129272570"));
        let still = worn_cosmetics(data.pointer("/worn/u_129272570/userByConnection").unwrap());
        assert_eq!(still.profile_picture.as_ref().map(|p| p.id.as_str()), Some("01KPNFT8JDMS68AD6GYCHGA3Z9"));
        let wire = serde_json::to_value(&still).unwrap();
        assert!(wire.get("profilePicture").is_some());
        assert!(wire.get("picture_id").is_none() && wire.get("pictureId").is_none());
    }

    fn put_cached(id: &str, data: UserCosmetics, age: Duration, personal_noted: bool) {
        let mut guard = USERS.lock().unwrap();
        guard
            .get_or_insert_with(|| LruCache::new(NonZeroUsize::new(USER_CACHE_CAPACITY).unwrap()))
            .put(
                id.to_string(),
                Cached { at: Instant::now().checked_sub(age).unwrap(), hard_fail: false, data, personal_noted },
            );
    }

    fn wearing(picture_id: &str) -> UserCosmetics {
        UserCosmetics { picture_id: Some(picture_id.into()), ..Default::default() }
    }

    #[test]
    fn a_cache_hit_attaches_a_picture_whose_images_arrived_later() {
        put_cached("90000001", wearing("pic-late"), Duration::from_secs(1), true);
        let (hit, _) = cached("90000001", true).expect("fresh");
        assert!(hit.data.profile_picture.is_none(), "images not known yet");
        let picture = ProfilePicture {
            id: "pic-late".into(),
            still: vec![PictureImage { url: "https://cdn.7tv.app/p/1x.webp".into(), scale: 1 }],
            animated: vec![],
        };
        remember_picture("pic-late", Some(picture.clone()));
        let (hit, _) = cached("90000001", true).expect("fresh");
        assert_eq!(hit.data.profile_picture, Some(picture));
    }

    #[test]
    fn a_picture_that_never_arrived_keeps_the_entry_only_briefly() {
        // Past HARD_FAIL_TTL with the images still unknown: asked again.
        put_cached("90000002", wearing("pic-lost"), HARD_FAIL_TTL + Duration::from_secs(1), true);
        assert!(cached("90000002", true).is_none());
        // The same age with the answer known ("no images") lives the full TTL.
        remember_picture("pic-gone", None);
        put_cached("90000003", wearing("pic-gone"), HARD_FAIL_TTL + Duration::from_secs(1), true);
        let (hit, _) = cached("90000003", true).expect("within USER_TTL");
        assert!(hit.data.profile_picture.is_none());
        // No picture worn at all: the full TTL as well.
        put_cached("90000004", UserCosmetics::default(), HARD_FAIL_TTL + Duration::from_secs(1), true);
        assert!(cached("90000004", true).is_some());
    }

    #[test]
    fn a_list_lookup_leaves_the_personal_set_to_the_first_chat_lookup() {
        let data = UserCosmetics { personal_set: Some("set-1".into()), ..Default::default() };
        put_cached("90000005", data, Duration::from_secs(1), false);
        // A list surface never hands the set on.
        assert_eq!(cached("90000005", false).map(|(_, note)| note), Some(false));
        // The first chat lookup does, once.
        assert_eq!(cached("90000005", true).map(|(_, note)| note), Some(true));
        assert_eq!(cached("90000005", true).map(|(_, note)| note), Some(false));
    }

    #[test]
    fn only_ids_7tv_can_know_are_looked_up() {
        assert!(has_seventv_platform("71092938"));
        assert!(has_seventv_platform("kick:26298"));
        assert!(has_seventv_platform("youtube:UCX6OQ3DkcsbYNE6H8uQQuVA"));
        assert!(!has_seventv_platform("tiktok:123"));
        assert!(!has_seventv_platform("youtube:@handle"));
        assert!(!has_seventv_platform("kick:abc"));
        assert!(!has_seventv_platform("xqc"));
        assert!(!has_seventv_platform(""));

        // A stream row's platform and user id, keyed the way 7TV knows them.
        assert_eq!(cosmetics_id("twitch", "71092938").as_deref(), Some("71092938"));
        assert_eq!(cosmetics_id("kick", "26298").as_deref(), Some("kick:26298"));
        assert_eq!(
            cosmetics_id("youtube", "UCX6OQ3DkcsbYNE6H8uQQuVA").as_deref(),
            Some("youtube:UCX6OQ3DkcsbYNE6H8uQQuVA")
        );
        assert_eq!(cosmetics_id("tiktok", "123"), None);
        assert_eq!(cosmetics_id("kick", "xqc"), None);
        assert_eq!(cosmetics_id("twitch", ""), None);
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
