//! The stream manager: running a Twitch channel from the app.
//!
//! One view that desktop, Android and iPhone all draw, and one door for every
//! action. The broadcaster's own channel goes through Helix (the public API,
//! which only accepts the broadcaster's token for title, polls, predictions,
//! raids and ads). Editors and moderators get what Twitch's own site lets them
//! do, through Twitch's private GQL on the in-app twitch.tv session: editors
//! change stream info, moderators and editors run polls and predictions, and
//! everyone with a role here works the channel points queue. The queue always
//! goes through GQL, because Helix only sees rewards StreamNook itself made.
//!
//! The pure parts (who may do what, the stream info rules, the preview tile,
//! the queue and ad schedule in words) are plain functions with tests; the
//! network parts read and write Twitch and hand their answers to them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::services::twitch_auth_service::TwitchAuthService;
use crate::services::twitch_service::{helix_client_id, helix_refusal_text, lacks_scope, TwitchService};
use crate::services::twitch_web_gql;

const LOG_TAG: &str = "StreamManager";

// ─────────────────────────────── revisions ───────────────────────────────────

/// One counter for every read and answer the manager sends, so a window can
/// drop an answer from a read that started before one it already shows.
static REV: AtomicU64 = AtomicU64::new(1);

/// The next revision. A read takes it before it asks Twitch anything.
pub fn next_rev() -> u64 {
    REV.fetch_add(1, Ordering::SeqCst)
}

/// Locks a map, carrying on past a panic in another holder.
fn locked<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Twitch's limits for the stream info form.
pub const TITLE_MAX: usize = 140;
pub const TAGS_MAX: usize = 10;
pub const TAG_MAX: usize = 25;

/// How long a raid counts down before it goes by itself.
const RAID_COUNTDOWN_MS: i64 = 90_000;

/// The lengths Twitch accepts for an ad break, in seconds.
const AD_LENGTHS: [u32; 6] = [30, 60, 90, 120, 150, 180];

/// The languages Twitch offers a broadcaster, in the order its dashboard lists
/// them, as (Helix code, name).
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("ar", "Arabic"),
    ("bg", "Bulgarian"),
    ("ca", "Catalan"),
    ("zh", "Chinese"),
    ("zh-hk", "Cantonese"),
    ("cs", "Czech"),
    ("da", "Danish"),
    ("nl", "Dutch"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("de", "German"),
    ("el", "Greek"),
    ("hi", "Hindi"),
    ("hu", "Hungarian"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("ko", "Korean"),
    ("ms", "Malay"),
    ("no", "Norwegian"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sk", "Slovak"),
    ("es", "Spanish"),
    ("sv", "Swedish"),
    ("tl", "Tagalog"),
    ("th", "Thai"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("vi", "Vietnamese"),
    ("asl", "American Sign Language"),
    ("other", "Other"),
];

pub fn language_name(code: &str) -> Option<&'static str> {
    LANGUAGES
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(code))
        .map(|(_, n)| *n)
}

// ─────────────────────────────── roles ───────────────────────────────────────

/// What the signed-in account is in this channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Broadcaster,
    Editor,
    Moderator,
    /// A moderator who is also one of the channel's editors.
    ModeratorEditor,
    Viewer,
}

impl Role {
    pub fn from_flags(own_channel: bool, editor: bool, moderator: bool) -> Role {
        match (own_channel, editor, moderator) {
            (true, _, _) => Role::Broadcaster,
            (false, true, true) => Role::ModeratorEditor,
            (false, true, false) => Role::Editor,
            (false, false, true) => Role::Moderator,
            _ => Role::Viewer,
        }
    }
    fn broadcaster(self) -> bool {
        self == Role::Broadcaster
    }
    fn editor(self) -> bool {
        matches!(self, Role::Broadcaster | Role::Editor | Role::ModeratorEditor)
    }
    fn moderator(self) -> bool {
        matches!(self, Role::Broadcaster | Role::Moderator | Role::ModeratorEditor)
    }
    /// Anything to manage here at all.
    pub fn manages(self) -> bool {
        self != Role::Viewer
    }
}

/// One thing the manager can do, as a row the shells draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Info,
    Poll,
    Prediction,
    Raid,
    Ads,
    Rewards,
    Chat,
    Announce,
}

/// Whether `role` may use `section`, or the one line that says why not.
pub fn can_use(role: Role, section: Section) -> Result<(), &'static str> {
    let ok = match section {
        Section::Info => role.editor(),
        Section::Poll | Section::Prediction | Section::Rewards => role.editor() || role.moderator(),
        Section::Raid | Section::Ads => role.broadcaster(),
        Section::Chat | Section::Announce => role.moderator(),
    };
    if ok {
        return Ok(());
    }
    Err(match section {
        Section::Info => "Only the broadcaster and editors can change stream info",
        Section::Raid => "Only the broadcaster can raid from this channel",
        Section::Ads => "Only the broadcaster can run ads",
        Section::Chat | Section::Announce => "This needs a moderator",
        _ => "This needs a moderator or editor",
    })
}

/// Who answers for a section: the broadcaster's own Helix, or Twitch's GQL on
/// the in-app session for everyone else.
fn helix_route(role: Role) -> bool {
    role.broadcaster()
}

// ─────────────────────────────── the view ────────────────────────────────────

#[derive(Clone, Debug, Serialize)]
pub struct StreamManagerView {
    /// Taken when this read started (`next_rev`).
    pub rev: u64,
    /// The Twitch account the view was read as.
    pub user_id: String,
    pub channel_id: String,
    pub channel_login: String,
    pub role: Role,
    /// "You are a moderator here" and the like, for the sheet's header.
    pub role_line: String,
    /// The rows of the Stream group, in order. Rows the role cannot use are
    /// left out, so no row ever opens onto a refusal.
    pub sections: Vec<SectionRow>,
    /// The Manage channel window's side list, in order.
    pub pages: Vec<PageRow>,
    pub info: Option<StreamInfoView>,
    pub poll: Option<RunningPoll>,
    pub prediction: Option<RunningPrediction>,
    /// False when the running poll could not be read this time: `poll` is the
    /// last one known, kept as it was.
    pub poll_known: bool,
    pub prediction_known: bool,
    pub raid: Option<RaidView>,
    pub ads: Option<AdsView>,
    pub chat: Option<ChatView>,
    /// Redemptions waiting, when the queue was read.
    pub queue_count: Option<usize>,
    pub limits: FormLimits,
    /// Polls and predictions started here before, newest first, for the
    /// builder's "Use a recent one".
    pub recent_polls: Vec<RecentPoll>,
    pub recent_predictions: Vec<RecentPoll>,
    /// Parts that could not be read, in words. The rest of the view still works.
    pub problems: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SectionRow {
    pub id: Section,
    pub label: String,
    /// A second line: the current title and category, the next ad.
    pub detail: Option<String>,
    /// A short trailing count ("3 waiting").
    pub badge: Option<String>,
}

#[derive(Clone, Debug, Serialize, Default)]
pub struct FormLimits {
    pub title: usize,
    pub tags: usize,
    pub tag: usize,
    pub poll_title: usize,
    pub poll_choice: usize,
    pub poll_choices_min: usize,
    pub poll_choices_max: usize,
    pub poll_seconds_min: u32,
    pub poll_seconds_max: u32,
    pub prediction_title: usize,
    pub prediction_outcome: usize,
    pub prediction_outcomes_min: usize,
    pub prediction_outcomes_max: usize,
    pub prediction_seconds_min: u32,
    pub prediction_seconds_max: u32,
    pub durations: Vec<Choice>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Choice {
    pub label: String,
    pub value: u32,
}

fn choice(label: &str, value: u32) -> Choice {
    Choice { label: label.to_string(), value }
}

pub fn form_limits() -> FormLimits {
    FormLimits {
        title: TITLE_MAX,
        tags: TAGS_MAX,
        tag: TAG_MAX,
        poll_title: 60,
        poll_choice: 25,
        poll_choices_min: 2,
        poll_choices_max: 5,
        poll_seconds_min: 15,
        poll_seconds_max: 1800,
        prediction_title: 45,
        prediction_outcome: 25,
        prediction_outcomes_min: 2,
        prediction_outcomes_max: 10,
        prediction_seconds_min: 30,
        prediction_seconds_max: 1800,
        durations: vec![
            choice("30s", 30),
            choice("1m", 60),
            choice("2m", 120),
            choice("5m", 300),
            choice("10m", 600),
            choice("30m", 1800),
        ],
    }
}

fn role_line(role: Role) -> String {
    match role {
        Role::Broadcaster => "Your channel".into(),
        Role::Editor => "You are an editor here".into(),
        Role::Moderator => "You are a moderator here".into(),
        Role::ModeratorEditor => "You are a moderator and editor here".into(),
        Role::Viewer => "You do not manage this channel".into(),
    }
}

/// The Stream group's rows for `role`, with their second lines filled from
/// what was read.
pub fn section_rows(
    role: Role,
    info: Option<&StreamInfoView>,
    poll: Option<&RunningPoll>,
    prediction: Option<&RunningPrediction>,
    raid: Option<&RaidView>,
    ads: Option<&AdsView>,
    queue_count: Option<usize>,
    now_ms: i64,
) -> Vec<SectionRow> {
    let mut rows = Vec::new();
    let mut push = |id: Section, label: &str, detail: Option<String>, badge: Option<String>| {
        if can_use(role, id).is_ok() {
            rows.push(SectionRow { id, label: label.to_string(), detail, badge });
        }
    };
    push(Section::Info, "Stream info", info.map(|i| i.summary.clone()).filter(|s| !s.is_empty()), None);
    push(
        Section::Poll,
        if poll.is_some() { "Poll running" } else { "Start a poll" },
        poll.map(|p| p.title.clone()),
        None,
    );
    push(
        Section::Prediction,
        if prediction.is_some() { "Prediction running" } else { "Start a prediction" },
        prediction.map(|p| p.title.clone()),
        None,
    );
    push(
        Section::Raid,
        if raid.is_some() { "Raid counting down" } else { "Raid a channel" },
        raid.map(|r| r.target_name.clone()),
        None,
    );
    push(Section::Ads, "Run an ad break", ads.and_then(|a| ads_detail(a, now_ms)), None);
    push(
        Section::Rewards,
        "Rewards queue",
        None,
        queue_count.filter(|n| *n > 0).map(|n| format!("{n} waiting")),
    );
    push(Section::Announce, "Announce", None, None);
    rows
}

/// One row of the Manage channel window's side list.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PageRow {
    /// "info", "poll", "prediction", "raid", "ads", "rewards", "announce",
    /// "chat_modes", "moderation".
    pub id: &'static str,
    /// The heading it sits under: "Your channel", "Channel" or "Chat".
    pub group: &'static str,
    pub label: &'static str,
    /// A live mark in words: "running", "locked", "counting down".
    pub live: Option<&'static str>,
    /// A short count ("3").
    pub badge: Option<String>,
}

/// The window's pages for `role`, in order: the channel's tools, then chat's.
/// Like the menu's rows, only what the role may use here.
pub fn page_rows(
    role: Role,
    poll: Option<&RunningPoll>,
    prediction: Option<&RunningPrediction>,
    raid: Option<&RaidView>,
    queue_count: Option<usize>,
) -> Vec<PageRow> {
    let channel = if role.broadcaster() { "Your channel" } else { "Channel" };
    let mut rows = Vec::new();
    let mut push = |section: Section, id: &'static str, group: &'static str, label: &'static str, live: Option<&'static str>, badge: Option<String>| {
        if can_use(role, section).is_ok() {
            rows.push(PageRow { id, group, label, live, badge });
        }
    };
    push(Section::Info, "info", channel, "Stream info", None, None);
    push(Section::Poll, "poll", channel, "Poll", poll.map(|_| "running"), None);
    push(
        Section::Prediction,
        "prediction",
        channel,
        "Prediction",
        prediction.map(|p| if p.status == "locked" { "locked" } else { "running" }),
        None,
    );
    push(Section::Raid, "raid", channel, "Raid", raid.map(|_| "counting down"), None);
    push(Section::Ads, "ads", channel, "Ad break", None, None);
    push(Section::Rewards, "rewards", channel, "Rewards queue", None, queue_count.filter(|n| *n > 0).map(|n| n.to_string()));
    push(Section::Announce, "announce", "Chat", "Announce", None, None);
    push(Section::Chat, "chat_modes", "Chat", "Chat modes", None, None);
    push(Section::Chat, "moderation", "Chat", "Moderation", None, None);
    rows
}

// ─────────────────────────────── stream info ─────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Category {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub box_art_url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct StreamInfoView {
    pub title: String,
    pub category: Option<Category>,
    pub tags: Vec<String>,
    pub language: String,
    pub language_name: String,
    pub languages: Vec<LanguageRow>,
    /// "Ranked grind to Diamond · Valorant", for the row's second line.
    pub summary: String,
    /// The category in use first, then the ones you saved here before,
    /// newest first: the category picker shows them before you type.
    pub recent_categories: Vec<Category>,
    /// The emoji row the title's emoji menu opens on: the emoji in the
    /// title now, then the ones you put in titles before, or a starter set
    /// while there are none.
    pub title_emoji: Vec<String>,
    /// That row's heading ("Your title emoji", or "Popular in titles" for the
    /// starter set).
    pub title_emoji_heading: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct LanguageRow {
    pub code: String,
    pub name: String,
}

fn language_rows() -> Vec<LanguageRow> {
    LANGUAGES
        .iter()
        .map(|(c, n)| LanguageRow { code: c.to_string(), name: n.to_string() })
        .collect()
}

/// The form as typed, sent to `check` on every change and to `save`.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct StreamInfoDraft {
    pub title: String,
    #[serde(default)]
    pub category: Option<Category>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub language: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct FieldProblem {
    /// "title", "tags", "tag:2" (the third tag), "language".
    pub field: String,
    pub line: String,
}

/// What the form shows under the fields, and the tile viewers will see.
#[derive(Clone, Debug, Serialize)]
pub struct InfoCheck {
    pub ok: bool,
    pub problems: Vec<FieldProblem>,
    /// "54 / 140", or "146 / 140 · 6 over" past the limit.
    pub title_count: String,
    /// How the count is drawn: "quiet", "near" (the last 15) or "over".
    pub title_tone: &'static str,
    pub tags_count: String,
    pub preview: PreviewTile,
    /// Changes against what Twitch has now; Save is pointless without one.
    pub changed: bool,
}

/// The Following tile, as viewers will see it with the draft applied.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PreviewTile {
    pub title: String,
    pub channel_name: String,
    pub avatar_url: Option<String>,
    pub category_name: String,
    pub box_art_url: Option<String>,
    pub tags: Vec<String>,
    /// The channel's live thumbnail when it is live, else none (the shells
    /// draw their own empty frame).
    pub thumbnail_url: Option<String>,
    /// While live: how many watch and when it started (RFC 3339), so the
    /// preview is the app's own live card with its chips.
    pub viewer_count: Option<u64>,
    pub started_at: Option<String>,
    /// A Twitch partner: the cards draw the verified mark after the name.
    pub partner: bool,
}

/// A tag is letters and digits only (any script), at most 25 of them.
fn tag_problem(tag: &str) -> Option<&'static str> {
    if tag.is_empty() {
        return Some("A tag can't be empty");
    }
    if tag.chars().count() > TAG_MAX {
        return Some("A tag can be at most 25 characters");
    }
    if !tag.chars().all(char::is_alphanumeric) {
        return Some("Tags use letters and numbers only, no spaces or symbols");
    }
    None
}

/// Typed tag text as Twitch will store it: trimmed, a leading # dropped.
pub fn clean_tag(raw: &str) -> String {
    raw.trim().trim_start_matches('#').to_string()
}

/// The count under the title and how loudly it reads. The length is in
/// Unicode scalar values (`chars`), what Twitch's limit is measured in as far
/// as we know: 🔴 is one, a family emoji joined from five is five.
fn title_count(len: usize) -> (String, &'static str) {
    if len > TITLE_MAX {
        (format!("{len} / {TITLE_MAX} · {} over", len - TITLE_MAX), "over")
    } else if len + TITLE_NEAR > TITLE_MAX {
        (format!("{len} / {TITLE_MAX}"), "near")
    } else {
        (format!("{len} / {TITLE_MAX}"), "quiet")
    }
}

/// The count turns amber this close to the limit.
const TITLE_NEAR: usize = 15;

pub fn check_stream_info(
    draft: &StreamInfoDraft,
    current: Option<&StreamInfoView>,
    channel_name: &str,
    avatar_url: Option<String>,
    thumbnail_url: Option<String>,
) -> InfoCheck {
    let mut problems = Vec::new();
    let title = draft.title.trim();
    let title_len = title.chars().count();
    if title.is_empty() {
        problems.push(FieldProblem { field: "title".into(), line: "Add a title".into() });
    } else if title_len > TITLE_MAX {
        problems.push(FieldProblem {
            field: "title".into(),
            line: format!("Titles can be at most {TITLE_MAX} characters"),
        });
    }
    let tags: Vec<String> = draft.tags.iter().map(|t| clean_tag(t)).collect();
    if tags.len() > TAGS_MAX {
        problems.push(FieldProblem { field: "tags".into(), line: format!("At most {TAGS_MAX} tags") });
    }
    let mut seen: Vec<String> = Vec::new();
    for (i, tag) in tags.iter().enumerate() {
        if let Some(line) = tag_problem(tag) {
            problems.push(FieldProblem { field: format!("tag:{i}"), line: line.into() });
            continue;
        }
        let folded = tag.to_lowercase();
        if seen.contains(&folded) {
            problems.push(FieldProblem { field: format!("tag:{i}"), line: "That tag is already on".into() });
        }
        seen.push(folded);
    }
    if !draft.language.is_empty() && language_name(&draft.language).is_none() {
        problems.push(FieldProblem { field: "language".into(), line: "Pick a language from the list".into() });
    }
    let changed = match current {
        None => true,
        Some(c) => {
            c.title != title
                || c.category.as_ref().map(|x| &x.id) != draft.category.as_ref().map(|x| &x.id)
                || c.tags != tags
                || (!draft.language.is_empty() && !c.language.eq_ignore_ascii_case(&draft.language))
        }
    };
    let category = draft.category.as_ref();
    let (title_count, title_tone) = title_count(title_len);
    InfoCheck {
        ok: problems.is_empty(),
        problems,
        title_count,
        title_tone,
        tags_count: format!("{} / {TAGS_MAX}", tags.len()),
        preview: PreviewTile {
            title: title.to_string(),
            channel_name: channel_name.to_string(),
            avatar_url,
            category_name: category.map(|c| c.name.clone()).unwrap_or_default(),
            box_art_url: category
                .map(|c| box_art(&c.box_art_url, 52, 72))
                .filter(|u| !u.is_empty()),
            tags,
            thumbnail_url,
            viewer_count: None,
            started_at: None,
            partner: false,
        },
        changed,
    }
}

/// Twitch's `{width}x{height}` templates sized.
fn box_art(template: &str, w: u32, h: u32) -> String {
    template
        .replace("{width}", &w.to_string())
        .replace("{height}", &h.to_string())
        .replace("-52x72", &format!("-{w}x{h}"))
}

fn summary(title: &str, category: Option<&Category>) -> String {
    let title = title.trim();
    match category.map(|c| c.name.as_str()).filter(|n| !n.is_empty()) {
        Some(name) if !title.is_empty() => format!("{title} · {name}"),
        Some(name) => name.to_string(),
        None => title.to_string(),
    }
}

/// Helix `GET channels` row to the view.
pub fn info_from_helix(row: &Value, box_art_url: Option<String>) -> StreamInfoView {
    let title = row["title"].as_str().unwrap_or_default().to_string();
    let category = row["game_id"].as_str().filter(|id| !id.is_empty()).map(|id| Category {
        id: id.to_string(),
        name: row["game_name"].as_str().unwrap_or_default().to_string(),
        box_art_url: box_art_url.unwrap_or_default(),
    });
    let tags: Vec<String> = row["tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let language = row["broadcaster_language"].as_str().unwrap_or_default().to_string();
    StreamInfoView {
        summary: summary(&title, category.as_ref()),
        language_name: language_name(&language).unwrap_or_default().to_string(),
        title,
        category,
        tags,
        language,
        languages: language_rows(),
        recent_categories: Vec::new(),
        title_emoji: Vec::new(),
        title_emoji_heading: String::new(),
    }
}

// ─────────────────────────────── recents ─────────────────────────────────────

/// What a channel's past saves leave behind for the next edit, kept on this
/// device only (`stream_manager_recents.json` in the app's data folder).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct InfoRecents {
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub emoji: Vec<String>,
}

const RECENT_CATEGORIES_MAX: usize = 10;
const RECENT_EMOJI_MAX: usize = 16;
/// The emoji row before you have put any emoji in a title.
const STARTER_EMOJI: [&str; 8] = ["🔴", "🎮", "🔥", "💜", "✨", "😂", "🏆", "🎉"];

/// True for a grapheme drawn as an emoji: a pictograph, a flag, a keycap, or
/// anything asking for emoji presentation.
fn is_emoji(g: &str) -> bool {
    let Some(first) = g.chars().next() else { return false };
    let cp = first as u32;
    matches!(cp, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0x2300..=0x23FF | 0x2B00..=0x2BFF | 0x3030 | 0x303D | 0x3297 | 0x3299)
        || g.contains('\u{20E3}')
        || (g.contains('\u{FE0F}') && !first.is_alphanumeric())
}

/// The emoji in a title, first use first, each once.
pub fn emoji_in(title: &str) -> Vec<String> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut out: Vec<String> = Vec::new();
    for g in title.graphemes(true) {
        if is_emoji(g) && !out.iter().any(|e| e == g) {
            out.push(g.to_string());
        }
    }
    out
}

/// `front` then `rest`, each item once (by `same`), at most `cap`.
fn merged<T: Clone>(front: &[T], rest: &[T], cap: usize, same: impl Fn(&T, &T) -> bool) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for item in front.iter().chain(rest) {
        if out.len() == cap {
            break;
        }
        if !out.iter().any(|o| same(o, item)) {
            out.push(item.clone());
        }
    }
    out
}

/// Fills the view's recent categories and emoji row from what is on the
/// channel now and what earlier saves left.
pub fn with_recents(info: &mut StreamInfoView, recents: Option<&InfoRecents>) {
    let empty = InfoRecents::default();
    let recents = recents.unwrap_or(&empty);
    let current: Vec<Category> = info.category.iter().cloned().collect();
    info.recent_categories = merged(&current, &recents.categories, RECENT_CATEGORIES_MAX, |a, b| a.id == b.id);
    let in_title = emoji_in(&info.title);
    info.title_emoji = merged(&in_title, &recents.emoji, RECENT_EMOJI_MAX, |a, b| a == b);
    if info.title_emoji.is_empty() {
        info.title_emoji = STARTER_EMOJI.iter().map(|e| e.to_string()).collect();
        info.title_emoji_heading = "Popular in titles".into();
    } else {
        info.title_emoji_heading = "Your title emoji".into();
    }
}

/// What a save adds to the channel's recents: its category and its emoji go
/// to the front.
pub fn after_save(recents: &InfoRecents, draft: &StreamInfoDraft) -> InfoRecents {
    let category: Vec<Category> = draft.category.iter().cloned().collect();
    InfoRecents {
        categories: merged(&category, &recents.categories, RECENT_CATEGORIES_MAX, |a, b| a.id == b.id),
        emoji: merged(&emoji_in(&draft.title), &recents.emoji, RECENT_EMOJI_MAX, |a, b| a == b),
    }
}

/// Held around every read-modify-write of both recents files, so two saves
/// never write over each other.
static RECENTS_LOCK: Mutex<()> = Mutex::new(());

fn data_file(name: &str) -> Option<std::path::PathBuf> {
    crate::services::cache_service::get_app_data_dir().ok().map(|d| d.join(name))
}

const INFO_RECENTS_FILE: &str = "stream_manager_recents.json";
const POLL_RECENTS_FILE: &str = "stream_manager_polls.json";

/// What one read of a recents file allows.
#[derive(Debug, PartialEq)]
pub enum ReadPlan<T> {
    /// No file yet: start empty, a write may create it.
    Missing,
    Parsed(T),
    /// The file is there but is not ours to read: start empty, a write
    /// replaces it.
    Unparseable,
    /// The disk would not say: never write over what may still be there.
    Error(String),
}

pub fn recents_read_plan<T: serde::de::DeserializeOwned>(read: std::io::Result<Vec<u8>>) -> ReadPlan<T> {
    match read {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(v) => ReadPlan::Parsed(v),
            Err(_) => ReadPlan::Unparseable,
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ReadPlan::Missing,
        Err(e) => ReadPlan::Error(e.to_string()),
    }
}

fn read_file<T: serde::de::DeserializeOwned>(name: &str) -> ReadPlan<T> {
    match data_file(name) {
        Some(p) => recents_read_plan(std::fs::read(p)),
        None => ReadPlan::Error("no data folder".into()),
    }
}

/// Write a whole file so a crash leaves the old one or the new one, never half.
fn write_whole(name: &str, value: &impl Serialize) -> Result<(), String> {
    use std::io::Write;
    let path = data_file(name).ok_or("no data folder")?;
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn read_recents(channel_id: &str) -> Option<InfoRecents> {
    let _held = locked(&RECENTS_LOCK);
    match read_file::<HashMap<String, InfoRecents>>(INFO_RECENTS_FILE) {
        ReadPlan::Parsed(mut all) => all.remove(channel_id),
        _ => None,
    }
}

/// A save's category and title emoji go to the front of the channel's
/// recents, for the fields the save named.
fn remember_save(channel_id: &str, draft: &StreamInfoDraft, fields: &[InfoField]) {
    let _held = locked(&RECENTS_LOCK);
    let mut all: HashMap<String, InfoRecents> = match read_file(INFO_RECENTS_FILE) {
        ReadPlan::Parsed(all) => all,
        ReadPlan::Missing | ReadPlan::Unparseable => HashMap::new(),
        ReadPlan::Error(e) => {
            log::warn!("[{LOG_TAG}] could not read the stream info recents, left as they are: {e}");
            return;
        }
    };
    let kept = all.get(channel_id).cloned().unwrap_or_default();
    let named = StreamInfoDraft {
        title: if fields.contains(&InfoField::Title) { draft.title.clone() } else { String::new() },
        category: if fields.contains(&InfoField::Category) { draft.category.clone() } else { None },
        tags: Vec::new(),
        language: String::new(),
    };
    all.insert(channel_id.to_string(), after_save(&kept, &named));
    if let Err(e) = write_whole(INFO_RECENTS_FILE, &all) {
        log::warn!("[{LOG_TAG}] could not keep the stream info recents: {e}");
    }
}

/// A poll or prediction started here, for the builder's recent list.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct RecentPoll {
    pub title: String,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct PollRecents {
    #[serde(default)]
    pub polls: Vec<RecentPoll>,
    #[serde(default)]
    pub predictions: Vec<RecentPoll>,
}

const RECENT_POLLS_MAX: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PollKind {
    Poll,
    Prediction,
}

/// `front` then `kept`, each title once (any case), at most 10.
pub fn merge_recents(front: &[RecentPoll], kept: &[RecentPoll]) -> Vec<RecentPoll> {
    merged(front, kept, RECENT_POLLS_MAX, |a, b| a.title.trim().eq_ignore_ascii_case(b.title.trim()))
}

/// The recents after a poll or prediction started with `title` and `options`.
pub fn after_start(recents: &PollRecents, kind: PollKind, title: &str, options: &[String]) -> PollRecents {
    let entry = [RecentPoll { title: title.trim().to_string(), options: options.to_vec() }];
    let mut next = recents.clone();
    match kind {
        PollKind::Poll => next.polls = merge_recents(&entry, &recents.polls),
        PollKind::Prediction => next.predictions = merge_recents(&entry, &recents.predictions),
    }
    next
}

/// One settings list (`recent_polls`) as entries, skipping any that do not read.
fn entries(value: Option<&Value>) -> Vec<RecentPoll> {
    value
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| serde_json::from_value::<RecentPoll>(v.clone()).ok())
                .filter(|r| !r.title.trim().is_empty())
                .take(RECENT_POLLS_MAX)
                .collect()
        })
        .unwrap_or_default()
}

/// The lists settings kept before Rust held them, or None when settings
/// never had either key (nothing to carry over).
pub fn seed_from(extra: &HashMap<String, Value>) -> Option<PollRecents> {
    if !extra.contains_key("recent_polls") && !extra.contains_key("recent_predictions") {
        return None;
    }
    Some(PollRecents {
        polls: entries(extra.get("recent_polls")),
        predictions: entries(extra.get("recent_predictions")),
    })
}

/// Carry the lists settings kept over to Rust's file, once: only while there
/// is no readable file yet.
pub fn seed_poll_recents(extra: &HashMap<String, Value>) {
    let Some(seed) = seed_from(extra) else { return };
    let _held = locked(&RECENTS_LOCK);
    if matches!(read_file::<PollRecents>(POLL_RECENTS_FILE), ReadPlan::Missing | ReadPlan::Unparseable) {
        if let Err(e) = write_whole(POLL_RECENTS_FILE, &seed) {
            log::warn!("[{LOG_TAG}] could not carry the poll recents over: {e}");
        }
    }
}

/// The recent polls and predictions, empty when none were kept.
pub fn poll_recents() -> PollRecents {
    let _held = locked(&RECENTS_LOCK);
    match read_file(POLL_RECENTS_FILE) {
        ReadPlan::Parsed(r) => r,
        _ => PollRecents::default(),
    }
}

fn update_poll_recents(change: impl FnOnce(&PollRecents) -> PollRecents) {
    let _held = locked(&RECENTS_LOCK);
    let kept: PollRecents = match read_file(POLL_RECENTS_FILE) {
        ReadPlan::Parsed(r) => r,
        ReadPlan::Missing | ReadPlan::Unparseable => PollRecents::default(),
        ReadPlan::Error(e) => {
            log::warn!("[{LOG_TAG}] could not read the poll recents, left as they are: {e}");
            return;
        }
    };
    if let Err(e) = write_whole(POLL_RECENTS_FILE, &change(&kept)) {
        log::warn!("[{LOG_TAG}] could not keep the poll recents: {e}");
    }
}

fn remember_poll(kind: PollKind, title: &str, options: &[String]) {
    update_poll_recents(|kept| after_start(kept, kind, title, options));
}

/// A settings backup's JSON with the recent lists put in, so a backup carries
/// them like any other setting.
pub fn with_poll_recents(value: &mut Value, recents: &PollRecents) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("recent_polls".into(), json!(recents.polls));
        obj.insert("recent_predictions".into(), json!(recents.predictions));
    }
}

/// An imported backup's lists, put in front of the ones kept here.
pub fn import_poll_recents(incoming: &serde_json::Map<String, Value>) {
    if !incoming.contains_key("recent_polls") && !incoming.contains_key("recent_predictions") {
        return;
    }
    let polls = entries(incoming.get("recent_polls"));
    let predictions = entries(incoming.get("recent_predictions"));
    update_poll_recents(|kept| PollRecents {
        polls: merge_recents(&polls, &kept.polls),
        predictions: merge_recents(&predictions, &kept.predictions),
    });
}

// ─────────────────────────────── polls and predictions ───────────────────────

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RunningPoll {
    pub id: String,
    pub title: String,
    /// When it ends by itself, on this device's clock.
    pub ends_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RunningPrediction {
    pub id: String,
    pub title: String,
    /// "active" (taking predictions) or "locked" (waiting for the result).
    pub status: String,
    pub outcomes: Vec<Outcome>,
    /// When it stops taking predictions by itself, on this device's clock;
    /// only while active.
    pub locks_at_ms: Option<i64>,
}

// ─────────────────────────────── clocks ──────────────────────────────────────

/// How far Twitch's clock runs ahead of this device's, from an answer's
/// `Date` header (`server_ms`) and when it arrived here. 0 without a header.
pub fn offset_from(server_ms: Option<i64>, local_ms: i64) -> i64 {
    server_ms.map(|s| s - local_ms).unwrap_or(0)
}

/// A Twitch time on this device's clock.
pub fn to_local(server_ms: i64, offset: i64) -> i64 {
    server_ms - offset
}

fn rfc3339_ms(v: &Value) -> Option<i64> {
    v.as_str()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.timestamp_millis())
}

/// `start` (an RFC 3339 Twitch time) plus `seconds`, on this device's clock.
fn local_end(start: &Value, seconds: &Value, offset: i64) -> Option<i64> {
    let s = seconds.as_i64().filter(|s| *s > 0)?;
    rfc3339_ms(start).map(|t| to_local(t + s * 1000, offset))
}

fn outcomes_of(list: &Value) -> Vec<Outcome> {
    list.as_array()
        .map(|a| {
            a.iter()
                .map(|o| Outcome {
                    id: o["id"].as_str().unwrap_or_default().to_string(),
                    title: o["title"].as_str().unwrap_or_default().to_string(),
                    color: o["color"].as_str().unwrap_or("BLUE").to_ascii_lowercase(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Outcome {
    pub id: String,
    pub title: String,
    pub color: String,
}

/// A Helix poll that is still running. `offset` is Twitch's clock against
/// this device's at that answer.
pub fn running_poll_helix(p: &Value, offset: i64) -> Option<RunningPoll> {
    (p["status"].as_str() == Some("ACTIVE")).then(|| RunningPoll {
        id: p["id"].as_str().unwrap_or_default().to_string(),
        title: p["title"].as_str().unwrap_or_default().to_string(),
        ends_at_ms: local_end(&p["started_at"], &p["duration"], offset),
    })
}

/// A Helix prediction that is open or locked.
pub fn running_prediction_helix(p: &Value, offset: i64) -> Option<RunningPrediction> {
    let status = match p["status"].as_str()? {
        "ACTIVE" => "active",
        "LOCKED" => "locked",
        _ => return None,
    };
    Some(RunningPrediction {
        id: p["id"].as_str().unwrap_or_default().to_string(),
        title: p["title"].as_str().unwrap_or_default().to_string(),
        status: status.into(),
        outcomes: outcomes_of(&p["outcomes"]),
        locks_at_ms: (status == "active").then(|| local_end(&p["created_at"], &p["prediction_window"], offset)).flatten(),
    })
}

/// What one read of the running poll and prediction found. An `Err` part
/// could not be read (the words say why); `Ok(None)` is nothing running.
#[derive(Clone, Debug, PartialEq)]
pub struct RunningRead {
    pub poll: Result<Option<RunningPoll>, String>,
    pub prediction: Result<Option<RunningPrediction>, String>,
}

impl RunningRead {
    fn failed(why: &str) -> RunningRead {
        RunningRead { poll: Err(why.to_string()), prediction: Err(why.to_string()) }
    }
}

/// Whether the part of a GQL answer at `part` (`["user", "latestPoll"]`)
/// came through: not when an error has no path, or its path is a prefix of
/// the part's or runs inside it.
pub fn part_known(errors: &Value, part: &[&str]) -> bool {
    let Some(list) = errors.as_array() else { return true };
    !list.iter().any(|e| {
        let path: Vec<String> = e["path"]
            .as_array()
            .map(|p| p.iter().map(|s| s.as_str().map(str::to_string).unwrap_or_else(|| s.to_string())).collect())
            .unwrap_or_default();
        if path.is_empty() {
            return true;
        }
        let n = path.len().min(part.len());
        path[..n].iter().zip(&part[..n]).all(|(a, b)| a == b)
    })
}

/// The GQL answer to RUNNING_QUERY: the latest poll while ACTIVE, and the
/// first active, else locked, prediction. `now` is this device's clock when
/// the answer came, `offset` Twitch's against it.
pub fn running_from_gql(body: &Value, now: i64, offset: i64) -> RunningRead {
    let errors = &body["errors"];
    let why = || {
        gql_error_words(errors[0]["message"].as_str().unwrap_or("Twitch did not answer"))
    };
    let poll = if part_known(errors, &["user", "latestPoll"]) {
        Ok(body["data"]["user"]["latestPoll"].as_object().and_then(|p| {
            (p.get("status").and_then(Value::as_str) == Some("ACTIVE")).then(|| RunningPoll {
                id: p.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                title: p.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                ends_at_ms: p
                    .get("remainingDurationMilliseconds")
                    .and_then(Value::as_i64)
                    .map(|ms| now + ms.max(0))
                    .or_else(|| {
                        local_end(
                            p.get("startedAt").unwrap_or(&Value::Null),
                            p.get("durationSeconds").unwrap_or(&Value::Null),
                            offset,
                        )
                    }),
            })
        }))
    } else {
        Err(why())
    };
    let pick = |key: &str, status: &str| -> Option<RunningPrediction> {
        let p = body["data"]["channel"][key].as_array()?.first()?;
        Some(RunningPrediction {
            id: p["id"].as_str().unwrap_or_default().to_string(),
            title: p["title"].as_str().unwrap_or_default().to_string(),
            status: status.into(),
            outcomes: outcomes_of(&p["outcomes"]),
            locks_at_ms: (status == "active").then(|| local_end(&p["createdAt"], &p["predictionWindowSeconds"], offset)).flatten(),
        })
    };
    let prediction = if part_known(errors, &["channel", "activePredictionEvents"])
        && part_known(errors, &["channel", "lockedPredictionEvents"])
    {
        Ok(pick("activePredictionEvents", "active").or_else(|| pick("lockedPredictionEvents", "locked")))
    } else {
        Err(why())
    };
    RunningRead { poll, prediction }
}

/// The poll and prediction a view shows: what this read found, and for a
/// part it could not read, the last one known for this account and channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Running {
    pub poll: Option<RunningPoll>,
    pub prediction: Option<RunningPrediction>,
    pub poll_known: bool,
    pub prediction_known: bool,
}

pub fn settle_running(read: &RunningRead, last: Option<&Running>) -> Running {
    let last = last.cloned().unwrap_or_default();
    let (poll, poll_known) = match &read.poll {
        Ok(p) => (p.clone(), true),
        Err(_) => (last.poll, false),
    };
    let (prediction, prediction_known) = match &read.prediction {
        Ok(p) => (p.clone(), true),
        Err(_) => (last.prediction, false),
    };
    Running { poll, prediction, poll_known, prediction_known }
}

/// The last poll and prediction read, per (account, channel).
static LAST_RUNNING: Lazy<Mutex<HashMap<(String, String), Running>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn settle_and_keep(user_id: &str, channel_id: &str, read: &RunningRead) -> Running {
    let key = (user_id.to_string(), channel_id.to_string());
    let mut last = locked(&LAST_RUNNING);
    let settled = settle_running(read, last.get(&key));
    last.insert(key, settled.clone());
    settled
}

/// The outcome `wanted` names, by its place (1 is the first) or its title.
pub fn pick_outcome(outcomes: &[Outcome], wanted: &str) -> Result<String, String> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Err("Say which outcome won, by number or title".into());
    }
    wanted
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|i| outcomes.get(i))
        .or_else(|| outcomes.iter().find(|o| o.title.trim().eq_ignore_ascii_case(wanted)))
        .map(|o| o.id.clone())
        .ok_or_else(|| format!("No outcome matches \"{wanted}\""))
}

/// Why a poll can't be created as filled in, or the cleaned choices.
pub fn check_poll(title: &str, choices: &[String], seconds: u32) -> Result<Vec<String>, String> {
    let l = form_limits();
    let title = title.trim();
    if title.is_empty() {
        return Err("Add a question".into());
    }
    if title.chars().count() > l.poll_title {
        return Err(format!("The question can be at most {} characters", l.poll_title));
    }
    let choices: Vec<String> = choices.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    if choices.len() < l.poll_choices_min || choices.len() > l.poll_choices_max {
        return Err(format!("A poll has {} to {} choices", l.poll_choices_min, l.poll_choices_max));
    }
    if choices.iter().any(|c| c.chars().count() > l.poll_choice) {
        return Err(format!("Each choice can be at most {} characters", l.poll_choice));
    }
    if !(l.poll_seconds_min..=l.poll_seconds_max).contains(&seconds) {
        return Err("A poll runs from 15 seconds to 30 minutes".into());
    }
    Ok(choices)
}

pub fn check_prediction(title: &str, outcomes: &[String], seconds: u32) -> Result<Vec<String>, String> {
    let l = form_limits();
    let title = title.trim();
    if title.is_empty() {
        return Err("Add a question".into());
    }
    if title.chars().count() > l.prediction_title {
        return Err(format!("The question can be at most {} characters", l.prediction_title));
    }
    let outcomes: Vec<String> = outcomes.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    if outcomes.len() < l.prediction_outcomes_min || outcomes.len() > l.prediction_outcomes_max {
        return Err(format!(
            "A prediction has {} to {} outcomes",
            l.prediction_outcomes_min, l.prediction_outcomes_max
        ));
    }
    if outcomes.iter().any(|c| c.chars().count() > l.prediction_outcome) {
        return Err(format!("Each outcome can be at most {} characters", l.prediction_outcome));
    }
    if !(l.prediction_seconds_min..=l.prediction_seconds_max).contains(&seconds) {
        return Err("Predictions stay open from 30 seconds to 30 minutes".into());
    }
    Ok(outcomes)
}

/// Twitch's colors for prediction outcomes as its dashboard sends them: the
/// first blue, every other pink.
fn outcome_color(i: usize) -> &'static str {
    if i == 0 {
        "BLUE"
    } else {
        "PINK"
    }
}

// ─────────────────────────────── raid ────────────────────────────────────────

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RaidView {
    pub target_id: String,
    pub target_login: String,
    pub target_name: String,
    /// When the countdown ends by itself; the shells tick `m:ss` to it.
    pub deadline_ms: i64,
    /// The whole countdown, so the shells can draw how much is left.
    pub total_ms: i64,
}

static RAIDS: Lazy<Mutex<HashMap<String, RaidView>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn pending_raid(channel_id: &str, now_ms: i64) -> Option<RaidView> {
    let mut raids = RAIDS.lock().ok()?;
    match raids.get(channel_id) {
        Some(r) if r.deadline_ms > now_ms => Some(r.clone()),
        Some(_) => {
            raids.remove(channel_id);
            None
        }
        None => None,
    }
}

fn set_raid(channel_id: &str, raid: Option<RaidView>) {
    if let Ok(mut raids) = RAIDS.lock() {
        match raid {
            Some(r) => raids.insert(channel_id.to_string(), r),
            None => raids.remove(channel_id),
        };
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RaidTarget {
    pub id: String,
    pub login: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub game_name: String,
    pub live: bool,
    /// "Valorant" for a live channel, "Offline" otherwise.
    pub line: String,
}

/// Search results as raid targets: live channels first, the channel itself
/// left out (a raid on yourself fails).
pub fn raid_targets(streams: &[crate::models::stream::TwitchStream], own_id: &str) -> Vec<RaidTarget> {
    let mut out: Vec<RaidTarget> = streams
        .iter()
        .filter(|s| s.user_id != own_id)
        .map(|s| {
            let live = s.is_live.unwrap_or(false);
            RaidTarget {
                id: s.user_id.clone(),
                login: s.user_login.clone(),
                name: s.user_name.clone(),
                avatar_url: s.profile_image_url.clone(),
                game_name: s.game_name.clone(),
                live,
                line: if !live {
                    "Offline".into()
                } else if s.game_name.is_empty() {
                    "Live".into()
                } else {
                    s.game_name.clone()
                },
            }
        })
        .collect();
    out.sort_by_key(|t| !t.live);
    out
}

// ─────────────────────────────── ads ─────────────────────────────────────────

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AdsView {
    pub lengths: Vec<Choice>,
    pub schedule: Option<AdSchedule>,
    /// Set when this sign-in predates the ad scopes: one line saying how to
    /// see the schedule. Running an ad works either way.
    pub schedule_hint: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AdSchedule {
    /// The next scheduled ad, or none when no ad is scheduled.
    pub next_ad_ms: Option<i64>,
    pub next_ad_seconds: u32,
    pub preroll_free_seconds: u32,
    pub snoozes_left: u32,
    pub snooze_refresh_ms: Option<i64>,
    /// "2 snoozes left", "No snoozes left, the next comes back in 42 min".
    pub snooze_line: String,
    /// "Pre-rolls are off for 23 min" when the last break bought that.
    pub preroll_line: Option<String>,
}

const ADS_SCOPE_HINT: &str = "Sign in to Twitch again from your accounts to see your ad schedule here.";

fn ad_lengths() -> Vec<Choice> {
    AD_LENGTHS
        .iter()
        .map(|s| Choice { label: if *s < 60 { format!("{s}s") } else if s % 60 == 0 { format!("{} min", s / 60) } else { format!("{}:{:02}", s / 60, s % 60) }, value: *s })
        .collect()
}

fn minutes_words(ms: i64) -> String {
    let mins = ((ms.max(0) + 59_999) / 60_000).max(1);
    if mins >= 60 {
        let h = mins / 60;
        let m = mins % 60;
        if m == 0 {
            format!("{h} h")
        } else {
            format!("{h} h {m} min")
        }
    } else {
        format!("{mins} min")
    }
}

/// Helix `GET channels/ads` to the schedule. Times there are unix seconds on
/// Twitch's clock (0 when unset); `offset` puts them on this device's.
pub fn ad_schedule(row: &Value, now_ms: i64, offset: i64) -> AdSchedule {
    let secs = |k: &str| -> i64 {
        row[k]
            .as_i64()
            .or_else(|| row[k].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0)
    };
    let at = |k: &str| -> Option<i64> { Some(secs(k)).filter(|s| *s > 0).map(|s| to_local(s * 1000, offset)) };
    let snoozes_left = secs("snooze_count").max(0) as u32;
    let snooze_refresh_ms = at("snooze_refresh_at");
    let snooze_line = match (snoozes_left, snooze_refresh_ms) {
        (0, Some(t)) if t > now_ms => format!("No snoozes left, one comes back in {}", minutes_words(t - now_ms)),
        (0, _) => "No snoozes left".into(),
        (1, _) => "1 snooze left".into(),
        (n, _) => format!("{n} snoozes left"),
    };
    let preroll = secs("preroll_free_time").max(0) as u32;
    AdSchedule {
        next_ad_ms: at("next_ad_at"),
        next_ad_seconds: secs("duration").max(0) as u32,
        preroll_free_seconds: preroll,
        snoozes_left,
        snooze_refresh_ms,
        snooze_line,
        preroll_line: (preroll > 0).then(|| format!("Pre-rolls are off for {}", minutes_words(preroll as i64 * 1000))),
    }
}

/// The ad row's second line: "Next ad in 14 min · 2 snoozes left".
pub fn ads_detail(ads: &AdsView, now_ms: i64) -> Option<String> {
    let s = ads.schedule.as_ref()?;
    let next = s
        .next_ad_ms
        .filter(|t| *t > now_ms)
        .map(|t| format!("Next ad in {}", minutes_words(t - now_ms)));
    match next {
        Some(n) => Some(format!("{n} · {}", s.snooze_line)),
        None => Some(s.snooze_line.clone()),
    }
}

// ─────────────────────────────── chat ────────────────────────────────────────

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ChatView {
    pub modes: Vec<ChatModeRow>,
    /// Shield Mode, when it could be read.
    pub shield: Option<bool>,
    pub announce_colors: Vec<AnnounceColor>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ChatModeRow {
    /// "followers", "slow", "subs", "emotes", "unique".
    pub kind: String,
    pub label: String,
    pub on: bool,
    /// The length in force (followers minutes, slow seconds), when it has one.
    pub value: Option<u32>,
    pub value_label: Option<String>,
    /// The lengths offered, empty for a plain switch.
    pub options: Vec<Choice>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AnnounceColor {
    pub id: String,
    pub label: String,
}

fn followers_options() -> Vec<Choice> {
    vec![
        choice("Any follower", 0),
        choice("10 min", 10),
        choice("30 min", 30),
        choice("1 hour", 60),
        choice("1 day", 1440),
        choice("1 week", 10080),
        choice("1 month", 43200),
        choice("3 months", 129600),
    ]
}

fn slow_options() -> Vec<Choice> {
    vec![
        choice("3s", 3),
        choice("5s", 5),
        choice("10s", 10),
        choice("20s", 20),
        choice("30s", 30),
        choice("60s", 60),
        choice("120s", 120),
    ]
}

fn label_for(options: &[Choice], value: u32, fallback: impl Fn(u32) -> String) -> String {
    options
        .iter()
        .find(|c| c.value == value)
        .map(|c| c.label.clone())
        .unwrap_or_else(|| fallback(value))
}

/// Helix `GET chat/settings` (and Shield Mode when read) to the chat block.
pub fn chat_view(settings: &Value, shield: Option<bool>) -> ChatView {
    let flag = |k: &str| settings[k].as_bool().unwrap_or(false);
    let num = |k: &str| settings[k].as_u64().map(|n| n as u32);
    let fo = flag("follower_mode");
    let fv = num("follower_mode_duration").filter(|_| fo);
    let so = flag("slow_mode");
    let sv = num("slow_mode_wait_time").filter(|_| so);
    let followers = followers_options();
    let slow = slow_options();
    let modes = vec![
        ChatModeRow {
            kind: "followers".into(),
            label: "Followers-only".into(),
            on: fo,
            value: fv,
            value_label: fv.map(|v| label_for(&followers, v, |v| format!("{v} min"))),
            options: followers,
        },
        ChatModeRow {
            kind: "slow".into(),
            label: "Slow mode".into(),
            on: so,
            value: sv,
            value_label: sv.map(|v| label_for(&slow, v, |v| format!("{v}s"))),
            options: slow,
        },
        ChatModeRow { kind: "subs".into(), label: "Subscriber-only".into(), on: flag("subscriber_mode"), value: None, value_label: None, options: vec![] },
        ChatModeRow { kind: "emotes".into(), label: "Emote-only".into(), on: flag("emote_mode"), value: None, value_label: None, options: vec![] },
        ChatModeRow { kind: "unique".into(), label: "Unique chat".into(), on: flag("unique_chat_mode"), value: None, value_label: None, options: vec![] },
    ];
    ChatView {
        modes,
        shield,
        announce_colors: ["primary", "blue", "green", "orange", "purple"]
            .iter()
            .map(|c| AnnounceColor {
                id: c.to_string(),
                label: if *c == "primary" { "Channel color".into() } else { capitalize(c) },
            })
            .collect(),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// The Helix chat settings body that turns `mode` on or off.
pub fn chat_mode_body(mode: &str, on: bool, value: Option<u32>) -> Result<Value, String> {
    Ok(match mode {
        "followers" => {
            let mut b = json!({ "follower_mode": on });
            if on {
                b["follower_mode_duration"] = json!(value.unwrap_or(0));
            }
            b
        }
        "slow" => {
            let mut b = json!({ "slow_mode": on });
            if on {
                b["slow_mode_wait_time"] = json!(value.unwrap_or(30).clamp(3, 120));
            }
            b
        }
        "subs" => json!({ "subscriber_mode": on }),
        "emotes" => json!({ "emote_mode": on }),
        "unique" => json!({ "unique_chat_mode": on }),
        other => return Err(format!("Unknown chat mode {other}")),
    })
}

// ─────────────────────────────── rewards queue ───────────────────────────────

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct QueueView {
    /// Taken when this read started (`next_rev`).
    pub rev: u64,
    /// The Twitch account the queue was read as.
    pub user_id: String,
    pub groups: Vec<QueueGroup>,
    pub count: usize,
    /// "Nothing waiting" when empty, so the shells never word it themselves.
    pub empty_line: Option<String>,
    /// More than one page waits; the shells say "and more on twitch.tv".
    pub more: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct QueueGroup {
    pub reward_id: String,
    pub title: String,
    pub cost: i64,
    pub paused: bool,
    pub enabled: bool,
    pub image_url: Option<String>,
    pub color: Option<String>,
    /// "Song request · 2".
    pub heading: String,
    pub rows: Vec<QueueRow>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct QueueRow {
    pub id: String,
    pub user_login: String,
    pub user_name: String,
    pub avatar_url: Option<String>,
    /// What the viewer typed, when the reward asks for text.
    pub input: Option<String>,
    pub at_ms: i64,
    /// "pixelfox · 2 min ago".
    pub line: String,
}

pub fn ago_words(at_ms: i64, now_ms: i64) -> String {
    let s = ((now_ms - at_ms).max(0)) / 1000;
    if s < 45 {
        "just now".into()
    } else if s < 3600 {
        format!("{} min ago", ((s + 30) / 60).max(1))
    } else if s < 86_400 {
        format!("{} h ago", s / 3600)
    } else {
        format!("{} d ago", s / 86_400)
    }
}

/// The GQL queue answer, grouped by reward in the order their oldest
/// redemption arrived.
pub fn parse_queue(body: &Value, now_ms: i64) -> QueueView {
    let conn = &body["data"]["channel"]["communityPointsRedemptionQueue"];
    let mut groups: Vec<QueueGroup> = Vec::new();
    let mut count = 0usize;
    for edge in conn["edges"].as_array().into_iter().flatten() {
        let n = &edge["node"];
        if n["status"].as_str().unwrap_or("UNFULFILLED") != "UNFULFILLED" {
            continue;
        }
        let r = &n["reward"];
        let reward_id = r["id"].as_str().unwrap_or_default().to_string();
        let at_ms = n["timestamp"]
            .as_str()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp_millis())
            .unwrap_or(now_ms);
        let name = n["user"]["displayName"].as_str().or(n["user"]["login"].as_str()).unwrap_or("Someone").to_string();
        let row = QueueRow {
            id: n["id"].as_str().unwrap_or_default().to_string(),
            user_login: n["user"]["login"].as_str().unwrap_or_default().to_string(),
            avatar_url: n["user"]["profileImageURL"].as_str().map(str::to_string),
            input: n["input"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
            line: format!("{name} · {}", ago_words(at_ms, now_ms)),
            user_name: name,
            at_ms,
        };
        count += 1;
        match groups.iter_mut().find(|g| g.reward_id == reward_id) {
            Some(g) => g.rows.push(row),
            None => groups.push(QueueGroup {
                title: r["title"].as_str().or(n["rewardTitle"].as_str()).unwrap_or("Reward").to_string(),
                cost: r["cost"].as_i64().unwrap_or(0),
                paused: r["isPaused"].as_bool().unwrap_or(false),
                enabled: r["isEnabled"].as_bool().unwrap_or(true),
                // A reward with no art of its own wears Twitch's default reward
                // picture. Not read from `defaultImage`: Twitch declares it
                // non-null yet answered nil for one, which failed the whole queue
                // ("got nil for non-null communitypointsimage", SN-264).
                image_url: Some(
                    r["image"]["url"].as_str().unwrap_or(DEFAULT_REWARD_IMAGE).to_string(),
                ),
                color: r["backgroundColor"].as_str().map(str::to_string),
                heading: String::new(),
                rows: vec![row],
                reward_id,
            }),
        }
    }
    for g in &mut groups {
        g.heading = format!("{} · {}", g.title, g.rows.len());
    }
    QueueView {
        rev: 0,
        user_id: String::new(),
        empty_line: (count == 0).then(|| "Nothing waiting".to_string()),
        more: conn["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false),
        groups,
        count,
    }
}

// ─────────────────────────────── GQL documents ───────────────────────────────

pub const QUEUE_QUERY: &str = "query StreamManagerQueue($channelID: ID!) { channel(id: $channelID) { communityPointsRedemptionQueue(first: 50, options: { order: OLDEST, status: UNFULFILLED }) { edges { node { id status input timestamp rewardTitle user { id login displayName profileImageURL(width: 50) } reward { id title cost isPaused isEnabled backgroundColor image { url } } } } pageInfo { hasNextPage } } } }";
/// Twitch's own picture for a custom reward with no image (the 4x size; the
/// queue draws it small).
pub const DEFAULT_REWARD_IMAGE: &str = "https://static-cdn.jtvnw.net/custom-reward-images/default-4.png";
pub const ROLE_QUERY: &str = "query StreamManagerRole($channelID: ID!) { user(id: $channelID) { self { isEditor isModerator } } }";
pub const RUNNING_QUERY: &str = "query StreamManagerRunning($channelID: ID!) { user(id: $channelID) { latestPoll { id status title startedAt durationSeconds remainingDurationMilliseconds } } channel(id: $channelID) { activePredictionEvents { id status title createdAt predictionWindowSeconds outcomes { id title color } } lockedPredictionEvents { id status title createdAt predictionWindowSeconds outcomes { id title color } } } }";
pub const REDEMPTIONS_MUTATION: &str = "mutation StreamManagerRedemptions($input: UpdateCommunityPointsCustomRewardRedemptionStatusesByRedemptionsInput!) { updateCommunityPointsCustomRewardRedemptionStatusesByRedemptions(input: $input) { error { code } } }";
pub const REWARD_QUEUE_MUTATION: &str = "mutation StreamManagerRewardQueue($input: UpdateCommunityPointsCustomRewardRedemptionStatusesByRewardInput!) { updateCommunityPointsCustomRewardRedemptionStatusesByReward(input: $input) { error { code } } }";
pub const REWARD_MUTATION: &str = "mutation StreamManagerReward($input: UpdateCommunityPointsCustomRewardInput!) { updateCommunityPointsCustomReward(input: $input) { reward { id isPaused isEnabled } error { code } } }";
pub const BROADCAST_MUTATION: &str = "mutation StreamManagerBroadcast($input: UpdateBroadcastSettingsInput!) { updateBroadcastSettings(input: $input) { error } }";
pub const TAGS_MUTATION: &str = "mutation StreamManagerTags($input: SetFreeformTagsInput!) { setFreeformTags(input: $input) { error { code } } }";
pub const CREATE_POLL_MUTATION: &str = "mutation StreamManagerCreatePoll($input: CreatePollInput!) { createPoll(input: $input) { poll { id } error { code } } }";
pub const TERMINATE_POLL_MUTATION: &str = "mutation StreamManagerEndPoll($input: TerminatePollInput!) { terminatePoll(input: $input) { poll { id status } } }";
pub const ARCHIVE_POLL_MUTATION: &str = "mutation StreamManagerArchivePoll($input: ArchivePollInput!) { archivePoll(input: $input) { poll { id status } } }";
pub const CREATE_PREDICTION_MUTATION: &str = "mutation StreamManagerCreatePrediction($input: CreatePredictionEventInput!) { createPredictionEvent(input: $input) { predictionEvent { id } error { code } } }";
pub const LOCK_PREDICTION_MUTATION: &str = "mutation StreamManagerLockPrediction($input: LockPredictionEventInput!) { lockPredictionEvent(input: $input) { error { code } } }";
pub const RESOLVE_PREDICTION_MUTATION: &str = "mutation StreamManagerResolvePrediction($input: ResolvePredictionEventInput!) { resolvePredictionEvent(input: $input) { error { code } } }";
pub const CANCEL_PREDICTION_MUTATION: &str = "mutation StreamManagerCancelPrediction($input: CancelPredictionEventInput!) { cancelPredictionEvent(input: $input) { error { code } } }";

fn op(name: &str, query: &str, variables: Value) -> Value {
    json!({ "operationName": name, "query": query, "variables": variables })
}

pub fn redemptions_request(channel_id: &str, ids: &[String], fulfill: bool) -> Value {
    op(
        "StreamManagerRedemptions",
        REDEMPTIONS_MUTATION,
        json!({ "input": {
            "channelID": channel_id,
            "redemptionIDs": ids,
            "newStatus": if fulfill { "FULFILLED" } else { "CANCELED" },
            "oldStatus": "UNFULFILLED",
        } }),
    )
}

pub fn reward_queue_request(channel_id: &str, reward_id: &str, fulfill: bool) -> Value {
    op(
        "StreamManagerRewardQueue",
        REWARD_QUEUE_MUTATION,
        json!({ "input": {
            "channelID": channel_id,
            "rewardID": reward_id,
            "newStatus": if fulfill { "FULFILLED" } else { "CANCELED" },
            "oldStatus": "UNFULFILLED",
        } }),
    )
}

pub fn reward_request(channel_id: &str, reward_id: &str, paused: Option<bool>, enabled: Option<bool>) -> Value {
    let mut input = json!({ "channelID": channel_id, "rewardID": reward_id });
    if let Some(p) = paused {
        input["isPaused"] = json!(p);
    }
    if let Some(e) = enabled {
        input["isEnabled"] = json!(e);
    }
    op("StreamManagerReward", REWARD_MUTATION, json!({ "input": input }))
}

pub fn broadcast_request(channel_id: &str, draft: &StreamInfoDraft) -> Value {
    let mut input = json!({ "userID": channel_id, "status": draft.title.trim() });
    if let Some(c) = &draft.category {
        input["categoryID"] = json!(c.id);
        input["game"] = json!(c.name);
    }
    if !draft.language.is_empty() {
        input["broadcasterLanguage"] = json!(draft.language);
    }
    op("StreamManagerBroadcast", BROADCAST_MUTATION, json!({ "input": input }))
}

pub fn tags_request(channel_id: &str, tags: &[String]) -> Value {
    op(
        "StreamManagerTags",
        TAGS_MUTATION,
        json!({ "input": { "contentID": channel_id, "contentType": "CHANNEL", "freeformTagNames": tags } }),
    )
}

pub fn create_poll_request(channel_id: &str, title: &str, choices: &[String], seconds: u32, points_per_vote: Option<u32>) -> Value {
    let mut input = json!({
        "ownedBy": channel_id,
        "title": title.trim(),
        "choices": choices.iter().map(|c| json!({ "title": c })).collect::<Vec<_>>(),
        "durationSeconds": seconds,
        "multichoiceEnabled": true,
        "isCommunityPointsVotingEnabled": points_per_vote.is_some(),
    });
    if let Some(p) = points_per_vote {
        input["communityPointsCost"] = json!(p);
    }
    op("StreamManagerCreatePoll", CREATE_POLL_MUTATION, json!({ "input": input }))
}

pub fn create_prediction_request(channel_id: &str, title: &str, outcomes: &[String], seconds: u32) -> Value {
    op(
        "StreamManagerCreatePrediction",
        CREATE_PREDICTION_MUTATION,
        json!({ "input": {
            "channelID": channel_id,
            "title": title.trim(),
            "predictionWindowSeconds": seconds,
            "outcomes": outcomes.iter().enumerate().map(|(i, t)| json!({ "title": t, "color": outcome_color(i) })).collect::<Vec<_>>(),
        } }),
    )
}

/// Read a GQL write's answer: request errors, then `<field>.error` (an object
/// with a code, or a bare code), in words.
pub fn gql_outcome(body: &Value, field: &str, failed: &str) -> Result<(), String> {
    if let Some(errors) = body["errors"].as_array().filter(|e| !e.is_empty()) {
        let msg = errors[0]["message"].as_str().unwrap_or("error");
        return Err(format!("{failed}: {}", gql_error_words(msg)));
    }
    let err = &body["data"][field]["error"];
    if err.is_null() {
        if body["data"][field].is_null() {
            return Err(format!("{failed}: Twitch did not answer"));
        }
        return Ok(());
    }
    let code = err["code"].as_str().or_else(|| err.as_str()).unwrap_or("UNKNOWN");
    Err(format!("{failed}: {}", gql_error_words(code)))
}

fn gql_error_words(code: &str) -> String {
    let upper = code.to_ascii_uppercase();
    if upper.contains("FORBIDDEN") || upper.contains("UNAUTHORIZED") || upper.contains("PERMISSION") {
        return "Twitch says you can't do that in this channel".into();
    }
    if upper.contains("INTEGRITY") {
        return "Twitch's security check stopped it. Try it in the desktop app".into();
    }
    if upper.contains("NOT_FOUND") {
        return "it is no longer there".into();
    }
    if upper.contains("ALREADY") || upper.contains("ACTIVE") {
        return "one is already running".into();
    }
    code.to_ascii_lowercase().replace('_', " ")
}

// ─────────────────────────────── actions ─────────────────────────────────────

/// Every write the manager makes, through one door.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    /// Save the named fields of `draft`; the rest stay as they are on Twitch.
    SaveInfo { draft: StreamInfoDraft, fields: Vec<InfoField> },
    CreatePoll { title: String, choices: Vec<String>, seconds: u32, #[serde(default)] points_per_vote: Option<u32> },
    /// End the running poll; `show_results` keeps it on screen, else it is removed.
    EndPoll { show_results: bool },
    CreatePrediction { title: String, outcomes: Vec<String>, seconds: u32 },
    LockPrediction,
    CancelPrediction,
    ResolvePrediction { outcome_id: String },
    /// Pay out to the outcome `pick` names, by place (1 is the first) or title
    /// (typed `/completeprediction`).
    ResolvePredictionPick { pick: String },
    Raid { target_id: String, target_login: String, target_name: String },
    CancelRaid,
    RaidNow,
    RunAd { seconds: u32 },
    SnoozeAd,
    /// Complete (`fulfill`) or reject and refund the given redemptions.
    Redemptions { ids: Vec<String>, fulfill: bool },
    /// Complete or reject everything waiting for one reward.
    RewardQueue { reward_id: String, fulfill: bool },
    Reward { reward_id: String, #[serde(default)] paused: Option<bool>, #[serde(default)] enabled: Option<bool> },
    ChatMode { mode: String, on: bool, #[serde(default)] value: Option<u32> },
    Shield { on: bool },
    Announce { text: String, #[serde(default)] color: Option<String> },
}

impl Action {
    fn section(&self) -> Section {
        match self {
            Action::SaveInfo { .. } => Section::Info,
            Action::CreatePoll { .. } | Action::EndPoll { .. } => Section::Poll,
            Action::CreatePrediction { .. }
            | Action::LockPrediction
            | Action::CancelPrediction
            | Action::ResolvePrediction { .. }
            | Action::ResolvePredictionPick { .. } => Section::Prediction,
            Action::Raid { .. } | Action::CancelRaid | Action::RaidNow => Section::Raid,
            Action::RunAd { .. } | Action::SnoozeAd => Section::Ads,
            Action::Redemptions { .. } | Action::RewardQueue { .. } | Action::Reward { .. } => Section::Rewards,
            Action::ChatMode { .. } | Action::Shield { .. } => Section::Chat,
            Action::Announce { .. } => Section::Announce,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ActOutcome {
    pub ok: bool,
    /// One line for a toast: what happened, or why not.
    pub message: String,
    /// The view after the write, so the shells redraw from Rust.
    pub view: Option<StreamManagerView>,
    /// The queue after a queue write.
    pub queue: Option<QueueView>,
    /// The Twitch account the action ran as, when one was signed in.
    pub user_id: Option<String>,
}

impl ActOutcome {
    fn refused(message: impl Into<String>, user_id: Option<String>) -> ActOutcome {
        ActOutcome { ok: false, message: message.into(), view: None, queue: None, user_id }
    }
}

/// One field of the stream info form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InfoField {
    Title,
    Category,
    Tags,
    Language,
}

impl InfoField {
    /// Whether a form problem (`"title"`, `"tag:2"`, ...) belongs to this field.
    fn owns(self, problem_field: &str) -> bool {
        match self {
            InfoField::Title => problem_field == "title",
            InfoField::Category => problem_field == "category",
            InfoField::Tags => problem_field == "tags" || problem_field.starts_with("tag:"),
            InfoField::Language => problem_field == "language",
        }
    }
}

/// The form's rules for the named fields only; the cleaned tags.
pub fn check_info_fields(draft: &StreamInfoDraft, fields: &[InfoField]) -> Result<Vec<String>, String> {
    if fields.is_empty() {
        return Err("Nothing changed".into());
    }
    let check = check_stream_info(draft, None, "", None, None);
    if let Some(p) = check.problems.iter().find(|p| fields.iter().any(|f| f.owns(&p.field))) {
        return Err(p.line.clone());
    }
    Ok(check.preview.tags)
}

/// The Helix PATCH body with only the named fields. A category or language
/// the form left empty is never sent: the form cannot clear them.
pub fn info_patch(draft: &StreamInfoDraft, fields: &[InfoField], tags: &[String]) -> Result<Value, String> {
    let mut body = serde_json::Map::new();
    for f in fields {
        match f {
            InfoField::Title => {
                body.insert("title".into(), json!(draft.title.trim()));
            }
            InfoField::Category => {
                if let Some(c) = &draft.category {
                    body.insert("game_id".into(), json!(c.id));
                }
            }
            InfoField::Tags => {
                body.insert("tags".into(), json!(tags));
            }
            InfoField::Language => {
                if !draft.language.is_empty() {
                    body.insert("broadcaster_language".into(), json!(draft.language));
                }
            }
        }
    }
    if body.is_empty() {
        return Err("Nothing changed".into());
    }
    Ok(Value::Object(body))
}

/// Which GQL writes a save makes: (broadcast settings, tags).
pub fn gql_save_plan(fields: &[InfoField]) -> (bool, bool) {
    let broadcast = fields.iter().any(|f| matches!(f, InfoField::Title | InfoField::Category | InfoField::Language));
    (broadcast, fields.contains(&InfoField::Tags))
}

/// The form as Twitch has it now.
pub fn draft_from_info(info: &StreamInfoView) -> StreamInfoDraft {
    StreamInfoDraft {
        title: info.title.clone(),
        category: info.category.clone(),
        tags: info.tags.clone(),
        language: info.language.clone(),
    }
}

/// One field's value, for comparing a remembered write with a fresh read.
pub fn field_value(d: &StreamInfoDraft, f: InfoField) -> Value {
    match f {
        InfoField::Title => json!(d.title.trim()),
        InfoField::Category => json!(d.category.as_ref().map(|c| c.id.clone())),
        InfoField::Tags => json!(d.tags.iter().map(|t| clean_tag(t).to_lowercase()).collect::<Vec<_>>()),
        InfoField::Language => json!(d.language.to_ascii_lowercase()),
    }
}

fn set_field(d: &mut StreamInfoDraft, from: &StreamInfoDraft, f: InfoField) {
    match f {
        InfoField::Title => d.title = from.title.clone(),
        InfoField::Category => d.category = from.category.clone(),
        InfoField::Tags => d.tags = from.tags.clone(),
        InfoField::Language => d.language = from.language.clone(),
    }
}

/// An earlier save's field: what it wrote, what the field held before it, and
/// the draft it came from.
#[derive(Clone, Debug)]
pub struct KeptWrite {
    pub written: Value,
    pub before: Value,
    pub draft: StreamInfoDraft,
    pub at_ms: i64,
}

/// How long a save's fields are remembered for the next save's merge.
const KEPT_WRITE_MS: i64 = 30_000;

/// What a GQL save sends: the named fields from `draft`; each other field
/// from `live`, unless an earlier save here wrote it so recently that a read
/// may not show it yet (live still holds what was there before that save).
pub fn merged_info(draft: &StreamInfoDraft, fields: &[InfoField], live: &StreamInfoDraft, kept: &HashMap<InfoField, KeptWrite>, now: i64) -> StreamInfoDraft {
    let mut out = live.clone();
    for f in [InfoField::Title, InfoField::Category, InfoField::Tags, InfoField::Language] {
        if fields.contains(&f) {
            set_field(&mut out, draft, f);
        } else if let Some(k) = kept.get(&f) {
            if now - k.at_ms < KEPT_WRITE_MS && field_value(live, f) == k.before {
                set_field(&mut out, &k.draft, f);
            }
        }
    }
    out
}

/// The kept writes after a fresh read: a field a read already shows, or one
/// past its time, is forgotten.
pub fn prune_kept(kept: &mut HashMap<InfoField, KeptWrite>, live: &StreamInfoDraft, now: i64) {
    kept.retain(|f, k| now - k.at_ms < KEPT_WRITE_MS && field_value(live, *f) != k.written);
}

/// Earlier in-app saves per channel, for the next save's merge.
static RECENT_WRITES: Lazy<Mutex<HashMap<String, HashMap<InfoField, KeptWrite>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// One save at a time per channel, so a second save merges onto the first.
static INFO_LOCKS: Lazy<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn info_lock(channel_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    locked(&INFO_LOCKS).entry(channel_id.to_string()).or_default().clone()
}

// ─────────────────────────────── network ─────────────────────────────────────

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

const SWITCHED: &str = "You switched Twitch accounts. Open it again to use this one.";
const WEB_OTHER_ACCOUNT: &str =
    "StreamNook's twitch.tv sign-in is another account. Sign in to Twitch again from your accounts to manage this channel.";

/// Whether the twitch.tv session belongs to the account the run is for.
pub fn web_binding(owner: Option<&str>, user_id: &str) -> Result<(), String> {
    match owner {
        Some(o) if o == user_id => Ok(()),
        Some(_) => Err(WEB_OTHER_ACCOUNT.into()),
        None => Err("Could not tell whose twitch.tv sign-in this is".into()),
    }
}

/// Who a Helix token belongs to, kept while the account and token stay.
static IDENTITY: Lazy<Mutex<Option<(u64, String, String, String)>>> = Lazy::new(|| Mutex::new(None));

/// Whose a twitch.tv session is, per session token (hashed; never logged).
static WEB_OWNERS: Lazy<Mutex<HashMap<u64, String>>> = Lazy::new(|| Mutex::new(HashMap::new()));

fn token_hash(t: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

/// One account's credentials for a whole read or action: every Helix call
/// carries this Helix token, every GQL call this twitch.tv session, and the
/// session is used only when it is the same account.
pub struct Session {
    helix_token: String,
    generation: u64,
    pub user_id: String,
    pub login: String,
    web: tokio::sync::OnceCell<Result<String, String>>,
}

impl Session {
    /// The signed-in account. `fresh` reads its identity from Twitch; else the
    /// last identity is reused while the account and token are the same.
    pub async fn open(fresh: bool) -> Result<Session, String> {
        let (token, generation) = TwitchService::get_token_with_generation()
            .await
            .map_err(|_| "Sign in to Twitch first".to_string())?;
        let cached = (!fresh)
            .then(|| locked(&IDENTITY).clone())
            .flatten()
            .filter(|(g, t, _, _)| *g == generation && *t == token);
        let (user_id, login) = match cached {
            Some((_, _, id, login)) => (id, login),
            None => {
                let me = TwitchService::get_user_info_with_token(&token)
                    .await
                    .map_err(|_| "Sign in to Twitch first".to_string())?;
                *locked(&IDENTITY) = Some((generation, token.clone(), me.id.clone(), me.login.clone()));
                (me.id, me.login)
            }
        };
        Ok(Session { helix_token: token, generation, user_id, login, web: tokio::sync::OnceCell::new() })
    }

    /// Still the account selected when this run began.
    pub fn still_current(&self) -> Result<(), String> {
        if TwitchService::account_generation() == self.generation {
            Ok(())
        } else {
            Err(SWITCHED.into())
        }
    }

    /// The twitch.tv session for GQL, read once, and only when it is this
    /// account's; else why not.
    async fn web(&self, auth: &TwitchAuthService) -> Result<String, String> {
        self.web
            .get_or_init(|| async {
                let token = twitch_web_gql::web_session_token(auth).await?;
                let key = token_hash(&token);
                let known = locked(&WEB_OWNERS).get(&key).cloned();
                let owner = match known {
                    Some(o) => Some(o),
                    None => {
                        let body = json!({ "operationName": "SnWhisperWho", "query": crate::services::whisper_gql::WHO_DOC, "variables": {} });
                        let answer = twitch_web_gql::post(Some(&token), body, LOG_TAG).await.ok();
                        let id = answer.and_then(|a| a["data"]["currentUser"]["id"].as_str().map(str::to_string));
                        if let Some(id) = &id {
                            locked(&WEB_OWNERS).insert(key, id.clone());
                        }
                        id
                    }
                };
                web_binding(owner.as_deref(), &self.user_id)?;
                Ok(token)
            })
            .await
            .clone()
    }

    /// Whether this run may use GQL at all.
    async fn has_web(&self, auth: &TwitchAuthService) -> bool {
        self.web(auth).await.is_ok()
    }

    /// One Helix call as this account: (status, body, Twitch's clock offset).
    async fn helix(&self, method: reqwest::Method, url: &str, body: Option<Value>) -> Result<(u16, String, i64), String> {
        let mut req = crate::services::http::client()
            .request(method, url)
            .header("Client-Id", helix_client_id())
            .header("Authorization", format!("Bearer {}", self.helix_token));
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.map_err(|e| format!("Could not reach Twitch: {e}"))?;
        let local = now_ms();
        let date = resp
            .headers()
            .get(reqwest::header::DATE)
            .and_then(|v| v.to_str().ok())
            .and_then(twitch_web_gql::http_date_ms);
        let status = resp.status().as_u16();
        Ok((status, resp.text().await.unwrap_or_default(), offset_from(date, local)))
    }

    async fn helix_get(&self, url: &str) -> Result<(Value, i64), String> {
        let (status, text, offset) = self.helix(reqwest::Method::GET, url, None).await?;
        if !(200..300).contains(&status) {
            return Err(helix_refusal_text(status, &text));
        }
        let v = serde_json::from_str(&text).map_err(|_| "Twitch sent an unreadable answer".to_string())?;
        Ok((v, offset))
    }

    /// One GQL call on this account's twitch.tv session: (answer, when it
    /// came on this device's clock, Twitch's clock offset).
    async fn gql(&self, auth: &TwitchAuthService, body: Value) -> Result<(Value, i64, i64), String> {
        let token = self.web(auth).await?;
        let send = |integrity: Option<crate::services::twitch_integrity::Integrity>| {
            let token = token.clone();
            let body = body.clone();
            async move {
                let (answer, date) = twitch_web_gql::post_status_dated(Some(&token), body, LOG_TAG, integrity.as_ref())
                    .await
                    .map_err(|(_, e)| e)?;
                let local = now_ms();
                if date.is_none() {
                    log::debug!("[{LOG_TAG}] a GQL answer had no Date header; its times are taken as local");
                }
                Ok::<_, String>((answer, local, offset_from(date, local)))
            }
        };
        let first = send(None).await?;
        // Some writes refuse a call without Twitch's security token: ask again
        // with one where the app can mint it.
        let integrity = first.0["errors"]
            .as_array()
            .map(|e| e.iter().any(|x| x["message"].as_str().unwrap_or("").to_ascii_lowercase().contains("integrity")))
            .unwrap_or(false);
        if integrity {
            // Every call here comes from the Stream manager the viewer has open.
            crate::services::twitch_integrity::user_asked();
            if let Ok(i) = crate::services::twitch_integrity::get().await {
                return send(Some(i)).await;
            }
        }
        Ok(first)
    }

    /// `gql`, the answer only.
    async fn gql_body(&self, auth: &TwitchAuthService, body: Value) -> Result<Value, String> {
        self.gql(auth, body).await.map(|(b, _, _)| b)
    }
}

// ─────────────────────────────── roles, kept ─────────────────────────────────

/// What GQL says the account is in a channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flags {
    pub editor: bool,
    pub moderator: bool,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    flags: Flags,
    read_rev: u64,
    at_ms: i64,
}

/// The last role flags read per (account, channel), so a failed read keeps an
/// editor an editor. A read that started earlier never replaces a later one.
#[derive(Default)]
pub struct RoleCache {
    entries: HashMap<(String, String), CacheEntry>,
}

const ROLE_FRESH_MS: i64 = 10 * 60_000;
const ROLE_KEEP_MS: i64 = 24 * 60 * 60_000;

impl RoleCache {
    pub fn put(&mut self, key: (String, String), flags: Flags, read_rev: u64, now: i64) {
        self.entries.retain(|_, e| now - e.at_ms < ROLE_KEEP_MS);
        if self.entries.get(&key).is_some_and(|e| e.read_rev > read_rev) {
            return;
        }
        self.entries.insert(key, CacheEntry { flags, read_rev, at_ms: now });
    }
    pub fn fresh(&self, key: &(String, String), now: i64) -> Option<Flags> {
        self.entries.get(key).filter(|e| now - e.at_ms < ROLE_FRESH_MS).map(|e| e.flags)
    }
    pub fn last(&self, key: &(String, String)) -> Option<Flags> {
        self.entries.get(key).map(|e| e.flags)
    }
}

static ROLES: Lazy<Mutex<RoleCache>> = Lazy::new(|| Mutex::new(RoleCache::default()));

/// When a view last found no twitch.tv session for (account, generation).
static NO_WEB: Lazy<Mutex<HashMap<(String, u64), i64>>> = Lazy::new(|| Mutex::new(HashMap::new()));
const NO_WEB_MS: i64 = 5 * 60_000;

/// The role from what is known. `read_now` is this run's GQL read, `known`
/// what chat's USERSTATE said, `cached` the last flags read before. Chat
/// never says editor, so editor comes only from GQL.
pub fn role_from(
    own: bool,
    known: Option<crate::services::chat_send_limiter::Role>,
    read_now: Option<Flags>,
    cached: Option<Flags>,
) -> Result<Role, String> {
    use crate::services::chat_send_limiter::Role as Chat;
    if own {
        return Ok(Role::Broadcaster);
    }
    if read_now.is_none() && known.is_none() && cached.is_none() {
        return Err("Could not read your role in this channel".into());
    }
    let moderator = match (read_now, known, cached) {
        (Some(f), _, _) => f.moderator,
        (None, Some(k), _) => matches!(k, Chat::Moderator | Chat::Broadcaster),
        (None, None, Some(f)) => f.moderator,
        (None, None, None) => false,
    };
    let editor = read_now.or(cached).map(|f| f.editor).unwrap_or(false);
    Ok(Role::from_flags(false, editor, moderator))
}

/// The account's role in a channel. `fresh` reads GQL even when the kept
/// flags are recent (an action); a view uses recent flags.
async fn role_in(session: &Session, auth: &TwitchAuthService, channel_id: &str, channel_login: &str, fresh: bool) -> Result<Role, String> {
    if channel_id == session.user_id {
        return Ok(Role::Broadcaster);
    }
    let known = crate::services::chat_send_limiter::known_role(&session.login, channel_login);
    let key = (session.user_id.clone(), channel_id.to_string());
    let now = now_ms();
    if !fresh {
        // Recent flags are kept flags, never a read just now: chat's live
        // USERSTATE still decides moderator over them.
        let recent = locked(&ROLES).fresh(&key, now);
        if let Some(f) = recent {
            return role_from(false, known, None, Some(f));
        }
    }
    // A view does not look for a twitch.tv session again soon after there was
    // none (on desktop that look builds a hidden webview); an action always looks.
    let no_web_key = (session.user_id.clone(), session.generation);
    let skip_web = !fresh && locked(&NO_WEB).get(&no_web_key).is_some_and(|at| now - at < NO_WEB_MS);
    if skip_web || !session.has_web(auth).await {
        if !skip_web {
            let mut no_web = locked(&NO_WEB);
            no_web.retain(|_, at| now - *at < NO_WEB_MS);
            no_web.insert(no_web_key, now);
        }
        // No GQL without this account's twitch.tv session: chat decides, and
        // every tool past the broadcaster's needs GQL anyway.
        let cached = locked(&ROLES).last(&key);
        return Ok(role_from(false, known, None, cached).unwrap_or(Role::Viewer));
    }
    locked(&NO_WEB).remove(&no_web_key);
    let read_rev = next_rev();
    let read_now = session
        .gql_body(auth, op("StreamManagerRole", ROLE_QUERY, json!({ "channelID": channel_id })))
        .await
        .ok()
        .map(|b| b["data"]["user"]["self"].clone())
        .filter(|s| s.is_object())
        .map(|s| Flags {
            editor: s["isEditor"].as_bool().unwrap_or(false),
            moderator: s["isModerator"].as_bool().unwrap_or(false),
        });
    if let Some(f) = read_now {
        locked(&ROLES).put(key.clone(), f, read_rev, now_ms());
    }
    let cached = locked(&ROLES).last(&key);
    role_from(false, known, read_now, cached)
}

async fn read_info(session: &Session, channel_id: &str) -> Result<StreamInfoView, String> {
    let (v, _) = session.helix_get(&format!("https://api.twitch.tv/helix/channels?broadcaster_id={channel_id}")).await?;
    let row = v["data"][0].clone();
    if row.is_null() {
        return Err("Twitch has no channel with that id".into());
    }
    let box_art = match row["game_id"].as_str().filter(|g| !g.is_empty()) {
        Some(g) => TwitchService::get_games_by_ids(&[g.to_string()])
            .await
            .ok()
            .and_then(|games| games.first().and_then(|x| x["box_art_url"].as_str().map(str::to_string))),
        None => None,
    };
    Ok(info_from_helix(&row, box_art))
}

/// The running poll and prediction, each part on its own: a part that could
/// not be read says why.
async fn read_running(session: &Session, auth: &TwitchAuthService, channel_id: &str, role: Role) -> RunningRead {
    if helix_route(role) {
        let poll = session
            .helix_get(&format!("https://api.twitch.tv/helix/polls?broadcaster_id={channel_id}&first=1"))
            .await
            .map(|(v, offset)| v["data"].get(0).and_then(|p| running_poll_helix(p, offset)));
        let prediction = session
            .helix_get(&format!("https://api.twitch.tv/helix/predictions?broadcaster_id={channel_id}&first=1"))
            .await
            .map(|(v, offset)| v["data"].get(0).and_then(|p| running_prediction_helix(p, offset)));
        return RunningRead { poll, prediction };
    }
    match session.gql(auth, op("StreamManagerRunning", RUNNING_QUERY, json!({ "channelID": channel_id }))).await {
        Ok((body, at, offset)) => running_from_gql(&body, at, offset),
        Err(why) => RunningRead::failed(&why),
    }
}

async fn read_ads(session: &Session, channel_id: &str) -> AdsView {
    let granted = TwitchService::token_scopes(&session.helix_token).await.unwrap_or_default();
    if lacks_scope(&granted, "channel:read:ads") {
        return AdsView { lengths: ad_lengths(), schedule: None, schedule_hint: Some(ADS_SCOPE_HINT.into()) };
    }
    let schedule = session
        .helix_get(&format!("https://api.twitch.tv/helix/channels/ads?broadcaster_id={channel_id}"))
        .await
        .ok()
        .map(|(v, offset)| ad_schedule(&v["data"][0], now_ms(), offset));
    AdsView { lengths: ad_lengths(), schedule, schedule_hint: None }
}

async fn read_chat(session: &Session, channel_id: &str) -> Result<ChatView, String> {
    let own_id = &session.user_id;
    let (settings, _) = session
        .helix_get(&format!(
            "https://api.twitch.tv/helix/chat/settings?broadcaster_id={channel_id}&moderator_id={own_id}"
        ))
        .await?;
    let shield = session
        .helix_get(&format!(
            "https://api.twitch.tv/helix/moderation/shield_mode?broadcaster_id={channel_id}&moderator_id={own_id}"
        ))
        .await
        .ok()
        .and_then(|(v, _)| v["data"][0]["is_active"].as_bool());
    Ok(chat_view(&settings["data"][0], shield))
}

async fn read_queue_as(session: &Session, auth: &TwitchAuthService, rev: u64, channel_id: &str) -> Result<QueueView, String> {
    let body = session.gql_body(auth, op("StreamManagerQueue", QUEUE_QUERY, json!({ "channelID": channel_id }))).await?;
    if let Some(e) = body["errors"].as_array().filter(|e| !e.is_empty()) {
        return Err(format!(
            "Could not read the rewards queue: {}",
            gql_error_words(e[0]["message"].as_str().unwrap_or("error"))
        ));
    }
    let mut q = parse_queue(&body, now_ms());
    q.rev = rev;
    q.user_id = session.user_id.clone();
    Ok(q)
}

/// The rewards queue as the signed-in account.
pub async fn read_queue(auth: &TwitchAuthService, channel_id: &str) -> Result<QueueView, String> {
    let rev = next_rev();
    let session = Session::open(false).await?;
    read_queue_as(&session, auth, rev, channel_id).await
}

/// Everything the manager shows for `channel_id`. Parts that fail say so in
/// `problems`; the rest still draws. An `Err` is a view that could not be
/// built at all (signed out, or the role could not be read).
pub async fn view(auth: &TwitchAuthService, channel_id: &str, channel_login: &str) -> Result<StreamManagerView, String> {
    let rev = next_rev();
    let session = Session::open(false).await?;
    let (view, _) = view_as(&session, auth, rev, channel_id, channel_login).await?;
    Ok(view)
}

/// `view`, plus the rewards queue it read on the way.
pub async fn view_and_queue(auth: &TwitchAuthService, channel_id: &str, channel_login: &str) -> Result<(StreamManagerView, Option<QueueView>), String> {
    let rev = next_rev();
    let session = Session::open(false).await?;
    view_as(&session, auth, rev, channel_id, channel_login).await
}

async fn view_as(
    session: &Session,
    auth: &TwitchAuthService,
    rev: u64,
    channel_id: &str,
    channel_login: &str,
) -> Result<(StreamManagerView, Option<QueueView>), String> {
    let role = role_in(session, auth, channel_id, channel_login, false).await?;
    // A viewer's read touches no file.
    let recents = if role.manages() { poll_recents() } else { PollRecents::default() };
    let mut problems = Vec::new();
    let empty = || StreamManagerView {
        rev,
        user_id: session.user_id.clone(),
        channel_id: channel_id.to_string(),
        channel_login: channel_login.to_string(),
        role,
        role_line: role_line(role),
        sections: Vec::new(),
        pages: Vec::new(),
        info: None,
        poll: None,
        prediction: None,
        poll_known: true,
        prediction_known: true,
        raid: None,
        ads: None,
        chat: None,
        queue_count: None,
        limits: form_limits(),
        recent_polls: recents.polls.clone(),
        recent_predictions: recents.predictions.clone(),
        problems: Vec::new(),
    };
    if !role.manages() {
        let view = empty();
        // Nothing to go out of date: no deadline reads for it.
        arm(&view);
        return Ok((view, None));
    }
    let wants = |s: Section| can_use(role, s).is_ok();
    let (info, running, ads, chat, queue) = tokio::join!(
        async { if wants(Section::Info) { Some(read_info(session, channel_id).await) } else { None } },
        async {
            if wants(Section::Poll) { Some(read_running(session, auth, channel_id, role).await) } else { None }
        },
        async { if wants(Section::Ads) { Some(read_ads(session, channel_id).await) } else { None } },
        async { if wants(Section::Chat) { Some(read_chat(session, channel_id).await) } else { None } },
        async { if wants(Section::Rewards) { Some(read_queue_as(session, auth, rev, channel_id).await) } else { None } },
    );
    let info = match info {
        Some(Ok(mut i)) => {
            prune_kept(locked(&RECENT_WRITES).entry(channel_id.to_string()).or_default(), &draft_from_info(&i), now_ms());
            with_recents(&mut i, read_recents(channel_id).as_ref());
            Some(i)
        }
        Some(Err(e)) => {
            problems.push(format!("Stream info: {e}"));
            None
        }
        None => None,
    };
    let chat = match chat {
        Some(Ok(c)) => Some(c),
        Some(Err(e)) => {
            problems.push(format!("Chat modes: {e}"));
            None
        }
        None => None,
    };
    let (queue_count, queue) = match queue {
        Some(Ok(q)) => (Some(q.count), Some(q)),
        Some(Err(e)) => {
            problems.push(e);
            (None, None)
        }
        None => (None, None),
    };
    let now = now_ms();
    let raid = if wants(Section::Raid) { pending_raid(channel_id, now) } else { None };
    let running = match running {
        Some(read) => {
            for why in [read.poll.as_ref().err(), read.prediction.as_ref().err()].into_iter().flatten().take(1) {
                problems.push(format!("Polls and predictions: {why}"));
            }
            settle_and_keep(&session.user_id, channel_id, &read)
        }
        None => Running { poll_known: true, prediction_known: true, ..Running::default() },
    };
    let Running { poll, prediction, poll_known, prediction_known } = running;
    let sections = section_rows(role, info.as_ref(), poll.as_ref(), prediction.as_ref(), raid.as_ref(), ads.as_ref(), queue_count, now);
    let pages = page_rows(role, poll.as_ref(), prediction.as_ref(), raid.as_ref(), queue_count);
    let view = StreamManagerView {
        sections,
        pages,
        info,
        poll,
        prediction,
        poll_known,
        prediction_known,
        raid,
        ads,
        chat,
        queue_count,
        problems,
        ..empty()
    };
    arm(&view);
    Ok((view, queue))
}

/// The info form's check, with the channel's name, picture and live
/// thumbnail filled in for the preview.
pub async fn check(channel_id: &str, draft: &StreamInfoDraft) -> InfoCheck {
    let Ok(session) = Session::open(false).await else {
        return check_stream_info(draft, None, "", None, None);
    };
    let current = read_info(&session, channel_id).await.ok();
    let users = session.helix_get(&format!("https://api.twitch.tv/helix/users?id={channel_id}")).await.ok();
    let user = users.as_ref().map(|(u, _)| u["data"][0].clone()).unwrap_or(Value::Null);
    let name = user["display_name"].as_str().unwrap_or_default().to_string();
    let avatar = user["profile_image_url"].as_str().map(str::to_string);
    let stream = session
        .helix_get(&format!("https://api.twitch.tv/helix/streams?user_id={channel_id}"))
        .await
        .ok()
        .map(|(s, _)| s["data"][0].clone())
        .unwrap_or(Value::Null);
    let thumb = stream["thumbnail_url"].as_str().map(|t| box_art(t, 440, 248));
    let mut out = check_stream_info(draft, current.as_ref(), &name, avatar, thumb);
    live_facts(&mut out.preview, &stream, &user);
    out
}

/// The live card's chips and mark for the preview, from Helix's stream row
/// (empty while offline) and user row.
pub fn live_facts(preview: &mut PreviewTile, stream: &Value, user: &Value) {
    if preview.thumbnail_url.is_some() {
        preview.viewer_count = stream["viewer_count"].as_u64();
        preview.started_at = stream["started_at"].as_str().filter(|s| !s.is_empty()).map(str::to_string);
    }
    preview.partner = user["broadcaster_type"].as_str() == Some("partner");
}

/// Whether the signed-in account may open `section` here, or why not (typed
/// `/poll` and `/prediction`).
pub async fn can_open(auth: &TwitchAuthService, channel_id: &str, channel_login: &str, section: Section) -> Result<(), String> {
    let session = Session::open(false).await?;
    let role = role_in(&session, auth, channel_id, channel_login, false).await?;
    can_use(role, section).map_err(str::to_string)
}

pub async fn search_raid_targets(state: &crate::models::settings::AppState, query: &str, channel_id: &str) -> Result<Vec<RaidTarget>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let streams = TwitchService::search_channels(state, q).await.map_err(|e| e.to_string())?;
    Ok(raid_targets(&streams, channel_id).into_iter().take(12).collect())
}

/// Run one action as the signed-in account, then read the view again. With
/// `expected_user_id`, refuses when another account is signed in now.
pub async fn act(auth: &TwitchAuthService, channel_id: &str, channel_login: &str, action: Action, expected_user_id: Option<&str>) -> ActOutcome {
    let session = match Session::open(true).await {
        Ok(s) => s,
        Err(e) => return ActOutcome::refused(e, None),
    };
    let me = Some(session.user_id.clone());
    if expected_user_id.is_some_and(|want| want != session.user_id) {
        return ActOutcome::refused(SWITCHED, me);
    }
    let role = match role_in(&session, auth, channel_id, channel_login, true).await {
        Ok(r) => r,
        Err(e) => return ActOutcome::refused(e, me),
    };
    if let Err(why) = can_use(role, action.section()) {
        return ActOutcome::refused(why, me);
    }
    let queue_write = action.section() == Section::Rewards;
    let saved_info = match &action {
        Action::SaveInfo { draft, fields } => Some((draft.clone(), fields.clone())),
        _ => None,
    };
    let started = match &action {
        Action::CreatePoll { title, choices, .. } => Some((PollKind::Poll, title.clone(), choices.clone())),
        Action::CreatePrediction { title, outcomes, .. } => Some((PollKind::Prediction, title.clone(), outcomes.clone())),
        _ => None,
    };
    let result = run(&session, auth, channel_id, role, action).await;
    if result.is_ok() {
        if let Some((draft, fields)) = &saved_info {
            remember_save(channel_id, draft, fields);
        }
        if let Some((kind, title, options)) = &started {
            let options: Vec<String> = options.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
            remember_poll(*kind, title, &options);
        }
    }
    let rev = next_rev();
    let (view, read_queue) = match view_as(&session, auth, rev, channel_id, channel_login).await {
        Ok((v, q)) => (Some(v), q),
        Err(_) => (None, None),
    };
    let queue = if queue_write { read_queue } else { None };
    match result {
        Ok(message) => ActOutcome { ok: true, message, view, queue, user_id: me },
        Err(message) => ActOutcome { ok: false, message, view, queue, user_id: me },
    }
}

/// The running poll, or why it can't be ended.
async fn open_poll(session: &Session, auth: &TwitchAuthService, channel_id: &str, role: Role) -> Result<RunningPoll, String> {
    match read_running(session, auth, channel_id, role).await.poll {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err("No poll is running right now".into()),
        Err(why) => Err(format!("Could not read the running poll: {why}")),
    }
}

/// The running prediction, or why it can't be ended.
async fn open_prediction(session: &Session, auth: &TwitchAuthService, channel_id: &str, role: Role) -> Result<RunningPrediction, String> {
    match read_running(session, auth, channel_id, role).await.prediction {
        Ok(Some(p)) => Ok(p),
        Ok(None) => Err("No prediction is running right now".into()),
        Err(why) => Err(format!("Could not read the running prediction: {why}")),
    }
}

async fn save_info(
    session: &Session,
    auth: &TwitchAuthService,
    channel_id: &str,
    helix_ok: bool,
    draft: StreamInfoDraft,
    fields: Vec<InfoField>,
) -> Result<String, String> {
    let tags = check_info_fields(&draft, &fields)?;
    if helix_ok {
        let body = info_patch(&draft, &fields, &tags)?;
        session.still_current()?;
        let (status, text, _) = session
            .helix(
                reqwest::Method::PATCH,
                &format!("https://api.twitch.tv/helix/channels?broadcaster_id={channel_id}"),
                Some(body),
            )
            .await?;
        if !(200..300).contains(&status) {
            return Err(helix_refusal_text(status, &text));
        }
        return Ok("Stream info saved".into());
    }
    // GQL writes the broadcast settings whole: read what is there now and
    // change only the named fields, one save at a time per channel.
    let lock = info_lock(channel_id);
    let _held = lock.lock().await;
    let live = read_info(session, channel_id)
        .await
        .map_err(|e| format!("Could not read the stream info to save it: {e}"))?;
    let live = draft_from_info(&live);
    let now = now_ms();
    let kept = {
        let mut writes = locked(&RECENT_WRITES);
        let entry = writes.entry(channel_id.to_string()).or_default();
        prune_kept(entry, &live, now);
        entry.clone()
    };
    let send = merged_info(&draft, &fields, &live, &kept, now);
    let (broadcast, with_tags) = gql_save_plan(&fields);
    let broadcast_fields: Vec<InfoField> = fields.iter().copied().filter(|f| *f != InfoField::Tags).collect();
    // What went through is remembered before the lock goes, so the next save
    // here merges onto it, even when a later part of this save fails.
    let keep = |which: &[InfoField]| {
        let mut writes = locked(&RECENT_WRITES);
        let entry = writes.entry(channel_id.to_string()).or_default();
        for f in which {
            entry.insert(*f, KeptWrite { written: field_value(&draft, *f), before: field_value(&live, *f), draft: draft.clone(), at_ms: now_ms() });
        }
    };
    if broadcast {
        session.still_current()?;
        gql_outcome(
            &session.gql_body(auth, broadcast_request(channel_id, &send)).await?,
            "updateBroadcastSettings",
            "Could not save the stream info",
        )?;
        keep(&broadcast_fields);
    }
    let tags_failed = if broadcast { "Saved, but the tags did not go through" } else { "Could not save the tags" };
    if with_tags {
        let tags_result = async {
            session.still_current()?;
            gql_outcome(&session.gql_body(auth, tags_request(channel_id, &tags)).await?, "setFreeformTags", tags_failed)
        }
        .await;
        if let Err(e) = tags_result {
            if broadcast {
                // The title, category and language did go through: their
                // recents move as for a save (the lock goes first).
                drop(_held);
                remember_save(channel_id, &draft, &broadcast_fields);
            }
            return Err(e);
        }
        keep(&[InfoField::Tags]);
    }
    Ok("Stream info saved".into())
}

async fn run(session: &Session, auth: &TwitchAuthService, channel_id: &str, role: Role, action: Action) -> Result<String, String> {
    let helix_ok = helix_route(role);
    let words = |e: anyhow::Error| e.to_string();
    let token = session.helix_token.as_str();
    match action {
        Action::SaveInfo { draft, fields } => save_info(session, auth, channel_id, helix_ok, draft, fields).await,
        Action::CreatePoll { title, choices, seconds, points_per_vote } => {
            let choices = check_poll(&title, &choices, seconds)?;
            session.still_current()?;
            if helix_ok {
                TwitchService::create_poll_as(token, channel_id, title.trim(), &choices, seconds, points_per_vote).await.map_err(words)?;
            } else {
                gql_outcome(&session.gql_body(auth, create_poll_request(channel_id, &title, &choices, seconds, points_per_vote)).await?, "createPoll", "Could not start the poll")?;
            }
            Ok("Poll started".into())
        }
        Action::EndPoll { show_results } => {
            let poll = open_poll(session, auth, channel_id, role).await?;
            session.still_current()?;
            if helix_ok {
                let status = if show_results { "TERMINATED" } else { "ARCHIVED" };
                TwitchService::end_poll_as(token, channel_id, &poll.id, status).await.map_err(words)?;
            } else {
                let (name, query, field) = if show_results {
                    ("StreamManagerEndPoll", TERMINATE_POLL_MUTATION, "terminatePoll")
                } else {
                    ("StreamManagerArchivePoll", ARCHIVE_POLL_MUTATION, "archivePoll")
                };
                gql_outcome(&session.gql_body(auth, op(name, query, json!({ "input": { "pollID": poll.id } }))).await?, field, "Could not end the poll")?;
            }
            Ok(if show_results { "Poll ended" } else { "Poll removed" }.into())
        }
        Action::CreatePrediction { title, outcomes, seconds } => {
            let outcomes = check_prediction(&title, &outcomes, seconds)?;
            session.still_current()?;
            if helix_ok {
                TwitchService::create_prediction_as(token, channel_id, title.trim(), &outcomes, seconds).await.map_err(words)?;
            } else {
                gql_outcome(&session.gql_body(auth, create_prediction_request(channel_id, &title, &outcomes, seconds)).await?, "createPredictionEvent", "Could not start the prediction")?;
            }
            Ok("Prediction started".into())
        }
        Action::LockPrediction | Action::CancelPrediction | Action::ResolvePrediction { .. } | Action::ResolvePredictionPick { .. } => {
            let open = open_prediction(session, auth, channel_id, role).await?;
            let winner = match &action {
                Action::ResolvePrediction { outcome_id } => Some(outcome_id.clone()),
                Action::ResolvePredictionPick { pick } => Some(pick_outcome(&open.outcomes, pick)?),
                _ => None,
            };
            if matches!(action, Action::LockPrediction) && open.status == "locked" {
                return Err("The prediction is already locked".into());
            }
            session.still_current()?;
            let (status, done) = match &action {
                Action::LockPrediction => ("LOCKED", "Prediction locked"),
                Action::CancelPrediction => ("CANCELED", "Prediction canceled, points refunded"),
                _ => ("RESOLVED", "Prediction paid out"),
            };
            if helix_ok {
                TwitchService::end_prediction_as(token, channel_id, &open.id, status, winner.as_deref()).await.map_err(words)?;
                return Ok(done.into());
            }
            let (name, query, field, input) = match status {
                "LOCKED" => ("StreamManagerLockPrediction", LOCK_PREDICTION_MUTATION, "lockPredictionEvent", json!({ "id": open.id })),
                "CANCELED" => ("StreamManagerCancelPrediction", CANCEL_PREDICTION_MUTATION, "cancelPredictionEvent", json!({ "id": open.id })),
                _ => ("StreamManagerResolvePrediction", RESOLVE_PREDICTION_MUTATION, "resolvePredictionEvent", json!({ "eventID": open.id, "outcomeID": winner })),
            };
            gql_outcome(&session.gql_body(auth, op(name, query, json!({ "input": input }))).await?, field, "That did not go through")?;
            Ok(done.into())
        }
        Action::Raid { target_id, target_login, target_name } => {
            session.still_current()?;
            TwitchService::start_raid_as(token, channel_id, &target_id).await.map_err(words)?;
            set_raid(
                channel_id,
                Some(RaidView {
                    target_id,
                    target_login,
                    target_name: target_name.clone(),
                    deadline_ms: now_ms() + RAID_COUNTDOWN_MS,
                    total_ms: RAID_COUNTDOWN_MS,
                }),
            );
            Ok(format!("Raiding {target_name}"))
        }
        Action::CancelRaid => {
            session.still_current()?;
            TwitchService::cancel_raid_as(token, channel_id).await.map_err(words)?;
            set_raid(channel_id, None);
            Ok("Raid canceled".into())
        }
        Action::RaidNow => {
            session.still_current()?;
            let body = session.gql_body(auth, crate::services::slash_commands::go_raid_request(channel_id)).await?;
            crate::services::slash_commands::go_raid_outcome(&body)?;
            set_raid(channel_id, None);
            Ok("Raid sent".into())
        }
        Action::RunAd { seconds } => {
            if !AD_LENGTHS.contains(&seconds) {
                return Err("Pick a length from the list".into());
            }
            session.still_current()?;
            TwitchService::start_commercial_as(token, channel_id, seconds).await.map_err(words)?;
            Ok(format!("Ad break started ({})", ad_lengths().into_iter().find(|c| c.value == seconds).map(|c| c.label).unwrap_or_default()))
        }
        Action::SnoozeAd => {
            let granted = TwitchService::token_scopes(token).await.map_err(words)?;
            if lacks_scope(&granted, "channel:manage:ads") {
                return Err(ADS_SCOPE_HINT.into());
            }
            session.still_current()?;
            let (status, text, _) = session
                .helix(
                    reqwest::Method::POST,
                    &format!("https://api.twitch.tv/helix/channels/ads/schedule/snooze?broadcaster_id={channel_id}"),
                    None,
                )
                .await?;
            if !(200..300).contains(&status) {
                return Err(helix_refusal_text(status, &text));
            }
            Ok("Next ad snoozed".into())
        }
        Action::Redemptions { ids, fulfill } => {
            if ids.is_empty() {
                return Err("Nothing picked".into());
            }
            session.still_current()?;
            gql_outcome(
                &session.gql_body(auth, redemptions_request(channel_id, &ids, fulfill)).await?,
                "updateCommunityPointsCustomRewardRedemptionStatusesByRedemptions",
                if fulfill { "Could not complete it" } else { "Could not reject it" },
            )?;
            Ok(match (fulfill, ids.len()) {
                (true, 1) => "Completed".into(),
                (true, n) => format!("Completed {n}"),
                (false, 1) => "Rejected, points refunded".into(),
                (false, n) => format!("Rejected {n}, points refunded"),
            })
        }
        Action::RewardQueue { reward_id, fulfill } => {
            session.still_current()?;
            gql_outcome(
                &session.gql_body(auth, reward_queue_request(channel_id, &reward_id, fulfill)).await?,
                "updateCommunityPointsCustomRewardRedemptionStatusesByReward",
                "That did not go through",
            )?;
            Ok(if fulfill { "Completed all" } else { "Rejected all, points refunded" }.into())
        }
        Action::Reward { reward_id, paused, enabled } => {
            if paused.is_none() && enabled.is_none() {
                return Err("Nothing to change".into());
            }
            session.still_current()?;
            gql_outcome(
                &session.gql_body(auth, reward_request(channel_id, &reward_id, paused, enabled)).await?,
                "updateCommunityPointsCustomReward",
                "Could not change the reward",
            )?;
            Ok(match (paused, enabled) {
                (Some(true), _) => "Reward paused",
                (Some(false), _) => "Reward resumed",
                (_, Some(true)) => "Reward turned on",
                _ => "Reward turned off",
            }
            .into())
        }
        Action::ChatMode { mode, on, value } => {
            let body = chat_mode_body(&mode, on, value)?;
            session.still_current()?;
            TwitchService::update_chat_settings_as(token, &session.user_id, channel_id, body).await.map_err(words)?;
            Ok("Chat updated".into())
        }
        Action::Shield { on } => {
            session.still_current()?;
            TwitchService::update_shield_mode_as(token, &session.user_id, channel_id, on).await.map_err(words)?;
            Ok(if on { "Shield Mode on" } else { "Shield Mode off" }.into())
        }
        Action::Announce { text, color } => {
            let text = text.trim();
            if text.is_empty() {
                return Err("Write the announcement first".into());
            }
            if text.chars().count() > 500 {
                return Err("Announcements can be at most 500 characters".into());
            }
            session.still_current()?;
            TwitchService::send_chat_announcement_as(token, &session.user_id, channel_id, text, color.as_deref()).await.map_err(words)?;
            Ok("Announced".into())
        }
    }
}

// ─────────────────────────────── staying current ─────────────────────────────

/// The times a view will be out of date by itself, and when to read again
/// for them: a future one 1.5 s after it; one that is overdue (Twitch may not
/// have moved yet) again in 2 s while under 10 s late, 5 s under 30 s, 15 s
/// under 2 min, then no more. The earliest wins.
pub fn reread_at(view: &StreamManagerView, now: i64) -> Option<i64> {
    let mut times: Vec<i64> = Vec::new();
    times.extend(view.poll.as_ref().and_then(|p| p.ends_at_ms));
    times.extend(view.prediction.as_ref().filter(|p| p.status == "active").and_then(|p| p.locks_at_ms));
    times.extend(view.raid.as_ref().map(|r| r.deadline_ms));
    if let Some(s) = view.ads.as_ref().and_then(|a| a.schedule.as_ref()) {
        times.extend(s.next_ad_ms);
        times.extend(s.snooze_refresh_ms);
    }
    times
        .into_iter()
        .filter_map(|t| {
            if t > now {
                return Some(t + 1500);
            }
            match now - t {
                late if late < 10_000 => Some(now + 2000),
                late if late < 30_000 => Some(now + 5000),
                late if late < 120_000 => Some(now + 15_000),
                _ => None,
            }
        })
        .min()
}

/// Every channel read again at least this often while a surface is open.
const PERIODIC_MS: i64 = 30_000;
/// At most one background read per channel this often.
const PUBLISH_GAP_MS: i64 = 3000;

/// Which window surfaces show which channel: (window label, surface id) to
/// the last call's sequence number and, while on, (channel id, login).
pub type InterestMap = HashMap<(String, String), (u64, Option<(String, String)>)>;

/// Drops the interest of windows that went away without saying so. A shell
/// with no webview windows at all (the iPhone's headless core, whose one
/// screen is native) lists none, and there nothing is dropped: its sheet's
/// interest would otherwise go on the first tick and nothing would read again.
pub fn keep_open_windows(interest: &mut InterestMap, open: &[String]) {
    if open.is_empty() {
        return;
    }
    interest.retain(|(label, _), _| open.iter().any(|l| l == label));
}

/// The most surfaces one window may hold on at once.
const SURFACES_PER_WINDOW: usize = 16;

/// The interest after one `stream_manager_watch` call. A call older than the
/// last one seen for its surface is ignored; a window past its cap is refused.
pub fn interest_after(
    map: &mut InterestMap,
    label: &str,
    surface: &str,
    channel: Option<(String, String)>,
    seq: u64,
) -> Result<(), &'static str> {
    let key = (label.to_string(), surface.to_string());
    if map.get(&key).is_some_and(|(last, _)| *last > seq) {
        return Ok(());
    }
    if channel.is_some() && map.get(&key).and_then(|(_, c)| c.as_ref()).is_none() {
        let on = map.iter().filter(|((l, _), (_, c))| l == label && c.is_some()).count();
        if on >= SURFACES_PER_WINDOW {
            return Err("too many open manager surfaces in one window");
        }
    }
    map.insert(key, (seq, channel));
    // Keep at most a few closed surfaces per window, for their sequence.
    let mut closed: Vec<(String, u64)> = map
        .iter()
        .filter(|((l, _), (_, c))| l == label && c.is_none())
        .map(|((_, s), (q, _))| (s.clone(), *q))
        .collect();
    if closed.len() > SURFACES_PER_WINDOW * 4 {
        closed.sort_by_key(|(_, q)| *q);
        for (s, _) in closed.iter().take(closed.len() - SURFACES_PER_WINDOW * 4) {
            map.remove(&(label.to_string(), s.clone()));
        }
    }
    Ok(())
}

/// The channels some surface shows: channel id to login.
pub fn watched(map: &InterestMap) -> HashMap<String, String> {
    map.values().filter_map(|(_, c)| c.clone()).collect()
}

/// Per watched channel: when it was last read in the background, when its
/// view goes out of date, which view set that, and whether Twitch said
/// something changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Freshness {
    pub last_ms: i64,
    pub reread: Option<i64>,
    pub armed_rev: u64,
    pub asked: bool,
    pub running: bool,
}

/// When a channel should be read next: right away (but not within 3 s of the
/// last) when Twitch said something changed, else at its out-of-date time or
/// 30 s after the last read, whichever comes first.
pub fn next_publish_at(f: &Freshness) -> i64 {
    let gap = f.last_ms + PUBLISH_GAP_MS;
    if f.asked {
        return gap;
    }
    let periodic = f.last_ms + PERIODIC_MS;
    f.reread.map_or(periodic, |r| r.min(periodic)).max(gap)
}

/// A view only moves a channel's out-of-date time when no later read did.
pub fn should_arm(armed_rev: u64, rev: u64) -> bool {
    rev >= armed_rev
}

static INTEREST: Lazy<Mutex<InterestMap>> = Lazy::new(|| Mutex::new(HashMap::new()));
static FRESH: Lazy<Mutex<HashMap<String, Freshness>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static TICKING: AtomicBool = AtomicBool::new(false);

/// Note a view's out-of-date time for its channel.
fn arm(view: &StreamManagerView) {
    let mut fresh = locked(&FRESH);
    let f = fresh.entry(view.channel_id.clone()).or_default();
    if !should_arm(f.armed_rev, view.rev) {
        return;
    }
    f.armed_rev = view.rev;
    f.reread = reread_at(view, now_ms());
    f.last_ms = f.last_ms.max(now_ms());
}

/// A window says a manager surface shows `channel` (or no longer does).
pub fn watch(label: &str, surface: &str, channel: Option<(String, String)>, seq: u64) {
    let on = channel.is_some();
    let channel_id = channel.as_ref().map(|(id, _)| id.clone());
    if let Err(why) = interest_after(&mut locked(&INTEREST), label, surface, channel, seq) {
        log::warn!("[{LOG_TAG}] {why} ({label})");
        return;
    }
    if on {
        if let Some(id) = channel_id {
            // A surface that opens reads at once (its own read), so the
            // background starts counting from now.
            locked(&FRESH).entry(id).or_default().last_ms = now_ms();
        }
        start_ticking();
    }
}

/// A window's page loaded again, or the window closed: its surfaces are gone.
pub fn clear_label(label: &str) {
    locked(&INTEREST).retain(|(l, _), _| l != label);
}

/// A poll, prediction or reward changed state on Twitch (pubsub).
pub fn pubsub_changes_manager(topic: &str, message: &Value) -> bool {
    let kind = message["type"].as_str().unwrap_or("");
    match topic {
        "polls" => matches!(kind, "POLL_CREATE" | "POLL_COMPLETE" | "POLL_TERMINATE" | "POLL_ARCHIVE"),
        "predictions-channel-v1" => {
            kind == "event-created"
                || (kind == "event-updated"
                    && matches!(message["data"]["event"]["status"].as_str(), Some("LOCKED" | "RESOLVED" | "CANCELED")))
        }
        "community-points-channel-v1" => matches!(
            kind,
            "custom-reward-created" | "custom-reward-updated" | "custom-reward-deleted" | "reward-redeemed" | "redemption-status-update"
        ),
        _ => false,
    }
}

/// Pubsub's hook: a watched channel whose poll, prediction or rewards changed
/// reads again soon.
pub fn pubsub_event(topic: &str, channel_id: &str, message: &Value) {
    if !pubsub_changes_manager(topic, message) {
        return;
    }
    if !watched(&locked(&INTEREST)).contains_key(channel_id) {
        return;
    }
    locked(&FRESH).entry(channel_id.to_string()).or_default().asked = true;
}

/// Clears a channel's running flag however its read ends.
struct RunningGuard(String);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        if let Some(f) = locked(&FRESH).get_mut(&self.0) {
            f.running = false;
        }
    }
}

fn start_ticking() {
    if TICKING.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        let mut was_hidden = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let channels = {
                let mut interest = locked(&INTEREST);
                // A window that went away without saying so.
                if let Some(app) = crate::services::providers::app_handle() {
                    use tauri::Manager;
                    let open: Vec<String> = app.webview_windows().into_keys().collect();
                    keep_open_windows(&mut interest, &open);
                }
                watched(&interest)
            };
            if channels.is_empty() {
                TICKING.store(false, Ordering::SeqCst);
                // A watch that came in while stopping starts it again.
                if !watched(&locked(&INTEREST)).is_empty() {
                    start_ticking();
                }
                return;
            }
            // A locked phone or a hidden desktop reads nothing; one read each
            // when it shows again.
            let hidden = crate::services::window_visibility::all_hidden();
            if hidden {
                was_hidden = true;
                continue;
            }
            let now = now_ms();
            let due: Vec<(String, String)> = {
                let mut fresh = locked(&FRESH);
                fresh.retain(|id, _| channels.contains_key(id));
                channels
                    .iter()
                    .filter_map(|(id, login)| {
                        let f = fresh.entry(id.clone()).or_default();
                        let go = !f.running && (was_hidden || now >= next_publish_at(f));
                        if go {
                            f.running = true;
                            f.asked = false;
                            f.last_ms = now;
                        }
                        go.then(|| (id.clone(), login.clone()))
                    })
                    .collect()
            };
            was_hidden = false;
            for (id, login) in due {
                tauri::async_runtime::spawn(async move {
                    let _guard = RunningGuard(id.clone());
                    publish(&id, &login).await;
                });
            }
        }
    });
}

/// Read a channel's manager again and send it to every window.
async fn publish(channel_id: &str, channel_login: &str) {
    use tauri::{Emitter, Manager};
    let Some(app) = crate::services::providers::app_handle() else { return };
    let Some(auth) = app.try_state::<crate::models::settings::AppState>().map(|s| s.twitch_auth.clone()) else { return };
    match view_and_queue(&auth, channel_id, channel_login).await {
        Ok((view, queue)) => {
            let _ = app.emit("stream-manager-state", json!({ "channel_id": channel_id, "view": view, "queue": queue }));
        }
        Err(e) => {
            log::debug!("[{LOG_TAG}] background read of {channel_login} failed: {e}");
            // No view to set the next time: back to the 30 s pace, never a
            // read every 3 s on a deadline that is long gone.
            if let Some(f) = locked(&FRESH).get_mut(channel_id) {
                disarm(f);
            }
        }
    }
}

/// A failed read forgets its out-of-date time, so the channel is read again
/// at the periodic pace.
pub fn disarm(f: &mut Freshness) {
    f.reread = None;
}

/// After a typed command changed something the manager shows: read again
/// soon when a surface shows that channel.
pub fn changed_outside(channel_id: &str) {
    if watched(&locked(&INTEREST)).contains_key(channel_id) {
        locked(&FRESH).entry(channel_id.to_string()).or_default().asked = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft(title: &str, tags: &[&str]) -> StreamInfoDraft {
        StreamInfoDraft {
            title: title.into(),
            category: Some(Category { id: "516575".into(), name: "Valorant".into(), box_art_url: "https://x/{width}x{height}.jpg".into() }),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            language: "en".into(),
        }
    }

    #[test]
    fn roles_get_what_twitch_lets_them() {
        let b = Role::Broadcaster;
        let m = Role::Moderator;
        let e = Role::Editor;
        for s in [Section::Info, Section::Poll, Section::Raid, Section::Ads, Section::Rewards, Section::Chat] {
            assert!(can_use(b, s).is_ok());
        }
        assert!(can_use(m, Section::Poll).is_ok());
        assert!(can_use(m, Section::Rewards).is_ok());
        assert!(can_use(m, Section::Chat).is_ok());
        assert_eq!(can_use(m, Section::Raid), Err("Only the broadcaster can raid from this channel"));
        assert!(can_use(m, Section::Info).is_err());
        assert!(can_use(e, Section::Info).is_ok());
        assert!(can_use(e, Section::Chat).is_err());
        assert!(can_use(Role::ModeratorEditor, Section::Info).is_ok());
        assert!(!Role::Viewer.manages());
        assert_eq!(Role::from_flags(false, true, true), Role::ModeratorEditor);
        assert_eq!(Role::from_flags(true, false, false), Role::Broadcaster);
    }

    #[test]
    fn a_moderator_sees_no_row_that_would_refuse_them() {
        let rows = section_rows(Role::Moderator, None, None, None, None, None, Some(3), 0);
        let ids: Vec<Section> = rows.iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![Section::Poll, Section::Prediction, Section::Rewards, Section::Announce]);
        assert_eq!(rows[2].badge.as_deref(), Some("3 waiting"));
    }

    #[test]
    fn running_poll_renames_its_row() {
        let poll = RunningPoll { id: "p".into(), title: "Which boss?".into(), ends_at_ms: None };
        let rows = section_rows(Role::Broadcaster, None, Some(&poll), None, None, None, Some(0), 0);
        let row = rows.iter().find(|r| r.id == Section::Poll).unwrap();
        assert_eq!(row.label, "Poll running");
        assert_eq!(row.detail.as_deref(), Some("Which boss?"));
        // An empty queue has no count.
        assert!(rows.iter().find(|r| r.id == Section::Rewards).unwrap().badge.is_none());
    }

    #[test]
    fn pages_follow_the_role() {
        let ids = |role: Role| page_rows(role, None, None, None, None).iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(
            ids(Role::Broadcaster),
            vec!["info", "poll", "prediction", "raid", "ads", "rewards", "announce", "chat_modes", "moderation"]
        );
        assert_eq!(ids(Role::Moderator), vec!["poll", "prediction", "rewards", "announce", "chat_modes", "moderation"]);
        assert_eq!(ids(Role::Editor), vec!["info", "poll", "prediction", "rewards"]);
        assert_eq!(
            ids(Role::ModeratorEditor),
            vec!["info", "poll", "prediction", "rewards", "announce", "chat_modes", "moderation"]
        );
        assert!(ids(Role::Viewer).is_empty());
        let mine = page_rows(Role::Broadcaster, None, None, None, None);
        assert_eq!(mine[0].group, "Your channel");
        assert_eq!(mine.iter().find(|p| p.id == "announce").unwrap().group, "Chat");
        assert_eq!(page_rows(Role::Moderator, None, None, None, None)[0].group, "Channel");
    }

    #[test]
    fn pages_mark_what_is_running() {
        let poll = RunningPoll { id: "p".into(), title: "Q".into(), ends_at_ms: None };
        let prediction = RunningPrediction { id: "e".into(), title: "W".into(), status: "locked".into(), outcomes: vec![], locks_at_ms: None };
        let raid = RaidView { target_id: "t".into(), target_login: "t".into(), target_name: "T".into(), deadline_ms: 1, total_ms: 1 };
        let rows = page_rows(Role::Broadcaster, Some(&poll), Some(&prediction), Some(&raid), Some(3));
        let get = |id: &str| rows.iter().find(|p| p.id == id).unwrap().clone();
        assert_eq!(get("poll").live, Some("running"));
        assert_eq!(get("prediction").live, Some("locked"));
        assert_eq!(get("raid").live, Some("counting down"));
        assert_eq!(get("rewards").badge.as_deref(), Some("3"));
        let quiet = page_rows(Role::Broadcaster, None, None, None, Some(0));
        assert!(quiet.iter().find(|p| p.id == "rewards").unwrap().badge.is_none());
        assert!(quiet.iter().all(|p| p.live.is_none()));
    }

    #[test]
    fn the_preview_carries_the_live_cards_facts_only_while_live() {
        let user = json!({ "broadcaster_type": "partner" });
        let stream = json!({ "viewer_count": 1234, "started_at": "2026-11-10T08:00:00Z" });
        let mut live = check_stream_info(&draft("t", &[]), None, "you", None, Some("https://thumb".into())).preview;
        live_facts(&mut live, &stream, &user);
        assert_eq!(live.viewer_count, Some(1234));
        assert_eq!(live.started_at.as_deref(), Some("2026-11-10T08:00:00Z"));
        assert!(live.partner);
        // Offline: no thumbnail, so no count or start even if a stale row says so.
        let mut offline = check_stream_info(&draft("t", &[]), None, "you", None, None).preview;
        live_facts(&mut offline, &stream, &json!({ "broadcaster_type": "affiliate" }));
        assert_eq!((offline.viewer_count, offline.started_at), (None, None));
        assert!(!offline.partner);
    }

    #[test]
    fn stream_info_rules() {
        let ok = check_stream_info(&draft("Ranked grind", &["English", "Ranked"]), None, "you", None, None);
        assert!(ok.ok);
        assert_eq!(ok.title_count, "12 / 140");
        assert_eq!(ok.tags_count, "2 / 10");
        assert_eq!(ok.preview.category_name, "Valorant");
        assert_eq!(ok.preview.box_art_url.as_deref(), Some("https://x/52x72.jpg"));

        let empty = check_stream_info(&draft("  ", &[]), None, "you", None, None);
        assert_eq!(empty.problems[0], FieldProblem { field: "title".into(), line: "Add a title".into() });

        let long = check_stream_info(&draft(&"a".repeat(141), &[]), None, "you", None, None);
        assert!(!long.ok);

        let bad = check_stream_info(&draft("t", &["two words", "ok", "OK", "#hash", &"x".repeat(26)]), None, "you", None, None);
        let fields: Vec<&str> = bad.problems.iter().map(|p| p.field.as_str()).collect();
        assert_eq!(fields, vec!["tag:0", "tag:2", "tag:4"]);
        // A leading # is dropped, not refused.
        assert!(bad.preview.tags.contains(&"hash".to_string()));

        let many: Vec<String> = (0..11).map(|i| format!("t{i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(check_stream_info(&draft("t", &refs), None, "you", None, None).problems[0].field, "tags");

        // Letters in any script are fine.
        assert!(check_stream_info(&draft("t", &["日本語", "Español"]), None, "you", None, None).ok);

        let mut lang = draft("t", &[]);
        lang.language = "xx".into();
        assert_eq!(check_stream_info(&lang, None, "you", None, None).problems[0].field, "language");
    }

    #[test]
    fn unchanged_info_is_not_a_change() {
        let row = json!({ "title": "Ranked grind", "game_id": "516575", "game_name": "Valorant", "tags": ["English"], "broadcaster_language": "en" });
        let current = info_from_helix(&row, None);
        assert_eq!(current.summary, "Ranked grind · Valorant");
        assert_eq!(current.language_name, "English");
        assert!(!check_stream_info(&draft("Ranked grind", &["English"]), Some(&current), "you", None, None).changed);
        assert!(check_stream_info(&draft("Ranked grind!", &["English"]), Some(&current), "you", None, None).changed);
    }

    #[test]
    fn title_count_tones() {
        let at = |n: usize| check_stream_info(&draft(&"a".repeat(n), &[]), None, "you", None, None);
        assert_eq!((at(54).title_count.as_str(), at(54).title_tone), ("54 / 140", "quiet"));
        assert_eq!(at(125).title_tone, "quiet");
        assert_eq!(at(126).title_tone, "near");
        assert_eq!((at(140).title_count.as_str(), at(140).title_tone), ("140 / 140", "near"));
        assert_eq!((at(146).title_count.as_str(), at(146).title_tone), ("146 / 140 · 6 over", "over"));
        // An emoji is one; a joined family counts each of its five parts.
        assert_eq!(at_title("🔴 Live").title_count, "6 / 140");
        assert_eq!(at_title("👨‍👩‍👧").title_count, "5 / 140");
    }

    fn at_title(t: &str) -> InfoCheck {
        check_stream_info(&draft(t, &[]), None, "you", None, None)
    }

    #[test]
    fn emoji_in_a_title() {
        assert_eq!(emoji_in("🔴 Ranked grind to Diamond 💎 then 🔴 chill 🎧"), vec!["🔴", "💎", "🎧"]);
        assert_eq!(emoji_in("👨‍👩‍👧 ❤️ 1️⃣ 🇺🇸 #1 © ok"), vec!["👨‍👩‍👧", "❤️", "1️⃣", "🇺🇸"]);
        assert!(emoji_in("Ranked grind, día 3 · 日本語").is_empty());
    }

    #[test]
    fn recents_lead_with_what_is_on_now() {
        let row = json!({ "title": "🔴 Ranked 💎", "game_id": "1", "game_name": "Valorant", "tags": [], "broadcaster_language": "en" });
        let mut info = info_from_helix(&row, None);
        with_recents(&mut info, None);
        assert_eq!(info.recent_categories.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["1"]);
        assert_eq!(info.title_emoji, vec!["🔴", "💎"]);
        assert_eq!(info.title_emoji_heading, "Your title emoji");

        let cat = |id: &str| Category { id: id.into(), name: id.into(), box_art_url: String::new() };
        let kept = InfoRecents { categories: vec![cat("2"), cat("1"), cat("3")], emoji: vec!["🎮".into(), "🔴".into()] };
        with_recents(&mut info, Some(&kept));
        assert_eq!(info.recent_categories.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), vec!["1", "2", "3"]);
        assert_eq!(info.title_emoji, vec!["🔴", "💎", "🎮"]);

        let plain = json!({ "title": "Ranked", "game_id": "", "tags": [], "broadcaster_language": "en" });
        let mut bare = info_from_helix(&plain, None);
        with_recents(&mut bare, None);
        assert!(bare.recent_categories.is_empty());
        assert_eq!(bare.title_emoji.len(), STARTER_EMOJI.len());
        assert_eq!(bare.title_emoji_heading, "Popular in titles");
    }

    #[test]
    fn a_save_moves_its_category_and_emoji_to_the_front() {
        let cat = |id: &str| Category { id: id.into(), name: id.into(), box_art_url: String::new() };
        let before = InfoRecents { categories: (0..10).map(|i| cat(&format!("c{i}"))).collect(), emoji: vec!["🎮".into()] };
        let after = after_save(&before, &draft("✨ new run 🎮", &[]));
        assert_eq!(after.categories.len(), RECENT_CATEGORIES_MAX);
        assert_eq!(after.categories[0].id, "516575");
        assert_eq!(after.categories[1].id, "c0");
        assert_eq!(after.emoji, vec!["✨", "🎮"]);
    }

    #[test]
    fn poll_and_prediction_limits() {
        assert!(check_poll("Q", &["a".into(), " ".into()], 60).is_err());
        assert_eq!(check_poll("Q", &["a".into(), " b ".into()], 60).unwrap(), vec!["a", "b"]);
        assert!(check_poll("Q", &["a".into(), "b".into()], 10).is_err());
        assert!(check_prediction(&"q".repeat(46), &["a".into(), "b".into()], 60).is_err());
        assert!(check_prediction("Q", &["a".into(), "b".into()], 20).is_err());
        assert!(check_prediction("Q", &["a".into(), "b".into()], 120).is_ok());
    }

    #[test]
    fn requests_match_the_probed_schema() {
        let r = redemptions_request("1", &["a".into(), "b".into()], false);
        assert_eq!(r["variables"]["input"]["newStatus"], "CANCELED");
        assert_eq!(r["variables"]["input"]["oldStatus"], "UNFULFILLED");
        assert_eq!(r["variables"]["input"]["redemptionIDs"], json!(["a", "b"]));
        assert_eq!(reward_queue_request("1", "r", true)["variables"]["input"]["newStatus"], "FULFILLED");
        let p = reward_request("1", "r", Some(true), None);
        assert_eq!(p["variables"]["input"], json!({ "channelID": "1", "rewardID": "r", "isPaused": true }));
        let b = broadcast_request("1", &draft(" Hi ", &[]));
        assert_eq!(b["variables"]["input"]["status"], "Hi");
        assert_eq!(b["variables"]["input"]["categoryID"], "516575");
        assert_eq!(b["variables"]["input"]["broadcasterLanguage"], "en");
        assert_eq!(tags_request("1", &["a".into()])["variables"]["input"]["contentType"], "CHANNEL");
        let pr = create_prediction_request("1", "Q", &["Yes".into(), "No".into(), "Maybe".into()], 60);
        let colors: Vec<&str> = pr["variables"]["input"]["outcomes"].as_array().unwrap().iter().map(|o| o["color"].as_str().unwrap()).collect();
        assert_eq!(colors, vec!["BLUE", "PINK", "PINK"]);
        let poll = create_poll_request("1", "Q", &["a".into(), "b".into()], 60, Some(100));
        assert_eq!(poll["variables"]["input"]["communityPointsCost"], 100);
        assert_eq!(poll["variables"]["input"]["isCommunityPointsVotingEnabled"], true);
    }

    #[test]
    fn gql_answers_in_words() {
        let ok = json!({ "data": { "createPoll": { "poll": { "id": "1" }, "error": null } } });
        assert!(gql_outcome(&ok, "createPoll", "x").is_ok());
        let refused = json!({ "data": { "createPoll": { "poll": null, "error": { "code": "FORBIDDEN" } } } });
        assert_eq!(gql_outcome(&refused, "createPoll", "Could not start the poll").unwrap_err(), "Could not start the poll: Twitch says you can't do that in this channel");
        let bare = json!({ "data": { "updateBroadcastSettings": { "error": "INVALID_CATEGORY" } } });
        assert_eq!(gql_outcome(&bare, "updateBroadcastSettings", "x").unwrap_err(), "x: invalid category");
        let failed = json!({ "errors": [{ "message": "failed integrity check" }] });
        assert!(gql_outcome(&failed, "createPoll", "x").unwrap_err().contains("security check"));
        let nothing = json!({ "data": { "createPoll": null } });
        assert!(gql_outcome(&nothing, "createPoll", "x").is_err());
    }

    #[test]
    fn queue_groups_by_reward_oldest_first() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-18T12:10:00Z").unwrap().timestamp_millis();
        let node = |id: &str, reward: &str, title: &str, user: &str, at: &str, input: Option<&str>| {
            json!({ "node": {
                "id": id, "status": "UNFULFILLED", "input": input, "timestamp": at, "rewardTitle": title,
                "user": { "id": "u", "login": user, "displayName": user, "profileImageURL": "https://p" },
                "reward": { "id": reward, "title": title, "cost": 500, "isPaused": false, "isEnabled": true,
                            "backgroundColor": "#00C7AC", "image": null }
            } })
        };
        let body = json!({ "data": { "channel": { "communityPointsRedemptionQueue": {
            "edges": [
                node("1", "song", "Song request", "pixelfox", "2026-10-18T12:08:00Z", Some("Veridis Quo")),
                node("2", "hydrate", "Hydrate", "lunar", "2026-10-18T12:09:50Z", None),
                node("3", "song", "Song request", "mossy", "2026-10-18T12:09:00Z", Some("  ")),
            ],
            "pageInfo": { "hasNextPage": false }
        } } } });
        let q = parse_queue(&body, now);
        assert_eq!(q.count, 3);
        assert_eq!(q.groups.len(), 2);
        assert_eq!(q.groups[0].heading, "Song request · 2");
        assert_eq!(q.groups[0].rows[0].line, "pixelfox · 2 min ago");
        assert_eq!(q.groups[0].rows[0].input.as_deref(), Some("Veridis Quo"));
        assert_eq!(q.groups[0].rows[1].input, None);
        assert_eq!(q.groups[1].rows[0].line, "lunar · just now");
        // No art of its own: Twitch's default reward picture (SN-264), and the
        // query no longer asks for `defaultImage`, which once came back nil.
        assert_eq!(q.groups[0].image_url.as_deref(), Some(DEFAULT_REWARD_IMAGE));
        assert!(!QUEUE_QUERY.contains("defaultImage"));
        assert!(q.empty_line.is_none());
        let empty = parse_queue(&json!({ "data": { "channel": { "communityPointsRedemptionQueue": { "edges": [] } } } }), now);
        assert_eq!(empty.empty_line.as_deref(), Some("Nothing waiting"));
    }

    #[test]
    fn ad_schedule_in_words() {
        let now = 1_000_000_000_000i64;
        let row = json!({ "next_ad_at": (now / 1000) + 14 * 60, "duration": 60, "preroll_free_time": 1380,
                          "snooze_count": 2, "snooze_refresh_at": 0, "last_ad_at": 0 });
        let s = ad_schedule(&row, now, 0);
        assert_eq!(s.snooze_line, "2 snoozes left");
        assert_eq!(s.preroll_line.as_deref(), Some("Pre-rolls are off for 23 min"));
        let ads = AdsView { lengths: ad_lengths(), schedule: Some(s), schedule_hint: None };
        assert_eq!(ads_detail(&ads, now).as_deref(), Some("Next ad in 14 min · 2 snoozes left"));
        let none = ad_schedule(&json!({ "snooze_count": "0", "snooze_refresh_at": (now / 1000) + 42 * 60 }), now, 0);
        assert_eq!(none.snooze_line, "No snoozes left, one comes back in 42 min");
        assert_eq!(none.next_ad_ms, None);
        // Twitch's clock 5 s ahead: its times land 5 s earlier here.
        let skewed = ad_schedule(&row, now, 5000);
        assert_eq!(skewed.next_ad_ms, Some(now + 14 * 60_000 - 5000));
        let labels: Vec<String> = ad_lengths().into_iter().map(|c| c.label).collect();
        assert_eq!(labels, vec!["30s", "1 min", "1:30", "2 min", "2:30", "3 min"]);
    }

    #[test]
    fn chat_modes_from_helix() {
        let v = chat_view(&json!({ "follower_mode": true, "follower_mode_duration": 10, "slow_mode": false, "slow_mode_wait_time": null,
                                   "subscriber_mode": false, "emote_mode": true, "unique_chat_mode": false }), Some(false));
        assert_eq!(v.modes[0].value_label.as_deref(), Some("10 min"));
        assert!(v.modes[3].on);
        assert_eq!(v.modes[1].value, None);
        assert_eq!(v.shield, Some(false));
        assert_eq!(chat_mode_body("slow", true, Some(500)).unwrap(), json!({ "slow_mode": true, "slow_mode_wait_time": 120 }));
        assert_eq!(chat_mode_body("followers", false, None).unwrap(), json!({ "follower_mode": false }));
        assert!(chat_mode_body("nope", true, None).is_err());
    }

    #[test]
    fn running_poll_and_prediction_from_gql() {
        let body = json!({ "data": {
            "user": { "latestPoll": { "id": "q", "status": "ACTIVE", "title": "Boss?" } },
            "channel": { "activePredictionEvents": [], "lockedPredictionEvents": [
                { "id": "p", "status": "LOCKED", "title": "Win?", "outcomes": [{ "id": "a", "title": "Yes", "color": "BLUE" }] }
            ] }
        } });
        let read = running_from_gql(&body, 1000, 0);
        assert_eq!(read.poll.unwrap().unwrap().id, "q");
        let pred = read.prediction.unwrap().unwrap();
        assert_eq!(pred.status, "locked");
        assert_eq!(pred.outcomes[0].color, "blue");
        // A locked prediction locks at no time.
        assert_eq!(pred.locks_at_ms, None);
        let ended = json!({ "data": { "user": { "latestPoll": { "id": "q", "status": "COMPLETED", "title": "x" } }, "channel": {} } });
        assert_eq!(running_from_gql(&ended, 0, 0).poll, Ok(None));
    }

    #[test]
    fn running_times_on_this_clock() {
        // The poll's end from what Twitch says is left, counted from arrival.
        let body = json!({ "data": {
            "user": { "latestPoll": { "id": "q", "status": "ACTIVE", "title": "Boss?", "startedAt": "2026-11-10T12:00:00Z",
                                       "durationSeconds": 60, "remainingDurationMilliseconds": 45_000 } },
            "channel": { "activePredictionEvents": [
                { "id": "p", "status": "ACTIVE", "title": "Win?", "createdAt": "2026-11-10T12:00:00Z", "predictionWindowSeconds": 120, "outcomes": [] }
            ], "lockedPredictionEvents": [] }
        } });
        let t0 = 1_794_312_000_000i64;
        let read = running_from_gql(&body, 5_000, 2000);
        assert_eq!(read.poll.unwrap().unwrap().ends_at_ms, Some(50_000));
        assert_eq!(read.prediction.unwrap().unwrap().locks_at_ms, Some(t0 + 120_000 - 2000));
        // Without the remaining time, from the start and length.
        let helix = json!({ "id": "q", "status": "ACTIVE", "title": "x", "started_at": "2026-11-10T12:00:00Z", "duration": 60 });
        assert_eq!(running_poll_helix(&helix, -3000).unwrap().ends_at_ms, Some(t0 + 63_000));
        assert_eq!(offset_from(Some(10_500), 10_000), 500);
        assert_eq!(offset_from(None, 10_000), 0);
        assert_eq!(to_local(10_500, 500), 10_000);
    }

    #[test]
    fn a_part_that_did_not_answer_is_unknown() {
        let errors = json!([{ "message": "service timeout", "path": ["channel", "activePredictionEvents"] }]);
        assert!(part_known(&errors, &["user", "latestPoll"]));
        assert!(!part_known(&errors, &["channel", "activePredictionEvents"]));
        assert!(!part_known(&json!([{ "message": "x", "path": ["channel"] }]), &["channel", "lockedPredictionEvents"]));
        assert!(!part_known(&json!([{ "message": "x", "path": ["user", "latestPoll", "title"] }]), &["user", "latestPoll"]));
        assert!(!part_known(&json!([{ "message": "x" }]), &["user", "latestPoll"]));
        assert!(part_known(&Value::Null, &["user", "latestPoll"]));

        let body = json!({ "errors": errors, "data": { "user": { "latestPoll": null }, "channel": null } });
        let read = running_from_gql(&body, 0, 0);
        assert_eq!(read.poll, Ok(None));
        assert!(read.prediction.is_err());

        // What could not be read stays as last known, marked so.
        let last = Running {
            poll: None,
            prediction: Some(RunningPrediction { id: "e".into(), title: "W".into(), status: "active".into(), outcomes: vec![], locks_at_ms: Some(9) }),
            poll_known: true,
            prediction_known: true,
        };
        let settled = settle_running(&read, Some(&last));
        assert!(settled.poll_known);
        assert!(!settled.prediction_known);
        assert_eq!(settled.prediction.unwrap().id, "e");
        assert!(settle_running(&RunningRead::failed("down"), None).poll.is_none());
    }

    fn quiet_view() -> StreamManagerView {
        StreamManagerView {
            rev: 1,
            user_id: "me".into(),
            channel_id: "c".into(),
            channel_login: "c".into(),
            role: Role::Broadcaster,
            role_line: String::new(),
            sections: vec![],
            pages: vec![],
            info: None,
            poll: None,
            prediction: None,
            poll_known: true,
            prediction_known: true,
            raid: None,
            ads: None,
            chat: None,
            queue_count: None,
            limits: form_limits(),
            recent_polls: vec![],
            recent_predictions: vec![],
            problems: vec![],
        }
    }

    #[test]
    fn reads_again_when_the_view_goes_out_of_date() {
        let now = 100_000;
        let mut v = quiet_view();
        assert_eq!(reread_at(&v, now), None);
        v.poll = Some(RunningPoll { id: "p".into(), title: "Q".into(), ends_at_ms: Some(now + 20_000) });
        assert_eq!(reread_at(&v, now), Some(now + 21_500));
        v.poll.as_mut().unwrap().ends_at_ms = Some(now - 1000);
        assert_eq!(reread_at(&v, now), Some(now + 2000));
        v.poll.as_mut().unwrap().ends_at_ms = Some(now - 20_000);
        assert_eq!(reread_at(&v, now), Some(now + 5000));
        v.poll.as_mut().unwrap().ends_at_ms = Some(now - 60_000);
        assert_eq!(reread_at(&v, now), Some(now + 15_000));
        v.poll.as_mut().unwrap().ends_at_ms = Some(now - 121_000);
        assert_eq!(reread_at(&v, now), None);
        // The earliest wins; a snooze coming back counts; a locked prediction does not.
        v.prediction = Some(RunningPrediction { id: "e".into(), title: "W".into(), status: "locked".into(), outcomes: vec![], locks_at_ms: Some(now + 1) });
        v.ads = Some(AdsView {
            lengths: vec![],
            schedule: Some(AdSchedule {
                next_ad_ms: Some(now + 90_000),
                next_ad_seconds: 60,
                preroll_free_seconds: 0,
                snoozes_left: 0,
                snooze_refresh_ms: Some(now + 40_000),
                snooze_line: String::new(),
                preroll_line: None,
            }),
            schedule_hint: None,
        });
        assert_eq!(reread_at(&v, now), Some(now + 41_500));
        v.raid = Some(RaidView { target_id: "t".into(), target_login: "t".into(), target_name: "T".into(), deadline_ms: now + 10_000, total_ms: 90_000 });
        assert_eq!(reread_at(&v, now), Some(now + 11_500));
    }

    #[test]
    fn background_reads_keep_their_pace() {
        let f = Freshness { last_ms: 1000, ..Freshness::default() };
        assert_eq!(next_publish_at(&f), 31_000);
        let soon = Freshness { reread: Some(5000), ..f.clone() };
        assert_eq!(next_publish_at(&soon), 5000);
        // Never within 3 s of the last read, however soon it is wanted.
        let now_please = Freshness { reread: Some(1500), ..f.clone() };
        assert_eq!(next_publish_at(&now_please), 4000);
        let asked = Freshness { asked: true, ..f };
        assert_eq!(next_publish_at(&asked), 4000);
        assert!(should_arm(5, 5));
        assert!(should_arm(5, 6));
        assert!(!should_arm(5, 4));
    }

    #[test]
    fn surfaces_say_what_they_show() {
        let mut m = InterestMap::new();
        let chan = || Some(("1".to_string(), "one".to_string()));
        interest_after(&mut m, "main", "panel", chan(), 2).unwrap();
        // An older call for the same surface changes nothing.
        interest_after(&mut m, "main", "panel", None, 1).unwrap();
        assert_eq!(watched(&m).get("1").map(String::as_str), Some("one"));
        interest_after(&mut m, "main", "panel", None, 3).unwrap();
        assert!(watched(&m).is_empty());
        // A stale "on" after the "off" stays off.
        interest_after(&mut m, "main", "panel", chan(), 2).unwrap();
        assert!(watched(&m).is_empty());
        for i in 0..SURFACES_PER_WINDOW {
            interest_after(&mut m, "main", &format!("s{i}"), chan(), 10).unwrap();
        }
        assert!(interest_after(&mut m, "main", "one-too-many", chan(), 10).is_err());
        assert!(interest_after(&mut m, "popout", "panel", chan(), 1).is_ok());
    }

    #[test]
    fn a_shell_without_webviews_keeps_its_sheet_watched() {
        let chan = || Some(("1".to_string(), "one".to_string()));
        let mut m = InterestMap::new();
        interest_after(&mut m, "main", "ios-sheet-1", chan(), 1).unwrap();
        interest_after(&mut m, "popout-x", "panel", chan(), 1).unwrap();
        // The iPhone's headless core lists no webview windows: nothing goes.
        keep_open_windows(&mut m, &[]);
        assert_eq!(m.len(), 2);
        // Desktop: a window gone without saying so loses its surfaces.
        keep_open_windows(&mut m, &["main".to_string()]);
        assert_eq!(m.keys().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), vec!["main"]);
        assert_eq!(watched(&m).get("1").map(String::as_str), Some("one"));
    }

    #[test]
    fn pubsub_moves_only_on_state_changes() {
        let t = |kind: &str| json!({ "type": kind });
        assert!(pubsub_changes_manager("polls", &t("POLL_CREATE")));
        assert!(pubsub_changes_manager("polls", &t("POLL_TERMINATE")));
        assert!(!pubsub_changes_manager("polls", &t("POLL_UPDATE")));
        assert!(pubsub_changes_manager("predictions-channel-v1", &t("event-created")));
        let upd = |s: &str| json!({ "type": "event-updated", "data": { "event": { "status": s } } });
        assert!(pubsub_changes_manager("predictions-channel-v1", &upd("LOCKED")));
        assert!(!pubsub_changes_manager("predictions-channel-v1", &upd("ACTIVE")));
        assert!(pubsub_changes_manager("community-points-channel-v1", &t("reward-redeemed")));
        assert!(!pubsub_changes_manager("community-points-user-v1", &t("reward-redeemed")));
    }

    #[test]
    fn roles_from_what_is_known() {
        use crate::services::chat_send_limiter::Role as Chat;
        let ed = Some(Flags { editor: true, moderator: false });
        assert_eq!(role_from(true, None, None, None), Ok(Role::Broadcaster));
        assert!(role_from(false, None, None, None).is_err());
        // Chat never says editor: a failed read keeps the last flags.
        assert_eq!(role_from(false, Some(Chat::Viewer), None, ed), Ok(Role::Editor));
        assert_eq!(role_from(false, Some(Chat::Moderator), None, ed), Ok(Role::ModeratorEditor));
        // This run's read wins over chat and the cache.
        assert_eq!(role_from(false, Some(Chat::Moderator), Some(Flags { editor: false, moderator: false }), ed), Ok(Role::Viewer));
        assert_eq!(role_from(false, Some(Chat::Moderator), None, None), Ok(Role::Moderator));
        assert_eq!(role_from(false, None, None, Some(Flags { editor: false, moderator: true })), Ok(Role::Moderator));
    }

    #[test]
    fn the_role_cache_keeps_the_newest_read() {
        let mut c = RoleCache::default();
        let key = ("me".to_string(), "c".to_string());
        let ed = Flags { editor: true, moderator: false };
        let none = Flags { editor: false, moderator: false };
        c.put(key.clone(), ed, 5, 0);
        // A read that started earlier lands later: ignored.
        c.put(key.clone(), none, 4, 10);
        assert_eq!(c.last(&key), Some(ed));
        assert_eq!(c.fresh(&key, ROLE_FRESH_MS - 1), Some(ed));
        assert_eq!(c.fresh(&key, ROLE_FRESH_MS), None);
        assert_eq!(c.last(&key), Some(ed));
        // A day on, it is dropped when the next one is kept.
        c.put(("me".into(), "other".into()), none, 9, ROLE_KEEP_MS + 1);
        assert_eq!(c.last(&key), None);
    }

    #[test]
    fn the_web_session_must_be_the_same_account() {
        assert!(web_binding(Some("1"), "1").is_ok());
        assert_eq!(web_binding(Some("2"), "1"), Err(WEB_OTHER_ACCOUNT.to_string()));
        assert!(web_binding(None, "1").is_err());
    }

    #[test]
    fn a_save_sends_only_what_it_names() {
        let d = draft("New title", &["One", "#Two"]);
        let tags = check_info_fields(&d, &[InfoField::Title, InfoField::Tags]).unwrap();
        assert_eq!(tags, vec!["One", "Two"]);
        assert_eq!(info_patch(&d, &[InfoField::Title], &tags).unwrap(), json!({ "title": "New title" }));
        assert_eq!(
            info_patch(&d, &[InfoField::Category, InfoField::Language], &tags).unwrap(),
            json!({ "game_id": "516575", "broadcaster_language": "en" })
        );
        let mut bare = d.clone();
        bare.category = None;
        assert_eq!(info_patch(&bare, &[InfoField::Category], &tags), Err("Nothing changed".to_string()));
        assert_eq!(check_info_fields(&d, &[]), Err("Nothing changed".to_string()));
        // A bad title does not stop a tags-only save, and a bad tag does not stop a title save.
        let mut bad_title = d.clone();
        bad_title.title = "  ".into();
        assert!(check_info_fields(&bad_title, &[InfoField::Tags]).is_ok());
        assert_eq!(check_info_fields(&bad_title, &[InfoField::Title]), Err("Add a title".to_string()));
        let bad_tag = draft("ok", &["two words"]);
        assert!(check_info_fields(&bad_tag, &[InfoField::Title]).is_ok());
        assert!(check_info_fields(&bad_tag, &[InfoField::Tags]).is_err());
        assert_eq!(gql_save_plan(&[InfoField::Tags]), (false, true));
        assert_eq!(gql_save_plan(&[InfoField::Title]), (true, false));
    }

    #[test]
    fn a_merged_save_keeps_the_other_fields() {
        let live = draft("Live title", &["Live"]);
        let mut mine = draft("My title", &["Mine"]);
        mine.category = Some(Category { id: "9".into(), name: "Other".into(), box_art_url: String::new() });
        let none = HashMap::new();
        let out = merged_info(&mine, &[InfoField::Title], &live, &none, 0);
        assert_eq!(out.title, "My title");
        assert_eq!(out.category.unwrap().id, "516575");
        // An earlier save here that a read does not show yet is kept.
        let mut earlier = draft("Live title", &[]);
        earlier.category = Some(Category { id: "7".into(), name: "Seven".into(), box_art_url: String::new() });
        let mut kept = HashMap::new();
        kept.insert(
            InfoField::Category,
            KeptWrite { written: field_value(&earlier, InfoField::Category), before: field_value(&live, InfoField::Category), draft: earlier, at_ms: 0 },
        );
        assert_eq!(merged_info(&mine, &[InfoField::Title], &live, &kept, 1000).category.unwrap().id, "7");
        // Not once it is old, and not once someone else changed it since.
        assert_eq!(merged_info(&mine, &[InfoField::Title], &live, &kept, KEPT_WRITE_MS).category.unwrap().id, "516575");
        let mut moved = live.clone();
        moved.category = Some(Category { id: "8".into(), name: "Eight".into(), box_art_url: String::new() });
        assert_eq!(merged_info(&mine, &[InfoField::Title], &moved, &kept, 1000).category.unwrap().id, "8");
        // A read that shows the write forgets it.
        let mut shown = live.clone();
        shown.category = Some(Category { id: "7".into(), name: "Seven".into(), box_art_url: String::new() });
        let mut k2 = kept.clone();
        prune_kept(&mut k2, &shown, 1000);
        assert!(k2.is_empty());
        let mut k3 = kept;
        prune_kept(&mut k3, &live, 1000);
        assert_eq!(k3.len(), 1);
    }

    #[test]
    fn poll_recents_lead_with_the_latest() {
        let r = |t: &str| RecentPoll { title: t.into(), options: vec!["a".into(), "b".into()] };
        let kept = PollRecents { polls: (0..10).map(|i| r(&format!("q{i}"))).collect(), predictions: vec![] };
        let next = after_start(&kept, PollKind::Poll, " Q3 ", &["x".into()]);
        assert_eq!(next.polls.len(), 10);
        assert_eq!(next.polls[0].title, "Q3");
        assert_eq!(next.polls[0].options, vec!["x"]);
        assert_eq!(next.polls.iter().filter(|p| p.title.eq_ignore_ascii_case("q3")).count(), 1);
        assert!(after_start(&kept, PollKind::Prediction, "Win?", &[]).predictions.len() == 1);
        assert_eq!(merge_recents(&[r("a")], &[r("A"), r("b")]).iter().map(|p| p.title.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn recents_carry_over_from_settings_once() {
        let mut extra = HashMap::new();
        assert_eq!(seed_from(&extra), None);
        extra.insert("recent_polls".to_string(), json!([{ "title": "Boss?", "options": ["a", "b"] }, { "nope": 1 }, "bad", { "title": " " }]));
        let seeded = seed_from(&extra).unwrap();
        assert_eq!(seeded.polls, vec![RecentPoll { title: "Boss?".into(), options: vec!["a".into(), "b".into()] }]);
        assert!(seeded.predictions.is_empty());

        let missing: ReadPlan<PollRecents> =
            recents_read_plan(Err(std::io::Error::new(std::io::ErrorKind::NotFound, "gone")));
        assert_eq!(missing, ReadPlan::Missing);
        let denied: ReadPlan<PollRecents> =
            recents_read_plan(Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no")));
        assert!(matches!(denied, ReadPlan::Error(_)));
        assert_eq!(recents_read_plan::<PollRecents>(Ok(b"{oops".to_vec())), ReadPlan::Unparseable);
        assert_eq!(recents_read_plan::<PollRecents>(Ok(b"{}".to_vec())), ReadPlan::Parsed(PollRecents::default()));
    }

    #[test]
    fn backups_carry_the_recents() {
        let mut v = json!({ "theme": "dark" });
        let r = PollRecents { polls: vec![RecentPoll { title: "Q".into(), options: vec![] }], predictions: vec![] };
        with_poll_recents(&mut v, &r);
        assert_eq!(v["recent_polls"][0]["title"], "Q");
        assert_eq!(v["recent_predictions"], json!([]));
        assert_eq!(v["theme"], "dark");
    }

    #[test]
    fn a_payout_picks_by_place_or_title() {
        let o = |id: &str, t: &str| Outcome { id: id.into(), title: t.into(), color: "blue".into() };
        let list = [o("a", "Yes"), o("b", "No")];
        assert_eq!(pick_outcome(&list, "2"), Ok("b".to_string()));
        assert_eq!(pick_outcome(&list, " yes "), Ok("a".to_string()));
        assert!(pick_outcome(&list, "3").is_err());
        assert!(pick_outcome(&list, "").is_err());
    }

    /// The source outside its tests, line ends folded.
    fn source_without_tests(src: &str) -> String {
        let src = src.replace("\r\n", "\n");
        let end = src.find("#[cfg(test)]\nmod tests").unwrap_or(src.len());
        src[..end].to_string()
    }

    #[test]
    fn every_call_goes_out_as_the_runs_account() {
        let src = source_without_tests(include_str!("stream_manager.rs"));
        // `impl Session` is the one place that reads the credentials.
        let start = src.find("impl Session {").expect("impl Session");
        let end = start + src[start..].find("\n}\n").expect("end of impl Session");
        let outside = format!("{}{}", &src[..start], &src[end..]);
        for banned in [
            "get_token(",
            "get_token_with_generation(",
            "web_session_token(",
            "get_user_info(",
            "TwitchService::get_active_poll(",
            "TwitchService::get_open_prediction(",
            "TwitchService::finish_open_prediction(",
            "TwitchService::create_poll(",
            "TwitchService::end_poll(",
            "TwitchService::create_prediction(",
            "TwitchService::end_prediction(",
            "TwitchService::start_raid(",
            "TwitchService::cancel_raid(",
            "TwitchService::start_commercial(",
            "TwitchService::update_chat_settings(",
            "TwitchService::update_shield_mode(",
            "TwitchService::send_chat_announcement(",
            "twitch_web_gql::post(",
        ] {
            assert!(!outside.contains(banned), "{banned} outside Session reads its own credentials");
        }
        // The token twins read nothing of their own either.
        let service = include_str!("twitch_service.rs").replace("\r\n", "\n");
        let twins = [
            "create_poll", "end_poll", "create_prediction", "end_prediction", "start_raid", "cancel_raid",
            "start_commercial", "update_chat_settings", "update_shield_mode", "send_chat_announcement",
        ];
        for twin in twins {
            let sig = format!("pub(crate) async fn {twin}_as(");
            let at = service.find(&sig).unwrap_or_else(|| panic!("{sig} not found"));
            let body = &service[at..at + service[at..].find("\n    }\n").expect("end of twin")];
            assert!(!body.contains("get_token"), "{twin}_as reads a token");
            assert!(!body.contains("get_user_info("), "{twin}_as reads the account");
            for other in twins {
                assert!(!body.contains(&format!("Self::{other}(")), "{twin}_as calls {other}");
            }
        }
    }

    #[test]
    fn a_view_takes_its_revision_first_and_every_write_checks_the_account() {
        let src = source_without_tests(include_str!("stream_manager.rs"));
        let at = src.find("pub async fn view(").expect("view");
        let body = &src[at..];
        let first = body[body.find('{').unwrap() + 1..].trim_start();
        assert!(first.starts_with("let rev = next_rev();"), "view must take its revision before anything");
        // Every write in `run` and `save_info` goes after a current-account check.
        let run = &src[src.find("async fn run(").unwrap()..];
        let run = &run[..run.find("\n}\n").unwrap()];
        let save = &src[src.find("async fn save_info(").unwrap()..];
        let save = &save[..save.find("\n}\n").unwrap()];
        let mut writes = 0;
        for part in [run, save] {
            for write in ["_as(token", "gql_body(auth, ", "reqwest::Method::PATCH", "reqwest::Method::POST"] {
                let mut from = 0;
                while let Some(i) = part[from..].find(write) {
                    let at = from + i;
                    let before = &part[..at];
                    // The nearest thing before a write that starts a new branch
                    // or is the check: it must be the check.
                    let nearest = ["still_current()?", "=> {", "if broadcast {", "if with_tags {"]
                        .iter()
                        .filter_map(|m| before.rfind(m).map(|p| (p, *m)))
                        .max();
                    assert_eq!(nearest.map(|(_, m)| m), Some("still_current()?"), "the write at {at} ({write}) has no account check right before it");
                    writes += 1;
                    from = at + write.len();
                }
            }
        }
        assert!(writes >= 18, "found only {writes} writes; the pin no longer sees them");
        // The background reads are paced by the pure rules.
        assert!(src.contains("now >= next_publish_at(f)"));
        assert!(src.contains("if !should_arm(f.armed_rev, view.rev)"));
    }

    /// The body of the first function whose signature starts with `sig`.
    fn fn_body<'a>(src: &'a str, sig: &str) -> &'a str {
        let at = src.find(sig).unwrap_or_else(|| panic!("{sig} not found"));
        let rest = &src[at..];
        &rest[..rest.find("\n}\n").or_else(|| rest.find("\n    }\n")).expect("end of function")]
    }

    #[test]
    fn the_wiring_is_in_place() {
        let src = source_without_tests(include_str!("stream_manager.rs"));
        // Kept flags never outrank chat's live answer.
        let role = fn_body(&src, "async fn role_in(");
        assert!(role.contains("return role_from(false, known, None, Some(f));"));
        // A started poll or prediction joins the recents; a failed read slows down.
        assert!(fn_body(&src, "pub async fn act(").contains("remember_poll(*kind, title, &options);"));
        assert!(fn_body(&src, "async fn publish(").contains("disarm(f);"));
        let view = fn_body(&src, "async fn view_as(");
        assert!(view.find("arm(&view);\n        return Ok((view, None));").is_some(), "a viewer's view clears the deadline");
        // Pubsub, page loads and closed windows reach the manager.
        let pubsub = include_str!("channel_points_websocket_service.rs").replace("\r\n", "\n");
        assert!(pubsub.contains("crate::services::stream_manager::pubsub_event(topic_type, cid, &message_data);"));
        let lib = include_str!("../lib.rs").replace("\r\n", "\n");
        let load = &lib[lib.find(".on_page_load(").unwrap()..];
        assert!(load[..load.find("if webview.label() == \"main\"").unwrap()].contains("services::stream_manager::clear_label(webview.label());"));
        let destroyed = &lib[lib.find("if let WindowEvent::Destroyed = event {\n                services::app_exit").unwrap()..];
        assert!(destroyed[..400].contains("services::stream_manager::clear_label(&label);"));
        // The settings' old lists carry over before the first read or write.
        let command = include_str!("../commands/stream_manager.rs").replace("\r\n", "\n");
        assert!(command.contains("stream_manager::seed_poll_recents(&extra);"));
        for cmd in ["pub async fn get_stream_manager(", "pub async fn stream_manager_act("] {
            let body = &command[command.find(cmd).unwrap()..];
            let body = &body[..body.find("\n}\n").unwrap()];
            assert!(body.contains("seed_recents(&state);"), "{cmd} reads before the carry-over");
        }
    }

    #[test]
    fn a_failed_read_goes_back_to_the_periodic_pace() {
        let mut f = Freshness { last_ms: 10_000, reread: Some(1_000), ..Freshness::default() };
        // Long past: without a view to move it, every tick would be 3 s apart.
        assert_eq!(next_publish_at(&f), 13_000);
        disarm(&mut f);
        assert_eq!(next_publish_at(&f), 40_000);
    }

    #[test]
    fn raid_targets_live_first_without_self() {
        use crate::models::stream::TwitchStream;
        let s = |id: &str, live: bool, game: &str| TwitchStream {
            id: id.into(), user_id: id.into(), user_name: id.into(), user_login: id.into(), title: String::new(),
            viewer_count: 0, game_id: String::new(), game_name: game.into(), thumbnail_url: String::new(),
            started_at: String::new(), broadcaster_type: None, profile_image_url: None, is_live: Some(live),
            tags: None, language: None, drops_earnable: false,
        };
        let t = raid_targets(&[s("off", false, ""), s("me", true, "x"), s("on", true, "Valorant")], "me");
        assert_eq!(t.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["on", "off"]);
        assert_eq!(t[0].line, "Valorant");
        assert_eq!(t[1].line, "Offline");
    }

    #[test]
    fn a_raid_countdown_expires() {
        set_raid("c1", Some(RaidView { target_id: "t".into(), target_login: "t".into(), target_name: "T".into(), deadline_ms: 100, total_ms: 100 }));
        assert!(pending_raid("c1", 50).is_some());
        assert!(pending_raid("c1", 150).is_none());
        assert!(pending_raid("c1", 50).is_none());
    }

    #[test]
    fn actions_parse_from_the_shells() {
        let a: Action = serde_json::from_value(json!({ "kind": "redemptions", "ids": ["1"], "fulfill": true })).unwrap();
        assert_eq!(a.section(), Section::Rewards);
        let c: Action = serde_json::from_value(json!({ "kind": "chat_mode", "mode": "slow", "on": true, "value": 30 })).unwrap();
        assert_eq!(c.section(), Section::Chat);
        let s: Action = serde_json::from_value(json!({ "kind": "save_info", "draft": { "title": "t" }, "fields": ["title", "tags"] })).unwrap();
        assert_eq!(s.section(), Section::Info);
        match s {
            Action::SaveInfo { fields, .. } => assert_eq!(fields, vec![InfoField::Title, InfoField::Tags]),
            _ => unreachable!(),
        }
        // A save always names its fields.
        assert!(serde_json::from_value::<Action>(json!({ "kind": "save_info", "draft": { "title": "t" } })).is_err());
        let pick: Action = serde_json::from_value(json!({ "kind": "resolve_prediction_pick", "pick": "2" })).unwrap();
        assert_eq!(pick.section(), Section::Prediction);
        let p: Action = serde_json::from_value(json!({ "kind": "lock_prediction" })).unwrap();
        assert_eq!(p.section(), Section::Prediction);
    }

    #[test]
    fn languages_have_names() {
        assert_eq!(language_name("EN"), Some("English"));
        assert_eq!(language_name("zh-hk"), Some("Cantonese"));
        assert_eq!(language_name("xx"), None);
        assert_eq!(ago_words(0, 3_600_000 * 3), "3 h ago");
    }
}
