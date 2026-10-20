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
use crate::models::chat_layout::PaintView;
use crate::services::badge_service::UserBadgesResponse;
use crate::services::cosmetics_registry::{self as registry, SnBadge};
use crate::services::css_fill::Fill;
use crate::services::member_profile::{self, MemberProfile};
use crate::services::member_theme;
use crate::services::seventv_cosmetics_resolver::{self as resolver, ProfilePicture, UserCosmetics};
use lru::LruCache;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::Mutex;

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

/// The look a member wears on their profile: an Atmosphere (wear-gated) or the
/// Cologne look. Non-members wear neither.
async fn worn_look(member_id: &str) -> (Option<registry::Atmosphere>, Option<member_theme::Cologne>) {
    match member_theme::resolve(member_id).await {
        Some(theme) => (theme.atmosphere_id.as_deref().and_then(registry::atmosphere), theme.cologne),
        None => (None, None),
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
}

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
}

pub fn member_rank(number: u32) -> MemberRank {
    let (tier, label, accent) = match number {
        1 => ("ethereal", "Ethereal", "#8B5CF6"),
        2..=10 => ("mythic", "Mythic", "#FBBF24"),
        11..=100 => ("ascendant", "Ascendant", "#22D3EE"),
        101..=250 => ("founder", "Founder", "#E2E8F0"),
        _ => ("member", "Member", "#94A3B8"),
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

static SHEETS: Mutex<Option<LruCache<String, ProfileSheet>>> = Mutex::new(None);

fn sheet_key(user_id: &str, channel_id: &str) -> String {
    format!("{user_id}:{channel_id}")
}

/// The sheet as last built for this person in this room, without asking
/// anything, so a reopen paints at once.
pub fn cached_sheet(user_id: &str, channel_id: Option<&str>) -> Option<ProfileSheet> {
    let key = sheet_key(user_id, channel_id.unwrap_or(user_id));
    SHEETS.lock().ok()?.as_mut()?.get(&key).cloned()
}

/// Everything the sheet shows for one person in one room. The badge lookup is in
/// the room's context (channel badges resolve there); without a room it falls
/// back to the person's own channel. Every source fails soft.
pub async fn profile_sheet(
    user_id: &str,
    login: &str,
    display_name: &str,
    color: Option<&str>,
    channel_id: Option<&str>,
    channel_login: Option<&str>,
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
    let (badges, inventory, helix, pictures, (atmosphere, cologne), member_reads) = tokio::join!(
        crate::commands::badge_service::get_user_badges_with_earned_unified(
            user_id.to_string(),
            login.to_string(),
            channel_id.clone(),
            channel_login,
        ),
        async {
            match resolver::inventory(user_id).await {
                Some(inv) => Some(inv),
                None => {
                    let worn = resolver::resolve(user_id.to_string()).await;
                    (!worn.hard_fail).then_some(worn.data)
                }
            }
        },
        crate::services::twitch_service::TwitchService::get_user_by_id(user_id),
        resolver::profile_pictures(vec![user_id.to_string()]),
        worn_look(user_id),
        member_profile::member_reads(user_id, login),
    );
    let badges = badges.map_err(|e| log::debug!("[ProfileView] badges unavailable: {e}")).ok();
    let seventv_count = inventory.as_ref().map(|i| (i.paints.len() + i.badges.len()) as u64).unwrap_or(0);
    let member = member_reads.map(|m| m.finish(seventv_count));
    let mut groups = Vec::new();
    let mut push = |title: &str, list: Vec<ProfileBadge>| {
        if !list.is_empty() {
            groups.push(BadgeGroup { title: title.into(), badges: list });
        }
    };
    // A member who hides their badge collection hides every group, as their
    // desktop card does; StreamNook comes last. Relics ride `member_profile`.
    let hide_badges = member.as_ref().is_some_and(|m| m.hide_badges);
    if !hide_badges {
        push("Twitch", badges.as_ref().map(|b| sheet_twitch_badges(b, in_room)).unwrap_or_default());
        push(
            "7TV",
            inventory
                .as_ref()
                .map(|i| {
                    let mut seen = HashSet::new();
                    i.badges.iter().filter_map(seventv_badge).filter(|b| seen.insert(b.id.clone())).collect()
                })
                .unwrap_or_default(),
        );
        push("Chat clients", badges.as_ref().map(third_party_badges).unwrap_or_default());
        // StreamNook last. Without the member's profile reads (their prefs failed to
        // read, or the member list was still loading) the badge they wear still shows.
        let streamnook = match member.as_ref() {
            Some(m) => m.streamnook.clone(),
            None => member_profile::streamnook_badges(registry::member_badge(user_id, false).as_ref(), &Default::default(), &[]),
        };
        push("StreamNook", streamnook);
    }
    let picture = pictures.get(user_id).cloned().flatten().and_then(|p| pick_avatar(&p, 56.0, 3.0, true));
    let (paint_def, paint) = worn_paint(user_id, inventory.as_ref());
    let sheet = ProfileSheet {
        user_id: user_id.to_string(),
        login: login.to_string(),
        display_name: display_name.to_string(),
        shown_login: shown_login(login, display_name),
        avatar_url: helix.ok().and_then(|u| u.profile_image_url).filter(|u| !u.is_empty()),
        picture,
        // A sheet opened from a mention carries no color; chat has usually seen it.
        name_color: crate::services::chat_row_view::readable_color(Some(
            color
                .map(String::from)
                .or_else(|| crate::services::chat_row_view::seen_color(login))
                .as_deref()
                .unwrap_or("#9147FF"),
        )),
        paint,
        paint_def,
        groups,
        streamnook: registry::member_badge(user_id, false),
        member: registry::member_number(user_id).map(member_rank),
        atmosphere,
        cologne,
        member_profile: member.map(|m| m.profile),
    };
    if let Ok(mut guard) = SHEETS.lock() {
        guard
            .get_or_insert_with(|| LruCache::new(NonZeroUsize::new(256).unwrap()))
            .put(sheet_key(user_id, &channel_id), sheet.clone());
    }
    sheet
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

/// The viewer's own identity header, built fresh (and saved for the next cold
/// launch).
pub async fn own_identity(user_id: &str, login: &str, display_name: &str) -> OwnIdentity {
    let (badges, inventory, resolved, pictures, (atmosphere, cologne), banner_url, colors) = tokio::join!(
        crate::commands::badge_service::get_user_badges_with_earned_unified(
            user_id.to_string(),
            login.to_string(),
            user_id.to_string(),
            login.to_string(),
        ),
        resolver::inventory(user_id),
        crate::commands::identity::get_streamnook_identity_resolved(user_id.to_string()),
        resolver::profile_pictures(vec![user_id.to_string()]),
        worn_look(user_id),
        twitch_banner(user_id),
        crate::services::twitch_service::TwitchService::get_user_chat_colors(vec![user_id.to_string()]),
    );
    let badges = badges.ok();
    // The card's name color before Edit profile has read it: what chat shows.
    let name_color = colors
        .get(user_id)
        .filter(|c| !c.is_empty())
        .cloned()
        .or_else(|| crate::services::chat_row_view::seen_color(login))
        .map(|c| crate::services::chat_row_view::readable_color(Some(&c)));
    // Twitch: what the account displays, minus the badges of one channel.
    let twitch_worn: Vec<ProfileBadge> = badges
        .as_ref()
        .map(|b| {
            let worn = UserBadgesResponse {
                display_badges: b
                    .display_badges
                    .iter()
                    .filter(|x| !CHANNEL_SCOPED.contains(&x.badge_info.set_id.as_str()))
                    .cloned()
                    .collect(),
                earned_badges: Vec::new(),
                third_party_badges: Vec::new(),
            };
            twitch_badges(&worn)
        })
        .unwrap_or_default();
    // 7TV: only the badge selected, what chat knows first.
    let selected_badge = resolver::peek(user_id)
        .and_then(|c| selected(&c.badges).cloned())
        .or_else(|| inventory.as_ref().and_then(|i| selected(&i.badges).cloned()));
    // Tagged as 7TV so the surface order places it.
    let seventv_worn: Vec<ProfileBadge> = selected_badge
        .as_ref()
        .and_then(seventv_badge)
        .map(|b| ProfileBadge { provider: Some("7tv".to_string()), ..b })
        .into_iter()
        .collect();
    let other = chat_clients_worn(resolved.ok(), badges.as_ref());
    let streamnook = registry::member_badge(user_id, false);
    let worn = own_worn_badges(streamnook.as_ref(), twitch_worn.into_iter().chain(seventv_worn).chain(other).collect());
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
        atmosphere,
        cologne,
    };
    if let (Some(path), Ok(body)) = (own_path(), serde_json::to_vec(&header)) {
        let _ = std::fs::write(path, body);
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
    use crate::services::badge_service::{BadgeInfo, BadgeProvider, UserBadge};
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
            member_profile: Some(MemberProfile {
                stats: vec![member_profile::MemberStat { key: "views".into(), label: "Profile views".into(), value: "12".into(), caption: None }],
                ..Default::default()
            }),
        };
        let v = serde_json::to_value(&sheet).unwrap();
        assert_eq!(v["member"]["number"], 7);
        assert_eq!(v["member"]["tier"], "mythic");
        assert_eq!(v["member_profile"]["stats"][0]["value"], "12");
        assert!(v["member_profile"]["top_emotes"].is_array());
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
}
