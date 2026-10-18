//! Who made an emote, for the hover cards in chat and the emote menu.
//!
//! Resolved lazily, one emote at a time, when a hover card opens: nothing rides
//! on chat messages. The emote sets Rust already holds answer first (7TV, BTTV
//! and FFZ carry their uploader), then the provider's per-emote API:
//!
//! - 7TV `GET /v3/emotes/{id}` (owner, original name)
//! - FFZ `GET /v1/emote/{id}` (owner, artist)
//! - Twitch GQL `emote(id)` (owner channel and artist; Helix has no per-id
//!   lookup and no artist field)
//!
//! BetterTTV's per-emote endpoint only answers requests that claim to come from
//! betterttv.com, so a BTTV emote outside every loaded set gets no credit.
//!
//! Clicking an emote asks for the detail: each creator as a chip with their
//! picture and where it goes (a Twitch account opens StreamNook's profile
//! view; 7TV, BetterTTV and FFZ open the creator's page there), plus the
//! emote's own page. Links are built here from validated ids.
//!
//! Answers are cached per (provider, id) in a bounded LRU, and concurrent asks
//! for the same emote share one request.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use log::debug;
use lru::LruCache;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde::Serialize;
use tokio::sync::OnceCell;

use crate::services::emote_service::{Emote, EmoteSet};

const CACHE_CAPACITY: usize = 512;
/// A miss (unknown emote, failed request) is asked again after this long.
const NEGATIVE_TTL: Duration = Duration::from_secs(10 * 60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
const WEB_CLIENT_ID: &str = env!("TWITCH_WEB_CLIENT_ID");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CreditProvider {
    Twitch,
    SevenTv,
    Bttv,
    Ffz,
}

impl CreditProvider {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "twitch" => Some(Self::Twitch),
            "7tv" | "seventv" => Some(Self::SevenTv),
            "bttv" | "betterttv" => Some(Self::Bttv),
            "ffz" | "frankerfacez" => Some(Self::Ffz),
            _ => None,
        }
    }

    fn wire(self) -> &'static str {
        match self {
            Self::Twitch => "twitch",
            Self::SevenTv => "7tv",
            Self::Bttv => "bttv",
            Self::Ffz => "ffz",
        }
    }

    /// Whether `id` has this provider's shape. Ids go into request URLs, so
    /// anything else is refused before a request is built.
    pub fn valid_id(self, id: &str) -> bool {
        let alnum = |c: char| c.is_ascii_alphanumeric();
        match self {
            // ULID (26) or a legacy 24-hex ObjectId.
            Self::SevenTv => (id.len() == 24 || id.len() == 26) && id.chars().all(alnum),
            Self::Bttv => id.len() == 24 && id.chars().all(|c| c.is_ascii_hexdigit()),
            Self::Ffz => !id.is_empty() && id.len() <= 12 && id.chars().all(|c| c.is_ascii_digit()),
            // `25`, `emotesv2_<32 hex>`.
            Self::Twitch => {
                let body = id.strip_prefix("emotesv2_").unwrap_or(id);
                !body.is_empty() && id.len() <= 64 && body.chars().all(|c| c.is_ascii_alphanumeric())
            }
        }
    }
}

/// What the hover card shows under the emote's name, and (in detail mode) the
/// creator chips and links the emote card shows when the emote is clicked.
#[derive(Serialize, Debug, Clone, Default, PartialEq)]
pub struct EmoteCredit {
    pub provider: &'static str,
    /// The uploader (7TV, BTTV, FFZ) or the channel that owns it (Twitch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// The person who drew it, where the provider records one (Twitch, FFZ).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artist_name: Option<String>,
    /// The emote's own name when the channel shows it under an alias.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_name: Option<String>,
    /// "Sub emote", "Global emote", "Channel emote", ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_label: Option<&'static str>,
    /// The people to credit, artist first, each with where their chip goes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub creators: Vec<Creator>,
    /// The emote's own page on its provider (none for Twitch).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
}

/// One person on the emote card.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Creator {
    /// `artist` (drew it), `uploader` (added it to the provider) or `channel`
    /// (a Twitch emote's channel).
    pub role: &'static str,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    pub link: CreatorLink,
}

/// Where a creator chip goes: a Twitch account opens StreamNook's own profile
/// view; everyone else their page on the emote's provider.
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreatorLink {
    Twitch {
        login: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    Web {
        url: String,
        site: &'static str,
    },
}

impl EmoteCredit {
    fn has_credit(&self) -> bool {
        self.owner_name.is_some() || self.artist_name.is_some()
    }
}

// ── link and avatar hygiene ───────────────────────────────────────────────
// Everything below lands in an <a> or <img> the page renders, so each piece
// is checked for its provider's shape and every link is built here, never
// passed through from a payload.

fn is_seventv_user_id(s: &str) -> bool {
    (s.len() == 26 || s.len() == 24) && s.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_bttv_user_id(s: &str) -> bool {
    s.len() == 24 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_ffz_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_twitch_login(s: &str) -> bool {
    !s.is_empty() && s.len() <= 25 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_twitch_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 20 && s.chars().all(|c| c.is_ascii_digit())
}

/// An https image URL, or None. 7TV hands out protocol-relative ones.
fn avatar(url: Option<String>) -> Option<String> {
    let url = url?;
    let url = if url.starts_with("//") { format!("https:{url}") } else { url };
    (url.starts_with("https://") && url.len() <= 512 && !url.chars().any(|c| c.is_whitespace() || c == '"'))
        .then_some(url)
}

fn seventv_creator(id: Option<String>, name: String, avatar_url: Option<String>) -> Option<Creator> {
    let id = id.filter(|i| is_seventv_user_id(i))?;
    Some(Creator {
        role: "uploader",
        name,
        avatar_url: avatar(avatar_url),
        link: CreatorLink::Web { url: format!("https://7tv.app/users/{id}"), site: "7TV" },
    })
}

fn bttv_creator(id: Option<String>, name: String, avatar_url: Option<String>) -> Option<Creator> {
    let id = id.filter(|i| is_bttv_user_id(i))?;
    Some(Creator {
        role: "uploader",
        name,
        avatar_url: avatar(avatar_url),
        link: CreatorLink::Web { url: format!("https://betterttv.com/users/{id}"), site: "BetterTTV" },
    })
}

fn ffz_creator(role: &'static str, login: Option<String>, name: String) -> Option<Creator> {
    let login = login.filter(|l| is_ffz_name(l))?;
    Some(Creator {
        role,
        name,
        avatar_url: None,
        link: CreatorLink::Web {
            url: format!("https://www.frankerfacez.com/{}/submissions", login.to_lowercase()),
            site: "FrankerFaceZ",
        },
    })
}

fn twitch_creator(role: &'static str, person: &serde_json::Value) -> Option<Creator> {
    let login = non_empty(person.get("login")).filter(|l| is_twitch_login(l))?;
    Some(Creator {
        role,
        name: non_empty(person.get("displayName")).unwrap_or_else(|| login.clone()),
        avatar_url: avatar(non_empty(person.get("profileImageURL"))),
        link: CreatorLink::Twitch {
            id: non_empty(person.get("id")).filter(|i| is_twitch_id(i)),
            login: login.to_lowercase(),
        },
    })
}

/// The emote's page on its provider. Ids were validated by the caller.
fn page_url(provider: CreditProvider, id: &str) -> Option<String> {
    match provider {
        CreditProvider::SevenTv => Some(format!("https://7tv.app/emotes/{id}")),
        CreditProvider::Bttv => Some(format!("https://betterttv.com/emotes/{id}")),
        CreditProvider::Ffz => Some(format!("https://www.frankerfacez.com/emoticon/{id}")),
        CreditProvider::Twitch => None,
    }
}

// ── caches ────────────────────────────────────────────────────────────────

type Cell<T> = Arc<OnceCell<(Option<T>, Instant)>>;
type Memo<T> = Mutex<LruCache<String, Cell<T>>>;

fn credit_cache() -> &'static Memo<EmoteCredit> {
    static CACHE: OnceLock<Memo<EmoteCredit>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(LruCache::new(NonZeroUsize::new(CACHE_CAPACITY).unwrap())))
}

/// Creator avatars looked up on the side (a BetterTTV uploader's Twitch
/// picture, an FFZ user's), keyed `tw:{twitch id}` / `ffz:{name}`.
fn avatar_cache() -> &'static Memo<String> {
    static CACHE: OnceLock<Memo<String>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(LruCache::new(NonZeroUsize::new(CACHE_CAPACITY).unwrap())))
}

/// Forget every answer (the emote cache's "clear" action).
pub fn clear() {
    if let Ok(mut c) = credit_cache().lock() {
        c.clear();
    }
    if let Ok(mut c) = avatar_cache().lock() {
        c.clear();
    }
}

/// The cell for one key: the cached answer, or the one request in flight.
/// A stale miss is replaced so it is asked again.
fn cell_for<T>(memo: &Memo<T>, key: &str) -> Cell<T> {
    let mut c = memo.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cell) = c.get(key) {
        let stale_miss = matches!(cell.get(), Some((None, at)) if at.elapsed() > NEGATIVE_TTL);
        if !stale_miss {
            return cell.clone();
        }
    }
    let cell: Cell<T> = Arc::new(OnceCell::new());
    c.put(key.to_string(), cell.clone());
    cell
}

async fn memo<T, F, Fut>(memo_ref: &'static Memo<T>, key: String, fetch: F) -> Option<T>
where
    T: Clone,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let cell = cell_for(memo_ref, &key);
    cell.get_or_init(|| async {
        let got = fetch().await;
        if got.is_none() {
            debug!("[EmoteCredit] nothing for {}", key);
        }
        (got, Instant::now())
    })
    .await
    .0
    .clone()
}

// ── resolve ───────────────────────────────────────────────────────────────

/// Credit for one emote. `shown_name` is the name chat or the menu shows (an
/// alias reveals itself against the provider's own name). `held` looks the id
/// up in the sets Rust already holds; it runs before any request. `detail`
/// is the click card: creators with avatars and links, and the emote's page.
pub async fn resolve(
    provider: CreditProvider,
    id: &str,
    shown_name: Option<&str>,
    held: Option<HeldEmote>,
    detail: bool,
) -> Option<EmoteCredit> {
    if !provider.valid_id(id) {
        return None;
    }
    let mut credit = if provider == CreditProvider::Bttv {
        // No public per-id endpoint: what the sets hold is all there is.
        let mut c = held.as_ref().map(|h| credit_from_held(provider, h)).unwrap_or(EmoteCredit {
            provider: provider.wire(),
            ..Default::default()
        });
        if detail {
            if let Some(h) = &held {
                let pic = match h.owner_twitch_id.as_deref() {
                    Some(tid) => twitch_avatar(tid).await,
                    None => None,
                };
                c.creators = h
                    .owner_name
                    .clone()
                    .and_then(|name| bttv_creator(h.owner_ref.clone(), name, pic))
                    .into_iter()
                    .collect();
            }
        }
        c
    } else {
        // A hover is answered from the sets when they know the uploader; the
        // click card always asks the provider, whose answer carries avatars.
        let from_sets = held
            .as_ref()
            .filter(|_| !detail && provider != CreditProvider::Twitch)
            .map(|h| credit_from_held(provider, h))
            .filter(EmoteCredit::has_credit);
        match from_sets {
            Some(c) => c,
            None => {
                let key = format!("{}:{}", provider.wire(), id);
                let fetched = memo(credit_cache(), key, || async move {
                    match provider {
                        CreditProvider::SevenTv => fetch_seventv(id).await,
                        CreditProvider::Ffz => fetch_ffz(id).await,
                        CreditProvider::Twitch => fetch_twitch(id).await,
                        CreditProvider::Bttv => None,
                    }
                })
                .await;
                let mut c = fetched.unwrap_or(EmoteCredit { provider: provider.wire(), ..Default::default() });
                if detail && provider == CreditProvider::Ffz {
                    for person in &mut c.creators {
                        if let CreatorLink::Web { url, .. } = &person.link {
                            let name = url
                                .trim_start_matches("https://www.frankerfacez.com/")
                                .trim_end_matches("/submissions")
                                .to_string();
                            person.avatar_url = ffz_avatar(&name).await;
                        }
                    }
                }
                c
            }
        }
    };
    if !detail {
        // The hover card shows names only; links ride on the click card.
        credit.creators.clear();
    } else {
        credit.page_url = page_url(provider, id);
    }
    // The alias is a property of where the emote is shown, not of the emote.
    credit.original_name = match (&credit.original_name, shown_name) {
        (Some(own), Some(shown)) if own != shown => Some(own.clone()),
        (Some(_), None) => credit.original_name.clone(),
        _ => None,
    };
    if let Some(h) = held {
        if credit.kind_label.is_none() {
            credit.kind_label = h.kind_label;
        }
    }
    Some(credit)
}

/// An emote found in a set Rust holds, with where it sat.
#[derive(Clone, Debug)]
pub struct HeldEmote {
    pub owner_name: Option<String>,
    pub original_name: Option<String>,
    pub kind_label: Option<&'static str>,
    /// The creator's id on the provider (7TV / BTTV id, FFZ name).
    pub owner_ref: Option<String>,
    /// The creator's Twitch id, where the set knows it (BetterTTV).
    pub owner_twitch_id: Option<String>,
}

fn credit_from_held(provider: CreditProvider, h: &HeldEmote) -> EmoteCredit {
    let creators = h
        .owner_name
        .clone()
        .and_then(|name| match provider {
            CreditProvider::SevenTv => seventv_creator(h.owner_ref.clone(), name, None),
            CreditProvider::Bttv => bttv_creator(h.owner_ref.clone(), name, None),
            CreditProvider::Ffz => ffz_creator("uploader", h.owner_ref.clone(), name),
            CreditProvider::Twitch => None,
        })
        .into_iter()
        .collect();
    EmoteCredit {
        provider: provider.wire(),
        owner_name: h.owner_name.clone(),
        artist_name: None,
        original_name: h.original_name.clone(),
        kind_label: h.kind_label,
        creators,
        page_url: None,
    }
}

/// Look `id` up in one emote set, by provider.
pub fn find_in_set(set: &EmoteSet, provider: CreditProvider, id: &str, seventv_global: bool) -> Option<HeldEmote> {
    let list: &[Emote] = match provider {
        CreditProvider::Twitch => &set.twitch,
        CreditProvider::SevenTv => &set.seven_tv,
        CreditProvider::Bttv => &set.bttv,
        CreditProvider::Ffz => &set.ffz,
    };
    list.iter().find(|e| e.id == id).map(|e| held_from(provider, e, seventv_global))
}

pub fn held_from(provider: CreditProvider, e: &Emote, seventv_global: bool) -> HeldEmote {
    let global = match provider {
        CreditProvider::SevenTv => seventv_global,
        CreditProvider::Twitch => matches!(e.emote_type.as_deref(), Some("globals" | "smilies")),
        _ => e.emote_type.as_deref() == Some(crate::services::emote_match::GLOBAL_EMOTE_TYPE),
    };
    let personal = e.emote_type.as_deref() == Some(crate::services::emote_match::PERSONAL_EMOTE_TYPE);
    let kind_label = if personal {
        Some("Personal emote")
    } else if provider == CreditProvider::Twitch {
        twitch_kind_label(e.emote_type.as_deref().unwrap_or_default())
    } else if global {
        Some("Global emote")
    } else {
        Some("Channel emote")
    };
    HeldEmote {
        owner_name: e.owner_name.clone(),
        original_name: e.original_name.clone(),
        kind_label,
        owner_ref: e.owner_ref.clone(),
        owner_twitch_id: if provider == CreditProvider::Bttv { e.owner_id.clone() } else { None },
    }
}

/// Helix `emote_type` and GQL `type` name the same kinds in two spellings.
fn twitch_kind_label(kind: &str) -> Option<&'static str> {
    Some(match kind.to_ascii_lowercase().as_str() {
        "subscriptions" => "Sub emote",
        "follower" => "Follower emote",
        "bitstier" | "bits_badge_tiers" => "Bits emote",
        "channelpoints" | "channel_points" => "Channel Points emote",
        "hypetrain" | "hype_train" => "Hype Train emote",
        "prime" => "Prime emote",
        "turbo" => "Turbo emote",
        "globals" | "smilies" => "Global emote",
        "limitedtime" | "limited_time" | "rewards" | "reward" => "Reward emote",
        _ => return None,
    })
}

fn non_empty(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

async fn get_json(url: &str) -> Option<serde_json::Value> {
    let resp = crate::services::http::client()
        .get(url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

async fn fetch_seventv(id: &str) -> Option<EmoteCredit> {
    let json = get_json(&format!("https://7tv.io/v3/emotes/{id}")).await?;
    Some(parse_seventv(&json))
}

pub(crate) fn parse_seventv(json: &serde_json::Value) -> EmoteCredit {
    // A deleted uploader comes back as a null owner.
    let owner_name =
        non_empty(json.pointer("/owner/display_name")).or_else(|| non_empty(json.pointer("/owner/username")));
    let creators = owner_name
        .clone()
        .and_then(|name| {
            seventv_creator(non_empty(json.pointer("/owner/id")), name, non_empty(json.pointer("/owner/avatar_url")))
        })
        .into_iter()
        .collect();
    EmoteCredit {
        provider: "7tv",
        owner_name,
        artist_name: None,
        original_name: non_empty(json.get("name")),
        kind_label: None,
        creators,
        page_url: None,
    }
}

async fn fetch_ffz(id: &str) -> Option<EmoteCredit> {
    let json = get_json(&format!("https://api.frankerfacez.com/v1/emote/{id}")).await?;
    Some(parse_ffz(&json))
}

pub(crate) fn parse_ffz(json: &serde_json::Value) -> EmoteCredit {
    let e = json.get("emote").unwrap_or(json);
    let person = |p: &str| {
        let login = non_empty(e.pointer(&format!("{p}/name")));
        let shown = non_empty(e.pointer(&format!("{p}/display_name"))).or_else(|| login.clone());
        (login, shown)
    };
    let (owner_login, owner_name) = person("/owner");
    let (artist_login, artist_name) = person("/artist");
    let mut creators = Vec::new();
    if let Some(name) = artist_name.clone() {
        creators.extend(ffz_creator("artist", artist_login.clone(), name));
    }
    if let Some(name) = owner_name.clone() {
        // The artist often uploaded it too: one chip is enough.
        if owner_login != artist_login || artist_login.is_none() {
            creators.extend(ffz_creator("uploader", owner_login, name));
        }
    }
    EmoteCredit {
        provider: "ffz",
        owner_name,
        artist_name,
        original_name: non_empty(e.get("name")),
        kind_label: None,
        creators,
        page_url: None,
    }
}

/// An FFZ user's picture: `/v1/user/{name}` names their Twitch id, and FFZ's
/// avatar URL for it redirects to the Twitch image.
async fn ffz_avatar(name: &str) -> Option<String> {
    if !is_ffz_name(name) {
        return None;
    }
    let name = name.to_lowercase();
    memo(avatar_cache(), format!("ffz:{name}"), || async move {
        let json = get_json(&format!("https://api.frankerfacez.com/v1/user/{name}")).await?;
        let tid = json
            .pointer("/user/twitch_id")
            .and_then(|v| v.as_u64().map(|n| n.to_string()).or_else(|| v.as_str().map(str::to_string)))
            .filter(|t| is_twitch_id(t))?;
        Some(format!("https://cdn.frankerfacez.com/avatar/twitch/{tid}"))
    })
    .await
}

fn gql_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("Client-ID", HeaderValue::from_static(WEB_CLIENT_ID));
    headers.insert(ACCEPT, HeaderValue::from_static("*/*"));
    if let Ok(v) = HeaderValue::from_str(&crate::services::twitch_service::gql_device_id()) {
        headers.insert("X-Device-Id", v);
    }
    headers
}

async fn gql(operation: &str, query: &str, variables: serde_json::Value) -> Option<serde_json::Value> {
    let body = serde_json::json!({ "operationName": operation, "query": query, "variables": variables });
    let resp = crate::services::http::client()
        .post("https://gql.twitch.tv/gql")
        .headers(gql_headers())
        .timeout(REQUEST_TIMEOUT)
        .json(&body)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json().await.ok()
}

const TWITCH_EMOTE_QUERY: &str = "query StreamNookEmoteCredit($id: ID!) { emote(id: $id) { id type \
     artist { id login displayName profileImageURL(width: 70) } \
     owner { id login displayName profileImageURL(width: 70) } } }";

async fn fetch_twitch(id: &str) -> Option<EmoteCredit> {
    // The id travels as a variable, never spliced into the query.
    let json = gql("StreamNookEmoteCredit", TWITCH_EMOTE_QUERY, serde_json::json!({ "id": id })).await?;
    parse_twitch(&json)
}

const TWITCH_AVATAR_QUERY: &str =
    "query StreamNookUserAvatar($id: ID!) { user(id: $id, lookupType: ALL) { profileImageURL(width: 70) } }";

/// A Twitch account's picture by id (a BetterTTV uploader's).
async fn twitch_avatar(twitch_id: &str) -> Option<String> {
    if !is_twitch_id(twitch_id) {
        return None;
    }
    let tid = twitch_id.to_string();
    memo(avatar_cache(), format!("tw:{tid}"), || async move {
        let json = gql("StreamNookUserAvatar", TWITCH_AVATAR_QUERY, serde_json::json!({ "id": tid })).await?;
        avatar(non_empty(json.pointer("/data/user/profileImageURL")))
    })
    .await
}

pub(crate) fn parse_twitch(json: &serde_json::Value) -> Option<EmoteCredit> {
    let e = json.pointer("/data/emote").filter(|v| !v.is_null())?;
    let name = |p: &str| non_empty(e.pointer(&format!("{p}/displayName"))).or_else(|| non_empty(e.pointer(&format!("{p}/login"))));
    let mut creators = Vec::new();
    if let Some(artist) = e.get("artist").filter(|v| !v.is_null()) {
        creators.extend(twitch_creator("artist", artist));
    }
    if let Some(owner) = e.get("owner").filter(|v| !v.is_null()) {
        creators.extend(twitch_creator("channel", owner));
    }
    Some(EmoteCredit {
        provider: "twitch",
        owner_name: name("/owner"),
        artist_name: name("/artist"),
        original_name: None,
        kind_label: e.get("type").and_then(|v| v.as_str()).and_then(twitch_kind_label),
        creators,
        page_url: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ids_are_checked_per_provider() {
        assert!(CreditProvider::SevenTv.valid_id("01G3WEGZN0000ET2J0MQP5YJ0G"));
        assert!(CreditProvider::SevenTv.valid_id("60ae958e229664e8667aea38"));
        assert!(!CreditProvider::SevenTv.valid_id("../users/1"));
        assert!(CreditProvider::Bttv.valid_id("5f3eb452b2efd65d77e867ed"));
        assert!(!CreditProvider::Bttv.valid_id("5f3eb452b2efd65d77e867eZ"));
        assert!(CreditProvider::Ffz.valid_id("246878"));
        assert!(!CreditProvider::Ffz.valid_id("246878?x=1"));
        assert!(CreditProvider::Twitch.valid_id("25"));
        assert!(CreditProvider::Twitch.valid_id("emotesv2_07fb2f0da3534f058a9f02b31b4eaf8b"));
        assert!(!CreditProvider::Twitch.valid_id("emotesv2_"));
        assert!(!CreditProvider::Twitch.valid_id("1 OR 1"));
    }

    #[test]
    fn seventv_owner_and_null_owner() {
        let c = parse_seventv(&json!({ "name": "xqcTours", "owner": { "username": "a", "display_name": "Alpha" } }));
        assert_eq!(c.owner_name.as_deref(), Some("Alpha"));
        assert_eq!(c.original_name.as_deref(), Some("xqcTours"));
        let deleted = parse_seventv(&json!({ "name": "x", "owner": null }));
        assert_eq!(deleted.owner_name, None);
    }

    #[test]
    fn ffz_owner_and_artist() {
        let c = parse_ffz(&json!({ "emote": {
            "name": "WideHard",
            "owner": { "_id": 1, "name": "own", "display_name": "Own" },
            "artist": { "name": "drawer", "display_name": "" }
        }}));
        assert_eq!(c.owner_name.as_deref(), Some("Own"));
        assert_eq!(c.artist_name.as_deref(), Some("drawer"));
    }

    #[test]
    fn twitch_owner_artist_and_kind() {
        let c = parse_twitch(&json!({ "data": { "emote": {
            "id": "emotesv2_x", "type": "SUBSCRIPTIONS",
            "artist": { "login": "seabii", "displayName": "Seabii" },
            "owner": { "login": "pokimane", "displayName": "pokimane" }
        }}}))
        .unwrap();
        assert_eq!(c.owner_name.as_deref(), Some("pokimane"));
        assert_eq!(c.artist_name.as_deref(), Some("Seabii"));
        assert_eq!(c.kind_label, Some("Sub emote"));
        let kappa = parse_twitch(&json!({ "data": { "emote": { "type": "GLOBALS", "owner": null, "artist": null } } })).unwrap();
        assert_eq!(kappa.kind_label, Some("Global emote"));
        assert!(kappa.owner_name.is_none());
        assert!(parse_twitch(&json!({ "data": { "emote": null } })).is_none());
    }

    #[test]
    fn held_labels() {
        let mut e = Emote {
            id: "1".into(),
            name: "a".into(),
            url: String::new(),
            provider: crate::services::emote_service::EmoteProvider::BTTV,
            is_zero_width: None,
            local_url: None,
            emote_type: Some("global".into()),
            owner_id: None,
            owner_name: None,
            original_name: None,
            owner_ref: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
        };
        assert_eq!(held_from(CreditProvider::Bttv, &e, false).kind_label, Some("Global emote"));
        e.emote_type = None;
        e.owner_name = Some("forsen".into());
        let h = held_from(CreditProvider::Bttv, &e, false);
        assert_eq!(h.kind_label, Some("Channel emote"));
        assert_eq!(h.owner_name.as_deref(), Some("forsen"));
    }

    #[test]
    fn seventv_creator_links_to_their_profile_with_a_fixed_avatar() {
        let c = parse_seventv(&json!({ "name": "GAMBA", "owner": {
            "id": "01FFFKKA2R0007P57XYW0BHEQV", "display_name": "victorleporc_",
            "avatar_url": "//cdn.7tv.app/user/x/profile-picture/y/4x.gif"
        }}));
        assert_eq!(c.creators.len(), 1);
        let p = &c.creators[0];
        assert_eq!(p.role, "uploader");
        assert_eq!(p.avatar_url.as_deref(), Some("https://cdn.7tv.app/user/x/profile-picture/y/4x.gif"));
        assert_eq!(p.link, CreatorLink::Web { url: "https://7tv.app/users/01FFFKKA2R0007P57XYW0BHEQV".into(), site: "7TV" });
        // An id that is not 7TV's shape gets no chip, never a crafted link.
        let bad = parse_seventv(&json!({ "owner": { "id": "../../evil", "display_name": "x" } }));
        assert!(bad.creators.is_empty());
        assert_eq!(bad.owner_name.as_deref(), Some("x"));
    }

    #[test]
    fn ffz_artist_and_uploader_chips_merge_when_the_same_person() {
        let both = parse_ffz(&json!({ "emote": {
            "owner": { "name": "own", "display_name": "Own" },
            "artist": { "name": "drawer", "display_name": "Drawer" }
        }}));
        let roles: Vec<_> = both.creators.iter().map(|c| c.role).collect();
        assert_eq!(roles, ["artist", "uploader"]);
        assert_eq!(both.creators[1].link, CreatorLink::Web {
            url: "https://www.frankerfacez.com/own/submissions".into(),
            site: "FrankerFaceZ",
        });
        let same = parse_ffz(&json!({ "emote": {
            "owner": { "name": "drawer", "display_name": "Drawer" },
            "artist": { "name": "drawer", "display_name": "Drawer" }
        }}));
        assert_eq!(same.creators.len(), 1);
        assert_eq!(same.creators[0].role, "artist");
    }

    #[test]
    fn twitch_creators_open_the_in_app_profile() {
        let c = parse_twitch(&json!({ "data": { "emote": {
            "type": "SUBSCRIPTIONS",
            "artist": { "id": "73755633", "login": "seabii", "displayName": "Seabii", "profileImageURL": "https://static-cdn.jtvnw.net/a.png" },
            "owner": { "id": "44445592", "login": "pokimane", "displayName": "pokimane", "profileImageURL": "http://insecure/b.png" }
        }}}))
        .unwrap();
        assert_eq!(c.creators.len(), 2);
        assert_eq!(c.creators[0].role, "artist");
        assert_eq!(c.creators[0].link, CreatorLink::Twitch { login: "seabii".into(), id: Some("73755633".into()) });
        assert_eq!(c.creators[0].avatar_url.as_deref(), Some("https://static-cdn.jtvnw.net/a.png"));
        assert_eq!(c.creators[1].role, "channel");
        // Only https pictures are drawn.
        assert_eq!(c.creators[1].avatar_url, None);
    }

    #[test]
    fn bttv_creator_comes_from_the_set() {
        let h = HeldEmote {
            owner_name: Some("NightDev".into()),
            original_name: None,
            kind_label: Some("Channel emote"),
            owner_ref: Some("5561169bd6b9d206222a8c19".into()),
            owner_twitch_id: Some("29045896".into()),
        };
        let c = credit_from_held(CreditProvider::Bttv, &h);
        assert_eq!(c.creators[0].link, CreatorLink::Web {
            url: "https://betterttv.com/users/5561169bd6b9d206222a8c19".into(),
            site: "BetterTTV",
        });
        assert_eq!(page_url(CreditProvider::Bttv, "54fa92ee01e468494b85b553").as_deref(), Some("https://betterttv.com/emotes/54fa92ee01e468494b85b553"));
        assert_eq!(page_url(CreditProvider::Twitch, "25"), None);
    }

    #[test]
    fn avatars_must_be_plain_https() {
        assert_eq!(avatar(Some("//cdn.7tv.app/a.avif".into())).as_deref(), Some("https://cdn.7tv.app/a.avif"));
        assert_eq!(avatar(Some("javascript:alert(1)".into())), None);
        assert_eq!(avatar(Some("https://x/a b.png".into())), None);
        assert_eq!(avatar(None), None);
    }
}
