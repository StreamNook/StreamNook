//! The signed-in account's badge standing: which global Twitch badges it owns,
//! when every badge can be earned, and the ones it is missing that are
//! earnable right now.
//!
//! One Rust copy serves every surface (the desktop Badges overlay and the phone
//! Rewards tab). The owned collection comes from Twitch and costs a network
//! call, so it is cached in memory, persisted to `badge_collection.json` so a
//! cold start paints at once, and refreshed in the background: a request never
//! waits on the network. When a refresh lands, `badge-standing-changed` tells
//! the surfaces to ask again.
//!
//! The collection is read anonymously: Twitch's `channelViewer.earnedBadges`
//! answers a user's whole global collection with no token, so this needs no
//! Drops sign-in or any other extra login.
//!
//! Honesty rules, each of which a surface relies on:
//! - "Missing" is only claimed from a collection Twitch actually answered.
//!   Until the first read lands, the missing list stays empty.
//! - A missing field is a failed read, never "owns nothing".
//! - A failed refresh keeps the last good collection rather than blanking it.
//! - Ownership is by exact badge id. A relaunched version of a badge you own is
//!   a different badge you can earn, so it is listed.

use crate::services::account_store::AccountStore;
use crate::services::badge_window::{self, WindowRun, WindowStatus};
use crate::services::universal_cache_service::{self, CacheType};
use log::{debug, warn};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use tokio::sync::RwLock;

/// A complete, fresh collection is re-read after this long.
const COMPLETE_TTL_MS: i64 = 10 * 60 * 1000;
/// A failed or not-yet-made read is retried sooner.
const INCOMPLETE_TTL_MS: i64 = 60 * 1000;
const COLLECTION_FILE: &str = "badge_collection.json";

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CollectionState {
    /// The collection as Twitch answered it for this account.
    Complete,
    /// Not read yet, or the only read failed. Never used to claim "missing".
    Partial,
    /// Nothing to go on: no signed-in account.
    Unavailable,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MissingBadge {
    /// `set_id/version`, the key every map in the standing uses.
    pub key: String,
    pub set_id: String,
    pub version: String,
    pub title: String,
    pub image_url: String,
    /// When the window the badge is earnable in closes. `None` = open-ended.
    pub ends_ms: Option<i64>,
    /// How it is earned, as steps a surface draws as chips.
    pub earn: crate::services::badge_earn::EarnPath,
    /// The campaign category, for a link.
    pub category: Option<String>,
}

/// The US price of one Tier 1 subscription, in cents.
pub const SUB_PRICE_US_CENTS: u32 = 599;

/// What it takes to earn every badge in `missing_now`, done efficiently.
///
/// Twitch counts one subscription or one stretch of watching toward every
/// campaign in the category it happens in, so badges that share a category
/// share the effort: subs are the most any one of them needs, and watch time is
/// the most any of them needs on each day, summed over the days.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct CatchUp {
    pub subs: u32,
    /// `subs` at the US Tier 1 price.
    pub sub_cost_cents: u32,
    pub watch_minutes: u32,
    /// Event passes to buy. Priced by the event, so not in `sub_cost_cents`.
    pub tickets: u32,
    /// Badges drawn at random from a pool: the totals get you a draw, not a
    /// promise of that badge, so they are a floor.
    pub random: u32,
    /// Badges with no number to add up (cheer, create, attend, other, or a
    /// watch with no stated time).
    pub unpriced: u32,
    /// Some numbers were read from copy rather than the campaign's own.
    pub estimated: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct BadgeStanding {
    pub login: Option<String>,
    pub collection: CollectionState,
    /// `not_signed_in` | `fetch_failed`
    pub collection_reason: Option<String>,
    /// Showing the last good collection because the latest read failed or has
    /// not happened yet.
    pub stale: bool,
    /// A background refresh is in flight; `badge-standing-changed` follows.
    pub refreshing: bool,
    /// False while the badge catalogue or its metadata is absent, so a surface
    /// shows loading instead of claiming there is nothing to earn.
    pub catalogue_ready: bool,
    /// Owned badge ids, `set_id/version`.
    pub owned: Vec<String>,
    /// Every badge with a known earn window, keyed `set_id/version`.
    pub windows: HashMap<String, Vec<WindowRun>>,
    pub missing_now: Vec<MissingBadge>,
    /// The next instant any window opens or closes; ask again then.
    pub next_change_ms: Option<i64>,
    /// Twitch's own art for earn chips, keyed by earn-step kind: `subscribe` is
    /// the Sub Gifter badge (the gift), `cheer` is the 100 Bits badge (the purple gem).
    /// Absent when the catalogue does not have them; a surface falls back to a
    /// drawn icon.
    pub earn_icons: HashMap<String, String>,
    /// The cost of earning everything in `missing_now`. `None` when it is empty.
    pub catch_up: Option<CatchUp>,
    pub generated_ms: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Cached {
    user_id: String,
    #[serde(default)]
    login: String,
    ids: Vec<String>,
    state: CollectionState,
    #[serde(default)]
    reason: Option<String>,
    fetched_at_ms: i64,
    #[serde(default)]
    stale: bool,
}

impl Cached {
    fn is_due(&self, now_ms: i64) -> bool {
        let ttl = if self.state == CollectionState::Complete && !self.stale {
            COMPLETE_TTL_MS
        } else {
            INCOMPLETE_TTL_MS
        };
        now_ms - self.fetched_at_ms >= ttl
    }
}

static COLLECTION: Lazy<RwLock<Option<Cached>>> = Lazy::new(|| RwLock::new(None));
static REFRESHING: AtomicBool = AtomicBool::new(false);
/// `fetched_at_ms` of the last collection the id-drift check logged for, so it
/// logs once per read rather than once per request.
static DRIFT_CHECKED_AT: AtomicI64 = AtomicI64::new(0);

/// Clears the in-flight flag however the refresh task ends, panics included.
struct RefreshGuard;
impl Drop for RefreshGuard {
    fn drop(&mut self) {
        REFRESHING.store(false, Ordering::Release);
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn collection_path() -> Option<std::path::PathBuf> {
    crate::services::twitch_service::get_app_data_dir()
        .ok()
        .map(|dir| dir.join(COLLECTION_FILE))
}

fn read_persisted() -> Option<Cached> {
    let raw = std::fs::read(collection_path()?).ok()?;
    let cached: Cached = serde_json::from_slice(&raw).ok()?;
    (cached.state == CollectionState::Complete && !cached.user_id.is_empty()).then_some(cached)
}

/// Written to a temp file and renamed, so a crash mid-write can never leave a
/// truncated collection for the next launch to trust.
fn persist(cached: &Cached) {
    let Some(path) = collection_path() else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    let result = serde_json::to_vec(cached)
        .map_err(|e| e.to_string())
        .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
        .and_then(|_| std::fs::rename(&tmp, &path).map_err(|e| e.to_string()));
    if let Err(e) = result {
        warn!("[BadgeStanding] Could not save the badge collection: {}", e);
    }
}

/// The cached collection: memory first, then the file a previous run left.
/// With `user_id`, only that account's; without one (the signed-in account is
/// not known yet), whichever account was last read, which the refresh then
/// confirms or replaces. Loading from disk seeds memory as stale.
async fn cached_for(user_id: Option<&str>) -> Option<Cached> {
    let matches = |c: &Cached| user_id.map(|u| c.user_id == u).unwrap_or(true);
    if let Some(c) = COLLECTION.read().await.as_ref() {
        if matches(c) {
            return Some(c.clone());
        }
    }
    let mut from_disk = tokio::task::spawn_blocking(read_persisted)
        .await
        .ok()
        .flatten()
        .filter(|c| matches(c))?;
    from_disk.stale = true;
    // Seed at fetched_at 0 so the first request after a launch refreshes.
    from_disk.fetched_at_ms = 0;
    let mut slot = COLLECTION.write().await;
    if slot.as_ref().map(|c| !matches(c)).unwrap_or(true) {
        *slot = Some(from_disk.clone());
    }
    Some(from_disk)
}

/// Something may have just changed what the account owns (a drop reward was
/// claimed): make the next request re-read the collection, and tell open
/// surfaces to ask now rather than wait out the cache.
///
/// Twitch can list a claimed badge a little after the claim, so the same nudge
/// repeats once, 45 s later, in case the first read was too early.
pub fn collection_may_have_changed(app: &crate::rt::AppHandle) {
    fn nudge(app: &crate::rt::AppHandle) {
        if let Ok(mut slot) = COLLECTION.try_write() {
            if let Some(c) = slot.as_mut() {
                c.fetched_at_ms = 0;
            }
        }
        use tauri::Emitter;
        let _ = app.emit("badge-standing-changed", ());
    }
    nudge(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(45)).await;
        nudge(&app);
    });
}

enum Identity {
    SignedIn {
        user_id: String,
        login: String,
    },
    SignedOut,
    /// Could not tell (offline, Twitch down); keep whatever is known.
    Unknown(String),
}

/// `(hash of the token it was resolved for, user id, login)`.
static TOKEN_IDENTITY: Lazy<std::sync::Mutex<Option<(u64, String, String)>>> =
    Lazy::new(|| std::sync::Mutex::new(None));

fn token_hash(token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}

/// Who is signed in: the account registry's main account, or, when the
/// registry has not recorded one, the owner of the main login's token as
/// Twitch's validate endpoint reports it (once per token). Deliberately not
/// `verify_token_health`, which can clear accounts as a side effect.
async fn signed_in_identity() -> Identity {
    if let Some(account) = AccountStore::primary() {
        return Identity::SignedIn {
            user_id: account.user_id,
            login: account.login,
        };
    }
    let token = match crate::services::twitch_service::TwitchService::get_token().await {
        Ok(t) => t,
        Err(_) => return Identity::SignedOut,
    };
    let hash = token_hash(&token);
    if let Ok(slot) = TOKEN_IDENTITY.lock() {
        if let Some((h, uid, login)) = slot.as_ref() {
            if *h == hash {
                return Identity::SignedIn {
                    user_id: uid.clone(),
                    login: login.clone(),
                };
            }
        }
    }
    let response = crate::services::http::client()
        .get("https://id.twitch.tv/oauth2/validate")
        .header("Authorization", format!("OAuth {}", token))
        .send()
        .await;
    let response = match response {
        Ok(r) => r,
        Err(e) => return Identity::Unknown(e.to_string()),
    };
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Identity::SignedOut;
    }
    if !response.status().is_success() {
        return Identity::Unknown(format!("validate answered {}", response.status()));
    }
    let body: serde_json::Value = match response.json().await {
        Ok(b) => b,
        Err(e) => return Identity::Unknown(e.to_string()),
    };
    match (
        body.get("user_id").and_then(|v| v.as_str()),
        body.get("login").and_then(|v| v.as_str()),
    ) {
        (Some(uid), Some(login)) if !uid.is_empty() => {
            if let Ok(mut slot) = TOKEN_IDENTITY.lock() {
                *slot = Some((hash, uid.to_string(), login.to_string()));
            }
            Identity::SignedIn {
                user_id: uid.to_string(),
                login: login.to_string(),
            }
        }
        _ => Identity::Unknown("validate returned no user".into()),
    }
}

/// Read the collection from Twitch and store the outcome. Runs as one spawned
/// task; the caller never awaits it.
async fn refresh() {
    let now = now_ms();
    let (user_id, login) = match signed_in_identity().await {
        Identity::SignedIn { user_id, login } => (user_id, login),
        Identity::SignedOut => {
            *COLLECTION.write().await = Some(Cached {
                user_id: String::new(),
                login: String::new(),
                ids: Vec::new(),
                state: CollectionState::Unavailable,
                reason: Some("not_signed_in".into()),
                fetched_at_ms: now,
                stale: false,
            });
            return;
        }
        Identity::Unknown(error) => {
            warn!("[BadgeStanding] Could not tell who is signed in: {}", error);
            let mut slot = COLLECTION.write().await;
            if let Some(prev) = slot.as_mut() {
                prev.fetched_at_ms = now;
                prev.stale = true;
                prev.reason = Some("fetch_failed".into());
            }
            return;
        }
    };

    let service_lock = match crate::commands::badge_service::get_service().await {
        Ok(lock) => lock,
        Err(_) => return,
    };
    if service_lock.read().await.is_none() {
        crate::commands::badge_service::initialize_badge_service().await;
    }
    let guard = service_lock.read().await;
    let Some(service) = guard.as_ref() else {
        return;
    };

    let previous = cached_for(Some(&user_id)).await;

    let next = match service.earned_badge_collection(&user_id, &login).await {
        Ok(ids) => {
            let fresh = Cached {
                user_id: user_id.clone(),
                login: login.clone(),
                ids,
                state: CollectionState::Complete,
                reason: None,
                fetched_at_ms: now,
                stale: false,
            };
            let to_save = fresh.clone();
            let _ = tokio::task::spawn_blocking(move || persist(&to_save)).await;
            fresh
        }
        Err(error) => {
            warn!(
                "[BadgeStanding] Could not read the badge collection for @{}: {}",
                login, error
            );
            match previous {
                // Keep the last good read as it was, completeness included, and
                // say it is stale. A blip must not blank the collection.
                Some(prev) => Cached {
                    reason: Some("fetch_failed".into()),
                    fetched_at_ms: now,
                    stale: true,
                    ..prev
                },
                // Nothing known yet: nothing may be claimed missing either.
                None => Cached {
                    user_id: user_id.clone(),
                    login: login.clone(),
                    ids: Vec::new(),
                    state: CollectionState::Partial,
                    reason: Some("fetch_failed".into()),
                    fetched_at_ms: now,
                    stale: false,
                },
            }
        }
    };

    *COLLECTION.write().await = Some(next);
}

/// A global badge from the catalogue.
#[derive(Clone, Debug)]
pub(crate) struct CatalogueBadge {
    pub set_id: String,
    pub version: String,
    pub title: String,
    pub image_url: String,
}

impl CatalogueBadge {
    pub(crate) fn key(&self) -> String {
        format!("{}/{}", self.set_id, self.version)
    }
}

/// The metadata a window is resolved from.
#[derive(Clone, Debug, Default)]
pub(crate) struct BadgeMeta {
    pub more_info: Option<String>,
    pub enrichment: Option<serde_json::Value>,
}

pub(crate) async fn load_catalogue() -> Vec<CatalogueBadge> {
    let response = match crate::commands::badges::get_cached_global_badges().await {
        Ok(Some(r)) if !r.data.is_empty() => Some(r),
        // Nothing cached yet (first run): fetch it once. This shares the
        // global-badges lock with the overlay's own fetch, so the two collapse
        // into one Helix call.
        _ => crate::commands::badges::fetch_global_badges().await.ok(),
    };
    let Some(response) = response else {
        return Vec::new();
    };
    response
        .data
        .into_iter()
        .flat_map(|set| {
            let set_id = set.set_id;
            set.versions.into_iter().map(move |v| {
                let image_url = [v.image_url_4x, v.image_url_2x, v.image_url_1x]
                    .into_iter()
                    .find(|u| !u.is_empty())
                    .unwrap_or_default();
                CatalogueBadge {
                    set_id: set_id.clone(),
                    version: v.id,
                    title: v.title,
                    image_url,
                }
            })
        })
        .collect()
}

fn metadata_key(set_id: &str, version: &str) -> String {
    format!("metadata:{}-v{}", set_id, version)
}

fn meta_from_entry(entry: &universal_cache_service::UniversalCacheEntry) -> BadgeMeta {
    BadgeMeta {
        more_info: entry
            .data
            .get("more_info")
            .and_then(|v| v.as_str())
            .map(String::from),
        enrichment: entry
            .data
            .get("enrichment")
            .filter(|v| !v.is_null())
            .cloned(),
    }
}

/// Metadata for every catalogue badge, keyed `set_id/version`. One read of the
/// cache under its lock.
pub(crate) fn load_metadata(catalogue: &[CatalogueBadge]) -> HashMap<String, BadgeMeta> {
    let keys: Vec<String> = catalogue
        .iter()
        .map(|b| metadata_key(&b.set_id, &b.version))
        .collect();
    let entries = universal_cache_service::get_cached_items_batch(CacheType::Badge, &keys)
        .unwrap_or_default();
    catalogue
        .iter()
        .filter_map(|b| {
            entries
                .get(&metadata_key(&b.set_id, &b.version))
                .map(|e| (b.key(), meta_from_entry(e)))
        })
        .collect()
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The global badges whose art stands for a way of earning, by earn-step kind.
const EARN_ICON_BADGES: [(&str, &str, &str); 2] =
    [("subscribe", "sub-gifter", "1"), ("cheer", "bits", "100")];

fn earn_icons(catalogue: &[CatalogueBadge]) -> HashMap<String, String> {
    EARN_ICON_BADGES
        .iter()
        .filter_map(|(kind, set, version)| {
            catalogue
                .iter()
                .find(|b| b.set_id == *set && b.version == *version && !b.image_url.is_empty())
                .map(|b| (kind.to_string(), b.image_url.clone()))
        })
        .collect()
}

/// A badge's window and earn path read off the live Drops campaign that
/// awards it. Only ever fills a badge whose own metadata has no window: a
/// window the relay pushed or the copy states always wins.
#[derive(Clone, Debug)]
struct CampaignFill {
    runs: Vec<WindowRun>,
    earn: crate::services::badge_earn::EarnPath,
    category: Option<String>,
}

/// The campaign fill for every catalogue badge an active campaign awards.
/// A drop's push can be missed (an Android worker that never saved it, a
/// relay history that has moved on), and without a window the badge never
/// reads as earnable, though Twitch's own Drops list says it is.
fn campaign_fills(
    catalogue: &[CatalogueBadge],
    campaigns: &[crate::models::drops::DropCampaign],
) -> HashMap<String, CampaignFill> {
    use crate::services::badge_earn::{EarnPath, EarnStep};
    let mut fills = HashMap::new();
    if campaigns.is_empty() {
        return fills;
    }
    use crate::services::drops_overview::{badge_tier_for, campaign_for_badge, untyped_reward_for};
    for badge in catalogue {
        // A typed badge reward first, then the reward-campaign list, whose
        // rewards carry no type.
        let found = match campaign_for_badge(campaigns, &badge.set_id, Some(&badge.title)) {
            Some(c) => Some((c, badge_tier_for(c, Some(&badge.title)))),
            None => untyped_reward_for(campaigns, &badge.title).map(|(c, d)| (c, Some(d))),
        };
        let Some((campaign, tier)) = found else {
            continue;
        };
        let mut steps = Vec::new();
        if let Some(d) = tier {
            if d.required_subs > 0 {
                steps.push(EarnStep::Subscribe { count: (d.required_subs > 1).then_some(d.required_subs) });
            }
            if d.required_minutes_watched > 0 {
                // The drop counts its total; a step reads per day.
                let days = d.required_days.max(1);
                steps.push(EarnStep::Watch {
                    minutes: Some(d.required_minutes_watched as u32 / days),
                    days: (days > 1).then_some(days),
                });
            }
        }
        let channels = (campaign.is_acl_based && !campaign.allowed_channels.is_empty())
            .then(|| campaign.allowed_channels.iter().map(|c| c.name.to_lowercase()).collect());
        fills.insert(
            badge.key(),
            CampaignFill {
                runs: vec![WindowRun {
                    start_ms: Some(campaign.start_at.timestamp_millis()),
                    end_ms: Some(campaign.end_at.timestamp_millis()),
                }],
                earn: EarnPath {
                    steps,
                    random_of: tier.and_then(|d| d.random_of),
                    inferred: false,
                    detail: None,
                    channels,
                },
                category: campaign.has_category.then(|| campaign.game_name.clone()),
            },
        );
    }
    fills
}

/// Everything a standing contains, from already-loaded inputs. Pure, so the
/// honesty rules can be tested without Twitch.
fn build_standing(
    login: Option<String>,
    collection: Option<&Cached>,
    refreshing: bool,
    catalogue: &[CatalogueBadge],
    metadata: &HashMap<String, BadgeMeta>,
    now: i64,
) -> BadgeStanding {
    build_standing_with(login, collection, refreshing, catalogue, metadata, &HashMap::new(), now)
}

/// `build_standing`, with windows from the Drops campaigns for badges whose
/// metadata has none.
fn build_standing_with(
    login: Option<String>,
    collection: Option<&Cached>,
    refreshing: bool,
    catalogue: &[CatalogueBadge],
    metadata: &HashMap<String, BadgeMeta>,
    fills: &HashMap<String, CampaignFill>,
    now: i64,
) -> BadgeStanding {
    let mut windows: HashMap<String, Vec<WindowRun>> = HashMap::new();
    let mut next_change_ms: Option<i64> = None;
    for badge in catalogue {
        let key = badge.key();
        let own = metadata
            .get(&key)
            .and_then(|meta| badge_window::resolve(meta.more_info.as_deref(), meta.enrichment.as_ref()))
            .map(|w| w.runs);
        let Some(runs) = own.or_else(|| fills.get(&key).map(|f| f.runs.clone())) else {
            continue;
        };
        if let Some(t) = badge_window::next_boundary_after(&runs, now) {
            next_change_ms = Some(next_change_ms.map_or(t, |c| c.min(t)));
        }
        windows.insert(key, runs);
    }

    let catalogue_ready = !catalogue.is_empty() && !metadata.is_empty();
    let state = collection
        .map(|c| c.state)
        .unwrap_or(CollectionState::Partial);
    let owned: Vec<String> = collection.map(|c| c.ids.clone()).unwrap_or_default();
    let owned_set: HashSet<&str> = owned.iter().map(String::as_str).collect();

    let mut missing_now: Vec<MissingBadge> = Vec::new();
    if state == CollectionState::Complete && catalogue_ready {
        use crate::services::drops_overview::badge_key;
        // Every catalogue title by its comparable key, and whether any badge of
        // that title is owned.
        let mut titles: HashMap<String, bool> = HashMap::new();
        for b in catalogue {
            *titles.entry(badge_key(&b.title)).or_default() |= owned_set.contains(b.key().as_str());
        }
        for badge in catalogue {
            let key = badge.key();
            if owned_set.contains(key.as_str()) {
                continue;
            }
            let Some(runs) = windows.get(&key) else {
                continue;
            };
            if badge_window::status_at(runs, now) != WindowStatus::Available {
                continue;
            }
            let meta = metadata.get(&key);
            let enrichment = meta.and_then(|m| m.enrichment.as_ref());
            let more_info = meta.and_then(|m| m.more_info.as_deref());
            // An evolved badge is earned from the one it evolves from, so it is
            // not earnable now for an account missing that badge.
            if let Some(base) = crate::services::badge_earn::evolves_from(enrichment, more_info) {
                if titles.get(&badge_key(&base)) == Some(&false) {
                    continue;
                }
            }
            // The relay has filed one badge's writeup under its sibling (Witch
            // Hat carried Witchspire Logo's "watch 60 minutes"). A headline that
            // names another catalogue badge means these steps are not this one's.
            let foreign = enrichment
                .and_then(|e| string_field(e, "headline"))
                .map(|h| badge_key(&h))
                .is_some_and(|h| h != badge_key(&badge.title) && titles.contains_key(&h));
            let mut earn = if foreign {
                crate::services::badge_earn::earn_path(None, None)
            } else {
                crate::services::badge_earn::earn_path(enrichment, more_info)
            };
            let mut category = enrichment
                .and_then(|e| e.get("requirement"))
                .and_then(|r| string_field(r, "category"))
                .or_else(|| enrichment.and_then(|e| string_field(e, "category")));
            // Where the badge's own copy says nothing, the campaign's numbers do.
            if let Some(fill) = fills.get(&key) {
                let says_nothing = earn
                    .steps
                    .iter()
                    .all(|s| matches!(s, crate::services::badge_earn::EarnStep::Other));
                if says_nothing {
                    earn = fill.earn.clone();
                }
                if category.is_none() {
                    category = fill.category.clone();
                }
            }
            missing_now.push(MissingBadge {
                key: key.clone(),
                set_id: badge.set_id.clone(),
                version: badge.version.clone(),
                title: badge.title.clone(),
                image_url: badge.image_url.clone(),
                ends_ms: badge_window::run_containing(runs, now).and_then(|r| r.end_ms),
                earn,
                category,
            });
        }
        missing_now.sort_by(|a, b| match (a.ends_ms, b.ends_ms) {
            (Some(x), Some(y)) => x.cmp(&y).then_with(|| a.title.cmp(&b.title)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.title.cmp(&b.title),
        });
    }

    BadgeStanding {
        login,
        collection: state,
        collection_reason: collection.and_then(|c| c.reason.clone()),
        stale: collection.map(|c| c.stale).unwrap_or(true),
        refreshing,
        catalogue_ready,
        owned,
        windows,
        next_change_ms,
        earn_icons: earn_icons(catalogue),
        catch_up: catch_up(&missing_now),
        missing_now,
        generated_ms: now,
    }
}

/// Totals for `CatchUp`. Badges are grouped by category (a badge with none is
/// its own group), because effort in one category counts toward all of it.
fn catch_up(missing: &[MissingBadge]) -> Option<CatchUp> {
    use crate::services::badge_earn::EarnStep;
    if missing.is_empty() {
        return None;
    }
    // Per group: the most subs any badge needs, and each badge's daily watch.
    let mut groups: HashMap<String, (u32, Vec<(u32, u32)>)> = HashMap::new();
    let mut out = CatchUp::default();
    for badge in missing {
        let group = badge
            .category
            .as_deref()
            .map(str::to_lowercase)
            .unwrap_or_else(|| badge.key.clone());
        let entry = groups.entry(group).or_default();
        let mut priced = false;
        for step in &badge.earn.steps {
            match step {
                EarnStep::Subscribe { count } => {
                    entry.0 = entry.0.max(count.unwrap_or(1));
                    priced = true;
                }
                EarnStep::Watch {
                    minutes: Some(m),
                    days,
                } => {
                    entry.1.push((*m, days.unwrap_or(1).max(1)));
                    priced = true;
                }
                EarnStep::Purchase { ticket: true } => {
                    out.tickets += 1;
                    priced = true;
                }
                _ => {}
            }
        }
        if !priced {
            out.unpriced += 1;
        }
        if badge.earn.random_of.is_some_and(|n| n > 1) {
            out.random += 1;
        }
        out.estimated |= badge.earn.inferred;
    }
    for (subs, watches) in groups.values() {
        out.subs += subs;
        // Day d needs the most any badge still asking for a day d wants.
        let longest = watches.iter().map(|&(_, d)| d).max().unwrap_or(0);
        for day in 0..longest {
            out.watch_minutes += watches
                .iter()
                .filter(|&&(_, d)| d > day)
                .map(|&(m, _)| m)
                .max()
                .unwrap_or(0);
        }
    }
    out.sub_cost_cents = out.subs * SUB_PRICE_US_CENTS;
    Some(out)
}

/// Twitch has changed id formats before without a word. When most of a fresh
/// collection matches nothing in the catalogue, say so once in a normal log.
fn check_id_drift(collection: &Cached, catalogue: &[CatalogueBadge]) {
    if collection.state != CollectionState::Complete || collection.stale || catalogue.is_empty() {
        return;
    }
    if DRIFT_CHECKED_AT.swap(collection.fetched_at_ms, Ordering::AcqRel) == collection.fetched_at_ms
    {
        return;
    }
    let known: HashSet<String> = catalogue.iter().map(CatalogueBadge::key).collect();
    let unmatched = collection
        .ids
        .iter()
        .filter(|id| !known.contains(*id))
        .count();
    if unmatched * 4 > collection.ids.len() {
        warn!(
            "[BadgeStanding] {} of {} owned badge ids match nothing in the catalogue; the id format may have changed",
            unmatched,
            collection.ids.len()
        );
    } else {
        debug!(
            "[BadgeStanding] {} owned badges, {} outside the global catalogue",
            collection.ids.len(),
            unmatched
        );
    }
}

/// The signed-in account's standing, answered from cache. A due refresh is
/// spawned in the background and announced with `badge-standing-changed`; it
/// also settles who is signed in when the account registry has not said.
/// A forced read (Badges opened, or a pull to refresh) renews a global badge
/// list older than six hours, in the background so opening Badges never waits
/// on Twitch, then has the wall read again. The list is otherwise kept up to
/// a week, which left new drop badges off the wall for days.
fn refresh_stale_catalogue(app: &crate::rt::AppHandle) {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let stale = crate::commands::badges::global_badges_age_secs()
            .await
            .map_or(true, |age| age > 6 * 60 * 60);
        if stale && crate::commands::badges::refresh_global_badges().await.is_ok() {
            use tauri::Emitter;
            let _ = app.emit("badge-standing-changed", ());
        }
        RUNNING.store(false, Ordering::Release);
    });
}

pub async fn get_standing(app: &crate::rt::AppHandle, force: bool) -> BadgeStanding {
    if force {
        refresh_stale_catalogue(app);
    }
    let now = now_ms();
    let primary = AccountStore::primary();
    let cached = cached_for(primary.as_ref().map(|a| a.user_id.as_str())).await;

    let due = force || cached.as_ref().map(|c| c.is_due(now)).unwrap_or(true);
    if due
        && REFRESHING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _guard = RefreshGuard;
            refresh().await;
            use tauri::Emitter;
            let _ = app.emit("badge-standing-changed", ());
        });
    }
    let refreshing = REFRESHING.load(Ordering::Acquire);

    if cached.as_ref().map(|c| c.state) == Some(CollectionState::Unavailable) {
        return build_standing(None, cached.as_ref(), refreshing, &[], &HashMap::new(), now);
    }

    let catalogue = load_catalogue().await;
    let campaigns = active_campaigns(app).await;
    let login = primary
        .map(|a| a.login)
        .or_else(|| cached.as_ref().map(|c| c.login.clone()))
        .filter(|l| !l.is_empty());
    let fallback_login = login.clone();
    let started = std::time::Instant::now();
    let standing = tokio::task::spawn_blocking(move || {
        let metadata = load_metadata(&catalogue);
        if let Some(c) = cached.as_ref() {
            check_id_drift(c, &catalogue);
        }
        let fills = campaign_fills(&catalogue, &campaigns);
        build_standing_with(
            login,
            cached.as_ref(),
            refreshing,
            &catalogue,
            &metadata,
            &fills,
            now,
        )
    })
    .await;

    match standing {
        Ok(s) => {
            debug!(
                "[BadgeStanding] {} windows, {} missing now, built in {} ms",
                s.windows.len(),
                s.missing_now.len(),
                started.elapsed().as_millis()
            );
            s
        }
        Err(e) => {
            warn!("[BadgeStanding] Building the standing failed: {}", e);
            build_standing(fallback_login, None, refreshing, &[], &HashMap::new(), now)
        }
    }
}

/// The Drops campaigns still running, from the Drops service's cached list
/// at any age (Rewards opens on Drops, which reads it). Never a fetch from
/// here: a fresh read resets the live drop progress, which a badge wall has no
/// business doing.
async fn active_campaigns(app: &crate::rt::AppHandle) -> Vec<crate::models::drops::DropCampaign> {
    use tauri::Manager;
    let Some(state) = app.try_state::<crate::models::settings::AppState>() else {
        return Vec::new();
    };
    let snapshot = state.drops_service.lock().await.cached_campaigns_snapshot().await;
    let Some(mut campaigns) = snapshot else {
        return Vec::new();
    };
    let now = chrono::Utc::now();
    campaigns.retain(|c| c.end_at > now);
    campaigns
}

/// One badge's earn window, for a surface that shows a single badge. Falls back
/// to the live campaign that awards it exactly as the standing does, so the
/// badge page and the gallery never disagree.
pub async fn window_for(
    app: &crate::rt::AppHandle,
    set_id: &str,
    version: &str,
) -> Option<Vec<WindowRun>> {
    let own = universal_cache_service::peek_cached_entry(&metadata_key(set_id, version))
        .ok()
        .flatten()
        .filter(|entry| entry.cache_type == CacheType::Badge)
        .and_then(|entry| {
            let meta = meta_from_entry(&entry);
            badge_window::resolve(meta.more_info.as_deref(), meta.enrichment.as_ref())
        })
        .map(|w| w.runs);
    if own.is_some() {
        return own;
    }
    let badge = load_catalogue()
        .await
        .into_iter()
        .find(|b| b.set_id == set_id && b.version == version)?;
    let key = badge.key();
    campaign_fills(&[badge], &active_campaigns(app).await)
        .remove(&key)
        .map(|f| f.runs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_790_000_000_000;

    fn badge(set: &str, version: &str, title: &str) -> CatalogueBadge {
        CatalogueBadge {
            set_id: set.into(),
            version: version.into(),
            title: title.into(),
            image_url: format!("https://img/{set}/{version}"),
        }
    }

    fn window(start: i64, end: i64) -> BadgeMeta {
        let iso = |ms: i64| {
            chrono::DateTime::from_timestamp_millis(ms)
                .unwrap()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        BadgeMeta {
            more_info: None,
            enrichment: Some(
                json!({ "starts_utc": iso(start), "ends_utc": iso(end), "action": "Watch it" }),
            ),
        }
    }

    fn collection(ids: &[&str], state: CollectionState) -> Cached {
        Cached {
            user_id: "1".into(),
            login: "me".into(),
            ids: ids.iter().map(|s| s.to_string()).collect(),
            state,
            reason: None,
            fetched_at_ms: NOW,
            stale: false,
        }
    }

    const HOUR: i64 = 3_600_000;

    fn fixture() -> (Vec<CatalogueBadge>, HashMap<String, BadgeMeta>) {
        let catalogue = vec![
            badge("wolf", "1", "Wolf"),
            badge("chains", "1", "Chains"),
            badge("old", "1", "Old"),
            badge("soon", "1", "Soon"),
            badge("permanent", "1", "Permanent"),
        ];
        let mut meta = HashMap::new();
        meta.insert("wolf/1".into(), window(NOW - HOUR, NOW + 48 * HOUR));
        meta.insert("chains/1".into(), window(NOW - HOUR, NOW + 2 * HOUR));
        meta.insert("old/1".into(), window(NOW - 48 * HOUR, NOW - HOUR));
        meta.insert("soon/1".into(), window(NOW + HOUR, NOW + 48 * HOUR));
        meta.insert(
            "permanent/1".into(),
            BadgeMeta {
                more_info: Some("Given to subscribers.".into()),
                enrichment: None,
            },
        );
        (catalogue, meta)
    }

    #[test]
    fn lists_unowned_badges_earnable_now_ending_soonest_first() {
        let (catalogue, meta) = fixture();
        let c = collection(&["other/1"], CollectionState::Complete);
        let s = build_standing(Some("me".into()), Some(&c), false, &catalogue, &meta, NOW);
        let keys: Vec<&str> = s.missing_now.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, vec!["chains/1", "wolf/1"]);
        assert_eq!(s.missing_now[0].earn.detail.as_deref(), Some("Watch it"));
        assert_eq!(s.next_change_ms, Some(NOW + HOUR));
        assert!(!s.windows.contains_key("permanent/1"));
    }

    #[test]
    fn a_campaign_fills_only_a_badge_with_no_window_of_its_own() {
        use crate::services::badge_earn::{EarnPath, EarnStep};
        let (mut catalogue, meta) = fixture();
        catalogue.push(badge("pushless", "1", "Pushless"));
        let fill = |start: i64, end: i64| CampaignFill {
            runs: vec![WindowRun { start_ms: Some(start), end_ms: Some(end) }],
            earn: EarnPath {
                steps: vec![EarnStep::Subscribe { count: None }],
                random_of: None,
                inferred: false,
                detail: None,
                channels: None,
            },
            category: Some("Space Marine 2".into()),
        };
        let mut fills = HashMap::new();
        // No metadata at all: the campaign's window and numbers fill it.
        fills.insert("pushless/1".to_string(), fill(NOW - HOUR, NOW + 11 * 24 * HOUR));
        // Its own pushed window says it ended: the campaign never overrides that.
        fills.insert("old/1".to_string(), fill(NOW - HOUR, NOW + 24 * HOUR));
        let c = collection(&["other/1"], CollectionState::Complete);
        let s = build_standing_with(None, Some(&c), false, &catalogue, &meta, &fills, NOW);
        let keys: Vec<&str> = s.missing_now.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, vec!["chains/1", "wolf/1", "pushless/1"]);
        let pushless = s.missing_now.iter().find(|m| m.key == "pushless/1").unwrap();
        assert_eq!(pushless.earn.steps, vec![EarnStep::Subscribe { count: None }]);
        assert_eq!(pushless.category.as_deref(), Some("Space Marine 2"));
        assert_eq!(pushless.ends_ms, Some(NOW + 11 * 24 * HOUR));
    }

    #[test]
    fn earn_chips_get_twitchs_own_gift_and_bits_art() {
        let mut catalogue = fixture().0;
        catalogue.push(badge("sub-gifter", "1", "Sub Gifter"));
        catalogue.push(badge("bits", "1", "cheer 1"));
        catalogue.push(badge("bits", "100", "cheer 100"));
        let icons = earn_icons(&catalogue);
        assert_eq!(
            icons.get("subscribe").map(String::as_str),
            Some("https://img/sub-gifter/1")
        );
        assert_eq!(
            icons.get("cheer").map(String::as_str),
            Some("https://img/bits/100")
        );
        assert!(earn_icons(&fixture().0).is_empty());
    }

    #[test]
    fn an_owned_badge_is_not_missing() {
        let (catalogue, meta) = fixture();
        let c = collection(&["wolf/1"], CollectionState::Complete);
        let s = build_standing(None, Some(&c), false, &catalogue, &meta, NOW);
        assert_eq!(s.missing_now.len(), 1);
        assert_eq!(s.missing_now[0].key, "chains/1");
    }

    #[test]
    fn a_partial_collection_never_claims_anything_is_missing() {
        let (catalogue, meta) = fixture();
        let c = collection(&[], CollectionState::Partial);
        let s = build_standing(None, Some(&c), false, &catalogue, &meta, NOW);
        assert!(s.missing_now.is_empty());
        assert_eq!(s.collection, CollectionState::Partial);
        let none = build_standing(None, None, true, &catalogue, &meta, NOW);
        assert!(none.missing_now.is_empty());
        assert!(none.stale);
    }

    #[test]
    fn an_unloaded_catalogue_is_not_ready_and_lists_nothing() {
        let c = collection(&["x/1"], CollectionState::Complete);
        let s = build_standing(None, Some(&c), false, &[], &HashMap::new(), NOW);
        assert!(!s.catalogue_ready);
        assert!(s.missing_now.is_empty());
        let (catalogue, _) = fixture();
        let s = build_standing(None, Some(&c), false, &catalogue, &HashMap::new(), NOW);
        assert!(!s.catalogue_ready);
    }

    #[test]
    fn owning_one_version_does_not_hide_an_earnable_relaunch() {
        let catalogue = vec![
            badge("glitch", "1", "Glitch"),
            badge("glitch", "2", "Glitch"),
        ];
        let mut meta = HashMap::new();
        meta.insert("glitch/2".into(), window(NOW - HOUR, NOW + HOUR));
        let c = collection(&["glitch/1"], CollectionState::Complete);
        let s = build_standing(None, Some(&c), false, &catalogue, &meta, NOW);
        assert_eq!(
            s.missing_now
                .iter()
                .map(|m| m.key.as_str())
                .collect::<Vec<_>>(),
            vec!["glitch/2"]
        );
    }

    #[test]
    fn catch_up_shares_effort_within_a_category_and_adds_across_them() {
        use crate::services::badge_earn::{EarnPath, EarnStep};
        let m = |key: &str, cat: Option<&str>, steps: Vec<EarnStep>, random_of: Option<u32>| {
            MissingBadge {
                key: key.into(),
                set_id: key.into(),
                version: "1".into(),
                title: key.into(),
                image_url: String::new(),
                ends_ms: None,
                earn: EarnPath {
                    steps,
                    random_of,
                    inferred: false,
                    detail: None,
                    channels: None,
                },
                category: cat.map(Into::into),
            }
        };
        let missing = vec![
            // One game: a sub plus 60 min, and 20 min on each of 3 days. The
            // 60 covers day one of the other, so 60 + 20 + 20.
            m(
                "a",
                Some("Pokemon"),
                vec![
                    EarnStep::Subscribe { count: None },
                    EarnStep::Watch {
                        minutes: Some(60),
                        days: None,
                    },
                ],
                None,
            ),
            m(
                "b",
                Some("pokemon"),
                vec![EarnStep::Watch {
                    minutes: Some(20),
                    days: Some(3),
                }],
                Some(3),
            ),
            // Another game needs its own two subs and its own watching.
            m(
                "c",
                Some("CONTROL"),
                vec![
                    EarnStep::Subscribe { count: Some(2) },
                    EarnStep::Watch {
                        minutes: Some(30),
                        days: None,
                    },
                ],
                None,
            ),
            m("d", None, vec![EarnStep::Purchase { ticket: true }], None),
            m("e", None, vec![EarnStep::Cheer], None),
        ];
        let c = catch_up(&missing).expect("totals");
        assert_eq!(c.subs, 3);
        assert_eq!(c.sub_cost_cents, 3 * 599);
        assert_eq!(c.watch_minutes, 100 + 30);
        assert_eq!(c.tickets, 1);
        assert_eq!(c.random, 1);
        assert_eq!(c.unpriced, 1);
        assert!(!c.estimated);
        assert_eq!(catch_up(&[]), None);
    }

    #[test]
    fn a_fresh_complete_collection_waits_longer_than_an_incomplete_one() {
        let fresh = collection(&["a/1"], CollectionState::Complete);
        assert!(!fresh.is_due(NOW + INCOMPLETE_TTL_MS));
        assert!(fresh.is_due(NOW + COMPLETE_TTL_MS));
        let partial = collection(&[], CollectionState::Partial);
        assert!(partial.is_due(NOW + INCOMPLETE_TTL_MS));
        let mut stale = collection(&["a/1"], CollectionState::Complete);
        stale.stale = true;
        assert!(stale.is_due(NOW + INCOMPLETE_TTL_MS));
    }

    #[test]
    fn an_evolved_badge_waits_for_the_badge_it_evolves_from() {
        let catalogue = vec![
            badge("pichu", "1", "Pichu"),
            badge("pikachu", "1", "Pikachu"),
            badge("bulbasaur", "1", "Bulbasaur"),
            badge("ivysaur", "1", "Ivysaur"),
            badge("venusaur", "1", "Venusaur"),
        ];
        let evolves = |base: &str| BadgeMeta {
            more_info: Some(format!(
                "An evolved form of the {base} global chat badge. Evolution is available until January 30, 2099."
            )),
            enrichment: None,
        };
        let mut meta = HashMap::new();
        meta.insert("pikachu/1".to_string(), evolves("Pichu"));
        meta.insert("ivysaur/1".to_string(), evolves("Bulbasaur"));
        meta.insert("venusaur/1".to_string(), evolves("Ivysaur"));
        let c = collection(&["pichu/1", "bulbasaur/1"], CollectionState::Complete);
        let s = build_standing(None, Some(&c), false, &catalogue, &meta, NOW);
        let keys: Vec<&str> = s.missing_now.iter().map(|m| m.key.as_str()).collect();
        // Venusaur needs Ivysaur, which this account does not hold yet.
        assert_eq!(keys, vec!["ivysaur/1", "pikachu/1"]);
        // The gallery still reads all three as open.
        assert!(s.windows.contains_key("venusaur/1"));
    }

    #[test]
    fn the_relays_stated_base_counts_where_its_prose_dropped_the_sentence() {
        let catalogue = vec![badge("gastly", "1", "Gastly"), badge("haunter", "1", "Haunter")];
        let mut haunter = window(NOW - HOUR, NOW + 48 * HOUR);
        haunter.more_info = Some("Haunter continues the line that began with Gastly.".into());
        haunter.enrichment.as_mut().unwrap()["evolves_from"] = json!("Gastly");
        let mut meta = HashMap::new();
        meta.insert("haunter/1".to_string(), haunter);
        let without = collection(&["other/1"], CollectionState::Complete);
        let s = build_standing(None, Some(&without), false, &catalogue, &meta, NOW);
        assert!(s.missing_now.is_empty(), "Gastly is not held, so Haunter cannot be started");
        let with = collection(&["gastly/1"], CollectionState::Complete);
        let s = build_standing(None, Some(&with), false, &catalogue, &meta, NOW);
        assert_eq!(s.missing_now.len(), 1);
    }

    #[test]
    fn a_writeup_filed_under_a_sibling_badge_gives_no_steps() {
        use crate::services::badge_earn::EarnStep;
        let catalogue = vec![
            badge("witch-hat", "1", "Witch Hat"),
            badge("witchspire-logo", "1", "Witchspire Logo"),
            badge("ultramarine", "1", "Ultramarine"),
        ];
        let writeup = |headline: &str| {
            let mut m = window(NOW - HOUR, NOW + 24 * HOUR);
            let e = m.enrichment.as_mut().unwrap();
            e["headline"] = json!(headline);
            e["action"] = json!("Watch 60 minutes of any livestream.");
            m
        };
        let mut meta = HashMap::new();
        meta.insert("witch-hat/1".to_string(), writeup("Witchspire Logo"));
        // A headline worded differently still names this badge.
        meta.insert("ultramarine/1".to_string(), writeup("Ultramarine Badge"));
        let c = collection(&["other/1"], CollectionState::Complete);
        let s = build_standing(None, Some(&c), false, &catalogue, &meta, NOW);
        let steps = |key: &str| s.missing_now.iter().find(|m| m.key == key).unwrap().earn.steps.clone();
        assert_eq!(steps("witch-hat/1"), vec![EarnStep::Other]);
        assert!(matches!(steps("ultramarine/1").as_slice(), [EarnStep::Watch { .. }]));
    }
}
