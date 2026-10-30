//! The viewer's own Discord look, for the Discord Status editor's preview:
//! banner, profile color, decoration, nameplate and server tag from
//! streamnook.app (StreamNook's bot reads the public profile), and the
//! avatar's dominant color straight from Discord's CDN.
//!
//! Asked only while the editor is open, cached per Discord id (and per avatar
//! for the color), and quiet when it cannot be had: the preview then draws what
//! Discord's handshake gave. No lock here is held across an await or while the
//! presence lock is taken.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;

use crate::rt::AppHandle;
use crate::services::discord_service::{DiscordService, DiscordUser, SendError};

const SITE_URL: &str = "https://streamnook.app/api/v1/discord-profile";
/// A profile the site found is shown this long before it is asked again.
const PROFILE_KEEP: Duration = Duration::from_secs(10 * 60);
/// No profile, or a failed ask: asked again after this.
const NONE_KEEP: Duration = Duration::from_secs(2 * 60);
/// A `retry-after` is honored up to this.
const RETRY_CAP: Duration = Duration::from_secs(10 * 60);
/// A fetch still pending after this counts as failed (site 10 s per route,
/// direct then relay; the avatar 8 s).
const PENDING_LIMIT: Duration = Duration::from_secs(45);
const SITE_TIMEOUT: Duration = Duration::from_secs(10);
const AVATAR_TIMEOUT: Duration = Duration::from_secs(8);
const SITE_BODY_CAP: usize = 64 * 1024;
const AVATAR_BODY_CAP: usize = 256 * 1024;
const URL_CAP: usize = 512;
/// Discord accounts kept (account switches); the oldest goes first.
const KEPT: usize = 8;
const PALETTES: [&str; 11] = [
    "crimson",
    "berry",
    "sky",
    "teal",
    "forest",
    "bubble_gum",
    "violet",
    "cobalt",
    "clover",
    "lemon",
    "white",
];

/// A picture that can move, and its first frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pair {
    pub animated: String,
    pub still: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Nameplate {
    pub image: String,
    pub video: Option<String>,
    pub palette: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ServerTag {
    pub tag: String,
    pub badge: Option<String>,
}

/// The look the preview draws. Rust decides every piece; the preview only
/// picks the animated or the still member of a pair.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct DiscordLook {
    pub banner: Option<Pair>,
    /// The banner area's color when there is no banner picture: the profile
    /// color, else the avatar's dominant color. None: the preview's default.
    pub strip: Option<String>,
    pub decoration: Option<Pair>,
    pub nameplate: Option<Nameplate>,
    pub server_tag: Option<ServerTag>,
}

/// What the site answered for a profile, checked field by field.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiteLook {
    pub banner: Option<Pair>,
    pub color: Option<String>,
    pub decoration: Option<Pair>,
    pub nameplate: Option<Nameplate>,
    pub server_tag: Option<ServerTag>,
}

// ---- checking what the site sent ------------------------------------------

/// A Discord CDN picture: https, `cdn.discordapp.com`, no port or user,
/// at most 512 bytes. Anything else is dropped.
fn cdn_url(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    if s.len() > URL_CAP {
        return None;
    }
    let url = url::Url::parse(s).ok()?;
    let clean = url.scheme() == "https"
        && url.host_str() == Some("cdn.discordapp.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    clean.then(|| url.to_string())
}

/// A picture's first frame: the same URL without the query pair that makes it
/// move (`passthrough` for a decoration, `animated` for a banner).
fn without_param(animated: &str, param: &str) -> Option<String> {
    let mut url = url::Url::parse(animated).ok()?;
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| k != param)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    url.query_pairs_mut().clear().extend_pairs(pairs);
    Some(url.to_string())
}

fn hex_color(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    let ok = s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit());
    ok.then(|| s.to_ascii_lowercase())
}

/// The site's `profile` object, every field checked on its own.
pub(crate) fn check_site(p: &Value) -> SiteLook {
    let banner = cdn_url(p.get("banner")).and_then(|animated| {
        let still =
            cdn_url(p.get("bannerStill")).or_else(|| without_param(&animated, "animated"))?;
        Some(Pair { animated, still })
    });
    let decoration = cdn_url(p.get("avatarDecoration")).and_then(|animated| {
        let still = without_param(&animated, "passthrough")?;
        Some(Pair { animated, still })
    });
    let nameplate = p.get("nameplate").and_then(|n| {
        Some(Nameplate {
            image: cdn_url(n.get("image"))?,
            video: cdn_url(n.get("video")),
            palette: n
                .get("palette")
                .and_then(Value::as_str)
                .filter(|s| PALETTES.contains(s))
                .map(str::to_string),
        })
    });
    let server_tag = p.get("serverTag").and_then(|t| {
        let tag = t.get("tag")?.as_str()?.trim();
        let count = tag.graphemes(true).count();
        if !(1..=4).contains(&count) {
            return None;
        }
        Some(ServerTag {
            tag: tag.to_string(),
            badge: cdn_url(t.get("badge")),
        })
    });
    SiteLook {
        banner,
        color: hex_color(p.get("bannerColor")),
        decoration,
        nameplate,
        server_tag,
    }
}

/// How one fetch ended.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Fetched<T> {
    /// Final for its keep time.
    Found(T),
    /// There is nothing (no such profile, or a picture that will not decode).
    Nothing,
    /// Could not ask now; ask again after this.
    Later(Duration),
}

/// The site's answer: 200 JSON `{ "profile": {...} | null }`. Anything else
/// (the site's page for an unknown path is a 200 HTML) is a failed ask.
pub(crate) fn site_answer(
    status: u16,
    content_type: Option<&str>,
    retry_after: Option<u64>,
    body: &[u8],
) -> Fetched<SiteLook> {
    match status {
        200 => {}
        429 | 503 => {
            let wait = Duration::from_secs(retry_after.unwrap_or(0)).clamp(NONE_KEEP, RETRY_CAP);
            return Fetched::Later(wait);
        }
        _ => return Fetched::Later(NONE_KEEP),
    }
    if !content_type.is_some_and(|c| c.starts_with("application/json")) {
        return Fetched::Later(NONE_KEEP);
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return Fetched::Later(NONE_KEEP);
    };
    match value.get("profile") {
        Some(Value::Null) => Fetched::Nothing,
        Some(p @ Value::Object(_)) => Fetched::Found(check_site(p)),
        _ => Fetched::Later(NONE_KEEP),
    }
}

// ---- the caches -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum State {
    Pending {
        gen: u64,
        since: Instant,
    },
    /// `until` None: final.
    Ready {
        until: Option<Instant>,
    },
}

#[derive(Debug, Clone, PartialEq)]
struct Slot<T> {
    /// What is shown, kept while a newer ask is pending.
    value: Option<T>,
    state: State,
}

/// One cache: per key, the value shown and whether an ask is due.
#[derive(Debug)]
pub(crate) struct Cache<T> {
    slots: HashMap<String, Slot<T>>,
    order: VecDeque<String>,
    next_gen: u64,
}

impl<T: Clone> Cache<T> {
    pub(crate) fn new() -> Self {
        Self {
            slots: HashMap::new(),
            order: VecDeque::new(),
            next_gen: 1,
        }
    }

    /// What is shown for `key` now, and the generation to fetch with when an
    /// ask is due (nothing yet, expired, or a pending ask past its limit).
    pub(crate) fn ask(&mut self, key: &str, now: Instant) -> (Option<T>, Option<u64>) {
        let due = match self.slots.get(key).map(|s| &s.state) {
            None => true,
            Some(State::Pending { since, .. }) => {
                now.saturating_duration_since(*since) >= PENDING_LIMIT
            }
            Some(State::Ready { until }) => until.is_some_and(|u| now >= u),
        };
        let gen = due.then(|| {
            let gen = self.next_gen;
            self.next_gen += 1;
            gen
        });
        if !self.slots.contains_key(key) {
            self.order.push_back(key.to_string());
            while self.order.len() > KEPT {
                if let Some(old) = self.order.pop_front() {
                    self.slots.remove(&old);
                }
            }
        }
        let slot = self.slots.entry(key.to_string()).or_insert(Slot {
            value: None,
            state: State::Ready { until: Some(now) },
        });
        if let Some(gen) = gen {
            slot.state = State::Pending { gen, since: now };
        }
        (slot.value.clone(), gen)
    }

    /// A fetch landed. Written only while `key` still waits on this `gen`, so
    /// a late answer never overwrites a newer one. `keep` None: final.
    pub(crate) fn land(
        &mut self,
        key: &str,
        gen: u64,
        fetched: Fetched<T>,
        keep: Option<Duration>,
        now: Instant,
    ) -> bool {
        let Some(slot) = self.slots.get_mut(key) else {
            return false;
        };
        if !matches!(slot.state, State::Pending { gen: waiting, .. } if waiting == gen) {
            return false;
        }
        let until = |d: Option<Duration>| d.map(|d| now + d);
        match fetched {
            Fetched::Found(v) => {
                slot.value = Some(v);
                slot.state = State::Ready { until: until(keep) };
            }
            Fetched::Nothing => {
                slot.value = None;
                slot.state = State::Ready { until: until(keep) };
            }
            // A failed ask keeps what was shown.
            Fetched::Later(wait) => {
                slot.state = State::Ready {
                    until: Some(now + wait),
                }
            }
        }
        true
    }
}

static SITES: Lazy<Mutex<Cache<SiteLook>>> = Lazy::new(|| Mutex::new(Cache::new()));
static COLORS: Lazy<Mutex<Cache<String>>> = Lazy::new(|| Mutex::new(Cache::new()));
static ACCOUNT: Lazy<Mutex<Cache<DiscordUser>>> = Lazy::new(|| Mutex::new(Cache::new()));
const ACCOUNT_KEY: &str = "account";
/// Discord not running (or not answering yet): the editor tries again after this.
const NO_DISCORD_RETRY: Duration = Duration::from_secs(15);

/// How one read of the account ended.
pub(crate) fn account_outcome(
    read: Result<Option<DiscordUser>, SendError>,
) -> Fetched<DiscordUser> {
    match read {
        Ok(Some(user)) => Fetched::Found(user),
        // Discord answered without naming the account: as good as closed.
        Ok(None) | Err(SendError::NoPipe) | Err(SendError::Busy) => {
            Fetched::Later(NO_DISCORD_RETRY)
        }
        Err(SendError::Unanswered) | Err(SendError::BackingOff) => Fetched::Later(NONE_KEEP),
    }
}

/// The Discord account the preview draws. With the status on, the pump's
/// (it connects anyway). With it off, the pump never connects, so the open
/// editor reads it itself: one handshake, no activity, the pipe closed again,
/// kept 10 minutes.
pub fn account(
    app: &AppHandle,
    pump_user: Option<DiscordUser>,
    enabled: bool,
) -> Option<DiscordUser> {
    if enabled || pump_user.is_some() {
        return pump_user;
    }
    let (known, gen) = ACCOUNT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .ask(ACCOUNT_KEY, Instant::now());
    if let Some(gen) = gen {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let fetched = account_outcome(DiscordService::peek_user().await);
            let later = match &fetched {
                Fetched::Later(d) => Some(*d),
                _ => None,
            };
            let landed = ACCOUNT.lock().unwrap_or_else(|e| e.into_inner()).land(
                ACCOUNT_KEY,
                gen,
                fetched,
                Some(PROFILE_KEEP),
                Instant::now(),
            );
            landed_then_retry(app, landed, later).await;
        });
    }
    known
}

/// The asks one preview makes, and the look it draws now.
#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    pub look: DiscordLook,
    /// (Discord id, generation) when the site is to be asked.
    pub site: Option<(String, u64)>,
    /// (still avatar URL, generation) when the avatar's color is to be read.
    pub color: Option<(String, u64)>,
}

/// The look from what is known. The decoration is the site's whenever the
/// site answered (fresher than the handshake, which is read only at connect);
/// the strip is the profile color, else the avatar's dominant color.
pub(crate) fn combine(
    user: &DiscordUser,
    site: Option<&SiteLook>,
    avatar_color: Option<&str>,
) -> DiscordLook {
    let ready_decoration = match (&user.decoration, &user.decoration_still) {
        (Some(animated), Some(still)) => Some(Pair {
            animated: animated.clone(),
            still: still.clone(),
        }),
        _ => None,
    };
    DiscordLook {
        banner: site.and_then(|s| s.banner.clone()),
        strip: site
            .and_then(|s| s.color.clone())
            .or_else(|| avatar_color.map(str::to_string)),
        decoration: match site {
            Some(s) => s.decoration.clone(),
            None => ready_decoration,
        },
        nameplate: site.and_then(|s| s.nameplate.clone()),
        server_tag: site.and_then(|s| s.server_tag.clone()),
    }
}

/// What one preview shows and asks for. Only the open editor asks, so the id
/// goes to streamnook.app only while it is open, status on or off.
pub(crate) fn plan(
    user: &DiscordUser,
    sites: &mut Cache<SiteLook>,
    colors: &mut Cache<String>,
    now: Instant,
) -> Plan {
    let (site, site_gen) = if !user.id.is_empty() {
        sites.ask(&user.id, now)
    } else {
        (None, None)
    };
    let needs_color = site.as_ref().is_none_or(|s| s.color.is_none());
    let (color, color_gen) = if needs_color && !user.avatar_still.is_empty() {
        colors.ask(&user.avatar_still, now)
    } else {
        (None, None)
    };
    Plan {
        look: combine(user, site.as_ref(), color.as_deref()),
        site: site_gen.map(|g| (user.id.clone(), g)),
        color: color_gen.map(|g| (user.avatar_still.clone(), g)),
    }
}

/// The look for the editor's preview now; asks that are due run in the
/// background and emit the changed event when they land.
pub fn resolve(app: &AppHandle, user: Option<&DiscordUser>) -> Option<DiscordLook> {
    let user = user?;
    let now = Instant::now();
    let plan = {
        let mut sites = SITES.lock().unwrap_or_else(|e| e.into_inner());
        let mut colors = COLORS.lock().unwrap_or_else(|e| e.into_inner());
        plan(user, &mut sites, &mut colors, now)
    };
    if let Some((id, gen)) = plan.site {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let fetched = fetch_site(&id).await;
            let keep = match &fetched {
                Fetched::Found(_) => Some(PROFILE_KEEP),
                _ => Some(NONE_KEEP),
            };
            let later = match &fetched {
                Fetched::Later(d) => Some(*d),
                _ => None,
            };
            let landed = SITES.lock().unwrap_or_else(|e| e.into_inner()).land(
                &id,
                gen,
                fetched,
                keep,
                Instant::now(),
            );
            landed_then_retry(app, landed, later).await;
        });
    }
    if let Some((url, gen)) = plan.color {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let fetched = fetch_color(&url).await;
            // A color, or a picture that will not decode, is final.
            let later = match &fetched {
                Fetched::Later(d) => Some(*d),
                _ => None,
            };
            let landed = COLORS.lock().unwrap_or_else(|e| e.into_inner()).land(
                &url,
                gen,
                fetched,
                None,
                Instant::now(),
            );
            landed_then_retry(app, landed, later).await;
        });
    }
    Some(plan.look)
}

/// The editor redraws when an answer lands, and an open editor asks again by
/// itself once a failed ask's wait is over. Only an open editor listens, and
/// the event itself fetches nothing.
async fn landed_then_retry(app: AppHandle, landed: bool, later: Option<Duration>) {
    use tauri::Emitter;
    if !landed {
        return;
    }
    let _ = app.emit(crate::services::discord_presence::CHANGED_EVENT, ());
    if let Some(wait) = later {
        tokio::time::sleep(wait).await;
        let _ = app.emit(crate::services::discord_presence::CHANGED_EVENT, ());
    }
}

// ---- fetching ----------------------------------------------------------------

fn retry_after(response: &reqwest::Response) -> Option<u64> {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// The whole body: `Ok(None)` past `cap`, `Err` when the download broke off
/// (a cut-off body is a failed download, never bad content).
async fn capped_body(mut response: reqwest::Response, cap: usize) -> Result<Option<Vec<u8>>, ()> {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > cap {
                    return Ok(None);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => return Ok(Some(body)),
            Err(_) => return Err(()),
        }
    }
}

fn app_agent() -> String {
    format!(
        "StreamNook/{}",
        crate::services::client_identity::build_version()
    )
}

/// The profile look from streamnook.app. The id travels in the body, never the URL.
async fn fetch_site(id: &str) -> Fetched<SiteLook> {
    let builder = crate::services::http::client()
        .post(SITE_URL)
        .header(reqwest::header::USER_AGENT, app_agent())
        .json(&serde_json::json!({ "id": id }))
        .timeout(SITE_TIMEOUT);
    let response = match crate::services::sn_transport::send(builder).await {
        Ok(r) => r,
        Err(e) => {
            log::debug!(
                "[DiscordLook] profile lookup failed: {}",
                crate::services::sn_transport::reason(&e)
            );
            return Fetched::Later(NONE_KEEP);
        }
    };
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase);
    let wait = retry_after(&response);
    let Ok(Some(body)) = capped_body(response, SITE_BODY_CAP).await else {
        return Fetched::Later(NONE_KEEP);
    };
    let fetched = site_answer(status, content_type.as_deref(), wait, &body);
    if let Fetched::Later(_) = fetched {
        log::debug!("[DiscordLook] profile lookup answered {status}");
    }
    fetched
}

/// The avatar's dominant color, read from Discord's CDN (no StreamNook call).
async fn fetch_color(url: &str) -> Fetched<String> {
    let request = crate::services::http::client()
        .get(url)
        .timeout(AVATAR_TIMEOUT);
    let Ok(response) = request.send().await else {
        return Fetched::Later(NONE_KEEP);
    };
    if !response.status().is_success() {
        return Fetched::Later(NONE_KEEP);
    }
    let body = capped_body(response, AVATAR_BODY_CAP).await;
    tokio::task::spawn_blocking(move || color_of(body))
        .await
        .unwrap_or(Fetched::Later(NONE_KEEP))
}

/// How a downloaded avatar ends: a color (final), nothing (final: too large
/// for an avatar, or not a picture), or a broken download (asked again later).
fn color_of(body: Result<Option<Vec<u8>>, ()>) -> Fetched<String> {
    match body {
        Ok(Some(bytes)) => match crate::services::song_id::cover_tint::dominant_rgb(&bytes) {
            Some([r, g, b]) => Fetched::Found(format!("#{r:02x}{g:02x}{b:02x}")),
            None => Fetched::Nothing,
        },
        // Far larger than an avatar at 128 px: not one to read.
        Ok(None) => Fetched::Nothing,
        Err(()) => Fetched::Later(NONE_KEEP),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CDN: &str = "https://cdn.discordapp.com";

    fn user(decoration: bool) -> DiscordUser {
        DiscordUser {
            id: "80351110224678912".into(),
            name: "Nelly".into(),
            username: "nelly".into(),
            avatar_url: format!(
                "{CDN}/avatars/80351110224678912/a_{}.webp?size=128&animated=true",
                "0".repeat(32)
            ),
            avatar_still: format!(
                "{CDN}/avatars/80351110224678912/a_{}.webp?size=128",
                "0".repeat(32)
            ),
            decoration: decoration.then(|| {
                format!(
                    "{CDN}/avatar-decoration-presets/{}.png?size=160&passthrough=true",
                    "1".repeat(32)
                )
            }),
            decoration_still: decoration.then(|| {
                format!(
                    "{CDN}/avatar-decoration-presets/{}.png?size=160",
                    "1".repeat(32)
                )
            }),
        }
    }

    fn profile() -> Value {
        json!({
            "banner": format!("{CDN}/banners/1/a_{}.webp?size=600&animated=true", "2".repeat(32)),
            "bannerStill": format!("{CDN}/banners/1/a_{}.webp?size=600", "2".repeat(32)),
            "bannerColor": "#1A2B3C",
            "avatarDecoration": format!("{CDN}/avatar-decoration-presets/{}.png?size=160&passthrough=true", "3".repeat(32)),
            "nameplate": { "image": format!("{CDN}/assets/collectibles/nameplates/nameplates/twilight/static.png"), "video": format!("{CDN}/assets/collectibles/nameplates/nameplates/twilight/asset.webm"), "palette": "violet" },
            "serverTag": { "tag": "SPCY", "badge": format!("{CDN}/guild-tag-badges/1/{}.png?size=32", "4".repeat(32)) }
        })
    }

    #[test]
    fn site_look_is_checked_field_by_field() {
        let good = check_site(&profile());
        assert!(
            good.banner.as_ref().is_some_and(
                |b| b.animated.contains("animated=true") && !b.still.contains("animated")
            )
        );
        assert_eq!(good.color.as_deref(), Some("#1a2b3c"));
        let deco = good.decoration.clone().expect("decoration");
        assert!(deco.animated.contains("passthrough=true") && !deco.still.contains("passthrough"));
        assert_eq!(
            good.nameplate.as_ref().and_then(|n| n.palette.as_deref()),
            Some("violet")
        );
        assert_eq!(
            good.server_tag.as_ref().map(|t| t.tag.as_str()),
            Some("SPCY")
        );

        // Each bad field goes alone; the rest stay.
        let mut p = profile();
        p["banner"] = json!("https://evil.example/banner.webp");
        p["bannerColor"] = json!("red");
        p["avatarDecoration"] = json!(format!(
            "https://cdn.discordapp.com:8443/x/{}.png",
            "3".repeat(32)
        ));
        p["nameplate"]["palette"] = json!("rainbow");
        p["serverTag"]["tag"] = json!("FIVES");
        let bad = check_site(&p);
        assert_eq!(bad.banner, None, "foreign host");
        assert_eq!(bad.color, None, "not a hex color");
        assert_eq!(bad.decoration, None, "a port");
        assert_eq!(
            bad.nameplate.as_ref().map(|n| n.palette.clone()),
            Some(None),
            "unknown palette only"
        );
        assert_eq!(bad.server_tag, None, "five graphemes");

        // A bad still is made from the moving one, never the moving one itself.
        let mut p = profile();
        p["bannerStill"] = json!("https://evil.example/still.webp");
        let derived = check_site(&p).banner.expect("banner");
        assert!(!derived.still.contains("animated") && derived.still.contains(&"2".repeat(32)));

        let mut p = profile();
        p["banner"] = json!(format!(
            "https://user@cdn.discordapp.com/banners/1/{}.png",
            "2".repeat(32)
        ));
        p["nameplate"]["image"] = json!(format!("{CDN}/{}", "a".repeat(600)));
        p["serverTag"]["tag"] = json!("👨‍👩‍👧‍👦AB");
        let bad = check_site(&p);
        assert_eq!(bad.banner, None, "userinfo");
        assert_eq!(bad.nameplate, None, "over 512 bytes");
        assert_eq!(
            bad.server_tag.map(|t| t.tag),
            Some("👨‍👩‍👧‍👦AB".to_string()),
            "a family is one grapheme"
        );

        // The answer as a whole.
        let body = serde_json::to_vec(&json!({ "profile": profile() })).unwrap();
        assert!(matches!(
            site_answer(200, Some("application/json; charset=utf-8"), None, &body),
            Fetched::Found(_)
        ));
        assert_eq!(
            site_answer(200, Some("application/json"), None, br#"{"profile":null}"#),
            Fetched::Nothing
        );
        assert_eq!(
            site_answer(200, Some("text/html"), None, b"<!doctype html>"),
            Fetched::Later(NONE_KEEP),
            "the site's page for an unknown path"
        );
        assert_eq!(site_answer(404, None, None, b""), Fetched::Later(NONE_KEEP));
        assert_eq!(
            site_answer(503, Some("application/json"), Some(300), b"{}"),
            Fetched::Later(Duration::from_secs(300))
        );
        assert_eq!(
            site_answer(429, None, Some(5), b""),
            Fetched::Later(NONE_KEEP),
            "at least 2 min"
        );
        assert_eq!(
            site_answer(503, None, Some(86_400), b""),
            Fetched::Later(RETRY_CAP),
            "at most 10 min"
        );
    }

    #[test]
    fn a_broken_avatar_download_is_asked_again() {
        assert_eq!(
            color_of(Err(())),
            Fetched::Later(NONE_KEEP),
            "cut off: try later"
        );
        assert_eq!(color_of(Ok(None)), Fetched::Nothing, "too large: final");
        assert_eq!(
            color_of(Ok(Some(b"not a picture".to_vec()))),
            Fetched::Nothing,
            "not a picture: final"
        );
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([0x33, 0x66, 0x99, 255]),
        ))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
        assert!(
            matches!(color_of(Ok(Some(png))), Fetched::Found(c) if c.starts_with('#') && c.len() == 7)
        );
    }

    #[test]
    fn look_cache_steps() {
        let t0 = Instant::now();
        let mut c: Cache<String> = Cache::new();
        let (v, g1) = c.ask("a", t0);
        assert_eq!(v, None);
        let g1 = g1.expect("first ask fetches");
        assert_eq!(
            c.ask("a", t0 + Duration::from_secs(5)).1,
            None,
            "pending is not asked again"
        );
        // Pending past its limit is asked again; the first answer then lands on nothing.
        let g2 = c
            .ask("a", t0 + PENDING_LIMIT)
            .1
            .expect("stale pending re-asks");
        assert!(
            !c.land(
                "a",
                g1,
                Fetched::Found("old".into()),
                Some(PROFILE_KEEP),
                t0 + PENDING_LIMIT
            ),
            "a late answer is dropped"
        );
        assert!(c.land(
            "a",
            g2,
            Fetched::Found("new".into()),
            Some(PROFILE_KEEP),
            t0 + PENDING_LIMIT
        ));
        let t1 = t0 + PENDING_LIMIT;
        assert_eq!(
            c.ask("a", t1 + Duration::from_secs(60)),
            (Some("new".into()), None),
            "ready until expiry"
        );
        let (v, g3) = c.ask("a", t1 + PROFILE_KEEP);
        assert_eq!(
            v.as_deref(),
            Some("new"),
            "kept while the next ask is pending"
        );
        // A failed ask keeps the value and waits its retry.
        assert!(c.land(
            "a",
            g3.unwrap(),
            Fetched::Later(Duration::from_secs(300)),
            None,
            t1 + PROFILE_KEEP
        ));
        let t2 = t1 + PROFILE_KEEP;
        assert_eq!(
            c.ask("a", t2 + Duration::from_secs(299)),
            (Some("new".into()), None)
        );
        assert!(
            c.ask("a", t2 + Duration::from_secs(300)).1.is_some(),
            "retry-after honored"
        );
        // Final values are never asked again.
        let (_, g) = c.ask("b", t0);
        c.land("b", g.unwrap(), Fetched::Found("#123456".into()), None, t0);
        assert_eq!(c.ask("b", t0 + Duration::from_secs(86_400)).1, None);
        // At most KEPT keys; the oldest goes.
        let mut c: Cache<String> = Cache::new();
        for i in 0..=KEPT {
            c.ask(&format!("k{i}"), t0);
        }
        assert!(c.slots.len() == KEPT && !c.slots.contains_key("k0"));
    }

    #[test]
    fn look_resolves_in_order() {
        let t0 = Instant::now();
        let u = user(true);
        let (mut sites, mut colors) = (Cache::new(), Cache::new());
        // First look: the handshake's decoration, both asks due.
        let first = plan(&u, &mut sites, &mut colors, t0);
        assert_eq!(
            first.look.decoration.as_ref().map(|d| d.animated.clone()),
            u.decoration.clone()
        );
        assert_eq!(
            first.look.decoration.as_ref().map(|d| d.still.clone()),
            u.decoration_still.clone()
        );
        assert_eq!(first.look.strip, None);
        let (id, sg) = first.site.clone().expect("site asked");
        let (url, cg) = first.color.clone().expect("color read");
        assert_eq!(
            (id.as_str(), url.as_str()),
            (u.id.as_str(), u.avatar_still.as_str())
        );
        // The avatar color lands: the strip uses it.
        colors.land(&url, cg, Fetched::Found("#336699".into()), None, t0);
        assert_eq!(
            plan(&u, &mut sites, &mut colors, t0).look.strip.as_deref(),
            Some("#336699")
        );
        // The site answers: its color and decoration win, its banner, plate and tag show.
        sites.land(
            &id,
            sg,
            Fetched::Found(check_site(&profile())),
            Some(PROFILE_KEEP),
            t0,
        );
        let full = plan(&u, &mut sites, &mut colors, t0);
        assert_eq!(full.look.strip.as_deref(), Some("#1a2b3c"));
        assert!(full
            .look
            .decoration
            .as_ref()
            .is_some_and(|d| d.animated.contains(&"3".repeat(32))));
        assert!(
            full.look
                .decoration
                .as_ref()
                .is_some_and(|d| !d.still.contains("passthrough")),
            "the still never moves"
        );
        assert!(
            full.look.banner.is_some()
                && full.look.nameplate.is_some()
                && full.look.server_tag.is_some()
        );
        assert_eq!(full.color, None, "no color read while the profile has one");
        // A profile without a decoration means none, whatever the handshake had.
        let mut p = profile();
        p["avatarDecoration"] = Value::Null;
        assert_eq!(combine(&u, Some(&check_site(&p)), None).decoration, None);
        // An account whose id was not Discord's is never looked up on the site.
        let mut nameless = u.clone();
        nameless.id.clear();
        let (mut sites, mut colors) = (Cache::new(), Cache::new());
        let plain = plan(&nameless, &mut sites, &mut colors, t0);
        assert_eq!(plain.site, None);
        assert!(
            plain.color.is_some(),
            "the color comes from Discord's CDN alone"
        );
    }

    #[test]
    fn the_account_is_read_again_until_discord_answers() {
        assert_eq!(
            account_outcome(Ok(Some(user(false)))),
            Fetched::Found(user(false))
        );
        assert_eq!(
            account_outcome(Err(SendError::NoPipe)),
            Fetched::Later(NO_DISCORD_RETRY),
            "Discord closed"
        );
        assert_eq!(
            account_outcome(Ok(None)),
            Fetched::Later(NO_DISCORD_RETRY),
            "READY without an account"
        );
        assert_eq!(
            account_outcome(Err(SendError::Busy)),
            Fetched::Later(NO_DISCORD_RETRY)
        );
        assert_eq!(
            account_outcome(Err(SendError::Unanswered)),
            Fetched::Later(NONE_KEEP),
            "a hung pipe waits longer"
        );
        assert_eq!(
            account_outcome(Err(SendError::BackingOff)),
            Fetched::Later(NONE_KEEP)
        );
    }
}
