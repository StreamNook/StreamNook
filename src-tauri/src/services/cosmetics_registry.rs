//! StreamNook's member and cosmetics registries, held once in Rust.
//!
//! Three world-readable sets decide what a StreamNook member wears: who is a
//! member (`user_numbers`), the cosmetics catalog with every member's grants and
//! active badge (`cosmetics`, `user_cosmetics`, `user_cosmetic_active`), and the
//! Atmosphere catalog (`atmospheres`). They are small (hundreds of rows) and
//! change rarely, so they are read whole at boot, re-read on every realtime
//! change and on focus or resume, and kept here for every window and both phone
//! shells. A failed read keeps the last good copy: one network blip must never
//! blank a paying member's badge for the session.
//!
//! The rules over them live here too: what a member owns, which badge they wear,
//! the art a badge draws with, and whether an Atmosphere may paint. The pages
//! read a snapshot (`snapshot`) and the native shells read resolved answers.

use crate::services::css_fill::{self, Fill};
use crate::services::streamnook_backend as backend;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

/// Emitted with the new version whenever any registry changes.
pub const EVENT: &str = "sn-registry-changed";

/// The badge every member wears until they equip another.
pub const DEFAULT_SLUG: &str = "streamnook-default";
pub const SUPPORTER_SLUG: &str = "streamnook-supporter";
pub const SUBSCRIBER_SLUG: &str = "streamnook-subscriber";
/// The CS2 Major Cologne look's base theme id; applied ids may carry `+coin` /
/// `+border` add-on suffixes.
pub const COLOGNE_THEME_ID: &str = "cs2-major-cologne";

/// House badges ship inside the apps rather than on the CDN. A native shell
/// reads this scheme as "the bundled art for this slug".
pub const HOUSE_SCHEME: &str = "streamnook-house:";
const HOUSE_SLUGS: &[&str] = &[DEFAULT_SLUG, SUPPORTER_SLUG, SUBSCRIBER_SLUG];

/// One catalog row, as the pages already read it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CatalogEntry {
    pub slug: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub asset_path: String,
    #[serde(default)]
    pub asset_manifest: Option<Value>,
    #[serde(default)]
    pub animated: bool,
    #[serde(default)]
    pub payment_type: Option<String>,
    #[serde(default)]
    pub ko_fi_url: Option<String>,
    #[serde(default)]
    pub stripe_url: Option<String>,
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub hidden: Option<bool>,
    #[serde(default)]
    pub contribution_min_points: Option<i64>,
}

/// One Atmosphere: the CSS the pages paint, and the same look read into layers
/// for native renderers.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Atmosphere {
    pub id: String,
    pub name: String,
    /// "r, g, b", as the catalog writes it.
    pub accent: String,
    pub swatch: String,
    pub base_color: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_layers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    pub image_profile_portrait: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layers2: Option<String>,
    /// "drift" or "aurora".
    pub motion: String,
    pub chat_edge: String,
    pub chat_frost: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_blur: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_rim: Option<String>,
    pub unlock: Unlock,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chrome_texture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chrome_coin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chrome_frame: Option<String>,
    /// The look as layers, for renderers without CSS.
    pub look: AtmosphereLook,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Unlock {
    #[default]
    Subscriber,
    #[serde(rename_all = "camelCase")]
    Accolade { accolade_id: String },
}

/// An Atmosphere's look as data: colours as `#rrggbbaa`, CSS backgrounds as
/// layers (see `css_fill`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AtmosphereLook {
    pub accent: String,
    pub swatch: Fill,
    pub base_color: String,
    #[serde(default, skip_serializing_if = "Fill::is_empty")]
    pub base_layers: Fill,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(default, skip_serializing_if = "Fill::is_empty")]
    pub layers: Fill,
    #[serde(default, skip_serializing_if = "Fill::is_empty")]
    pub layers2: Fill,
    pub motion: String,
    /// The 1 px edge down the left of a chat row; None draws none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_edge: Option<Fill>,
    pub chat_frost: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_blur: Option<f32>,
    /// A 1 px rim around a chat row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_rim: Option<Fill>,
    /// "cologne-chrome" for the Cologne look, drawn from `chrome_*` instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome_texture: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome_coin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chrome_frame: Option<String>,
}

#[derive(Default)]
struct State {
    members: HashMap<String, u32>,
    members_loaded: bool,
    catalog: Vec<CatalogEntry>,
    grants: HashMap<String, BTreeSet<String>>,
    active: HashMap<String, String>,
    cosmetics_loaded: bool,
    atmospheres: Vec<Atmosphere>,
    atmospheres_loaded: bool,
}

static STATE: OnceLock<RwLock<State>> = OnceLock::new();
static VERSION: AtomicU64 = AtomicU64::new(0);
static LOADING: AsyncMutex<()> = AsyncMutex::const_new(());
static QUEUED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

fn state() -> &'static RwLock<State> {
    STATE.get_or_init(|| RwLock::new(State::default()))
}

fn read<T>(f: impl FnOnce(&State) -> T) -> T {
    let guard = state().read().unwrap_or_else(|e| e.into_inner());
    f(&guard)
}

fn write(f: impl FnOnce(&mut State)) {
    let mut guard = state().write().unwrap_or_else(|e| e.into_inner());
    f(&mut guard);
}

pub fn version() -> u64 {
    VERSION.load(Ordering::Relaxed)
}

fn bump() {
    let v = VERSION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Some(app) = crate::services::providers::app_handle() {
        use tauri::Emitter;
        let _ = app.emit(EVENT, serde_json::json!({ "version": v }));
    }
    // What members own may have changed, so every known theme is gated again.
    crate::services::member_theme::regate();
}

// ---- Loading -----------------------------------------------------------------

/// Reads every registry now. One load at a time; a call that lands mid-load
/// queues exactly one more pass, since a change may postdate what the running
/// load already read.
pub async fn refresh() {
    if !backend::configured() {
        return;
    }
    let Ok(_guard) = LOADING.try_lock() else {
        QUEUED.store(true, Ordering::Relaxed);
        return;
    };
    loop {
        QUEUED.store(false, Ordering::Relaxed);
        let (members, cosmetics, atmospheres) = tokio::join!(load_members(), load_cosmetics(), load_atmospheres());
        let mut changed = false;
        match members {
            Ok(m) => write(|s| {
                changed |= s.members != m || !s.members_loaded;
                s.members = m;
                s.members_loaded = true;
            }),
            Err(e) => log::warn!("[CosmeticsRegistry] member registry read failed, keeping the last copy: {e}"),
        }
        match cosmetics {
            Ok((catalog, grants, active)) => write(|s| {
                changed |= s.catalog != catalog || s.grants != grants || s.active != active || !s.cosmetics_loaded;
                s.catalog = catalog;
                s.grants = grants;
                s.active = active;
                s.cosmetics_loaded = true;
            }),
            Err(e) => log::warn!("[CosmeticsRegistry] cosmetics read failed, keeping the last copy: {e}"),
        }
        match atmospheres {
            Ok(a) => write(|s| {
                changed |= s.atmospheres != a || !s.atmospheres_loaded;
                s.atmospheres = a;
                s.atmospheres_loaded = true;
            }),
            Err(e) => log::warn!("[CosmeticsRegistry] atmosphere catalog read failed, keeping the last copy: {e}"),
        }
        if changed {
            bump();
        }
        if !QUEUED.load(Ordering::Relaxed) {
            break;
        }
    }
}

/// Waits (up to `limit`) until the member list and the cosmetics and atmosphere
/// catalogs have loaded once, so an answer is never computed against an empty one.
pub async fn ready(limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    loop {
        // The member list too: a sheet or backlog read before it would find no member.
        if read(|s| s.members_loaded && s.cosmetics_loaded && s.atmospheres_loaded) {
            return true;
        }
        if tokio::time::Instant::now() >= deadline || !backend::configured() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn load_members() -> Result<HashMap<String, u32>, String> {
    let rows = backend::select_all("user_numbers", "select=id,user_number&order=id").await?;
    Ok(rows
        .iter()
        .filter_map(|r| Some((r.get("id")?.as_str()?.to_string(), r.get("user_number")?.as_u64()? as u32)))
        .collect())
}

type CosmeticsTables = (Vec<CatalogEntry>, HashMap<String, BTreeSet<String>>, HashMap<String, String>);

async fn load_cosmetics() -> Result<CosmeticsTables, String> {
    let (catalog, grants, active) = tokio::join!(
        backend::select("cosmetics", "select=*&is_active=eq.true&order=sort_order"),
        backend::select_all("user_cosmetics", "select=twitch_user_id,slug&order=twitch_user_id,slug"),
        backend::select_all("user_cosmetic_active", "select=twitch_user_id,active_slug&order=twitch_user_id"),
    );
    let catalog: Vec<CatalogEntry> = catalog?.into_iter().filter_map(|r| serde_json::from_value(r).ok()).collect();
    let mut grant_map: HashMap<String, BTreeSet<String>> = HashMap::new();
    for row in grants? {
        if let (Some(id), Some(slug)) = (row["twitch_user_id"].as_str(), row["slug"].as_str()) {
            grant_map.entry(id.to_string()).or_default().insert(slug.to_string());
        }
    }
    let active_map = active?
        .iter()
        .filter_map(|r| Some((r["twitch_user_id"].as_str()?.to_string(), r["active_slug"].as_str()?.to_string())))
        .collect();
    Ok((catalog, grant_map, active_map))
}

async fn load_atmospheres() -> Result<Vec<Atmosphere>, String> {
    let rows = backend::select("atmospheres", "select=*&is_active=eq.true&order=sort_order").await?;
    Ok(rows.iter().filter_map(atmosphere_from_row).collect())
}

fn text(row: &Value, key: &str) -> Option<String> {
    row.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from)
}

/// A catalog row as an Atmosphere, with the same defaults the pages apply.
pub(crate) fn atmosphere_from_row(row: &Value) -> Option<Atmosphere> {
    let id = text(row, "id")?;
    let accent = text(row, "accent").unwrap_or_default();
    let swatch = text(row, "swatch").unwrap_or_default();
    let base_color = text(row, "base_color").unwrap_or_else(|| "#000000".into());
    let chat_edge = text(row, "chat_edge").unwrap_or_else(|| "none".into());
    let chat_blur = row.get("chat_blur").and_then(Value::as_f64).filter(|b| *b > 0.0).map(|b| b as f32);
    let chat_rim = text(row, "chat_rim");
    let motion = if text(row, "motion").as_deref() == Some("drift") { "drift" } else { "aurora" }.to_string();
    let kind = text(row, "kind").filter(|k| k == "cologne-chrome");
    let unlock = match (text(row, "unlock_kind").as_deref(), text(row, "unlock_accolade_id")) {
        (Some("accolade"), Some(accolade_id)) => Unlock::Accolade { accolade_id },
        _ => Unlock::Subscriber,
    };
    let base_layers = text(row, "base_layers");
    let image = text(row, "image");
    let layers = text(row, "layers");
    let layers2 = text(row, "layers2");
    let look = AtmosphereLook {
        accent: css_fill::rgb_triple(&accent).unwrap_or_else(|| "#ffffffff".into()),
        swatch: css_fill::parse(&swatch),
        base_color: css_fill::color(&base_color).unwrap_or_else(|| "#000000ff".into()),
        base_layers: base_layers.as_deref().map(css_fill::parse).unwrap_or_default(),
        image: image.clone(),
        layers: layers.as_deref().map(css_fill::parse).unwrap_or_default(),
        layers2: layers2.as_deref().map(css_fill::parse).unwrap_or_default(),
        motion: motion.clone(),
        chat_edge: Some(css_fill::parse(&chat_edge)).filter(|f| !f.is_empty()),
        chat_frost: row.get("chat_frost").and_then(Value::as_bool).unwrap_or(false),
        chat_blur,
        chat_rim: chat_rim.as_deref().map(css_fill::parse).filter(|f| !f.is_empty()),
        kind: kind.clone(),
        chrome_texture: text(row, "chrome_texture"),
        chrome_coin: text(row, "chrome_coin"),
        chrome_frame: text(row, "chrome_frame"),
    };
    Some(Atmosphere {
        id,
        name: text(row, "name").unwrap_or_default(),
        accent,
        swatch,
        base_color,
        base_layers,
        image,
        image_profile_portrait: row.get("image_profile_portrait").and_then(Value::as_bool).unwrap_or(false),
        layers,
        layers2,
        motion,
        chat_edge,
        chat_frost: look.chat_frost,
        chat_blur,
        chat_rim,
        unlock,
        kind,
        chrome_texture: look.chrome_texture.clone(),
        chrome_coin: look.chrome_coin.clone(),
        chrome_frame: look.chrome_frame.clone(),
        look,
    })
}

/// Loads the registries, then follows the realtime feed and re-reads on every
/// change to them (and on a rejoin after a gap). Started once at boot.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) || !backend::configured() {
        return;
    }
    let mut changes = backend::subscribe();
    backend::start_realtime();
    tauri::async_runtime::spawn(async move {
        refresh().await;
        loop {
            match changes.recv().await {
                Ok(backend::Change::Row { table, .. }) if table == "user_profile_prefs" => {}
                Ok(_) => refresh().await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => refresh().await,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

// ---- Reads -------------------------------------------------------------------

/// Whether the cosmetics catalog has loaded. An empty catalog and "owns nothing"
/// are the same answer from `owned_slugs`, so gates must allow while this is
/// false.
pub fn cosmetics_loaded() -> bool {
    read(|s| s.cosmetics_loaded && !s.catalog.is_empty())
}

/// The member number filed under a Twitch user id, when they are a member.
pub fn member_number(member_id: &str) -> Option<u32> {
    read(|s| s.members.get(member_id).copied())
}

pub fn is_member(member_id: &str) -> bool {
    member_number(member_id).is_some()
}

pub fn catalog_entry(slug: &str) -> Option<CatalogEntry> {
    read(|s| s.catalog.iter().find(|c| c.slug == slug).cloned())
}

/// The catalog in sort order.
pub fn catalog() -> Vec<CatalogEntry> {
    read(|s| s.catalog.clone())
}

fn owned_in(s: &State, member_id: &str) -> BTreeSet<String> {
    let mut owned: BTreeSet<String> = s.catalog.iter().filter(|c| c.is_default).map(|c| c.slug.clone()).collect();
    if let Some(grants) = s.grants.get(member_id) {
        owned.extend(grants.iter().cloned());
    }
    owned
}

/// Every slug a member can equip: their grants plus every default cosmetic.
pub fn owned_slugs(member_id: &str) -> BTreeSet<String> {
    read(|s| owned_in(s, member_id))
}

/// Kinds that own an equip slot of their own. `user_cosmetic_active` is the
/// single untyped slot the chat badge renders, so these never belong in it. An
/// unknown kind counts as a badge on purpose: blanking a legacy badge row for
/// everyone is worse than letting an odd kind through.
const OTHER_SLOT_KINDS: &[&str] = &["atmosphere", "frame", "cipher", "member_plate", "relic", "accolade_pin_style"];

/// Whether a catalog kind wears in the badge slot.
pub fn kind_is_badge(kind: &str) -> bool {
    !OTHER_SLOT_KINDS.contains(&kind)
}

fn is_badge_kind_in(s: &State, slug: &str) -> bool {
    kind_is_badge(s.catalog.iter().find(|c| c.slug == slug).map(|c| c.kind.as_str()).unwrap_or_default())
}

pub fn is_badge_kind(slug: &str) -> bool {
    read(|s| is_badge_kind_in(s, slug))
}

/// The badge a member wears: their active slug, only while they still own it and
/// it is a badge.
pub fn worn_badge_slug(member_id: &str) -> Option<String> {
    read(|s| worn_in(s, member_id))
}

fn worn_in(s: &State, member_id: &str) -> Option<String> {
    let slug = s.active.get(member_id)?;
    (owned_in(s, member_id).contains(slug) && is_badge_kind_in(s, slug)).then(|| slug.clone())
}

/// The art a cosmetic draws with: the bundled house art for the three house
/// slugs, then the asset manifest, then the catalog's cloud `asset_path`. Full
/// size; `chat` picks the house subscriber badge's own chat-size cut.
pub fn badge_art(entry: &CatalogEntry, chat: bool) -> Option<String> {
    if HOUSE_SLUGS.contains(&entry.slug.as_str()) {
        let cut = if chat && entry.slug == SUBSCRIBER_SLUG { "@chat" } else { "" };
        return Some(format!("{HOUSE_SCHEME}{}{cut}", entry.slug));
    }
    if let Some(assets) = entry.asset_manifest.as_ref().and_then(|m| m.get("assets")) {
        for key in ["animated", "static", "poster"] {
            if let Some(url) = assets.get(key).and_then(Value::as_str).filter(|u| !u.is_empty()) {
                return Some(url.to_string());
            }
        }
    }
    let path = entry.asset_path.as_str();
    (path.starts_with("https://") || path.starts_with("http://")).then(|| path.to_string())
}

/// A member's StreamNook badge as a renderer draws it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SnBadge {
    /// The equipped slug; None wears the plain StreamNook mark.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    pub image_url: String,
    /// What it is called aloud: "StreamNook", "StreamNook <name>", or the name
    /// when it already says StreamNook.
    pub label: String,
}

/// The StreamNook badge a member wears, None for anyone who is not a member.
pub fn member_badge(member_id: &str, chat: bool) -> Option<SnBadge> {
    if !is_member(member_id) {
        return None;
    }
    let entry = worn_badge_slug(member_id).and_then(|slug| catalog_entry(&slug));
    let image_url = entry
        .as_ref()
        .and_then(|e| badge_art(e, chat))
        .unwrap_or_else(|| format!("{HOUSE_SCHEME}{DEFAULT_SLUG}"));
    let label = match entry.as_ref().map(|e| e.name.trim()).filter(|n| !n.is_empty()) {
        None => "StreamNook".to_string(),
        Some(name) if name.to_ascii_lowercase().starts_with("streamnook") => name.to_string(),
        Some(name) => format!("StreamNook {name}"),
    };
    Some(SnBadge { slug: entry.map(|e| e.slug), image_url, label })
}

/// The Atmosphere for a theme id: a Cologne id with add-on suffixes finds its
/// base row and keeps the full id.
pub fn atmosphere(theme: &str) -> Option<Atmosphere> {
    if theme.starts_with(COLOGNE_THEME_ID) {
        return read(|s| s.atmospheres.iter().find(|a| a.id == COLOGNE_THEME_ID).cloned()).map(|mut a| {
            a.id = theme.to_string();
            a
        });
    }
    read(|s| s.atmospheres.iter().find(|a| a.id == theme).cloned())
}

pub fn atmospheres() -> Vec<Atmosphere> {
    read(|s| s.atmospheres.clone())
}

/// isSupporter / everSubscribed from what a member owns; an active Stripe
/// subscription also counts as subscribed where the caller knows it.
pub fn ever_subscribed(owned: &BTreeSet<String>, active_subscription: bool) -> bool {
    active_subscription || owned.contains(SUBSCRIBER_SLUG)
}

/// Whether a member may wear the Atmosphere they selected, for painting it on
/// somebody else's screen. `user_profile_prefs` is world-writable under the anon
/// key, so rendering is the last place that can refuse.
///
/// Per CLASS for subscriber atmospheres (a viewer cannot see whether someone is
/// subscribed right now, and per-item ownership is granted on their login, not
/// ours). Accolade atmospheres pass on purpose: accolades are not readable
/// cross-user, and granting them into ownership would turn a forgeable accolade
/// into a permanent row. Unknown ids pass (nothing paints), and so does
/// everything while the catalog has not loaded, so missing data never blanks a
/// paying member. `own` short-circuits for the viewer's own accounts, whose pick
/// the server already gated.
pub fn may_wear_atmosphere(member_id: &str, theme: Option<&str>, own: bool) -> bool {
    let Some(theme) = theme else { return true };
    if own || !cosmetics_loaded() {
        return true;
    }
    let Some(atm) = atmosphere(theme) else { return true };
    if matches!(atm.unlock, Unlock::Accolade { .. }) {
        return true;
    }
    let base = theme.split('+').next().unwrap_or(theme);
    let owned = owned_slugs(member_id);
    owned.contains(base) || ever_subscribed(&owned, false)
}

// ---- Writes ------------------------------------------------------------------

/// Equips (or with None, takes off) a member's StreamNook badge. The registry
/// flips first so every surface shows the pick at once, and rolls back if the
/// write is refused. Errors are the server's words, or None when it gave none.
pub async fn equip_badge(member_id: &str, slug: Option<&str>) -> Result<(), Option<String>> {
    if member_id.is_empty() {
        return Err(None);
    }
    if let Some(slug) = slug {
        if !is_badge_kind(slug) {
            log::warn!("[CosmeticsRegistry] refusing to equip non-badge cosmetic in the badge slot: {slug}");
            return Err(None);
        }
    }
    let previous = read(|s| s.active.get(member_id).cloned());
    set_active(member_id, slug.map(String::from));
    let result = write_active(member_id, slug).await;
    if result.is_err() {
        set_active(member_id, previous);
    }
    result
}

fn set_active(member_id: &str, slug: Option<String>) {
    write(|s| match slug {
        Some(slug) => {
            s.active.insert(member_id.to_string(), slug);
        }
        None => {
            s.active.remove(member_id);
        }
    });
    bump();
}

async fn write_active(member_id: &str, slug: Option<&str>) -> Result<(), Option<String>> {
    if crate::services::client_config::get().await.write_via_api {
        let body = serde_json::json!({ "slug": slug });
        let res = crate::commands::streamnook_api::post_json("/api/cosmetics/equip", &body, None).await;
        match res {
            Ok(r) if r.ok => return Ok(()),
            Ok(r) => return Err(Some(r.body.chars().take(200).collect::<String>()).filter(|b| !b.trim().is_empty())),
            Err(e) => log::warn!("[CosmeticsRegistry] /api/cosmetics/equip unavailable, falling back: {e}"),
        }
    }
    backend::upsert(
        "user_cosmetic_active",
        "twitch_user_id",
        serde_json::json!({
            "twitch_user_id": member_id,
            "active_slug": slug,
            "updated_at": chrono::Utc::now().to_rfc3339(),
        }),
    )
    .await
    .map_err(|e| {
        log::warn!("[CosmeticsRegistry] equip failed: {e}");
        None
    })
}

// ---- The pages' copy -----------------------------------------------------------

/// Everything the pages' synchronous reads need, in one answer.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub version: u64,
    pub members_loaded: bool,
    pub cosmetics_loaded: bool,
    pub atmospheres_loaded: bool,
    /// Twitch user id to member number.
    pub members: HashMap<String, u32>,
    pub catalog: Vec<CatalogEntry>,
    /// Explicit grants per member (defaults are in the catalog's `is_default`).
    pub grants: HashMap<String, Vec<String>>,
    /// The badge each member wears, already checked against what they own.
    pub worn: HashMap<String, String>,
    pub atmospheres: Vec<Atmosphere>,
}

pub fn snapshot() -> Snapshot {
    read(|s| Snapshot {
        version: version(),
        members_loaded: s.members_loaded,
        cosmetics_loaded: s.cosmetics_loaded,
        atmospheres_loaded: s.atmospheres_loaded,
        members: s.members.clone(),
        catalog: s.catalog.clone(),
        grants: s.grants.iter().map(|(k, v)| (k.clone(), v.iter().cloned().collect())).collect(),
        worn: s.active.keys().filter_map(|id| worn_in(s, id).map(|slug| (id.clone(), slug))).collect(),
        atmospheres: s.atmospheres.clone(),
    })
}

/// Fills the registry for tests.
#[cfg(test)]
pub(crate) fn set_for_test(
    members: &[(&str, u32)],
    catalog: Vec<CatalogEntry>,
    grants: &[(&str, &str)],
    active: &[(&str, &str)],
    atmospheres: Vec<Atmosphere>,
) {
    write(|s| {
        s.members = members.iter().map(|(id, n)| (id.to_string(), *n)).collect();
        s.members_loaded = true;
        s.catalog = catalog;
        s.grants = HashMap::new();
        for (id, slug) in grants {
            s.grants.entry(id.to_string()).or_default().insert(slug.to_string());
        }
        s.active = active.iter().map(|(id, slug)| (id.to_string(), slug.to_string())).collect();
        s.cosmetics_loaded = true;
        s.atmospheres = atmospheres;
        s.atmospheres_loaded = true;
    });
}

#[cfg(test)]
pub(crate) fn entry_for_test(slug: &str, kind: &str, asset_path: &str, is_default: bool) -> CatalogEntry {
    CatalogEntry {
        slug: slug.into(),
        name: slug.replace('-', " "),
        kind: kind.into(),
        asset_path: asset_path.into(),
        is_active: true,
        is_default,
        ..Default::default()
    }
}

#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() {
        let aurora = atmosphere_from_row(&json!({
            "id": "aurora", "name": "Aurora", "accent": "45, 212, 191",
            "swatch": "linear-gradient(135deg, #10b981, #8b5cf6)", "base_color": "#04070c",
            "motion": "aurora", "chat_edge": "none", "unlock_kind": "subscriber", "sort_order": 1
        }))
        .unwrap();
        let midnight = atmosphere_from_row(&json!({
            "id": "midnight", "name": "Midnight", "accent": "74, 222, 128",
            "swatch": "url(https://cdn.streamnook.app/atmospheres/midnight.webp) center / cover",
            "base_color": "#05060d", "image": "https://cdn.streamnook.app/atmospheres/midnight.webp",
            "motion": "aurora", "chat_edge": "linear-gradient(to bottom, transparent, #4ade80)", "chat_frost": true,
            "chat_blur": 8, "unlock_kind": "accolade", "unlock_accolade_id": "insomniac"
        }))
        .unwrap();
        let cologne = atmosphere_from_row(&json!({
            "id": COLOGNE_THEME_ID, "name": "Cologne", "accent": "214, 177, 92", "swatch": "#0a1322",
            "base_color": "#0a1322", "motion": "drift", "chat_edge": "none", "unlock_kind": "accolade",
            "unlock_accolade_id": "cs2_major_cologne_2026", "kind": "cologne-chrome"
        }))
        .unwrap();
        set_for_test(
            &[("1", 1), ("2", 2), ("3", 3)],
            vec![
                entry_for_test(DEFAULT_SLUG, "badge", "streamnook-logo.png", true),
                entry_for_test(SUBSCRIBER_SLUG, "badge", "streamnook-badge-gold-animated.webp", false),
                entry_for_test("streamnook-mobius", "badge", "https://cdn.streamnook.app/badges/mobius.webp", false),
                entry_for_test("aurora", "atmosphere", "atmosphere", false),
                entry_for_test("cologne-frame", "frame", "https://cdn.streamnook.app/frame.png", false),
            ],
            &[("1", SUBSCRIBER_SLUG), ("1", "streamnook-mobius"), ("2", "cologne-frame"), ("3", "aurora")],
            &[("1", "streamnook-mobius"), ("2", "cologne-frame"), ("3", "streamnook-mobius")],
            vec![aurora, midnight, cologne],
        );
    }

    #[test]
    fn a_worn_badge_must_be_owned_and_a_badge() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        assert_eq!(worn_badge_slug("1").as_deref(), Some("streamnook-mobius"));
        // A frame in the badge slot is ignored, restoring the plain badge.
        assert_eq!(worn_badge_slug("2"), None);
        // An active slug the member does not own is ignored.
        assert_eq!(worn_badge_slug("3"), None);
        assert!(owned_slugs("3").contains(DEFAULT_SLUG));
        assert!(!is_badge_kind("cologne-frame"));
        assert!(is_badge_kind("something-new"));
    }

    #[test]
    fn badge_art_prefers_the_house_cut_then_the_cdn() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        let sub = catalog_entry(SUBSCRIBER_SLUG).unwrap();
        assert_eq!(badge_art(&sub, false).as_deref(), Some("streamnook-house:streamnook-subscriber"));
        assert_eq!(badge_art(&sub, true).as_deref(), Some("streamnook-house:streamnook-subscriber@chat"));
        let mobius = catalog_entry("streamnook-mobius").unwrap();
        assert_eq!(badge_art(&mobius, true).as_deref(), Some("https://cdn.streamnook.app/badges/mobius.webp"));
        assert_eq!(badge_art(&catalog_entry("aurora").unwrap(), false), None);
    }

    #[test]
    fn member_badges_label_themselves_and_skip_non_members() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        let one = member_badge("1", false).unwrap();
        // The name already says StreamNook, so it is not said twice.
        assert_eq!(one.label, "streamnook mobius");
        assert_eq!(one.image_url, "https://cdn.streamnook.app/badges/mobius.webp");
        let plain = member_badge("2", false).unwrap();
        assert_eq!(plain.label, "StreamNook");
        assert_eq!(plain.image_url, "streamnook-house:streamnook-default");
        assert!(member_badge("999", false).is_none());
    }

    #[test]
    fn the_atmosphere_gate_is_per_class_and_lets_accolades_through() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        // Subscriber badge owner: any subscriber atmosphere.
        assert!(may_wear_atmosphere("1", Some("aurora"), false));
        // Owns the aurora row itself.
        assert!(may_wear_atmosphere("3", Some("aurora"), false));
        // Owns neither: refused on other people's screens, allowed on their own.
        assert!(!may_wear_atmosphere("2", Some("aurora"), false));
        assert!(may_wear_atmosphere("2", Some("aurora"), true));
        // Accolade and unknown ids pass; Cologne finds its base row.
        assert!(may_wear_atmosphere("2", Some("midnight"), false));
        assert!(may_wear_atmosphere("2", Some("nope"), false));
        assert_eq!(atmosphere("cs2-major-cologne+coin").map(|a| a.id).as_deref(), Some("cs2-major-cologne+coin"));
    }

    #[test]
    fn the_gate_allows_clearing_cologne_by_base_id_and_everything_before_the_catalog_loads() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        assert!(may_wear_atmosphere("2", None, false));
        // Cologne is accolade-unlocked, so it passes; ownership of its base row is
        // what a subscriber-unlocked look with suffixes would be judged by.
        assert!(may_wear_atmosphere("2", Some("cs2-major-cologne+coin+border"), false));
        set_for_test(&[], vec![], &[], &[], vec![]);
        assert!(!cosmetics_loaded());
        assert!(may_wear_atmosphere("2", Some("aurora"), false));
    }

    #[test]
    fn atmosphere_rows_read_into_css_and_layers() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        let midnight = atmosphere("midnight").unwrap();
        assert!(matches!(midnight.unlock, Unlock::Accolade { ref accolade_id } if accolade_id == "insomniac"));
        assert_eq!(midnight.look.chat_blur, Some(8.0));
        assert!(midnight.look.chat_frost);
        assert_eq!(midnight.look.accent, "#4ade80ff");
        assert!(midnight.look.chat_edge.is_some());
        assert_eq!(atmosphere("aurora").unwrap().look.chat_edge, None);
        let page = serde_json::to_value(&midnight).unwrap();
        assert_eq!(page["unlock"], json!({ "kind": "accolade", "accoladeId": "insomniac" }));
        assert_eq!(page["chatEdge"], "linear-gradient(to bottom, transparent, #4ade80)");
    }

    #[test]
    fn the_snapshot_carries_resolved_worn_badges() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        let snap = snapshot();
        assert_eq!(snap.worn.get("1").map(String::as_str), Some("streamnook-mobius"));
        assert!(!snap.worn.contains_key("2"));
        assert_eq!(snap.members.len(), 3);
    }
}
