use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

// ============================================================================
// GQL STRUCTS (per-user badge lookup)
// ============================================================================
//
// Inline query, not a persisted hash. Twitch rotates persisted-query hashes
// without notice (the old `ViewerCard` hash died 2026-09 with
// `PersistedQueryNotFound`), and an inline query keeps working as long as the
// fields exist. Anonymous with the web client id: `user.displayBadges` and
// `channelViewer.earnedBadges` are both public.

const BADGE_LOOKUP_QUERY: &str = r#"
query StreamNookBadgeLookup($id: ID!, $login: String!, $channelID: ID!, $channelLogin: String!) {
    user(id: $id) {
        displayBadges(channelID: $channelID) { setID version }
    }
    channelViewer(userLogin: $login, channelLogin: $channelLogin) {
        earnedBadges { setID version }
    }
}
"#;

#[derive(Debug, Serialize)]
struct BadgeLookupRequest {
    query: &'static str,
    variables: BadgeLookupVariables,
}

#[derive(Debug, Serialize)]
struct BadgeLookupVariables {
    id: String,
    login: String,
    #[serde(rename = "channelID")]
    channel_id: String,
    #[serde(rename = "channelLogin")]
    channel_login: String,
}

#[derive(Debug, Deserialize)]
struct GQLResponse {
    data: Option<GQLData>,
    #[serde(default)]
    errors: Vec<GQLError>,
}

#[derive(Debug, Deserialize)]
struct GQLError {
    #[serde(default)]
    message: String,
}

#[derive(Debug, Deserialize)]
struct GQLData {
    #[serde(default)]
    user: Option<TargetUser>,
    #[serde(rename = "channelViewer", default)]
    channel_viewer: Option<ChannelViewer>,
}

// Twitch answers `null` (not `[]`) for an empty badge list, so these are
// Option<Vec>: `#[serde(default)]` alone only covers a MISSING field.
#[derive(Debug, Deserialize)]
struct TargetUser {
    #[serde(rename = "displayBadges", default)]
    display_badges: Option<Vec<GQLBadge>>,
}

#[derive(Debug, Deserialize)]
struct ChannelViewer {
    #[serde(rename = "earnedBadges", default)]
    earned_badges: Option<Vec<GQLBadge>>,
}

#[derive(Debug, Deserialize)]
struct GQLBadge {
    #[serde(rename = "setID")]
    set_id: String,
    version: String,
}

/// `id.twitch.tv/oauth2/validate` body: the subset we need to learn which
/// Twitch user a Drops token belongs to.
#[derive(Debug, Deserialize)]
struct TokenValidation {
    #[serde(default)]
    user_id: String,
}

// ============================================================================
// MODELS
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BadgeInfo {
    pub id: String,
    pub set_id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub image_1x: String,
    pub image_2x: String,
    pub image_4x: String,
    pub click_action: Option<String>,
    pub click_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBadge {
    pub badge_info: BadgeInfo,
    pub provider: BadgeProvider,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::upper_case_acronyms)] // FFZ / BTTV are established acronyms (FrankerFaceZ / BetterTTV)
pub enum BadgeProvider {
    Twitch,
    FFZ,
    BTTV,
    Chatterino,
    Homies,
    Chatsen,
    Chatty,
    DankChat,
    Moltorino,
}

impl BadgeProvider {
    /// The canonical lowercase id this provider carries EVERYWHERE it crosses a
    /// boundary: the serde wire form (`rename_all = "lowercase"` above), the
    /// `<provider>:<id>` loadout key the Identity API dispatches on, and the
    /// overlay's per-provider badge toggles.
    ///
    /// `format!("{:?}", provider)` yields PascalCase and must never reach any of
    /// those. `get_user_profile_complete` used to send the Debug form while every
    /// other command sent the serde form, so the profile card held two spellings
    /// of one provider: it grouped third-party badges under a catch-all "Other"
    /// whenever the cosmetics cache answered first, and built `FFZ:ffz-3` style
    /// loadout keys that matched nothing whenever the Rust profile answered
    /// first, hiding a member's curated badge outright.
    ///
    /// Pinned to the serde form by `provider_key_matches_the_serde_wire_form`.
    pub fn as_key(&self) -> &'static str {
        match self {
            BadgeProvider::Twitch => "twitch",
            BadgeProvider::FFZ => "ffz",
            BadgeProvider::BTTV => "bttv",
            BadgeProvider::Chatterino => "chatterino",
            BadgeProvider::Homies => "homies",
            BadgeProvider::Chatsen => "chatsen",
            BadgeProvider::Chatty => "chatty",
            BadgeProvider::DankChat => "dankchat",
            BadgeProvider::Moltorino => "moltorino",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserBadgesResponse {
    pub display_badges: Vec<UserBadge>,
    pub earned_badges: Vec<UserBadge>,
    pub third_party_badges: Vec<UserBadge>,
}

/// One distinct third-party badge for the browse gallery (not per-user). The
/// gallery groups these by `provider` into collapsible sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThirdPartyGalleryBadge {
    pub id: String,
    pub provider: BadgeProvider,
    pub title: String,
    pub image_1x: String,
    pub image_2x: String,
    pub image_4x: String,
    pub user_count: usize,
    pub owned: bool,
    pub click_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedBadgeLink {
    pub link_type: String, // "category" or "drops"
    pub name: String,
    pub original_text: String,
}

// ============================================================================
// THIRD PARTY BADGE STRUCTS
// ============================================================================

#[derive(Debug, Clone, Deserialize)]
struct FFZBadgesResponse {
    badges: Vec<FFZBadge>,
    users: HashMap<String, Vec<u32>>,
}

#[derive(Debug, Clone, Deserialize)]
struct FFZBadge {
    id: u32,
    title: Option<String>,
    name: Option<String>,
    urls: HashMap<String, String>,
}

// BetterTTV: GET https://api.betterttv.net/3/cached/badges (bare JSON array).
// One entry PER USER (not per distinct badge): `provider_id` is the holder's
// Twitch user id, and the nested `badge` carries the single SVG image plus its
// description (which doubles as the title). The feed is small (~160 entries
// across only 4 distinct badge types: Translator, Emote Approver, NightDev
// Developer, Support Volunteer), so the gallery's (provider, title) dedupe
// collapses it to one tile per type.
#[derive(Debug, Clone, Deserialize)]
struct BttvBadge {
    #[serde(rename = "providerId")]
    provider_id: String,
    badge: BttvBadgeInner,
}

#[derive(Debug, Clone, Deserialize)]
struct BttvBadgeInner {
    description: String,
    svg: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatterinoBadgesResponse {
    badges: Vec<ChatterinoBadge>,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatterinoBadge {
    tooltip: String,
    image1: String,
    image2: Option<String>,
    image3: Option<String>,
    users: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct HomiesBadgesResponse {
    badges: Vec<HomiesBadge>,
}

#[derive(Debug, Clone, Deserialize)]
struct HomiesBadge {
    tooltip: String,
    image1: String,
    image2: Option<String>,
    image3: Option<String>,
    users: Vec<String>,
}

// Chatsen: GET https://api.chatsen.app/account/badges (bare JSON array, served as
// text/plain, 403 without a browser User-Agent which our http_client already sends).
#[derive(Debug, Clone, Deserialize)]
struct ChatsenBadge {
    id: String,
    name: String,
    #[serde(default)]
    mipmap: Vec<String>,
    #[serde(default)]
    users: Vec<String>,
}

// Chatty (tduva): GET https://tduva.com/res/badges (bare JSON array). Currently only
// re-hosts FFZ badges, so it overlaps with the FFZ provider.
#[derive(Debug, Clone, Deserialize)]
struct ChattyBadge {
    id: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    meta_title: Option<String>,
    image_url: String,
    #[serde(default)]
    image_url_2: Option<String>,
    #[serde(default)]
    image_url_4: Option<String>,
    #[serde(default)]
    meta_url: Option<String>,
    #[serde(default)]
    userids: Vec<String>,
}

// DankChat (flex3r): GET https://flxrs.com/api/badges (bare JSON array). Single image
// url per badge (often an animated gif), `type` doubles as the name.
#[derive(Debug, Clone, Deserialize)]
struct DankChatBadge {
    #[serde(rename = "type")]
    badge_type: String,
    url: String,
    #[serde(default)]
    users: Vec<String>,
}

// Moltorino (MoltoBenne's Chatterino7 fork): GET https://api.moltorino.com/badges.
// One JSON snapshot of the whole supporter roster: `badges` is the tier list in
// TIER ORDER (developer, top_donor, founder, supporter) and each tier carries its
// holders. Moltorino's own client shows only the FIRST tier a user appears in,
// so a user gets at most one badge here too. Feed facts, asset sizes and the
// client rules are mirrored in the tier handling below.
//
// Every asset is an animated webp of 120-160 frames (1x 118 KB, 2x 353 KB,
// 3x 962 KB), which is why the chat row renders third-party badges at 2x.
const MOLTORINO_BADGES_URL: &str = "https://api.moltorino.com/badges";
/// The roster changes a few times a week, not a few times an hour, and the
/// feed is `no-store` with no ETag, so a refresh is always a full 20 KB read.
/// Six hours is plenty for a badge nobody is waiting on.
const MOLTORINO_REFRESH: Duration = Duration::from_secs(6 * 60 * 60);
/// Bounds copied from Moltorino's own parser so a runaway feed cannot balloon
/// the index: it caps at 64 tiers and 250,000 assignments.
const MOLTORINO_MAX_TIERS: usize = 64;
const MOLTORINO_MAX_ASSIGNMENTS: usize = 250_000;

#[derive(Debug, Clone, Deserialize)]
struct MoltorinoBadgesResponse {
    #[serde(default)]
    version: Option<i64>,
    #[serde(default)]
    badges: Vec<MoltorinoBadge>,
}

#[derive(Debug, Clone, Deserialize)]
struct MoltorinoBadge {
    id: String,
    #[serde(default)]
    tooltip: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    images: MoltorinoImages,
    #[serde(default)]
    users: Vec<MoltorinoUser>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct MoltorinoImages {
    #[serde(rename = "1x", default)]
    x1: String,
    #[serde(rename = "2x", default)]
    x2: String,
    #[serde(rename = "3x", default)]
    x3: String,
}

#[derive(Debug, Clone, Deserialize)]
struct MoltorinoUser {
    id: String,
}

impl MoltorinoBadge {
    /// Display title: the feed's tooltip, or the tier id when that is blank.
    fn title(&self) -> String {
        if self.tooltip.trim().is_empty() {
            self.id.clone()
        } else {
            self.tooltip.clone()
        }
    }

    /// (1x, 2x, 4x) URLs with the same fall-through the other providers use:
    /// a missing larger variant falls back to the next smaller one.
    fn image_urls(&self) -> Option<(String, String, String)> {
        let x1 = self.images.x1.trim();
        if x1.is_empty() {
            return None;
        }
        let x2 = if self.images.x2.trim().is_empty() { x1 } else { self.images.x2.trim() };
        let x3 = if self.images.x3.trim().is_empty() { x2 } else { self.images.x3.trim() };
        Some((x1.to_string(), x2.to_string(), x3.to_string()))
    }
}

/// Moltorino accepts only numeric Twitch ids of 1-32 digits; anything else in
/// the feed is a typo or a future key space and must not reach the index.
fn is_valid_moltorino_user_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}

/// One gallery tile per Moltorino tier, in tier order. Holder counts follow
/// the same one-badge-per-user rule as the chat index, so a user listed under
/// two tiers is counted (and flagged `owned`) only for the higher one.
fn moltorino_gallery_tiles(
    feed: &MoltorinoBadgesResponse,
    viewer_user_id: Option<&str>,
) -> Vec<ThirdPartyGalleryBadge> {
    let mut tiles = Vec::new();
    let mut assigned: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for badge in feed.badges.iter().take(MOLTORINO_MAX_TIERS) {
        let Some((x1, x2, x4)) = badge.image_urls() else {
            continue;
        };
        let mut user_count = 0usize;
        let mut owned = false;
        for user in &badge.users {
            let uid = user.id.trim();
            if !is_valid_moltorino_user_id(uid) || !assigned.insert(uid) {
                continue;
            }
            user_count += 1;
            if viewer_user_id == Some(uid) {
                owned = true;
            }
        }
        tiles.push(ThirdPartyGalleryBadge {
            id: format!("moltorino-{}", badge.id),
            provider: BadgeProvider::Moltorino,
            title: badge.title(),
            image_1x: x1,
            image_2x: x2,
            image_4x: x4,
            user_count,
            owned,
            click_url: Some("https://moltorino.com/".to_string()),
        });
    }
    tiles
}

// ============================================================================
// TWITCH HELIX STRUCTS
// ============================================================================

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelixBadgesResponse {
    pub data: Vec<HelixBadgeSet>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelixBadgeSet {
    pub set_id: String,
    pub versions: Vec<HelixBadgeVersion>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HelixBadgeVersion {
    pub id: String,
    pub image_url_1x: String,
    pub image_url_2x: String,
    pub image_url_4x: String,
    pub title: String,
    pub description: String,
    pub click_action: Option<String>,
    pub click_url: Option<String>,
}

// ============================================================================
// CACHE STRUCTURES
// ============================================================================

struct ThirdPartyCache {
    ffz: Option<FFZBadgesResponse>,
    bttv: Option<Vec<BttvBadge>>,
    chatterino: Option<ChatterinoBadgesResponse>,
    homies: Option<HomiesBadgesResponse>,
    chatsen: Option<Vec<ChatsenBadge>>,
    chatty: Option<Vec<ChattyBadge>>,
    dankchat: Option<Vec<DankChatBadge>>,
    moltorino: Option<MoltorinoBadgesResponse>,
    /// Moltorino refreshes on its own, slower clock (`MOLTORINO_REFRESH`);
    /// the seven feeds above share `last_updated`.
    moltorino_last_updated: SystemTime,
    /// Inverted index over every provider feed: user_id -> the badges that
    /// user holds. Rebuilt once per feed refresh so per-chatter lookups are a
    /// single HashMap get instead of a scan over every holder list. One
    /// Arc<UserBadge> exists per distinct badge, shared across its holders.
    by_user: HashMap<String, Vec<Arc<UserBadge>>>,
    last_updated: SystemTime,
}

impl ThirdPartyCache {
    /// Build the user_id -> badges index from the current provider feeds.
    /// Providers run in the same order the old per-chatter scan checked them
    /// (FFZ, BTTV, Chatterino, Homies, Moltorino, Chatsen, Chatty, DankChat), and within
    /// a provider in feed order, so each user's Vec preserves the exact badge
    /// order the scan produced. A holder is skipped when they already carry a
    /// badge with the same title (case-insensitive), which reproduces the
    /// scan's keep-first title dedupe (FFZ badges re-hosted by Chatty,
    /// duplicate per-user feed entries sharing one title).
    fn build_by_user_index(&self) -> HashMap<String, Vec<Arc<UserBadge>>> {
        let mut by_user: HashMap<String, Vec<Arc<UserBadge>>> = HashMap::new();

        fn push(
            by_user: &mut HashMap<String, Vec<Arc<UserBadge>>>,
            user_id: String,
            badge: &Arc<UserBadge>,
            title_lower: &str,
        ) {
            let entry = by_user.entry(user_id).or_default();
            if entry
                .iter()
                .any(|b| b.badge_info.title.to_lowercase() == title_lower)
            {
                return;
            }
            entry.push(Arc::clone(badge));
        }

        // FFZ. `users` is keyed by badge_id (as a string) -> [numeric user_id],
        // so the holder key is each numeric id rendered back to a String.
        if let Some(ffz) = &self.ffz {
            for badge in &ffz.badges {
                let image_url = badge
                    .urls
                    .get("4")
                    .or_else(|| badge.urls.get("2"))
                    .or_else(|| badge.urls.get("1"))
                    .cloned()
                    .unwrap_or_default();
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("ffz-{}", badge.id),
                        set_id: "ffz".to_string(),
                        version: badge.id.to_string(),
                        title: badge
                            .title
                            .clone()
                            .or_else(|| badge.name.clone())
                            .unwrap_or_else(|| format!("FFZ Badge {}", badge.id)),
                        description: String::new(),
                        image_1x: badge.urls.get("1").cloned().unwrap_or_default(),
                        image_2x: badge.urls.get("2").cloned().unwrap_or_default(),
                        image_4x: image_url,
                        click_action: None,
                        click_url: Some("https://www.frankerfacez.com/badges".to_string()),
                    },
                    provider: BadgeProvider::FFZ,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                if let Some(holders) = ffz.users.get(&badge.id.to_string()) {
                    for uid in holders {
                        push(&mut by_user, uid.to_string(), &arc, &title_lower);
                    }
                }
            }
        }

        // BetterTTV. One feed entry per holder; `provider_id` is the Twitch
        // user id and the SVG is the only image (no size variants).
        if let Some(bttv) = &self.bttv {
            for badge in bttv {
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("bttv-{}", badge.badge.description),
                        set_id: "bttv".to_string(),
                        version: "1".to_string(),
                        title: badge.badge.description.clone(),
                        description: String::new(),
                        image_1x: badge.badge.svg.clone(),
                        image_2x: badge.badge.svg.clone(),
                        image_4x: badge.badge.svg.clone(),
                        click_action: None,
                        click_url: Some("https://betterttv.com".to_string()),
                    },
                    provider: BadgeProvider::BTTV,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                push(&mut by_user, badge.provider_id.clone(), &arc, &title_lower);
            }
        }

        // Chatterino badges
        if let Some(chatterino) = &self.chatterino {
            for badge in &chatterino.badges {
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("chatterino-{}", badge.tooltip),
                        set_id: "chatterino".to_string(),
                        version: "1".to_string(),
                        title: badge.tooltip.clone(),
                        description: String::new(),
                        image_1x: badge.image1.clone(),
                        image_2x: badge.image2.clone().unwrap_or_else(|| badge.image1.clone()),
                        image_4x: badge
                            .image3
                            .clone()
                            .or_else(|| badge.image2.clone())
                            .unwrap_or_else(|| badge.image1.clone()),
                        click_action: None,
                        click_url: Some("https://chatterino.com/".to_string()),
                    },
                    provider: BadgeProvider::Chatterino,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                for uid in &badge.users {
                    push(&mut by_user, uid.clone(), &arc, &title_lower);
                }
            }
        }

        // Homies badges
        if let Some(homies) = &self.homies {
            for badge in &homies.badges {
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("homies-{}", badge.tooltip),
                        set_id: "homies".to_string(),
                        version: "1".to_string(),
                        title: badge.tooltip.clone(),
                        description: String::new(),
                        image_1x: badge.image1.clone(),
                        image_2x: badge.image2.clone().unwrap_or_else(|| badge.image1.clone()),
                        image_4x: badge
                            .image3
                            .clone()
                            .or_else(|| badge.image2.clone())
                            .unwrap_or_else(|| badge.image1.clone()),
                        click_action: None,
                        click_url: Some("https://chatterinohomies.com/".to_string()),
                    },
                    provider: BadgeProvider::Homies,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                for uid in &badge.users {
                    push(&mut by_user, uid.clone(), &arc, &title_lower);
                }
            }
        }

        // Moltorino supporter badges. Tiers arrive in tier order and a user
        // gets only the FIRST tier they appear in (Moltorino's own rule), so
        // one badge per holder even when the feed lists them under several.
        // Sits after Homies: both are Chatterino forks, and the title dedupe
        // above cannot collide ("Moltorino ..." titles are unique to this feed).
        if let Some(moltorino) = &self.moltorino {
            let mut assigned: std::collections::HashSet<&str> = std::collections::HashSet::new();
            let mut assignments = 0usize;
            for badge in moltorino.badges.iter().take(MOLTORINO_MAX_TIERS) {
                let Some((x1, x2, x4)) = badge.image_urls() else {
                    continue;
                };
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("moltorino-{}", badge.id),
                        set_id: "moltorino".to_string(),
                        version: "1".to_string(),
                        title: badge.title(),
                        description: badge.description.clone(),
                        image_1x: x1,
                        image_2x: x2,
                        image_4x: x4,
                        click_action: None,
                        click_url: Some("https://moltorino.com/".to_string()),
                    },
                    provider: BadgeProvider::Moltorino,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                for user in &badge.users {
                    if assignments >= MOLTORINO_MAX_ASSIGNMENTS {
                        break;
                    }
                    let uid = user.id.trim();
                    if !is_valid_moltorino_user_id(uid) || !assigned.insert(uid) {
                        continue;
                    }
                    assignments += 1;
                    push(&mut by_user, uid.to_string(), &arc, &title_lower);
                }
            }
        }

        // Chatsen badges
        if let Some(chatsen) = &self.chatsen {
            for badge in chatsen {
                let image = badge.mipmap.last().cloned().unwrap_or_default();
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("chatsen-{}", badge.id),
                        set_id: "chatsen".to_string(),
                        version: "1".to_string(),
                        title: badge.name.clone(),
                        description: String::new(),
                        image_1x: badge.mipmap.first().cloned().unwrap_or_default(),
                        image_2x: image.clone(),
                        image_4x: image,
                        click_action: None,
                        click_url: Some("https://chatsen.app".to_string()),
                    },
                    provider: BadgeProvider::Chatsen,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                for uid in &badge.users {
                    push(&mut by_user, uid.clone(), &arc, &title_lower);
                }
            }
        }

        // Chatty (tduva) badges
        if let Some(chatty) = &self.chatty {
            for badge in chatty {
                let image_4x = badge
                    .image_url_4
                    .clone()
                    .or_else(|| badge.image_url_2.clone())
                    .unwrap_or_else(|| badge.image_url.clone());
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!(
                            "chatty-{}-{}",
                            badge.id,
                            badge.version.clone().unwrap_or_default()
                        ),
                        set_id: "chatty".to_string(),
                        version: badge.version.clone().unwrap_or_else(|| "1".to_string()),
                        title: badge.meta_title.clone().unwrap_or_else(|| badge.id.clone()),
                        description: String::new(),
                        image_1x: badge.image_url.clone(),
                        image_2x: badge
                            .image_url_2
                            .clone()
                            .unwrap_or_else(|| badge.image_url.clone()),
                        image_4x,
                        click_action: None,
                        click_url: badge
                            .meta_url
                            .clone()
                            .or_else(|| Some("https://chatty.github.io".to_string())),
                    },
                    provider: BadgeProvider::Chatty,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                for uid in &badge.userids {
                    push(&mut by_user, uid.clone(), &arc, &title_lower);
                }
            }
        }

        // DankChat (flex3r) badges
        if let Some(dankchat) = &self.dankchat {
            for badge in dankchat {
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("dankchat-{}", badge.badge_type),
                        set_id: "dankchat".to_string(),
                        version: "1".to_string(),
                        title: badge.badge_type.clone(),
                        description: String::new(),
                        image_1x: badge.url.clone(),
                        image_2x: badge.url.clone(),
                        image_4x: badge.url.clone(),
                        click_action: None,
                        click_url: Some("https://github.com/flex3r/DankChat".to_string()),
                    },
                    provider: BadgeProvider::DankChat,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                for uid in &badge.users {
                    push(&mut by_user, uid.clone(), &arc, &title_lower);
                }
            }
        }

        by_user
    }
}

struct BadgeCache {
    global_badges: Option<HelixBadgesResponse>,
    channel_badges: LruCache<String, HelixBadgesResponse>,
    third_party: ThirdPartyCache,
    // Store user's last known badge string from IRC for profile lookups
    user_badge_strings: LruCache<String, String>,
}

impl BadgeCache {
    fn new() -> Self {
        Self {
            global_badges: None,
            channel_badges: LruCache::new(NonZeroUsize::new(50).unwrap()),
            third_party: ThirdPartyCache {
                ffz: None,
                bttv: None,
                chatterino: None,
                homies: None,
                chatsen: None,
                chatty: None,
                dankchat: None,
                moltorino: None,
                moltorino_last_updated: UNIX_EPOCH,
                by_user: HashMap::new(),
                last_updated: UNIX_EPOCH,
            },
            // Cache last badge string for up to 1000 users
            user_badge_strings: LruCache::new(NonZeroUsize::new(1000).unwrap()),
        }
    }
}

// ============================================================================
// BADGE SERVICE
// ============================================================================

pub struct BadgeService {
    cache: Arc<RwLock<BadgeCache>>,
    client_id: String,
    http_client: reqwest::Client,
    /// (drops token, Twitch user id that token belongs to). The global badge
    /// collection query (`ChatSettings_Badges`) only ever answers for the
    /// token's OWN user, so before attributing its result to a profile we
    /// confirm the profile is that user. Cached per token: one validate call
    /// per login, not one per profile open.
    drops_identity: RwLock<Option<(String, String)>>,
}

impl BadgeService {
    /// (channel badge sets, user badge strings) resident in the LRUs, or
    /// `None` while the cache is write-locked. Diagnostics for the resource line.
    pub fn cache_counts(&self) -> Option<(usize, usize)> {
        self.cache
            .try_read()
            .ok()
            .map(|c| (c.channel_badges.len(), c.user_badge_strings.len()))
    }


    pub fn new(client_id: String) -> Self {
        Self {
            cache: Arc::new(RwLock::new(BadgeCache::new())),
            client_id,
            drops_identity: RwLock::new(None),
            http_client: reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
        }
    }

    // ========================================================================
    // GLOBAL BADGES (Helix API)
    // ========================================================================

    pub async fn fetch_global_badges(&self, token: &str) -> Result<(), String> {
        let url = "https://api.twitch.tv/helix/chat/badges/global";
        let response = self
            .http_client
            .get(url)
            .header("Client-Id", &self.client_id)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch global badges: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Helix API returned status: {}", response.status()));
        }

        let badges: HelixBadgesResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse badges: {}", e))?;

        let mut cache = self.cache.write().await;
        cache.global_badges = Some(badges);

        Ok(())
    }

    pub async fn get_global_badges(&self) -> Option<HelixBadgesResponse> {
        let cache = self.cache.read().await;
        cache.global_badges.clone()
    }

    /// Cheap existence probe: no clone of the full Helix response.
    async fn has_global_badges(&self) -> bool {
        self.cache.read().await.global_badges.is_some()
    }

    // ========================================================================
    // CHANNEL BADGES (Helix API)
    // ========================================================================

    pub async fn fetch_channel_badges(&self, channel_id: &str, token: &str) -> Result<(), String> {
        let url = format!(
            "https://api.twitch.tv/helix/chat/badges?broadcaster_id={}",
            channel_id
        );

        let response = self
            .http_client
            .get(&url)
            .header("Client-Id", &self.client_id)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("Failed to fetch channel badges: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Helix API returned status: {}", response.status()));
        }

        let badges: HelixBadgesResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse badges: {}", e))?;

        let mut cache = self.cache.write().await;
        cache
            .channel_badges
            .put(channel_id.to_string(), badges);

        Ok(())
    }

    pub async fn get_channel_badges(&self, channel_id: &str) -> Option<HelixBadgesResponse> {
        let mut cache = self.cache.write().await;
        cache.channel_badges.get(channel_id).cloned()
    }

    /// Cheap existence probe: read lock + peek, no write lock, no entry clone.
    async fn has_channel_badges(&self, channel_id: &str) -> bool {
        self.cache.read().await.channel_badges.peek(channel_id).is_some()
    }

    // ========================================================================
    // THIRD PARTY BADGES
    // ========================================================================

    pub async fn fetch_third_party_badges(&self) -> Result<(), String> {
        let cache_duration = Duration::from_secs(10 * 60); // 10 minutes

        let mut cache = self.cache.write().await;

        // Check if cache is still valid
        if let Ok(elapsed) = cache.third_party.last_updated.elapsed() {
            if elapsed < cache_duration {
                return Ok(());
            }
        }

        // Moltorino rides this gate but on its own, slower clock: it is only
        // re-read once `MOLTORINO_REFRESH` has passed, at whichever 10-minute
        // boundary comes next.
        let moltorino_due = cache
            .third_party
            .moltorino_last_updated
            .elapsed()
            .map(|e| e >= MOLTORINO_REFRESH)
            .unwrap_or(true);

        drop(cache); // Release lock during network calls

        // Fetch FFZ badges
        let ffz_result = self
            .http_client
            .get("https://api.frankerfacez.com/v1/badges/ids")
            .send()
            .await;

        let ffz_badges = if let Ok(response) = ffz_result {
            if response.status().is_success() {
                response.json::<FFZBadgesResponse>().await.ok()
            } else {
                None
            }
        } else {
            None
        };

        // Fetch BetterTTV badges (bare array; one entry per user).
        let bttv_result = self
            .http_client
            .get("https://api.betterttv.net/3/cached/badges")
            .send()
            .await;
        let bttv_badges = if let Ok(response) = bttv_result {
            if response.status().is_success() {
                response.json::<Vec<BttvBadge>>().await.ok()
            } else {
                None
            }
        } else {
            None
        };

        // Fetch Chatterino badges
        let chatterino_result = self
            .http_client
            .get("https://api.chatterino.com/badges")
            .send()
            .await;

        let chatterino_badges = if let Ok(response) = chatterino_result {
            if response.status().is_success() {
                response.json::<ChatterinoBadgesResponse>().await.ok()
            } else {
                None
            }
        } else {
            None
        };

        // Fetch Homies badges (try both endpoints)
        let homies1_result = self
            .http_client
            .get("https://itzalex.github.io/badges")
            .send()
            .await;

        let homies2_result = self
            .http_client
            .get("https://itzalex.github.io/badges2")
            .send()
            .await;

        let homies_badges = Self::merge_homies_responses(homies1_result, homies2_result).await;

        // Fetch Chatsen badges (bare array; relies on the client's browser User-Agent
        // to avoid the 403 the endpoint returns for programmatic UAs).
        let chatsen_result = self
            .http_client
            .get("https://api.chatsen.app/account/badges")
            .send()
            .await;
        let chatsen_badges = if let Ok(response) = chatsen_result {
            if response.status().is_success() {
                response.json::<Vec<ChatsenBadge>>().await.ok()
            } else {
                None
            }
        } else {
            None
        };

        // Fetch Chatty (tduva) badges (bare array).
        let chatty_result = self
            .http_client
            .get("https://tduva.com/res/badges")
            .send()
            .await;
        let chatty_badges = if let Ok(response) = chatty_result {
            if response.status().is_success() {
                response.json::<Vec<ChattyBadge>>().await.ok()
            } else {
                None
            }
        } else {
            None
        };

        // Fetch DankChat (flex3r) badges (bare array).
        let dankchat_result = self
            .http_client
            .get("https://flxrs.com/api/badges")
            .send()
            .await;
        let dankchat_badges = if let Ok(response) = dankchat_result {
            if response.status().is_success() {
                response.json::<Vec<DankChatBadge>>().await.ok()
            } else {
                None
            }
        } else {
            None
        };

        // Fetch Moltorino supporter badges (single JSON snapshot, ~20 KB). A
        // hobby API run by one person: identify ourselves honestly instead of
        // the browser UA the client carries for Chatsen.
        let moltorino_badges = if moltorino_due {
            match self
                .http_client
                .get(MOLTORINO_BADGES_URL)
                .header(
                    "User-Agent",
                    concat!(
                        "StreamNook/",
                        env!("CARGO_PKG_VERSION"),
                        " (+https://streamnook.app)"
                    ),
                )
                .header("Accept", "application/json")
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    response.json::<MoltorinoBadgesResponse>().await.ok()
                }
                _ => None,
            }
        } else {
            None
        };

        // Update cache. A provider that failed this round (network, 5xx, a
        // body that no longer parses) keeps its LAST GOOD list instead of being
        // wiped to None: overwriting it made every badge from that provider
        // vanish from profiles and the picker for the whole cache window, with
        // nothing in the log to say why (seen 2026-09-05 as "not all my
        // third-party badges are there to select").
        let mut cache = self.cache.write().await;
        let mut failed: Vec<&str> = Vec::new();
        macro_rules! keep_last_good {
            ($field:ident, $fresh:expr, $name:literal) => {
                match $fresh {
                    Some(v) => cache.third_party.$field = Some(v),
                    None => {
                        if cache.third_party.$field.is_none() {
                            failed.push($name);
                        } else {
                            failed.push(concat!($name, " (kept previous)"));
                        }
                    }
                }
            };
        }
        keep_last_good!(ffz, ffz_badges, "ffz");
        keep_last_good!(bttv, bttv_badges, "bttv");
        keep_last_good!(chatterino, chatterino_badges, "chatterino");
        keep_last_good!(homies, homies_badges, "homies");
        keep_last_good!(chatsen, chatsen_badges, "chatsen");
        keep_last_good!(chatty, chatty_badges, "chatty");
        keep_last_good!(dankchat, dankchat_badges, "dankchat");
        // Moltorino keeps its last good roster too, but a failure must NOT
        // back-date the shared stamp below: that would re-read all seven other
        // feeds every minute for as long as one hobby API is down. Instead its
        // own clock is set so the next attempt lands at the next 10-minute gate.
        if moltorino_due {
            match moltorino_badges {
                Some(feed) => {
                    log::debug!(
                        "[BadgeService] Moltorino roster v{}: {} tiers, {} assignments",
                        feed.version.unwrap_or(-1),
                        feed.badges.len(),
                        feed.badges.iter().map(|b| b.users.len()).sum::<usize>()
                    );
                    cache.third_party.moltorino = Some(feed);
                    cache.third_party.moltorino_last_updated = SystemTime::now();
                }
                None => {
                    log::warn!(
                        "[BadgeService] Moltorino badge feed failed to refresh{}; retrying at the next gate",
                        if cache.third_party.moltorino.is_some() { " (kept previous)" } else { "" }
                    );
                    cache.third_party.moltorino_last_updated = SystemTime::now()
                        .checked_sub(MOLTORINO_REFRESH.saturating_sub(cache_duration))
                        .unwrap_or(UNIX_EPOCH);
                }
            }
        }
        // Rebuild the inverted per-user index once per refresh (~10 min) so
        // per-chatter lookups never scan the full holder lists.
        let by_user = cache.third_party.build_by_user_index();
        cache.third_party.by_user = by_user;
        if failed.is_empty() {
            cache.third_party.last_updated = SystemTime::now();
        } else {
            log::warn!(
                "[BadgeService] Third-party badge providers failed to refresh: {}; retrying in 60s",
                failed.join(", ")
            );
            // Back-date the stamp so the next lookup retries after a minute
            // instead of serving the stale set for the full window.
            cache.third_party.last_updated = SystemTime::now()
                .checked_sub(cache_duration.saturating_sub(Duration::from_secs(60)))
                .unwrap_or(UNIX_EPOCH);
        }

        Ok(())
    }

    async fn merge_homies_responses(
        result1: Result<reqwest::Response, reqwest::Error>,
        result2: Result<reqwest::Response, reqwest::Error>,
    ) -> Option<HomiesBadgesResponse> {
        let mut all_badges: Vec<HomiesBadge> = Vec::new();

        if let Ok(response) = result1 {
            if response.status().is_success() {
                if let Ok(data) = response.json::<HomiesBadgesResponse>().await {
                    all_badges.extend(data.badges);
                }
            }
        }

        if let Ok(response) = result2 {
            if response.status().is_success() {
                if let Ok(data) = response.json::<HomiesBadgesResponse>().await {
                    for badge in data.badges {
                        if !all_badges.iter().any(|b| b.tooltip == badge.tooltip) {
                            all_badges.push(badge);
                        } else if let Some(existing) =
                            all_badges.iter_mut().find(|b| b.tooltip == badge.tooltip)
                        {
                            existing.users.extend(badge.users);
                        }
                    }
                }
            }
        }

        if all_badges.is_empty() {
            None
        } else {
            Some(HomiesBadgesResponse { badges: all_badges })
        }
    }

    // ========================================================================
    // USER BADGE STRING CACHE (for profile lookups)
    // ========================================================================

    /// Store a user's badge string from IRC for later profile lookups
    pub async fn store_user_badge_string(&self, user_id: &str, badge_string: &str) {
        let mut cache = self.cache.write().await;
        cache
            .user_badge_strings
            .put(user_id.to_string(), badge_string.to_string());
    }

    /// Get a user's cached badge string. Read lock + peek: recency is already
    /// refreshed by `store_user_badge_string` on every IRC message, so losing
    /// read-side LRU promotion on a 1000-entry cache is a fine trade for not
    /// serializing every chatter lookup through the write lock.
    pub async fn get_user_badge_string(&self, user_id: &str) -> Option<String> {
        let cache = self.cache.read().await;
        cache.user_badge_strings.peek(user_id).cloned()
    }

    // ========================================================================
    // GQL FALLBACK (for fetching user's displayBadges when no IRC data)
    // ========================================================================

    /// Fetch a user's display badges and channel-earned badges from Twitch GQL
    /// (anonymous, inline query). Used when we have no IRC badge data for a user
    /// and for the profile overlay. Returns (display_badges, earned_badges) as
    /// "set_id/version" strings.
    async fn fetch_badges_from_gql(
        &self,
        user_id: &str,
        username: &str,
        channel_id: &str,
        channel_name: &str,
    ) -> Result<(Vec<String>, Vec<String>), String> {
        let request = BadgeLookupRequest {
            query: BADGE_LOOKUP_QUERY,
            variables: BadgeLookupVariables {
                id: user_id.to_string(),
                login: username.to_lowercase(),
                channel_id: channel_id.to_string(),
                channel_login: channel_name.to_lowercase(),
            },
        };

        let response = self
            .http_client
            .post("https://gql.twitch.tv/gql")
            .header("Accept-Language", "en-US")
            .header("Client-ID", env!("TWITCH_WEB_CLIENT_ID"))
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Failed to send GQL request: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "GQL request failed with status: {}",
                response.status()
            ));
        }

        let response_text = response
            .text()
            .await
            .map_err(|e| format!("Failed to read GQL response: {}", e))?;

        let gql_response: GQLResponse = serde_json::from_str(&response_text).map_err(|e| {
            format!(
                "Failed to parse GQL response: {} - Raw: {}",
                e,
                &response_text[..200.min(response_text.len())]
            )
        })?;

        let gql_data = match gql_response.data {
            Some(data) => data,
            None => {
                let messages: Vec<String> =
                    gql_response.errors.into_iter().map(|e| e.message).collect();
                return Err(format!(
                    "No data in GQL response (errors: {})",
                    messages.join("; ")
                ));
            }
        };

        // A partial error (e.g. an unrelated field failing an integrity check)
        // still ships the badge fields, so only surface it at debug.
        if !gql_response.errors.is_empty() {
            log::debug!(
                "[BadgeService] Badge lookup returned partial errors: {:?}",
                gql_response.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
            );
        }

        let to_ids = |badges: Option<Vec<GQLBadge>>| -> Vec<String> {
            badges
                .unwrap_or_default()
                .into_iter()
                .map(|b| format!("{}/{}", b.set_id, b.version))
                .collect()
        };

        let display_badges = to_ids(gql_data.user.and_then(|u| u.display_badges));
        let earned_badges = to_ids(gql_data.channel_viewer.and_then(|cv| cv.earned_badges));

        Ok((display_badges, earned_badges))
    }

    /// Resolve which Twitch user the current Drops token belongs to, cached per
    /// token so a re-login is picked up and a stable login costs one call.
    async fn drops_token_user_id(&self, token: &str) -> Result<String, String> {
        if let Some((cached_token, cached_user)) = self.drops_identity.read().await.as_ref() {
            if cached_token == token {
                return Ok(cached_user.clone());
            }
        }

        let response = self
            .http_client
            .get("https://id.twitch.tv/oauth2/validate")
            .header("Authorization", format!("OAuth {}", token))
            .send()
            .await
            .map_err(|e| format!("Failed to validate drops token: {}", e))?;

        if !response.status().is_success() {
            return Err(format!(
                "Drops token validation failed with status: {}",
                response.status()
            ));
        }

        let validation: TokenValidation = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse token validation: {}", e))?;

        if validation.user_id.is_empty() {
            return Err("Token validation returned no user_id".to_string());
        }

        *self.drops_identity.write().await =
            Some((token.to_string(), validation.user_id.clone()));
        Ok(validation.user_id)
    }

    /// Fetch ALL global badges the SIGNED-IN user has earned, as
    /// "set_id/version" strings.
    ///
    /// Twitch removed `user.globalBadgeCollection` from its GQL schema
    /// (observed 2026-09-05), so the only remaining source for the full global
    /// collection is the badge picker's own `ChatSettings_Badges` query, which
    /// answers for the token's user and nobody else. It rides the Drops token
    /// (Android client) the way chat_identity.rs already does. Errors out,
    /// rather than returning someone else's badges, when `user_id` is not the
    /// token's user or when no Drops token is stored.
    pub async fn fetch_current_user_global_collection(
        &self,
        user_id: &str,
        username: &str,
    ) -> Result<Vec<String>, String> {
        let token = crate::services::drops_auth_service::DropsAuthService::get_token()
            .await
            .map_err(|e| format!("No drops token for badge collection: {}", e))?;

        let token_user = self.drops_token_user_id(&token).await?;
        if token_user != user_id {
            return Err(format!(
                "Global badge collection is only readable for the signed-in user (token user {}, requested {})",
                token_user, user_id
            ));
        }

        let ids = crate::commands::chat_identity::fetch_badge_collection_ids(username, &token)
            .await
            .map_err(|e| {
                log::warn!(
                    "[BadgeService] ChatSettings_Badges collection fetch failed for the signed-in user: {}",
                    e
                );
                e
            })?;

        log::debug!(
            "[BadgeService] Fetched {} global earned badges for the signed-in user",
            ids.len()
        );

        Ok(ids)
    }

    /// Fetch ALL earned badges from both channel-specific and global sources
    /// This merges channelViewer.earnedBadges with the signed-in user's global
    /// badge collection for complete coverage. For any OTHER user only the
    /// channel-earned set is available (Twitch no longer exposes third-party
    /// global collections).
    /// NOTE: This is only used for profile overlays, not for normal chat!
    async fn fetch_all_earned_badges(
        &self,
        channel_earned_ids: Vec<String>,
        user_id: &str,
        username: &str,
        channel_id: &str,
    ) -> Vec<UserBadge> {
        let mut all_badge_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 1. Use already-fetched channel-specific earned badges
        for badge_id in channel_earned_ids {
            all_badge_ids.insert(badge_id);
        }

        // 2. Fetch the global badge collection (signed-in user only)
        match self
            .fetch_current_user_global_collection(user_id, username)
            .await
        {
            Ok(global_badge_ids) => {
                for badge_id in global_badge_ids {
                    all_badge_ids.insert(badge_id);
                }
            }
            Err(e) => {
                // Expected for other users' profiles (not the token's user), so
                // this stays at debug; a failure for the signed-in user is
                // logged at WARN by fetch_current_user_global_collection.
                log::debug!(
                    "[BadgeService] Global badge collection unavailable for {}: {}",
                    username,
                    e
                );
            }
        }

        // 3. Resolve all unique badge IDs to full badge info
        let badge_ids: Vec<String> = all_badge_ids.into_iter().collect();
        if badge_ids.is_empty() {
            return Vec::new();
        }

        // Resolve using both channel and global badges
        self.resolve_badge_ids(&badge_ids, channel_id).await
    }

    // ========================================================================
    // USER BADGES LOOKUP (Helix + GQL fallback)
    // ========================================================================

    /// Get all badges for a user (for normal chat - display badges only)
    /// Priority: IRC cached badges -> GQL ViewerCard badges -> empty
    pub async fn get_user_badges(
        &self,
        user_id: &str,
        username: &str,
        channel_id: &str,
        channel_name: &str,
        token: &str,
    ) -> Result<UserBadgesResponse, String> {
        // Ensure badge metadata is fetched
        if !self.has_global_badges().await {
            self.fetch_global_badges(token).await?;
        }

        if !self.has_channel_badges(channel_id).await {
            self.fetch_channel_badges(channel_id, token).await?;
        }

        // Try to get user's badge string from IRC cache first
        let badge_string = self.get_user_badge_string(user_id).await;

        // Resolve badges - use IRC cache if available, otherwise try GQL
        let display_badges = if let Some(badge_str) = badge_string {
            self.resolve_badge_string(&badge_str, channel_id).await
        } else {
            // No IRC data - try GQL fallback
            match self
                .fetch_badges_from_gql(user_id, username, channel_id, channel_name)
                .await
            {
                Ok((display_ids, _)) => {
                    if !display_ids.is_empty() {
                        // Store display in cache for future use
                        let badge_str = display_ids.join(",");
                        self.store_user_badge_string(user_id, &badge_str).await;
                        self.resolve_badge_ids(&display_ids, channel_id).await
                    } else {
                        Vec::new()
                    }
                }
                Err(_) => Vec::new(),
            }
        };

        // Fetch third-party badges (cached, fast)
        self.fetch_third_party_badges().await?;
        let third_party_badges = self.get_third_party_badges_for_user(user_id).await;

        // For normal chat, earned badges = display badges (no extra queries needed)
        Ok(UserBadgesResponse {
            display_badges: display_badges.clone(),
            earned_badges: display_badges,
            third_party_badges,
        })
    }

    /// Get all badges for a user INCLUDING full earned badge collection (for profile overlay)
    /// This makes additional queries to fetch ALL earned badges
    pub async fn get_user_badges_with_earned(
        &self,
        user_id: &str,
        username: &str,
        channel_id: &str,
        channel_name: &str,
        token: &str,
    ) -> Result<UserBadgesResponse, String> {
        // Ensure badge metadata is fetched
        if !self.has_global_badges().await {
            self.fetch_global_badges(token).await?;
        }

        if !self.has_channel_badges(channel_id).await {
            self.fetch_channel_badges(channel_id, token).await?;
        }

        // Fetch display badges and channel-earned badges from GQL
        let (display_ids, channel_earned_ids) = match self
            .fetch_badges_from_gql(user_id, username, channel_id, channel_name)
            .await
        {
            Ok(result) => result,
            Err(e) => {
                log::warn!(
                    "[BadgeService] GQL badge lookup failed for {} in {}: {}",
                    username,
                    channel_name,
                    e
                );
                // Fall back to IRC cache if available
                if let Some(badge_str) = self.get_user_badge_string(user_id).await {
                    let display = self.resolve_badge_string(&badge_str, channel_id).await;
                    return Ok(UserBadgesResponse {
                        display_badges: display.clone(),
                        earned_badges: display.clone(),
                        third_party_badges: self.get_third_party_badges_for_user(user_id).await,
                    });
                }
                return Ok(UserBadgesResponse {
                    display_badges: Vec::new(),
                    earned_badges: Vec::new(),
                    third_party_badges: Vec::new(),
                });
            }
        };

        // Resolve display badges
        let display_badges = if !display_ids.is_empty() {
            self.resolve_badge_ids(&display_ids, channel_id).await
        } else {
            Vec::new()
        };

        // Fetch all earned badges (merges channel + global)
        let earned_badges = self
            .fetch_all_earned_badges(channel_earned_ids, user_id, username, channel_id)
            .await;

        // Fetch third-party badges
        self.fetch_third_party_badges().await?;
        let third_party_badges = self.get_third_party_badges_for_user(user_id).await;

        Ok(UserBadgesResponse {
            display_badges,
            earned_badges,
            third_party_badges,
        })
    }

    /// Resolve a badge string (e.g., "subscriber/12,premium/1") to full badge info
    pub async fn resolve_badge_string(
        &self,
        badge_string: &str,
        channel_id: &str,
    ) -> Vec<UserBadge> {
        let badge_ids = self.parse_badge_string(badge_string);
        self.resolve_badge_ids(&badge_ids, channel_id).await
    }

    async fn resolve_badge_ids(&self, badge_ids: &[String], channel_id: &str) -> Vec<UserBadge> {
        let cache = self.cache.read().await;
        let mut resolved = Vec::new();

        for badge_id in badge_ids {
            let parts: Vec<&str> = badge_id.split('/').collect();
            if parts.len() != 2 {
                continue;
            }

            let set_id = parts[0];
            let version = parts[1];

            // Try channel badges first (subscriber, bits, etc.)
            if let Some(channel_badges) = cache.channel_badges.peek(channel_id) {
                if let Some(badge_info) =
                    Self::find_badge_in_response(channel_badges, set_id, version)
                {
                    resolved.push(UserBadge {
                        badge_info,
                        provider: BadgeProvider::Twitch,
                    });
                    continue;
                }
            }

            // Fall back to global badges
            if let Some(global_badges) = &cache.global_badges {
                if let Some(badge_info) =
                    Self::find_badge_in_response(global_badges, set_id, version)
                {
                    resolved.push(UserBadge {
                        badge_info,
                        provider: BadgeProvider::Twitch,
                    });
                }
            }
        }

        resolved
    }

    /// Resolve badge IDs using only global badges (for earned badge collection)
    /// The global badge collection only contains global badges, not channel-specific ones
    async fn resolve_badge_ids_global_only(&self, badge_ids: &[String]) -> Vec<UserBadge> {
        let cache = self.cache.read().await;
        let mut resolved = Vec::new();

        for badge_id in badge_ids {
            let parts: Vec<&str> = badge_id.split('/').collect();
            if parts.len() != 2 {
                continue;
            }

            let set_id = parts[0];
            let version = parts[1];

            // Only look up global badges for earned badges
            if let Some(global_badges) = &cache.global_badges {
                if let Some(badge_info) =
                    Self::find_badge_in_response(global_badges, set_id, version)
                {
                    resolved.push(UserBadge {
                        badge_info,
                        provider: BadgeProvider::Twitch,
                    });
                }
            }
        }

        resolved
    }

    fn find_badge_in_response(
        response: &HelixBadgesResponse,
        set_id: &str,
        version: &str,
    ) -> Option<BadgeInfo> {
        for badge_set in &response.data {
            if badge_set.set_id == set_id {
                for badge_version in &badge_set.versions {
                    if badge_version.id == version {
                        return Some(BadgeInfo {
                            id: format!("{}/{}", set_id, version),
                            set_id: set_id.to_string(),
                            version: version.to_string(),
                            title: badge_version.title.clone(),
                            description: badge_version.description.clone(),
                            image_1x: badge_version.image_url_1x.clone(),
                            image_2x: badge_version.image_url_2x.clone(),
                            image_4x: badge_version.image_url_4x.clone(),
                            click_action: badge_version.click_action.clone(),
                            click_url: badge_version.click_url.clone(),
                        });
                    }
                }
            }
        }
        None
    }

    /// Resolve ONLY a user's real chat-client (third-party) badges from the
    /// already-prefetched provider databases. Pure in-memory cache lookup: no
    /// Twitch GQL, no token, no network round-trip, so it is safe to call once
    /// per chatter in the live chat path. Display/earned are left empty (chat
    /// renders Twitch badges straight from IRC tags). BTTV *Pro* loyalty badges
    /// are intentionally NOT included here: they live behind a per-user live
    /// socket lookup, the one thing that would reintroduce a per-chatter network
    /// cost, so Pro stays an opt-in identity badge resolved elsewhere.
    pub async fn get_third_party_badges_only(&self, user_id: &str) -> UserBadgesResponse {
        UserBadgesResponse {
            display_badges: Vec::new(),
            earned_badges: Vec::new(),
            third_party_badges: self.get_third_party_badges_for_user(user_id).await,
        }
    }

    async fn get_third_party_badges_for_user(&self, user_id: &str) -> Vec<UserBadge> {
        // Single lookup in the inverted index built at feed-refresh time (see
        // ThirdPartyCache::build_by_user_index). Provider order and the
        // duplicate-title collapse are baked into the index, so this is just a
        // clone-out of the user's shared Arc entries.
        let cache = self.cache.read().await;
        cache
            .third_party
            .by_user
            .get(user_id)
            .map(|arcs| arcs.iter().map(|arc| (**arc).clone()).collect())
            .unwrap_or_default()
    }

    /// Build the full distinct badge set for every third-party provider, for the
    /// browse gallery (one entry per distinct badge, not per user). When
    /// `viewer_user_id` is provided, each badge is flagged `owned` if that user
    /// has it, and `user_count` reports how many users carry it.
    pub async fn get_all_third_party_badges(
        &self,
        viewer_user_id: Option<&str>,
    ) -> Vec<ThirdPartyGalleryBadge> {
        let cache = self.cache.read().await;
        let mut out: Vec<ThirdPartyGalleryBadge> = Vec::new();

        // FFZ. `users` is keyed by badge_id (as a string) -> [numeric user_id].
        if let Some(ffz) = &cache.third_party.ffz {
            let viewer_num = viewer_user_id.and_then(|uid| uid.parse::<u32>().ok());
            for badge in &ffz.badges {
                let holders = ffz.users.get(&badge.id.to_string());
                let user_count = holders.map(|h| h.len()).unwrap_or(0);
                let owned = match (viewer_num, holders) {
                    (Some(n), Some(h)) => h.contains(&n),
                    _ => false,
                };
                let img_1x = badge.urls.get("1").cloned().unwrap_or_default();
                let img_2x = badge
                    .urls
                    .get("2")
                    .cloned()
                    .unwrap_or_else(|| img_1x.clone());
                let img_4x = badge
                    .urls
                    .get("4")
                    .cloned()
                    .unwrap_or_else(|| img_2x.clone());
                out.push(ThirdPartyGalleryBadge {
                    id: format!("ffz-{}", badge.id),
                    provider: BadgeProvider::FFZ,
                    title: badge
                        .title
                        .clone()
                        .or_else(|| badge.name.clone())
                        .unwrap_or_else(|| format!("FFZ Badge {}", badge.id)),
                    image_1x: img_1x,
                    image_2x: img_2x,
                    image_4x: img_4x,
                    user_count,
                    owned,
                    click_url: Some("https://www.frankerfacez.com/badges".to_string()),
                });
            }
        }

        // BetterTTV. One entry per holder; the (provider, title) dedupe below
        // collapses the ~160 entries into one tile per distinct badge type and
        // sums the holder counts.
        if let Some(bttv) = &cache.third_party.bttv {
            for badge in bttv {
                let owned = viewer_user_id
                    .map(|uid| badge.provider_id == uid)
                    .unwrap_or(false);
                out.push(ThirdPartyGalleryBadge {
                    id: format!("bttv-{}", badge.badge.description),
                    provider: BadgeProvider::BTTV,
                    title: badge.badge.description.clone(),
                    image_1x: badge.badge.svg.clone(),
                    image_2x: badge.badge.svg.clone(),
                    image_4x: badge.badge.svg.clone(),
                    user_count: 1,
                    owned,
                    click_url: Some("https://betterttv.com".to_string()),
                });
            }
        }

        // Chatterino
        if let Some(chatterino) = &cache.third_party.chatterino {
            for badge in &chatterino.badges {
                let owned = viewer_user_id
                    .map(|uid| badge.users.iter().any(|u| u == uid))
                    .unwrap_or(false);
                out.push(ThirdPartyGalleryBadge {
                    id: format!("chatterino-{}", badge.tooltip),
                    provider: BadgeProvider::Chatterino,
                    title: badge.tooltip.clone(),
                    image_1x: badge.image1.clone(),
                    image_2x: badge.image2.clone().unwrap_or_else(|| badge.image1.clone()),
                    image_4x: badge
                        .image3
                        .clone()
                        .or_else(|| badge.image2.clone())
                        .unwrap_or_else(|| badge.image1.clone()),
                    user_count: badge.users.len(),
                    owned,
                    click_url: Some("https://chatterino.com/".to_string()),
                });
            }
        }

        // Homies
        if let Some(homies) = &cache.third_party.homies {
            for badge in &homies.badges {
                let owned = viewer_user_id
                    .map(|uid| badge.users.iter().any(|u| u == uid))
                    .unwrap_or(false);
                out.push(ThirdPartyGalleryBadge {
                    id: format!("homies-{}", badge.tooltip),
                    provider: BadgeProvider::Homies,
                    title: badge.tooltip.clone(),
                    image_1x: badge.image1.clone(),
                    image_2x: badge.image2.clone().unwrap_or_else(|| badge.image1.clone()),
                    image_4x: badge
                        .image3
                        .clone()
                        .or_else(|| badge.image2.clone())
                        .unwrap_or_else(|| badge.image1.clone()),
                    user_count: badge.users.len(),
                    owned,
                    click_url: Some("https://chatterinohomies.com/".to_string()),
                });
            }
        }

        // Chatsen
        if let Some(chatsen) = &cache.third_party.chatsen {
            for badge in chatsen {
                let owned = viewer_user_id
                    .map(|uid| badge.users.iter().any(|u| u == uid))
                    .unwrap_or(false);
                let img_1x = badge.mipmap.first().cloned().unwrap_or_default();
                let img_4x = badge
                    .mipmap
                    .last()
                    .cloned()
                    .unwrap_or_else(|| img_1x.clone());
                out.push(ThirdPartyGalleryBadge {
                    id: format!("chatsen-{}", badge.id),
                    provider: BadgeProvider::Chatsen,
                    title: badge.name.clone(),
                    image_1x: img_1x,
                    image_2x: img_4x.clone(),
                    image_4x: img_4x,
                    user_count: badge.users.len(),
                    owned,
                    click_url: Some("https://chatsen.app".to_string()),
                });
            }
        }

        // Chatty (tduva)
        if let Some(chatty) = &cache.third_party.chatty {
            for badge in chatty {
                let owned = viewer_user_id
                    .map(|uid| badge.userids.iter().any(|u| u == uid))
                    .unwrap_or(false);
                let img_4x = badge
                    .image_url_4
                    .clone()
                    .or_else(|| badge.image_url_2.clone())
                    .unwrap_or_else(|| badge.image_url.clone());
                out.push(ThirdPartyGalleryBadge {
                    id: format!(
                        "chatty-{}-{}",
                        badge.id,
                        badge.version.clone().unwrap_or_default()
                    ),
                    provider: BadgeProvider::Chatty,
                    title: badge.meta_title.clone().unwrap_or_else(|| badge.id.clone()),
                    image_1x: badge.image_url.clone(),
                    image_2x: badge
                        .image_url_2
                        .clone()
                        .unwrap_or_else(|| badge.image_url.clone()),
                    image_4x: img_4x,
                    user_count: badge.userids.len(),
                    owned,
                    click_url: badge
                        .meta_url
                        .clone()
                        .or_else(|| Some("https://chatty.github.io".to_string())),
                });
            }
        }

        // DankChat (flex3r)
        if let Some(dankchat) = &cache.third_party.dankchat {
            for badge in dankchat {
                let owned = viewer_user_id
                    .map(|uid| badge.users.iter().any(|u| u == uid))
                    .unwrap_or(false);
                out.push(ThirdPartyGalleryBadge {
                    id: format!("dankchat-{}", badge.badge_type),
                    provider: BadgeProvider::DankChat,
                    title: badge.badge_type.clone(),
                    image_1x: badge.url.clone(),
                    image_2x: badge.url.clone(),
                    image_4x: badge.url.clone(),
                    user_count: badge.users.len(),
                    owned,
                    click_url: Some("https://github.com/flex3r/DankChat".to_string()),
                });
            }
        }

        // Moltorino
        if let Some(moltorino) = &cache.third_party.moltorino {
            out.extend(moltorino_gallery_tiles(moltorino, viewer_user_id));
        }

        // Collapse entries that share a (provider, title) into one tile. Some feeds
        // (notably Chatty's FFZ re-host) emit one entry PER USER under the same title
        // (e.g. 122x "FFZ:AP Supporter"), which would otherwise flood the gallery.
        // Merging sums the user counts and keeps the badge "owned" if any matched.
        let mut deduped: Vec<ThirdPartyGalleryBadge> = Vec::with_capacity(out.len());
        let mut index: HashMap<(String, String), usize> = HashMap::new();
        for badge in out {
            let key = (badge.provider.as_key().to_string(), badge.title.clone());
            if let Some(&i) = index.get(&key) {
                deduped[i].user_count += badge.user_count;
                deduped[i].owned = deduped[i].owned || badge.owned;
            } else {
                index.insert(key, deduped.len());
                deduped.push(badge);
            }
        }

        deduped
    }

    // ========================================================================
    // BADGE PARSING
    // ========================================================================

    pub fn parse_badge_string(&self, badge_string: &str) -> Vec<String> {
        if badge_string.is_empty() {
            return Vec::new();
        }

        badge_string
            .split(',')
            .filter_map(|badge| {
                let parts: Vec<&str> = badge.split('/').collect();
                if parts.len() == 2 {
                    Some(format!("{}/{}", parts[0], parts[1]))
                } else {
                    None
                }
            })
            .collect()
    }

    // ========================================================================
    // CACHE MANAGEMENT
    // ========================================================================

    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.global_badges = None;
        cache.channel_badges.clear();
        cache.third_party.ffz = None;
        cache.third_party.bttv = None;
        cache.third_party.chatterino = None;
        cache.third_party.homies = None;
        cache.third_party.chatsen = None;
        cache.third_party.chatty = None;
        cache.third_party.dankchat = None;
        cache.third_party.by_user.clear();
        cache.third_party.last_updated = UNIX_EPOCH;
        cache.user_badge_strings.clear();
    }

    pub async fn clear_channel_cache(&self, channel_id: &str) {
        let mut cache = self.cache.write().await;
        cache.channel_badges.pop(channel_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_user_index_resolves_ffz_holder() {
        let mut users = HashMap::new();
        // ffz.users is keyed by badge_id (string) -> numeric holder ids
        users.insert("3".to_string(), vec![11111u32]);

        let mut urls = HashMap::new();
        urls.insert("1".to_string(), "https://example.test/badge/1".to_string());

        let third_party = ThirdPartyCache {
            ffz: Some(FFZBadgesResponse {
                badges: vec![FFZBadge {
                    id: 3,
                    title: Some("Developer".to_string()),
                    name: None,
                    urls,
                }],
                users,
            }),
            bttv: None,
            chatterino: None,
            homies: None,
            chatsen: None,
            chatty: None,
            dankchat: None,
            moltorino: None,
            moltorino_last_updated: UNIX_EPOCH,
            by_user: HashMap::new(),
            last_updated: UNIX_EPOCH,
        };

        let index = third_party.build_by_user_index();

        let held = index.get("11111").expect("holder should resolve");
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].badge_info.id, "ffz-3");
        assert_eq!(held[0].badge_info.title, "Developer");
        assert_eq!(held[0].provider, BadgeProvider::FFZ);

        assert!(index.get("99999").is_none());
    }

    /// A trimmed copy of the live feed (version 369, 2026-09-07) with the
    /// shapes that matter: tier order, a holder listed under two tiers, a
    /// non-numeric id, and a tier with no images.
    const MOLTORINO_FIXTURE: &str = r#"{
      "version": 369,
      "generatedAt": "2026-09-07T00:12:49.620Z",
      "badges": [
        {
          "id": "developer",
          "tooltip": "Moltorino Developer",
          "description": "Given to people who help build and maintain Moltorino.",
          "images": {
            "1x": "https://api.moltorino.com/badges/assets/developer/1x.webp?v=369",
            "2x": "https://api.moltorino.com/badges/assets/developer/2x.webp?v=369",
            "3x": "https://api.moltorino.com/badges/assets/developer/3x.webp?v=369"
          },
          "users": [ { "id": "506954718", "username": "moltobenne_" } ]
        },
        {
          "id": "broken",
          "tooltip": "No Images",
          "images": {},
          "users": [ { "id": "777" } ]
        },
        {
          "id": "supporter",
          "tooltip": "Moltorino Supporter",
          "description": "Thanks for supporting Moltorino.",
          "images": {
            "1x": "https://api.moltorino.com/badges/assets/supporter/1x.webp?v=369",
            "2x": "https://api.moltorino.com/badges/assets/supporter/2x.webp?v=369",
            "3x": "https://api.moltorino.com/badges/assets/supporter/3x.webp?v=369"
          },
          "users": [
            { "id": "506954718", "username": "moltobenne_" },
            { "id": "249031143", "username": "br_winters" },
            { "id": "not-a-number", "username": "typo" }
          ]
        }
      ],
      "users": [ { "id": "1051860673", "username": "omarbasilz", "decorations": false } ]
    }"#;

    fn moltorino_cache(feed: MoltorinoBadgesResponse) -> ThirdPartyCache {
        ThirdPartyCache {
            ffz: None,
            bttv: None,
            chatterino: None,
            homies: None,
            chatsen: None,
            chatty: None,
            dankchat: None,
            moltorino: Some(feed),
            moltorino_last_updated: UNIX_EPOCH,
            by_user: HashMap::new(),
            last_updated: UNIX_EPOCH,
        }
    }

    #[test]
    fn moltorino_feed_parses_with_unknown_fields() {
        let feed: MoltorinoBadgesResponse =
            serde_json::from_str(MOLTORINO_FIXTURE).expect("feed should parse");
        assert_eq!(feed.version, Some(369));
        assert_eq!(feed.badges.len(), 3);
        assert_eq!(feed.badges[0].users[0].id, "506954718");
    }

    #[test]
    fn moltorino_holder_gets_one_badge_highest_tier_first() {
        let feed: MoltorinoBadgesResponse = serde_json::from_str(MOLTORINO_FIXTURE).unwrap();
        let index = moltorino_cache(feed).build_by_user_index();

        // Listed under developer AND supporter: only the first tier sticks.
        let dev = index.get("506954718").expect("developer should resolve");
        assert_eq!(dev.len(), 1);
        assert_eq!(dev[0].badge_info.id, "moltorino-developer");
        assert_eq!(dev[0].badge_info.title, "Moltorino Developer");
        assert_eq!(dev[0].provider, BadgeProvider::Moltorino);
        assert_eq!(
            dev[0].badge_info.image_4x,
            "https://api.moltorino.com/badges/assets/developer/3x.webp?v=369"
        );

        let sup = index.get("249031143").expect("supporter should resolve");
        assert_eq!(sup.len(), 1);
        assert_eq!(sup[0].badge_info.id, "moltorino-supporter");

        // A tier with no images never reaches the index; nor does a bad id.
        assert!(index.get("777").is_none());
        assert!(index.get("not-a-number").is_none());
    }

    #[test]
    fn moltorino_gallery_counts_holders_and_ownership() {
        let feed: MoltorinoBadgesResponse = serde_json::from_str(MOLTORINO_FIXTURE).unwrap();
        let tiles = moltorino_gallery_tiles(&feed, Some("249031143"));
        let ids: Vec<&str> = tiles.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["moltorino-developer", "moltorino-supporter"]);
        assert_eq!(tiles[0].user_count, 1);
        assert!(!tiles[0].owned);
        // The duplicate holder and the bad id are not counted.
        assert_eq!(tiles[1].user_count, 1);
        assert!(tiles[1].owned);
    }

    #[test]
    fn moltorino_provider_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&BadgeProvider::Moltorino).unwrap(),
            "\"moltorino\""
        );
        // The Debug form differs, which is exactly why `as_key` exists: shipping
        // this string to the page gave the profile card a second spelling of
        // every provider. See BadgeProvider::as_key.
        assert_eq!(format!("{:?}", BadgeProvider::Moltorino), "Moltorino");
    }

    /// `as_key` is the id the page, the loadout keys and the overlay toggles all
    /// match on, so it must stay identical to what serde puts on the wire. A new
    /// variant that forgets an arm here (or spells it in PascalCase) fails this
    /// instead of silently dropping that provider's badges on one surface.
    #[test]
    fn provider_key_matches_the_serde_wire_form() {
        for provider in [
            BadgeProvider::Twitch,
            BadgeProvider::FFZ,
            BadgeProvider::BTTV,
            BadgeProvider::Chatterino,
            BadgeProvider::Homies,
            BadgeProvider::Chatsen,
            BadgeProvider::Chatty,
            BadgeProvider::DankChat,
            BadgeProvider::Moltorino,
        ] {
            let wire = serde_json::to_string(&provider).unwrap();
            assert_eq!(
                format!("\"{}\"", provider.as_key()),
                wire,
                "as_key drifted from the serde form for {:?}",
                provider
            );
        }
    }
}
