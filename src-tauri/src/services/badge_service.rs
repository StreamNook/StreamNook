use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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

// The global badge a user chose on Twitch, the one chat shows in every channel.
// Not `displayBadges` on some channel: Twitch's own channel swaps in its event
// badges (GlitchCon for someone who wears another badge everywhere else).
const SELECTED_BADGE_QUERY: &str =
    "query StreamNookSelectedBadge($id: ID!) { user(id: $id) { selectedBadge { title imageURL(size: QUADRUPLE) } } }";

/// How long a user's selected badge is reused before Twitch is asked again.
const SELECTED_BADGE_TTL: Duration = Duration::from_secs(300);

/// A user's selected Twitch badge, as a profile card draws it.
#[derive(Debug, Clone, Serialize)]
pub struct SelectedTwitchBadge {
    pub src: String,
    pub title: String,
}

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

/// The one order every surface draws third-party badges in, by provider key:
/// the order the chat index below walks its feeds in. A member's loadout
/// chooses which badges show, never where they sit, so every chatter's badges
/// line up alike. The site's `PROVIDER_ORDER` is the same list.
pub const THIRD_PARTY_ORDER: [&str; 8] = ["ffz", "bttv", "chatterino", "homies", "moltorino", "chatsen", "chatty", "dankchat"];

/// A provider key's place in `THIRD_PARTY_ORDER`; an unknown provider sorts
/// last and is never dropped on that account.
pub fn third_party_rank(provider_key: &str) -> usize {
    THIRD_PARTY_ORDER.iter().position(|p| *p == provider_key).unwrap_or(THIRD_PARTY_ORDER.len())
}

/// Where a badge sits on every surface that lines badges up (chat rows, the
/// You card), by provider key: the platform's own (Twitch, Kick, YouTube,
/// TikTok; an empty key is Twitch) first, then 7TV, then the chat clients in
/// `THIRD_PARTY_ORDER`, and a member's StreamNook badge last. Sort stably by
/// it, so badges of one provider keep their own order.
pub fn surface_rank(provider_key: &str) -> usize {
    match provider_key {
        "" | "twitch" | "kick" | "youtube" | "tiktok" => 0,
        "7tv" => 1,
        "streamnook" => 3 + THIRD_PARTY_ORDER.len(),
        other => 2 + third_party_rank(other),
    }
}

/// A third-party badge's `set_id` is its WEAR CATEGORY: what the provider
/// itself treats as one separate thing. A chatter wears at most one badge per
/// (provider, category) (`worn_third_party`), and the frontend receives it as
/// the badge's `category` so the pickers and the member bundle follow the
/// same rule.
///
/// - Homies: three, each drawn on its own by Homies' client: the custom badge
///   (`HOMIES_CUSTOM_SET`), the `/badges` role list (`HOMIES_ROLES_SET`) and
///   the `/badges2` staff list (`HOMIES_STAFF_SET`). A Homies holder wears up
///   to three.
/// - FFZ: its badge slot (`ffz_wear_set`); FFZ draws one badge per slot.
/// - Every other provider: the provider's own id, so one badge.
const HOMIES_CUSTOM_SET: &str = "homies-custom";
const HOMIES_ROLES_SET: &str = "homies";
const HOMIES_STAFF_SET: &str = "homies-staff";

/// An FFZ badge's wear category: its slot, or plain `ffz` when the feed names
/// none.
fn ffz_wear_set(slot: Option<u32>) -> String {
    slot.map(|s| format!("ffz-slot-{s}")).unwrap_or_else(|| "ffz".to_string())
}

/// What a chatter WEARS in chat: at most one badge per wear category (see
/// `HOMIES_CUSTOM_SET`). The chat index stores a provider's badges best-first,
/// so the first of each category is the one the provider's own client draws.
/// What a user owns is unchanged; this only limits what is drawn beside their
/// name.
pub fn worn_third_party(badges: &[Arc<UserBadge>]) -> Vec<UserBadge> {
    let mut seen: Vec<(&BadgeProvider, &str)> = Vec::new();
    badges
        .iter()
        .filter(|b| {
            let key = (&b.provider, b.badge_info.set_id.as_str());
            if seen.contains(&key) {
                return false;
            }
            seen.push(key);
            true
        })
        .map(|b| (**b).clone())
        .collect()
}

/// A Homies role badge's place among one holder's Homies badges (lower draws
/// first; the custom badge is 0). Homies' client reads `/badges` and
/// `/badges2` separately, keeps the LAST entry listing a user in each, and
/// draws the `/badges` one first. `index` is the role's position in the
/// combined list, whose `/badges2` entries all follow the `/badges` ones.
fn homies_role_rank(roles: usize, index: usize, second_feed: bool) -> usize {
    1 + if second_feed { roles } else { 0 } + roles.saturating_sub(1 + index)
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
    /// FFZ draws one badge per slot; badges sharing a slot replace each other.
    #[serde(default)]
    slot: Option<u32>,
}

/// FFZ's own name for its bot badge. Its holders are chat bots, and they get
/// Twitch's native `bot-badge` instead (see `add_ffz_bot_badge`), so this one
/// is never drawn.
const FFZ_BOT_BADGE_NAME: &str = "bot";

impl FFZBadge {
    fn is_bot(&self) -> bool {
        self.name.as_deref() == Some(FFZ_BOT_BADGE_NAME)
    }

    /// (1x, 2x, 4x) URLs of the finished badge, with a missing larger size
    /// falling back to the next smaller one.
    fn image_urls(&self) -> (String, String, String) {
        let x1 = self.urls.get("1").map(|u| ffz_colored_badge_url(u)).unwrap_or_default();
        let x2 = self.urls.get("2").map(|u| ffz_colored_badge_url(u)).unwrap_or_else(|| x1.clone());
        let x4 = self.urls.get("4").map(|u| ffz_colored_badge_url(u)).unwrap_or_else(|| x2.clone());
        (x1, x2, x4)
    }
}

/// An FFZ badge image is a white glyph on a transparent canvas; the feed's
/// `color` is meant to be painted behind it. FFZ's CDN serves that finished
/// badge (colour plus rounded corners) at `<url>/rounded`, so every surface
/// that draws the URL gets the coloured badge. Any other URL, including one
/// that already names a variant, comes back unchanged.
fn ffz_colored_badge_url(url: &str) -> String {
    match url.strip_prefix(FFZ_BADGE_CDN) {
        Some(rest) if rest.split('/').count() == 2 && !rest.ends_with('/') => format!("{url}/rounded"),
        _ => url.to_string(),
    }
}

const FFZ_BADGE_CDN: &str = "https://cdn.frankerfacez.com/badge/";

/// A scheme-relative URL (`//host/path`, which Chatty's feed lists for its own
/// badges) made `https:`. A webview resolves those against its own origin and a
/// native shell cannot load them at all; any other URL comes back unchanged.
fn with_scheme(url: &str) -> String {
    if url.starts_with("//") { format!("https:{url}") } else { url.to_string() }
}

fn is_ffz_cdn_badge(url: &str) -> bool {
    url.starts_with(FFZ_BADGE_CDN)
}

/// A glyph badge image is refused past this size; FFZ:AP's are ~3 KB.
const BACKDROP_SOURCE_MAX_BYTES: usize = 256 * 1024;

/// A glyph badge image wider or taller than this is refused; badges are 72 px.
const BACKDROP_MAX_SIDE: u32 = 512;

/// A glyph PNG with `color` (`#RRGGBB`) behind it as a rounded square, the
/// shape FFZ's own finished badges have (corner radius side / 9, which is what
/// FFZ's `/rounded` images use at 72 px), as a PNG data URI. The picture is
/// finished here, a plain PNG every surface can draw: native shells decode
/// with the platform's image library, which cannot read SVG, so a picture
/// left for the webview to composite drew nothing there. `None` for a colour
/// that isn't `#RRGGBB` or bytes that aren't a PNG.
pub(crate) fn badge_with_backdrop(png: &[u8], color: &str) -> Option<String> {
    let hex = color.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|c| c as f32 / 255.0);
    let back = [channel(0)?, channel(2)?, channel(4)?];
    let (w, h, glyph) = decode_rgba(png)?;
    let radius = w.min(h) as f32 / 9.0;
    let mut out = Vec::with_capacity(glyph.len());
    for (i, px) in glyph.chunks_exact(4).enumerate() {
        let (x, y) = ((i as u32 % w) as f32, (i as u32 / w) as f32);
        // The glyph over the square, both straight (not premultiplied) alpha.
        let under = rounded_square_cover(x, y, w as f32, h as f32, radius);
        let over = px[3] as f32 / 255.0;
        let alpha = over + under * (1.0 - over);
        if alpha <= 0.0 {
            out.extend_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        for c in 0..3 {
            let mixed = (px[c] as f32 / 255.0 * over + back[c] * under * (1.0 - over)) / alpha;
            out.push((mixed * 255.0).round() as u8);
        }
        out.push((alpha * 255.0).round() as u8);
    }
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut encoded, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&out).ok()?;
        writer.finish().ok()?;
    }
    use base64::Engine;
    Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(encoded)))
}

/// A PNG as 8-bit RGBA: (width, height, pixels). `None` for bytes that aren't
/// a PNG or one past `BACKDROP_MAX_SIDE`.
pub(crate) fn decode_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (w, h) = (reader.info().width, reader.info().height);
    if w == 0 || h == 0 || w > BACKDROP_MAX_SIDE || h > BACKDROP_MAX_SIDE {
        return None;
    }
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..frame.buffer_size()];
    let rgba: Vec<u8> = match frame.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => data.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    (rgba.len() == (w * h * 4) as usize).then_some((w, h, rgba))
}

/// How much of pixel (x, y) a `w` x `h` square with corners of `radius`
/// covers, 0 to 1, from a 4 x 4 grid of samples, so the corners are smooth.
fn rounded_square_cover(x: f32, y: f32, w: f32, h: f32, radius: f32) -> f32 {
    let mut inside = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let px = x + (sx as f32 + 0.5) / 4.0;
            let py = y + (sy as f32 + 0.5) / 4.0;
            let dx = px - px.clamp(radius, w - radius);
            let dy = py - py.clamp(radius, h - radius);
            if dx * dx + dy * dy <= radius * radius {
                inside += 1;
            }
        }
    }
    inside as f32 / 16.0
}

fn backdrop_key(url: &str, color: &str) -> String {
    format!("{color}|{url}")
}

/// Twitch user ids FFZ flags as bots, republished on every FFZ feed refresh.
/// Read from the synchronous IRC parse, hence a std lock and not the async
/// badge cache.
static FFZ_BOT_IDS: once_cell::sync::Lazy<std::sync::RwLock<std::collections::HashSet<String>>> =
    once_cell::sync::Lazy::new(Default::default);

/// Give an FFZ-flagged bot Twitch's native Chat Bot badge (`bot-badge/1`,
/// drawn from the global badge set like any other Twitch badge) when Twitch
/// did not already send it. Called on the Twitch badge list of every chat
/// row, before it is put in chat order.
pub fn add_ffz_bot_badge(user_id: &str, badges: &mut Vec<crate::models::chat_layout::Badge>) {
    const TWITCH_BOT_BADGE: &str = "bot-badge";
    if user_id.is_empty() || badges.iter().any(|b| b.name == TWITCH_BOT_BADGE) {
        return;
    }
    let is_bot = FFZ_BOT_IDS.read().map(|ids| ids.contains(user_id)).unwrap_or(false);
    if is_bot {
        badges.push(crate::models::chat_layout::Badge {
            name: TWITCH_BOT_BADGE.to_string(),
            version: "1".to_string(),
            image_url_1x: None,
            image_url_2x: None,
            image_url_4x: None,
            title: None,
            description: None,
        });
    }
}

fn publish_ffz_bot_ids(ffz: Option<&FFZBadgesResponse>) {
    let ids: std::collections::HashSet<String> = ffz
        .into_iter()
        .flat_map(|feed| {
            feed.badges
                .iter()
                .filter(|b| b.is_bot())
                .filter_map(|b| feed.users.get(&b.id.to_string()))
                .flatten()
                .map(|uid| uid.to_string())
        })
        .collect();
    if let Ok(mut current) = FFZ_BOT_IDS.write() {
        *current = ids;
    }
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
    /// True for an entry of the `/badges2` staff list. Homies' own client
    /// draws a holder's `/badges` role and their `/badges2` badge as two
    /// separate badges, `/badges` first, so each entry remembers its list
    /// (see `homies_role_rank` and `HOMIES_STAFF_SET`).
    #[serde(skip)]
    second_feed: bool,
}

// Homies serves two kinds of badge from two hosts, and its own client loads
// both. The role badges above (developer, supporter, founder, mod, bot) live on
// `itzalex.github.io/badges` and `/badges2`, one entry per role with a holder
// list. The CUSTOM badges, art a supporter uploads for themselves, live on
// `chatterinohomies.com/api/badges/list`: one entry PER USER, keyed by Twitch
// user id, about 9,500 entries and 4 MB (570 KB gzipped). Most holders only
// have a custom badge and appear in no role list at all, so without this feed
// their Homies badge never shows.
//
// Entry shape: `{badgeFileType, badgeId, image1, image2, image3, tooltip,
// userId, username}`; images are 18/36/72 px (often animated) webp on
// `cdn.chatterinohomies.com`, the tooltip is "<name> Badge". The list holds
// only badges Homies has approved; `/api/v2/badges/{twitchId}` answers one
// user and is what the website resolver uses.
const HOMIES_CUSTOM_BADGES_URL: &str = "https://chatterinohomies.com/api/badges/list";
/// New custom badges trickle in a few a day and the feed is 4 MB, so it gets
/// its own clock instead of the shared 10-minute one (and its 60-second retry
/// whenever any other provider is down).
const HOMIES_CUSTOM_REFRESH: Duration = Duration::from_secs(60 * 60);
/// A runaway feed cannot balloon the index past this many holders.
const HOMIES_CUSTOM_MAX_ENTRIES: usize = 250_000;
/// Every custom badge carries this one id: a user has at most one, so the
/// member loadout key `homies:homies-custom` names it without depending on
/// the tooltip, which follows the holder's display name.
pub const HOMIES_CUSTOM_BADGE_ID: &str = "homies-custom";
/// The gallery's one tile for all of them (`homies_custom_gallery_tile`).
const HOMIES_CUSTOM_TILE_TITLE: &str = "Homies Custom Badge";
const HOMIES_CLICK_URL: &str = "https://chatterinohomies.com/";

#[derive(Debug, Clone, Default, Deserialize)]
struct HomiesCustomBadgesResponse {
    #[serde(default)]
    badges: Vec<HomiesCustomBadge>,
}

/// One user's custom badge. Every field is read leniently (a number or a
/// null where a string belongs is taken, not fatal) so one odd entry never
/// costs every holder their badge.
#[derive(Debug, Clone, Default, Deserialize)]
struct HomiesCustomBadge {
    #[serde(rename = "userId", default, deserialize_with = "lenient_string")]
    user_id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    tooltip: String,
    #[serde(default, deserialize_with = "lenient_string")]
    image1: String,
    #[serde(default, deserialize_with = "lenient_string")]
    image2: String,
    #[serde(default, deserialize_with = "lenient_string")]
    image3: String,
}

/// A string field that may arrive as a string, a number or null.
fn lenient_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    })
}

/// Twitch user ids are 1-32 ASCII digits.
fn is_twitch_user_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}

/// user id -> that user's custom badge, ready for the chat index. A later
/// entry for the same user replaces an earlier one, as in Homies' own client;
/// an entry without a numeric id or an image is skipped.
fn homies_custom_index(feed: HomiesCustomBadgesResponse) -> HashMap<String, Arc<UserBadge>> {
    let mut out = HashMap::with_capacity(feed.badges.len().min(HOMIES_CUSTOM_MAX_ENTRIES));
    for entry in feed.badges.into_iter().take(HOMIES_CUSTOM_MAX_ENTRIES) {
        let uid = entry.user_id.trim();
        let x1 = entry.image1.trim();
        if !is_twitch_user_id(uid) || x1.is_empty() {
            continue;
        }
        let x2 = if entry.image2.trim().is_empty() { x1 } else { entry.image2.trim() };
        let x4 = if entry.image3.trim().is_empty() { x2 } else { entry.image3.trim() };
        let title = match entry.tooltip.trim() {
            "" => "Homies Badge".to_string(),
            t => t.to_string(),
        };
        let badge = UserBadge {
            badge_info: BadgeInfo {
                id: HOMIES_CUSTOM_BADGE_ID.to_string(),
                set_id: HOMIES_CUSTOM_SET.to_string(),
                version: "custom".to_string(),
                title,
                description: String::new(),
                image_1x: x1.to_string(),
                image_2x: x2.to_string(),
                image_4x: x4.to_string(),
                click_action: None,
                click_url: Some(HOMIES_CLICK_URL.to_string()),
            },
            provider: BadgeProvider::Homies,
        };
        out.insert(uid.to_string(), Arc::new(badge));
    }
    out
}

/// The browse gallery's one tile for every Homies custom badge: thousands of
/// one-holder badges would flood it. It shows the viewer's own art when they
/// have one, and otherwise the art of the first holder by user id, so the
/// tile does not change between refreshes.
fn homies_custom_gallery_tile(
    custom: &HashMap<String, Arc<UserBadge>>,
    viewer_user_id: Option<&str>,
) -> Option<ThirdPartyGalleryBadge> {
    let own = viewer_user_id.and_then(|uid| custom.get(uid));
    let shown = own.or_else(|| {
        custom
            .iter()
            .min_by_key(|(uid, _)| (uid.len(), uid.as_str()))
            .map(|(_, badge)| badge)
    })?;
    let info = &shown.badge_info;
    Some(ThirdPartyGalleryBadge {
        id: HOMIES_CUSTOM_BADGE_ID.to_string(),
        provider: BadgeProvider::Homies,
        title: HOMIES_CUSTOM_TILE_TITLE.to_string(),
        image_1x: info.image_1x.clone(),
        image_2x: info.image_2x.clone(),
        image_4x: info.image_4x.clone(),
        user_count: custom.len(),
        owned: own.is_some(),
        click_url: Some(HOMIES_CLICK_URL.to_string()),
    })
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
    /// The colour painted behind the image. Null when the image is already
    /// coloured (an FFZ:AP supporter who picked a coloured badge).
    #[serde(default)]
    color: Option<String>,
}

impl ChattyBadge {
    /// Chatty's re-host of FFZ's bot badge, dropped for the same reason.
    fn is_ffz_bot(&self) -> bool {
        self.id == "ffz" && self.version.as_deref() == Some(FFZ_BOT_BADGE_NAME)
    }

    /// The largest image and the colour to paint behind it, when this badge
    /// needs one drawn for it. FFZ's own CDN draws its badges finished (see
    /// `ffz_colored_badge_url`); the rest of the feed (FFZ:AP) is a white
    /// glyph with no finished variant anywhere, so we paint it ourselves.
    fn backdrop(&self) -> Option<(String, String)> {
        let color = self.color.as_deref()?.trim();
        if color.is_empty() || is_ffz_cdn_badge(&with_scheme(&self.image_url)) {
            return None;
        }
        Some((self.image_urls().2, color.to_string()))
    }

    /// (1x, 2x, 4x) URLs; re-hosted FFZ badges get their coloured variant.
    fn image_urls(&self) -> (String, String, String) {
        let x1 = self.image_url.as_str();
        let x2 = self.image_url_2.as_deref().unwrap_or(x1);
        let x4 = self.image_url_4.as_deref().unwrap_or(x2);
        let finished = |url: &str| ffz_colored_badge_url(&with_scheme(url));
        (finished(x1), finished(x2), finished(x4))
    }
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
    /// Homies custom badges, already in index form (user id -> badge): one
    /// distinct badge per holder, so there is nothing to share and keeping
    /// the raw 4 MB feed beside it would only double the memory.
    homies_custom: Option<HashMap<String, Arc<UserBadge>>>,
    /// Refreshed on its own clock (`HOMIES_CUSTOM_REFRESH`).
    homies_custom_last_updated: SystemTime,
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
    /// Badge images that need a colour painted behind them (FFZ:AP via
    /// Chatty): largest raw image URL -> `#RRGGBB`. Rebuilt per refresh.
    backdrops: HashMap<String, String>,
    /// Finished images built so far, `backdrop_key(url, colour)` -> SVG
    /// data URI (`badge_with_backdrop`). Filled on first lookup of a holder, so only chatters who are
    /// actually seen cost a download; kept across refreshes while the
    /// feed still lists that image and colour.
    painted: HashMap<String, String>,
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
    ///
    /// The index holds every badge a user OWNS (the profile card and the
    /// badge picker list them all), but within one provider they sit
    /// best-first, so `worn_third_party` can keep the first of each wear
    /// category (the badge's `set_id`, see `HOMIES_CUSTOM_SET`) for chat.
    /// Each provider's "best" is the badge its own client draws first for
    /// that user:
    ///
    /// - FFZ: one badge per slot (the slot is the category); the feed lists
    ///   them in FFZ's own order, so within a slot the first listed wins.
    /// - BetterTTV: the feed has one entry per user; the first one wins.
    /// - Chatterino: its client keeps the LAST entry listing a user (each entry
    ///   overwrites the user's slot as the feed is read), so the last wins.
    /// - Homies: the custom badge, then the `/badges` role, then the
    ///   `/badges2` staff badge, each its own category; within one list the
    ///   last entry listing the user (Homies' client overwrites per list).
    /// - Moltorino: the first tier in feed order (Moltorino's own rule).
    /// - Chatsen, Chatty: the first entry in feed order.
    /// - DankChat: the first entry listing the user, exactly as DankChat looks
    ///   one up.
    fn build_by_user_index(&self) -> HashMap<String, Vec<Arc<UserBadge>>> {
        let mut by_user: HashMap<String, Vec<(Arc<UserBadge>, usize)>> = HashMap::new();

        /// Add `badge` to a user's list unless they already carry a badge with
        /// that title. `rank` orders badges of one provider (lower draws
        /// first); ties keep feed order. Providers are walked one after
        /// another, so a provider's badges are contiguous at the end of the
        /// list while it is being indexed.
        fn push(
            by_user: &mut HashMap<String, Vec<(Arc<UserBadge>, usize)>>,
            user_id: String,
            badge: &Arc<UserBadge>,
            title_lower: &str,
            rank: usize,
        ) {
            let entry = by_user.entry(user_id).or_default();
            if entry
                .iter()
                .any(|(b, _)| b.badge_info.title.to_lowercase() == title_lower)
            {
                return;
            }
            let at = entry
                .iter()
                .position(|(b, r)| b.provider == badge.provider && *r > rank)
                .unwrap_or(entry.len());
            entry.insert(at, (Arc::clone(badge), rank));
        }

        // FFZ. `users` is keyed by badge_id (as a string) -> [numeric user_id],
        // so the holder key is each numeric id rendered back to a String.
        if let Some(ffz) = &self.ffz {
            for (rank, badge) in ffz.badges.iter().filter(|b| !b.is_bot()).enumerate() {
                let (image_1x, image_2x, image_4x) = badge.image_urls();
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("ffz-{}", badge.id),
                        set_id: ffz_wear_set(badge.slot),
                        version: badge.id.to_string(),
                        title: badge
                            .title
                            .clone()
                            .or_else(|| badge.name.clone())
                            .unwrap_or_else(|| format!("FFZ Badge {}", badge.id)),
                        description: String::new(),
                        image_1x,
                        image_2x,
                        image_4x,
                        click_action: None,
                        click_url: Some("https://www.frankerfacez.com/badges".to_string()),
                    },
                    provider: BadgeProvider::FFZ,
                });
                let title_lower = arc.badge_info.title.to_lowercase();
                if let Some(holders) = ffz.users.get(&badge.id.to_string()) {
                    for uid in holders {
                        push(&mut by_user, uid.to_string(), &arc, &title_lower, rank);
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
                push(&mut by_user, badge.provider_id.clone(), &arc, &title_lower, 0);
            }
        }

        // Chatterino badges
        if let Some(chatterino) = &self.chatterino {
            let chatterino_last = chatterino.badges.len().saturating_sub(1);
            for (rank, badge) in chatterino.badges.iter().enumerate() {
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
                    push(&mut by_user, uid.clone(), &arc, &title_lower, chatterino_last - rank);
                }
            }
        }

        // Homies: the holder's custom badge first, as Homies' own client
        // draws it, then their role badges.
        if let Some(custom) = &self.homies_custom {
            for (uid, arc) in custom {
                let title_lower = arc.badge_info.title.to_lowercase();
                push(&mut by_user, uid.clone(), arc, &title_lower, 0);
            }
        }
        if let Some(homies) = &self.homies {
            let roles = homies.badges.len();
            for (rank, badge) in homies.badges.iter().enumerate() {
                let arc = Arc::new(UserBadge {
                    badge_info: BadgeInfo {
                        id: format!("homies-{}", badge.tooltip),
                        set_id: if badge.second_feed { HOMIES_STAFF_SET } else { HOMIES_ROLES_SET }.to_string(),
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
                    push(&mut by_user, uid.clone(), &arc, &title_lower, homies_role_rank(roles, rank, badge.second_feed));
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
            for (rank, badge) in moltorino.badges.iter().take(MOLTORINO_MAX_TIERS).enumerate() {
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
                    push(&mut by_user, uid.to_string(), &arc, &title_lower, rank);
                }
            }
        }

        // Chatsen badges
        if let Some(chatsen) = &self.chatsen {
            for (rank, badge) in chatsen.iter().enumerate() {
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
                    push(&mut by_user, uid.clone(), &arc, &title_lower, rank);
                }
            }
        }

        // Chatty (tduva) badges
        if let Some(chatty) = &self.chatty {
            for (rank, badge) in chatty.iter().filter(|b| !b.is_ffz_bot()).enumerate() {
                let (image_1x, image_2x, image_4x) = badge.image_urls();
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
                        image_1x,
                        image_2x,
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
                    push(&mut by_user, uid.clone(), &arc, &title_lower, rank);
                }
            }
        }

        // DankChat (flex3r) badges
        if let Some(dankchat) = &self.dankchat {
            for (rank, badge) in dankchat.iter().enumerate() {
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
                    push(&mut by_user, uid.clone(), &arc, &title_lower, rank);
                }
            }
        }

        by_user
            .into_iter()
            .map(|(uid, ranked)| (uid, ranked.into_iter().map(|(badge, _)| badge).collect()))
            .collect()
    }
}

/// Every distinct third-party badge as a browse-gallery tile, from the feeds
/// already in memory (no network). Entries sharing a (provider, title) are one
/// tile. Chatty glyph backdrops are NOT painted here; `get_all_third_party_badges`
/// does that, since it may download.
fn third_party_gallery_tiles(tp: &ThirdPartyCache, viewer_user_id: Option<&str>) -> Vec<ThirdPartyGalleryBadge> {
    let mut out: Vec<ThirdPartyGalleryBadge> = Vec::new();

    // FFZ. `users` is keyed by badge_id (as a string) -> [numeric user_id].
    if let Some(ffz) = &tp.ffz {
        let viewer_num = viewer_user_id.and_then(|uid| uid.parse::<u32>().ok());
        for badge in ffz.badges.iter().filter(|b| !b.is_bot()) {
            let holders = ffz.users.get(&badge.id.to_string());
            let user_count = holders.map(|h| h.len()).unwrap_or(0);
            let owned = match (viewer_num, holders) {
                (Some(n), Some(h)) => h.contains(&n),
                _ => false,
            };
            let (img_1x, img_2x, img_4x) = badge.image_urls();
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
    if let Some(bttv) = &tp.bttv {
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
    if let Some(chatterino) = &tp.chatterino {
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
    if let Some(homies) = &tp.homies {
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
    if let Some(custom) = &tp.homies_custom {
        out.extend(homies_custom_gallery_tile(custom, viewer_user_id));
    }

    // Chatsen
    if let Some(chatsen) = &tp.chatsen {
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
    if let Some(chatty) = &tp.chatty {
        for badge in chatty.iter().filter(|b| !b.is_ffz_bot()) {
            let owned = viewer_user_id
                .map(|uid| badge.userids.iter().any(|u| u == uid))
                .unwrap_or(false);
            let (img_1x, img_2x, img_4x) = badge.image_urls();
            out.push(ThirdPartyGalleryBadge {
                id: format!(
                    "chatty-{}-{}",
                    badge.id,
                    badge.version.clone().unwrap_or_default()
                ),
                provider: BadgeProvider::Chatty,
                title: badge.meta_title.clone().unwrap_or_else(|| badge.id.clone()),
                image_1x: img_1x,
                image_2x: img_2x,
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
    if let Some(dankchat) = &tp.dankchat {
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
    if let Some(moltorino) = &tp.moltorino {
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

// ============================================================================
// BADGE DETAIL (the badge spotlight card)
// ============================================================================

/// What the badge spotlight shows for a clicked third-party badge, from the
/// feeds already in memory. A field is `None` when Rust does not know it: the
/// clicked badge itself (its art and title, which React already holds) fills
/// those in, so a detail never needs a gallery hit. That matters because the
/// gallery merges badges by (provider, title) and keeps ONE tile for every
/// Homies custom badge.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BadgeDetail {
    /// The canonical lowercase provider key (`BadgeProvider::as_key`).
    pub provider: String,
    pub provider_label: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub image_1x: Option<String>,
    pub image_2x: Option<String>,
    pub image_4x: Option<String>,
    /// The badge's own link (a Chatty badge's page, FFZ's badge list).
    pub click_url: Option<String>,
    /// The provider's site, always known.
    pub homepage: String,
    pub holder_count: Option<u32>,
    /// Whether the viewer holds this badge; `None` when unknown (no viewer
    /// given, or a badge the feeds do not list, such as BetterTTV Pro).
    pub owned: Option<bool>,
}

/// The longest badge id or title accepted from the WebView. Real ones are a
/// few dozen characters (Chatterino ids carry the tooltip).
const MAX_BADGE_REF_LEN: usize = 256;

impl BadgeProvider {
    /// The provider for a wire key, in any letter case (`BTTV` for the
    /// BetterTTV Pro badge, `ffz` from Rust).
    pub fn from_key(key: &str) -> Option<BadgeProvider> {
        Some(match key.to_ascii_lowercase().as_str() {
            "twitch" => BadgeProvider::Twitch,
            "ffz" => BadgeProvider::FFZ,
            "bttv" => BadgeProvider::BTTV,
            "chatterino" => BadgeProvider::Chatterino,
            "homies" => BadgeProvider::Homies,
            "chatsen" => BadgeProvider::Chatsen,
            "chatty" => BadgeProvider::Chatty,
            "dankchat" => BadgeProvider::DankChat,
            "moltorino" => BadgeProvider::Moltorino,
            _ => return None,
        })
    }

    /// The provider's name as people know it.
    pub fn label(&self) -> &'static str {
        match self {
            BadgeProvider::Twitch => "Twitch",
            BadgeProvider::FFZ => "FrankerFaceZ",
            BadgeProvider::BTTV => "BetterTTV",
            BadgeProvider::Chatterino => "Chatterino",
            BadgeProvider::Homies => "Homies",
            BadgeProvider::Chatsen => "Chatsen",
            BadgeProvider::Chatty => "Chatty",
            BadgeProvider::DankChat => "DankChat",
            BadgeProvider::Moltorino => "Moltorino",
        }
    }

    /// The provider's site (the same links the gallery's section headers use).
    pub fn homepage(&self) -> &'static str {
        match self {
            BadgeProvider::Twitch => "https://www.twitch.tv/",
            BadgeProvider::FFZ => "https://www.frankerfacez.com/badges",
            BadgeProvider::BTTV => "https://betterttv.com",
            BadgeProvider::Chatterino => "https://chatterino.com/",
            BadgeProvider::Homies => HOMIES_CLICK_URL,
            BadgeProvider::Chatsen => "https://chatsen.app",
            BadgeProvider::Chatty => "https://chatty.github.io",
            BadgeProvider::DankChat => "https://github.com/flex3r/DankChat",
            BadgeProvider::Moltorino => "https://moltorino.com/",
        }
    }
}

fn has_control_chars(s: &str) -> bool {
    s.chars().any(char::is_control)
}

/// Turn a clicked badge's (provider, id) into the id space the feeds use.
///
/// Three id spaces reach the WebView: the Rust path (`ffz-3`), the member
/// loadout path (`ffz:ffz-3`), and a bare provider id (`3`). All three become
/// `ffz-3`. Providers arrive in any case (`BTTV` for the BetterTTV Pro badge,
/// whose id `bttv-pro` passes through). Twitch is not a third-party provider
/// and is refused, as are empty, overlong and control-character ids.
pub fn normalize_badge_ref(provider: &str, id: &str) -> Result<(BadgeProvider, String), String> {
    let provider_key = provider.trim();
    if provider_key.is_empty() || provider_key.len() > 32 || !provider_key.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err("Invalid badge provider".to_string());
    }
    let provider = match BadgeProvider::from_key(provider_key) {
        Some(BadgeProvider::Twitch) | None => return Err(format!("Not a third-party badge provider: {provider_key}")),
        Some(p) => p,
    };
    let id = id.trim();
    if id.is_empty() || id.len() > MAX_BADGE_REF_LEN || has_control_chars(id) {
        return Err("Invalid badge id".to_string());
    }
    let key = provider.as_key();
    // `ffz:ffz-3` -> `ffz-3` (the loadout key carries its provider in front).
    let id = match id.split_once(':') {
        Some((prefix, rest)) if prefix.eq_ignore_ascii_case(key) => rest.trim(),
        _ => id,
    };
    if id.is_empty() {
        return Err("Invalid badge id".to_string());
    }
    let prefix = format!("{key}-");
    // `get` (not indexing): the id may hold multi-byte characters.
    let id = match (id.get(..prefix.len()), id.get(prefix.len()..)) {
        (Some(head), Some(rest)) if head.eq_ignore_ascii_case(&prefix) && !rest.is_empty() => format!("{prefix}{rest}"),
        _ => format!("{prefix}{id}"),
    };
    Ok((provider, id))
}

/// A clicked badge's title, checked like its id. Blank means none.
fn normalize_badge_title(title: Option<&str>) -> Result<Option<String>, String> {
    let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if title.len() > MAX_BADGE_REF_LEN || has_control_chars(title) {
        return Err("Invalid badge title".to_string());
    }
    Ok(Some(title.to_string()))
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// The detail for one (already normalised) badge, from the in-memory feeds.
/// A gallery tile is used only when it is this badge: the same id, or, for
/// feeds whose ids repeat per holder, the same title. The Homies custom tile
/// stands for thousands of different badges, so it is never used; whether the
/// viewer wears the clicked custom badge is read from their own entry instead.
fn badge_detail_from_cache(
    tp: &ThirdPartyCache,
    provider: BadgeProvider,
    id: &str,
    title: Option<&str>,
    viewer_user_id: Option<&str>,
) -> BadgeDetail {
    let mut detail = BadgeDetail {
        provider: provider.as_key().to_string(),
        provider_label: provider.label().to_string(),
        title: None,
        description: None,
        image_1x: None,
        image_2x: None,
        image_4x: None,
        click_url: None,
        homepage: provider.homepage().to_string(),
        holder_count: None,
        owned: None,
    };

    if provider == BadgeProvider::Homies && id == HOMIES_CUSTOM_BADGE_ID {
        // The gallery tile itself (no title of its own badge) stands for "a
        // Homies custom badge": its count and the viewer's having one are true
        // of it. A chat badge is one person's own, told apart by its title.
        let whole_tile = title.is_none_or(|t| t.eq_ignore_ascii_case(HOMIES_CUSTOM_TILE_TITLE));
        if let Some(custom) = tp.homies_custom.as_ref() {
            if whole_tile {
                detail.holder_count = u32::try_from(custom.len()).ok().filter(|n| *n > 0);
            }
            if let Some(uid) = viewer_user_id {
                let own = custom.get(uid);
                detail.owned = Some(match (own, title) {
                    (Some(_), _) if whole_tile => true,
                    (Some(badge), Some(t)) => badge.badge_info.title.eq_ignore_ascii_case(t),
                    _ => false,
                });
            }
        }
        return detail;
    }

    let tiles = third_party_gallery_tiles(tp, viewer_user_id);
    let tile = tiles
        .iter()
        .filter(|t| t.provider == provider)
        .find(|t| t.id == id)
        .or_else(|| {
            let t = title?;
            tiles
                .iter()
                .filter(|tile| tile.provider == provider && tile.id != HOMIES_CUSTOM_BADGE_ID)
                .find(|tile| tile.title.eq_ignore_ascii_case(t))
        });
    if let Some(tile) = tile {
        detail.title = non_empty(&tile.title);
        detail.image_1x = non_empty(&tile.image_1x);
        detail.image_2x = non_empty(&tile.image_2x);
        detail.image_4x = non_empty(&tile.image_4x);
        detail.click_url = tile.click_url.as_deref().and_then(non_empty);
        detail.holder_count = u32::try_from(tile.user_count).ok().filter(|n| *n > 0);
        if viewer_user_id.is_some() {
            detail.owned = Some(tile.owned);
        }
    }

    // The chat index knows exactly what the viewer holds, by id.
    if let Some(uid) = viewer_user_id {
        let held = tp
            .by_user
            .get(uid)
            .is_some_and(|badges| badges.iter().any(|b| b.provider == provider && b.badge_info.id == id));
        if held {
            detail.owned = Some(true);
        }
    }

    // Moltorino is the one feed that writes descriptions.
    if provider == BadgeProvider::Moltorino {
        if let Some(feed) = &tp.moltorino {
            detail.description = feed
                .badges
                .iter()
                .find(|b| format!("moltorino-{}", b.id) == id)
                .and_then(|b| non_empty(&b.description));
        }
    }

    detail
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
                homies_custom: None,
                homies_custom_last_updated: UNIX_EPOCH,
                chatsen: None,
                chatty: None,
                dankchat: None,
                moltorino: None,
                moltorino_last_updated: UNIX_EPOCH,
                by_user: HashMap::new(),
                backdrops: HashMap::new(),
                painted: HashMap::new(),
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
    /// Selected Twitch badge per user id, with when it was read (see
    /// `selected_badge`). Bounded: profile cards open for many chatters.
    selected_badges: tokio::sync::Mutex<LruCache<String, (Option<SelectedTwitchBadge>, Instant)>>,
    /// One third-party badge refresh at a time (`fetch_third_party_badges`).
    third_party_refresh: tokio::sync::Mutex<()>,
    /// Refreshes in a row that had a provider fail, and which ones failed
    /// last: the retry backs off on the first and logs when the second changes.
    third_party_failures: std::sync::Mutex<(u32, String)>,
}

/// When a third-party refresh with failed providers is retried: a minute, then
/// doubling per refresh that fails again, up to the normal refresh window.
fn third_party_retry_after(failures_in_a_row: u32, window: Duration) -> Duration {
    let doublings = failures_in_a_row.saturating_sub(1).min(8);
    Duration::from_secs(60).saturating_mul(1 << doublings).min(window)
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
            selected_badges: tokio::sync::Mutex::new(LruCache::new(NonZeroUsize::new(256).unwrap())),
            third_party_refresh: tokio::sync::Mutex::new(()),
            third_party_failures: std::sync::Mutex::new((0, String::new())),
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

        // One refresh at a time. The freshness check below drops the cache lock
        // for the network calls, so without this every caller arriving during a
        // refresh started its own (a profile burst refetched every provider
        // dozens of times in a few seconds while one was down). A caller that
        // finds a refresh running uses the set already loaded; only the first
        // load waits for it.
        let _refresh = match self.third_party_refresh.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                if self.cache.read().await.third_party.last_updated != UNIX_EPOCH {
                    return Ok(());
                }
                self.third_party_refresh.lock().await
            }
        };

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
        let homies_custom_due = cache
            .third_party
            .homies_custom_last_updated
            .elapsed()
            .map(|e| e >= HOMIES_CUSTOM_REFRESH)
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

        // Homies custom badges (one entry per holder, 4 MB), on their own clock.
        // Indexed here, before the cache lock is taken again.
        let homies_custom = if homies_custom_due {
            match self.http_client.get(HOMIES_CUSTOM_BADGES_URL).send().await {
                Ok(response) if response.status().is_success() => response
                    .json::<HomiesCustomBadgesResponse>()
                    .await
                    .map_err(|e| log::warn!("[BadgeService] Homies custom badge list did not parse: {e}"))
                    .ok()
                    .map(homies_custom_index),
                Ok(response) => {
                    log::warn!("[BadgeService] Homies custom badge list returned {}", response.status());
                    None
                }
                Err(e) => {
                    log::warn!("[BadgeService] Homies custom badge list failed: {e}");
                    None
                }
            }
        } else {
            None
        };

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
                    format!(
                        "StreamNook/{} (+https://streamnook.app)",
                        crate::services::client_identity::build_version()
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
        publish_ffz_bot_ids(cache.third_party.ffz.as_ref());
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
        // Homies custom badges follow the same rule: keep the last good list,
        // and a failure retries at the next 10-minute gate without back-dating
        // the shared stamp (the 4 MB list must never ride the 60-second retry).
        if homies_custom_due {
            match homies_custom {
                Some(index) => {
                    log::debug!("[BadgeService] Homies custom badges: {} holders", index.len());
                    cache.third_party.homies_custom = Some(index);
                    cache.third_party.homies_custom_last_updated = SystemTime::now();
                }
                None => {
                    cache.third_party.homies_custom_last_updated = SystemTime::now()
                        .checked_sub(HOMIES_CUSTOM_REFRESH.saturating_sub(cache_duration))
                        .unwrap_or(UNIX_EPOCH);
                }
            }
        }
        // Rebuild the inverted per-user index once per refresh (~10 min) so
        // per-chatter lookups never scan the full holder lists.
        let by_user = cache.third_party.build_by_user_index();
        cache.third_party.by_user = by_user;
        let backdrops: HashMap<String, String> = cache
            .third_party
            .chatty
            .iter()
            .flatten()
            .filter_map(ChattyBadge::backdrop)
            .collect();
        let current: std::collections::HashSet<String> =
            backdrops.iter().map(|(url, color)| backdrop_key(url, color)).collect();
        cache.third_party.painted.retain(|key, _| current.contains(key));
        cache.third_party.backdrops = backdrops;
        if failed.is_empty() {
            cache.third_party.last_updated = SystemTime::now();
            if let Ok(mut failures) = self.third_party_failures.lock() {
                *failures = (0, String::new());
            }
        } else {
            let failed_list = failed.join(", ");
            let (in_a_row, changed) = match self.third_party_failures.lock() {
                Ok(mut failures) => {
                    failures.0 = failures.0.saturating_add(1);
                    let changed = failures.1 != failed_list;
                    failures.1 = failed_list.clone();
                    (failures.0, changed)
                }
                Err(_) => (1, true),
            };
            let retry = third_party_retry_after(in_a_row, cache_duration);
            if changed {
                log::warn!(
                    "[BadgeService] Third-party badge providers failed to refresh: {}; retrying in {}s",
                    failed_list,
                    retry.as_secs()
                );
            } else {
                log::debug!(
                    "[BadgeService] Third-party badge providers still failing ({} in a row): {}; retrying in {}s",
                    in_a_row,
                    failed_list,
                    retry.as_secs()
                );
            }
            // Back-date the stamp so the next lookup retries after the backoff
            // instead of serving the stale set for the full window.
            cache.third_party.last_updated = SystemTime::now()
                .checked_sub(cache_duration.saturating_sub(retry))
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
                    // Kept apart from `/badges`: Homies draws a holder's
                    // badge from each list separately, so a user listed in both
                    // wears one of each.
                    for mut badge in data.badges {
                        badge.second_feed = true;
                        all_badges.push(badge);
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
        let (display, earned) = self
            .lookup_badges(user_id, username, channel_id, channel_name)
            .await?;
        Ok((display.unwrap_or_default(), earned.unwrap_or_default()))
    }

    /// A user's earned badges, read anonymously, as "set_id/version" strings.
    ///
    /// `channelViewer.earnedBadges` is the user's WHOLE global collection plus
    /// the channel's own badges (subscriber, broadcaster...) for the channel it
    /// is asked in, so no sign-in of any kind is needed to know what a user owns.
    /// Asked in the user's own channel. Errs, rather than answering empty, when
    /// Twitch did not return the field: an empty list must mean "owns nothing".
    pub async fn earned_badge_collection(
        &self,
        user_id: &str,
        login: &str,
    ) -> Result<Vec<String>, String> {
        let (_, earned) = self.lookup_badges(user_id, login, user_id, login).await?;
        earned.ok_or_else(|| "Twitch returned no earnedBadges for this user".to_string())
    }

    /// The global badge `user_id` selected on Twitch: the one chat shows for
    /// them in every channel, as its image and title. `Ok(None)` when they wear
    /// none. Profile cards read this rather than the identity loadout's
    /// `twitch:` key, which is only a copy taken when that member last opened
    /// Profile settings. Anonymous (a public field), cached per user briefly.
    pub async fn selected_badge(&self, user_id: &str) -> Result<Option<SelectedTwitchBadge>, String> {
        if let Some((badge, at)) = self.selected_badges.lock().await.get(user_id) {
            if at.elapsed() < SELECTED_BADGE_TTL {
                return Ok(badge.clone());
            }
        }
        let body = serde_json::json!({ "query": SELECTED_BADGE_QUERY, "variables": { "id": user_id } });
        let response = self
            .http_client
            .post("https://gql.twitch.tv/gql")
            .header("Accept-Language", "en-US")
            .header("Client-ID", env!("TWITCH_WEB_CLIENT_ID"))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Failed to send GQL request: {}", e))?;
        if !response.status().is_success() {
            return Err(format!("GQL request failed with status: {}", response.status()));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse GQL response: {}", e))?;
        let user = value
            .pointer("/data/user")
            .filter(|u| !u.is_null())
            .ok_or_else(|| "Twitch returned no user for this id".to_string())?;
        let badge = user.get("selectedBadge").filter(|b| !b.is_null()).and_then(|b| {
            let src = b.get("imageURL")?.as_str()?.to_string();
            let title = b
                .get("title")
                .and_then(|t| t.as_str())
                .filter(|t| !t.is_empty())
                .unwrap_or("Twitch")
                .to_string();
            Some(SelectedTwitchBadge { src, title })
        });
        self.selected_badges
            .lock()
            .await
            .put(user_id.to_string(), (badge.clone(), Instant::now()));
        Ok(badge)
    }

    /// The badge lookup with each field's absence kept: `None` means Twitch did
    /// not answer that field, `Some(vec![])` means it answered "none".
    async fn lookup_badges(
        &self,
        user_id: &str,
        username: &str,
        channel_id: &str,
        channel_name: &str,
    ) -> Result<(Option<Vec<String>>, Option<Vec<String>>), String> {
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

        let to_ids = |badges: Option<Vec<GQLBadge>>| -> Option<Vec<String>> {
            badges.map(|list| {
                list.into_iter()
                    .map(|b| format!("{}/{}", b.set_id, b.version))
                    .collect()
            })
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
        // 1. The already-fetched channel-specific earned badges, then
        // 2. the global badge collection (signed-in user only).
        let global_badge_ids = match self
            .fetch_current_user_global_collection(user_id, username)
            .await
        {
            Ok(global_badge_ids) => global_badge_ids,
            Err(e) => {
                // Expected for other users' profiles (not the token's user), so
                // this stays at debug; a failure for the signed-in user is
                // logged at WARN by fetch_current_user_global_collection.
                log::debug!(
                    "[BadgeService] Global badge collection unavailable for {}: {}",
                    username,
                    e
                );
                Vec::new()
            }
        };

        // 3. Resolve all unique badge IDs to full badge info, in the order
        // Twitch gave them, so two answers for one person list the same way.
        let badge_ids = earned_ids_in_order(channel_earned_ids, global_badge_ids);
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

        // The channel's own moderator / VIP art from its FFZ room, when the
        // viewer has it on. Identity and title stay Twitch's.
        for badge in &mut resolved {
            if let Some(art) =
                crate::services::ffz_room_badges::art_for(channel_id, &badge.badge_info.set_id)
            {
                badge.badge_info.image_1x = art.x1;
                badge.badge_info.image_2x = art.x2;
                badge.badge_info.image_4x = art.x4;
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
    /// already-prefetched provider databases. In-memory cache lookup: no
    /// Twitch GQL, no token, and no network round-trip except the first
    /// sighting of an FFZ:AP holder, whose ~3 KB glyph is fetched once to
    /// put its colour behind it (`painted_badge_image`). Safe to call once per
    /// chatter in the live chat path. Display/earned are left empty (chat
    /// renders Twitch badges straight from IRC tags). BTTV *Pro* loyalty badges
    /// are intentionally NOT included here: they live behind a per-user live
    /// socket lookup, the one thing that would reintroduce a per-chatter network
    /// cost, so Pro stays an opt-in identity badge resolved elsewhere.
    ///
    /// This is the chat path, so it answers what the chatter WEARS: one badge
    /// per wear category (`worn_third_party`). The profile and picker paths below
    /// list everything the user owns.
    /// What a chatter wears from the third-party feeds, read without waiting:
    /// empty while a refresh holds the cache, so a row stamped then carries
    /// none rather than holding up chat. Chatty's painted variants are not
    /// built here; their raw image is drawn.
    pub fn worn_third_party_now(&self, user_id: &str) -> Vec<UserBadge> {
        let Ok(cache) = self.cache.try_read() else { return Vec::new() };
        cache.third_party.by_user.get(user_id).map(|entries| worn_third_party(entries)).unwrap_or_default()
    }

    pub async fn get_third_party_badges_only(&self, user_id: &str) -> UserBadgesResponse {
        UserBadgesResponse {
            display_badges: Vec::new(),
            earned_badges: Vec::new(),
            third_party_badges: self.get_third_party_badges_for_user_worn(user_id, true).await,
        }
    }

    async fn get_third_party_badges_for_user(&self, user_id: &str) -> Vec<UserBadge> {
        self.get_third_party_badges_for_user_worn(user_id, false).await
    }

    async fn get_third_party_badges_for_user_worn(&self, user_id: &str, worn_only: bool) -> Vec<UserBadge> {
        // Single lookup in the inverted index built at feed-refresh time (see
        // ThirdPartyCache::build_by_user_index). Provider order and the
        // duplicate-title collapse are baked into the index, so this is just a
        // clone-out of the user's shared Arc entries.
        let mut badges: Vec<UserBadge> = {
            let cache = self.cache.read().await;
            match cache.third_party.by_user.get(user_id) {
                Some(entries) if worn_only => worn_third_party(entries),
                Some(entries) => entries.iter().map(|b| (**b).clone()).collect(),
                None => Vec::new(),
            }
        };
        for badge in badges.iter_mut().filter(|b| b.provider == BadgeProvider::Chatty) {
            if let Some(painted) = self.painted_badge_image(&badge.badge_info.image_4x).await {
                let info = &mut badge.badge_info;
                info.image_1x = painted.clone();
                info.image_2x = painted.clone();
                info.image_4x = painted;
            }
        }
        badges
    }

    /// The finished image for a badge whose raw image needs a colour painted
    /// behind it, as an SVG data URI: built on first use, then served from
    /// memory. `None` when the image needs no backdrop, or could not be
    /// fetched or read (the raw image is drawn instead, and the next lookup
    /// tries again). One image serves every size: at 72 px it scales down
    /// cleanly to the chat row.
    async fn painted_badge_image(&self, url: &str) -> Option<String> {
        let (key, color) = {
            let cache = self.cache.read().await;
            let color = cache.third_party.backdrops.get(url)?;
            let key = backdrop_key(url, color);
            if let Some(done) = cache.third_party.painted.get(&key) {
                return Some(done.clone());
            }
            (key, color.clone())
        };

        let response = self.http_client.get(url).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().await.ok()?;
        if bytes.len() > BACKDROP_SOURCE_MAX_BYTES {
            return None;
        }
        let data_uri = badge_with_backdrop(&bytes, &color)?;

        let mut cache = self.cache.write().await;
        // Only keep it while the feed still asks for this image and colour.
        if cache.third_party.backdrops.get(url).map(String::as_str) == Some(color.as_str()) {
            cache.third_party.painted.insert(key, data_uri.clone());
        }
        Some(data_uri)
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
        let mut deduped = third_party_gallery_tiles(&cache.third_party, viewer_user_id);
        drop(cache);

        // A Chatty tile drawn from a glyph image gets its backdrop painted.
        for tile in deduped.iter_mut().filter(|t| t.provider == BadgeProvider::Chatty) {
            if let Some(painted) = self.painted_badge_image(&tile.image_4x).await {
                tile.image_1x = painted.clone();
                tile.image_2x = painted.clone();
                tile.image_4x = painted;
            }
        }

        deduped
    }

    /// The badge spotlight's detail for one clicked third-party badge. The id
    /// may be in any of the three id spaces (`normalize_badge_ref`). Reads only
    /// the feeds in memory after the usual cached warm-up; a viewer id that is
    /// not a Twitch user id is ignored rather than refused.
    pub async fn get_badge_detail(
        &self,
        provider: &str,
        id: &str,
        title: Option<&str>,
        viewer_user_id: Option<&str>,
    ) -> Result<BadgeDetail, String> {
        let (provider, id) = normalize_badge_ref(provider, id)?;
        let title = normalize_badge_title(title)?;
        let viewer = viewer_user_id.map(str::trim).filter(|v| is_twitch_user_id(v));

        // Warm the feeds (a no-op while they are fresh).
        let _ = self.fetch_third_party_badges().await;

        let cache = self.cache.read().await;
        Ok(badge_detail_from_cache(&cache.third_party, provider, &id, title.as_deref(), viewer))
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
        cache.third_party.homies_custom = None;
        cache.third_party.homies_custom_last_updated = UNIX_EPOCH;
        cache.third_party.chatsen = None;
        cache.third_party.chatty = None;
        cache.third_party.dankchat = None;
        cache.third_party.by_user.clear();
        cache.third_party.backdrops.clear();
        cache.third_party.painted.clear();
        cache.third_party.last_updated = UNIX_EPOCH;
        cache.user_badge_strings.clear();
    }

    pub async fn clear_channel_cache(&self, channel_id: &str) {
        let mut cache = self.cache.write().await;
        cache.channel_badges.pop(channel_id);
    }
}

/// The earned badge ids once each, channel ones first and then the global
/// collection, each in the order Twitch gave it. (A set reshuffled them on
/// every answer, so a profile's badges jumped between the cached and the fresh
/// copy.)
fn earned_ids_in_order(channel: Vec<String>, global: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    channel.into_iter().chain(global).filter(|id| seen.insert(id.clone())).collect()
}

#[cfg(test)]
mod earned_order_tests {
    use super::earned_ids_in_order;

    #[test]
    fn earned_ids_keep_twitchs_order_once_each() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            earned_ids_in_order(ids(&["sub/12", "glhf/1"]), ids(&["premium/1", "glhf/1", "turbo/1"])),
            ids(&["sub/12", "glhf/1", "premium/1", "turbo/1"])
        );
        assert!(earned_ids_in_order(Vec::new(), Vec::new()).is_empty());
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
                    slot: None,
                }],
                users,
            }),
            bttv: None,
            chatterino: None,
            homies: None,
            homies_custom: None,
            homies_custom_last_updated: UNIX_EPOCH,
            chatsen: None,
            chatty: None,
            dankchat: None,
            moltorino: None,
            moltorino_last_updated: UNIX_EPOCH,
            by_user: HashMap::new(),
            backdrops: HashMap::new(),
            painted: HashMap::new(),
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

    #[test]
    fn ffz_badges_draw_the_coloured_variant() {
        assert_eq!(
            ffz_colored_badge_url("https://cdn.frankerfacez.com/badge/3/4"),
            "https://cdn.frankerfacez.com/badge/3/4/rounded"
        );
        // Already a variant, or not FFZ's badge CDN: untouched.
        assert_eq!(
            ffz_colored_badge_url("https://cdn.frankerfacez.com/badge/3/4/rounded"),
            "https://cdn.frankerfacez.com/badge/3/4/rounded"
        );
        assert_eq!(
            ffz_colored_badge_url("https://api.ffzap.com/v1/user/badge/123/3"),
            "https://api.ffzap.com/v1/user/badge/123/3"
        );
        assert_eq!(
            ffz_colored_badge_url("https://cdn.frankerfacez.com/room-badge/mod/x/2"),
            "https://cdn.frankerfacez.com/room-badge/mod/x/2"
        );
    }

    #[test]
    fn a_glyph_gets_ffzs_rounded_backdrop_behind_it() {
        // A 72 px glyph: transparent, with one opaque white pixel at (36, 36).
        let mut pixels = vec![0u8; 72 * 72 * 4];
        let centre = (36 * 72 + 36) * 4;
        pixels[centre..centre + 4].copy_from_slice(&[255, 255, 255, 255]);
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 72, 72);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&pixels).unwrap();
            writer.finish().unwrap();
        }

        use base64::Engine;
        let uri = badge_with_backdrop(&png, "#755000").expect("builds");
        // A plain PNG, which every surface's image decoder reads (no SVG).
        let body = uri.strip_prefix("data:image/png;base64,").expect("a PNG data URI");
        let bytes = base64::engine::general_purpose::STANDARD.decode(body).unwrap();
        let (w, h, out) = decode_rgba(&bytes).expect("decodes");
        assert_eq!((w, h), (72, 72));
        let at = |x: usize, y: usize| &out[(y * 72 + x) * 4..(y * 72 + x) * 4 + 4];
        // The square in the badge colour, the glyph drawn over it.
        assert_eq!(at(10, 10), &[0x75, 0x50, 0x00, 255]);
        assert_eq!(at(36, 36), &[255, 255, 255, 255]);
        // Rounded corners: the corner pixel is outside the square, an edge's middle inside.
        assert_eq!(at(0, 0)[3], 0);
        assert_eq!(at(0, 36)[3], 255);

        assert!(badge_with_backdrop(&png, "755000").is_none());
        assert!(badge_with_backdrop(&png, "#75500").is_none());
        assert!(badge_with_backdrop(&png, "#75500g").is_none());
        assert!(badge_with_backdrop(b"not a png", "#755000").is_none());
    }

    #[test]
    fn only_glyph_images_outside_ffzs_cdn_get_a_painted_backdrop() {
        let feed: Vec<ChattyBadge> = serde_json::from_str(
            r##"[
              {"id":"ffzap","version":"1","image_url":"https://api.ffzap.com/v1/user/badge/1/1",
               "image_url_2":"https://api.ffzap.com/v1/user/badge/1/2","image_url_4":"https://api.ffzap.com/v1/user/badge/1/3",
               "color":"#E60C7B","userids":["1"]},
              {"id":"ffzap","version":"2","image_url":"https://api.ffzap.com/v1/user/badge/2/1","color":null,"userids":["2"]},
              {"id":"ffz","version":"supporter","image_url":"https://cdn.frankerfacez.com/badge/3/1","color":"#755000","userids":["3"]}
            ]"##,
        )
        .unwrap();
        assert_eq!(
            feed[0].backdrop(),
            Some(("https://api.ffzap.com/v1/user/badge/1/3".to_string(), "#E60C7B".to_string()))
        );
        // Already coloured, and FFZ's own CDN (drawn finished via /rounded).
        assert_eq!(feed[1].backdrop(), None);
        assert_eq!(feed[2].backdrop(), None);
    }

    #[test]
    fn chattys_scheme_relative_badges_load_over_https() {
        let feed: Vec<ChattyBadge> = serde_json::from_str(
            r##"[{"id":"chatty","version":"supporter","image_url":"//tduva.com/img/chatty_badge_1.png",
                  "image_url_2":"//tduva.com/img/chatty_badge_2.png","userids":["1"]}]"##,
        )
        .unwrap();
        assert_eq!(
            feed[0].image_urls(),
            (
                "https://tduva.com/img/chatty_badge_1.png".to_string(),
                "https://tduva.com/img/chatty_badge_2.png".to_string(),
                "https://tduva.com/img/chatty_badge_2.png".to_string(),
            )
        );
    }

    /// The live FFZ feed's shape (2026-09): the bot badge plus a coloured one.
    const FFZ_FIXTURE: &str = r##"{
      "badges": [
        {"id": 2, "name": "bot", "title": "Bot", "color": "#595959",
         "urls": {"1": "https://cdn.frankerfacez.com/badge/2/1", "2": "https://cdn.frankerfacez.com/badge/2/2", "4": "https://cdn.frankerfacez.com/badge/2/4"}},
        {"id": 3, "name": "supporter", "title": "FFZ Supporter", "color": "#755000",
         "urls": {"1": "https://cdn.frankerfacez.com/badge/3/1", "2": "https://cdn.frankerfacez.com/badge/3/2", "4": "https://cdn.frankerfacez.com/badge/3/4"}}
      ],
      "users": {"2": [19264788, 11111], "3": [11111]}
    }"##;

    #[test]
    fn ffz_bots_get_twitchs_bot_badge_and_never_the_ffz_cog() {
        let feed: FFZBadgesResponse = serde_json::from_str(FFZ_FIXTURE).unwrap();
        let third_party = ThirdPartyCache {
            ffz: Some(feed),
            bttv: None,
            chatterino: None,
            homies: None,
            homies_custom: None,
            homies_custom_last_updated: UNIX_EPOCH,
            chatsen: None,
            chatty: None,
            dankchat: None,
            moltorino: None,
            moltorino_last_updated: UNIX_EPOCH,
            by_user: HashMap::new(),
            backdrops: HashMap::new(),
            painted: HashMap::new(),
            last_updated: UNIX_EPOCH,
        };

        // The third-party index carries the supporter badge, coloured, and no bot badge.
        let index = third_party.build_by_user_index();
        assert!(index.get("19264788").is_none());
        let held = index.get("11111").expect("supporter should resolve");
        assert_eq!(held.len(), 1);
        assert_eq!(held[0].badge_info.title, "FFZ Supporter");
        assert_eq!(held[0].badge_info.image_2x, "https://cdn.frankerfacez.com/badge/3/2/rounded");

        // The bot's Twitch row gains Twitch's own bot badge, once.
        publish_ffz_bot_ids(third_party.ffz.as_ref());
        let badge = |name: &str| crate::models::chat_layout::Badge {
            name: name.to_string(),
            version: "1".to_string(),
            image_url_1x: None,
            image_url_2x: None,
            image_url_4x: None,
            title: None,
            description: None,
        };
        let mut row = vec![badge("moderator")];
        add_ffz_bot_badge("19264788", &mut row);
        let names: Vec<&str> = row.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["moderator", "bot-badge"]);

        let mut sent_by_twitch = vec![badge("bot-badge")];
        add_ffz_bot_badge("19264788", &mut sent_by_twitch);
        assert_eq!(sent_by_twitch.len(), 1);

        let mut not_a_bot = vec![badge("subscriber")];
        add_ffz_bot_badge("55555", &mut not_a_bot);
        assert_eq!(not_a_bot.len(), 1);
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
            homies_custom: None,
            homies_custom_last_updated: UNIX_EPOCH,
            chatsen: None,
            chatty: None,
            dankchat: None,
            moltorino: Some(feed),
            moltorino_last_updated: UNIX_EPOCH,
            by_user: HashMap::new(),
            backdrops: HashMap::new(),
            painted: HashMap::new(),
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

    /// Trimmed from `itzalex.github.io/badges`: the role feed, ids as strings.
    /// 67534991 (robits) is in neither role list.
    const HOMIES_ROLES_FIXTURE: &str = r#"{
       "badges":[
          {
             "id": "1",
             "tooltip":"Homies Developer",
             "image1":"https://itzalex.github.io/badgesusers/dev/badge.png",
             "users":[ "59842770", "536387934" ],
             "image2":"https://itzalex.github.io/badgesusers/dev/badge2x.png",
             "image3":"https://itzalex.github.io/badgesusers/dev/badge3x.png"
          },
          {
             "id": "3",
             "tooltip":"Homies Supporter",
             "image1":"https://itzalex.github.io/badgesusers/supporter2/badge.png",
             "users":[ "95700563", "" ],
             "image2":"https://itzalex.github.io/badgesusers/supporter2/badge2x.png",
             "image3":"https://itzalex.github.io/badgesusers/supporter2/badge3x.png"
          }
       ]
    }"#;

    /// Trimmed from `chatterinohomies.com/api/badges/list`, plus two damaged
    /// entries (a numeric id, and one with no image) that must not sink the rest.
    const HOMIES_CUSTOM_FIXTURE: &str = r#"{"badges":[
      {"badgeFileType":"image/webp","badgeId":"68d98dd23d60203ffbfdce6a","image1":"https://cdn.chatterinohomies.com/badges/90b5d49e-b5fd-4a0a-bc92-6a74408bee82/18.webp","image2":"https://cdn.chatterinohomies.com/badges/90b5d49e-b5fd-4a0a-bc92-6a74408bee82/36.webp","image3":"https://cdn.chatterinohomies.com/badges/90b5d49e-b5fd-4a0a-bc92-6a74408bee82/72.webp","tooltip":"usVesper Badge","userId":"95700563","username":"usVesper"},
      {"badgeFileType":"image/webp","badgeId":"6ac28297411918ca99a0a2ba","image1":"https://cdn.chatterinohomies.com/badges/318e047d-a0c4-4bc1-aa01-12630be46ede/18.webp","image2":"https://cdn.chatterinohomies.com/badges/318e047d-a0c4-4bc1-aa01-12630be46ede/36.webp","image3":"https://cdn.chatterinohomies.com/badges/318e047d-a0c4-4bc1-aa01-12630be46ede/72.webp","tooltip":"Robits Badge","userId":"67534991","username":"Robits"},
      {"badgeFileType":"image/webp","badgeId":"x1","image1":"https://cdn.chatterinohomies.com/badges/a/18.webp","image2":null,"image3":null,"tooltip":null,"userId":123456,"username":"numeric"},
      {"badgeFileType":"image/webp","badgeId":"x2","image1":"","tooltip":"No Art Badge","userId":"222","username":"noart"}
    ]}"#;

    fn homies_cache(roles: &str, custom: Option<&str>) -> ThirdPartyCache {
        ThirdPartyCache {
            ffz: None,
            bttv: None,
            chatterino: None,
            homies: Some(serde_json::from_str(roles).expect("role feed parses")),
            homies_custom: custom.map(|c| homies_custom_index(serde_json::from_str(c).expect("custom feed parses"))),
            homies_custom_last_updated: UNIX_EPOCH,
            chatsen: None,
            chatty: None,
            dankchat: None,
            moltorino: None,
            moltorino_last_updated: UNIX_EPOCH,
            by_user: HashMap::new(),
            backdrops: HashMap::new(),
            painted: HashMap::new(),
            last_updated: UNIX_EPOCH,
        }
    }

    #[test]
    fn a_custom_only_homies_holder_had_no_badge_from_the_role_feeds() {
        // The role feeds alone: robits, who only has a custom badge, gets nothing.
        let index = homies_cache(HOMIES_ROLES_FIXTURE, None).build_by_user_index();
        assert!(index.get("67534991").is_none());
        assert_eq!(index.get("59842770").map(Vec::len), Some(1));
    }

    #[test]
    fn homies_custom_badge_resolves_by_twitch_id() {
        let index = homies_cache(HOMIES_ROLES_FIXTURE, Some(HOMIES_CUSTOM_FIXTURE)).build_by_user_index();

        let robits = index.get("67534991").expect("robits' custom badge resolves");
        assert_eq!(robits.len(), 1);
        let info = &robits[0].badge_info;
        assert_eq!(robits[0].provider, BadgeProvider::Homies);
        assert_eq!(info.id, HOMIES_CUSTOM_BADGE_ID);
        assert_eq!(info.title, "Robits Badge");
        assert_eq!(
            info.image_1x,
            "https://cdn.chatterinohomies.com/badges/318e047d-a0c4-4bc1-aa01-12630be46ede/18.webp"
        );
        assert_eq!(
            info.image_4x,
            "https://cdn.chatterinohomies.com/badges/318e047d-a0c4-4bc1-aa01-12630be46ede/72.webp"
        );

        // A holder of both: the custom badge first, then the role badge.
        let both: Vec<&str> = index
            .get("95700563")
            .expect("supporter resolves")
            .iter()
            .map(|b| b.badge_info.title.as_str())
            .collect();
        assert_eq!(both, ["usVesper Badge", "Homies Supporter"]);

        // A numeric id is taken; missing sizes fall back and a null tooltip gets a title.
        let numeric = &index.get("123456").expect("numeric id still resolves")[0].badge_info;
        assert_eq!(numeric.title, "Homies Badge");
        assert_eq!(numeric.image_4x, "https://cdn.chatterinohomies.com/badges/a/18.webp");
        // An entry without art never reaches the index.
        assert!(index.get("222").is_none());
    }

    #[test]
    fn homies_custom_badges_are_one_gallery_tile() {
        let custom = homies_custom_index(serde_json::from_str(HOMIES_CUSTOM_FIXTURE).unwrap());
        assert_eq!(custom.len(), 3);

        let mine = homies_custom_gallery_tile(&custom, Some("67534991")).expect("a tile");
        assert_eq!(mine.id, HOMIES_CUSTOM_BADGE_ID);
        assert_eq!(mine.provider, BadgeProvider::Homies);
        assert_eq!(mine.user_count, 3);
        assert!(mine.owned);
        assert!(mine.image_2x.contains("318e047d"));

        let theirs = homies_custom_gallery_tile(&custom, Some("1")).expect("a tile");
        assert!(!theirs.owned);
        // Stable pick: the shortest, then lowest, holder id.
        assert!(theirs.image_1x.ends_with("/a/18.webp"));

        assert!(homies_custom_gallery_tile(&HashMap::new(), None).is_none());
    }

    // One badge per wear category in chat.

    fn empty_cache() -> ThirdPartyCache {
        ThirdPartyCache {
            ffz: None,
            bttv: None,
            chatterino: None,
            homies: None,
            homies_custom: None,
            homies_custom_last_updated: UNIX_EPOCH,
            chatsen: None,
            chatty: None,
            dankchat: None,
            moltorino: None,
            moltorino_last_updated: UNIX_EPOCH,
            by_user: HashMap::new(),
            backdrops: HashMap::new(),
            painted: HashMap::new(),
            last_updated: UNIX_EPOCH,
        }
    }

    fn titles(badges: &[UserBadge]) -> Vec<&str> {
        badges.iter().map(|b| b.badge_info.title.as_str()).collect()
    }

    fn owned(index: &HashMap<String, Vec<Arc<UserBadge>>>, uid: &str) -> Vec<UserBadge> {
        index.get(uid).map(|v| v.iter().map(|e| (**e).clone()).collect()).unwrap_or_default()
    }

    fn worn(index: &HashMap<String, Vec<Arc<UserBadge>>>, uid: &str) -> Vec<UserBadge> {
        index.get(uid).map(|v| worn_third_party(v)).unwrap_or_default()
    }

    /// Trimmed from `flxrs.com/api/badges`: two "DankChat Supporter" entries
    /// (the same gnome art) around "DankChat Contributor".
    const DANKCHAT_FIXTURE: &str = r#"[
      {"type":"DuckerZ","url":"https://flxrs.com/dankchat/badges/ente.gif","users":["147950640"]},
      {"type":"DankChat Supporter","url":"https://flxrs.com/dankchat/badges/dank.png","users":["11","147950640"]},
      {"type":"DankChat Contributor","url":"https://flxrs.com/dankchat/badges/contributor.png","users":["11","22"]},
      {"type":"DankChat Supporter","url":"https://flxrs.com/dankchat/badges/dank.png","users":["22"]}
    ]"#;

    #[test]
    fn a_multi_tier_dankchat_holder_wears_one_badge() {
        let mut cache = empty_cache();
        cache.dankchat = Some(serde_json::from_str(DANKCHAT_FIXTURE).unwrap());
        let index = cache.build_by_user_index();

        // DankChat draws the first entry listing the user.
        assert_eq!(titles(&owned(&index, "11")), ["DankChat Supporter", "DankChat Contributor"]);
        assert_eq!(titles(&worn(&index, "11")), ["DankChat Supporter"]);
        assert_eq!(titles(&worn(&index, "22")), ["DankChat Contributor"]);
        assert_eq!(titles(&worn(&index, "147950640")), ["DuckerZ"]);
    }

    #[test]
    fn a_homies_custom_and_role_holder_wears_both() {
        let index = homies_cache(HOMIES_ROLES_FIXTURE, Some(HOMIES_CUSTOM_FIXTURE)).build_by_user_index();
        // The custom badge and the role list are separate Homies categories.
        assert_eq!(titles(&worn(&index, "95700563")), ["usVesper Badge", "Homies Supporter"]);
    }

    fn homies_role(tooltip: &str, users: &[&str], second_feed: bool) -> HomiesBadge {
        HomiesBadge {
            tooltip: tooltip.to_string(),
            image1: format!("https://itzalex.github.io/{tooltip}.png"),
            image2: None,
            image3: None,
            users: users.iter().map(|u| u.to_string()).collect(),
            second_feed,
        }
    }

    #[test]
    fn a_homies_custom_role_and_staff_holder_wears_three() {
        let mut cache = homies_cache(HOMIES_ROLES_FIXTURE, Some(HOMIES_CUSTOM_FIXTURE));
        cache.homies = Some(HomiesBadgesResponse {
            badges: vec![
                homies_role("Homies Developer", &["95700563"], false),
                homies_role("Homies Supporter", &["95700563"], false),
                homies_role("Homies Mod", &["95700563"], true),
                homies_role("Senior Homies Mod", &["95700563"], true),
            ],
        });
        let index = cache.build_by_user_index();

        // One from each list, in Homies' draw order: custom, /badges, /badges2,
        // and within a list the last entry listing the user.
        assert_eq!(
            titles(&worn(&index, "95700563")),
            ["usVesper Badge", "Homies Supporter", "Senior Homies Mod"]
        );
        // Everything owned is still listed.
        assert_eq!(owned(&index, "95700563").len(), 5);
    }

    #[test]
    fn two_homies_roles_from_one_list_wear_one() {
        let mut cache = empty_cache();
        cache.homies = Some(HomiesBadgesResponse {
            badges: vec![
                homies_role("Homies Developer", &["1", "2"], false),
                homies_role("Homies Supporter", &["1"], false),
                homies_role("Homies Founder", &["1"], false),
                homies_role("Homies Mod", &["3"], true),
                homies_role("Bot", &["3"], true),
            ],
        });
        let index = cache.build_by_user_index();

        assert_eq!(titles(&worn(&index, "1")), ["Homies Founder"]);
        assert_eq!(titles(&worn(&index, "2")), ["Homies Developer"]);
        assert_eq!(titles(&worn(&index, "3")), ["Bot"]);
        assert_eq!(titles(&owned(&index, "1")), ["Homies Founder", "Homies Supporter", "Homies Developer"]);
    }

    #[test]
    fn ffz_holders_wear_one_badge_per_slot() {
        let badge = |id: u32, title: &str, slot: u32| FFZBadge {
            id,
            title: Some(title.to_string()),
            name: None,
            urls: HashMap::new(),
            slot: Some(slot),
        };
        let mut users = HashMap::new();
        for id in ["3", "1", "9"] {
            users.insert(id.to_string(), vec![5u32]);
        }
        let mut cache = empty_cache();
        cache.ffz = Some(FFZBadgesResponse {
            badges: vec![badge(3, "FFZ Supporter", 5), badge(1, "FFZ Developer", 5), badge(9, "FFZ Event", 6)],
            users,
        });
        let index = cache.build_by_user_index();

        // Supporter and Developer share slot 5: the first listed is drawn.
        assert_eq!(titles(&worn(&index, "5")), ["FFZ Supporter", "FFZ Event"]);
        assert_eq!(owned(&index, "5").len(), 3);
    }

    #[test]
    fn chatterino_keeps_the_last_entry_listing_a_user() {
        let mut cache = empty_cache();
        cache.chatterino = Some(
            serde_json::from_str(
                r#"{"badges":[
                  {"tooltip":"Chatterino Supporter","image1":"a","image2":null,"image3":null,"users":["7"]},
                  {"tooltip":"Chatterino Contributor","image1":"b","image2":null,"image3":null,"users":["7"]}
                ]}"#,
            )
            .unwrap(),
        );
        let index = cache.build_by_user_index();
        assert_eq!(titles(&worn(&index, "7")), ["Chatterino Contributor"]);
    }

    #[test]
    fn different_providers_still_stack_one_each() {
        let mut cache = empty_cache();
        cache.dankchat = Some(
            serde_json::from_str(
                r#"[{"type":"DankChat Supporter","url":"d","users":["95700563"]},
                    {"type":"DankChat Contributor","url":"c","users":["95700563"]}]"#,
            )
            .unwrap(),
        );
        // Chatty re-hosts FFZ's badge (dropped by its title, FFZ carries it)
        // and adds the holder's FFZ:AP badge, a different thing from a
        // different provider, so it stays beside FFZ's own.
        cache.chatty = Some(
            serde_json::from_str(
                r#"[{"id":"ffz","version":"supporter","meta_title":"FFZ Supporter","image_url":"https://cdn.frankerfacez.com/badge/3/1","userids":["95700563"]},
                    {"id":"ffzap","version":"95700563","meta_title":"FFZ:AP Supporter","image_url":"https://api.ffzap.com/v1/user/badge/95700563/1","userids":["95700563"]}]"#,
            )
            .unwrap(),
        );
        let mut ffz_users = HashMap::new();
        ffz_users.insert("3".to_string(), vec![95700563u32]);
        cache.ffz = Some(FFZBadgesResponse {
            badges: vec![FFZBadge {
                id: 3,
                title: Some("FFZ Supporter".to_string()),
                name: Some("supporter".to_string()),
                urls: HashMap::new(),
                slot: Some(5),
            }],
            users: ffz_users,
        });
        let index = cache.build_by_user_index();

        let worn = worn(&index, "95700563");
        assert_eq!(titles(&worn), ["FFZ Supporter", "FFZ:AP Supporter", "DankChat Supporter"]);
        let providers: Vec<_> = worn.iter().map(|b| b.provider.as_key()).collect();
        assert_eq!(providers, ["ffz", "chatty", "dankchat"]);
    }

    /// The badge services setting hides a Homies custom badge by the key
    /// `homies-custom` (the frontend's `HOMIES_CUSTOM_SOURCE`), apart from
    /// the Homies role and staff badges, and the frontend reads that key from
    /// the badge's `set_id`. This pins the strings both sides compare.
    #[test]
    fn homies_custom_badges_carry_their_own_visibility_key() {
        let mut cache = homies_cache(HOMIES_ROLES_FIXTURE, Some(HOMIES_CUSTOM_FIXTURE));
        cache.homies.as_mut().unwrap().badges.push(homies_role("Homies Mod", &["95700563"], true));
        let index = cache.build_by_user_index();
        let sets: Vec<(String, String)> = owned(&index, "95700563")
            .into_iter()
            .map(|b| (b.badge_info.title, b.badge_info.set_id))
            .collect();
        assert_eq!(
            sets,
            [
                ("usVesper Badge".to_string(), "homies-custom".to_string()),
                ("Homies Supporter".to_string(), "homies".to_string()),
                ("Homies Mod".to_string(), "homies-staff".to_string()),
            ]
        );
        // Every one carries provider `homies`, so only the set_id tells them apart.
        assert!(owned(&index, "95700563").iter().all(|b| b.provider == BadgeProvider::Homies));
    }

    // ------------------------------------------------------------------
    // Badge detail (the spotlight card)
    // ------------------------------------------------------------------

    #[test]
    fn badge_ids_from_every_id_space_normalise_to_the_feed_id() {
        let n = |p: &str, id: &str| normalize_badge_ref(p, id).map(|(p, id)| (p.as_key(), id));
        // Rust path, member loadout path, bare provider id.
        assert_eq!(n("ffz", "ffz-3"), Ok(("ffz", "ffz-3".to_string())));
        assert_eq!(n("ffz", "ffz:ffz-3"), Ok(("ffz", "ffz-3".to_string())));
        assert_eq!(n("ffz", "3"), Ok(("ffz", "ffz-3".to_string())));
        // Providers arrive in any case; BetterTTV Pro is `BTTV` + `bttv-pro`.
        assert_eq!(n("BTTV", "bttv-pro"), Ok(("bttv", "bttv-pro".to_string())));
        assert_eq!(n("FFZ", "FFZ:FFZ-3"), Ok(("ffz", "ffz-3".to_string())));
        // Ids that carry spaces and their own prefix pass through.
        assert_eq!(
            n("chatterino", "chatterino-Chatterino Top Donator"),
            Ok(("chatterino", "chatterino-Chatterino Top Donator".to_string()))
        );
        assert_eq!(n("homies", "homies-custom"), Ok(("homies", "homies-custom".to_string())));
        // A colon that is not the provider prefix stays part of the id.
        assert_eq!(n("chatty", "x:y"), Ok(("chatty", "chatty-x:y".to_string())));
        // Multi-byte ids never split a character.
        assert_eq!(n("chatsen", "ü"), Ok(("chatsen", "chatsen-ü".to_string())));
    }

    #[test]
    fn badge_refs_outside_the_rules_are_refused() {
        assert!(normalize_badge_ref("twitch", "subscriber").is_err(), "Twitch is not third-party");
        assert!(normalize_badge_ref("7tv", "abc").is_err());
        assert!(normalize_badge_ref("", "ffz-3").is_err());
        assert!(normalize_badge_ref("ff z", "ffz-3").is_err());
        assert!(normalize_badge_ref("ffz", "").is_err());
        assert!(normalize_badge_ref("ffz", "ffz:").is_err());
        assert!(normalize_badge_ref("ffz", "ffz-3\n").is_ok(), "surrounding whitespace is trimmed");
        assert!(normalize_badge_ref("ffz", "ffz\u{0}3").is_err());
        assert!(normalize_badge_ref("ffz", &"9".repeat(MAX_BADGE_REF_LEN + 1)).is_err());
        assert!(normalize_badge_title(Some("a\u{7}b")).is_err());
        assert!(normalize_badge_title(Some(&"t".repeat(MAX_BADGE_REF_LEN + 1))).is_err());
        assert_eq!(normalize_badge_title(Some("  ")), Ok(None));
    }

    #[test]
    fn a_badge_the_feeds_do_not_list_still_gets_its_provider_details() {
        // BetterTTV Pro badges never appear in the contributor feed.
        let (p, id) = normalize_badge_ref("BTTV", "bttv-pro").unwrap();
        let detail = badge_detail_from_cache(&empty_cache(), p, &id, Some("BTTV Pro"), Some("12345"));
        assert_eq!(detail.provider, "bttv");
        assert_eq!(detail.provider_label, "BetterTTV");
        assert_eq!(detail.homepage, "https://betterttv.com");
        // Nothing invented: React fills title and art from the clicked badge.
        assert_eq!(detail.title, None);
        assert_eq!(detail.image_4x, None);
        assert_eq!(detail.holder_count, None);
        assert_eq!(detail.owned, None, "ownership of an unlisted badge is unknown, not false");
    }

    #[test]
    fn a_homies_custom_badge_never_takes_the_shared_gallery_tile() {
        let tp = homies_cache(HOMIES_ROLES_FIXTURE, Some(HOMIES_CUSTOM_FIXTURE));
        let (p, id) = normalize_badge_ref("homies", "homies:homies-custom").unwrap();
        // Robits clicks their own custom badge.
        let own = badge_detail_from_cache(&tp, p.clone(), &id, Some("Robits Badge"), Some("67534991"));
        assert_eq!(own.owned, Some(true));
        // The one tile stands for every custom badge: not its title, art or count.
        assert_eq!(own.title, None);
        assert_eq!(own.image_4x, None);
        assert_eq!(own.holder_count, None);
        assert_eq!(own.homepage, HOMIES_CLICK_URL);
        // Someone else's custom badge, seen by a viewer who has a different one.
        let other = badge_detail_from_cache(&tp, p.clone(), &id, Some("Robits Badge"), Some("95700563"));
        assert_eq!(other.owned, Some(false));
        // The gallery tile itself means "a Homies custom badge": its holder
        // count is true of it, and any custom badge of the viewer's counts.
        let tile = badge_detail_from_cache(&tp, p, &id, Some(HOMIES_CUSTOM_TILE_TITLE), Some("95700563"));
        assert_eq!(tile.owned, Some(true));
        assert_eq!(tile.holder_count, u32::try_from(tp.homies_custom.as_ref().unwrap().len()).ok());
        assert_eq!(tile.title, None, "the tile's art and title stay the clicked tile's own");
    }

    #[test]
    fn a_moltorino_badge_carries_its_description_and_holders() {
        let feed: MoltorinoBadgesResponse = serde_json::from_str(MOLTORINO_FIXTURE).unwrap();
        let mut tp = moltorino_cache(feed);
        tp.by_user = tp.build_by_user_index();
        let (p, id) = normalize_badge_ref("moltorino", "moltorino:moltorino-supporter").unwrap();
        let detail = badge_detail_from_cache(&tp, p, &id, None, Some("249031143"));
        assert_eq!(detail.title.as_deref(), Some("Moltorino Supporter"));
        assert_eq!(detail.description.as_deref(), Some("Thanks for supporting Moltorino."));
        assert_eq!(detail.holder_count, Some(1));
        assert_eq!(detail.owned, Some(true));
        assert!(detail.image_4x.as_deref().is_some_and(|u| u.ends_with("3x.webp?v=369")));
        // No viewer: ownership is unknown.
        let anon = badge_detail_from_cache(&tp, BadgeProvider::Moltorino, "moltorino-developer", None, None);
        assert_eq!(anon.owned, None);
        assert_eq!(anon.description.as_deref(), Some("Given to people who help build and maintain Moltorino."));
    }

    #[test]
    fn badge_detail_serializes_the_wire_shape() {
        let (p, id) = normalize_badge_ref("ffz", "ffz-3").unwrap();
        let json = serde_json::to_value(badge_detail_from_cache(&empty_cache(), p, &id, None, None)).unwrap();
        for key in [
            "provider", "provider_label", "title", "description", "image_1x", "image_2x", "image_4x",
            "click_url", "homepage", "holder_count", "owned",
        ] {
            assert!(json.get(key).is_some(), "missing {key}");
        }
        assert_eq!(json["provider"], "ffz");
    }
}

#[cfg(test)]
mod third_party_refresh_tests {
    use super::*;

    #[test]
    fn a_failing_provider_backs_off_doubling_up_to_the_window() {
        let window = Duration::from_secs(10 * 60);
        let secs = |n| third_party_retry_after(n, window).as_secs();
        assert_eq!(secs(1), 60);
        assert_eq!(secs(2), 120);
        assert_eq!(secs(3), 240);
        assert_eq!(secs(4), 480);
        assert_eq!(secs(5), 600, "capped at the normal refresh window");
        assert_eq!(secs(40), 600);
    }

    #[tokio::test]
    async fn a_caller_during_a_refresh_uses_the_loaded_set_instead_of_refetching() {
        let service = BadgeService::new("test".into());
        // A set loaded earlier and now stale: a lone caller would refresh.
        service.cache.write().await.third_party.last_updated = SystemTime::now()
            .checked_sub(Duration::from_secs(60 * 60))
            .unwrap();
        // Another caller's refresh is running.
        let running = service.third_party_refresh.lock().await;
        let answered = tokio::time::timeout(Duration::from_millis(200), service.fetch_third_party_badges()).await;
        assert!(matches!(answered, Ok(Ok(()))), "returns at once, no second refresh");
        drop(running);
    }

    #[tokio::test]
    async fn the_first_load_waits_for_the_refresh_already_running() {
        let service = BadgeService::new("test".into());
        let running = service.third_party_refresh.lock().await;
        // Nothing loaded yet: it waits for the running refresh rather than
        // starting its own.
        let waited = tokio::time::timeout(Duration::from_millis(100), service.fetch_third_party_badges()).await;
        assert!(waited.is_err(), "still waiting while the refresh holds the lock");
        // That refresh finishes and leaves a fresh set: the waiter returns at once.
        service.cache.write().await.third_party.last_updated = SystemTime::now();
        drop(running);
        let answered = tokio::time::timeout(Duration::from_millis(200), service.fetch_third_party_badges()).await;
        assert!(matches!(answered, Ok(Ok(()))));
    }
}
