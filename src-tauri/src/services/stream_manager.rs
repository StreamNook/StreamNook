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
use std::sync::Mutex;

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::services::twitch_auth_service::TwitchAuthService;
use crate::services::twitch_service::{helix_client_id, helix_refusal_text, lacks_scope, TwitchService};
use crate::services::twitch_web_gql;

const LOG_TAG: &str = "StreamManager";

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
    pub channel_id: String,
    pub channel_login: String,
    pub role: Role,
    /// "You are a moderator here" and the like, for the sheet's header.
    pub role_line: String,
    /// The rows of the Stream group, in order. Rows the role cannot use are
    /// left out, so no row ever opens onto a refusal.
    pub sections: Vec<SectionRow>,
    pub info: Option<StreamInfoView>,
    pub poll: Option<RunningPoll>,
    pub prediction: Option<RunningPrediction>,
    pub raid: Option<RaidView>,
    pub ads: Option<AdsView>,
    pub chat: Option<ChatView>,
    /// Redemptions waiting, when the queue was read.
    pub queue_count: Option<usize>,
    pub limits: FormLimits,
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
    pub title_count: String,
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
    InfoCheck {
        ok: problems.is_empty(),
        problems,
        title_count: format!("{title_len} / {TITLE_MAX}"),
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
    }
}

// ─────────────────────────────── polls and predictions ───────────────────────

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RunningPoll {
    pub id: String,
    pub title: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RunningPrediction {
    pub id: String,
    pub title: String,
    /// "active" (taking predictions) or "locked" (waiting for the result).
    pub status: String,
    pub outcomes: Vec<Outcome>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Outcome {
    pub id: String,
    pub title: String,
    pub color: String,
}

/// A Helix poll that is still running.
pub fn running_poll_helix(p: &Value) -> Option<RunningPoll> {
    (p["status"].as_str() == Some("ACTIVE")).then(|| RunningPoll {
        id: p["id"].as_str().unwrap_or_default().to_string(),
        title: p["title"].as_str().unwrap_or_default().to_string(),
    })
}

/// A Helix prediction that is open or locked.
pub fn running_prediction_helix(p: &Value) -> Option<RunningPrediction> {
    let status = match p["status"].as_str()? {
        "ACTIVE" => "active",
        "LOCKED" => "locked",
        _ => return None,
    };
    Some(RunningPrediction {
        id: p["id"].as_str().unwrap_or_default().to_string(),
        title: p["title"].as_str().unwrap_or_default().to_string(),
        status: status.into(),
        outcomes: p["outcomes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|o| Outcome {
                        id: o["id"].as_str().unwrap_or_default().to_string(),
                        title: o["title"].as_str().unwrap_or_default().to_string(),
                        color: o["color"].as_str().unwrap_or("BLUE").to_ascii_lowercase(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// The GQL answer to RUNNING_QUERY: the latest poll while ACTIVE, and the
/// first active, else locked, prediction.
pub fn running_from_gql(body: &Value) -> (Option<RunningPoll>, Option<RunningPrediction>) {
    let poll = body["data"]["user"]["latestPoll"].as_object().and_then(|p| {
        (p.get("status").and_then(Value::as_str) == Some("ACTIVE")).then(|| RunningPoll {
            id: p.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
            title: p.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
        })
    });
    let pick = |key: &str, status: &str| -> Option<RunningPrediction> {
        let list = body["data"]["channel"][key].as_array()?;
        let p = list.first()?;
        Some(RunningPrediction {
            id: p["id"].as_str().unwrap_or_default().to_string(),
            title: p["title"].as_str().unwrap_or_default().to_string(),
            status: status.into(),
            outcomes: p["outcomes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|o| Outcome {
                            id: o["id"].as_str().unwrap_or_default().to_string(),
                            title: o["title"].as_str().unwrap_or_default().to_string(),
                            color: o["color"].as_str().unwrap_or("BLUE").to_ascii_lowercase(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    };
    let prediction = pick("activePredictionEvents", "active").or_else(|| pick("lockedPredictionEvents", "locked"));
    (poll, prediction)
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

/// Helix `GET channels/ads` to the schedule. Times there are unix seconds
/// (0 when unset).
pub fn ad_schedule(row: &Value, now_ms: i64) -> AdSchedule {
    let secs = |k: &str| -> i64 {
        row[k]
            .as_i64()
            .or_else(|| row[k].as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0)
    };
    let at = |k: &str| -> Option<i64> { Some(secs(k)).filter(|s| *s > 0).map(|s| s * 1000) };
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
                image_url: r["image"]["url"].as_str().or(r["defaultImage"]["url"].as_str()).map(str::to_string),
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
        empty_line: (count == 0).then(|| "Nothing waiting".to_string()),
        more: conn["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false),
        groups,
        count,
    }
}

// ─────────────────────────────── GQL documents ───────────────────────────────

pub const QUEUE_QUERY: &str = "query StreamManagerQueue($channelID: ID!) { channel(id: $channelID) { communityPointsRedemptionQueue(first: 50, options: { order: OLDEST, status: UNFULFILLED }) { edges { node { id status input timestamp rewardTitle user { id login displayName profileImageURL(width: 50) } reward { id title cost isPaused isEnabled backgroundColor image { url } defaultImage { url } } } } pageInfo { hasNextPage } } } }";
pub const ROLE_QUERY: &str = "query StreamManagerRole($channelID: ID!) { user(id: $channelID) { self { isEditor isModerator } } }";
pub const RUNNING_QUERY: &str = "query StreamManagerRunning($channelID: ID!) { user(id: $channelID) { latestPoll { id status title } } channel(id: $channelID) { activePredictionEvents { id status title outcomes { id title color } } lockedPredictionEvents { id status title outcomes { id title color } } } }";
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
    SaveInfo { draft: StreamInfoDraft },
    CreatePoll { title: String, choices: Vec<String>, seconds: u32, #[serde(default)] points_per_vote: Option<u32> },
    /// End the running poll; `show_results` keeps it on screen, else it is removed.
    EndPoll { show_results: bool },
    CreatePrediction { title: String, outcomes: Vec<String>, seconds: u32 },
    LockPrediction,
    CancelPrediction,
    ResolvePrediction { outcome_id: String },
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
            | Action::ResolvePrediction { .. } => Section::Prediction,
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
}

// ─────────────────────────────── network ─────────────────────────────────────

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// One Helix call as the signed-in account: (status, body).
async fn helix(method: reqwest::Method, url: &str, body: Option<Value>) -> Result<(u16, String), String> {
    let token = TwitchService::get_token().await.map_err(|_| "Sign in to Twitch first".to_string())?;
    let mut req = crate::services::http::client()
        .request(method, url)
        .header("Client-Id", helix_client_id())
        .header("Authorization", format!("Bearer {token}"));
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(|e| format!("Could not reach Twitch: {e}"))?;
    let status = resp.status().as_u16();
    Ok((status, resp.text().await.unwrap_or_default()))
}

async fn helix_get(url: &str) -> Result<Value, String> {
    let (status, text) = helix(reqwest::Method::GET, url, None).await?;
    if !(200..300).contains(&status) {
        return Err(helix_refusal_text(status, &text));
    }
    serde_json::from_str(&text).map_err(|_| "Twitch sent an unreadable answer".to_string())
}

async fn gql(auth: &TwitchAuthService, body: Value) -> Result<Value, String> {
    let token = twitch_web_gql::web_session_token(auth).await?;
    let answer = twitch_web_gql::post(Some(&token), body.clone(), LOG_TAG).await?;
    // Some writes refuse a call without Twitch's security token: ask again
    // with one where the app can mint it.
    let integrity = answer["errors"]
        .as_array()
        .map(|e| e.iter().any(|x| x["message"].as_str().unwrap_or("").to_ascii_lowercase().contains("integrity")))
        .unwrap_or(false);
    if integrity {
        if let Ok(i) = crate::services::twitch_integrity::get().await {
            return twitch_web_gql::post_with(Some(&token), body, LOG_TAG, Some(&i)).await;
        }
    }
    Ok(answer)
}

/// The signed-in account's role in `channel_id`.
async fn role_in(auth: &TwitchAuthService, channel_id: &str, channel_login: &str, own_id: &str) -> Role {
    if channel_id == own_id {
        return Role::Broadcaster;
    }
    let chat_mod = matches!(
        crate::services::chat_send_limiter::budget(channel_login).role,
        crate::services::chat_send_limiter::Role::Moderator | crate::services::chat_send_limiter::Role::Broadcaster
    );
    let flags = gql(auth, op("StreamManagerRole", ROLE_QUERY, json!({ "channelID": channel_id })))
        .await
        .ok()
        .map(|b| b["data"]["user"]["self"].clone())
        .filter(|s| s.is_object());
    let editor = flags.as_ref().and_then(|s| s["isEditor"].as_bool()).unwrap_or(false);
    let moderator = flags.as_ref().and_then(|s| s["isModerator"].as_bool()).unwrap_or(false) || chat_mod;
    Role::from_flags(false, editor, moderator)
}

async fn read_info(channel_id: &str) -> Result<StreamInfoView, String> {
    let v = helix_get(&format!("https://api.twitch.tv/helix/channels?broadcaster_id={channel_id}")).await?;
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

async fn read_running(auth: &TwitchAuthService, channel_id: &str, role: Role) -> (Option<RunningPoll>, Option<RunningPrediction>) {
    if helix_route(role) {
        let poll = TwitchService::get_active_poll(channel_id).await.ok().flatten().and_then(|p| running_poll_helix(&p));
        let prediction = TwitchService::get_open_prediction(channel_id)
            .await
            .ok()
            .flatten()
            .and_then(|p| running_prediction_helix(&p));
        return (poll, prediction);
    }
    match gql(auth, op("StreamManagerRunning", RUNNING_QUERY, json!({ "channelID": channel_id }))).await {
        Ok(body) => running_from_gql(&body),
        Err(_) => (None, None),
    }
}

async fn read_ads(channel_id: &str) -> AdsView {
    let token = TwitchService::get_token().await.ok();
    let granted = match &token {
        Some(t) => TwitchService::token_scopes(t).await.unwrap_or_default(),
        None => Vec::new(),
    };
    if lacks_scope(&granted, "channel:read:ads") {
        return AdsView { lengths: ad_lengths(), schedule: None, schedule_hint: Some(ADS_SCOPE_HINT.into()) };
    }
    let schedule = helix_get(&format!("https://api.twitch.tv/helix/channels/ads?broadcaster_id={channel_id}"))
        .await
        .ok()
        .map(|v| ad_schedule(&v["data"][0], now_ms()));
    AdsView { lengths: ad_lengths(), schedule, schedule_hint: None }
}

async fn read_chat(channel_id: &str, own_id: &str) -> Result<ChatView, String> {
    let settings = helix_get(&format!(
        "https://api.twitch.tv/helix/chat/settings?broadcaster_id={channel_id}&moderator_id={own_id}"
    ))
    .await?;
    let shield = helix_get(&format!(
        "https://api.twitch.tv/helix/moderation/shield_mode?broadcaster_id={channel_id}&moderator_id={own_id}"
    ))
    .await
    .ok()
    .and_then(|v| v["data"][0]["is_active"].as_bool());
    Ok(chat_view(&settings["data"][0], shield))
}

pub async fn read_queue(auth: &TwitchAuthService, channel_id: &str) -> Result<QueueView, String> {
    let body = gql(auth, op("StreamManagerQueue", QUEUE_QUERY, json!({ "channelID": channel_id }))).await?;
    if let Some(e) = body["errors"].as_array().filter(|e| !e.is_empty()) {
        return Err(format!(
            "Could not read the rewards queue: {}",
            gql_error_words(e[0]["message"].as_str().unwrap_or("error"))
        ));
    }
    Ok(parse_queue(&body, now_ms()))
}

/// Everything the manager shows for `channel_id`. Parts that fail say so in
/// `problems`; the rest still draws.
pub async fn view(auth: &TwitchAuthService, channel_id: &str, channel_login: &str) -> Result<StreamManagerView, String> {
    let me = TwitchService::get_user_info().await.map_err(|_| "Sign in to Twitch first".to_string())?;
    let role = role_in(auth, channel_id, channel_login, &me.id).await;
    let mut problems = Vec::new();
    let empty = || StreamManagerView {
        channel_id: channel_id.to_string(),
        channel_login: channel_login.to_string(),
        role,
        role_line: role_line(role),
        sections: Vec::new(),
        info: None,
        poll: None,
        prediction: None,
        raid: None,
        ads: None,
        chat: None,
        queue_count: None,
        limits: form_limits(),
        problems: Vec::new(),
    };
    if !role.manages() {
        return Ok(empty());
    }
    let wants = |s: Section| can_use(role, s).is_ok();
    let (info, running, ads, chat, queue) = tokio::join!(
        async { if wants(Section::Info) { Some(read_info(channel_id).await) } else { None } },
        async {
            if wants(Section::Poll) { read_running(auth, channel_id, role).await } else { (None, None) }
        },
        async { if wants(Section::Ads) { Some(read_ads(channel_id).await) } else { None } },
        async { if wants(Section::Chat) { Some(read_chat(channel_id, &me.id).await) } else { None } },
        async { if wants(Section::Rewards) { Some(read_queue(auth, channel_id).await) } else { None } },
    );
    let info = match info {
        Some(Ok(i)) => Some(i),
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
    let queue_count = match queue {
        Some(Ok(q)) => Some(q.count),
        Some(Err(e)) => {
            problems.push(e);
            None
        }
        None => None,
    };
    let now = now_ms();
    let raid = if wants(Section::Raid) { pending_raid(channel_id, now) } else { None };
    let (poll, prediction) = running;
    let sections = section_rows(role, info.as_ref(), poll.as_ref(), prediction.as_ref(), raid.as_ref(), ads.as_ref(), queue_count, now);
    Ok(StreamManagerView { sections, info, poll, prediction, raid, ads, chat, queue_count, problems, ..empty() })
}

/// The info form's check, with the channel's name, picture and live
/// thumbnail filled in for the preview.
pub async fn check(channel_id: &str, draft: &StreamInfoDraft) -> InfoCheck {
    let current = read_info(channel_id).await.ok();
    let users = helix_get(&format!("https://api.twitch.tv/helix/users?id={channel_id}")).await.ok();
    let user = users.as_ref().map(|u| u["data"][0].clone()).unwrap_or(Value::Null);
    let name = user["display_name"].as_str().unwrap_or_default().to_string();
    let avatar = user["profile_image_url"].as_str().map(str::to_string);
    let thumb = helix_get(&format!("https://api.twitch.tv/helix/streams?user_id={channel_id}"))
        .await
        .ok()
        .and_then(|s| s["data"][0]["thumbnail_url"].as_str().map(|t| box_art(t, 440, 248)));
    check_stream_info(draft, current.as_ref(), &name, avatar, thumb)
}

pub async fn search_raid_targets(state: &crate::models::settings::AppState, query: &str, channel_id: &str) -> Result<Vec<RaidTarget>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let streams = TwitchService::search_channels(state, q).await.map_err(|e| e.to_string())?;
    Ok(raid_targets(&streams, channel_id).into_iter().take(12).collect())
}

/// Run one action as the signed-in account, then read the view again.
pub async fn act(auth: &TwitchAuthService, channel_id: &str, channel_login: &str, action: Action) -> ActOutcome {
    let me = match TwitchService::get_user_info().await {
        Ok(m) => m,
        Err(_) => return ActOutcome { ok: false, message: "Sign in to Twitch first".into(), view: None, queue: None },
    };
    let role = role_in(auth, channel_id, channel_login, &me.id).await;
    if let Err(why) = can_use(role, action.section()) {
        return ActOutcome { ok: false, message: why.into(), view: None, queue: None };
    }
    let queue_write = action.section() == Section::Rewards;
    let result = run(auth, channel_id, role, action).await;
    let queue = if queue_write { read_queue(auth, channel_id).await.ok() } else { None };
    let view = view(auth, channel_id, channel_login).await.ok();
    match result {
        Ok(message) => ActOutcome { ok: true, message, view, queue },
        Err(message) => ActOutcome { ok: false, message, view, queue },
    }
}

async fn run(auth: &TwitchAuthService, channel_id: &str, role: Role, action: Action) -> Result<String, String> {
    let helix_ok = helix_route(role);
    let words = |e: anyhow::Error| e.to_string();
    match action {
        Action::SaveInfo { draft } => {
            let check = check_stream_info(&draft, None, "", None, None);
            if let Some(p) = check.problems.first() {
                return Err(p.line.clone());
            }
            let tags = check.preview.tags;
            if helix_ok {
                let mut body = json!({ "title": draft.title.trim(), "tags": tags });
                if let Some(c) = &draft.category {
                    body["game_id"] = json!(c.id);
                }
                if !draft.language.is_empty() {
                    body["broadcaster_language"] = json!(draft.language);
                }
                let (status, text) = helix(
                    reqwest::Method::PATCH,
                    &format!("https://api.twitch.tv/helix/channels?broadcaster_id={channel_id}"),
                    Some(body),
                )
                .await?;
                if !(200..300).contains(&status) {
                    return Err(helix_refusal_text(status, &text));
                }
            } else {
                gql_outcome(&gql(auth, broadcast_request(channel_id, &draft)).await?, "updateBroadcastSettings", "Could not save the stream info")?;
                gql_outcome(&gql(auth, tags_request(channel_id, &tags)).await?, "setFreeformTags", "Saved, but the tags did not go through")?;
            }
            Ok("Stream info saved".into())
        }
        Action::CreatePoll { title, choices, seconds, points_per_vote } => {
            let choices = check_poll(&title, &choices, seconds)?;
            if helix_ok {
                TwitchService::create_poll(channel_id, title.trim(), &choices, seconds, points_per_vote).await.map_err(words)?;
            } else {
                gql_outcome(&gql(auth, create_poll_request(channel_id, &title, &choices, seconds, points_per_vote)).await?, "createPoll", "Could not start the poll")?;
            }
            Ok("Poll started".into())
        }
        Action::EndPoll { show_results } => {
            let (poll, _) = read_running(auth, channel_id, role).await;
            let poll = poll.ok_or("No poll is running right now")?;
            if helix_ok {
                let status = if show_results { "TERMINATED" } else { "ARCHIVED" };
                TwitchService::end_poll(channel_id, &poll.id, status).await.map_err(words)?;
            } else {
                let (name, query, field) = if show_results {
                    ("StreamManagerEndPoll", TERMINATE_POLL_MUTATION, "terminatePoll")
                } else {
                    ("StreamManagerArchivePoll", ARCHIVE_POLL_MUTATION, "archivePoll")
                };
                gql_outcome(&gql(auth, op(name, query, json!({ "input": { "pollID": poll.id } }))).await?, field, "Could not end the poll")?;
            }
            Ok(if show_results { "Poll ended" } else { "Poll removed" }.into())
        }
        Action::CreatePrediction { title, outcomes, seconds } => {
            let outcomes = check_prediction(&title, &outcomes, seconds)?;
            if helix_ok {
                TwitchService::create_prediction(channel_id, title.trim(), &outcomes, seconds).await.map_err(words)?;
            } else {
                gql_outcome(&gql(auth, create_prediction_request(channel_id, &title, &outcomes, seconds)).await?, "createPredictionEvent", "Could not start the prediction")?;
            }
            Ok("Prediction started".into())
        }
        Action::LockPrediction | Action::CancelPrediction | Action::ResolvePrediction { .. } => {
            if helix_ok {
                let (end, outcome) = match &action {
                    Action::LockPrediction => (crate::services::twitch_service::PredictionEnd::Lock, None),
                    Action::CancelPrediction => (crate::services::twitch_service::PredictionEnd::Cancel, None),
                    Action::ResolvePrediction { outcome_id } => {
                        (crate::services::twitch_service::PredictionEnd::Complete, Some(outcome_id.as_str()))
                    }
                    _ => unreachable!(),
                };
                let open = TwitchService::get_open_prediction(channel_id)
                    .await
                    .map_err(words)?
                    .and_then(|p| running_prediction_helix(&p))
                    .ok_or("No prediction is running right now")?;
                // finish_open_prediction picks a winner by number or title.
                let pick = outcome.and_then(|id| open.outcomes.iter().position(|o| o.id == id)).map(|i| (i + 1).to_string());
                return TwitchService::finish_open_prediction(channel_id, end, pick.as_deref()).await;
            }
            let (_, open) = read_running(auth, channel_id, role).await;
            let open = open.ok_or("No prediction is running right now")?;
            let (name, query, field, input, done) = match &action {
                Action::LockPrediction => ("StreamManagerLockPrediction", LOCK_PREDICTION_MUTATION, "lockPredictionEvent", json!({ "id": open.id }), "Prediction locked"),
                Action::CancelPrediction => ("StreamManagerCancelPrediction", CANCEL_PREDICTION_MUTATION, "cancelPredictionEvent", json!({ "id": open.id }), "Prediction canceled, points refunded"),
                Action::ResolvePrediction { outcome_id } => ("StreamManagerResolvePrediction", RESOLVE_PREDICTION_MUTATION, "resolvePredictionEvent", json!({ "eventID": open.id, "outcomeID": outcome_id }), "Prediction paid out"),
                _ => unreachable!(),
            };
            gql_outcome(&gql(auth, op(name, query, json!({ "input": input }))).await?, field, "That did not go through")?;
            Ok(done.into())
        }
        Action::Raid { target_id, target_login, target_name } => {
            TwitchService::start_raid(channel_id, &target_id).await.map_err(words)?;
            set_raid(
                channel_id,
                Some(RaidView {
                    target_id,
                    target_login,
                    target_name: target_name.clone(),
                    deadline_ms: now_ms() + RAID_COUNTDOWN_MS,
                }),
            );
            Ok(format!("Raiding {target_name}"))
        }
        Action::CancelRaid => {
            TwitchService::cancel_raid(channel_id).await.map_err(words)?;
            set_raid(channel_id, None);
            Ok("Raid canceled".into())
        }
        Action::RaidNow => {
            let body = gql(auth, crate::services::slash_commands::go_raid_request(channel_id)).await?;
            crate::services::slash_commands::go_raid_outcome(&body)?;
            set_raid(channel_id, None);
            Ok("Raid sent".into())
        }
        Action::RunAd { seconds } => {
            if !AD_LENGTHS.contains(&seconds) {
                return Err("Pick a length from the list".into());
            }
            TwitchService::start_commercial(channel_id, seconds).await.map_err(words)?;
            Ok(format!("Ad break started ({})", ad_lengths().into_iter().find(|c| c.value == seconds).map(|c| c.label).unwrap_or_default()))
        }
        Action::SnoozeAd => {
            let token = TwitchService::get_token().await.map_err(words)?;
            let granted = TwitchService::token_scopes(&token).await.map_err(words)?;
            if lacks_scope(&granted, "channel:manage:ads") {
                return Err(ADS_SCOPE_HINT.into());
            }
            let (status, text) = helix(
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
            gql_outcome(
                &gql(auth, redemptions_request(channel_id, &ids, fulfill)).await?,
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
            gql_outcome(
                &gql(auth, reward_queue_request(channel_id, &reward_id, fulfill)).await?,
                "updateCommunityPointsCustomRewardRedemptionStatusesByReward",
                "That did not go through",
            )?;
            Ok(if fulfill { "Completed all" } else { "Rejected all, points refunded" }.into())
        }
        Action::Reward { reward_id, paused, enabled } => {
            if paused.is_none() && enabled.is_none() {
                return Err("Nothing to change".into());
            }
            gql_outcome(
                &gql(auth, reward_request(channel_id, &reward_id, paused, enabled)).await?,
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
            TwitchService::update_chat_settings(channel_id, body).await.map_err(words)?;
            Ok("Chat updated".into())
        }
        Action::Shield { on } => {
            TwitchService::update_shield_mode(channel_id, on).await.map_err(words)?;
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
            TwitchService::send_chat_announcement(channel_id, text, color.as_deref()).await.map_err(words)?;
            Ok("Announced".into())
        }
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
        let poll = RunningPoll { id: "p".into(), title: "Which boss?".into() };
        let rows = section_rows(Role::Broadcaster, None, Some(&poll), None, None, None, Some(0), 0);
        let row = rows.iter().find(|r| r.id == Section::Poll).unwrap();
        assert_eq!(row.label, "Poll running");
        assert_eq!(row.detail.as_deref(), Some("Which boss?"));
        // An empty queue has no count.
        assert!(rows.iter().find(|r| r.id == Section::Rewards).unwrap().badge.is_none());
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
                            "backgroundColor": "#00C7AC", "image": null, "defaultImage": { "url": "https://d" } }
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
        assert_eq!(q.groups[0].image_url.as_deref(), Some("https://d"));
        assert!(q.empty_line.is_none());
        let empty = parse_queue(&json!({ "data": { "channel": { "communityPointsRedemptionQueue": { "edges": [] } } } }), now);
        assert_eq!(empty.empty_line.as_deref(), Some("Nothing waiting"));
    }

    #[test]
    fn ad_schedule_in_words() {
        let now = 1_000_000_000_000i64;
        let row = json!({ "next_ad_at": (now / 1000) + 14 * 60, "duration": 60, "preroll_free_time": 1380,
                          "snooze_count": 2, "snooze_refresh_at": 0, "last_ad_at": 0 });
        let s = ad_schedule(&row, now);
        assert_eq!(s.snooze_line, "2 snoozes left");
        assert_eq!(s.preroll_line.as_deref(), Some("Pre-rolls are off for 23 min"));
        let ads = AdsView { lengths: ad_lengths(), schedule: Some(s), schedule_hint: None };
        assert_eq!(ads_detail(&ads, now).as_deref(), Some("Next ad in 14 min · 2 snoozes left"));
        let none = ad_schedule(&json!({ "snooze_count": "0", "snooze_refresh_at": (now / 1000) + 42 * 60 }), now);
        assert_eq!(none.snooze_line, "No snoozes left, one comes back in 42 min");
        assert_eq!(none.next_ad_ms, None);
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
        let (poll, pred) = running_from_gql(&body);
        assert_eq!(poll.unwrap().id, "q");
        let pred = pred.unwrap();
        assert_eq!(pred.status, "locked");
        assert_eq!(pred.outcomes[0].color, "blue");
        let ended = json!({ "data": { "user": { "latestPoll": { "id": "q", "status": "COMPLETED", "title": "x" } }, "channel": {} } });
        assert!(running_from_gql(&ended).0.is_none());
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
        set_raid("c1", Some(RaidView { target_id: "t".into(), target_login: "t".into(), target_name: "T".into(), deadline_ms: 100 }));
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
        let s: Action = serde_json::from_value(json!({ "kind": "save_info", "draft": { "title": "t" } })).unwrap();
        assert_eq!(s.section(), Section::Info);
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
