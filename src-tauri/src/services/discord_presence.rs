//! Discord Rich Presence: what the status says, decided in one place.
//!
//! Three writers feed it, each through its own slot, and none clears another's:
//! the watch session (the stream or offline room on the main view), the main
//! window's player (a VOD or clip) and MultiNook. The slot that wins is shown
//! (MultiNook, then the VOD or clip, then the stream, else browsing), built
//! with the viewer's style by one pure function, and sent by one pump at most
//! five frames per 21 s. The editor's preview is built by the same function,
//! so it shows exactly what Discord gets.
//!
//! Desktop only: the phones have no Discord client to talk to.

use crate::models::settings::{AppState, DiscordPresenceSettings};
use crate::rt::AppHandle;
use crate::services::discord_service::{
    CategoryIcon, DiscordService, DiscordUser, SendError, DISCORD_CLIENT_ID,
};
use discord_rich_presence::activity::{
    Activity, ActivityType, Assets, Button, StatusDisplayType, Timestamps,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{Emitter, Manager};
use tokio::sync::Notify;
use unicode_segmentation::UnicodeSegmentation;

/// Something the editor shows changed (the scene, the delivery, a picture).
pub const CHANGED_EVENT: &str = "discord-presence://changed";
/// To the main window only: its page generation moved on; read it again and
/// resend the grid and the player.
pub const PAGE_EVENT: &str = "discord-presence://page";

const STREAMNOOK_LOGO_KEY: &str = "streamnook_logo";
/// What Discord draws for `streamnook_logo`, for the preview.
const STREAMNOOK_LOGO_URL: &str =
    "https://cdn.discordapp.com/app-assets/1524098616648663110/1524099140210921512.png";
const TWITCH_BADGE_URL: &str =
    "https://raw.githubusercontent.com/StreamNook/StreamNook/refs/heads/main/src-tauri/images/logo_1704751143960.JPG";
const BADGE_BASE: &str = "https://raw.githubusercontent.com/StreamNook/StreamNook/refs/heads/main/src-tauri/images/discord";
const PROMO_URL: &str = "https://streamnook.app";
/// The name `playMedia` gives a clip whose broadcaster Twitch did not name.
const MEDIA_PLACEHOLDER_NAME: &str = "StreamNook Media";

const GATE_FRAMES: usize = 5;
const GATE_WINDOW: Duration = Duration::from_secs(21);
const NO_PIPE_RETRY: Duration = Duration::from_secs(15);
const UNANSWERED_RETRY: Duration = Duration::from_secs(60);
const ART_WAIT: Duration = Duration::from_secs(2);
const IDLE_SETTLE: Duration = Duration::from_secs(5);
const META_GRACE: Duration = Duration::from_secs(10);
const TILE_ERROR_GRACE: Duration = Duration::from_secs(30);
const ART_RETRY: Duration = Duration::from_secs(60);
const ART_PENDING_LIMIT: Duration = Duration::from_secs(10);
const ART_CAPACITY: usize = 512;
const PREVIEW_KEEPS_ART: Duration = Duration::from_secs(30);
const AVATAR_WAIT: Duration = Duration::from_millis(1500);
const OFFLINE_ROOM_WAIT: Duration = Duration::from_secs(3);
/// The sample stream's elapsed time in the preview: 1 h 12 min.
const SAMPLE_ELAPSED_SECS: i64 = 72 * 60;

// One pool of self-contained one-liners for the browsing status.
const IDLE_PHRASES: &[&str] = &[
    "Channel surfing for poggers",
    "Lost in the Twitch jungle",
    "Hunting for the next hype train",
    "Just chatting... with myself",
    "Searching for the legendary Kappa",
    "Dodging spoilers like a pro gamer",
    "Vibing in the VODs",
    "Exploring the emote-verse",
    "Where's the 'unfollow' button for reality?",
    "AFK, but my eyes are still watching",
    "In a staring contest with my screen",
    "My brain is in emote-only mode",
    "Currently respawning...",
    "Thinking about what to raid next",
    "Powered by caffeine and good vibes",
    "Waiting for the next 'clip that!' moment",
    "Procrastinating, but make it 1080p60",
    "Avoiding responsibilities in 4K",
    "Touching grass... in a loading screen",
    "Mentally subscribed, financially undecided",
    "Emotionally invested in strangers' gameplay",
    "Living vicariously through better gamers",
    "Lurking like it's a competitive sport",
    "Watching someone else be productive",
    "My personality is just other people's streams",
    "Here for the chat, staying for the chaos",
    "One more stream, then bed (a lie)",
    "Letting the algorithm raise me",
    "Donating my watch time, not my wallet",
    "Refreshing the followed page like it's a job",
    "Professional spectator, unpaid",
    "Chat is my coworking space now",
];

// Browsing lines that roast the Discord user by name; `{}` is the name.
const PERSONALIZED_PHRASES: &[&str] = &[
    "Hi, I'm {} and I have no off button",
    "{} said 'one more stream' four streams ago",
    "{} calls this 'research'",
    "Somebody please tell {} to go outside",
    "{} is touching grass tomorrow. Allegedly.",
    "{} is rotting, but in glorious HD",
    "{} mistook watching streams for a personality",
    "{}'s watchlist is a cry for help",
    "{} is here instead of doing literally anything else",
    "{} types in chat like it pays the rent",
    "Loading {}'s entire personality... 99%",
    "{} is emotionally supported by strangers online",
    "{} would rather be here than be productive",
    "{} has strong opinions and zero invitations",
    "{} clicked 'just one video' and lost the whole day",
    "{} is why the 'are you still watching?' prompt exists",
];

// Labels for the button to streamnook.app; only the wording rotates. Each fits
// Discord's 32-character label limit.
const PROMO_LABELS: &[&str] = &[
    "Download StreamNook",
    "Join them?",
    "Get StreamNook",
    "Become one of us",
    "Ditch the browser",
    "Steal my setup",
    "yes, it's free",
    "Watch like this too",
    "Upgrade your lurking",
    "You know you want it",
    "Join the nook",
    "Lurk in style",
    "Resistance is futile",
    "One of us. One of us.",
    "Free, no catch",
    "Do it. Do it now.",
];

// ---- Facts and scenes ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SceneKind {
    #[default]
    Live,
    Offline,
    Vod,
    Clip,
}

/// One thing on screen, as the status describes it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Scene {
    pub kind: SceneKind,
    pub provider: String,
    pub login: String,
    pub user_id: String,
    pub name: String,
    pub title: String,
    pub category: String,
    /// The platform's own picture of the category (Kick ships one per row).
    pub category_art: String,
    pub avatar_url: String,
    /// Unix seconds the broadcast began, when known.
    pub started_at: Option<i64>,
    pub url: String,
}

/// A MultiNook tile as the main window's grid holds it. Raw facts only: which
/// tile leads and which ones count is decided here. Every field is required:
/// the page sends them all, and a misspelled key fails loudly.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct SlotFacts {
    pub provider: String,
    pub login: String,
    pub user_id: String,
    pub name: String,
    pub title: String,
    pub category: String,
    pub avatar_url: String,
    pub maximized: bool,
    pub minimized: bool,
    pub chat_focused: bool,
    pub offline: bool,
    pub load_error: bool,
}

/// The main window's player, as its store holds it. Every field is required.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct MediaFacts {
    /// "clip", "video" or "offline_chat" (anything else is no replay).
    pub media_type: String,
    /// A stream URL is set and it is not the offline room's placeholder.
    pub playing: bool,
    /// A VOD position is held (the offline room's "play latest broadcast").
    pub vod: bool,
    pub url: String,
    pub login: String,
    pub user_id: String,
    pub name: String,
    pub title: String,
    pub category: String,
    pub avatar_url: String,
}

/// What a grid write or a media write did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteResult {
    Applied,
    /// Not from the main window, or older than the last one: dropped.
    Ignored,
    /// From an earlier page of the main window: read the page again and resend.
    Stale,
}

// ---- The payload ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PayloadButton {
    pub label: String,
    pub url: String,
}

/// Everything sent to Discord, already within its limits.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Payload {
    /// 0 Playing, 2 Listening, 3 Watching; `None` leaves it unset (Playing).
    pub activity_type: Option<u8>,
    /// 1 the bottom line (State), 2 the top line (Details); `None` the app name.
    pub status_display: Option<u8>,
    pub details: Option<String>,
    pub details_url: Option<String>,
    pub state: Option<String>,
    /// An asset key (`streamnook_logo`) or an https URL.
    pub large_image: String,
    pub large_text: Option<String>,
    pub large_url: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    /// Unix seconds the timer counts from.
    pub start: Option<i64>,
    pub buttons: Vec<PayloadButton>,
}

impl Payload {
    pub fn to_activity(&self) -> Activity<'static> {
        let mut assets = Assets::new().large_image(self.large_image.clone());
        if let Some(text) = &self.large_text {
            assets = assets.large_text(text.clone());
        }
        if let Some(url) = &self.large_url {
            assets = assets.large_url(url.clone());
        }
        if let Some(image) = &self.small_image {
            assets = assets.small_image(image.clone());
            if let Some(text) = &self.small_text {
                assets = assets.small_text(text.clone());
            }
        }
        let mut activity = Activity::new().assets(assets);
        if let Some(details) = &self.details {
            activity = activity.details(details.clone());
        }
        if let Some(url) = &self.details_url {
            activity = activity.details_url(url.clone());
        }
        if let Some(state) = &self.state {
            activity = activity.state(state.clone());
        }
        if let Some(start) = self.start {
            activity = activity.timestamps(Timestamps::new().start(start));
        }
        activity = match self.activity_type {
            Some(0) => activity.activity_type(ActivityType::Playing),
            Some(2) => activity.activity_type(ActivityType::Listening),
            Some(3) => activity.activity_type(ActivityType::Watching),
            _ => activity,
        };
        activity = match self.status_display {
            Some(1) => activity.status_display_type(StatusDisplayType::State),
            Some(2) => activity.status_display_type(StatusDisplayType::Details),
            _ => activity,
        };
        let buttons: Vec<Button<'static>> = self
            .buttons
            .iter()
            .map(|b| Button::new(b.label.clone(), b.url.clone()))
            .collect();
        activity.buttons(buttons)
    }
}

// ---- Limits ---------------------------------------------------------------------------

/// Length the way Discord counts it (UTF-16 units).
fn units(s: &str) -> usize {
    s.encode_utf16().count()
}

/// Control characters become spaces, whitespace collapses, the ends trim.
pub(crate) fn tidy_text(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `s` within `max` units: when it is longer, whole graphemes up to the room
/// left for `tail`, then `tail`.
fn clip(s: &str, max: usize, tail: &str) -> String {
    if units(s) <= max {
        return s.to_string();
    }
    let budget = max.saturating_sub(units(tail));
    let mut out = String::new();
    let mut used = 0;
    for g in s.graphemes(true) {
        let n = units(g);
        if used + n > budget {
            break;
        }
        used += n;
        out.push_str(g);
    }
    let mut out = out.trim_end().to_string();
    out.push_str(tail);
    out
}

/// A text field: 2 to 128 units, or left out.
pub(crate) fn text_field(s: &str) -> Option<String> {
    let t = tidy_text(s);
    (units(&t) >= 2).then(|| clip(&t, 128, "..."))
}

/// A button label: 1 to 32 units, or no button.
fn label_field(s: &str) -> Option<String> {
    let t = tidy_text(s);
    (!t.is_empty()).then(|| clip(&t, 32, "…"))
}

/// An https URL of at most `max` units, or left out.
pub(crate) fn url_field(s: &str, max: usize) -> Option<String> {
    let t = s.trim();
    (t.starts_with("https://") && !t.chars().any(char::is_whitespace) && units(t) <= max)
        .then(|| t.to_string())
}

/// A picture Discord will take: https, at most 256 units, at a useful size.
pub(crate) fn usable_picture(provider: &str, url: &str) -> Option<String> {
    url_field(&larger_avatar(provider, url), 256)
}

/// Twitch avatar links name their size (`-70x70.`); YouTube's end `=sNN`.
/// Both serve 300 px, sharp at Discord's sizes.
pub(crate) fn larger_avatar(provider: &str, url: &str) -> String {
    match provider {
        "twitch" => {
            let re = TWITCH_SIZE.get_or_init(|| regex::Regex::new(r"-\d{2,4}x\d{2,4}\.").unwrap());
            re.replace(url, "-300x300.").to_string()
        }
        "youtube" => {
            let re = YOUTUBE_SIZE.get_or_init(|| regex::Regex::new(r"=s\d{2,4}").unwrap());
            re.replace(url, "=s300").to_string()
        }
        _ => url.to_string(),
    }
}

static TWITCH_SIZE: OnceLock<regex::Regex> = OnceLock::new();
static YOUTUBE_SIZE: OnceLock<regex::Regex> = OnceLock::new();

// ---- Templates ---------------------------------------------------------------------------

/// The values a template's tokens stand for.
#[derive(Debug, Clone, Default)]
pub(crate) struct Vars {
    pub streamer: String,
    pub title: String,
    pub category: String,
    pub platform: String,
    pub kind: String,
    pub count: usize,
    pub streamers: String,
}

impl Vars {
    fn get(&self, token: &str) -> Option<String> {
        Some(match token {
            "streamer" => self.streamer.clone(),
            "title" => self.title.clone(),
            "category" => self.category.clone(),
            "platform" => self.platform.clone(),
            "kind" => self.kind.clone(),
            "count" => self.count.to_string(),
            "streamers" => self.streamers.clone(),
            _ => return None,
        })
    }
}

const SEPARATORS: &[char] = &['-', '–', '—', '|', '·', '•', ':', ','];

/// Fill `template`'s tokens. An empty value takes one separator next to it
/// along (the one before it, else the one after it), so "{category} · {title}"
/// with no category reads "title", never "· title". Titles are never edited.
pub(crate) fn fill(template: &str, vars: &Vars) -> String {
    // (text, is_value)
    let mut pieces: Vec<(String, bool)> = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, after) = rest.split_at(open);
        if !before.is_empty() {
            pieces.push((before.to_string(), false));
        }
        match after[1..].find(|c: char| c == '}' || c == '{') {
            Some(i) if after.as_bytes()[1 + i] == b'}' => {
                let token = &after[1..1 + i];
                match vars.get(token) {
                    Some(value) => pieces.push((value, true)),
                    None => pieces.push((after[..2 + i].to_string(), false)),
                }
                rest = &after[2 + i..];
            }
            Some(i) => {
                pieces.push((after[..1 + i].to_string(), false));
                rest = &after[1 + i..];
            }
            None => {
                pieces.push((after.to_string(), false));
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        pieces.push((rest.to_string(), false));
    }
    for i in 0..pieces.len() {
        if !pieces[i].1 || !pieces[i].0.trim().is_empty() {
            continue;
        }
        pieces[i].0.clear();
        let took_before =
            i > 0 && !pieces[i - 1].1 && drop_trailing_separator(&mut pieces[i - 1].0);
        if !took_before && i + 1 < pieces.len() && !pieces[i + 1].1 {
            drop_leading_separator(&mut pieces[i + 1].0);
        }
    }
    tidy_text(&pieces.into_iter().map(|(t, _)| t).collect::<String>())
}

fn drop_trailing_separator(text: &mut String) -> bool {
    let trimmed = text.trim_end();
    match trimmed.chars().last() {
        Some(c) if SEPARATORS.contains(&c) => {
            let cut = trimmed.len() - c.len_utf8();
            *text = trimmed[..cut].trim_end().to_string();
            true
        }
        _ => false,
    }
}

fn drop_leading_separator(text: &mut String) -> bool {
    let trimmed = text.trim_start();
    match trimmed.chars().next() {
        Some(c) if SEPARATORS.contains(&c) => {
            *text = trimmed[c.len_utf8()..].trim_start().to_string();
            true
        }
        _ => false,
    }
}

/// Names joined ", " within 100 units, then " +N more".
pub(crate) fn join_names(names: &[String]) -> String {
    let mut out = String::new();
    for (i, name) in names.iter().enumerate() {
        let candidate = if out.is_empty() {
            name.clone()
        } else {
            format!("{out}, {name}")
        };
        let left = names.len() - i - 1;
        let tail = if left > 0 {
            units(&format!(" +{left} more"))
        } else {
            0
        };
        if !out.is_empty() && units(&candidate) + tail > 100 {
            return format!("{out} +{} more", names.len() - i);
        }
        out = candidate;
    }
    out
}

pub(crate) fn platform_label(provider: &str) -> &'static str {
    match provider {
        "kick" => "Kick",
        "youtube" => "YouTube",
        "tiktok" => "TikTok",
        _ => "Twitch",
    }
}

fn kind_label(kind: SceneKind) -> &'static str {
    match kind {
        SceneKind::Vod => "VOD",
        SceneKind::Clip => "Clip",
        SceneKind::Live | SceneKind::Offline => "Stream",
    }
}

pub(crate) fn watch_url(provider: &str, login: &str) -> String {
    if provider == "twitch" || provider.is_empty() {
        format!("https://twitch.tv/{login}")
    } else {
        crate::services::providers::watch_urls::watch_url(provider, login)
    }
}

pub(crate) fn twitch_boxart_url(category: &str) -> String {
    format!(
        "https://static-cdn.jtvnw.net/ttv-boxart/{}-288x384.jpg",
        urlencoding::encode(category)
    )
}

/// The platform badge: the uploaded asset when the app has it, else the
/// image in the public repository.
pub(crate) fn badge_image(provider: &str, assets: &HashMap<String, String>) -> Option<String> {
    let key = format!("{provider}_logo");
    if assets.contains_key(&key) {
        return Some(key);
    }
    match provider {
        "twitch" | "" => Some(TWITCH_BADGE_URL.to_string()),
        "kick" | "youtube" | "tiktok" => Some(format!("{BADGE_BASE}/{provider}.png")),
        _ => None,
    }
}

// ---- What is shown ---------------------------------------------------------------------

/// A grid of two or more counted tiles.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GridScene {
    /// The tile that leads: its streamer, picture, link and badge.
    pub featured: Scene,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Shown {
    Idle,
    Scene(Scene),
    Grid(GridScene),
}

impl Shown {
    fn scene(&self) -> Option<&Scene> {
        match self {
            Shown::Idle => None,
            Shown::Scene(s) => Some(s),
            Shown::Grid(g) => Some(&g.featured),
        }
    }
}

fn slot_scene(slot: &SlotFacts) -> Scene {
    Scene {
        kind: SceneKind::Live,
        provider: slot.provider.clone(),
        login: slot.login.clone(),
        user_id: slot.user_id.clone(),
        name: slot.name.clone(),
        title: slot.title.clone(),
        category: slot.category.clone(),
        category_art: String::new(),
        avatar_url: slot.avatar_url.clone(),
        started_at: None,
        url: watch_url(&slot.provider, &slot.login),
    }
}

fn tile_key(slot: &SlotFacts) -> String {
    format!("{}:{}", slot.provider, slot.login.to_lowercase())
}

/// Which tile leads, which ones count, and what that adds up to.
pub(crate) fn decide_grid(
    slots: &[SlotFacts],
    errors: &HashMap<String, Instant>,
    now: Instant,
) -> Option<Shown> {
    if slots.is_empty() {
        return None;
    }
    let featured = slots
        .iter()
        .position(|s| s.maximized)
        .or_else(|| slots.iter().position(|s| s.chat_focused))
        .or_else(|| slots.iter().position(|s| !s.minimized))
        .unwrap_or(0);
    let failed = |s: &SlotFacts| {
        s.load_error
            && errors
                .get(&tile_key(s))
                .is_some_and(|t| now.saturating_duration_since(*t) >= TILE_ERROR_GRACE)
    };
    let mut counted: Vec<usize> = (0..slots.len())
        .filter(|&i| !slots[i].offline && !failed(&slots[i]))
        .collect();
    if counted.is_empty() {
        counted = (0..slots.len()).collect();
    }
    if counted.len() == 1 {
        return Some(Shown::Scene(slot_scene(&slots[counted[0]])));
    }
    // The most common category among counted tiles, ties to the first in order.
    let mut tally: Vec<(String, usize)> = Vec::new();
    for &i in &counted {
        let c = slots[i].category.trim();
        if c.is_empty() {
            continue;
        }
        match tally.iter_mut().find(|(name, _)| name == c) {
            Some((_, n)) => *n += 1,
            None => tally.push((c.to_string(), 1)),
        }
    }
    let mut category = String::new();
    let mut best = 0;
    for (name, n) in tally {
        if n > best {
            best = n;
            category = name;
        }
    }
    let mut lead = slot_scene(&slots[featured]);
    lead.category = category;
    let names = counted
        .iter()
        .map(|&i| {
            if slots[i].name.is_empty() {
                slots[i].login.clone()
            } else {
                slots[i].name.clone()
            }
        })
        .collect();
    Some(Shown::Grid(GridScene {
        featured: lead,
        names,
    }))
}

/// The replay a player state describes, if any.
pub(crate) fn media_scene(m: &MediaFacts) -> Option<Scene> {
    let kind = match (m.media_type.as_str(), m.playing, m.vod) {
        ("clip", true, _) => SceneKind::Clip,
        ("video", true, _) => SceneKind::Vod,
        ("offline_chat", true, true) => SceneKind::Vod,
        _ => return None,
    };
    let name = if m.name == MEDIA_PLACEHOLDER_NAME {
        String::new()
    } else {
        m.name.clone()
    };
    Some(Scene {
        kind,
        provider: "twitch".into(),
        login: m.login.to_lowercase(),
        user_id: m.user_id.clone(),
        name,
        title: m.title.clone(),
        category: m.category.clone(),
        category_art: String::new(),
        avatar_url: m.avatar_url.clone(),
        started_at: None,
        url: m.url.clone(),
    })
}

fn scene_identity(s: &Scene) -> String {
    let login = s.login.to_lowercase();
    match s.kind {
        SceneKind::Live => format!("solo:{}:{login}", s.provider),
        SceneKind::Offline => format!("offline:{}:{login}", s.provider),
        SceneKind::Vod | SceneKind::Clip => format!("media:{}", s.url),
    }
}

fn grid_identity(slots: &[SlotFacts]) -> String {
    match slots {
        [one] => format!("solo:{}:{}", one.provider, one.login.to_lowercase()),
        _ => "grid".into(),
    }
}

/// Which tab of the editor a shown card belongs to: the one whose lines it is
/// built from (a grid down to one counted tile reads as a stream).
fn tab_of(shown: &Shown) -> &'static str {
    match shown {
        Shown::Idle => "browsing",
        Shown::Grid(_) => "grid",
        Shown::Scene(s) => match s.kind {
            SceneKind::Live => "live",
            SceneKind::Offline => "offline",
            SceneKind::Vod | SceneKind::Clip => "replay",
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Grid,
    Media,
    Solo,
    Idle,
}

// ---- The builder ---------------------------------------------------------------------------

/// Pictures found for the shown scene.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SceneArt {
    pub avatar: Option<String>,
    pub category: Option<String>,
}

pub(crate) struct BuildCtx<'a> {
    pub style: &'a DiscordPresenceSettings,
    pub art: &'a SceneArt,
    pub start: i64,
    pub seed: u64,
    pub discord_name: Option<&'a str>,
    pub assets: &'a HashMap<String, String>,
}

/// What each token stands for in this scene: the one source for the builder
/// and for the editor's token chips.
pub(crate) fn scene_vars(scene: &Scene, grid: Option<&GridScene>) -> Vars {
    let streamer = if scene.name.trim().is_empty() {
        scene.login.clone()
    } else {
        scene.name.clone()
    };
    let platform = platform_label(&scene.provider).to_string();
    let title = if scene.title.trim().is_empty() && scene.kind == SceneKind::Live && grid.is_none()
    {
        format!("Live on {platform}")
    } else {
        scene.title.clone()
    };
    Vars {
        streamer: streamer.clone(),
        title,
        category: scene.category.clone(),
        platform,
        kind: kind_label(scene.kind).to_string(),
        count: grid.map_or(1, |g| g.names.len()),
        streamers: grid.map_or(streamer, |g| join_names(&g.names)),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Large {
    Category,
    Streamer,
    Logo,
}

/// The activity for what is shown, within Discord's limits. `None` clears it
/// (browsing set to show nothing).
pub(crate) fn build(shown: &Shown, ctx: &BuildCtx) -> Option<Payload> {
    let style = ctx.style;
    let (scene, grid) = match shown {
        Shown::Idle => return build_idle(ctx),
        Shown::Scene(s) => (s, None),
        Shown::Grid(g) => (&g.featured, Some(g)),
    };
    let vars = scene_vars(scene, grid);
    let streamer = vars.streamer.clone();
    let platform = vars.platform.clone();
    let (top, bottom) = match (grid.is_some(), scene.kind) {
        (true, _) => (&style.grid_top, &style.grid_bottom),
        (false, SceneKind::Live) => (&style.watching_top, &style.watching_bottom),
        (false, SceneKind::Offline) => (&style.offline_top, &style.offline_bottom),
        (false, SceneKind::Vod | SceneKind::Clip) => (&style.replay_top, &style.replay_bottom),
    };
    let details = text_field(&fill(top, &vars));
    let state = text_field(&fill(bottom, &vars));

    // Pictures, checked against Discord's limit again: they come from caches.
    // An offline room shows no category picture (today's look: it has no
    // stream, so no category of its own).
    let avatar = ctx.art.avatar.as_deref().and_then(|u| url_field(u, 256));
    let category_pic = if scene.kind == SceneKind::Offline {
        None
    } else {
        ctx.art.category.as_deref().and_then(|u| url_field(u, 256))
    };
    let (large_image, large) = match style.large_image.as_str() {
        "streamer" => match (&avatar, &category_pic) {
            (Some(a), _) => (a.clone(), Large::Streamer),
            (None, Some(c)) => (c.clone(), Large::Category),
            (None, None) => (STREAMNOOK_LOGO_KEY.to_string(), Large::Logo),
        },
        "streamnook" => (STREAMNOOK_LOGO_KEY.to_string(), Large::Logo),
        _ => match &category_pic {
            Some(c) => (c.clone(), Large::Category),
            None => (STREAMNOOK_LOGO_KEY.to_string(), Large::Logo),
        },
    };
    let large_text = match large {
        Large::Category => text_field(&scene.category),
        Large::Streamer => text_field(&streamer),
        Large::Logo => Some("StreamNook".to_string()),
    };
    let (small_image, small_text) = match style.small_image.as_str() {
        "streamer" => (avatar.clone(), text_field(&streamer)),
        "category" => (category_pic.clone(), text_field(&scene.category)),
        "none" => (None, None),
        _ => (
            badge_image(&scene.provider, ctx.assets),
            Some(platform.clone()),
        ),
    };
    let (small_image, small_text) = match small_image {
        Some(img) if img != large_image => (Some(img), small_text),
        _ => (None, None),
    };

    let start = match style.timer.as_str() {
        "off" => None,
        "uptime" if scene.kind == SceneKind::Live && grid.is_none() => {
            Some(scene.started_at.unwrap_or(ctx.start))
        }
        _ => Some(ctx.start),
    };
    let activity_type = Some(match style.activity_type.as_str() {
        "playing" => 0,
        "listening" => 2,
        _ => 3,
    });
    let status_display = match style.status_display.as_str() {
        "top" if details.is_some() => Some(2),
        "bottom" if state.is_some() => Some(1),
        _ => None,
    };
    let link = if style.link_text {
        url_field(&scene.url, 256)
    } else {
        None
    };
    let details_url = link.clone().filter(|_| details.is_some());
    let large_url = link.filter(|_| large == Large::Streamer);

    let mut buttons = Vec::new();
    if style.watch_button {
        if let (Some(url), Some(label)) = (
            url_field(&scene.url, 512),
            label_field(&fill(&style.watch_button_label, &vars)),
        ) {
            buttons.push(PayloadButton { label, url });
        }
    }
    if style.promo_button {
        buttons.push(promo_button(ctx.seed));
    }
    buttons.truncate(2);

    Some(Payload {
        activity_type,
        status_display,
        details,
        details_url,
        state,
        large_image,
        large_text,
        large_url,
        small_image,
        small_text,
        start,
        buttons,
    })
}

fn promo_button(seed: u64) -> PayloadButton {
    let label = PROMO_LABELS[(seed % PROMO_LABELS.len() as u64) as usize];
    PayloadButton {
        label: label.to_string(),
        url: PROMO_URL.to_string(),
    }
}

fn idle_phrase(seed: u64, name: Option<&str>) -> String {
    let pick = (seed >> 1) as usize;
    match name.filter(|n| !n.trim().is_empty()) {
        Some(name) if seed % 2 == 0 => {
            PERSONALIZED_PHRASES[pick % PERSONALIZED_PHRASES.len()].replace("{}", name)
        }
        _ => IDLE_PHRASES[pick % IDLE_PHRASES.len()].to_string(),
    }
}

fn build_idle(ctx: &BuildCtx) -> Option<Payload> {
    let style = ctx.style;
    let details = match style.idle_mode.as_str() {
        "hidden" => return None,
        "custom" => text_field(&style.idle_text)
            .or_else(|| text_field(&idle_phrase(ctx.seed, ctx.discord_name))),
        _ => text_field(&idle_phrase(ctx.seed, ctx.discord_name)),
    };
    let mut buttons = Vec::new();
    if style.promo_button {
        buttons.push(promo_button(ctx.seed));
    }
    Some(Payload {
        details,
        large_image: STREAMNOOK_LOGO_KEY.to_string(),
        large_text: Some("StreamNook".to_string()),
        start: Some(ctx.start),
        buttons,
        ..Payload::default()
    })
}

// ---- Presets ----------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub blurb: &'static str,
    /// Only the fields this preset sets (the look); browsing and the promo
    /// button are never part of it, so applying it keeps the viewer's.
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// The fields a preset sets.
const LOOK_FIELDS: &[&str] = &[
    "large_image",
    "small_image",
    "activity_type",
    "status_display",
    "timer",
    "link_text",
    "watching_top",
    "watching_bottom",
    "replay_top",
    "replay_bottom",
    "grid_top",
    "grid_bottom",
    "offline_top",
    "offline_bottom",
    "watch_button",
    "watch_button_label",
];

fn look_of(style: &DiscordPresenceSettings) -> serde_json::Map<String, serde_json::Value> {
    let serde_json::Value::Object(all) = serde_json::to_value(style).unwrap_or_default() else {
        return serde_json::Map::new();
    };
    all.into_iter()
        .filter(|(k, _)| LOOK_FIELDS.contains(&k.as_str()))
        .collect()
}

/// `style` with a preset's fields applied.
pub(crate) fn apply_look(
    style: &DiscordPresenceSettings,
    fields: &serde_json::Map<String, serde_json::Value>,
) -> DiscordPresenceSettings {
    let mut value = serde_json::to_value(style).unwrap_or_default();
    if let serde_json::Value::Object(map) = &mut value {
        for (k, v) in fields {
            map.insert(k.clone(), v.clone());
        }
    }
    serde_json::from_value(value).unwrap_or_else(|_| style.clone())
}

/// Whether `style` already looks like this preset.
pub(crate) fn preset_active(style: &DiscordPresenceSettings, preset: &Preset) -> bool {
    look_of(style) == preset.fields
}

pub(crate) fn presets() -> Vec<Preset> {
    let classic = DiscordPresenceSettings::default();
    // The member list puts the verb ("Watching") before the top line, so the
    // top lines here start with the name, never a verb of their own.
    let spotlight = DiscordPresenceSettings {
        large_image: "streamer".into(),
        status_display: "top".into(),
        watching_top: "{streamer}".into(),
        replay_top: "{streamer}".into(),
        offline_top: "{streamer}'s chat".into(),
        grid_top: "{count} Streams".into(),
        ..DiscordPresenceSettings::default()
    };
    let lowkey = DiscordPresenceSettings {
        large_image: "streamnook".into(),
        watching_top: "Watching {platform}".into(),
        watching_bottom: String::new(),
        replay_top: "Watching a replay".into(),
        replay_bottom: String::new(),
        grid_top: "Watching {count} Streams".into(),
        grid_bottom: String::new(),
        offline_top: "Hanging out in chat".into(),
        offline_bottom: String::new(),
        watch_button: false,
        ..DiscordPresenceSettings::default()
    };
    [
        (
            "classic",
            "Classic",
            "Category art, platform badge, the stream title",
            classic,
        ),
        (
            "spotlight",
            "Spotlight",
            "The streamer's picture, their name in the member list",
            spotlight,
        ),
        (
            "lowkey",
            "Low-key",
            "No names or titles, just StreamNook",
            lowkey,
        ),
    ]
    .into_iter()
    .map(|(id, label, blurb, look)| Preset {
        id,
        label,
        blurb,
        fields: look_of(&look),
    })
    .collect()
}

// ---- State --------------------------------------------------------------------------------------

struct Meta {
    start: i64,
    seed: u64,
    unheld_since: Option<Instant>,
}

enum ArtEntry {
    Found(String),
    /// Used now, looked up again after `PROVISIONAL_RETRY`.
    Provisional(String, Instant),
    Missing(Instant),
    Pending(Instant),
}

/// How long a provisional picture is used before it is looked up again
/// (Discord's game list retries after five minutes).
const PROVISIONAL_RETRY: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
struct ArtCache {
    map: HashMap<String, ArtEntry>,
    order: VecDeque<String>,
}

impl ArtCache {
    fn found(&self, key: &str) -> Option<String> {
        match self.map.get(key) {
            Some(ArtEntry::Found(url)) | Some(ArtEntry::Provisional(url, _)) => Some(url.clone()),
            _ => None,
        }
    }

    /// A lookup that never answered within 10 s is started again; a miss is
    /// retried after a minute; a provisional picture after five.
    fn needs_lookup(&self, key: &str, now: Instant) -> bool {
        match self.map.get(key) {
            None => true,
            Some(ArtEntry::Found(_)) => false,
            Some(ArtEntry::Provisional(_, at)) => {
                now.saturating_duration_since(*at) >= PROVISIONAL_RETRY
            }
            Some(ArtEntry::Missing(at)) => now.saturating_duration_since(*at) >= ART_RETRY,
            Some(ArtEntry::Pending(at)) => now.saturating_duration_since(*at) >= ART_PENDING_LIMIT,
        }
    }

    fn pending_since(&self, key: &str) -> Option<Instant> {
        match self.map.get(key) {
            Some(ArtEntry::Pending(at)) => Some(*at),
            _ => None,
        }
    }

    /// When this key may be looked up again: a miss after a minute, a lookup
    /// that never answered after ten seconds.
    fn retry_at(&self, key: &str) -> Option<Instant> {
        match self.map.get(key) {
            Some(ArtEntry::Missing(at)) => Some(*at + ART_RETRY),
            Some(ArtEntry::Pending(at)) => Some(*at + ART_PENDING_LIMIT),
            Some(ArtEntry::Provisional(_, at)) => Some(*at + PROVISIONAL_RETRY),
            _ => None,
        }
    }

    fn put(&mut self, key: String, entry: ArtEntry, keep: &HashSet<String>) {
        if self.map.insert(key.clone(), entry).is_none() {
            self.order.push_back(key);
        }
        let mut guard = self.order.len();
        while self.map.len() > ART_CAPACITY && guard > 0 {
            guard -= 1;
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if keep.contains(&oldest) {
                self.order.push_back(oldest);
            } else {
                self.map.remove(&oldest);
            }
        }
    }
}

/// What Discord holds, as far as the pump knows.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) enum Delivered {
    #[default]
    Off,
    Unknown,
    /// `None`: the activity was cleared.
    Sent(Option<Payload>),
}

#[derive(Default)]
struct Presence {
    enabled: bool,
    style: DiscordPresenceSettings,
    solo: Option<Scene>,
    media: Option<MediaFacts>,
    media_seq: u64,
    grid: Option<Vec<SlotFacts>>,
    grid_seq: u64,
    gen: u64,
    tile_errors: HashMap<String, Instant>,
    meta: HashMap<String, Meta>,
    art: ArtCache,
    /// The shown scene's picture keys: never evicted from `art`.
    shown_keys: HashSet<String>,
    assets: Arc<HashMap<String, String>>,
    assets_checked: Option<Instant>,
    /// The asset list is being read (the first send waits for it, up to 2 s).
    assets_pending: Option<Instant>,
    preview_seen: Option<Instant>,
    discord_user: Option<DiscordUser>,
    connected: bool,
    delivered: Delivered,
    last_shown: String,
    idle_since: Option<Instant>,
    rev: u64,
}

static PRESENCE: Lazy<Mutex<Presence>> = Lazy::new(|| Mutex::new(Presence::default()));
static PUMP_WAKE: Notify = Notify::const_new();
static APP: OnceLock<AppHandle> = OnceLock::new();

fn lock() -> MutexGuard<'static, Presence> {
    PRESENCE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Wake the pump: something it builds from changed.
pub fn wake() {
    PUMP_WAKE.notify_one();
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A seed for the scene's phrase and promo label: varied, no RNG needed.
fn fresh_seed() -> u64 {
    static STEP: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut z = nanos ^ STEP.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Presence {
    /// What is shown, where it comes from, and its identity.
    fn shown(&self, now: Instant) -> (Shown, Source, String) {
        if let Some(slots) = self.grid.as_ref().filter(|s| !s.is_empty()) {
            if let Some(shown) = decide_grid(slots, &self.tile_errors, now) {
                return (shown, Source::Grid, grid_identity(slots));
            }
        }
        if let Some(scene) = self.media.as_ref().and_then(media_scene) {
            let id = scene_identity(&scene);
            return (Shown::Scene(scene), Source::Media, id);
        }
        if let Some(scene) = &self.solo {
            return (
                Shown::Scene(scene.clone()),
                Source::Solo,
                scene_identity(scene),
            );
        }
        (Shown::Idle, Source::Idle, "idle".into())
    }

    /// Keep start and seed for every identity a slot holds or that is shown;
    /// forget one only after it has been held by nobody for 10 s, so a
    /// handoff between two writers keeps its timer in either order.
    fn touch_meta(&mut self, shown_identity: &str, now: Instant) {
        let mut held: HashSet<String> = HashSet::new();
        held.insert(shown_identity.to_string());
        if let Some(slots) = self.grid.as_ref().filter(|s| !s.is_empty()) {
            held.insert(grid_identity(slots));
        }
        if let Some(scene) = self.media.as_ref().and_then(media_scene) {
            held.insert(scene_identity(&scene));
        }
        if let Some(scene) = &self.solo {
            held.insert(scene_identity(scene));
        }
        let start = unix_now();
        for id in &held {
            let meta = self.meta.entry(id.clone()).or_insert_with(|| Meta {
                start,
                seed: fresh_seed(),
                unheld_since: None,
            });
            meta.unheld_since = None;
        }
        for (id, meta) in self.meta.iter_mut() {
            if !held.contains(id) && meta.unheld_since.is_none() {
                meta.unheld_since = Some(now);
            }
        }
        self.meta.retain(|_, m| {
            m.unheld_since
                .is_none_or(|t| now.saturating_duration_since(t) < META_GRACE)
        });
        if shown_identity == "idle" {
            if self.last_shown != "idle" && !self.last_shown.is_empty() {
                self.idle_since = Some(now);
            }
        } else {
            self.idle_since = None;
        }
        self.last_shown = shown_identity.to_string();
    }

    /// The pictures found so far for `scene`.
    fn scene_art(&self, scene: &Scene) -> SceneArt {
        let avatar = usable_picture(&scene.provider, &scene.avatar_url)
            .or_else(|| self.art.found(&avatar_key(scene)));
        let category = if scene.category.trim().is_empty() {
            None
        } else {
            url_field(&scene.category_art, 256).or_else(|| self.art.found(&category_key(scene)))
        };
        SceneArt { avatar, category }
    }

    /// The earliest moment still ahead when a deadline the pump waits on comes
    /// due: a failing tile reaching its 30 s, or an unheld identity its 10 s.
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let tiles = self.tile_errors.values().map(|t| *t + TILE_ERROR_GRACE);
        let metas = self
            .meta
            .values()
            .filter_map(|m| m.unheld_since.map(|t| t + META_GRACE));
        tiles.chain(metas).filter(|t| *t > now).min()
    }
}

fn avatar_key(scene: &Scene) -> String {
    if scene.user_id.trim().is_empty() {
        format!(
            "avatar:{}:login:{}",
            scene.provider,
            scene.login.to_lowercase()
        )
    } else {
        format!("avatar:{}:id:{}", scene.provider, scene.user_id.trim())
    }
}

fn category_key(scene: &Scene) -> String {
    format!(
        "cat:{}:{}",
        scene.provider,
        scene.category.trim().to_lowercase()
    )
}

// ---- Writers --------------------------------------------------------------------------------------

/// The watch session's slot: the stream or offline room on the main view.
/// Called inside the session's own lock, so it always equals the session.
pub fn set_solo(scene: Option<Scene>) {
    let changed = {
        let mut p = lock();
        if p.solo == scene {
            false
        } else {
            p.solo = scene;
            p.rev += 1;
            true
        }
    };
    if changed {
        wake();
    }
}

/// The main window's current page generation.
pub fn page() -> u64 {
    lock().gen
}

/// The main window's page ended (a reload started, or the window is gone):
/// whatever it declared goes, and anything it still sends is stale.
pub fn page_ended() {
    {
        let mut p = lock();
        p.gen += 1;
        p.grid = None;
        p.grid_seq = 0;
        p.media = None;
        p.media_seq = 0;
        p.tile_errors.clear();
        p.rev += 1;
    }
    wake();
    // A write the new page made before this ran was just cleared: tell it to
    // resend. A page that has not listened yet reads the new generation itself.
    if let Some(app) = APP.get() {
        let _ = app.emit_to("main", PAGE_EVENT, ());
    }
}

fn check_write(label: &str, gen: u64, current: u64) -> Option<WriteResult> {
    if label != "main" {
        return Some(WriteResult::Ignored);
    }
    if gen != current {
        return Some(WriteResult::Stale);
    }
    None
}

/// MultiNook's tiles, from the main window (`None` when the grid is closed).
pub fn set_grid(label: &str, gen: u64, seq: u64, slots: Option<Vec<SlotFacts>>) -> WriteResult {
    let now = Instant::now();
    let changed = {
        let mut p = lock();
        if let Some(refused) = check_write(label, gen, p.gen) {
            return refused;
        }
        if seq <= p.grid_seq {
            return WriteResult::Ignored;
        }
        p.grid_seq = seq;
        let slots = slots.filter(|s| !s.is_empty());
        // A tile's error counts from when it was first reported.
        let mut errors = std::mem::take(&mut p.tile_errors);
        if let Some(list) = &slots {
            let failing: HashSet<String> =
                list.iter().filter(|s| s.load_error).map(tile_key).collect();
            errors.retain(|k, _| failing.contains(k));
            for key in failing {
                errors.entry(key).or_insert(now);
            }
        } else {
            errors.clear();
        }
        let errors_changed = errors != p.tile_errors;
        p.tile_errors = errors;
        if p.grid == slots && !errors_changed {
            false
        } else {
            p.grid = slots;
            p.rev += 1;
            true
        }
    };
    if changed {
        wake();
    }
    WriteResult::Applied
}

/// The main window's player (`None` when it plays nothing worth naming).
pub fn set_media(label: &str, gen: u64, seq: u64, media: Option<MediaFacts>) -> WriteResult {
    let changed = {
        let mut p = lock();
        if let Some(refused) = check_write(label, gen, p.gen) {
            return refused;
        }
        if seq <= p.media_seq {
            return WriteResult::Ignored;
        }
        p.media_seq = seq;
        if p.media == media {
            false
        } else {
            p.media = media;
            p.rev += 1;
            true
        }
    };
    if changed {
        wake();
    }
    WriteResult::Applied
}

/// The switch or the style changed (any window's save). Reads the settings
/// Rust holds, never the caller's copy, so two saves cannot apply out of order.
pub fn refresh() {
    let Some(app) = APP.get() else { return };
    let read = {
        let Some(state) = app.try_state::<AppState>() else {
            log::warn!("[Discord] presence: settings not ready");
            return;
        };
        let Ok(settings) = state.settings.lock() else {
            return;
        };
        (
            settings.discord_rpc_enabled,
            settings.discord_presence.clone(),
        )
    };
    let changed = {
        let mut p = lock();
        if p.enabled == read.0 && p.style == read.1 {
            false
        } else {
            p.enabled = read.0;
            p.style = read.1;
            p.rev += 1;
            true
        }
    };
    if changed {
        wake();
    }
}

/// Start presence: read the settings and run the pump.
pub fn init(app: &AppHandle) {
    if APP.set(app.clone()).is_err() {
        return;
    }
    refresh();
    tauri::async_runtime::spawn(pump(app.clone()));
    wake();
}

// ---- Art lookups -----------------------------------------------------------------------------------

/// Pictures are looked up only while presence is on, or while the editor has
/// asked for a preview in the last 30 s: a switched-off feature costs no network.
pub(crate) fn lookups_wanted(enabled: bool, preview_seen: Option<Instant>, now: Instant) -> bool {
    enabled || preview_seen.is_some_and(|t| now.saturating_duration_since(t) < PREVIEW_KEEPS_ART)
}

/// Start a lookup for each picture `scene` can use that is neither found nor
/// being looked up, when `lookups_wanted`.
fn spawn_lookups(p: &mut Presence, app: &AppHandle, scene: &Scene, now: Instant) {
    if !lookups_wanted(p.enabled, p.preview_seen, now) {
        return;
    }
    let keep: HashSet<String> = [avatar_key(scene), category_key(scene)]
        .into_iter()
        .collect();
    if usable_picture(&scene.provider, &scene.avatar_url).is_none() {
        let key = avatar_key(scene);
        if p.art.needs_lookup(&key, now) {
            p.art.put(key.clone(), ArtEntry::Pending(now), &keep);
            let (app, scene) = (app.clone(), scene.clone());
            tauri::async_runtime::spawn(async move {
                let found = find_avatar(&app, &scene).await;
                land(key, found);
            });
        }
    }
    if !scene.category.trim().is_empty() && url_field(&scene.category_art, 256).is_none() {
        let key = category_key(scene);
        if p.art.needs_lookup(&key, now) {
            p.art.put(key.clone(), ArtEntry::Pending(now), &keep);
            let category = scene.category.clone();
            tauri::async_runtime::spawn(async move {
                let (found, settled) = find_category_art(&category).await;
                land_category(key, found, settled);
            });
        }
    }
    if p.assets_checked
        .is_none_or(|t| now.saturating_duration_since(t) >= Duration::from_secs(5 * 60))
    {
        p.assets_checked = Some(now);
        p.assets_pending = Some(now);
        tauri::async_runtime::spawn(async {
            let ids = DiscordService::asset_ids().await;
            {
                let mut p = lock();
                p.assets = ids;
                p.assets_pending = None;
            }
            wake();
        });
    }
}

fn land(key: String, found: Option<String>) {
    {
        let mut p = lock();
        let entry = match found {
            Some(url) => ArtEntry::Found(url),
            None => ArtEntry::Missing(Instant::now()),
        };
        let keep = p.shown_keys.clone();
        p.art.put(key, entry, &keep);
    }
    wake();
}

/// A category picture found while Discord's game list was unreadable is used
/// now and looked up again later, when the list may have the real game.
fn land_category(key: String, found: Option<String>, settled: bool) {
    if settled || found.is_none() {
        return land(key, found);
    }
    {
        let mut p = lock();
        let keep = p.shown_keys.clone();
        p.art.put(
            key,
            ArtEntry::Provisional(found.unwrap_or_default(), Instant::now()),
            &keep,
        );
    }
    wake();
}

async fn find_avatar(app: &AppHandle, scene: &Scene) -> Option<String> {
    use crate::services::channel_avatars::{wait_for, AvatarRequest};
    let provider = scene.provider.as_str();
    let request = match provider {
        "twitch" if !scene.user_id.is_empty() => Some(scene.user_id.clone()),
        "kick" if !scene.login.is_empty() => Some(scene.login.to_lowercase()),
        "youtube" if scene.user_id.starts_with("UC") => Some(scene.user_id.clone()),
        _ => None,
    };
    if let Some(id) = request {
        let found = wait_for(
            app,
            vec![AvatarRequest {
                provider: provider.to_string(),
                id,
            }],
            AVATAR_WAIT,
        )
        .await;
        if let Some(url) = found.values().find_map(|u| usable_picture(provider, u)) {
            return Some(url);
        }
    }
    match provider {
        "twitch" if !scene.login.is_empty() => {
            let room = tokio::time::timeout(
                OFFLINE_ROOM_WAIT,
                crate::services::offline_room::open(&scene.login),
            )
            .await
            .ok()?
            .ok()?;
            room.avatar_url.and_then(|u| usable_picture("twitch", &u))
        }
        "tiktok" => crate::services::providers::tiktok::channel_meta(&scene.login)
            .and_then(|m| m.profile_pic)
            .and_then(|u| usable_picture("tiktok", &u)),
        _ => None,
    }
}

/// The category's picture, and whether that answer is final (Discord's game
/// list was read). Box art only counts when Twitch serves it without a
/// redirect to its placeholder, and only within Discord's URL limit.
async fn find_category_art(category: &str) -> (Option<String>, bool) {
    let settled = match DiscordService::category_icon(category).await {
        CategoryIcon::Match(icon) => return (url_field(&icon, 256), true),
        CategoryIcon::NoMatch => true,
        CategoryIcon::Unavailable => false,
    };
    let Some(boxart) = url_field(&twitch_boxart_url(category), 256) else {
        return (None, settled);
    };
    match DiscordService::image_exists(&boxart).await {
        Some(true) => (Some(boxart), settled),
        _ => (None, settled),
    }
}

// ---- The pump ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Step {
    Shutdown,
    Connect,
    Send,
    Wait(Instant),
    Idle,
}

#[derive(Debug, Default)]
pub(crate) struct PumpState {
    pub delivered: Delivered,
    pub gate: VecDeque<Instant>,
    pub retry_at: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct View {
    pub enabled: bool,
    /// Pictures are being looked up (on, or the editor is open).
    pub art_wanted: bool,
    pub busy: bool,
    pub connected: bool,
    pub payload: Option<Payload>,
    /// Set while the shown scene just dropped to browsing.
    pub idle_since: Option<Instant>,
    /// A picture of the shown scene is being looked up; wait for it until then.
    pub art_wait_until: Option<Instant>,
    /// The next tile-error or meta deadline.
    pub later: Option<Instant>,
}

fn gate_opens(gate: &VecDeque<Instant>) -> Option<Instant> {
    (gate.len() >= GATE_FRAMES).then(|| gate[gate.len() - GATE_FRAMES] + GATE_WINDOW)
}

fn record(gate: &mut VecDeque<Instant>, at: Instant) {
    gate.push_back(at);
    while gate.len() > GATE_FRAMES {
        gate.pop_front();
    }
}

/// What the pump does next. Pure: the tests drive it with made-up instants.
pub(crate) fn next(s: &PumpState, v: &View, now: Instant) -> Step {
    if !v.enabled {
        if s.delivered != Delivered::Off {
            return Step::Shutdown;
        }
        // Off, but the editor is open: its pictures still retry on time.
        return match v.later.filter(|t| *t > now) {
            Some(t) if v.art_wanted => Step::Wait(t),
            _ => Step::Idle,
        };
    }
    if v.busy {
        return Step::Idle;
    }
    if !v.connected {
        return match s.retry_at {
            Some(at) if at > now => Step::Wait(at),
            _ => Step::Connect,
        };
    }
    if let Some(until) = v.art_wait_until.filter(|u| *u > now) {
        return Step::Wait(until);
    }
    if let Some(end) = v.idle_since.map(|t| t + IDLE_SETTLE).filter(|e| *e > now) {
        return Step::Wait(end);
    }
    if s.delivered == Delivered::Sent(v.payload.clone()) {
        return v.later.filter(|t| *t > now).map_or(Step::Idle, Step::Wait);
    }
    if let Some(open) = gate_opens(&s.gate).filter(|o| *o > now) {
        return Step::Wait(open);
    }
    Step::Send
}

fn retry_for(error: SendError, now: Instant, backoff: Option<Instant>) -> Option<Instant> {
    match error {
        SendError::NoPipe => Some(now + NO_PIPE_RETRY),
        SendError::Unanswered | SendError::BackingOff => {
            Some(backoff.unwrap_or(now + UNANSWERED_RETRY))
        }
        SendError::Busy => None,
    }
}

/// What the editor shows depends on these; a change emits `CHANGED_EVENT`.
#[derive(Debug, Clone, PartialEq)]
struct Mark {
    rev: u64,
    delivered: Delivered,
    art: SceneArt,
    connected: bool,
    user: Option<DiscordUser>,
}

async fn pump(app: AppHandle) {
    let mut st = PumpState::default();
    let mut seen_rev = u64::MAX;
    let mut emitted: Option<Mark> = None;
    let mut failing = false;
    loop {
        let now = Instant::now();
        let (view, mark) = {
            let mut p = lock();
            let (shown, _, identity) = p.shown(now);
            p.touch_meta(&identity, now);
            if p.rev != seen_rev {
                seen_rev = p.rev;
                // A change retries at once instead of waiting out the backoff.
                st.retry_at = None;
            }
            let art = shown.scene().map(|s| p.scene_art(s)).unwrap_or_default();
            let (art_wait_until, art_retry) = match shown.scene() {
                Some(scene) => {
                    let keys = [avatar_key(scene), category_key(scene)];
                    p.shown_keys = keys.iter().cloned().collect();
                    spawn_lookups(&mut p, &app, scene, now);
                    let wait = keys
                        .iter()
                        .filter_map(|k| p.art.pending_since(k))
                        .chain(p.assets_pending)
                        .map(|t| t + ART_WAIT)
                        .filter(|t| *t > now)
                        .min();
                    let retry = keys
                        .iter()
                        .filter_map(|k| p.art.retry_at(k))
                        .filter(|t| *t > now)
                        .min();
                    (wait, retry)
                }
                None => {
                    p.shown_keys.clear();
                    (None, None)
                }
            };
            let meta = p.meta.get(&identity);
            let (start, seed) = meta.map_or((unix_now(), 0), |m| (m.start, m.seed));
            let name = p.discord_user.as_ref().map(|u| u.name.clone());
            let ctx = BuildCtx {
                style: &p.style,
                art: &art,
                start,
                seed,
                discord_name: name.as_deref(),
                assets: &p.assets,
            };
            let payload = build(&shown, &ctx);
            let connected = DiscordService::connected();
            p.connected = connected;
            p.delivered = st.delivered.clone();
            let view = View {
                enabled: p.enabled,
                art_wanted: lookups_wanted(p.enabled, p.preview_seen, now),
                busy: DiscordService::busy(),
                connected,
                payload,
                idle_since: p.idle_since,
                art_wait_until,
                later: [p.next_deadline(now), art_retry]
                    .into_iter()
                    .flatten()
                    .min(),
            };
            let mark = Mark {
                rev: p.rev,
                delivered: st.delivered.clone(),
                art,
                connected,
                user: p.discord_user.clone(),
            };
            (view, mark)
        };
        if emitted.as_ref() != Some(&mark) {
            let _ = app.emit(CHANGED_EVENT, ());
            emitted = Some(mark);
        }
        let step = next(&st, &view, now);
        match step {
            Step::Shutdown => {
                DiscordService::shutdown().await;
                st.delivered = Delivered::Off;
                st.retry_at = None;
                // Counted toward Discord's limit, never waited on.
                record(&mut st.gate, Instant::now());
            }
            Step::Connect => match DiscordService::connect().await {
                Ok(user) => {
                    st.retry_at = None;
                    st.delivered = Delivered::Unknown;
                    failing = false;
                    let mut p = lock();
                    if user.is_some() && p.discord_user != user {
                        p.discord_user = user;
                    }
                }
                Err(e) => {
                    st.delivered = Delivered::Unknown;
                    st.retry_at =
                        retry_for(e, Instant::now(), DiscordService::backoff_until().await);
                    note_failure(&mut failing, e);
                }
            },
            Step::Send => {
                let payload = view.payload.clone();
                match DiscordService::send(payload.as_ref().map(Payload::to_activity)).await {
                    Ok(()) => {
                        st.delivered = Delivered::Sent(payload);
                        record(&mut st.gate, Instant::now());
                        failing = false;
                        // A reconnect inside the send may have reached another account.
                        let user = DiscordService::user().await;
                        let mut p = lock();
                        if user.is_some() && p.discord_user != user {
                            p.discord_user = user;
                        }
                    }
                    Err(e) => {
                        st.delivered = Delivered::Unknown;
                        st.retry_at =
                            retry_for(e, Instant::now(), DiscordService::backoff_until().await);
                        note_failure(&mut failing, e);
                    }
                }
            }
            Step::Wait(until) => {
                tokio::select! {
                    _ = PUMP_WAKE.notified() => {}
                    _ = tokio::time::sleep_until(tokio::time::Instant::from_std(until)) => {}
                }
            }
            Step::Idle => PUMP_WAKE.notified().await,
        }
    }
}

fn note_failure(failing: &mut bool, e: SendError) {
    if e == SendError::Busy {
        return;
    }
    if *failing {
        log::debug!("[Discord] presence not delivered: {e}");
    } else {
        log::info!("[Discord] presence not delivered: {e}");
        *failing = true;
    }
}

// ---- Preview ------------------------------------------------------------------------------------------

/// A channel the preview may use as its sample.
#[derive(Debug, Clone)]
pub struct SampleChannel {
    pub provider: String,
    pub channel: String,
    pub name: String,
    pub avatar: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewActivity {
    /// "Watching", "Playing" or "Listening to".
    pub verb: String,
    pub app_name: String,
    /// The line the member list shows: "Watching StreamNook" or the chosen line.
    pub member_line: String,
    pub details: Option<String>,
    pub details_url: Option<String>,
    pub state: Option<String>,
    pub large_image: String,
    pub large_text: Option<String>,
    pub small_image: Option<String>,
    pub small_text: Option<String>,
    pub start: Option<i64>,
    pub buttons: Vec<PayloadButton>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenValue {
    pub token: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewModel {
    pub activity: Option<PreviewActivity>,
    /// The tab Discord shows now: live, replay, grid, offline or browsing.
    pub shown: &'static str,
    /// The requested tab is what Discord shows now.
    pub live: bool,
    /// off, connected, connecting (Discord open, status on its way) or waiting (Discord closed).
    pub connection: &'static str,
    pub user: Option<DiscordUser>,
    pub tokens: Vec<TokenValue>,
    pub presets: Vec<Preset>,
}

/// A picture as the preview can draw it.
fn display_image(image: &str, assets: &HashMap<String, String>) -> String {
    if image.starts_with("https://") {
        return image.to_string();
    }
    if image == STREAMNOOK_LOGO_KEY {
        return STREAMNOOK_LOGO_URL.to_string();
    }
    assets
        .get(image)
        .map(|id| format!("https://cdn.discordapp.com/app-assets/{DISCORD_CLIENT_ID}/{id}.png"))
        .unwrap_or_default()
}

fn preview_activity(payload: &Payload, assets: &HashMap<String, String>) -> PreviewActivity {
    let verb = match payload.activity_type {
        Some(2) => "Listening to",
        Some(3) => "Watching",
        _ => "Playing",
    };
    let shown = match payload.status_display {
        Some(2) => payload.details.clone(),
        Some(1) => payload.state.clone(),
        _ => None,
    };
    PreviewActivity {
        verb: verb.to_string(),
        app_name: "StreamNook".to_string(),
        member_line: format!(
            "{verb} {}",
            shown.unwrap_or_else(|| "StreamNook".to_string())
        ),
        details: payload.details.clone(),
        details_url: payload.details_url.clone(),
        state: payload.state.clone(),
        large_image: display_image(&payload.large_image, assets),
        large_text: payload.large_text.clone(),
        small_image: payload
            .small_image
            .as_deref()
            .map(|i| display_image(i, assets))
            .filter(|u| !u.is_empty()),
        small_text: payload.small_text.clone(),
        start: payload.start,
        buttons: payload.buttons.clone(),
    }
}

/// The sample streams for a tab Discord is not showing: up to three of the
/// viewer's favorites, then StreamNook and stand-ins, so a sample grid always
/// shows three distinct names.
fn samples(favorites: &[SampleChannel]) -> Vec<SampleChannel> {
    let mut list: Vec<SampleChannel> = favorites.iter().take(3).cloned().collect();
    let fillers = [
        ("streamnook", "StreamNook"),
        ("second", "Second stream"),
        ("third", "Third stream"),
    ];
    for (channel, name) in fillers {
        if list.len() >= 3 {
            break;
        }
        if list.iter().any(|c| c.name == name) {
            continue;
        }
        list.push(SampleChannel {
            provider: "twitch".into(),
            channel: channel.into(),
            name: name.into(),
            avatar: STREAMNOOK_LOGO_URL.into(),
        });
    }
    list
}

fn sample_scene(c: &SampleChannel, kind: SceneKind) -> Scene {
    Scene {
        kind,
        provider: c.provider.clone(),
        login: c.channel.clone(),
        user_id: String::new(),
        name: c.name.clone(),
        title: "Your stream title shows here".into(),
        category: "Just Chatting".into(),
        category_art: twitch_boxart_url("Just Chatting"),
        avatar_url: c.avatar.clone(),
        started_at: None,
        url: watch_url(&c.provider, &c.channel),
    }
}

/// The editor's token chips, with what each stands for in the card shown.
fn tokens_for(shown: &Shown) -> Vec<TokenValue> {
    let vars = match shown {
        Shown::Idle => return Vec::new(),
        Shown::Scene(s) => scene_vars(s, None),
        Shown::Grid(g) => scene_vars(&g.featured, Some(g)),
    };
    [
        ("{streamer}", vars.streamer.clone()),
        ("{title}", vars.title.clone()),
        ("{category}", vars.category.clone()),
        ("{platform}", vars.platform.clone()),
        ("{kind}", vars.kind.clone()),
        ("{count}", vars.count.to_string()),
        ("{streamers}", vars.streamers.clone()),
    ]
    .into_iter()
    .map(|(t, v)| TokenValue {
        token: t.into(),
        value: v,
    })
    .collect()
}

/// The editor's card for `tab` with `style` (the draft, not yet saved).
pub fn preview(
    style: &DiscordPresenceSettings,
    tab: &str,
    favorites: &[SampleChannel],
) -> PreviewModel {
    let now = Instant::now();
    let mut p = lock();
    let first_look = p
        .preview_seen
        .is_none_or(|t| now.saturating_duration_since(t) >= PREVIEW_KEEPS_ART);
    p.preview_seen = Some(now);
    let (live_shown, _, identity) = p.shown(now);
    let shown_tab = tab_of(&live_shown);
    let live = shown_tab == tab;
    let (shown, art, start, seed) = if live {
        let art = live_shown
            .scene()
            .map(|s| p.scene_art(s))
            .unwrap_or_default();
        let (start, seed) = p
            .meta
            .get(&identity)
            .map_or((unix_now(), 0), |m| (m.start, m.seed));
        (live_shown, art, start, seed)
    } else {
        let list = samples(favorites);
        let shown = match tab {
            "grid" => Shown::Grid(GridScene {
                featured: sample_scene(&list[0], SceneKind::Live),
                names: list.iter().map(|c| c.name.clone()).collect(),
            }),
            "replay" => Shown::Scene(sample_scene(&list[0], SceneKind::Vod)),
            "offline" => Shown::Scene(sample_scene(&list[0], SceneKind::Offline)),
            "browsing" => Shown::Idle,
            _ => Shown::Scene(sample_scene(&list[0], SceneKind::Live)),
        };
        let art = match shown.scene() {
            Some(s) => SceneArt {
                avatar: usable_picture(&s.provider, &s.avatar_url),
                category: Some(twitch_boxart_url("Just Chatting")),
            },
            None => SceneArt::default(),
        };
        (shown, art, unix_now() - SAMPLE_ELAPSED_SECS, 0)
    };
    let name = p.discord_user.as_ref().map(|u| u.name.clone());
    let ctx = BuildCtx {
        style,
        art: &art,
        start,
        seed,
        discord_name: name.as_deref(),
        assets: &p.assets,
    };
    let payload = build(&shown, &ctx);
    let connection = if !p.enabled {
        "off"
    } else if p.connected && matches!(p.delivered, Delivered::Sent(_)) {
        "connected"
    } else if p.connected {
        // Discord is open; the status is on its way (rate limit, a picture, a settle).
        "connecting"
    } else {
        "waiting"
    };
    let model = PreviewModel {
        activity: payload.as_ref().map(|pl| preview_activity(pl, &p.assets)),
        shown: shown_tab,
        live,
        connection,
        user: p.discord_user.clone(),
        tokens: tokens_for(&shown),
        presets: presets(),
    };
    drop(p);
    if first_look {
        // Pictures for the live scene start looking up now that the editor is open.
        wake();
    }
    model
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> DiscordPresenceSettings {
        DiscordPresenceSettings::default()
    }

    fn live(provider: &str, category: &str) -> Scene {
        Scene {
            kind: SceneKind::Live,
            provider: provider.into(),
            login: "caster".into(),
            user_id: "42".into(),
            name: "Caster".into(),
            title: "Big news today".into(),
            category: category.into(),
            category_art: String::new(),
            avatar_url: String::new(),
            started_at: Some(1_700_000_000),
            url: watch_url(provider, "caster"),
        }
    }

    fn ctx<'a>(
        style: &'a DiscordPresenceSettings,
        art: &'a SceneArt,
        assets: &'a HashMap<String, String>,
    ) -> BuildCtx<'a> {
        BuildCtx {
            style,
            art,
            start: 1_700_000_100,
            seed: 2,
            discord_name: None,
            assets,
        }
    }

    #[test]
    fn templates_fill_and_tidy() {
        let vars = Vars {
            streamer: "xQc".into(),
            title: "big - news".into(),
            category: String::new(),
            platform: "Twitch".into(),
            kind: "VOD".into(),
            count: 3,
            streamers: "a, b".into(),
        };
        assert_eq!(fill("Watching {streamer}", &vars), "Watching xQc");
        assert_eq!(fill("{category} · {title}", &vars), "big - news");
        assert_eq!(
            fill("{streamer} · {category} · {title}", &vars),
            "xQc · big - news"
        );
        assert_eq!(fill("{kind}: {category}", &vars), "VOD");
        assert_eq!(fill("{nope} {streamer}", &vars), "{nope} xQc");
        assert_eq!(fill("{{streamer}", &vars), "{xQc");
        assert_eq!(fill("{count} on {platform}", &vars), "3 on Twitch");
        assert_eq!(fill("open {", &vars), "open {");
    }

    #[test]
    fn text_is_cut_to_discords_limits() {
        assert_eq!(text_field("a"), None);
        assert_eq!(text_field("  \u{0007} ok\n\tthen "), Some("ok then".into()));
        let long = "x".repeat(200);
        let cut = text_field(&long).unwrap();
        assert_eq!(units(&cut), 128);
        assert!(cut.ends_with("..."));
        let cjk = "漢".repeat(200);
        assert_eq!(
            text_field(&cjk).unwrap(),
            format!("{}...", "漢".repeat(125))
        );
        let emoji = format!("{}😀bbb", "a".repeat(124));
        let cut = text_field(&emoji).unwrap();
        assert!(units(&cut) <= 128);
        assert!(
            cut.starts_with(&"a".repeat(124)) && !cut.contains('😀'),
            "the emoji never splits"
        );
        let family = format!(
            "{}👨\u{200d}👩\u{200d}👧tail-padding-padding",
            "b".repeat(120)
        );
        let cut = text_field(&family).unwrap();
        assert!(units(&cut) <= 128);
        assert!(
            !cut.contains('\u{200d}'),
            "a ZWJ family is kept whole or left out"
        );
    }

    #[test]
    fn urls_and_buttons_respect_limits() {
        assert_eq!(url_field("http://x.y/a.png", 256), None);
        assert_eq!(url_field("https://x.y/a b.png", 256), None);
        let long = format!("https://x.y/{}", "a".repeat(300));
        assert_eq!(url_field(&long, 256), None);
        assert!(url_field(&long, 512).is_some());
        assert_eq!(label_field("   "), None);
        let label = label_field(&"w".repeat(40)).unwrap();
        assert_eq!(units(&label), 32);
        assert!(label.ends_with('…'));
        // A picture too long for Discord falls back, and the activity still builds.
        let mut scene = live("twitch", "League of Legends");
        scene.avatar_url = format!("https://static-cdn.jtvnw.net/{}.png", "a".repeat(300));
        let s = DiscordPresenceSettings {
            large_image: "streamer".into(),
            ..style()
        };
        let art = SceneArt {
            avatar: usable_picture("twitch", &scene.avatar_url),
            category: Some("https://c/icon.png".into()),
        };
        let assets = HashMap::new();
        let p = build(&Shown::Scene(scene), &ctx(&s, &art, &assets)).unwrap();
        assert_eq!(p.large_image, "https://c/icon.png");
        // A cached picture over Discord's limit (a long CJK category's box art)
        // is never sent: the logo stands in.
        let long_cjk = twitch_boxart_url(&"漢".repeat(30));
        assert!(units(&long_cjk) > 256);
        let art = SceneArt {
            avatar: None,
            category: Some(long_cjk),
        };
        let p = build(
            &Shown::Scene(live("twitch", &"漢".repeat(30))),
            &ctx(&style(), &art, &assets),
        )
        .unwrap();
        assert_eq!(p.large_image, STREAMNOOK_LOGO_KEY);
    }

    #[test]
    fn default_style_matches_todays_presence() {
        let s = style();
        let assets = HashMap::new();
        let icon = "https://cdn.discordapp.com/app-icons/1/h.png?size=512".to_string();
        let art = SceneArt {
            avatar: None,
            category: Some(icon.clone()),
        };
        let p = build(
            &Shown::Scene(live("twitch", "League of Legends")),
            &ctx(&s, &art, &assets),
        )
        .unwrap();
        assert_eq!(p.details.as_deref(), Some("Watching Caster"));
        assert_eq!(p.state.as_deref(), Some("Big news today"));
        assert_eq!(p.large_image, icon);
        assert_eq!(p.large_text.as_deref(), Some("League of Legends"));
        assert_eq!(p.small_image.as_deref(), Some(TWITCH_BADGE_URL));
        assert_eq!(p.small_text.as_deref(), Some("Twitch"));
        assert_eq!(p.activity_type, Some(3));
        assert_eq!(p.start, Some(1_700_000_100));
        assert_eq!(p.status_display, None);
        assert_eq!(p.details_url, None);
        assert_eq!(p.large_url, None);
        assert_eq!(p.buttons.len(), 2);
        assert_eq!(
            p.buttons[0],
            PayloadButton {
                label: "Watch Stream".into(),
                url: "https://twitch.tv/caster".into()
            }
        );
        assert!(PROMO_LABELS.contains(&p.buttons[1].label.as_str()));
        assert_eq!(p.buttons[1].url, PROMO_URL);

        let boxart = twitch_boxart_url("Just Chatting");
        let art = SceneArt {
            avatar: None,
            category: Some(boxart.clone()),
        };
        let mut untitled = live("twitch", "Just Chatting");
        untitled.title = String::new();
        let p = build(&Shown::Scene(untitled), &ctx(&s, &art, &assets)).unwrap();
        assert_eq!(p.large_image, boxart);
        assert_eq!(p.state.as_deref(), Some("Live on Twitch"));

        let none = SceneArt::default();
        let p = build(&Shown::Scene(live("twitch", "")), &ctx(&s, &none, &assets)).unwrap();
        assert_eq!(p.large_image, STREAMNOOK_LOGO_KEY);
        assert_eq!(p.large_text.as_deref(), Some("StreamNook"));
    }

    #[test]
    fn each_platform_gets_its_own_badge() {
        let none = HashMap::new();
        assert_eq!(
            badge_image("twitch", &none).as_deref(),
            Some(TWITCH_BADGE_URL)
        );
        for p in ["kick", "youtube", "tiktok"] {
            assert_eq!(badge_image(p, &none), Some(format!("{BADGE_BASE}/{p}.png")));
        }
        let uploaded: HashMap<String, String> = [("kick_logo".to_string(), "123".to_string())]
            .into_iter()
            .collect();
        assert_eq!(badge_image("kick", &uploaded).as_deref(), Some("kick_logo"));
        assert_eq!(
            display_image("kick_logo", &uploaded),
            format!("https://cdn.discordapp.com/app-assets/{DISCORD_CLIENT_ID}/123.png")
        );
    }

    #[test]
    fn badge_files_exist() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("images");
        assert!(root.join("logo_1704751143960.JPG").is_file());
        for p in ["kick", "youtube", "tiktok"] {
            assert!(
                root.join("discord").join(format!("{p}.png")).is_file(),
                "{p} badge missing"
            );
        }
    }

    #[test]
    fn pictures_fall_back_in_order() {
        let assets = HashMap::new();
        let streamer_big = DiscordPresenceSettings {
            large_image: "streamer".into(),
            small_image: "streamer".into(),
            ..style()
        };
        let both = SceneArt {
            avatar: Some("https://a/face.png".into()),
            category: Some("https://c/art.png".into()),
        };
        let p = build(
            &Shown::Scene(live("twitch", "Art")),
            &ctx(&streamer_big, &both, &assets),
        )
        .unwrap();
        assert_eq!(p.large_image, "https://a/face.png");
        assert_eq!(p.large_text.as_deref(), Some("Caster"));
        assert_eq!(
            p.small_image, None,
            "a badge equal to the big picture is left out"
        );
        let no_face = SceneArt {
            avatar: None,
            ..both.clone()
        };
        let p = build(
            &Shown::Scene(live("twitch", "Art")),
            &ctx(&streamer_big, &no_face, &assets),
        )
        .unwrap();
        assert_eq!(p.large_image, "https://c/art.png");
        let nothing = SceneArt::default();
        let p = build(
            &Shown::Scene(live("twitch", "Art")),
            &ctx(&streamer_big, &nothing, &assets),
        )
        .unwrap();
        assert_eq!(p.large_image, STREAMNOOK_LOGO_KEY);
        // Kick's own category picture comes first.
        let mut kick = live("kick", "Slots & Casino");
        kick.category_art = "https://files.kick.com/cat.webp".into();
        let mut p = Presence::default();
        assert_eq!(
            p.scene_art(&kick).category.as_deref(),
            Some("https://files.kick.com/cat.webp")
        );
        kick.category_art = String::new();
        assert_eq!(p.scene_art(&kick).category, None);
        p.art.put(
            category_key(&kick),
            ArtEntry::Found("https://x/y.png".into()),
            &HashSet::new(),
        );
        assert_eq!(
            p.scene_art(&kick).category.as_deref(),
            Some("https://x/y.png")
        );
    }

    #[test]
    fn status_line_follows_the_choice() {
        let assets = HashMap::new();
        let art = SceneArt::default();
        let top = DiscordPresenceSettings {
            status_display: "top".into(),
            watching_top: "{streamer}".into(),
            ..style()
        };
        let p = build(&Shown::Scene(live("twitch", "")), &ctx(&top, &art, &assets)).unwrap();
        assert_eq!(p.status_display, Some(2));
        assert_eq!(preview_activity(&p, &assets).member_line, "Watching Caster");
        // Spotlight's own top line never doubles the verb.
        let spot = apply_look(&style(), &presets().remove(1).fields);
        let p = build(
            &Shown::Scene(live("twitch", "")),
            &ctx(&spot, &art, &assets),
        )
        .unwrap();
        assert_eq!(preview_activity(&p, &assets).member_line, "Watching Caster");
        let bottom = DiscordPresenceSettings {
            status_display: "bottom".into(),
            watching_bottom: "x".into(),
            ..style()
        };
        let p = build(
            &Shown::Scene(live("twitch", "")),
            &ctx(&bottom, &art, &assets),
        )
        .unwrap();
        assert_eq!(p.state, None);
        assert_eq!(p.status_display, None, "the chosen line was dropped");
        let listening = DiscordPresenceSettings {
            activity_type: "listening".into(),
            ..style()
        };
        let p = build(
            &Shown::Scene(live("twitch", "")),
            &ctx(&listening, &art, &assets),
        )
        .unwrap();
        assert_eq!(
            preview_activity(&p, &assets).member_line,
            "Listening to StreamNook"
        );
        let unknown = DiscordPresenceSettings {
            activity_type: "dancing".into(),
            ..style()
        };
        assert_eq!(
            build(
                &Shown::Scene(live("twitch", "")),
                &ctx(&unknown, &art, &assets)
            )
            .unwrap()
            .activity_type,
            Some(3)
        );
    }

    #[test]
    fn timer_keeps_counting_across_handoffs() {
        let t0 = Instant::now();
        let mut p = Presence::default();
        p.solo = Some(live("twitch", "Art"));
        p.touch_meta("solo:twitch:caster", t0);
        // A start no rebuild could produce, so "kept" means kept.
        p.meta.get_mut("solo:twitch:caster").unwrap().start = 1;
        let seed = p.meta["solo:twitch:caster"].seed;
        let kept = |p: &Presence| {
            p.meta["solo:twitch:caster"].start == 1 && p.meta["solo:twitch:caster"].seed == seed
        };
        // A style change is not a new scene.
        p.style.large_image = "streamer".into();
        p.touch_meta("solo:twitch:caster", t0 + Duration::from_millis(500));
        assert!(kept(&p));
        // The grid lands first, then the solo stop: same identity, same start.
        p.grid = Some(vec![SlotFacts {
            provider: "twitch".into(),
            login: "Caster".into(),
            ..SlotFacts::default()
        }]);
        p.touch_meta("solo:twitch:caster", t0 + Duration::from_secs(1));
        p.solo = None;
        p.touch_meta("solo:twitch:caster", t0 + Duration::from_secs(2));
        assert!(kept(&p));
        // The other order: the solo stop first leaves it unheld for a moment.
        p.grid = None;
        p.touch_meta("idle", t0 + Duration::from_secs(3));
        p.solo = Some(live("twitch", "Art"));
        p.touch_meta("solo:twitch:caster", t0 + Duration::from_secs(5));
        assert!(kept(&p), "kept within 10 s");
        // Another channel is a new scene with its own start.
        let mut other = live("twitch", "Art");
        other.login = "someone".into();
        p.solo = Some(other);
        p.touch_meta("solo:twitch:someone", t0 + Duration::from_secs(6));
        assert_ne!(p.meta["solo:twitch:someone"].start, 1);
        p.solo = Some(live("twitch", "Art"));
        p.touch_meta("solo:twitch:caster", t0 + Duration::from_secs(7));
        assert!(kept(&p), "back within 10 s");
        p.solo = None;
        p.touch_meta("idle", t0 + Duration::from_secs(8));
        p.touch_meta("idle", t0 + Duration::from_secs(19));
        assert!(
            !p.meta.contains_key("solo:twitch:caster"),
            "forgotten after 10 s unheld"
        );
        // Uptime uses the broadcast start; without one, the watching start.
        let assets = HashMap::new();
        let art = SceneArt::default();
        let uptime = DiscordPresenceSettings {
            timer: "uptime".into(),
            ..style()
        };
        let b = build(
            &Shown::Scene(live("twitch", "")),
            &ctx(&uptime, &art, &assets),
        )
        .unwrap();
        assert_eq!(b.start, Some(1_700_000_000));
        let mut no_start = live("twitch", "");
        no_start.started_at = None;
        assert_eq!(
            build(&Shown::Scene(no_start), &ctx(&uptime, &art, &assets))
                .unwrap()
                .start,
            Some(1_700_000_100)
        );
        let off = DiscordPresenceSettings {
            timer: "off".into(),
            ..style()
        };
        assert_eq!(
            build(&Shown::Scene(live("twitch", "")), &ctx(&off, &art, &assets))
                .unwrap()
                .start,
            None
        );
    }

    #[test]
    fn slots_follow_their_owners() {
        // Uses the shared state: one test drives it start to finish.
        page_ended();
        let gen = page();
        let media = MediaFacts {
            media_type: "video".into(),
            playing: true,
            url: "https://www.twitch.tv/videos/1".into(),
            login: "c".into(),
            ..MediaFacts::default()
        };
        assert_eq!(
            set_media("multichat-1", gen, 1, Some(media.clone())),
            WriteResult::Ignored
        );
        assert_eq!(
            set_media("main", gen + 7, 1, Some(media.clone())),
            WriteResult::Stale
        );
        assert_eq!(
            set_media("main", gen, 1, Some(media.clone())),
            WriteResult::Applied
        );
        assert_eq!(
            set_media("main", gen, 1, None),
            WriteResult::Ignored,
            "not newer"
        );
        set_solo(Some(live("twitch", "Art")));
        let (_, source, _) = lock().shown(Instant::now());
        assert_eq!(source, Source::Media, "a replay outranks the stream");
        let slots = vec![
            SlotFacts {
                provider: "twitch".into(),
                login: "a".into(),
                ..SlotFacts::default()
            },
            SlotFacts {
                provider: "kick".into(),
                login: "b".into(),
                ..SlotFacts::default()
            },
        ];
        assert_eq!(set_grid("main", gen, 1, Some(slots)), WriteResult::Applied);
        let (_, source, id) = lock().shown(Instant::now());
        assert_eq!((source, id.as_str()), (Source::Grid, "grid"));
        let rev = lock().rev;
        // Each lock in its own statement: a guard in a call's arguments lives
        // until the statement ends, and the call locks again.
        let same = lock().grid.clone().unwrap();
        assert_eq!(set_grid("main", gen, 2, Some(same)), WriteResult::Applied);
        assert_eq!(lock().rev, rev, "an equal write changes nothing");
        page_ended();
        let cleared = {
            let p = lock();
            p.grid.is_none() && p.media.is_none()
        };
        assert!(cleared);
        assert_eq!(
            set_grid("main", gen, 3, None),
            WriteResult::Stale,
            "the old page's straggler"
        );
        let (_, source, _) = lock().shown(Instant::now());
        assert_eq!(source, Source::Solo, "the stream shows again");
        set_solo(None);
    }

    fn view(payload: Option<Payload>) -> View {
        View {
            enabled: true,
            busy: false,
            connected: true,
            payload,
            ..View::default()
        }
    }

    fn payload(n: u32) -> Payload {
        Payload {
            details: Some(format!("edit {n}")),
            large_image: "k".into(),
            ..Payload::default()
        }
    }

    #[test]
    fn pump_lands_the_last_edit_within_the_limit() {
        let t0 = Instant::now();
        let mut st = PumpState::default();
        let mut sent = Vec::new();
        let mut now = t0;
        // Eight edits in the first second; the pump runs on every edit and
        // whenever its wait ends, always building the newest one.
        let edit_at = |i: u32| t0 + Duration::from_millis(125 * i as u64);
        let mut latest = 0;
        while now < t0 + Duration::from_secs(60) {
            while latest < 8 && edit_at(latest) <= now {
                latest += 1;
            }
            match next(&st, &view(Some(payload(latest))), now) {
                Step::Send => {
                    st.delivered = Delivered::Sent(Some(payload(latest)));
                    record(&mut st.gate, now);
                    sent.push((now, latest));
                }
                Step::Wait(until) => {
                    now = until.min(if latest < 8 { edit_at(latest) } else { until })
                }
                Step::Idle => {
                    now = if latest < 8 {
                        edit_at(latest)
                    } else {
                        t0 + Duration::from_secs(60)
                    }
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        let early = sent.iter().filter(|(t, _)| *t < t0 + GATE_WINDOW).count();
        assert!(
            early <= GATE_FRAMES,
            "{early} frames inside the first window"
        );
        assert_eq!(sent.last().map(|(_, n)| *n), Some(8), "the last edit lands");
    }

    #[test]
    fn pump_rules() {
        let t0 = Instant::now();
        let st = PumpState {
            delivered: Delivered::Sent(Some(payload(1))),
            ..PumpState::default()
        };
        let off = View {
            enabled: false,
            ..view(None)
        };
        assert_eq!(next(&st, &off, t0), Step::Shutdown);
        let full = PumpState {
            delivered: Delivered::Sent(Some(payload(1))),
            gate: (0..5).map(|i| t0 - Duration::from_secs(i)).collect(),
            ..PumpState::default()
        };
        assert_eq!(
            next(&full, &off, t0),
            Step::Shutdown,
            "off is never rate limited"
        );
        assert_eq!(next(&PumpState::default(), &off, t0), Step::Idle);
        let busy = View {
            busy: true,
            ..view(Some(payload(2)))
        };
        assert_eq!(next(&st, &busy, t0), Step::Idle);
        let down = View {
            connected: false,
            ..view(Some(payload(2)))
        };
        assert_eq!(next(&PumpState::default(), &down, t0), Step::Connect);
        let later = PumpState {
            retry_at: Some(t0 + NO_PIPE_RETRY),
            ..PumpState::default()
        };
        assert_eq!(next(&later, &down, t0), Step::Wait(t0 + NO_PIPE_RETRY));
        assert_eq!(
            retry_for(SendError::NoPipe, t0, None),
            Some(t0 + NO_PIPE_RETRY)
        );
        assert_eq!(
            retry_for(
                SendError::Unanswered,
                t0,
                Some(t0 + Duration::from_secs(42))
            ),
            Some(t0 + Duration::from_secs(42))
        );
        assert_eq!(retry_for(SendError::Busy, t0, None), None);
        let art = View {
            art_wait_until: Some(t0 + ART_WAIT),
            ..view(Some(payload(2)))
        };
        assert_eq!(next(&st, &art, t0), Step::Wait(t0 + ART_WAIT));
        assert_eq!(next(&st, &art, t0 + ART_WAIT), Step::Send);
        let settling = View {
            idle_since: Some(t0),
            ..view(Some(payload(2)))
        };
        assert_eq!(
            next(&st, &settling, t0 + Duration::from_secs(1)),
            Step::Wait(t0 + IDLE_SETTLE)
        );
        let same = View {
            later: Some(t0 + TILE_ERROR_GRACE),
            ..view(Some(payload(1)))
        };
        assert_eq!(next(&st, &same, t0), Step::Wait(t0 + TILE_ERROR_GRACE));
        assert_eq!(next(&st, &view(Some(payload(1))), t0), Step::Idle);
        // The first wake after start delivers browsing.
        assert_eq!(
            next(
                &PumpState {
                    delivered: Delivered::Unknown,
                    ..PumpState::default()
                },
                &view(Some(payload(0))),
                t0
            ),
            Step::Send
        );
        // Off with the editor open: a picture's retry still comes due; off and
        // closed: nothing at all.
        let off_editing = View {
            enabled: false,
            art_wanted: true,
            later: Some(t0 + ART_RETRY),
            ..view(None)
        };
        assert_eq!(
            next(&PumpState::default(), &off_editing, t0),
            Step::Wait(t0 + ART_RETRY)
        );
        let off_closed = View {
            art_wanted: false,
            ..off_editing
        };
        assert_eq!(next(&PumpState::default(), &off_closed, t0), Step::Idle);
    }

    #[test]
    fn pictures_are_looked_up_only_when_useful() {
        let t0 = Instant::now();
        assert!(!lookups_wanted(false, None, t0));
        assert!(lookups_wanted(
            false,
            Some(t0),
            t0 + Duration::from_secs(29)
        ));
        assert!(!lookups_wanted(false, Some(t0), t0 + PREVIEW_KEEPS_ART));
        assert!(lookups_wanted(true, None, t0));
        let mut cache = ArtCache::default();
        cache.put(
            "k".into(),
            ArtEntry::Provisional("https://x/box.jpg".into(), t0),
            &HashSet::new(),
        );
        assert_eq!(
            cache.found("k").as_deref(),
            Some("https://x/box.jpg"),
            "used now"
        );
        assert!(!cache.needs_lookup("k", t0 + Duration::from_secs(60)));
        assert!(
            cache.needs_lookup("k", t0 + PROVISIONAL_RETRY),
            "asked again later"
        );
    }

    #[test]
    fn grid_presence_follows_the_featured_slot() {
        let t0 = Instant::now();
        let slot = |p: &str, l: &str, c: &str| SlotFacts {
            provider: p.into(),
            login: l.into(),
            name: l.to_uppercase(),
            category: c.into(),
            ..SlotFacts::default()
        };
        let mut slots = vec![
            slot("twitch", "a", "Art"),
            slot("kick", "b", "Slots"),
            slot("twitch", "c", "Slots"),
            slot("twitch", "d", "Art"),
        ];
        slots[1].chat_focused = true;
        let errors = HashMap::new();
        let Some(Shown::Grid(g)) = decide_grid(&slots, &errors, t0) else {
            panic!("a grid")
        };
        assert_eq!(g.featured.login, "b", "the chat-focused tile leads");
        assert_eq!(g.featured.category, "Art", "ties go to the first in order");
        assert_eq!(g.names, vec!["A", "B", "C", "D"]);
        slots[3].maximized = true;
        let Some(Shown::Grid(g)) = decide_grid(&slots, &errors, t0) else {
            panic!()
        };
        assert_eq!(g.featured.login, "d", "a maximized tile wins");
        slots[2].offline = true;
        slots[0].load_error = true;
        let young: HashMap<String, Instant> = [("twitch:a".to_string(), t0)].into_iter().collect();
        let Some(Shown::Grid(g)) = decide_grid(&slots, &young, t0 + Duration::from_secs(5)) else {
            panic!()
        };
        assert_eq!(
            g.names,
            vec!["A", "B", "D"],
            "a new error still counts; offline never does"
        );
        let Some(shown) = decide_grid(&slots[..2], &young, t0 + TILE_ERROR_GRACE) else {
            panic!()
        };
        assert!(
            matches!(shown, Shown::Scene(ref s) if s.login == "b"),
            "one counted tile reads as a stream"
        );
        let names: Vec<String> = (0..40).map(|i| format!("streamer{i:02}")).collect();
        let joined = join_names(&names);
        assert!(units(&joined) <= 110 && joined.ends_with("more"));
        let assets = HashMap::new();
        let art = SceneArt::default();
        let empty = vec![slot("twitch", "a", ""), slot("kick", "b", "")];
        let shown = decide_grid(&empty, &errors, t0).unwrap();
        let p = build(&shown, &ctx(&style(), &art, &assets)).unwrap();
        assert_eq!(p.large_image, STREAMNOOK_LOGO_KEY);
        assert_eq!(p.details.as_deref(), Some("Watching 2 Streams"));
        assert_eq!(
            p.small_image.as_deref(),
            Some(TWITCH_BADGE_URL),
            "the leading tile's platform"
        );
        assert_eq!(p.buttons[0].url, "https://twitch.tv/a");
    }

    #[test]
    fn idle_modes() {
        let assets = HashMap::new();
        let art = SceneArt::default();
        let s = style();
        let a = build(&Shown::Idle, &ctx(&s, &art, &assets)).unwrap();
        let b = build(&Shown::Idle, &ctx(&s, &art, &assets)).unwrap();
        assert_eq!(a, b, "the same seed, the same phrase");
        assert_eq!(a.activity_type, None);
        assert_eq!(a.large_image, STREAMNOOK_LOGO_KEY);
        assert_eq!(a.buttons.len(), 1);
        let named = BuildCtx {
            discord_name: Some("Nelly"),
            ..ctx(&s, &art, &assets)
        };
        assert!(
            build(&Shown::Idle, &named)
                .unwrap()
                .details
                .unwrap()
                .contains("Nelly"),
            "seed 2 is even: personalized"
        );
        let custom = DiscordPresenceSettings {
            idle_mode: "custom".into(),
            idle_text: "Picking a stream".into(),
            ..style()
        };
        assert_eq!(
            build(&Shown::Idle, &ctx(&custom, &art, &assets))
                .unwrap()
                .details
                .as_deref(),
            Some("Picking a stream")
        );
        let hidden = DiscordPresenceSettings {
            idle_mode: "hidden".into(),
            ..style()
        };
        assert_eq!(build(&Shown::Idle, &ctx(&hidden, &art, &assets)), None);
    }

    #[test]
    fn replays_say_what_they_are() {
        let clip = MediaFacts {
            media_type: "clip".into(),
            playing: true,
            url: "https://clips.twitch.tv/x".into(),
            login: String::new(),
            name: "Caster".into(),
            title: "the play".into(),
            ..MediaFacts::default()
        };
        let scene = media_scene(&clip).unwrap();
        assert_eq!(scene.kind, SceneKind::Clip);
        let assets = HashMap::new();
        let art = SceneArt::default();
        let p = build(&Shown::Scene(scene), &ctx(&style(), &art, &assets)).unwrap();
        assert_eq!(p.details.as_deref(), Some("Watching Caster"));
        assert_eq!(p.state.as_deref(), Some("Clip: the play"));
        assert_eq!(p.buttons[0].label, "Watch Clip");
        let placeholder = MediaFacts {
            name: MEDIA_PLACEHOLDER_NAME.into(),
            login: "who".into(),
            ..clip.clone()
        };
        assert_eq!(media_scene(&placeholder).unwrap().name, "");
        assert!(media_scene(&MediaFacts {
            playing: false,
            ..clip.clone()
        })
        .is_none());
        let room_vod = MediaFacts {
            media_type: "offline_chat".into(),
            vod: true,
            ..clip.clone()
        };
        assert_eq!(media_scene(&room_vod).unwrap().kind, SceneKind::Vod);
        assert!(media_scene(&MediaFacts {
            vod: false,
            ..room_vod
        })
        .is_none());
        assert!(media_scene(&MediaFacts {
            media_type: "live".into(),
            ..clip
        })
        .is_none());
        let mut offline = live("twitch", "");
        offline.kind = SceneKind::Offline;
        let p = build(&Shown::Scene(offline), &ctx(&style(), &art, &assets)).unwrap();
        assert_eq!(p.details.as_deref(), Some("In Caster's chat"));
        assert_eq!(p.buttons[0].label, "Watch Stream");
    }

    #[test]
    fn avatars_come_in_large() {
        assert_eq!(
            larger_avatar(
                "twitch",
                "https://static-cdn.jtvnw.net/jtv_user_pictures/x-profile_image-70x70.png"
            ),
            "https://static-cdn.jtvnw.net/jtv_user_pictures/x-profile_image-300x300.png"
        );
        assert_eq!(
            larger_avatar(
                "youtube",
                "https://yt3.ggpht.com/abc=s88-c-k-c0x00ffffff-no-rj"
            ),
            "https://yt3.ggpht.com/abc=s300-c-k-c0x00ffffff-no-rj"
        );
        assert_eq!(
            larger_avatar("kick", "https://files.kick.com/a-70x70.webp"),
            "https://files.kick.com/a-70x70.webp"
        );
    }

    #[test]
    fn presets_change_only_the_look() {
        let mine = DiscordPresenceSettings {
            idle_mode: "custom".into(),
            idle_text: "hi".into(),
            promo_button: false,
            ..style()
        };
        let list = presets();
        assert_eq!(
            list.iter().map(|p| p.id).collect::<Vec<_>>(),
            vec!["classic", "spotlight", "lowkey"]
        );
        assert!(
            list.iter().all(|p| p.fields.len() == LOOK_FIELDS.len()),
            "every preset sets the whole look"
        );
        assert!(
            list.iter()
                .all(|p| !p.fields.contains_key("idle_mode")
                    && !p.fields.contains_key("promo_button"))
        );
        assert!(
            preset_active(&mine, &list[0]),
            "the defaults look like Classic"
        );
        let spot = apply_look(&mine, &list[1].fields);
        assert_eq!(spot.large_image, "streamer");
        assert_eq!(
            (
                spot.idle_mode.as_str(),
                spot.idle_text.as_str(),
                spot.promo_button
            ),
            ("custom", "hi", false)
        );
        assert!(preset_active(&spot, &list[1]) && !preset_active(&spot, &list[0]));
    }

    #[test]
    fn presence_settings_load_from_old_files() {
        let empty: DiscordPresenceSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, DiscordPresenceSettings::default());
        let partial: DiscordPresenceSettings =
            serde_json::from_str(r#"{"large_image":"streamer"}"#).unwrap();
        assert_eq!(partial.large_image, "streamer");
        assert_eq!(partial.watching_top, "Watching {streamer}");
        let newer: DiscordPresenceSettings =
            serde_json::from_str(r#"{"small_image":"hologram","glow":true}"#).unwrap();
        assert_eq!(newer.extra.get("glow"), Some(&serde_json::json!(true)));
        assert_eq!(
            serde_json::to_value(&newer).unwrap()["glow"],
            serde_json::json!(true),
            "kept on save"
        );
        let assets = HashMap::new();
        let art = SceneArt::default();
        let p = build(
            &Shown::Scene(live("twitch", "")),
            &ctx(&newer, &art, &assets),
        )
        .unwrap();
        assert_eq!(
            p.small_image.as_deref(),
            Some(TWITCH_BADGE_URL),
            "an unknown choice acts as the default"
        );
    }
}
