//! The first paint of a member's profile card: the cross-user row streamnook.app
//! writes into `user_profile_snapshot`, read in one query so the card draws at
//! once while its live sources resolve.
//!
//! Only rows the server wrote are used (`server_at` set): older apps wrote the
//! card they had drawn, links to files on their own disk included, and anyone
//! could write any member's row. Whatever the row holds is checked again here:
//! every image link must be a public web link, the avatar must come from
//! Twitch's CDN, and a name paint may only carry the style properties the card's
//! paint code produces, with every `url(...)` pointing at 7TV's CDN. When a row
//! is missing, invalidated or older than a minute, the server is asked to
//! rebuild it for the next viewer, at most once a minute per member.

use crate::services::web_link::{public_web_url, twitch_avatar_url};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// A row further than this from the PC clock, either way, is rebuilt for the
/// next viewer (the old app's rewrite throttle, kept). A skewed clock therefore
/// asks on every open, and the server answers `fresh` cheaply.
const REFRESH_AFTER_SECS: i64 = 60;
/// One rebuild ask per member per this long, from this app.
const ASK_GAP: Duration = Duration::from_secs(60);
/// Members remembered for the ask gap; older entries are dropped past this.
const ASKED_CAP: usize = 512;
const SNAPSHOT_API: &str = "https://streamnook.app/api/v1/profile/snapshot";

const PAINT_CDN: &str = "https://cdn.7tv.app/";
const PAINT_HOST: &str = "cdn.7tv.app";
/// Exactly the style properties the card's paint code (`computePaintStyle` in
/// src/services/paintStyle.ts) can emit.
const PAINT_KEYS: [&str; 10] = [
    "backgroundColor",
    "WebkitBackgroundClip",
    "backgroundClip",
    "backgroundSize",
    "backgroundPosition",
    "backgroundRepeat",
    "color",
    "backgroundImage",
    "filter",
    "opacity",
];
/// The CSS functions those properties use besides `url(...)`.
const PAINT_FUNCTIONS: [&str; 5] = [
    "linear-gradient",
    "repeating-linear-gradient",
    "radial-gradient",
    "repeating-radial-gradient",
    "drop-shadow",
];
const PAINT_CSS_MAX: usize = 16 * 1024;

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    pub login: String,
    pub display_name: String,
    /// A picture on Twitch's CDN, else empty.
    pub avatar: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct WornBadge {
    pub src: String,
    pub title: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ThirdPartyBadge {
    pub src: String,
    pub title: String,
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// The worn 7TV badge in the shape the card's badge row reads
/// (`getBadgeImageUrls` builds the CDN links from the id).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SeventvBadge {
    pub id: String,
    pub name: String,
    pub tooltip: String,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WornBadges {
    pub twitch: Option<WornBadge>,
    pub third_party: Vec<ThirdPartyBadge>,
    pub bttv_pro: Option<WornBadge>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Counts {
    pub paints: u32,
    pub badges: u32,
    pub sn: u32,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub profile_theme: String,
    pub hidden_sections: Vec<String>,
    pub member_number: Option<u32>,
    pub cosmetic_slug: Option<String>,
    /// The name's paint as style properties, only when it is within the paint
    /// code's output and every image in it is 7TV's.
    pub name_paint: Option<Map<String, Value>>,
    pub seventv_badge: Option<SeventvBadge>,
    pub worn_badges: WornBadges,
    pub counts: Counts,
}

/// What the card can draw before its live sources answer.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FirstPaint {
    pub identity: Identity,
    /// None when only the identity is known (no usable row).
    pub card: Option<Card>,
}

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or_default().to_string()
}

/// Whether `raw`, the inside of a `url(...)`, is a 7TV CDN link (quoted or not).
fn paint_image_url(raw: &str) -> bool {
    let trimmed = raw.trim();
    let bare = match trimmed.chars().next() {
        Some(q @ ('"' | '\'')) => match trimmed.strip_prefix(q).and_then(|rest| rest.strip_suffix(q)) {
            Some(inner) => inner,
            None => return false,
        },
        _ => trimmed,
    };
    if bare.is_empty() || bare.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        return false;
    }
    if !bare.starts_with(PAINT_CDN) || public_web_url(bare, true).is_none() {
        return false;
    }
    url::Url::parse(bare).is_ok_and(|u| {
        u.scheme() == "https" && u.host_str() == Some(PAINT_HOST) && u.username().is_empty() && u.password().is_none()
    })
}

/// Whether a style value only uses what the paint code produces: plain
/// gradients, drop shadows and `url(...)` to 7TV's CDN. Function names are read
/// in any case, nothing may nest, and any other function (`image-set(`, `src(`,
/// `var(`, ...) fails, so no image can be named any other way.
fn paint_css_ok(css: &str) -> bool {
    if css.len() > PAINT_CSS_MAX || css.contains(['\\', ';', '{', '}', '<', '>']) || css.contains("/*") {
        return false;
    }
    let mut rest = css;
    while let Some(open) = rest.find('(') {
        let before = &rest[..open];
        if before.contains([')', '"', '\'']) {
            return false;
        }
        let name_start = before
            .char_indices()
            .rev()
            .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '-'))
            .map_or(0, |(i, c)| i + c.len_utf8());
        let name = before[name_start..].to_ascii_lowercase();
        let after = &rest[open + 1..];
        let Some(close) = after.find(')') else { return false };
        let inner = &after[..close];
        if inner.contains('(') {
            return false;
        }
        let ok = if name == "url" {
            paint_image_url(inner)
        } else {
            PAINT_FUNCTIONS.contains(&name.as_str()) && !inner.contains(['"', '\''])
        };
        if !ok {
            return false;
        }
        rest = &after[close + 1..];
    }
    !rest.contains([')', '"', '\''])
}

/// The paint when every key is one the paint code emits and every value passes
/// `paint_css_ok` (`opacity` is a number from 0 to 1); else None, whole.
fn clean_paint(paint: &Map<String, Value>) -> Option<Map<String, Value>> {
    if paint.is_empty() {
        return None;
    }
    for (key, value) in paint {
        let ok = PAINT_KEYS.contains(&key.as_str())
            && match value {
                Value::String(s) => paint_css_ok(s),
                Value::Number(n) if key == "opacity" => n.as_f64().is_some_and(|o| (0.0..=1.0).contains(&o)),
                _ => false,
            };
        if !ok {
            return None;
        }
    }
    Some(paint.clone())
}

fn worn_badge(v: Option<&Value>) -> Option<WornBadge> {
    let v = v?.as_object()?;
    let src = public_web_url(v.get("src")?.as_str()?, true)?;
    Some(WornBadge { src, title: text(v.get("title")) })
}

fn seventv_badge(v: Option<&Value>) -> Option<SeventvBadge> {
    let v = v?.as_object()?;
    let id = text(v.get("id"));
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let name = text(v.get("name"));
    let tooltip = Some(text(v.get("tooltip"))).filter(|t| !t.is_empty()).unwrap_or_else(|| name.clone());
    Some(SeventvBadge { id, name, tooltip })
}

fn count(v: Option<&Value>) -> u32 {
    v.and_then(Value::as_u64).map(|n| n.min(u64::from(u32::MAX)) as u32).unwrap_or(0)
}

fn identity_of(login: String, display_name: &str, avatar: Option<&str>) -> Identity {
    Identity {
        display_name: if display_name.is_empty() { login.clone() } else { display_name.to_string() },
        avatar: avatar.and_then(twitch_avatar_url).unwrap_or_default(),
        login,
    }
}

/// The identity and card a server-written snapshot holds, keeping only what
/// passed the link rules. None for anything that is not a v1 snapshot with a
/// login.
pub fn card_from(snapshot: &Value) -> Option<(Identity, Card)> {
    let s = snapshot.as_object()?;
    if s.get("v").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let id = s.get("identity")?.as_object()?;
    let login = text(id.get("login"));
    if login.is_empty() {
        return None;
    }
    let identity = identity_of(login, &text(id.get("displayName")), id.get("avatar").and_then(Value::as_str));

    let name_paint = s.get("namePaint").and_then(Value::as_object).and_then(clean_paint);
    let worn = s.get("wornBadges").and_then(Value::as_object);
    let third_party = worn
        .and_then(|w| w.get("thirdParty"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|b| {
                    let o = b.as_object()?;
                    let src = public_web_url(o.get("src")?.as_str()?, true)?;
                    Some(ThirdPartyBadge {
                        src,
                        title: text(o.get("title")),
                        provider: text(o.get("provider")),
                        key: o.get("key").and_then(Value::as_str).map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let counts = s.get("counts").and_then(Value::as_object);
    let card = Card {
        profile_theme: Some(text(s.get("profileTheme"))).filter(|t| !t.is_empty()).unwrap_or_else(|| "tier".into()),
        hidden_sections: s
            .get("hiddenSections")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default(),
        member_number: s.get("memberNumber").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
        cosmetic_slug: s.get("cosmeticSlug").and_then(Value::as_str).filter(|c| !c.is_empty()).map(String::from),
        name_paint,
        seventv_badge: seventv_badge(s.get("seventvBadge")),
        worn_badges: WornBadges {
            twitch: worn_badge(worn.and_then(|w| w.get("twitch"))),
            third_party,
            bttv_pro: worn_badge(worn.and_then(|w| w.get("bttvPro"))),
        },
        counts: Counts {
            paints: count(counts.and_then(|c| c.get("paints"))),
            badges: count(counts.and_then(|c| c.get("badges"))),
            sn: count(counts.and_then(|c| c.get("sn"))),
        },
    };
    Some((identity, card))
}

/// The row's snapshot only when the server wrote it (`server_at` set). The view
/// shows a null snapshot while a change is waiting to be rebuilt.
fn server_snapshot(row: &Value) -> Option<&Value> {
    row.get("server_at").and_then(Value::as_str)?;
    row.get("snapshot").filter(|s| !s.is_null())
}

/// The card from the snapshot row, else the identity from the `users` row.
pub fn first_paint_from(row: Option<&Value>, user: Option<&Value>) -> Option<FirstPaint> {
    if let Some((identity, card)) = row.and_then(server_snapshot).and_then(card_from) {
        return Some(FirstPaint { identity, card: Some(card) });
    }
    let u = user?.as_object()?;
    let login = text(u.get("username"));
    if login.is_empty() {
        return None;
    }
    let identity = identity_of(login, &text(u.get("display_name")), u.get("avatar_url").and_then(Value::as_str));
    Some(FirstPaint { identity, card: None })
}

/// Whether the server should rebuild this member's row: none, hidden by a
/// change, written by an app, or more than a minute from the PC clock either way.
pub fn needs_refresh(row: Option<&Value>, now: DateTime<Utc>) -> bool {
    let Some(row) = row else { return true };
    if server_snapshot(row).is_none() {
        return true;
    }
    let at = row
        .get("server_at")
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc));
    match at {
        Some(at) => (now - at).num_milliseconds().abs() > REFRESH_AFTER_SECS * 1000,
        None => true,
    }
}

/// Whether to ask for a rebuild: only when the snapshot read answered (a
/// failure is not "no row") and the `users` read found the member (a stranger
/// has no row to build).
pub fn should_ask(snapshot_read_ok: bool, member_found: bool, row: Option<&Value>, now: DateTime<Utc>) -> bool {
    snapshot_read_ok && member_found && needs_refresh(row, now)
}

/// Remembers when each member was last asked for, so one is asked at most once
/// per `ASK_GAP`. Pure: the caller passes the time.
#[derive(Default)]
pub struct AskGate {
    asked: HashMap<String, Instant>,
}

impl AskGate {
    /// True the first time in `ASK_GAP` for this member, and records the ask.
    pub fn allow(&mut self, user_id: &str, now: Instant) -> bool {
        if self.asked.get(user_id).is_some_and(|at| now.saturating_duration_since(*at) < ASK_GAP) {
            return false;
        }
        if self.asked.len() >= ASKED_CAP {
            self.asked.retain(|_, at| now.saturating_duration_since(*at) < ASK_GAP);
            if self.asked.len() >= ASKED_CAP {
                self.asked.clear();
            }
        }
        self.asked.insert(user_id.to_string(), now);
        true
    }
}

static GATE: OnceLock<Mutex<AskGate>> = OnceLock::new();

/// Takes the lock for the length of one call, never across an await.
fn ask_allowed(user_id: &str) -> bool {
    let gate = GATE.get_or_init(|| Mutex::new(AskGate::default()));
    let mut gate = gate.lock().unwrap_or_else(|e| e.into_inner());
    gate.allow(user_id, Instant::now())
}

/// A Twitch user id: 1 to 15 digits.
fn valid_id(user_id: &str) -> bool {
    !user_id.is_empty() && user_id.len() <= 15 && user_id.bytes().all(|b| b.is_ascii_digit())
}

/// The first paint for a member's card, and a rebuild ask when their row is
/// stale. Two reads in parallel (the snapshot row and the `users` row). The
/// view and the old table both answer `select=*`, so this works on either.
pub async fn first_paint(user_id: &str) -> Option<FirstPaint> {
    if !valid_id(user_id) {
        return None;
    }
    use crate::services::streamnook_backend::select;
    let row_query = format!("select=*&twitch_user_id=eq.{user_id}&limit=1");
    let user_query = format!("select=username,display_name,avatar_url&id=eq.{user_id}&limit=1");
    let (rows, users) = tokio::join!(select("user_profile_snapshot", &row_query), select("users", &user_query));
    if let Err(e) = &rows {
        log::debug!("[ProfileSnapshot] snapshot read failed: {e}");
    }
    let row = rows.as_ref().ok().and_then(|r| r.first());
    let user = users.as_ref().ok().and_then(|u| u.first());
    if should_ask(rows.is_ok(), user.is_some(), row, Utc::now()) && ask_allowed(user_id) {
        let url = format!("{SNAPSHOT_API}/{user_id}/refresh");
        tauri::async_runtime::spawn(async move {
            let request = crate::services::http::client().post(&url);
            match crate::services::sn_transport::send(request).await {
                Ok(resp) if !resp.status().is_success() => {
                    log::debug!("[ProfileSnapshot] rebuild ask answered {}", resp.status());
                }
                Ok(_) => {}
                Err(e) => log::debug!("[ProfileSnapshot] rebuild ask failed: {e}"),
            }
        });
    }
    first_paint_from(row, user)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const GOLDEN: &str = include_str!("../../../src/components/profile/__fixtures__/profile-snapshot.golden.json");
    const AVATAR: &str = "https://static-cdn.jtvnw.net/jtv_user_pictures/golden-profile_image-300x300.png";
    const SEVENTV_IMAGE: &str =
        "https://cdn.7tv.app/paint/01GOLDENPAINT0000000000000/layer/01GOLDENLAYERIMAGE00000000/1x.webp";

    fn golden() -> Value {
        serde_json::from_str(GOLDEN).expect("golden fixture")
    }

    /// The golden snapshot as a row the server wrote.
    fn row(snapshot: Value) -> Value {
        json!({ "twitch_user_id": "1", "snapshot": snapshot, "updated_at": "2026-10-30T20:00:00+00:00", "server_at": "2026-10-30T20:00:00+00:00" })
    }

    fn user() -> Value {
        json!({ "username": "golden_member", "display_name": "Golden_Member", "avatar_url": AVATAR })
    }

    fn card_of(snapshot: &Value) -> Card {
        card_from(snapshot).expect("card").1
    }

    fn paint_with(image: &str) -> Value {
        let mut s = golden();
        s["namePaint"]["backgroundImage"] = json!(image);
        s
    }

    // ---- The golden row (what released apps read) -----------------------------------

    #[test]
    fn card_from_reads_every_value_of_the_golden_row() {
        let (identity, card) = card_from(&golden()).expect("golden is a v1 card");
        assert_eq!(identity, Identity { login: "golden_member".into(), display_name: "Golden_Member".into(), avatar: AVATAR.into() });
        assert_eq!(card.profile_theme, "dispersion");
        assert_eq!(card.hidden_sections, vec!["roast".to_string()]);
        assert_eq!(card.member_number, Some(42));
        assert_eq!(card.cosmetic_slug.as_deref(), Some("streamnook-penrose"));
        assert_eq!(
            card.seventv_badge,
            Some(SeventvBadge { id: "01KYQHQ7FVRR3X4S0F2KG74207".into(), name: "XDX".into(), tooltip: "XDX".into() })
        );
        assert_eq!(
            card.worn_badges.twitch,
            Some(WornBadge {
                src: "https://static-cdn.jtvnw.net/badges/v1/0d2d0ac5-5ec3-4c3e-a2b4-3eaa5d6f7a3b/3".into(),
                title: "Twitch Recap 2025".into()
            })
        );
        assert_eq!(
            card.worn_badges.third_party,
            vec![ThirdPartyBadge {
                src: "https://cdn.frankerfacez.com/badge/3/4/rounded".into(),
                title: "FFZ Supporter".into(),
                provider: "ffz".into(),
                key: Some("ffz:ffz-3".into())
            }]
        );
        assert_eq!(
            card.worn_badges.bttv_pro,
            Some(WornBadge {
                src: "https://cdn.betterttv.net/badges/pro/97c7dfb8-d2a5-4ab0-9e42-e9e14630da5b.webp".into(),
                title: "BTTV Pro".into()
            })
        );
        assert_eq!(card.counts, Counts { paints: 12, badges: 7, sn: 3 });
        // The paint passes whole, within the property whitelist and 7TV's CDN.
        let paint = card.name_paint.expect("the golden paint is clean");
        assert!(paint.keys().all(|k| PAINT_KEYS.contains(&k.as_str())));
        let image = paint["backgroundImage"].as_str().unwrap();
        assert!(image.contains(&format!("url(\"{SEVENTV_IMAGE}\")")), "{image}");
        assert!(image.starts_with("linear-gradient(90deg, #ff0000 0%, #0000ff 100%)"));
        assert_eq!(paint["filter"], "drop-shadow(#000000 0px 1px 2px)");
        assert!(!GOLDEN.contains("localUrl"));
    }

    #[test]
    fn the_answer_serializes_in_the_shape_the_page_types_read() {
        let answer = first_paint_from(Some(&row(golden())), Some(&user())).unwrap();
        let v = serde_json::to_value(&answer).unwrap();
        assert_eq!(v["identity"]["displayName"], "Golden_Member");
        let card = &v["card"];
        for key in ["profileTheme", "hiddenSections", "memberNumber", "cosmeticSlug", "namePaint", "seventvBadge", "wornBadges", "counts"] {
            assert!(card.get(key).is_some(), "card.{key}");
        }
        assert_eq!(card["wornBadges"]["thirdParty"][0]["provider"], "ffz");
        assert!(card["wornBadges"].get("bttvPro").is_some());
        assert_eq!(card["seventvBadge"], json!({ "id": "01KYQHQ7FVRR3X4S0F2KG74207", "name": "XDX", "tooltip": "XDX" }));
        // No row: the card is null, never absent.
        let bare = serde_json::to_value(first_paint_from(None, Some(&user())).unwrap()).unwrap();
        assert!(bare["card"].is_null());
    }

    #[test]
    fn a_snapshot_that_is_not_v1_with_a_login_is_not_a_card() {
        let mut s = golden();
        s["v"] = json!(2);
        assert!(card_from(&s).is_none());
        let mut s = golden();
        s["identity"]["login"] = json!("");
        assert!(card_from(&s).is_none());
        assert!(card_from(&json!("nope")).is_none());
    }

    // ---- Which rows are trusted -----------------------------------------------------

    #[test]
    fn a_row_without_server_at_is_never_a_card() {
        let mut app_written = row(golden());
        app_written.as_object_mut().unwrap().remove("server_at");
        let answer = first_paint_from(Some(&app_written), Some(&user())).unwrap();
        assert!(answer.card.is_none());
        assert_eq!(answer.identity.login, "golden_member");
        // The old table has no such column at all.
        let old_table = json!({ "twitch_user_id": "1", "snapshot": golden(), "updated_at": "2026-10-30T20:00:00+00:00" });
        assert!(first_paint_from(Some(&old_table), Some(&user())).unwrap().card.is_none());
    }

    #[test]
    fn a_hidden_snapshot_falls_back_to_the_users_row() {
        let hidden = json!({ "twitch_user_id": "1", "snapshot": null, "updated_at": null, "server_at": "2026-10-30T20:00:00+00:00" });
        assert!(first_paint_from(Some(&hidden), Some(&user())).unwrap().card.is_none());
        assert!(first_paint_from(None, Some(&user())).unwrap().card.is_none());
    }

    #[test]
    fn no_card_and_no_member_is_no_answer() {
        assert!(first_paint_from(None, None).is_none());
        assert!(first_paint_from(Some(&row(golden())), None).unwrap().card.is_some());
        assert!(first_paint_from(None, Some(&json!({ "username": "", "display_name": "X" }))).is_none());
    }

    #[test]
    fn the_users_row_gives_the_identity_with_a_checked_avatar() {
        let a = first_paint_from(None, Some(&user())).unwrap().identity;
        assert_eq!(a, Identity { login: "golden_member".into(), display_name: "Golden_Member".into(), avatar: AVATAR.into() });
        let local = json!({ "username": "x", "avatar_url": "http://asset.localhost/C%3A%5CUsers%5Cx%5Ca.png" });
        let b = first_paint_from(None, Some(&local)).unwrap().identity;
        assert_eq!((b.display_name.as_str(), b.avatar.as_str()), ("x", ""));
    }

    // ---- The sanitizer --------------------------------------------------------------

    #[test]
    fn an_avatar_off_twitchs_cdn_is_dropped() {
        for bad in [
            "http://asset.localhost/C%3A%5CUsers%5Cx%5Ca.png",
            "asset://localhost/C:/Users/x/a.png",
            "C:\\Users\\x\\a.png",
            "http://127.0.0.1:24567/cache/a.png",
            "https://example.com/a.png",
            "",
        ] {
            let mut s = golden();
            s["identity"]["avatar"] = json!(bad);
            let (identity, _) = card_from(&s).unwrap();
            assert_eq!(identity.avatar, "", "{bad}");
        }
    }

    #[test]
    fn a_paint_with_a_local_image_is_dropped_whole() {
        for bad in [
            "url(\"http://asset.localhost/C%3A%5CUsers%5Cx%5Cp.webp\")",
            "linear-gradient(90deg, #ff0000 0%, #0000ff 100%), url(\"http://127.0.0.1:24567/cache/p.webp\")",
            "url(asset://localhost/C:/Users/x/p.webp)",
            "url(C:\\Users\\x\\p.webp)",
            "url(file:///C:/Users/x/p.webp)",
            "url(data:image/png;base64,AAAA)",
        ] {
            assert!(card_of(&paint_with(bad)).name_paint.is_none(), "{bad}");
        }
    }

    #[test]
    fn url_is_checked_in_any_case_quoted_or_not() {
        for bad in [
            "URL(http://asset.localhost/x.webp)",
            "Url(\"https://example.com/x.webp\")",
            "uRL('http://127.0.0.1/x.webp')",
            "url(https://example.com/x.webp)",
            "url( https://cdn.7tv.app.evil.com/x.webp )",
            "url(https://cdn.7tv.app@evil.com/x.webp)",
            "url(\"https://cdn.7tv.app/x.webp)",
            "url(http://cdn.7tv.app/x.webp)",
            "url(https://cdn.7tv.app/a b.webp)",
        ] {
            assert!(card_of(&paint_with(bad)).name_paint.is_none(), "{bad}");
        }
        for good in [
            format!("url({SEVENTV_IMAGE})"),
            format!("url(\"{SEVENTV_IMAGE}\")"),
            format!("URL('{SEVENTV_IMAGE}')"),
            format!("url( {SEVENTV_IMAGE} )"),
        ] {
            assert!(card_of(&paint_with(&good)).name_paint.is_some(), "{good}");
        }
    }

    #[test]
    fn image_set_cannot_smuggle_an_image() {
        for bad in [
            "image-set(url(\"http://asset.localhost/x.webp\") 1x)",
            "-webkit-image-set(\"http://asset.localhost/x.webp\" 1x)",
            "IMAGE-SET(\"C:\\\\Users\\\\x\\\\p.webp\" 1x)",
            "image-set(\"https://cdn.7tv.app/x.webp\" 1x)",
            "src(\"https://cdn.7tv.app/x.webp\")",
            "cross-fade(url(https://cdn.7tv.app/x.webp), red)",
            "var(--user-color)",
            "linear-gradient(90deg, url(https://cdn.7tv.app/x.webp))",
            "u\\72l(http://127.0.0.1/x.webp)",
            "url(https://cdn.7tv.app/x.webp /* */)",
        ] {
            assert!(card_of(&paint_with(bad)).name_paint.is_none(), "{bad}");
        }
    }

    #[test]
    fn a_paint_key_the_card_never_emits_drops_the_paint() {
        for key in ["background", "backgroundImageExtra", "behavior", "content", "mask", "listStyleImage", "cursor"] {
            let mut s = golden();
            s["namePaint"][key] = json!("none");
            assert!(card_of(&s).name_paint.is_none(), "{key}");
        }
        let mut s = golden();
        s["namePaint"]["opacity"] = json!(0.5);
        assert!(card_of(&s).name_paint.is_some());
        s["namePaint"]["opacity"] = json!(5);
        assert!(card_of(&s).name_paint.is_none());
        let mut s = golden();
        s["namePaint"]["backgroundSize"] = json!(100);
        assert!(card_of(&s).name_paint.is_none(), "a number outside opacity");
        // An empty paint is no paint.
        let mut s = golden();
        s["namePaint"] = json!({});
        assert!(card_of(&s).name_paint.is_none());
    }

    #[test]
    fn every_property_the_paint_code_emits_passes() {
        let mut s = golden();
        s["namePaint"] = json!({
            "backgroundColor": "#111111", "WebkitBackgroundClip": "text", "backgroundClip": "text",
            "backgroundSize": "100% 100%", "backgroundPosition": "center", "backgroundRepeat": "no-repeat",
            "color": "transparent",
            "backgroundImage": "repeating-radial-gradient(circle, #222222 50%), repeating-linear-gradient(0deg, #333333 33.33333333333333%, #444444 100%)",
            "filter": "drop-shadow(#000000 0px 1px 2px) drop-shadow(#ffffff 0px -1px 1.5px)",
            "opacity": 0.5
        });
        assert!(card_of(&s).name_paint.is_some());
    }

    #[test]
    fn a_badge_with_a_local_or_private_link_is_dropped() {
        let mut s = golden();
        s["wornBadges"]["twitch"]["src"] = json!("http://asset.localhost/C%3A%5CUsers%5Cx%5Cb.webp");
        s["wornBadges"]["bttvPro"]["src"] = json!("http://127.0.0.1:24567/cache/b.webp");
        s["wornBadges"]["thirdParty"] = json!([
            { "key": "ffz:1", "provider": "ffz", "title": "Local", "src": "C:\\Users\\x\\b.png" },
            { "key": "ffz:2", "provider": "ffz", "title": "Intranet", "src": "https://intranet/b.png" },
            { "key": "ffz:3", "provider": "ffz", "title": "Plain http", "src": "http://cdn.frankerfacez.com/b.png" },
            { "key": "ffz:4", "provider": "ffz", "title": "Kept", "src": "https://cdn.frankerfacez.com/b.png" },
        ]);
        let card = card_of(&s);
        assert!(card.worn_badges.twitch.is_none());
        assert!(card.worn_badges.bttv_pro.is_none());
        let kept: Vec<_> = card.worn_badges.third_party.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(kept, vec!["Kept"]);
    }

    #[test]
    fn a_seventv_badge_id_is_a_plain_id() {
        for bad in ["", "../x", "a/b", "a b", "a?x=1"] {
            let mut s = golden();
            s["seventvBadge"]["id"] = json!(bad);
            assert!(card_of(&s).seventv_badge.is_none(), "{bad:?}");
        }
        let mut s = golden();
        s["seventvBadge"] = json!({ "id": "01ABC", "name": "Name" });
        assert_eq!(card_of(&s).seventv_badge.unwrap().tooltip, "Name");
    }

    #[test]
    fn a_third_party_badge_always_has_a_provider_string() {
        let mut s = golden();
        s["wornBadges"]["thirdParty"] = json!([{ "title": "No provider", "src": "https://cdn.frankerfacez.com/b.png" }]);
        let card = card_of(&s);
        assert_eq!(card.worn_badges.third_party[0].provider, "");
        assert!(card.worn_badges.third_party[0].key.is_none());
    }

    // ---- When to ask the server -----------------------------------------------------

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn written(server_at: &str) -> Value {
        json!({ "snapshot": golden(), "server_at": server_at })
    }

    #[test]
    fn a_missing_hidden_or_app_written_row_needs_a_rebuild() {
        let now = at("2026-10-30T20:00:30Z");
        assert!(needs_refresh(None, now));
        assert!(needs_refresh(Some(&json!({ "snapshot": null, "server_at": "2026-10-30T20:00:20Z" })), now));
        assert!(needs_refresh(Some(&json!({ "snapshot": golden(), "updated_at": "2026-10-30T20:00:20Z" })), now));
        assert!(needs_refresh(Some(&written("not a time")), now));
        assert!(!needs_refresh(Some(&written("2026-10-30T20:00:20Z")), now));
    }

    #[test]
    fn a_row_more_than_a_minute_away_in_either_direction_needs_a_rebuild() {
        let now = at("2026-10-30T20:10:00Z");
        assert!(!needs_refresh(Some(&written("2026-10-30T20:09:01Z")), now));
        assert!(!needs_refresh(Some(&written("2026-10-30T20:09:00Z")), now));
        assert!(needs_refresh(Some(&written("2026-10-30T20:08:59Z")), now));
        // A clock set behind the server: the row is "from the future".
        assert!(!needs_refresh(Some(&written("2026-10-30T20:10:59Z")), now));
        assert!(needs_refresh(Some(&written("2026-10-30T20:11:01Z")), now));
        assert!(needs_refresh(Some(&written("2026-10-30T20:09:00.001+00:00")), at("2026-10-30T20:10:00.002Z")));
    }

    #[test]
    fn it_asks_only_when_the_read_answered_and_the_member_exists() {
        let now = at("2026-10-30T20:10:00Z");
        let stale = written("2026-10-30T19:00:00Z");
        assert!(should_ask(true, true, Some(&stale), now));
        assert!(should_ask(true, true, None, now));
        assert!(!should_ask(false, true, None, now), "a failed read is not 'no row'");
        assert!(!should_ask(true, false, None, now), "a stranger has no row to build");
        assert!(!should_ask(true, true, Some(&written("2026-10-30T20:09:50Z")), now));
    }

    #[test]
    fn the_ask_gate_lets_one_through_a_minute_per_member() {
        let t0 = Instant::now();
        let mut gate = AskGate::default();
        assert!(gate.allow("1", t0));
        assert!(!gate.allow("1", t0));
        assert!(!gate.allow("1", t0 + Duration::from_secs(59)));
        assert!(gate.allow("2", t0 + Duration::from_secs(1)), "another member is its own gate");
        assert!(gate.allow("1", t0 + Duration::from_secs(60)));
        assert!(!gate.allow("1", t0 + Duration::from_secs(61)));
    }

    #[test]
    fn the_ask_gate_stays_bounded() {
        let t0 = Instant::now();
        let mut gate = AskGate::default();
        for n in 0..ASKED_CAP {
            assert!(gate.allow(&n.to_string(), t0));
        }
        // Full of fresh entries: the next one clears the table rather than grow.
        assert!(gate.allow("new", t0 + Duration::from_secs(1)));
        assert!(gate.asked.len() <= ASKED_CAP);
        // Full of expired entries: only they are dropped.
        let mut gate = AskGate::default();
        for n in 0..ASKED_CAP {
            gate.allow(&n.to_string(), t0);
        }
        assert!(gate.allow("late", t0 + Duration::from_secs(120)));
        assert_eq!(gate.asked.len(), 1);
    }

    #[test]
    fn only_twitch_user_ids_are_read() {
        assert!(valid_id("12826"));
        for bad in ["", "abc", "1;drop", "12 3", "1234567890123456", "-1", "1&select=*"] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }
}
