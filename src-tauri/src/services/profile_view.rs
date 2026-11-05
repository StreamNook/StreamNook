//! What a profile surface shows, worked out once in Rust: the user profile sheet
//! a name tap opens, the viewer's own identity header, and the Cosmetics
//! screen's picker state.
//!
//! The phone shells used to gather these from four or five reads each and merge
//! the badge sources in TypeScript (`utils/profileBadges.ts`), with rules that
//! are fiddlier than they look: every provider spells its image fields
//! differently, Twitch splits one badge across its display and earned lists, and
//! the header must show what is WORN where the sheet shows what is OWNED. One
//! answer per surface keeps the shells to presentation.

use crate::commands::chat_identity::ChatIdentityBadge;
use crate::commands::identity::IdentityLoadout;
use crate::models::chat_layout::PaintView;
use crate::models::user::UserInfo;
use crate::services::badge_service::UserBadgesResponse;
use crate::services::bttv_pro_service::{self, BttvProBadge};
use crate::services::cosmetics_registry::{self as registry, SnBadge};
use crate::services::css_fill::Fill;
use crate::services::member_link::{self, PageLink};
use crate::services::member_profile::{self, Clock, MemberFacts, MemberProfile, MemberRead, Part, ShapeCtx};
use crate::services::member_theme;
use crate::services::seventv_cosmetics_resolver::{self as resolver, ProfilePicture, UserCosmetics};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::watch;

/// One badge as a profile surface draws it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ProfileBadge {
    pub id: String,
    /// The image to draw (the 4x where the provider has one).
    pub url: String,
    /// Tried in order when `url` fails; all failing hides the badge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

impl ProfileBadge {
    /// What the badge is called aloud: its title, else its name, else "Badge".
    pub fn label(&self) -> String {
        if !self.title.is_empty() {
            self.title.clone()
        } else {
            self.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "Badge".into())
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BadgeGroup {
    /// "Twitch", "7TV" or "Chat clients".
    pub title: String,
    pub badges: Vec<ProfileBadge>,
}

/// A 7TV profile picture reduced to the two renditions one surface draws.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AvatarPick {
    pub still: String,
    /// Present when the picture moves and the viewer's setting lets this
    /// surface animate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animated: Option<String>,
}

/// The rendition for a box of `px` points at `scale` screen pixels per point:
/// the smallest scale covering `px * scale / 32` (7TV's 1x is 32 px), else the
/// largest. `focal` surfaces (the sheet, the You header) move under the
/// "hover" setting; "never" always draws the still.
pub fn pick_avatar(picture: &ProfilePicture, px: f32, scale: f32, focal: bool) -> Option<AvatarPick> {
    let need = (px * scale / 32.0).ceil().max(1.0) as u8;
    let choose = |list: &[resolver::PictureImage]| -> Option<String> {
        list.iter().find(|i| i.scale >= need).or_else(|| list.iter().max_by_key(|i| i.scale)).map(|i| i.url.clone())
    };
    let still = choose(&picture.still).or_else(|| choose(&picture.animated))?;
    let mode = crate::services::providers::setting(|s| s.cosmetics.profile_picture_animation.clone())
        .unwrap_or_else(|| "always".into());
    let moves = match mode.as_str() {
        "never" => false,
        "hover" => focal,
        _ => true,
    };
    let animated = if moves { choose(&picture.animated) } else { None };
    Some(AvatarPick { still, animated })
}

/// 7TV's best image of a badge for a scale: the asked scale, the animated (not
/// `_static`) file, then webp, avif, png, gif. A badge's id is not its image id,
/// so the listed URLs come first and the legacy CDN path is the fallback.
fn seventv_badge_image(def: &Value, scale: u64) -> Option<String> {
    let images = def.get("images").and_then(Value::as_array)?;
    images
        .iter()
        .filter_map(|i| {
            let url = i.get("url").and_then(Value::as_str)?;
            let mime = i.get("mime").and_then(Value::as_str).unwrap_or_default();
            let s = i.get("scale").and_then(Value::as_u64).unwrap_or(1);
            let mut rank = s.abs_diff(scale) * 10;
            if url.contains("_static") {
                rank += 3;
            }
            rank += if mime.contains("webp") { 0 } else if mime.contains("avif") { 1 } else if mime.contains("png") { 2 } else { 4 };
            Some((rank, url.to_string()))
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, url)| url)
}

pub fn seventv_badge(def: &Value) -> Option<ProfileBadge> {
    let id = def.get("id").and_then(Value::as_str)?.to_string();
    let legacy = format!("https://cdn.7tv.app/badge/{id}");
    let text = |k: &str| def.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    Some(ProfileBadge {
        url: seventv_badge_image(def, 4).unwrap_or_else(|| format!("{legacy}/4x.webp")),
        fallbacks: ["3x", "2x", "1x"].iter().map(|s| format!("{legacy}/{s}.webp")).collect(),
        title: text("tooltip").or_else(|| text("description")).or_else(|| text("name")).unwrap_or_default(),
        name: text("name"),
        id,
        ..Default::default()
    })
}

/// Twitch badges: the displayed set then every earned one not already there,
/// deduped by id, art at 4x with the 2x and 1x behind it.
pub fn twitch_badges(resp: &UserBadgesResponse) -> Vec<ProfileBadge> {
    let mut seen = HashSet::new();
    resp.display_badges
        .iter()
        .chain(resp.earned_badges.iter())
        .filter_map(|b| {
            let info = &b.badge_info;
            let id = if info.id.is_empty() { format!("{}-{}", info.set_id, info.version) } else { info.id.clone() };
            if !seen.insert(id.clone()) {
                return None;
            }
            let urls: Vec<String> = [&info.image_4x, &info.image_2x, &info.image_1x]
                .into_iter()
                .filter(|u| !u.is_empty())
                .cloned()
                .collect();
            let (url, fallbacks) = urls.split_first()?;
            Some(ProfileBadge {
                id,
                url: url.clone(),
                fallbacks: fallbacks.to_vec(),
                title: info.title.clone(),
                description: Some(info.description.clone()).filter(|d| !d.is_empty()),
                ..Default::default()
            })
        })
        .collect()
}

/// Chat-client badges (FFZ, Chatterino, Homies and the rest), deduped by id.
pub fn third_party_badges(resp: &UserBadgesResponse) -> Vec<ProfileBadge> {
    let mut seen = HashSet::new();
    resp.third_party_badges
        .iter()
        .filter_map(|b| {
            let info = &b.badge_info;
            if !seen.insert(info.id.clone()) {
                return None;
            }
            let urls: Vec<String> = [&info.image_4x, &info.image_2x, &info.image_1x]
                .into_iter()
                .filter(|u| !u.is_empty())
                .cloned()
                .collect();
            let (url, fallbacks) = urls.split_first()?;
            Some(ProfileBadge {
                id: info.id.clone(),
                url: url.clone(),
                fallbacks: fallbacks.to_vec(),
                title: info.title.clone(),
                provider: Some(b.provider.as_key().to_string()),
                ..Default::default()
            })
        })
        .collect()
}

fn selected(list: &[Value]) -> Option<&Value> {
    list.iter().find(|v| v.get("selected").and_then(Value::as_bool).unwrap_or(false))
}

/// The paint a person wears: what chat already knows first, else the selected
/// one in their inventory. The 7TV definition as 7TV wrote it (the web pages
/// paint it with CSS) and the same paint as native layers.
fn worn_paint(cosmetics_id: &str, inventory: Option<&UserCosmetics>) -> (Option<Value>, Option<PaintView>) {
    let def = resolver::peek(cosmetics_id)
        .and_then(|c| selected(&c.paints).cloned())
        .or_else(|| inventory.and_then(|i| selected(&i.paints).cloned()));
    let view = def.as_ref().and_then(crate::services::chat_row_view::paint_view);
    (def, view)
}

/// What a member wears on their profile besides their badges.
#[derive(Clone, Debug, Default)]
struct Look {
    /// The Atmosphere that paints, after the wear gate.
    atmosphere: Option<registry::Atmosphere>,
    /// The Cologne event look instead of an Atmosphere.
    cologne: Option<member_theme::Cologne>,
    /// The Cologne look's accent as "r, g, b", read off its catalog row.
    cologne_accent: Option<String>,
    /// They picked their 7TV paint as the profile theme.
    paint_theme: bool,
}

/// The look a member wears on their profile: an Atmosphere (wear-gated), the
/// Cologne look, or their paint as the theme. Non-members wear none.
async fn worn_look(member_id: &str) -> Look {
    match member_theme::resolve(member_id).await {
        Some(theme) => Look {
            atmosphere: theme.atmosphere_id.as_deref().and_then(registry::atmosphere),
            cologne: theme.cologne,
            cologne_accent: theme.cologne.and_then(|_| registry::atmosphere(registry::COLOGNE_THEME_ID)).map(|a| a.accent),
            paint_theme: theme.theme.as_deref() == Some("paint"),
        },
        None => Look::default(),
    }
}

/// A Twitch account's profile banner, read anonymously (public data); None
/// when it has none or Twitch does not answer.
async fn twitch_banner(user_id: &str) -> Option<String> {
    let body = serde_json::json!({
        "query": "query ProfileBanner($id: ID!) { user(id: $id) { bannerImageURL } }",
        "variables": { "id": user_id },
    });
    let json = crate::services::twitch_web_gql::post(None, body, "ProfileBanner").await.ok()?;
    banner_from_answer(&json)
}

fn banner_from_answer(json: &Value) -> Option<String> {
    json.pointer("/data/user/bannerImageURL").and_then(Value::as_str).filter(|u| !u.is_empty()).map(String::from)
}

/// `@login` when it says something the display name does not (they differ
/// beyond letter case).
fn shown_login(login: &str, display_name: &str) -> Option<String> {
    (!login.is_empty() && !login.eq_ignore_ascii_case(display_name)).then(|| login.to_string())
}

// ---- The profile sheet ---------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ProfileSheet {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_login: Option<String>,
    /// The Helix profile picture; None until (or unless) Helix answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// The 7TV picture drawn over it, when the viewer shows 7TV pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<AvatarPick>,
    /// The name colour after the readability pass.
    pub name_color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<PaintView>,
    /// The same paint as 7TV defines it, for renderers that paint with CSS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint_def: Option<Value>,
    /// Non-empty groups in their fixed order, images only.
    pub groups: Vec<BadgeGroup>,
    /// Present only for a StreamNook member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamnook: Option<SnBadge>,
    /// A member's number and rank, drawn on the right of the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member: Option<MemberRank>,
    /// The Atmosphere they wear on their profile, after the wear gate. It shows
    /// here whether or not they keep it out of chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atmosphere: Option<registry::Atmosphere>,
    /// The Cologne event look instead of an Atmosphere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cologne: Option<member_theme::Cologne>,
    /// A member's relics, overview, accolades and top emotes, with what they
    /// hide left out. Their StreamNook badges ride `groups`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_profile: Option<MemberProfile>,
    /// The member's StreamNook page, built from their Twitch id and the login
    /// Twitch holds for it. Absent for a non-member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_link: Option<PageLink>,
    /// What they WEAR, in surface order, capped like the You card's row (members only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub worn: Vec<ProfileBadge>,
    /// Their 7TV account, for the "open on 7TV" button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seventv_user_id: Option<String>,
    /// How the profile is themed: `tier`, `paint` or `atmosphere` (members only;
    /// the Cologne look counts as an atmosphere).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The Frame they equipped, with the nine-slice numbers to draw it by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<FrameView>,
    /// The profile's accent as #rrggbb (members only): the Atmosphere's, the
    /// paint's middle color, else the rank's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    /// The name of the StreamNook badge they wear, none for the default one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

/// An equipped Frame: its art and the numbers the desktop card cuts it by
/// (`src/utils/profileFrame.json`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FrameView {
    pub url: String,
    /// The nine-slice cut: top, right, bottom, left.
    pub slice: [u16; 4],
    /// The border width: vertical, horizontal.
    pub border: [u16; 2],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameNumbers {
    slice: [u16; 4],
    border: [u16; 2],
}

static FRAME: LazyLock<FrameNumbers> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../src/utils/profileFrame.json")).expect("profileFrame.json")
});

/// A member's place in line as the profile cards show it: the number, its rank
/// and the rank's color. The bands and the famous numbers match the desktop
/// card's tier card.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MemberRank {
    pub number: u32,
    /// `ethereal`, `mythic`, `ascendant`, `founder` or `member`.
    pub tier: String,
    /// The rank's name, or a famous number's own ("Nice" for 69).
    pub label: String,
    /// The rank's color as #rrggbb.
    pub accent: String,
    /// How strong the rank's ambient glow is when no Atmosphere is worn (the
    /// desktop card's `getTierAccent` `auraAlpha`), two decimal places.
    #[serde(default)]
    pub aura: f64,
}

pub fn member_rank(number: u32) -> MemberRank {
    let (tier, label, accent, aura) = match number {
        1 => ("ethereal", "Ethereal", "#8B5CF6", 0.16),
        2..=10 => ("mythic", "Mythic", "#FBBF24", 0.13),
        11..=100 => ("ascendant", "Ascendant", "#22D3EE", 0.10),
        101..=250 => ("founder", "Founder", "#E2E8F0", 0.05),
        _ => ("member", "Member", "#94A3B8", 0.04),
    };
    let famous = match number {
        42 => Some("The Answer"),
        67 => Some("Six Seven"),
        69 => Some("Nice"),
        141 => Some("Going Dark"),
        404 => Some("Not Found"),
        420 => Some("Blaze It"),
        1337 => Some("Leet"),
        9000 => Some("Over 9000"),
        _ => None,
    };
    MemberRank {
        number,
        tier: tier.into(),
        label: famous.unwrap_or(label).into(),
        accent: accent.into(),
        aura,
    }
}

/// The person's Twitch badges for the sheet. Outside a room a channel's badges
/// (broadcaster, moderator, subscriber and the rest) say nothing, and Twitch
/// would answer for the person's own channel, where everyone who streams is
/// the broadcaster; so they show only in the room the sheet was opened from.
/// The earned list is never one room's, so it never carries them.
fn sheet_twitch_badges(resp: &UserBadgesResponse, in_room: bool) -> Vec<ProfileBadge> {
    let personal = |b: &&crate::services::badge_service::UserBadge| !CHANNEL_SCOPED.contains(&b.badge_info.set_id.as_str());
    twitch_badges(&UserBadgesResponse {
        display_badges: resp.display_badges.iter().filter(|b| in_room || personal(b)).cloned().collect(),
        earned_badges: resp.earned_badges.iter().filter(personal).cloned().collect(),
        third_party_badges: Vec::new(),
    })
}

// ---- Chat-client groups, the worn row and the loadout -----------------------------------

/// Chat-client badge groups, in `badge_service::THIRD_PARTY_ORDER`'s order, with
/// the name each is shown under. A provider this list does not know falls into
/// "Other", so a new Rust-side provider can never vanish. The desktop card's
/// `THIRD_PARTY_PROVIDER_GROUPS` is pinned to this list by a test.
pub const PROVIDER_GROUPS: [(&str, &str); 10] = [
    ("ffz", "FrankerFaceZ"),
    ("bttv", "BetterTTV"),
    ("chatterino", "Chatterino"),
    ("homies", "Homies"),
    ("moltorino", "Moltorino"),
    ("limerino", "Limerino"),
    ("jilchat", "JilChat"),
    ("chatsen", "Chatsen"),
    ("chatty", "Chatty"),
    ("dankchat", "DankChat"),
];

/// The loadout key a member's BetterTTV Pro badge is stored under. It spells the
/// provider in capitals on purpose (the persisted key the desktop editor writes),
/// so it is matched literally, never rebuilt from the badge.
pub const BTTV_PRO_KEY: &str = "BTTV:bttv-pro";
const BTTV_PRO_ID: &str = "bttv-pro";

/// Which chat-client badges a member's loadout lets show.
#[derive(Clone, Debug, PartialEq)]
enum LoadoutFilter {
    /// Everything (a non-member, or a member who never curated).
    All,
    /// Only the badges whose `<provider>:<id>` key the member chose, possibly none.
    Only(HashSet<String>),
    /// A member whose loadout could not be read and is not cached.
    Unknown,
}

fn loadout_filter(is_member: bool, loadout: &Part<IdentityLoadout>) -> LoadoutFilter {
    if !is_member {
        return LoadoutFilter::All;
    }
    match loadout {
        Part::Answered(l) if l.customized => LoadoutFilter::Only(l.badges.iter().cloned().collect()),
        Part::Answered(_) => LoadoutFilter::All,
        Part::Failed => LoadoutFilter::Unknown,
    }
}

/// A badge's loadout key, exactly the desktop's `${provider}:${id}`.
fn badge_key(b: &ProfileBadge) -> String {
    format!("{}:{}", b.provider.as_deref().unwrap_or(""), b.id)
}

fn provider_key(b: &ProfileBadge) -> String {
    b.provider.as_deref().unwrap_or("").to_lowercase()
}

fn pro_profile_badge(b: &BttvProBadge) -> ProfileBadge {
    ProfileBadge {
        id: BTTV_PRO_ID.to_string(),
        url: b.url.clone(),
        title: "BTTV Pro".to_string(),
        name: Some("BTTV Pro".to_string()),
        provider: Some("bttv".to_string()),
        ..Default::default()
    }
}

/// Puts the BetterTTV Pro badge after the FrankerFaceZ and BetterTTV badges of a
/// list already in the fixed provider order (the desktop's `withBttvPro`).
fn insert_bttv_pro(list: &mut Vec<ProfileBadge>, pro: ProfileBadge) {
    let at = list.iter().rposition(|b| matches!(provider_key(b).as_str(), "ffz" | "bttv")).map_or(0, |i| i + 1);
    list.insert(at, pro);
}

/// The chat-client badges a loadout lets show, with BetterTTV Pro among them when
/// it resolved. `unknown_shows` says what an unread loadout does: the groups show
/// everything, the worn row shows none (it must never show a hidden badge).
fn pick_chat_clients(raw: Vec<ProfileBadge>, pro: Option<ProfileBadge>, filter: &LoadoutFilter, unknown_shows: bool) -> Vec<ProfileBadge> {
    let allowed = |key: &str| match filter {
        LoadoutFilter::All => true,
        LoadoutFilter::Only(chosen) => chosen.contains(key),
        LoadoutFilter::Unknown => unknown_shows,
    };
    let mut list: Vec<ProfileBadge> = raw.into_iter().filter(|b| allowed(&badge_key(b))).collect();
    if let Some(pro) = pro {
        if allowed(BTTV_PRO_KEY) {
            insert_bttv_pro(&mut list, pro);
        }
    }
    list
}

/// Everyone's badge groups: Twitch, 7TV, one group per chat client in the chat's
/// own order, "Other" for a provider this list does not know, then StreamNook.
fn badge_groups(twitch: Vec<ProfileBadge>, seventv: Vec<ProfileBadge>, chat: Vec<ProfileBadge>, streamnook: Vec<ProfileBadge>) -> Vec<BadgeGroup> {
    let mut groups = Vec::new();
    let mut push = |title: &str, list: Vec<ProfileBadge>| {
        if !list.is_empty() {
            groups.push(BadgeGroup { title: title.into(), badges: list });
        }
    };
    push("Twitch", twitch);
    push("7TV", seventv);
    for (key, title) in PROVIDER_GROUPS {
        push(title, chat.iter().filter(|b| provider_key(b) == key).cloned().collect());
    }
    push("Other", chat.iter().filter(|b| !PROVIDER_GROUPS.iter().any(|(key, _)| provider_key(b) == *key)).cloned().collect());
    push("StreamNook", streamnook);
    groups
}

/// The Twitch badges a person wears: what the account displays, minus the badges
/// of one channel.
fn worn_twitch_badges(resp: &UserBadgesResponse) -> Vec<ProfileBadge> {
    twitch_badges(&UserBadgesResponse {
        display_badges: resp.display_badges.iter().filter(|x| !CHANNEL_SCOPED.contains(&x.badge_info.set_id.as_str())).cloned().collect(),
        earned_badges: Vec::new(),
        third_party_badges: Vec::new(),
    })
}

/// The 7TV badge a person has selected, tagged as 7TV so the surface order
/// places it: what chat knows first, else their inventory.
fn selected_seventv_badge(user_id: &str, inventory: Option<&UserCosmetics>) -> Option<ProfileBadge> {
    let selected_badge = resolver::peek(user_id)
        .and_then(|c| selected(&c.badges).cloned())
        .or_else(|| inventory.and_then(|i| selected(&i.badges).cloned()));
    selected_badge.as_ref().and_then(seventv_badge).map(|b| ProfileBadge { provider: Some("7tv".to_string()), ..b })
}

struct WornInputs<'a> {
    twitch: Vec<ProfileBadge>,
    seventv: Option<ProfileBadge>,
    chat: Vec<ProfileBadge>,
    pro: Option<ProfileBadge>,
    filter: &'a LoadoutFilter,
    streamnook: Option<&'a SnBadge>,
}

/// The row of what a member WEARS, one builder for the sheet and the You header:
/// Twitch, the selected 7TV badge, the chat-client badges their loadout lets show
/// (with BetterTTV Pro), their StreamNook badge last, in surface order, capped.
fn worn_row(w: WornInputs) -> Vec<ProfileBadge> {
    let chat = pick_chat_clients(w.chat, w.pro, w.filter, false);
    let rest: Vec<ProfileBadge> = w.twitch.into_iter().chain(w.seventv).chain(chat).collect();
    own_worn_badges(w.streamnook, rest)
}

// ---- The look: theme, accent, frame, caption --------------------------------------

/// "r, g, b" (as the catalog writes an accent) as #rrggbb.
fn rgb_triple_hex(triple: &str) -> Option<String> {
    let parts: Vec<u8> = triple.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    (parts.len() == 3).then(|| format!("#{:02x}{:02x}{:02x}", parts[0], parts[1], parts[2]))
}

/// A paint's representative color: the middle of the gradient colors in layer
/// order, as the desktop card picks it (`resolveProfileTheme`).
fn paint_accent(paint: &PaintView) -> Option<String> {
    let colors: Vec<&str> = paint
        .layers
        .iter()
        .filter(|l| l.kind != "image")
        .flat_map(|l| l.stops.iter().map(|s| s.color.as_str()))
        .collect();
    colors.get(colors.len() / 2).and_then(|c| c.get(..7)).map(str::to_lowercase)
}

/// How a member's profile is themed and the accent that goes with it (the
/// desktop's `themeRgb`: the theme's own, else the rank's).
fn theme_and_accent(look: &Look, paint: Option<&PaintView>, number: Option<u32>) -> (Option<String>, Option<String>) {
    let tier = number.map(|n| member_rank(n).accent.to_lowercase());
    if let Some(atmosphere) = look.atmosphere.as_ref() {
        return (Some("atmosphere".to_string()), rgb_triple_hex(&atmosphere.accent).or(tier));
    }
    if look.cologne.is_some() {
        return (Some("atmosphere".to_string()), look.cologne_accent.as_deref().and_then(rgb_triple_hex).or(tier));
    }
    if look.paint_theme {
        if let Some(paint) = paint {
            return (Some("paint".to_string()), paint_accent(paint).or(tier));
        }
    }
    (Some("tier".to_string()), tier)
}

/// The Frame a member equipped, with its art, when the equipment read answered.
fn frame_view(facts: &MemberFacts) -> Option<FrameView> {
    let slug = member_profile::equipped_slot(facts, "frame")?;
    let url = registry::badge_art(&registry::catalog_entry(&slug)?, false)?;
    Some(FrameView { url, slice: FRAME.slice, border: FRAME.border })
}

/// The name of the StreamNook badge they wear; none for the default one, whose
/// name only repeats their rank.
fn worn_caption(user_id: &str) -> Option<String> {
    registry::worn_badge_slug(user_id)
        .filter(|slug| slug != registry::DEFAULT_SLUG)
        .and_then(|slug| registry::catalog_entry(&slug))
        .map(|c| c.name)
        .filter(|n| !n.trim().is_empty())
}

/// The login a page link is built from: the one Twitch holds for the member's
/// id. The caller's login stands in only when Twitch did not answer and it is
/// shaped like a login, so a chat platform's name can never become a link.
fn page_login<'a>(helix_login: Option<&'a str>, caller_login: &'a str) -> &'a str {
    match helix_login {
        Some(login) if !login.is_empty() => login,
        _ if member_link::is_valid_login(&caller_login.to_lowercase()) => caller_login,
        _ => "",
    }
}

// ---- Member epochs and generations -------------------------------------------------

static EPOCHS: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static EPOCH_CLOCK: AtomicU64 = AtomicU64::new(0);

/// A number that moves forward whenever something a member's sheet shows
/// changed by an own write (a save, an equip, a theme, a preference) or a
/// realtime feed row. Only members whose writes were seen have one; the map is
/// bounded by the members this process has seen change.
pub fn member_epoch(member_id: &str) -> u64 {
    EPOCHS.lock().ok().and_then(|m| m.get(member_id).copied()).unwrap_or(0)
}

pub fn bump_member_epoch(member_id: &str) {
    if member_id.is_empty() {
        return;
    }
    let next = EPOCH_CLOCK.fetch_add(1, Ordering::Relaxed) + 1;
    if let Ok(mut epochs) = EPOCHS.lock() {
        epochs.insert(member_id.to_string(), next);
    }
}

/// Which state of the world a build read: the cosmetics registry's version and
/// the member's epoch.
type Generation = (u64, u64);

fn generation(member_id: &str) -> Generation {
    (registry::version(), member_epoch(member_id))
}

/// Whether a build read at least everything a caller asked at.
fn covers(build: Generation, wanted: Generation) -> bool {
    build.0 >= wanted.0 && build.1 >= wanted.1
}

fn newest(a: Generation, b: Generation) -> Generation {
    (a.0.max(b.0), a.1.max(b.1))
}

/// A finished build may write the cache only if no caller has asked, since it
/// began, for something it did not read.
fn should_write(newest_seen: Generation, build: Generation) -> bool {
    covers(build, newest_seen)
}

// ---- The sheet cache and its roast -----------------------------------------------------

/// The reads that can fail on their own, each with its state, kept with the sheet
/// so a build whose read failed takes the last answer instead of dropping it.
#[derive(Clone, Debug, Default)]
struct Soft {
    /// None for a non-member, or when their preferences never read and none are cached.
    member: Option<MemberFacts>,
    loadout: Part<IdentityLoadout>,
    bttv: Part<Option<BttvProBadge>>,
}

/// The roast line a sheet shows and the presentation of the sheet it was picked for.
#[derive(Clone, Debug, PartialEq)]
struct KeptRoast {
    open_id: Option<String>,
    line: String,
}

#[derive(Clone)]
struct SheetEntry {
    sheet: ProfileSheet,
    soft: Soft,
    roast: Option<KeptRoast>,
    /// The hours the roast is picked for; None when the hours or the line are hidden or unknown.
    roast_hours: Option<f64>,
}

static SHEETS: Mutex<Option<LruCache<String, SheetEntry>>> = Mutex::new(None);

fn sheet_key(user_id: &str, channel_id: &str) -> String {
    format!("{user_id}:{channel_id}")
}

fn rand_unit() -> f64 {
    rand::random::<f64>()
}

/// The roast line one presentation of a sheet shows. A line already kept stays
/// while the presentation is the same (or none is named) and it still fits the
/// hours; a new presentation rolls once, avoiding the kept line, and keeps the
/// new one. Hidden or unknown hours show no line and keep none.
fn next_roast(kept: &mut Option<KeptRoast>, hours: Option<f64>, open_id: Option<&str>, rand: &mut dyn FnMut() -> f64) -> Option<String> {
    let Some(hours) = hours else {
        *kept = None;
        return None;
    };
    if let Some(k) = kept.as_ref() {
        let same_open = open_id.is_none() || k.open_id.as_deref() == open_id;
        if same_open && member_profile::roast_is_eligible(&k.line, hours) {
            return Some(k.line.clone());
        }
    }
    let last = kept.as_ref().map(|k| k.line.clone());
    let line = member_profile::roast_line(hours, rand, last.as_deref())?;
    let open = open_id.map(String::from).or_else(|| kept.as_ref().and_then(|k| k.open_id.clone()));
    *kept = Some(KeptRoast { open_id: open, line: line.clone() });
    Some(line)
}

fn set_roast(sheet: &mut ProfileSheet, line: Option<String>) {
    if let Some(hours) = sheet.member_profile.as_mut().and_then(|m| m.hours.as_mut()) {
        hours.roast = line;
    }
}

/// The sheet as last built for this person in this room, without asking
/// anything, so a reopen paints at once. Its roast line is the one this open
/// shows (see `next_roast`).
pub fn cached_sheet(user_id: &str, channel_id: Option<&str>, open_id: Option<&str>) -> Option<ProfileSheet> {
    let key = sheet_key(user_id, channel_id.filter(|c| !c.is_empty()).unwrap_or(user_id));
    let mut guard = SHEETS.lock().ok()?;
    let entry = guard.as_mut()?.get_mut(&key)?;
    let mut rand = rand_unit;
    let line = next_roast(&mut entry.roast, entry.roast_hours, open_id, &mut rand);
    set_roast(&mut entry.sheet, line);
    Some(entry.sheet.clone())
}

/// A finished build, written to the cache when it is the newest asked for. The
/// line the entry already keeps stays with it while it still fits the new hours.
fn put_entry(key: &str, built: &Built) {
    let Ok(mut guard) = SHEETS.lock() else { return };
    let cache = guard.get_or_insert_with(|| LruCache::new(NonZeroUsize::new(256).unwrap()));
    let carried = cache
        .peek(key)
        .and_then(|e| e.roast.clone())
        .filter(|r| built.roast_hours.is_some_and(|h| member_profile::roast_is_eligible(&r.line, h)));
    let mut sheet = built.sheet.clone();
    set_roast(&mut sheet, carried.as_ref().map(|r| r.line.clone()));
    cache.put(
        key.to_string(),
        SheetEntry { sheet, soft: built.soft.clone(), roast: carried, roast_hours: built.roast_hours },
    );
}

/// The last roast decision for a finished build: from the cache entry when it
/// holds one, else from the build alone.
fn settle_roast(key: &str, open_id: Option<&str>, mut built: Built) -> ProfileSheet {
    let mut rand = rand_unit;
    let line = {
        let mut guard = SHEETS.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_mut().and_then(|c| c.get_mut(key)) {
            Some(entry) => {
                let line = next_roast(&mut entry.roast, entry.roast_hours, open_id, &mut rand);
                set_roast(&mut entry.sheet, line.clone());
                line
            }
            None => next_roast(&mut None, built.roast_hours, open_id, &mut rand),
        }
    };
    set_roast(&mut built.sheet, line);
    built.sheet
}

// ---- The build ---------------------------------------------------------------------------

/// What one sheet build asks for.
#[derive(Clone)]
struct SheetRequest {
    user_id: String,
    login: String,
    display_name: String,
    color: Option<String>,
    channel_id: String,
    channel_login: String,
    in_room: bool,
    tz_offset_min: Option<i32>,
}

/// What a build read that is not kept for a later build to fall back on.
struct Fetched {
    user_id: String,
    login: String,
    display_name: String,
    in_room: bool,
    badges: Option<UserBadgesResponse>,
    /// The 7TV group: every badge they own, deduped.
    seventv_badges: Vec<ProfileBadge>,
    /// The selected 7TV badge, tagged `7tv`.
    seventv_selected: Option<ProfileBadge>,
    seventv_user_id: Option<String>,
    paint_def: Option<Value>,
    paint: Option<PaintView>,
    helix: Option<UserInfo>,
    picture: Option<AvatarPick>,
    name_color: String,
    look: Look,
}

#[derive(Clone)]
struct Built {
    sheet: ProfileSheet,
    soft: Soft,
    roast_hours: Option<f64>,
}

/// The longest the sheet waits on the two slow soft reads (the loadout and BTTV
/// Pro) together: they run side by side, and each read goes on in the background
/// and fills its cache when it ends.
const SOFT_BUDGET: Duration = Duration::from_millis(1500);

/// A member's loadout, within the soft budget; `Failed` for a non-member, a read
/// that failed, or one still running.
async fn loadout_part(member_id: &str) -> Part<IdentityLoadout> {
    // A cold launch may ask before the member list has loaded.
    registry::ready(Duration::from_secs(3)).await;
    if !registry::is_member(member_id) {
        return Part::Failed;
    }
    match crate::commands::identity::identity_loadout_within(member_id, SOFT_BUDGET).await {
        Ok(Some(loadout)) => Part::Answered(loadout),
        Ok(None) => Part::Failed,
        Err(e) => {
            log::debug!("[ProfileView] loadout unavailable: {e}");
            Part::Failed
        }
    }
}

/// Whether anyone has BetterTTV Pro, within the soft budget; `Failed` while the
/// socket has not answered (a non-Pro person is only known after its silence).
async fn bttv_part(user_id: &str) -> Part<Option<BttvProBadge>> {
    match bttv_pro_service::pro_badge_within(user_id, SOFT_BUDGET).await {
        Some(answer) => Part::Answered(answer),
        None => Part::Failed,
    }
}

/// A fresh build's soft parts with the cached sheet's answer for any that failed.
fn merge_soft(mut new: Soft, cached: Option<&Soft>) -> Soft {
    let Some(old) = cached else { return new };
    new.loadout.or_cached(&old.loadout);
    new.bttv.or_cached(&old.bttv);
    new.member = match (new.member, old.member.as_ref()) {
        (Some(fresh), Some(old_facts)) => Some(member_profile::merge_facts(fresh, old_facts)),
        (member, _) => member,
    };
    new
}

/// Reads everything one sheet needs and builds it. Every source fails soft; a
/// part that failed takes the cached sheet's last answer.
async fn fetch_and_build(req: SheetRequest, key: &str) -> Built {
    let user_id = req.user_id.as_str();
    let (badges, (inventory, full_inventory), helix, pictures, look, read, loadout, bttv) = tokio::join!(
        crate::commands::badge_service::get_user_badges_with_earned_unified(
            req.user_id.clone(),
            req.login.clone(),
            req.channel_id.clone(),
            req.channel_login.clone(),
        ),
        async {
            match resolver::inventory(user_id).await {
                Some(inv) => (Some(inv), true),
                None => {
                    let worn = resolver::resolve(user_id.to_string()).await;
                    ((!worn.hard_fail).then_some(worn.data), false)
                }
            }
        },
        crate::services::twitch_service::TwitchService::get_user_by_id(user_id),
        resolver::profile_pictures(vec![user_id.to_string()]),
        worn_look(user_id),
        member_profile::member_reads(user_id, &req.login),
        loadout_part(user_id),
        bttv_part(user_id),
    );
    let badges = badges.map_err(|e| log::debug!("[ProfileView] badges unavailable: {e}")).ok();
    // Only the full inventory counts what they own; the worn-only fallback does not.
    let seventv_counts = match (&inventory, full_inventory) {
        (Some(inv), true) => Part::Answered((inv.paints.len() as u64, inv.badges.len() as u64)),
        _ => Part::Failed,
    };
    let cached: Option<Soft> = SHEETS.lock().ok().and_then(|mut g| g.as_mut().and_then(|c| c.get(key).map(|e| e.soft.clone())));
    let member = match read {
        MemberRead::Read(mut facts) => {
            facts.seventv = seventv_counts;
            Some(facts)
        }
        MemberRead::NotMember => None,
        MemberRead::Unknown => cached.as_ref().and_then(|c| c.member.clone()),
    };
    let soft = merge_soft(Soft { member, loadout, bttv }, cached.as_ref());
    let (paint_def, paint) = worn_paint(user_id, inventory.as_ref());
    let fetched = Fetched {
        user_id: req.user_id.clone(),
        login: req.login.clone(),
        display_name: req.display_name.clone(),
        in_room: req.in_room,
        badges,
        seventv_badges: inventory
            .as_ref()
            .map(|i| {
                let mut seen = HashSet::new();
                i.badges.iter().filter_map(seventv_badge).filter(|b| seen.insert(b.id.clone())).collect()
            })
            .unwrap_or_default(),
        seventv_selected: selected_seventv_badge(user_id, inventory.as_ref()),
        seventv_user_id: inventory.as_ref().and_then(|i| i.seventv_user_id.clone()),
        paint_def,
        paint,
        helix: helix.ok(),
        picture: pictures.get(user_id).cloned().flatten().and_then(|p| pick_avatar(&p, 56.0, 3.0, true)),
        // A sheet opened from a mention carries no color; chat has usually seen it.
        name_color: crate::services::chat_row_view::readable_color(Some(
            req.color
                .clone()
                .or_else(|| crate::services::chat_row_view::seen_color(&req.login))
                .as_deref()
                .unwrap_or("#9147FF"),
        )),
        look,
    };
    let clock = Clock::system(req.tz_offset_min);
    let (sheet, roast_hours) = assemble(&fetched, &soft, &clock, &member_profile::CATALOG);
    Built { sheet, soft, roast_hours }
}

/// Works what a build read into the sheet. Pure: it reads only what it is given
/// and the registries (so the tests set those), never the network or the clock.
/// The second value is the hours the roast is picked for.
fn assemble(f: &Fetched, soft: &Soft, clock: &Clock, catalog: &member_profile::Catalog) -> (ProfileSheet, Option<f64>) {
    let user_id = f.user_id.as_str();
    let number = registry::member_number(user_id);
    let is_member = number.is_some();
    let sn_badge = registry::member_badge(user_id, false);
    let shaped = soft.member.as_ref().map(|facts| {
        member_profile::shape(
            facts,
            &ShapeCtx {
                display_name: &f.display_name,
                login: &f.login,
                broadcaster_type: f.helix.as_ref().and_then(|u| u.broadcaster_type.as_deref()).unwrap_or(""),
                clock,
                catalog,
            },
        )
    });
    // A member who hides their badge collection hides every group, as their
    // desktop card does; StreamNook comes last. Relics ride `member_profile`.
    let hide_badges = shaped.as_ref().is_some_and(|s| s.hide_badges);
    let filter = loadout_filter(is_member, &soft.loadout);
    let pro = soft.bttv.as_answer().and_then(|p| p.as_ref()).map(pro_profile_badge);
    let raw_chat = f.badges.as_ref().map(third_party_badges).unwrap_or_default();
    let groups = if hide_badges {
        Vec::new()
    } else {
        // StreamNook last. Without the member's profile reads (their prefs failed to
        // read, or the member list was still loading) the badge they wear still shows.
        let streamnook = if shaped.is_some() {
            member_profile::streamnook_badges(sn_badge.as_ref(), &registry::owned_slugs(user_id), &registry::catalog())
        } else {
            member_profile::streamnook_badges(sn_badge.as_ref(), &Default::default(), &[])
        };
        badge_groups(
            f.badges.as_ref().map(|b| sheet_twitch_badges(b, f.in_room)).unwrap_or_default(),
            f.seventv_badges.clone(),
            pick_chat_clients(raw_chat.clone(), pro.clone(), &filter, true),
            streamnook,
        )
    };
    let worn = if is_member {
        worn_row(WornInputs {
            twitch: f.badges.as_ref().map(worn_twitch_badges).unwrap_or_default(),
            seventv: f.seventv_selected.clone(),
            chat: raw_chat,
            pro,
            filter: &filter,
            streamnook: sn_badge.as_ref(),
        })
    } else {
        Vec::new()
    };
    let (theme, accent) = if is_member { theme_and_accent(&f.look, f.paint.as_ref(), number) } else { (None, None) };
    let sheet = ProfileSheet {
        user_id: f.user_id.clone(),
        login: f.login.clone(),
        display_name: f.display_name.clone(),
        shown_login: shown_login(&f.login, &f.display_name),
        avatar_url: f.helix.as_ref().and_then(|u| u.profile_image_url.clone()).filter(|u| !u.is_empty()),
        picture: f.picture.clone(),
        name_color: f.name_color.clone(),
        paint: f.paint.clone(),
        paint_def: f.paint_def.clone(),
        groups,
        streamnook: sn_badge.clone(),
        member: number.map(member_rank),
        atmosphere: f.look.atmosphere.clone(),
        cologne: f.look.cologne,
        member_profile: shaped.as_ref().map(|s| s.profile.clone()),
        page_link: member_link::page_link(user_id, page_login(f.helix.as_ref().map(|u| u.login.as_str()), &f.login)),
        worn,
        seventv_user_id: f.seventv_user_id.clone(),
        theme,
        frame: soft.member.as_ref().and_then(frame_view),
        accent,
        caption: if is_member { worn_caption(user_id) } else { None },
    };
    (sheet, shaped.and_then(|s| s.roast_hours))
}

// ---- One build at a time per sheet ------------------------------------------------------

struct Running {
    id: u64,
    born_at: Generation,
    done: watch::Receiver<Option<Built>>,
}

#[derive(Default)]
struct KeyState {
    running: Option<Running>,
    /// The newest generation any caller has asked at.
    newest: Generation,
    waiters: usize,
}

static FLIGHTS: LazyLock<Mutex<HashMap<String, KeyState>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static FLIGHT_IDS: AtomicU64 = AtomicU64::new(0);

fn flights() -> std::sync::MutexGuard<'static, HashMap<String, KeyState>> {
    FLIGHTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A key's state goes when nothing runs for it and nobody waits.
fn tidy(f: &mut HashMap<String, KeyState>, key: &str) {
    if f.get(key).is_some_and(|s| s.running.is_none() && s.waiters == 0) {
        f.remove(key);
    }
}

struct Waiter {
    key: String,
}

impl Drop for Waiter {
    fn drop(&mut self) {
        let mut f = flights();
        if let Some(s) = f.get_mut(&self.key) {
            s.waiters = s.waiters.saturating_sub(1);
        }
        tidy(&mut f, &self.key);
    }
}

/// Clears a build's claim however its task ends (a panic included).
struct Landing {
    key: String,
    id: u64,
}

impl Drop for Landing {
    fn drop(&mut self) {
        let mut f = flights();
        if let Some(s) = f.get_mut(&self.key) {
            if s.running.as_ref().is_some_and(|r| r.id == self.id) {
                s.running = None;
            }
        }
        tidy(&mut f, &self.key);
    }
}

fn spawn_build(key: String, req: SheetRequest, born_at: Generation, id: u64, tx: watch::Sender<Option<Built>>) {
    tauri::async_runtime::spawn(async move {
        let _landing = Landing { key: key.clone(), id };
        let built = fetch_and_build(req, &key).await;
        {
            let mut f = flights();
            let current = f.get(&key).map_or(true, |s| should_write(s.newest, born_at));
            if current {
                put_entry(&key, &built);
            }
            if let Some(s) = f.get_mut(&key) {
                if s.running.as_ref().is_some_and(|r| r.id == id) {
                    s.running = None;
                }
            }
            tidy(&mut f, &key);
        }
        let _ = tx.send(Some(built));
    });
}

async fn wait_built(mut rx: watch::Receiver<Option<Built>>) -> Option<Built> {
    loop {
        if let Some(built) = rx.borrow_and_update().clone() {
            return Some(built);
        }
        if rx.changed().await.is_err() {
            return rx.borrow().clone();
        }
    }
}

/// The one build for a sheet key. A caller joins the build already running when
/// that build read at least what the caller asked at; otherwise it waits for it
/// to end and asks again, so exactly one more pass runs. The build runs in its
/// own task, so it finishes into the cache even if every caller left.
async fn single_flight(key: &str, req: SheetRequest) -> Built {
    flights().entry(key.to_string()).or_default().waiters += 1;
    let _waiter = Waiter { key: key.to_string() };
    loop {
        let born_at = generation(&req.user_id);
        let (rx, join) = {
            let mut f = flights();
            let state = f.entry(key.to_string()).or_default();
            state.newest = newest(state.newest, born_at);
            match state.running.as_ref() {
                Some(r) => (r.done.clone(), covers(r.born_at, born_at)),
                None => {
                    let (tx, rx) = watch::channel(None);
                    let id = FLIGHT_IDS.fetch_add(1, Ordering::Relaxed) + 1;
                    state.running = Some(Running { id, born_at, done: rx.clone() });
                    spawn_build(key.to_string(), req.clone(), born_at, id, tx);
                    (rx, true)
                }
            }
        };
        match wait_built(rx).await {
            Some(built) if join => return built,
            // An older build ended: ask again.
            Some(_) => continue,
            // The build's task died without an answer: build here, once.
            None if join => return fetch_and_build(req.clone(), key).await,
            None => continue,
        }
    }
}

/// Everything the sheet shows for one person in one room. The badge lookup is in
/// the room's context (channel badges resolve there); without a room it falls
/// back to the person's own channel. Every source fails soft. `tz_offset_min` is
/// the viewer's offset from UTC in minutes, east positive; `open_id` names this
/// presentation of the sheet (the roast line changes once per presentation).
#[allow(clippy::too_many_arguments)]
pub async fn profile_sheet(
    user_id: &str,
    login: &str,
    display_name: &str,
    color: Option<&str>,
    channel_id: Option<&str>,
    channel_login: Option<&str>,
    tz_offset_min: Option<i32>,
    open_id: Option<&str>,
) -> ProfileSheet {
    // A login alone (a mention, the long-press Profile item) resolves to its
    // account first: every lookup below is keyed by the user id.
    let looked_up = if user_id.is_empty() && !login.is_empty() {
        crate::services::twitch_service::TwitchService::get_user_by_login(login)
            .await
            .map_err(|e| log::debug!("[ProfileView] no account for {login}: {e}"))
            .ok()
    } else {
        None
    };
    let user_id = looked_up.as_ref().map(|u| u.id.as_str()).unwrap_or(user_id);
    let display_name = looked_up
        .as_ref()
        .map(|u| u.display_name.as_str())
        .filter(|_| display_name.is_empty() || display_name.eq_ignore_ascii_case(login))
        .unwrap_or(display_name);
    let in_room = channel_id.is_some_and(|c| !c.is_empty());
    let channel_id = channel_id.filter(|c| !c.is_empty()).unwrap_or(user_id).to_string();
    let channel_login = channel_login.filter(|c| !c.is_empty()).unwrap_or(login).to_string();
    // The key is computed after the login lookup, so a login and an id share one build.
    let key = sheet_key(user_id, &channel_id);
    let built = single_flight(
        &key,
        SheetRequest {
            user_id: user_id.to_string(),
            login: login.to_string(),
            display_name: display_name.to_string(),
            color: color.map(String::from),
            channel_id,
            channel_login,
            in_room,
            tz_offset_min,
        },
    )
    .await;
    settle_roast(&key, open_id, built)
}

// ---- The viewer's own identity header ----------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OwnIdentity {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shown_login: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<PaintView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint_def: Option<Value>,
    /// The account's Twitch name color, made readable as chat shows it; None
    /// when Twitch did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_color: Option<String>,
    /// The 7TV picture drawn over the platform one, when the viewer shows 7TV pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<AvatarPick>,
    /// The Twitch profile banner; None when the account has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub banner_url: Option<String>,
    /// What is WORN, in surface order (Twitch, 7TV, the other chat apps, the
    /// StreamNook badge last), images only, deduped, capped.
    pub badges: Vec<ProfileBadge>,
    /// Present only for a StreamNook member; drawn after the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub streamnook: Option<SnBadge>,
    /// The Atmosphere they wear on their profile, after the wear gate. It shows
    /// here whether or not they keep it out of chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atmosphere: Option<registry::Atmosphere>,
    /// The Cologne event look instead of an Atmosphere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cologne: Option<member_theme::Cologne>,
    /// Their StreamNook page, so the You card's first frame has it. Absent for a
    /// non-member.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_link: Option<PageLink>,
}

/// Twitch badge sets that belong to one channel rather than to the person.
const CHANNEL_SCOPED: &[&str] = &[
    "broadcaster",
    "moderator",
    "lead_moderator",
    "vip",
    "subscriber",
    "founder",
    "bits",
    "bits-leader",
    "sub-gifter",
    "sub-gift-leader",
    "artist-badge",
    "predictions",
    "hype-train",
    "clip-champ",
];

/// How many badges the You card's row draws (a 127-badge wall once buried the
/// You tab). Decided here, so the phones never cut the row themselves and drop
/// the StreamNook badge at its end.
const HEADER_BADGE_CAP: usize = 6;
const OWN_FILE: &str = "own_identity_header.json";

fn own_path() -> Option<std::path::PathBuf> {
    crate::services::cache_service::get_app_data_dir().ok().map(|d| d.join(OWN_FILE))
}

/// The header as last built, from disk, so the You tab paints on its first
/// frame. None for another account or before the first build.
pub fn cached_own_identity(user_id: &str) -> Option<OwnIdentity> {
    let bytes = std::fs::read(own_path()?).ok()?;
    let header: OwnIdentity = serde_json::from_slice(&bytes).ok()?;
    (header.user_id == user_id).then_some(header)
}

/// The header's last inputs, so a saved loadout can rebuild the file without a
/// caller.
static LAST_OWN: Mutex<Option<(String, String, String)>> = Mutex::new(None);
/// One writer of the header file at a time, so the stamp check and the write
/// cannot interleave with a newer build's.
static OWN_WRITE: Mutex<()> = Mutex::new(());

/// Whether a built header may replace the saved one: the member list had loaded
/// (a header built before it would drop the member badge and the page link) and,
/// for a member, the loadout was read (a header without it would drop their chat
/// badges). An unknown part is never saved over a known one.
fn should_persist_own(members_loaded: bool, is_member: bool, loadout: &Part<IdentityLoadout>) -> bool {
    members_loaded && (!is_member || loadout.is_answered())
}

/// Writes the header file unless a newer loadout save landed after this build
/// read the loadout (that save rebuilds the file itself).
fn save_own_header(user_id: &str, loadout_stamp: u64, header: &OwnIdentity) {
    let _write = OWN_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    if crate::commands::identity::loadout_stamp(user_id) != loadout_stamp {
        return;
    }
    if let (Some(path), Ok(body)) = (own_path(), serde_json::to_vec(header)) {
        let _ = std::fs::write(path, body);
    }
}

/// Builds the header again from the last inputs `own_identity` saw, for the
/// account a save was made for; never deletes the file. Nothing when that
/// account's header was never built here.
pub async fn rebuild_own_header(member_id: &str) {
    let last = LAST_OWN.lock().ok().and_then(|g| g.clone());
    if let Some((id, login, display_name)) = last {
        if id == member_id {
            let _ = own_identity(&id, &login, &display_name).await;
        }
    }
}

/// The viewer's own identity header, built fresh (and saved for the next cold
/// launch). What they wear comes from the same builder the sheet uses.
pub async fn own_identity(user_id: &str, login: &str, display_name: &str) -> OwnIdentity {
    if let Ok(mut last) = LAST_OWN.lock() {
        *last = Some((user_id.to_string(), login.to_string(), display_name.to_string()));
    }
    let (badges, inventory, pictures, look, banner_url, colors, loadout, bttv) = tokio::join!(
        crate::commands::badge_service::get_user_badges_with_earned_unified(
            user_id.to_string(),
            login.to_string(),
            user_id.to_string(),
            login.to_string(),
        ),
        resolver::inventory(user_id),
        resolver::profile_pictures(vec![user_id.to_string()]),
        worn_look(user_id),
        twitch_banner(user_id),
        crate::services::twitch_service::TwitchService::get_user_chat_colors(vec![user_id.to_string()]),
        loadout_part(user_id),
        bttv_part(user_id),
    );
    // The stamp of the loadout this build read, checked again before the file is written.
    let loadout_stamp = crate::commands::identity::loadout_stamp(user_id);
    let badges = badges.ok();
    // The card's name color before Edit profile has read it: what chat shows.
    let name_color = colors
        .get(user_id)
        .filter(|c| !c.is_empty())
        .cloned()
        .or_else(|| crate::services::chat_row_view::seen_color(login))
        .map(|c| crate::services::chat_row_view::readable_color(Some(&c)));
    let is_member = registry::is_member(user_id);
    let filter = loadout_filter(is_member, &loadout);
    let streamnook = registry::member_badge(user_id, false);
    let worn = worn_row(WornInputs {
        twitch: badges.as_ref().map(worn_twitch_badges).unwrap_or_default(),
        seventv: selected_seventv_badge(user_id, inventory.as_ref()),
        chat: badges.as_ref().map(third_party_badges).unwrap_or_default(),
        pro: bttv.as_answer().and_then(|p| p.as_ref()).map(pro_profile_badge),
        filter: &filter,
        streamnook: streamnook.as_ref(),
    });
    let (paint_def, paint) = worn_paint(user_id, inventory.as_ref());
    let header = OwnIdentity {
        user_id: user_id.to_string(),
        login: login.to_string(),
        display_name: display_name.to_string(),
        shown_login: shown_login(login, display_name),
        paint,
        paint_def,
        name_color,
        picture: pictures.get(user_id).cloned().flatten().and_then(|p| pick_avatar(&p, 56.0, 3.0, true)),
        banner_url,
        badges: worn,
        streamnook,
        atmosphere: look.atmosphere,
        cologne: look.cologne,
        page_link: member_link::page_link(user_id, login),
    };
    if should_persist_own(registry::members_loaded(), is_member, &loadout) {
        save_own_header(user_id, loadout_stamp, &header);
    }
    header
}

/// The chat-client badges (FFZ, BetterTTV, Chatterino and the rest) someone wears:
/// their StreamNook Identity loadout, else the raw chat-client set.
fn chat_clients_worn(
    resolved: Option<crate::commands::identity::ResolvedIdentity>,
    badges: Option<&UserBadgesResponse>,
) -> Vec<ProfileBadge> {
    let loadout: Vec<ProfileBadge> = resolved
        .map(|r| {
            r.badges
                .into_iter()
                .filter(|b| !b.image_url.is_empty())
                .map(|b| ProfileBadge {
                    id: b.key,
                    url: b.image_url,
                    title: b.title.clone(),
                    name: Some(b.title),
                    provider: Some(b.provider),
                    ..Default::default()
                })
                .collect()
        })
        .unwrap_or_default();
    if loadout.is_empty() { badges.map(third_party_badges).unwrap_or_default() } else { loadout }
}

/// The header's badge row: what Twitch, 7TV and the other apps show plus your
/// StreamNook badge, deduped, in the one surface order every badge row uses
/// (`badge_service::surface_rank`: StreamNook last). The cap never squeezes the
/// StreamNook badge out, and one the other apps also list is not drawn twice.
fn own_worn_badges(streamnook: Option<&SnBadge>, rest: Vec<ProfileBadge>) -> Vec<ProfileBadge> {
    let own = streamnook.map(|b| ProfileBadge {
        id: b.slug.clone().unwrap_or_else(|| registry::DEFAULT_SLUG.to_string()),
        url: b.image_url.clone(),
        title: b.label.clone(),
        name: Some(b.label.clone()),
        provider: Some("streamnook".to_string()),
        ..Default::default()
    });
    let mut seen = HashSet::new();
    let mut row: Vec<ProfileBadge> = rest
        .into_iter()
        .filter(|b| b.provider.as_deref() != Some("streamnook"))
        .filter(|b| !b.url.is_empty() && seen.insert(b.id.clone()))
        .collect();
    let rank = |b: &ProfileBadge| crate::services::badge_service::surface_rank(b.provider.as_deref().unwrap_or(""));
    row.sort_by_key(rank);
    row.truncate(HEADER_BADGE_CAP - usize::from(own.is_some()));
    row.extend(own);
    row.sort_by_key(rank);
    row
}

// ---- The Cosmetics screen --------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PickerBadge {
    pub slug: String,
    pub name: String,
    /// None draws the placeholder square.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_url: Option<String>,
    pub worn: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PickerAtmosphere {
    /// `tier` for the Default tile.
    pub id: String,
    pub name: String,
    /// Empty for the Default tile (it draws the plain surface).
    pub swatch: Fill,
    pub allowed: bool,
    pub worn: bool,
}

/// The StreamNook half of the Cosmetics screen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StreamNookPicker {
    pub badges: Vec<PickerBadge>,
    /// The Default tile first, then each Atmosphere the account may see.
    pub atmospheres: Vec<PickerAtmosphere>,
    pub theme: String,
    /// Whether an Atmosphere (or the Cologne look) is worn, which is when the
    /// "Show in my chat messages" row shows.
    pub show_chat_toggle: bool,
    pub chat_atmosphere: bool,
}

/// What an account owns and wears of StreamNook's own cosmetics, with every
/// gate decided: accolade Atmospheres hidden until earned and then allowed,
/// the others allowed when owned or subscribed. Each read fails soft.
pub async fn streamnook_picker(user_id: &str) -> StreamNookPicker {
    registry::ready(std::time::Duration::from_secs(5)).await;
    let (prefs, subscribed, accolades) =
        tokio::join!(member_theme::profile_prefs(user_id), stripe_subscriber(user_id), accolades(user_id));
    let accolades: HashSet<String> = accolades.unwrap_or_default().into_iter().collect();
    let owned = registry::owned_slugs(user_id);
    let worn_badge = registry::worn_badge_slug(user_id);
    let badges = registry::catalog()
        .into_iter()
        .filter(|c| (c.kind.is_empty() || c.kind == "badge") && owned.contains(&c.slug))
        .map(|c| PickerBadge {
            image_url: registry::badge_art(&c, false),
            worn: worn_badge.as_deref() == Some(c.slug.as_str()),
            name: c.name,
            slug: c.slug,
        })
        .collect();
    let theme = prefs.profile_theme.clone();
    let mut atmospheres = vec![PickerAtmosphere {
        id: "tier".into(),
        name: "Default".into(),
        swatch: Fill::default(),
        allowed: true,
        worn: theme == "tier",
    }];
    for atm in registry::atmospheres() {
        let allowed = match &atm.unlock {
            registry::Unlock::Accolade { accolade_id } => {
                if !accolades.contains(accolade_id) {
                    continue;
                }
                true
            }
            registry::Unlock::Subscriber => owned.contains(&atm.id) || subscribed,
        };
        let worn = theme == atm.id || theme.starts_with(&format!("{}+", atm.id));
        atmospheres.push(PickerAtmosphere { swatch: atm.look.swatch.clone(), id: atm.id, name: atm.name, allowed, worn });
    }
    StreamNookPicker {
        badges,
        atmospheres,
        show_chat_toggle: registry::atmosphere(&theme).is_some(),
        chat_atmosphere: prefs.chat_atmosphere,
        theme,
    }
}

/// Whether the member's latest Stripe subscription is live (active, past due or
/// trialing). A failed read counts as not subscribed.
pub async fn stripe_subscriber(user_id: &str) -> bool {
    if user_id.is_empty() {
        return false;
    }
    let query = format!(
        "select=status&twitch_user_id=eq.{}&order=updated_at.desc&limit=1",
        user_id.chars().filter(char::is_ascii_digit).collect::<String>()
    );
    match crate::services::streamnook_backend::select("stripe_subscriptions", &query).await {
        Ok(rows) => matches!(
            rows.first().and_then(|r| r.get("status")).and_then(Value::as_str),
            Some("active" | "past_due" | "trialing")
        ),
        Err(e) => {
            log::warn!("[ProfileView] subscriber status read failed: {e}");
            false
        }
    }
}

/// The accolades a member has earned. A failure is an error, never an empty
/// set, so lit medallions do not blank on a blip.
pub async fn accolades(user_id: &str) -> Result<Vec<String>, String> {
    if user_id.is_empty() {
        return Ok(Vec::new());
    }
    let query = format!("select=accolade_id&twitch_user_id=eq.{}", user_id.chars().filter(char::is_ascii_digit).collect::<String>());
    let rows = crate::services::streamnook_backend::select("user_accolades", &query).await?;
    Ok(rows.iter().filter_map(|r| r.get("accolade_id").and_then(Value::as_str).map(String::from)).collect())
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PickerPaint {
    pub id: String,
    pub name: String,
    pub paint: PaintView,
    /// The paint as 7TV defines it, for renderers that paint with CSS.
    pub def: Value,
    pub worn: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PickerSevenTvBadge {
    pub badge: ProfileBadge,
    pub worn: bool,
}

/// The 7TV and Twitch half of the Cosmetics screen.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ExternalPicker {
    pub seventv_signed_in: bool,
    /// Whose 7TV account a paint or badge change is written to: the signed-in
    /// token's account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seventv_user_id: Option<String>,
    pub paints: Vec<PickerPaint>,
    pub seventv_badges: Vec<PickerSevenTvBadge>,
    pub drops_signed_in: bool,
    /// Twitch's global badges with the one worn marked, deduped; empty without
    /// the Drops sign-in or when Twitch declined.
    pub twitch_selectable: Vec<ChatIdentityBadge>,
    /// Every displayed and earned Twitch badge, for the read-only list.
    pub twitch_earned: Vec<ProfileBadge>,
    /// The chat-client badges worn (FFZ, BetterTTV, Chatterino and the rest), in
    /// their shared order, for the chat-row preview.
    #[serde(default)]
    pub chat_clients: Vec<ProfileBadge>,
    /// The account's Twitch name colour (`#rrggbb`), when Twitch says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_color: Option<String>,
    /// Twitch's named colours, which every account may pick (any hex needs Prime or Turbo).
    pub named_colors: Vec<NamedColor>,
}

/// One of Twitch's named chat colours: what to call it, the value Helix takes, its hex.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NamedColor {
    pub label: String,
    pub value: String,
    pub hex: String,
}

/// Twitch's own list, in the order its colour picker shows.
const NAMED_CHAT_COLORS: &[(&str, &str, &str)] = &[
    ("Blue", "blue", "#0000ff"),
    ("Blue Violet", "blue_violet", "#8a2be2"),
    ("Cadet Blue", "cadet_blue", "#5f9ea0"),
    ("Chocolate", "chocolate", "#d2691e"),
    ("Coral", "coral", "#ff7f50"),
    ("Dodger Blue", "dodger_blue", "#1e90ff"),
    ("Firebrick", "firebrick", "#b22222"),
    ("Golden Rod", "golden_rod", "#daa520"),
    ("Green", "green", "#008000"),
    ("Hot Pink", "hot_pink", "#ff69b4"),
    ("Orange Red", "orange_red", "#ff4500"),
    ("Red", "red", "#ff0000"),
    ("Sea Green", "sea_green", "#2e8b57"),
    ("Spring Green", "spring_green", "#00ff7f"),
    ("Yellow Green", "yellow_green", "#9acd32"),
];

pub fn named_chat_colors() -> Vec<NamedColor> {
    NAMED_CHAT_COLORS
        .iter()
        .map(|(label, value, hex)| NamedColor { label: label.to_string(), value: value.to_string(), hex: hex.to_string() })
        .collect()
}

pub async fn external_picker(user_id: &str, login: &str) -> ExternalPicker {
    let status = crate::commands::seventv_cosmetics::get_seventv_auth_status().await;
    let (inventory, badges, drops, colors, resolved) = tokio::join!(
        resolver::inventory(user_id),
        crate::commands::badge_service::get_user_badges_with_earned_unified(
            user_id.to_string(),
            login.to_string(),
            user_id.to_string(),
            login.to_string(),
        ),
        drops_signed_in(),
        crate::services::twitch_service::TwitchService::get_user_chat_colors(vec![user_id.to_string()]),
        crate::commands::identity::get_streamnook_identity_resolved(user_id.to_string()),
    );
    let twitch_selectable = if drops && !login.is_empty() {
        selectable_twitch_badges(login).await
    } else {
        Vec::new()
    };
    let inventory = inventory.unwrap_or_default();
    let paints = inventory
        .paints
        .iter()
        .filter_map(|p| {
            let view = crate::services::chat_row_view::paint_view(p)?;
            Some(PickerPaint {
                id: view.id.clone(),
                name: p.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                worn: p.get("selected").and_then(Value::as_bool).unwrap_or(false),
                paint: view,
                def: p.clone(),
            })
        })
        .collect();
    let mut seen = HashSet::new();
    let seventv_badges = inventory
        .badges
        .iter()
        .filter_map(|b| {
            let badge = seventv_badge(b)?;
            seen.insert(badge.id.clone()).then(|| PickerSevenTvBadge {
                worn: b.get("selected").and_then(Value::as_bool).unwrap_or(false),
                badge,
            })
        })
        .collect();
    ExternalPicker {
        seventv_signed_in: status.is_authenticated,
        seventv_user_id: status.user_id.clone().or(inventory.seventv_user_id.clone()),
        paints,
        seventv_badges,
        drops_signed_in: drops,
        twitch_selectable,
        twitch_earned: badges.as_ref().map(twitch_badges).unwrap_or_default(),
        chat_clients: chat_clients_worn(resolved.ok(), badges.as_ref().ok()),
        chat_color: colors.get(user_id).filter(|c| !c.is_empty()).map(|c| c.to_lowercase()),
        named_colors: named_chat_colors(),
    }
}

async fn drops_signed_in() -> bool {
    crate::services::drops_auth_service::DropsAuthService::is_authenticated().await
}

#[cfg(mobile)]
async fn selectable_twitch_badges(login: &str) -> Vec<ChatIdentityBadge> {
    crate::commands::chat_identity::fetch_chat_identity_badges_gql(login.to_string()).await.unwrap_or_default()
}

#[cfg(not(mobile))]
async fn selectable_twitch_badges(_login: &str) -> Vec<ChatIdentityBadge> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat_layout::{PaintLayerView, PaintStop};
    use crate::services::badge_service::{BadgeInfo, BadgeProvider, UserBadge};
    use crate::services::cosmetics_registry::{entry_for_test, set_for_test, TEST_LOCK};
    use crate::services::member_profile::{AccoladeWall, Zone};
    use chrono::{FixedOffset, TimeZone, Utc};
    use serde_json::json;

    fn badge(id: &str, set: &str, x4: &str) -> UserBadge {
        UserBadge {
            badge_info: BadgeInfo {
                id: id.into(),
                set_id: set.into(),
                version: "1".into(),
                title: format!("{set} title"),
                description: String::new(),
                image_1x: format!("{x4}-1x"),
                image_2x: format!("{x4}-2x"),
                image_4x: x4.into(),
                click_action: None,
                click_url: None,
            },
            provider: BadgeProvider::Twitch,
        }
    }

    #[test]
    fn twitch_badges_merge_display_and_earned_once_each() {
        let resp = UserBadgesResponse {
            display_badges: vec![badge("a", "premium", "A4")],
            earned_badges: vec![badge("a", "premium", "A4"), badge("b", "glhf-pledge", "B4")],
            third_party_badges: vec![],
        };
        let out = twitch_badges(&resp);
        assert_eq!(out.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        assert_eq!(out[0].url, "A4");
        assert_eq!(out[0].fallbacks, vec!["A4-2x".to_string(), "A4-1x".to_string()]);
        assert_eq!(out[0].provider, None);
    }

    #[test]
    fn seventv_badges_pick_the_animated_webp_at_4x_and_fall_back_to_the_cdn() {
        let def = json!({
            "id": "bid", "name": "Sub 1y", "description": "Subscriber 1 year",
            "images": [
                { "url": "https://cdn.7tv.app/badge/x/4x_static.webp", "mime": "image/webp", "scale": 4 },
                { "url": "https://cdn.7tv.app/badge/x/4x.avif", "mime": "image/avif", "scale": 4 },
                { "url": "https://cdn.7tv.app/badge/x/4x.webp", "mime": "image/webp", "scale": 4 },
                { "url": "https://cdn.7tv.app/badge/x/1x.webp", "mime": "image/webp", "scale": 1 }
            ]
        });
        let b = seventv_badge(&def).unwrap();
        assert_eq!(b.url, "https://cdn.7tv.app/badge/x/4x.webp");
        assert_eq!(b.fallbacks[0], "https://cdn.7tv.app/badge/bid/3x.webp");
        assert_eq!(b.title, "Subscriber 1 year");
        let bare = seventv_badge(&json!({ "id": "q", "name": "Q" })).unwrap();
        assert_eq!(bare.url, "https://cdn.7tv.app/badge/q/4x.webp");
        assert_eq!(bare.label(), "Q");
    }

    #[test]
    fn the_avatar_pick_covers_the_box_or_takes_the_largest() {
        let picture = ProfilePicture {
            id: "p".into(),
            still: (1..=4).map(|s| resolver::PictureImage { url: format!("s{s}"), scale: s }).collect(),
            animated: (1..=4).map(|s| resolver::PictureImage { url: format!("a{s}"), scale: s }).collect(),
        };
        // 56 pt at 3x needs 5.25, so nothing covers it: the largest.
        let big = pick_avatar(&picture, 56.0, 3.0, true).unwrap();
        assert_eq!(big.still, "s4");
        assert_eq!(big.animated.as_deref(), Some("a4"));
        let small = pick_avatar(&picture, 18.0, 3.0, false).unwrap();
        assert_eq!(small.still, "s2");
    }

    #[test]
    fn the_banner_reads_from_twitchs_answer() {
        let set = serde_json::json!({"data": {"user": {"bannerImageURL": "https://static-cdn.jtvnw.net/b-profile_banner-480.png"}}});
        assert_eq!(banner_from_answer(&set).as_deref(), Some("https://static-cdn.jtvnw.net/b-profile_banner-480.png"));
        let none = serde_json::json!({"data": {"user": {"bannerImageURL": null}}});
        assert_eq!(banner_from_answer(&none), None);
        assert_eq!(banner_from_answer(&serde_json::json!({"data": {"user": null}})), None);
    }

    #[test]
    fn the_shapes_keep_the_keys_the_iphone_decodes() {
        let sheet = ProfileSheet {
            user_id: "1".into(),
            login: "a".into(),
            display_name: "A".into(),
            shown_login: None,
            avatar_url: Some("u".into()),
            picture: Some(AvatarPick { still: "s".into(), animated: None }),
            name_color: "#ffffff".into(),
            paint: None,
            paint_def: None,
            groups: vec![BadgeGroup { title: "Twitch".into(), badges: vec![ProfileBadge { id: "b".into(), url: "x".into(), ..Default::default() }] }],
            streamnook: Some(SnBadge { slug: None, image_url: "streamnook-house:streamnook-default".into(), label: "StreamNook".into() }),
            member: Some(member_rank(7)),
            atmosphere: Some(registry::Atmosphere { id: "aurora".into(), name: "Aurora".into(), ..Default::default() }),
            cologne: None,
            member_profile: Some(MemberProfile { views: Some("12".into()), ..Default::default() }),
            page_link: member_link::link_for(7, "a", &[]),
            worn: vec![ProfileBadge { id: "w".into(), url: "x".into(), ..Default::default() }],
            seventv_user_id: Some("7tv-user".into()),
            theme: Some("tier".into()),
            frame: Some(FrameView { url: "f".into(), slice: [199, 159, 199, 159], border: [18, 14] }),
            accent: Some("#fbbf24".into()),
            caption: Some("Mobius".into()),
        };
        let v = serde_json::to_value(&sheet).unwrap();
        assert_eq!(v["member"]["number"], 7);
        assert_eq!(v["member"]["tier"], "mythic");
        assert_eq!(v["member_profile"]["views"], "12");
        assert!(v["member_profile"]["top_emotes"].is_array());
        assert!(v["member_profile"].get("stats").is_none());
        assert_eq!(v["page_link"]["href"], "https://streamnook.app/u/a");
        assert_eq!(v["worn"][0]["id"], "w");
        assert_eq!(v["seventv_user_id"], "7tv-user");
        assert_eq!((v["theme"].as_str(), v["accent"].as_str(), v["caption"].as_str()), (Some("tier"), Some("#fbbf24"), Some("Mobius")));
        assert_eq!(v["frame"], json!({ "url": "f", "slice": [199, 159, 199, 159], "border": [18, 14] }));
        assert_eq!(v["atmosphere"]["id"], "aurora");
        assert!(v["atmosphere"].get("look").is_some(), "the look rides with the Atmosphere");
        assert!(v.get("cologne").is_none());
        for key in ["user_id", "display_name", "avatar_url", "picture", "name_color", "groups", "streamnook"] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert_eq!(v["streamnook"]["image_url"], "streamnook-house:streamnook-default");
        assert_eq!(v["groups"][0]["badges"][0]["url"], "x");
        let picker = StreamNookPicker {
            badges: vec![PickerBadge { slug: "s".into(), name: "S".into(), image_url: None, worn: true }],
            atmospheres: vec![],
            theme: "tier".into(),
            show_chat_toggle: false,
            chat_atmosphere: true,
        };
        let v = serde_json::to_value(&picker).unwrap();
        assert!(v.get("show_chat_toggle").is_some() && v.get("chat_atmosphere").is_some());
        assert!(v["badges"][0].get("image_url").is_none());
    }

    #[test]
    fn channel_badges_show_only_in_the_room_they_belong_to() {
        let resp = UserBadgesResponse {
            display_badges: vec![badge("bc", "broadcaster", "BC4"), badge("p", "premium", "P4")],
            earned_badges: vec![badge("m", "moderator", "M4"), badge("g", "glhf-pledge", "G4")],
            third_party_badges: vec![],
        };
        let ids = |list: Vec<ProfileBadge>| list.into_iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(sheet_twitch_badges(&resp, false)), vec!["p", "g"]);
        assert_eq!(ids(sheet_twitch_badges(&resp, true)), vec!["bc", "p", "g"]);
    }

    #[test]
    fn member_ranks_follow_the_bands_and_the_famous_numbers() {
        assert_eq!(member_rank(1).tier, "ethereal");
        assert_eq!(member_rank(10).label, "Mythic");
        assert_eq!(member_rank(11).label, "Ascendant");
        assert_eq!(member_rank(250).tier, "founder");
        assert_eq!(member_rank(251).label, "Member");
        assert_eq!(member_rank(69).label, "Nice");
        assert_eq!(member_rank(69).tier, "ascendant");
        assert_eq!(member_rank(9000).accent, "#94A3B8");
    }

    #[test]
    fn a_rank_carries_the_desktop_cards_aura() {
        let aura = |n: u32| member_rank(n).aura;
        assert_eq!((aura(1), aura(2), aura(10), aura(11), aura(100)), (0.16, 0.13, 0.13, 0.10, 0.10));
        assert_eq!((aura(101), aura(250), aura(251), aura(9000)), (0.05, 0.05, 0.04, 0.04));
        // The wire form is a plain number (0.13, exactly); a copy saved before the field existed reads as 0.
        let old: MemberRank = serde_json::from_value(json!({ "number": 5, "tier": "mythic", "label": "Mythic", "accent": "#FBBF24" })).unwrap();
        assert_eq!(old.aura, 0.0);
    }

    #[test]
    fn the_login_line_shows_only_beyond_letter_case() {
        assert_eq!(shown_login("xqc", "xQc"), None);
        assert_eq!(shown_login("kr_user", "한국어").as_deref(), Some("kr_user"));
    }

    #[test]
    fn the_header_row_follows_the_surface_order_with_your_streamnook_badge_last() {
        let shown = |id: &str, provider: Option<&str>| ProfileBadge {
            id: id.into(),
            url: format!("https://x/{id}.webp"),
            provider: provider.map(String::from),
            ..Default::default()
        };
        let sn = SnBadge {
            slug: Some("streamnook-singularity".into()),
            image_url: "https://cdn.streamnook.app/badges/streamnook-singularity.webp?v=2".into(),
            label: "StreamNook Singularity".into(),
        };
        let rest = vec![
            shown("ffz-sub", Some("ffz")),
            shown("premium", None),
            shown("7tv-sub", Some("7tv")),
            shown("sn-loadout", Some("streamnook")),
        ];
        let row = own_worn_badges(Some(&sn), rest.clone());
        let ids: Vec<&str> = row.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["premium", "7tv-sub", "ffz-sub", "streamnook-singularity"]);
        let last = row.last().unwrap();
        assert_eq!((last.url.as_str(), last.provider.as_deref()), (sn.image_url.as_str(), Some("streamnook")));
        // Not a member: the row is what the platforms show.
        assert_eq!(own_worn_badges(None, rest).len(), 3);
        // The cap holds with the StreamNook badge counted, and never drops it.
        let many: Vec<ProfileBadge> = (0..20).map(|i| shown(&format!("b{i}"), None)).collect();
        let capped = own_worn_badges(Some(&sn), many);
        assert_eq!(capped.len(), HEADER_BADGE_CAP);
        assert_eq!(capped.last().map(|b| b.id.as_str()), Some("streamnook-singularity"));
    }

    #[test]
    fn every_badge_row_shares_one_surface_order() {
        use crate::services::badge_service::surface_rank;
        let order = ["", "twitch", "kick", "7tv", "ffz", "bttv", "dankchat", "some-new-client", "streamnook"];
        let ranks: Vec<usize> = order.iter().map(|p| surface_rank(p)).collect();
        assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{ranks:?}");
        assert!(surface_rank("some-new-client") < surface_rank("streamnook"));
    }

    // ---- Groups, the worn row and the loadout ----

    fn cb(provider: &str, id: &str) -> ProfileBadge {
        ProfileBadge { id: id.into(), url: format!("https://x/{id}.webp"), provider: Some(provider.into()), ..Default::default() }
    }

    fn pro() -> BttvProBadge {
        BttvProBadge { url: "https://cdn.betterttv.net/badges/pro/pro1.png".into(), started_at: None, glow: false }
    }

    fn loadout(customized: bool, badges: &[&str]) -> Part<IdentityLoadout> {
        Part::Answered(IdentityLoadout {
            twitch_user_id: "100".into(),
            customized,
            badges: badges.iter().map(|b| b.to_string()).collect(),
            paint: None,
            updated_at: None,
        })
    }

    fn ids(list: &[ProfileBadge]) -> Vec<&str> {
        list.iter().map(|b| b.id.as_str()).collect()
    }

    #[test]
    fn the_group_titles_follow_the_chats_provider_order() {
        let keys: Vec<&str> = PROVIDER_GROUPS.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, crate::services::badge_service::THIRD_PARTY_ORDER.to_vec());
        assert_eq!(PROVIDER_GROUPS[0], ("ffz", "FrankerFaceZ"));
        assert_eq!(PROVIDER_GROUPS[9], ("dankchat", "DankChat"));
    }

    #[test]
    fn a_non_member_gets_one_group_per_chat_client_then_other_with_pro_in_betterttv() {
        let chat = vec![cb("ffz", "ffz-1"), cb("bttv", "bttv-dev"), cb("homies", "h-1"), cb("newclient", "n-1")];
        let picked = pick_chat_clients(chat, Some(pro_profile_badge(&pro())), &LoadoutFilter::All, true);
        let groups = badge_groups(vec![cb("", "tw")], vec![cb("", "s7")], picked, vec![]);
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, vec!["Twitch", "7TV", "FrankerFaceZ", "BetterTTV", "Homies", "Other"]);
        let bttv = groups.iter().find(|g| g.title == "BetterTTV").unwrap();
        assert_eq!(ids(&bttv.badges), vec!["bttv-dev", "bttv-pro"], "Pro joins BetterTTV after its contributor badge");
        assert_eq!(ids(&groups.last().unwrap().badges), vec!["n-1"], "a provider the list does not know is Other");
        assert!(badge_groups(vec![], vec![], vec![], vec![]).is_empty());
    }

    #[test]
    fn a_curated_loadout_keeps_only_the_chosen_badges_by_their_exact_key() {
        let filter = loadout_filter(true, &loadout(true, &["ffz:ffz-1", "BTTV:bttv-pro"]));
        let chat = vec![cb("ffz", "ffz-1"), cb("ffz", "ffz-2"), cb("bttv", "bttv-dev")];
        let kept = pick_chat_clients(chat.clone(), Some(pro_profile_badge(&pro())), &filter, false);
        assert_eq!(ids(&kept), vec!["ffz-1", "bttv-pro"]);
        // The Pro badge is matched by its literal capitalised key and by nothing else.
        assert_eq!(BTTV_PRO_KEY, "BTTV:bttv-pro");
        let lower = loadout_filter(true, &loadout(true, &["bttv:bttv-pro"]));
        assert!(ids(&pick_chat_clients(vec![], Some(pro_profile_badge(&pro())), &lower, false)).is_empty());
        // A curated member who left Pro out does not show it.
        let without = loadout_filter(true, &loadout(true, &["ffz:ffz-1"]));
        assert_eq!(ids(&pick_chat_clients(chat, Some(pro_profile_badge(&pro())), &without, false)), vec!["ffz-1"]);
    }

    #[test]
    fn a_member_who_curated_nothing_shows_no_chat_client_badge_at_all() {
        let filter = loadout_filter(true, &loadout(true, &[]));
        assert_eq!(filter, LoadoutFilter::Only(HashSet::new()));
        let kept = pick_chat_clients(vec![cb("ffz", "ffz-1")], Some(pro_profile_badge(&pro())), &filter, true);
        assert!(kept.is_empty(), "curated none is an answer, not a missing one");
        // A member who never curated shows everything, a non-member too.
        assert_eq!(loadout_filter(true, &loadout(false, &["ffz:x"])), LoadoutFilter::All);
        assert_eq!(loadout_filter(false, &Part::Failed), LoadoutFilter::All);
    }

    #[test]
    fn an_unread_loadout_never_puts_a_possibly_hidden_badge_in_the_worn_row() {
        let filter = loadout_filter(true, &Part::Failed);
        assert_eq!(filter, LoadoutFilter::Unknown);
        let chat = vec![cb("ffz", "ffz-1")];
        let worn = worn_row(WornInputs {
            twitch: vec![cb("", "tw")],
            seventv: Some(cb("7tv", "s7")),
            chat: chat.clone(),
            pro: Some(pro_profile_badge(&pro())),
            filter: &filter,
            streamnook: None,
        });
        assert_eq!(ids(&worn), vec!["tw", "s7"], "Twitch and 7TV only");
        // The groups show everything while the loadout is unknown.
        let groups = pick_chat_clients(chat, Some(pro_profile_badge(&pro())), &filter, true);
        assert_eq!(ids(&groups), vec!["ffz-1", "bttv-pro"]);
    }

    #[test]
    fn the_worn_row_is_in_surface_order_with_the_streamnook_badge_last_and_capped() {
        let sn = SnBadge { slug: Some("streamnook-mobius".into()), image_url: "https://x/m.webp".into(), label: "StreamNook Mobius".into() };
        let worn = worn_row(WornInputs {
            twitch: vec![cb("", "tw")],
            seventv: Some(cb("7tv", "s7")),
            chat: vec![cb("ffz", "ffz-1"), cb("bttv", "bttv-dev")],
            pro: Some(pro_profile_badge(&pro())),
            filter: &LoadoutFilter::All,
            streamnook: Some(&sn),
        });
        assert_eq!(ids(&worn), vec!["tw", "s7", "ffz-1", "bttv-dev", "bttv-pro", "streamnook-mobius"]);
        let many: Vec<ProfileBadge> = (0..20).map(|i| cb("ffz", &format!("f{i}"))).collect();
        let capped = worn_row(WornInputs { twitch: vec![], seventv: None, chat: many, pro: None, filter: &LoadoutFilter::All, streamnook: Some(&sn) });
        assert_eq!(capped.len(), HEADER_BADGE_CAP);
        assert_eq!(capped.last().map(|b| b.id.as_str()), Some("streamnook-mobius"));
    }

    // ---- The page link's login ----

    #[test]
    fn the_page_link_comes_from_the_login_twitch_holds_for_the_id() {
        // Twitch's login wins over whatever the caller named.
        assert_eq!(page_login(Some("tyler"), "kick_name"), "tyler");
        // Only a failed Helix lets the caller's login stand, and only if it is shaped like one.
        assert_eq!(page_login(None, "Tyler"), "Tyler");
        assert_eq!(page_login(None, "한국어"), "");
        assert_eq!(page_login(None, "has space"), "");
        assert_eq!(page_login(Some(""), "tyler"), "tyler");
    }

    // ---- Generations, epochs, merges ----

    #[test]
    fn a_build_covers_a_caller_only_if_it_read_everything_the_caller_asked_at() {
        assert!(covers((2, 5), (2, 5)) && covers((3, 5), (2, 5)) && covers((2, 6), (2, 5)));
        assert!(!covers((2, 4), (2, 5)) && !covers((1, 9), (2, 5)));
        assert_eq!(newest((1, 9), (2, 5)), (2, 9));
        assert!(should_write((2, 5), (2, 5)));
        assert!(should_write((2, 5), (3, 6)));
        assert!(!should_write((2, 6), (2, 5)), "a caller asked for something newer than this build read");
    }

    #[test]
    fn an_epoch_only_moves_forward_and_an_empty_id_has_none() {
        let id = "test-epoch-1";
        assert_eq!(member_epoch(id), 0);
        bump_member_epoch(id);
        let first = member_epoch(id);
        assert!(first > 0);
        bump_member_epoch(id);
        assert!(member_epoch(id) > first);
        bump_member_epoch("");
        assert_eq!(member_epoch(""), 0);
        assert_eq!(member_epoch("test-epoch-never"), 0);
    }

    #[test]
    fn a_part_that_failed_keeps_the_cached_answer_and_no_answer_is_never_invented() {
        let cached = Soft {
            member: Some(MemberFacts { member_id: "100".into(), stats: Part::Answered(Some(json!({ "hours_watched": 5 }))), ..Default::default() }),
            loadout: loadout(true, &["ffz:ffz-1"]),
            bttv: Part::Answered(Some(pro())),
        };
        let fresh = Soft { member: Some(MemberFacts { member_id: "100".into(), ..Default::default() }), loadout: Part::Failed, bttv: Part::Answered(None) };
        let merged = merge_soft(fresh, Some(&cached));
        assert_eq!(merged.loadout, loadout(true, &["ffz:ffz-1"]), "a failed loadout takes the cached one");
        assert_eq!(merged.bttv, Part::Answered(None), "an answered 'not Pro' is kept over the old Pro");
        assert!(merged.member.unwrap().stats.is_answered(), "a failed member part takes the cached copy");
        // With no cached copy a failed part stays failed.
        let alone = merge_soft(Soft::default(), None);
        assert!(!alone.loadout.is_answered() && !alone.bttv.is_answered() && alone.member.is_none());
        // A non-member is not given a cached member half.
        assert!(merge_soft(Soft::default(), Some(&cached)).member.is_none());
    }

    #[test]
    fn the_own_header_is_saved_only_when_nothing_it_shows_is_unknown() {
        assert!(should_persist_own(true, false, &Part::Failed), "a non-member has no loadout to wait for");
        assert!(should_persist_own(true, true, &loadout(false, &[])));
        assert!(!should_persist_own(true, true, &Part::Failed), "an unread loadout is never saved over a known one");
        assert!(!should_persist_own(false, false, &Part::Failed), "the member list had not loaded");
    }

    #[test]
    fn the_roast_is_kept_per_presentation_and_rolled_once_for_a_new_one() {
        let mut kept = None;
        let mut half = || 0.5;
        let mut high = || 0.99;
        let first = next_roast(&mut kept, Some(1300.0), Some("open-a"), &mut half).expect("a line");
        // The same presentation never rolls again, whatever the numbers, and neither does a caller naming none.
        assert_eq!(next_roast(&mut kept, Some(1300.0), Some("open-a"), &mut high).as_deref(), Some(first.as_str()));
        assert_eq!(next_roast(&mut kept, Some(1300.0), None, &mut high).as_deref(), Some(first.as_str()));
        // A new presentation rolls once, avoiding the line it replaces, then holds.
        let second = next_roast(&mut kept, Some(1300.0), Some("open-b"), &mut half).expect("a line");
        assert_ne!(second, first);
        assert_eq!(next_roast(&mut kept, Some(1300.0), Some("open-b"), &mut high).as_deref(), Some(second.as_str()));
        // A kept line that no longer fits the hours is replaced; the presentation stays.
        let low = next_roast(&mut kept, Some(1.0), Some("open-b"), &mut half).expect("a low-hours line");
        assert_ne!(low, second);
        assert!(member_profile::roast_is_eligible(&low, 1.0));
        assert_eq!(kept.as_ref().and_then(|k| k.open_id.as_deref()), Some("open-b"));
        // Hidden or unknown hours: no line, nothing kept.
        assert_eq!(next_roast(&mut kept, None, Some("open-b"), &mut half), None);
        assert!(kept.is_none());
    }

    // ---- Theme, accent, frame, caption ----

    #[test]
    fn the_theme_and_accent_follow_the_desktop_rule() {
        let tier = |n| theme_and_accent(&Look::default(), None, Some(n));
        assert_eq!(tier(7), (Some("tier".into()), Some("#fbbf24".into())));
        assert_eq!(tier(1).1.as_deref(), Some("#8b5cf6"));
        // An Atmosphere: its own accent triple.
        let atm = Look {
            atmosphere: Some(registry::Atmosphere { id: "aurora".into(), accent: "45, 212, 191".into(), ..Default::default() }),
            ..Default::default()
        };
        assert_eq!(theme_and_accent(&atm, None, Some(7)), (Some("atmosphere".into()), Some("#2dd4bf".into())));
        // The Cologne look counts as an atmosphere and wears its accent.
        let cologne = Look {
            cologne: Some(member_theme::Cologne { coin: true, frame: false }),
            cologne_accent: Some("214, 177, 92".into()),
            ..Default::default()
        };
        assert_eq!(theme_and_accent(&cologne, None, Some(7)), (Some("atmosphere".into()), Some("#d6b15c".into())));
        // A paint theme needs a paint, and takes the middle gradient color (layer order).
        let stop = |c: &str| PaintStop { at: 0.0, color: c.into() };
        let paint = PaintView {
            id: "p".into(),
            kind: "linear".into(),
            layers: vec![
                PaintLayerView { kind: "linear".into(), stops: vec![stop("#112233ff"), stop("#445566ff")], ..Default::default() },
                PaintLayerView { kind: "image".into(), image_url: Some("https://x/i.webp".into()), ..Default::default() },
                PaintLayerView { kind: "radial".into(), stops: vec![stop("#778899ff"), stop("#aabbccff")], ..Default::default() },
            ],
            ..Default::default()
        };
        let paint_theme = Look { paint_theme: true, ..Default::default() };
        assert_eq!(theme_and_accent(&paint_theme, Some(&paint), Some(7)), (Some("paint".into()), Some("#778899".into())));
        assert_eq!(paint_accent(&paint).as_deref(), Some("#778899"));
        assert_eq!(theme_and_accent(&paint_theme, None, Some(7)), (Some("tier".into()), Some("#fbbf24".into())), "no paint, no paint theme");
        // A paint with no gradient colors falls back to the rank's accent.
        let flat = PaintView { id: "f".into(), kind: "solid".into(), ..Default::default() };
        assert_eq!(theme_and_accent(&paint_theme, Some(&flat), Some(7)), (Some("paint".into()), Some("#fbbf24".into())));
        assert_eq!(rgb_triple_hex("1, 2"), None);
        assert_eq!(rgb_triple_hex("300, 1, 2"), None);
    }

    #[test]
    fn the_frame_and_the_caption_come_from_what_they_equipped_and_wear() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_for_test(
            &[("100", 7)],
            vec![
                entry_for_test(registry::DEFAULT_SLUG, "badge", "streamnook-logo.png", true),
                entry_for_test("streamnook-mobius", "badge", "https://cdn.streamnook.app/badges/mobius.webp", false),
                entry_for_test("gold-frame", "frame", "https://cdn.streamnook.app/frames/gold.png", false),
            ],
            &[("100", "streamnook-mobius"), ("100", "gold-frame")],
            &[("100", "streamnook-mobius")],
            vec![],
        );
        let mut facts = MemberFacts { member_id: "100".into(), ..Default::default() };
        assert_eq!(frame_view(&facts), None, "no equipment read, no frame");
        facts.equipment = Part::Answered(vec![json!({ "slot": "frame", "cosmetic_slug": "gold-frame" })]);
        let frame = frame_view(&facts).expect("a frame");
        assert_eq!((frame.url.as_str(), frame.slice, frame.border), ("https://cdn.streamnook.app/frames/gold.png", [199, 159, 199, 159], [18, 14]));
        facts.equipment = Part::Answered(vec![json!({ "slot": "frame", "cosmetic_slug": "gone" })]);
        assert_eq!(frame_view(&facts), None);
        assert_eq!(worn_caption("100").as_deref(), Some("streamnook mobius"));
        assert_eq!(worn_caption("999"), None, "not a member");
        set_for_test(&[("100", 7)], vec![entry_for_test(registry::DEFAULT_SLUG, "badge", "streamnook-logo.png", true)], &[], &[("100", registry::DEFAULT_SLUG)], vec![]);
        assert_eq!(worn_caption("100"), None, "the default badge only repeats the rank");
    }

    // ---- A non-member's sheet and the golden member sheet ----

    fn twitch_badge_info(id: &str, set: &str, x4: &str) -> BadgeInfo {
        BadgeInfo {
            id: id.into(),
            set_id: set.into(),
            version: "1".into(),
            title: format!("{set} title"),
            description: String::new(),
            image_1x: format!("{x4}-1x"),
            image_2x: format!("{x4}-2x"),
            image_4x: x4.into(),
            click_action: None,
            click_url: None,
        }
    }

    fn user_info(id: &str, login: &str, name: &str, broadcaster_type: &str) -> UserInfo {
        UserInfo {
            id: id.into(),
            login: login.into(),
            display_name: name.into(),
            email: None,
            profile_image_url: Some("https://static-cdn.jtvnw.net/avatar.png".into()),
            broadcaster_type: Some(broadcaster_type.into()),
        }
    }

    fn fixed_clock() -> Clock {
        Clock { now: Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap(), zone: Zone::Fixed(FixedOffset::west_opt(8 * 3600).unwrap()) }
    }

    fn fetched(user_id: &str, login: &str, name: &str, badges: UserBadgesResponse) -> Fetched {
        Fetched {
            user_id: user_id.into(),
            login: login.into(),
            display_name: name.into(),
            in_room: false,
            badges: Some(badges),
            seventv_badges: vec![],
            seventv_selected: None,
            seventv_user_id: None,
            paint_def: None,
            paint: None,
            helix: Some(user_info(user_id, login, name, "")),
            picture: None,
            name_color: "#9147ff".into(),
            look: Look::default(),
        }
    }

    #[test]
    fn a_non_members_sheet_has_groups_and_none_of_the_member_fields() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_for_test(&[("100", 7)], vec![entry_for_test(registry::DEFAULT_SLUG, "badge", "streamnook-logo.png", true)], &[], &[], vec![]);
        let ffz = UserBadge { badge_info: twitch_badge_info("ffz-3", "ffz", "F3"), provider: BadgeProvider::FFZ };
        let resp = UserBadgesResponse {
            display_badges: vec![UserBadge { badge_info: twitch_badge_info("prem", "premium", "P4"), provider: BadgeProvider::Twitch }],
            earned_badges: vec![],
            third_party_badges: vec![ffz],
        };
        let f = fetched("555", "viewer", "Viewer", resp);
        let (sheet, roast_hours) = assemble(&f, &Soft::default(), &fixed_clock(), &member_profile::Catalog::parse(member_profile::TEST_CATALOG_JSON).unwrap());
        assert_eq!(sheet.groups.iter().map(|g| g.title.as_str()).collect::<Vec<_>>(), vec!["Twitch", "FrankerFaceZ"]);
        assert!(sheet.member.is_none() && sheet.member_profile.is_none() && sheet.streamnook.is_none());
        assert!(sheet.worn.is_empty() && sheet.page_link.is_none() && sheet.theme.is_none() && sheet.accent.is_none());
        assert!(sheet.frame.is_none() && sheet.caption.is_none());
        assert_eq!(roast_hours, None);
        let v = serde_json::to_value(&sheet).unwrap();
        for key in ["worn", "page_link", "theme", "frame", "accent", "caption", "seventv_user_id", "member_profile", "member"] {
            assert!(v.get(key).is_none(), "{key} stays off a non-member's wire");
        }
    }

    #[test]
    fn an_atmosphere_rides_the_sheet_only_when_the_theme_is_atmosphere() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_for_test(&[("100", 7)], vec![entry_for_test(registry::DEFAULT_SLUG, "badge", "streamnook-logo.png", true)], &[], &[], vec![]);
        let cat = member_profile::Catalog::parse(member_profile::TEST_CATALOG_JSON).unwrap();
        let aurora = registry::Atmosphere { id: "aurora".into(), accent: "45, 212, 191".into(), ..Default::default() };
        let paint = PaintView { id: "p".into(), kind: "solid".into(), ..Default::default() };
        let cologne = member_theme::Cologne { coin: false, frame: false };
        // (the member's look, a paint worn, a member?) -> the theme the sheet reports.
        let looks = [
            (Look { atmosphere: Some(aurora.clone()), paint_theme: true, ..Default::default() }, true, "100", "atmosphere"),
            (Look { atmosphere: Some(aurora.clone()), ..Default::default() }, false, "100", "atmosphere"),
            (Look { cologne: Some(cologne), cologne_accent: Some("1, 2, 3".into()), ..Default::default() }, false, "100", "atmosphere"),
            (Look { paint_theme: true, ..Default::default() }, true, "100", "paint"),
            (Look::default(), true, "100", "tier"),
        ];
        for (look, with_paint, id, theme) in looks {
            let mut f = fetched(id, "tyler", "Tyler", UserBadgesResponse { display_badges: vec![], earned_badges: vec![], third_party_badges: vec![] });
            f.look = look;
            f.paint = with_paint.then(|| paint.clone());
            let member = Soft { member: Some(MemberFacts { member_id: id.into(), number: Some(7), ..Default::default() }), ..Default::default() };
            let (sheet, _) = assemble(&f, &member, &fixed_clock(), &cat);
            assert_eq!(sheet.theme.as_deref(), Some(theme));
            assert_eq!(sheet.atmosphere.is_some(), f.look.atmosphere.is_some());
            if sheet.atmosphere.is_some() {
                assert_eq!(sheet.theme.as_deref(), Some("atmosphere"), "an atmosphere is only ever on a sheet themed as one");
            }
            if sheet.theme.as_deref() != Some("atmosphere") {
                assert!(sheet.atmosphere.is_none(), "{theme}: no atmosphere outside an atmosphere theme");
            }
        }
        // Someone who is not a member wears no theme and no atmosphere.
        let mut stranger = fetched("555", "viewer", "Viewer", UserBadgesResponse { display_badges: vec![], earned_badges: vec![], third_party_badges: vec![] });
        stranger.look = Look::default();
        let (sheet, _) = assemble(&stranger, &Soft::default(), &fixed_clock(), &cat);
        assert!(sheet.theme.is_none() && sheet.atmosphere.is_none() && sheet.cologne.is_none());
    }

    #[test]
    fn golden_member_sheet() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_for_test(
            &[("100", 7)],
            vec![
                entry_for_test(registry::DEFAULT_SLUG, "badge", "streamnook-logo.png", true),
                entry_for_test("streamnook-mobius", "badge", "https://cdn.streamnook.app/badges/mobius.webp", false),
                entry_for_test("relic-gem", "relic", "https://cdn.streamnook.app/relics/gem.webp", false),
                entry_for_test("gold-frame", "frame", "https://cdn.streamnook.app/frames/gold.png", false),
            ],
            &[("100", "streamnook-mobius"), ("100", "relic-gem"), ("100", "gold-frame")],
            &[("100", "streamnook-mobius")],
            vec![],
        );
        let cat = member_profile::Catalog::parse(member_profile::TEST_CATALOG_JSON).unwrap();
        let roasts = member_profile::Roasts::parse(member_profile::TEST_ROASTS_JSON).unwrap();
        let ffz = |id: &str, x4: &str| UserBadge { badge_info: twitch_badge_info(id, "ffz", x4), provider: BadgeProvider::FFZ };
        let mut f = fetched(
            "100",
            "tyler",
            "Tyler",
            UserBadgesResponse {
                display_badges: vec![UserBadge { badge_info: twitch_badge_info("prem", "premium", "P4"), provider: BadgeProvider::Twitch }],
                earned_badges: vec![],
                third_party_badges: vec![ffz("ffz-3", "F3"), ffz("ffz-9", "F9")],
            },
        );
        let sevenv = seventv_badge(&json!({ "id": "7tvb", "name": "Sub 1y", "tooltip": "Sub 1y" })).unwrap();
        f.seventv_badges = vec![sevenv.clone()];
        f.seventv_selected = Some(ProfileBadge { provider: Some("7tv".into()), ..sevenv });
        f.seventv_user_id = Some("7tv-user".into());
        f.helix = Some(user_info("100", "tyler", "Tyler", "affiliate"));
        f.look = Look {
            cologne: Some(member_theme::Cologne { coin: true, frame: false }),
            cologne_accent: Some("214, 177, 92".into()),
            ..Default::default()
        };
        let facts = MemberFacts {
            member_id: "100".into(),
            number: Some(7),
            hidden: vec![],
            stats: Part::Answered(Some(json!({
                "hours_watched": 1200.4, "recap_hours": 99.7, "messages_sent": 9500, "recap_messages": "600",
                "streams_watched": 300, "recap_subs_gifted": 5, "profile_views": 4321,
            }))),
            favorite: Part::Answered(Some(json!({ "channel_login": "xqc", "channel_name": "xQc", "minutes": 18_750 }))),
            saved: Part::Answered(["halloween".to_string()].into_iter().collect()),
            ivr: Part::Answered(Some(crate::services::ivr::IvrUserSummary {
                followers: Some(1_234_567),
                created_at: Some("2014-03-20T10:00:00Z".into()),
                is_affiliate: true,
                is_partner: false,
                is_staff: false,
            })),
            emotes: Part::Answered(vec![
                json!({ "emote_id": "e1", "emote_name": "OMEGALUL", "provider": "bttv", "image_url": "https://x/1", "count": 90 }),
                json!({ "emote_id": "e2", "emote_name": "KEKW", "provider": "7tv", "image_url": "https://x/2", "count": 50 }),
                json!({ "emote_id": "e3", "emote_name": "nourl", "provider": "7tv", "image_url": null, "count": 5 }),
            ]),
            totals: Part::Answered((12, 345)),
            equipment: Part::Answered(vec![
                json!({ "slot": "frame", "cosmetic_slug": "gold-frame" }),
                json!({ "slot": "relic_primary", "cosmetic_slug": "relic-gem" }),
            ]),
            seventv: Part::Answered((2, 1)),
            counted_views: Some(4322),
        };
        let soft = Soft { member: Some(facts), loadout: loadout(true, &["ffz:ffz-3", "BTTV:bttv-pro"]), bttv: Part::Answered(Some(pro())) };
        let (mut sheet, roast_hours) = assemble(&f, &soft, &fixed_clock(), &cat);
        // The roast, from the pool this test owns and the fixed numbers [0.5, 0.5].
        let mut rand = || 0.5;
        let line = roasts.pick(roast_hours.expect("the hours are shown"), &mut rand, None).text;
        set_roast(&mut sheet, Some(line));
        let wall: &AccoladeWall = sheet.member_profile.as_ref().unwrap().accolades.as_ref().expect("the wall");
        assert_eq!((wall.earned, wall.total), (6, 7));

        let built = serde_json::to_value(&sheet).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/mobile/profile/fixtures/profileSheet.member.json");
        if std::env::var("UPDATE_FIXTURES").as_deref() == Ok("1") {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("{}\n", serde_json::to_string_pretty(&built).unwrap())).unwrap();
            return;
        }
        let text = std::fs::read_to_string(&path).expect("src/mobile/profile/fixtures/profileSheet.member.json is committed");
        let fixture: Value = serde_json::from_str(&text).expect("the fixture is JSON");
        assert_eq!(built, fixture, "the member sheet changed: run with UPDATE_FIXTURES=1 and review the fixture diff");
    }
}
