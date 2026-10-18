//! The phones' badge wall (Rewards tab, Badges segment): Twitch's global
//! badge catalogue with the account's own badges marked, each tile's earn
//! status, its sheet copy, the sort, and the "Missing, earnable now" strip,
//! all worked out here from the badge standing (services/badge_standing.rs)
//! and the badge metadata cache. The phones only draw it.
//!
//! Once per app session the metadata Twitch's catalogue lacks (dates, usage,
//! copy) is fetched in the background, five badges at a time; `badge-wall-sync`
//! carries how many are left and `badge-standing-changed` follows the last.

use crate::commands::badges::HelixBadgesResponse;
use crate::services::badge_earn::{EarnPath, EarnStep};
use crate::services::badge_standing::{BadgeStanding, CatchUp, CollectionState, MissingBadge};
use crate::services::badge_window::{self, WindowRun, WindowStatus};
use crate::services::universal_cache_service::{self, CacheType, UniversalCacheEntry};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};

pub const SYNC_EVENT: &str = "badge-wall-sync";

/// How the missing strip starts: the first rows, the rest behind "Show all".
pub const MISSING_COLLAPSED: usize = 4;

static BACKFILL_STARTED: AtomicBool = AtomicBool::new(false);
static SYNC_LEFT: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Newest,
    Available,
    Soon,
    Usage,
    Oldest,
}

impl Sort {
    pub fn parse(name: &str) -> Self {
        match name {
            "available" => Sort::Available,
            "soon" => Sort::Soon,
            "usage" => Sort::Usage,
            "oldest" => Sort::Oldest,
            _ => Sort::Newest,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BadgeWall {
    /// No tiles yet and the catalogue is still on its way.
    pub loading: bool,
    /// In the asked sort.
    pub tiles: Vec<BadgeTile>,
    /// "12 of 340 collected": the account's own badges, not a user count.
    pub count_label: String,
    /// Badges whose metadata is still being fetched; 0 when idle.
    pub sync_left: u32,
    pub missing: MissingNow,
    /// The next instant a window opens or closes: ask again then.
    pub next_change_ms: Option<i64>,
}

/// A run of text, or an instant the phone writes in its own time zone.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TextPart {
    pub text: Option<String>,
    pub ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BadgeTile {
    /// `set-version` of the version the tile shows.
    pub key: String,
    /// Every `set/version` merged into this tile; owning any owns the tile.
    pub keys: Vec<String>,
    pub title: String,
    /// 4x art, else 2x, as served.
    pub image: String,
    pub owned: bool,
    /// `available`, `coming-soon`, `expired`, or None with no known window.
    pub status: Option<String>,
    /// The tile's word: "OWNED", "AVAILABLE", "SOON", "ENDED" or empty.
    pub word: String,
    /// The sheet's status line: "OWNED", "AVAILABLE NOW", "COMING SOON",
    /// "NO LONGER EARNABLE" or empty.
    pub sheet_word: String,
    /// The earn copy, the window line and the eligibility caveat, split.
    pub prose: Vec<TextPart>,
    pub window: Vec<TextPart>,
    pub caveat: Option<String>,
    /// "1,234 users have this badge".
    pub usage_label: Option<String>,
    pub info_url: Option<String>,
    pub added_ms: i64,
    pub usage: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissingNow {
    /// `hidden` (no account), `loading`, `failed`, `none`, or `list`.
    pub state: String,
    /// A collection refresh is in flight ("Checking Twitch").
    pub refreshing: bool,
    /// "in 6d", "in 5h", "soon": when the first row closes; empty without.
    pub closes: String,
    pub catch_up: Option<CatchUpView>,
    pub rows: Vec<MissingRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatchUpView {
    /// "To catch up" or "To catch up, at least".
    pub label: String,
    pub stats: Vec<Stat>,
    /// The long-press note, one line each.
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stat {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissingRow {
    /// `set/version`, to find the tile holding it.
    pub key: String,
    pub title: String,
    pub image: String,
    pub chips: Vec<EarnChip>,
    /// Three steps or more: the pill shows icons only.
    pub icons_only: bool,
    /// "1 of 3 at random".
    pub random_note: Option<String>,
    /// "Only on a, b" or "Only on 5 channels": the campaign counts only there.
    pub channels_note: Option<String>,
    /// "3d left", "5h left", "ends soon", "ending", or empty.
    pub time_left: String,
    /// It closes within a day: the time left is drawn as a warning.
    pub urgent: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EarnChip {
    /// `watch`, `subscribe`, `purchase`, `cheer`, `create`, `attend`, `other`.
    pub kind: String,
    /// A paid step that is a ticket (drawn as a ticket, else coins).
    pub ticket: bool,
    pub text: String,
    pub hint: String,
    /// Twitch's own art for the kind, when the catalogue has it.
    pub icon_url: Option<String>,
}

// ---- Words ------------------------------------------------------------------

fn minutes_text(minutes: u32) -> String {
    if minutes < 60 || minutes % 60 != 0 {
        format!("{minutes} min")
    } else {
        format!("{} hr", minutes / 60)
    }
}

fn step_kind(step: &EarnStep) -> &'static str {
    match step {
        EarnStep::Watch { .. } => "watch",
        EarnStep::Subscribe { .. } => "subscribe",
        EarnStep::Purchase { .. } => "purchase",
        EarnStep::Cheer => "cheer",
        EarnStep::Create => "create",
        EarnStep::Attend => "attend",
        EarnStep::Other => "other",
    }
}

/// "30 min", "20 min × 3d", "2 subs", "Ticket".
pub fn chip_text(step: &EarnStep) -> String {
    match step {
        EarnStep::Watch { minutes, days } => {
            let base = minutes.map(minutes_text).unwrap_or_else(|| "Watch".into());
            match days {
                Some(d) => format!("{base} × {d}d"),
                None => base,
            }
        }
        EarnStep::Subscribe { count: Some(n) } => format!("{n} subs"),
        EarnStep::Subscribe { count: None } => "Sub".into(),
        EarnStep::Purchase { ticket: true } => "Ticket".into(),
        EarnStep::Purchase { ticket: false } => "Paid".into(),
        EarnStep::Cheer => "Bits".into(),
        EarnStep::Create => "Creator".into(),
        EarnStep::Attend => "In person".into(),
        EarnStep::Other => "Special".into(),
    }
}

/// The long form, for the chip's accessibility hint.
pub fn chip_hint(step: &EarnStep) -> String {
    match step {
        EarnStep::Watch { minutes: Some(m), days } => match days {
            Some(d) => format!("Watch {} on {d} different days", minutes_text(*m)),
            None => format!("Watch {}", minutes_text(*m)),
        },
        EarnStep::Watch { minutes: None, .. } => "Watch during the campaign".into(),
        EarnStep::Subscribe { count: Some(n) } => format!("Subscribe or gift {n} subscriptions"),
        EarnStep::Subscribe { count: None } => "Subscribe or gift a subscription".into(),
        EarnStep::Purchase { ticket: true } => "Granted with a ticket purchase".into(),
        EarnStep::Purchase { ticket: false } => "Granted with a purchase".into(),
        EarnStep::Cheer => "Cheer with Bits".into(),
        EarnStep::Create => "Something you do as a creator".into(),
        EarnStep::Attend => "Be there in person".into(),
        EarnStep::Other => "See the details".into(),
    }
}

/// "3d left", "5h left", "ends soon", "ending"; empty with no end.
pub fn time_left(ends_ms: Option<i64>, now: i64) -> String {
    let Some(end) = ends_ms else { return String::new() };
    let left = end - now;
    if left <= 0 {
        return "ending".into();
    }
    let hours = left / 3_600_000;
    if hours >= 48 {
        format!("{}d left", hours / 24)
    } else if hours >= 1 {
        format!("{hours}h left")
    } else {
        "ends soon".into()
    }
}

/// "in 6d", "in 5h", "soon".
pub fn closes_in(ends_ms: Option<i64>, now: i64) -> String {
    let Some(end) = ends_ms else { return String::new() };
    let hours = (end - now).div_euclid(3_600_000);
    if hours >= 48 {
        format!("in {}d", hours / 24)
    } else if hours >= 1 {
        format!("in {hours}h")
    } else {
        "soon".into()
    }
}

/// "45m", "6h 20m", "2d 3h".
pub fn watch_time(minutes: u32) -> String {
    let m = minutes;
    if m < 60 {
        return format!("{m}m");
    }
    let h = m / 60;
    if h < 48 {
        return if m % 60 > 0 { format!("{h}h {}m", m % 60) } else { format!("{h}h") };
    }
    if h % 24 > 0 {
        format!("{}d {}h", h / 24, h % 24)
    } else {
        format!("{}d", h / 24)
    }
}

/// "$17.97".
pub fn usd(cents: u32) -> String {
    format!("${}.{:02}", grouped(u64::from(cents / 100)), cents % 100)
}

/// "1,234".
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn catch_up_view(totals: &CatchUp) -> Option<CatchUpView> {
    let mut stats = Vec::new();
    if totals.subs > 0 {
        let label = if totals.subs == 1 { "in 1 sub".to_string() } else { format!("in {} subs", totals.subs) };
        stats.push(Stat { value: usd(totals.sub_cost_cents), label });
    }
    if totals.watch_minutes > 0 {
        stats.push(Stat { value: watch_time(totals.watch_minutes), label: "of watching".into() });
    }
    if totals.tickets > 0 {
        let label = if totals.tickets == 1 { "event pass" } else { "event passes" };
        stats.push(Stat { value: totals.tickets.to_string(), label: label.into() });
    }
    if stats.is_empty() {
        return None;
    }
    let mut notes = vec![
        "Subs and watch time in the same category count toward every badge there, so they are counted once. Subs at the US price of $5.99.".to_string(),
    ];
    if totals.random > 0 {
        notes.push(if totals.random == 1 {
            "One badge is drawn at random, so it may take more.".to_string()
        } else {
            format!("{} badges are drawn at random, so it may take more.", totals.random)
        });
    }
    if totals.unpriced > 0 {
        notes.push(if totals.unpriced == 1 {
            "One badge has no cost to add up (Bits, creator tasks).".to_string()
        } else {
            format!("{} badges have no cost to add up (Bits, creator tasks).", totals.unpriced)
        });
    }
    if totals.estimated {
        notes.push("Some numbers come from badge descriptions rather than Twitch.".into());
    }
    let floor = totals.random > 0 || totals.unpriced > 0;
    Some(CatchUpView {
        label: if floor { "To catch up, at least" } else { "To catch up" }.into(),
        stats,
        notes,
    })
}

/// Where a campaign with an allow-list counts: up to three channels by name,
/// else how many.
fn channels_note(channels: &[String]) -> Option<String> {
    match channels.len() {
        0 => None,
        1..=3 => Some(format!("Only on {}", channels.join(", "))),
        n => Some(format!("Only on {n} channels")),
    }
}

fn missing_row(badge: &MissingBadge, icons: &HashMap<String, String>, now: i64) -> MissingRow {
    let EarnPath { steps, random_of, channels, .. } = &badge.earn;
    MissingRow {
        key: badge.key.clone(),
        title: badge.title.clone(),
        image: badge.image_url.clone(),
        chips: steps
            .iter()
            .map(|s| EarnChip {
                kind: step_kind(s).into(),
                ticket: matches!(s, EarnStep::Purchase { ticket: true }),
                text: chip_text(s),
                hint: chip_hint(s),
                icon_url: icons.get(step_kind(s)).cloned(),
            })
            .collect(),
        icons_only: steps.len() >= 3,
        random_note: random_of.filter(|n| *n > 1).map(|n| format!("1 of {n} at random")),
        channels_note: channels.as_deref().and_then(channels_note),
        time_left: time_left(badge.ends_ms, now),
        urgent: badge.ends_ms.is_some_and(|end| end - now < 24 * 3_600_000),
    }
}

/// The strip above the wall, from the standing.
pub fn missing_now(standing: &BadgeStanding, now: i64) -> MissingNow {
    let failed = standing.collection == CollectionState::Partial
        && standing.collection_reason.as_deref() == Some("fetch_failed");
    let loading = !standing.catalogue_ready || (standing.collection == CollectionState::Partial && !failed);
    let state = if standing.collection == CollectionState::Unavailable {
        "hidden"
    } else if loading {
        "loading"
    } else if failed {
        "failed"
    } else if standing.missing_now.is_empty() {
        "none"
    } else {
        "list"
    };
    let list = state == "list";
    MissingNow {
        state: state.into(),
        refreshing: standing.refreshing,
        closes: if list { closes_in(standing.missing_now[0].ends_ms, now) } else { String::new() },
        catch_up: if list { standing.catch_up.as_ref().and_then(catch_up_view) } else { None },
        rows: if list {
            standing.missing_now.iter().map(|b| missing_row(b, &standing.earn_icons, now)).collect()
        } else {
            Vec::new()
        },
    }
}

// ---- The badge blurb ----------------------------------------------------------

static WINDOW_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?im)^[ \t]*(?:event duration|available)[ \t]*:?[ \t]*(.+)$").unwrap());
static CAVEAT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\(([^)]*(?:don't count|do not count|not eligible|excluded|doesn't count)[^)]*)\)").unwrap()
});
static ISO_STAMP: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2})?Z?").unwrap());
static SPACES: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]{2,}").unwrap());
static SPACE_PUNCT: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t]+([.,])").unwrap());
static BLANK_LINES: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());

/// Text with every ISO stamp cut out as an instant (read as UTC without a zone).
pub fn parts(text: &str) -> Vec<TextPart> {
    let mut out = Vec::new();
    let mut at = 0;
    for m in ISO_STAMP.find_iter(text) {
        let stamp = m.as_str();
        let bare = stamp.trim_end_matches(['Z', 'z']);
        let iso = if bare.len() == 16 { format!("{bare}:00Z") } else { format!("{bare}Z") };
        let Ok(when) = chrono::DateTime::parse_from_rfc3339(&iso) else { continue };
        if m.start() > at {
            out.push(TextPart { text: Some(text[at..m.start()].to_string()), ms: None });
        }
        out.push(TextPart { text: None, ms: Some(when.timestamp_millis()) });
        at = m.end();
    }
    if at < text.len() {
        out.push(TextPart { text: Some(text[at..].to_string()), ms: None });
    }
    out
}

/// The blurb as prose, the event-window line and an eligibility caveat.
pub fn split_blurb(raw: &str) -> (Vec<TextPart>, Vec<TextPart>, Option<String>) {
    let text = badge_window::decode_html_entities(raw);
    let mut rest = text.clone();
    let window = WINDOW_LINE.captures(&text).map(|c| {
        rest = rest.replacen(c.get(0).unwrap().as_str(), "", 1);
        c[1].trim().to_string()
    });
    let caveat = CAVEAT.captures(&rest.clone()).map(|c| {
        rest = rest.replacen(c.get(0).unwrap().as_str(), "", 1);
        c[1].trim().to_string()
    });
    let prose = SPACES.replace_all(&rest, " ");
    let prose = SPACE_PUNCT.replace_all(&prose, "$1");
    let prose = BLANK_LINES.replace_all(&prose, "\n\n");
    let prose = prose.trim();
    (parts(prose), window.map(|w| parts(&w)).unwrap_or_default(), caveat)
}

// ---- The wall -------------------------------------------------------------------

/// A badge version from the catalogue, with what the metadata cache holds for it.
struct Version<'a> {
    set_id: &'a str,
    version: &'a str,
    title: &'a str,
    description: &'a str,
    image: &'a str,
    meta: Option<&'a UniversalCacheEntry>,
}

fn metadata_key(set_id: &str, version: &str) -> String {
    format!("metadata:{set_id}-v{version}")
}

fn digits(text: &str) -> u64 {
    let d: String = text.chars().filter(char::is_ascii_digit).collect();
    d.parse().unwrap_or(0)
}

fn date_ms(text: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(text)
        .map(|d| d.timestamp_millis())
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis())
        })
        .unwrap_or(0)
}

fn version_rank(id: &str) -> u64 {
    id.parse().unwrap_or(0)
}

/// Tiles, merged by title: the same title in another set keeps the first; a
/// higher version id of the title in the SAME set replaces it (a relaunch).
fn tiles(
    catalogue: &HelixBadgesResponse,
    metadata: &HashMap<String, UniversalCacheEntry>,
    windows: &HashMap<String, Vec<WindowRun>>,
    owned: &HashSet<&str>,
    now: i64,
) -> Vec<(BadgeTile, u32)> {
    let mut order: Vec<String> = Vec::new();
    let mut by_title: HashMap<String, (BadgeTile, u32, String, String)> = HashMap::new();
    for set in &catalogue.data {
        for v in &set.versions {
            let image = if !v.image_url_4x.is_empty() { &v.image_url_4x } else { &v.image_url_2x };
            if v.title.is_empty() || image.is_empty() || set.set_id.is_empty() || v.id.is_empty() {
                continue;
            }
            let version = Version {
                set_id: &set.set_id,
                version: &v.id,
                title: &v.title,
                description: &v.description,
                image,
                meta: metadata.get(&metadata_key(&set.set_id, &v.id)),
            };
            let own_key = format!("{}/{}", set.set_id, v.id);
            let (tile, position) = tile(&version, windows.get(&own_key), now);
            match by_title.get_mut(&v.title) {
                None => {
                    order.push(v.title.clone());
                    by_title.insert(v.title.clone(), (tile, position, set.set_id.clone(), v.id.clone()));
                }
                Some(prev) if prev.2 == set.set_id && version_rank(&v.id) > version_rank(&prev.3) => {
                    let mut keys = std::mem::take(&mut prev.0.keys);
                    keys.push(own_key);
                    *prev = (BadgeTile { keys, ..tile }, position, set.set_id.clone(), v.id.clone());
                }
                Some(prev) => prev.0.keys.push(own_key),
            }
        }
    }
    order
        .into_iter()
        .filter_map(|t| by_title.remove(&t))
        .map(|(mut tile, position, _, _)| {
            tile.owned = tile.keys.iter().any(|k| owned.contains(k.as_str()));
            let (word, sheet_word) = words(tile.owned, tile.status.as_deref());
            tile.word = word.into();
            tile.sheet_word = sheet_word.into();
            (tile, position)
        })
        .collect()
}

fn words(owned: bool, status: Option<&str>) -> (&'static str, &'static str) {
    if owned {
        return ("OWNED", "OWNED");
    }
    match status {
        Some("available") => ("AVAILABLE", "AVAILABLE NOW"),
        Some("coming-soon") => ("SOON", "COMING SOON"),
        // Only a window known to be over says so; no window says nothing.
        Some("expired") => ("ENDED", "NO LONGER EARNABLE"),
        _ => ("", ""),
    }
}

fn tile(v: &Version, runs: Option<&Vec<WindowRun>>, now: i64) -> (BadgeTile, u32) {
    let data = v.meta.map(|m| &m.data);
    let field = |key: &str| data.and_then(|d| d.get(key)).and_then(|x| x.as_str()).filter(|s| !s.is_empty());
    // A pushed badge has no scraped date yet; its campaign window opening is
    // an honest stand-in, so a fresh badge does not sort as ancient.
    let added_ms = field("date_added").map(date_ms).filter(|ms| *ms > 0).unwrap_or_else(|| {
        data.and_then(|d| d.get("enrichment"))
            .and_then(|e| e.get("starts_utc"))
            .and_then(|s| s.as_str())
            .map(date_ms)
            .unwrap_or(0)
    });
    let usage = field("usage_stats").map(digits).unwrap_or(0);
    let status = runs.filter(|r| !r.is_empty()).map(|r| {
        match badge_window::status_at(r, now) {
            WindowStatus::Available => "available",
            WindowStatus::ComingSoon => "coming-soon",
            WindowStatus::Expired => "expired",
        }
        .to_string()
    });
    let blurb = field("more_info").unwrap_or(v.description);
    let (prose, window, caveat) = split_blurb(blurb);
    let tile = BadgeTile {
        key: format!("{}-{}", v.set_id, v.version),
        keys: vec![format!("{}/{}", v.set_id, v.version)],
        title: v.title.to_string(),
        image: v.image.to_string(),
        owned: false,
        status,
        word: String::new(),
        sheet_word: String::new(),
        prose,
        window,
        caveat,
        usage_label: (usage > 0).then(|| format!("{} users have this badge", grouped(usage))),
        info_url: field("info_url").map(str::to_string),
        added_ms,
        usage,
    };
    (tile, v.meta.and_then(|m| m.position).unwrap_or(u32::MAX))
}

/// Newest first by date added; the metadata rank breaks ties within a day; a
/// badge with no date sorts last, and the key keeps the order stable.
pub fn order(mut tiles: Vec<(BadgeTile, u32)>, sort: Sort) -> Vec<BadgeTile> {
    tiles.sort_by(|(a, pa), (b, pb)| {
        b.added_ms.cmp(&a.added_ms).then(pa.cmp(pb)).then_with(|| a.key.cmp(&b.key))
    });
    let newest: Vec<BadgeTile> = tiles.into_iter().map(|(t, _)| t).collect();
    let lead = |status: &str, list: Vec<BadgeTile>| {
        let (mut first, rest): (Vec<_>, Vec<_>) = list.into_iter().partition(|t| t.status.as_deref() == Some(status));
        first.extend(rest);
        first
    };
    match sort {
        Sort::Newest => newest,
        Sort::Oldest => newest.into_iter().rev().collect(),
        Sort::Available => lead("available", newest),
        Sort::Soon => lead("coming-soon", newest),
        // A stable sort keeps the Newest order within equal counts.
        Sort::Usage => {
            let mut list = newest;
            list.sort_by(|a, b| b.usage.cmp(&a.usage));
            list
        }
    }
}

/// The wall in `sort`, from the standing and the caches. Kicks the session's
/// metadata backfill once.
pub async fn get(app: &crate::rt::AppHandle, sort: Sort, force: bool) -> BadgeWall {
    // A forced read (pull to refresh) takes Twitch's current list; otherwise
    // the cached one, which may be days old. First, so the standing (and its
    // missing strip, built from the cached list) sees it too.
    let fresh = if force { crate::commands::badges::refresh_global_badges().await.ok() } else { None };
    let standing = crate::services::badge_standing::get_standing(app, force).await;
    let catalogue = match fresh {
        Some(r) if !r.data.is_empty() => Some(r),
        _ => match crate::commands::badges::get_cached_global_badges().await {
            Ok(Some(r)) if !r.data.is_empty() => Some(r),
            _ => crate::commands::badges::fetch_global_badges().await.ok(),
        },
    };
    let now = chrono::Utc::now().timestamp_millis();
    let missing = missing_now(&standing, now);
    let Some(catalogue) = catalogue else {
        return BadgeWall {
            loading: true,
            tiles: Vec::new(),
            count_label: String::new(),
            sync_left: SYNC_LEFT.load(Ordering::Acquire),
            missing,
            next_change_ms: standing.next_change_ms,
        };
    };
    let keys: Vec<String> = catalogue
        .data
        .iter()
        .flat_map(|s| s.versions.iter().map(|v| metadata_key(&s.set_id, &v.id)))
        .collect();
    let metadata = universal_cache_service::get_cached_items_batch(CacheType::Badge, &keys).unwrap_or_default();
    let owned: HashSet<&str> = standing.owned.iter().map(String::as_str).collect();
    let built = tiles(&catalogue, &metadata, &standing.windows, &owned, now);
    let owned_count = built.iter().filter(|(t, _)| t.owned).count();
    let total = built.len();
    start_backfill(app);
    wake_at_next_change(app, standing.next_change_ms, now);
    BadgeWall {
        loading: false,
        tiles: order(built, sort),
        count_label: if total > 0 { format!("{} of {} collected", grouped(owned_count as u64), grouped(total as u64)) } else { String::new() },
        sync_left: SYNC_LEFT.load(Ordering::Acquire),
        missing,
        next_change_ms: standing.next_change_ms,
    }
}

/// When the next earn window opens or closes, tell the wall to read again, so
/// tiles and the missing strip move on their own. One timer per boundary,
/// clamped to a second and six hours (a later wall read sets the next one).
fn wake_at_next_change(app: &crate::rt::AppHandle, next: Option<i64>, now: i64) {
    static SCHEDULED: AtomicI64 = AtomicI64::new(0);
    let Some(at) = next else { return };
    if SCHEDULED.swap(at, Ordering::AcqRel) == at {
        return;
    }
    let delay = (at - now + 500).clamp(1_000, 6 * 3_600_000) as u64;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        if SCHEDULED.load(Ordering::Acquire) == at {
            SCHEDULED.store(0, Ordering::Release);
            use tauri::Emitter;
            let _ = app.emit("badge-standing-changed", ());
        }
    });
}

/// Once per session: fetch the metadata the cache lacks, five at a time,
/// announcing what is left, then tell the wall to read again.
fn start_backfill(app: &crate::rt::AppHandle) {
    if BACKFILL_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;
        let missing = crate::commands::badges::get_badges_missing_metadata().await.unwrap_or_default();
        if missing.is_empty() {
            return;
        }
        log::info!("[BadgeWall] fetching metadata for {} badges", missing.len());
        let total = missing.len() as u32;
        SYNC_LEFT.store(total, Ordering::Release);
        let _ = app.emit(SYNC_EVENT, serde_json::json!({ "left": total }));
        for (i, batch) in missing.chunks(5).enumerate() {
            let fetches = batch.iter().map(|(set, version)| {
                crate::commands::badge_metadata::fetch_badge_metadata(set.clone(), version.clone(), None)
            });
            futures::future::join_all(fetches).await;
            let left = total.saturating_sub(((i + 1) * 5) as u32);
            SYNC_LEFT.store(left, Ordering::Release);
            let _ = app.emit(SYNC_EVENT, serde_json::json!({ "left": left }));
        }
        SYNC_LEFT.store(0, Ordering::Release);
        let _ = app.emit("badge-standing-changed", ());
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::badges::{HelixBadgeSet, HelixBadgeVersion};

    fn version(id: &str, title: &str) -> HelixBadgeVersion {
        HelixBadgeVersion {
            id: id.into(),
            image_url_1x: String::new(),
            image_url_2x: format!("2x-{title}-{id}"),
            image_url_4x: format!("4x-{title}-{id}"),
            title: title.into(),
            description: format!("About {title}"),
            click_action: None,
            click_url: None,
        }
    }

    fn catalogue() -> HelixBadgesResponse {
        HelixBadgesResponse {
            data: vec![
                HelixBadgeSet { set_id: "glhf".into(), versions: vec![version("1", "GLHF"), version("2", "GLHF")] },
                HelixBadgeSet { set_id: "other".into(), versions: vec![version("1", "GLHF"), version("1", "Mouse")] },
                HelixBadgeSet { set_id: "bare".into(), versions: vec![version("1", "")] },
            ],
        }
    }

    #[test]
    fn tiles_merge_by_title_and_a_relaunch_replaces() {
        let owned: HashSet<&str> = HashSet::from(["other/1"]);
        let built = tiles(&catalogue(), &HashMap::new(), &HashMap::new(), &owned, 0);
        let titles: Vec<&str> = built.iter().map(|(t, _)| t.title.as_str()).collect();
        assert_eq!(titles, ["GLHF", "Mouse"]);
        let glhf = &built[0].0;
        // Version 2 of the same set replaced version 1; the other set's copy merged in.
        assert_eq!(glhf.key, "glhf-2");
        assert_eq!(glhf.keys, ["glhf/1", "glhf/2", "other/1"]);
        assert!(glhf.owned);
        assert_eq!(glhf.word, "OWNED");
        assert_eq!(glhf.image, "4x-GLHF-2");
    }

    fn tile_at(key: &str, added: i64, usage: u64, status: Option<&str>) -> (BadgeTile, u32) {
        (
            BadgeTile {
                key: key.into(),
                keys: vec![],
                title: key.into(),
                image: String::new(),
                owned: false,
                status: status.map(str::to_string),
                word: String::new(),
                sheet_word: String::new(),
                prose: vec![],
                window: vec![],
                caveat: None,
                usage_label: None,
                info_url: None,
                added_ms: added,
                usage,
            },
            u32::MAX,
        )
    }

    fn keys(list: Vec<BadgeTile>) -> Vec<String> {
        list.into_iter().map(|t| t.key).collect()
    }

    #[test]
    fn the_five_orders() {
        let set = || {
            vec![
                tile_at("a", 100, 5, None),
                tile_at("b", 300, 1, Some("coming-soon")),
                tile_at("c", 0, 9, Some("available")),
                tile_at("d", 200, 5, Some("available")),
            ]
        };
        assert_eq!(keys(order(set(), Sort::Newest)), ["b", "d", "a", "c"]);
        assert_eq!(keys(order(set(), Sort::Oldest)), ["c", "a", "d", "b"]);
        assert_eq!(keys(order(set(), Sort::Available)), ["d", "c", "b", "a"]);
        assert_eq!(keys(order(set(), Sort::Soon)), ["b", "d", "a", "c"]);
        assert_eq!(keys(order(set(), Sort::Usage)), ["c", "d", "a", "b"]);
    }

    #[test]
    fn the_blurb_splits_into_prose_window_and_caveat() {
        let (prose, window, caveat) = split_blurb(
            "Watch 30 minutes of any stream.  (Prime subs don&#39;t count)\nEvent duration: 2026-10-01T17:00Z to Oct 9",
        );
        assert_eq!(prose, vec![TextPart { text: Some("Watch 30 minutes of any stream.".into()), ms: None }]);
        assert_eq!(caveat.as_deref(), Some("Prime subs don't count"));
        assert_eq!(window[0].ms, Some(chrono::DateTime::parse_from_rfc3339("2026-10-01T17:00:00Z").unwrap().timestamp_millis()));
        assert_eq!(window[1].text.as_deref(), Some(" to Oct 9"));
    }

    #[test]
    fn words_for_time_and_money() {
        assert_eq!(time_left(Some(3 * 86_400_000), 0), "3d left");
        assert_eq!(time_left(Some(5 * 3_600_000), 0), "5h left");
        assert_eq!(time_left(Some(60_000), 0), "ends soon");
        assert_eq!(time_left(Some(0), 1), "ending");
        assert_eq!(time_left(None, 0), "");
        assert_eq!(closes_in(Some(30 * 60_000), 0), "soon");
        assert_eq!(watch_time(45), "45m");
        assert_eq!(watch_time(380), "6h 20m");
        assert_eq!(watch_time(51 * 60), "2d 3h");
        assert_eq!(usd(1797), "$17.97");
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(chip_text(&EarnStep::Watch { minutes: Some(20), days: Some(3) }), "20 min × 3d");
        assert_eq!(chip_hint(&EarnStep::Subscribe { count: Some(2) }), "Subscribe or gift 2 subscriptions");
    }

    #[test]
    fn catch_up_reads_as_a_floor_with_draws() {
        let view = catch_up_view(&CatchUp {
            subs: 3,
            sub_cost_cents: 1797,
            watch_minutes: 120,
            tickets: 0,
            random: 1,
            unpriced: 0,
            estimated: false,
        })
        .unwrap();
        assert_eq!(view.label, "To catch up, at least");
        assert_eq!(view.stats[0], Stat { value: "$17.97".into(), label: "in 3 subs".into() });
        assert_eq!(view.stats[1], Stat { value: "2h".into(), label: "of watching".into() });
        assert_eq!(view.notes.len(), 2);
    }
}
