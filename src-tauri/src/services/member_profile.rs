//! A StreamNook member's public profile, as the phone profile sheets show it:
//! their StreamNook badges and relics, the overview tiles, the accolades they
//! have earned, their most-used emotes and how often their profile was opened,
//! with everything they chose to hide left out.
//!
//! The desktop member card (`components/profile/memberProfile.ts` and
//! `settings/ProfileOverview.tsx`) works the same answer out in TypeScript; the
//! accolade list and the visibility rules here mirror it, so a member reads the
//! same on every screen. Both phones draw this one shape and compute nothing.

use crate::services::cosmetics_registry::{self as registry, CatalogEntry};
use crate::services::ivr::IvrUserSummary;
use crate::services::profile_view::ProfileBadge;
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::sync::Mutex;

// ---- What a member hides ------------------------------------------------------

/// Each section key older apps saved, with the stats it stands for
/// (`services/profileVisibility.ts`).
const SECTIONS: &[(&str, &[&str])] = &[
    ("roast", &["hours", "roast_line"]),
    ("twitch", &["twitch_age", "followers", "account_type"]),
    ("lifetime", &["messages", "streams", "favorite_channel", "subs_gifted", "cosmetics", "member_rank"]),
    ("emotes", &["emotes"]),
    ("accolades", &["accolades"]),
    ("views", &["views"]),
];

/// Whether a stat is hidden, given what `hidden_sections` holds. A section key
/// alone hides its whole section only when none of its stats were saved on
/// their own (the way older apps saved it).
pub fn is_stat_hidden(stored: &[String], stat: &str) -> bool {
    let has = |k: &str| stored.iter().any(|s| s == k);
    if has(stat) {
        return true;
    }
    let Some((section, stats)) = SECTIONS.iter().find(|(section, stats)| *section != stat && stats.contains(&stat)) else {
        return false;
    };
    has(section) && !stats.iter().any(|k| has(k))
}

// ---- The shape ----------------------------------------------------------------

/// One overview tile.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MemberStat {
    /// The stat's visibility key (`hours`, `messages`, `streams`,
    /// `favorite_channel`, `subs_gifted`, `twitch_age`, `followers`, `views`),
    /// which also picks its icon.
    pub key: String,
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

/// One earned accolade, drawn as the desktop's medallion.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AccoladeMedal {
    pub id: String,
    pub label: String,
    /// The medallion's icon as a lucide name (`message-circle`).
    pub icon: String,
    /// The medallion's two gradient stops, `#rrggbb`.
    pub colors: [String; 2],
}

/// One of their most-used emotes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TopEmote {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub url: String,
    pub count: u64,
}

/// The member sections of a profile sheet. Each list is empty when the member
/// hides it or has nothing in it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MemberProfile {
    /// Their relics with art, the featured one first. The sheet's StreamNook
    /// block leads with them, ahead of every badge group.
    #[serde(default)]
    pub relics: Vec<ProfileBadge>,
    /// The overview, in the desktop card's order, ending on how many times
    /// other people opened their profile.
    pub stats: Vec<MemberStat>,
    /// Earned accolades only, in the desktop wall's order.
    pub accolades: Vec<AccoladeMedal>,
    /// At most eight, most used first.
    pub top_emotes: Vec<TopEmote>,
}

// ---- Accolades ------------------------------------------------------------------

/// What decides an accolade on someone else's profile.
#[derive(Clone, Copy)]
enum Rule {
    Messages(u64),
    Hours(u64),
    Streams(u64),
    /// Drops and channel points are only ever read on your own profile, so on
    /// a public one these light only from the saved set.
    Saved,
    Cosmetics(u64),
    Years(i64),
    FirstHundred,
    /// Every base accolade.
    Completionist,
    TripleThreat,
    Palindrome,
    Nice,
}

struct Def {
    id: &'static str,
    label: &'static str,
    icon: &'static str,
    colors: (&'static str, &'static str),
    rule: Rule,
}

const fn def(id: &'static str, label: &'static str, icon: &'static str, a: &'static str, b: &'static str, rule: Rule) -> Def {
    Def { id, label, icon, colors: (a, b), rule }
}

/// The base accolades, which Completionist asks for all of.
const BASE: &[Def] = &[
    def("chatterbox", "Chatterbox", "message-circle", "#6aa9ff", "#3b6fd4", Rule::Messages(1_000)),
    def("motormouth", "Motormouth", "megaphone", "#5b8def", "#2f5fc0", Rule::Messages(10_000)),
    def("yapper", "Yapper Supreme", "mic", "#8b5cf6", "#6d28d9", Rule::Messages(50_000)),
    def("marathoner", "Marathoner", "timer", "#f59e0b", "#b45309", Rule::Hours(100)),
    def("nosleep", "No Sleep", "moon", "#6366f1", "#4338ca", Rule::Hours(1_000)),
    def("grassless", "Grassless", "armchair", "#84cc16", "#4d7c0f", Rule::Hours(5_000)),
    def("surfer", "Channel Surfer", "waves", "#06b6d4", "#0e7490", Rule::Streams(50)),
    def("lurker", "Certified Lurker", "ghost", "#94a3b8", "#475569", Rule::Streams(250)),
    def("dropgoblin", "Drop Goblin", "package", "#22c55e", "#15803d", Rule::Saved),
    def("dropdragon", "Drop Dragon", "flame", "#ef4444", "#b91c1c", Rule::Saved),
    def("hoarder", "Point Hoarder", "piggy-bank", "#eab308", "#a16207", Rule::Saved),
    def("tycoon", "Point Tycoon", "gem", "#2dd4bf", "#0f766e", Rule::Saved),
    def("collector", "Collector", "shapes", "#ec4899", "#be185d", Rule::Cosmetics(5)),
    def("driplord", "Drip Lord", "droplets", "#38bdf8", "#0284c7", Rule::Cosmetics(15)),
    def("veteran", "Veteran", "medal", "#fb923c", "#c2410c", Rule::Years(5)),
    def("ancient", "Ancient One", "hourglass", "#a855f7", "#7e22ce", Rule::Years(10)),
    def("og", "OG", "crown", "#fbbf24", "#d97706", Rule::FirstHundred),
];

/// The rest, in the wall's order: the secret ones, the offline room's, the
/// seasons and holidays, Cake Day, then the limited events. A secret one never
/// shows on someone else's profile until it is earned, which is all this list
/// ever shows anyway.
const MORE: &[Def] = &[
    // Names too long for a medallion's column carry soft hyphens, so they
    // break at a syllable (with a hyphen) instead of mid-letter.
    def("completionist", "Com\u{ad}ple\u{ad}tion\u{ad}ist", "star", "#f472b6", "#db2777", Rule::Completionist),
    def("triple", "Triple Threat", "layers", "#34d399", "#0f766e", Rule::TripleThreat),
    def("palindrome", "Palindrome", "repeat", "#c084fc", "#7e22ce", Rule::Palindrome),
    def("nice", "Nice", "smile", "#facc15", "#ca8a04", Rule::Nice),
    def("insomniac", "Restless", "bell-ring", "#818cf8", "#4338ca", Rule::Saved),
    def("grand_finale_2026", "Grand Finale", "sparkles", "#fcd34d", "#dc3d55", Rule::Saved),
    def("perfect_corner", "Perfect Corner", "target", "#5eead4", "#0f766e", Rule::Saved),
    def("shiny_peepo", "Shiny", "sparkles", "#fde68a", "#d97706", Rule::Saved),
    def("worth_the_wait", "Worth the Wait", "hourglass", "#f9a8d4", "#9333ea", Rule::Saved),
    def("new_year", "New Year's", "party-popper", "#fbbf24", "#d97706", Rule::Saved),
    def("valentines", "Valentine's", "heart", "#fb7185", "#be123c", Rule::Saved),
    def("st_patricks", "St. Patrick's", "clover", "#4ade80", "#15803d", Rule::Saved),
    def("april_fools", "April Fools'", "laugh", "#c084fc", "#7c3aed", Rule::Saved),
    def("may_fourth", "May the 4th", "sword", "#64748b", "#1e293b", Rule::Saved),
    def("halloween", "Halloween", "skull", "#fb923c", "#c2410c", Rule::Saved),
    def("thanksgiving", "Turkey Day", "drumstick", "#d8a657", "#92400e", Rule::Saved),
    def("winter_holiday", "Winter Holiday", "tree-pine", "#4ade80", "#b91c1c", Rule::Saved),
    def("spring", "Spring", "flower-2", "#f9a8d4", "#4ade80", Rule::Saved),
    def("summer", "Summer", "sun", "#fcd34d", "#f59e0b", Rule::Saved),
    def("fall", "Fall", "leaf", "#fb923c", "#b45309", Rule::Saved),
    def("winter", "Winter", "snowflake", "#93c5fd", "#2563eb", Rule::Saved),
    def("cake_day", "Cake Day", "cake", "#f9a8d4", "#db2777", Rule::Saved),
    def("cs2_major_cologne_2026", "CS2 Major Cologne 2026", "trophy", "#f6e3a0", "#9c7528", Rule::Saved),
    def("semiquincentennial_2026", "Semi\u{ad}quin\u{ad}cen\u{ad}ten\u{ad}nial", "flag", "#93b4ff", "#dc3d55", Rule::Saved),
];

/// The public numbers the accolade rules read.
#[derive(Clone, Copy, Default)]
struct Tally {
    messages: u64,
    hours: u64,
    streams: u64,
    cosmetics: u64,
    years: i64,
    number: Option<u32>,
}

fn is_palindrome(n: u32) -> bool {
    n >= 10 && {
        let s = n.to_string();
        s.chars().eq(s.chars().rev())
    }
}

fn reached(rule: Rule, t: &Tally, all_base: bool) -> bool {
    match rule {
        Rule::Messages(n) => t.messages >= n,
        Rule::Hours(n) => t.hours >= n,
        Rule::Streams(n) => t.streams >= n,
        Rule::Saved => false,
        Rule::Cosmetics(n) => t.cosmetics >= n,
        Rule::Years(n) => t.years >= n,
        Rule::FirstHundred => t.number.is_some_and(|n| n <= 100),
        Rule::Completionist => all_base,
        // Drops read 0 on a public profile, so only the saved set lights it.
        Rule::TripleThreat => false,
        Rule::Palindrome => t.number.is_some_and(is_palindrome),
        Rule::Nice => t.number.is_some_and(|n| [69, 420, 666, 777, 1337].contains(&n)),
    }
}

/// The accolades earned, in the wall's order: reached by the public numbers,
/// or saved when the member earned them (an earned accolade stays earned).
fn earned_accolades(t: &Tally, saved: &HashSet<String>) -> Vec<AccoladeMedal> {
    let lit = |d: &Def, all_base: bool| saved.contains(d.id) || reached(d.rule, t, all_base);
    let all_base = BASE.iter().all(|d| lit(d, false));
    BASE.iter()
        .chain(MORE)
        .filter(|d| lit(d, all_base))
        .map(|d| AccoladeMedal {
            id: d.id.into(),
            label: d.label.into(),
            icon: d.icon.into(),
            colors: [d.colors.0.into(), d.colors.1.into()],
        })
        .collect()
}

// ---- The overview ----------------------------------------------------------------

/// `1234567` as "1,234,567".
pub fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn num(row: Option<&Value>, key: &str) -> f64 {
    row.and_then(|r| r.get(key)).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0)
}

fn plural(n: i64, one: &str) -> String {
    format!("{n} {one}{}", if n == 1 { "" } else { "s" })
}

/// A Twitch account's age as the desktop card says it ("2 yrs, 4 mos"), its
/// whole years (for the milestones) and the days until its next cake day.
fn account_age(created: NaiveDate, today: NaiveDate) -> (String, i64, i64) {
    let mut years = i64::from(today.year() - created.year());
    let mut months = i64::from(today.month()) - i64::from(created.month());
    if today.day() < created.day() {
        months -= 1;
    }
    if months < 0 {
        years -= 1;
        months += 12;
    }
    // A Feb 29 account has its cake day on Mar 1 in the other years.
    let on = |y: i32| NaiveDate::from_ymd_opt(y, created.month(), created.day()).or_else(|| NaiveDate::from_ymd_opt(y, 3, 1));
    let next = on(today.year()).filter(|d| *d >= today).or_else(|| on(today.year() + 1)).unwrap_or(today);
    let label = if years > 0 {
        let mut s = plural(years, "yr");
        if months > 0 {
            s.push_str(&format!(", {}", plural(months, "mo")));
        }
        s
    } else {
        plural(months, "mo")
    };
    (label, years, (next - today).num_days())
}

fn cake_caption(days: i64) -> String {
    match days {
        0 => "Cake day is today".into(),
        1 => "Cake day tomorrow".into(),
        n => format!("Cake day in {n} days"),
    }
}

/// Everything read for one member, before it is shaped.
#[derive(Default)]
pub struct MemberFacts {
    pub number: Option<u32>,
    pub hidden: Vec<String>,
    /// Their `user_stats` row.
    pub stats: Option<Value>,
    /// Their most-watched `user_channel_watch` row.
    pub favorite: Option<Value>,
    pub saved_accolades: HashSet<String>,
    pub ivr: Option<IvrUserSummary>,
    /// Their `user_emote_usage` rows, most used first.
    pub emotes: Vec<Value>,
    /// 7TV paints and badges plus StreamNook cosmetics owned.
    pub cosmetics: u64,
    /// The count the view itself returned, when this open counted one.
    pub counted_views: Option<u64>,
    pub today: Option<NaiveDate>,
}

pub fn build(f: &MemberFacts) -> MemberProfile {
    let hidden = |k: &str| is_stat_hidden(&f.hidden, k);
    let stats = f.stats.as_ref();
    let hours = (num(stats, "hours_watched") + num(stats, "recap_hours")).round().max(0.0) as u64;
    let messages = (num(stats, "messages_sent") + num(stats, "recap_messages")).max(0.0) as u64;
    let streams = num(stats, "streams_watched").max(0.0) as u64;
    let subs_gifted = num(stats, "recap_subs_gifted").max(0.0) as u64;
    let today = f.today.unwrap_or_else(|| chrono::Utc::now().date_naive());
    let age = f
        .ivr
        .as_ref()
        .and_then(|i| i.created_at.as_deref())
        .and_then(|c| chrono::DateTime::parse_from_rfc3339(c).ok())
        .map(|c| account_age(c.date_naive(), today));

    let mut tiles = Vec::new();
    let mut tile = |key: &str, label: &str, value: String, caption: Option<String>| {
        if !hidden(key) {
            tiles.push(MemberStat { key: key.into(), label: label.into(), value, caption });
        }
    };
    if f.stats.is_some() {
        tile("hours", "Hours watched", grouped(hours), None);
        tile("messages", "Messages sent", grouped(messages), None);
        tile("streams", "Streams watched", grouped(streams), None);
    }
    if let Some(fav) = f.favorite.as_ref() {
        let text = |k: &str| fav.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        if let Some(name) = text("channel_name").or_else(|| text("channel_login")) {
            let watched = (num(Some(fav), "minutes") / 60.0).round().max(0.0) as u64;
            tile("favorite_channel", "Favorite channel", name.to_string(), Some(format!("{}h watched", grouped(watched))));
        }
    }
    if subs_gifted > 0 {
        tile("subs_gifted", "Subs gifted", grouped(subs_gifted), None);
    }
    if let Some((label, _, days)) = age.as_ref() {
        tile("twitch_age", "Twitch age", label.clone(), Some(cake_caption(*days)));
    }
    if let Some(followers) = f.ivr.as_ref().and_then(|i| i.followers) {
        tile("followers", "Followers", grouped(followers.max(0) as u64), None);
    }
    if let Some(views) = f.counted_views.or_else(|| stats.and_then(|s| s.get("profile_views")).and_then(Value::as_u64)) {
        tile("views", "Profile views", grouped(views), None);
    }

    let tally = Tally { messages, hours, streams, cosmetics: f.cosmetics, years: age.map(|a| a.1).unwrap_or(0), number: f.number };
    let accolades = if hidden("accolades") { Vec::new() } else { earned_accolades(&tally, &f.saved_accolades) };

    let top_emotes = if hidden("emotes") {
        Vec::new()
    } else {
        f.emotes
            .iter()
            .filter_map(|r| {
                let text = |k: &str| r.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
                Some(TopEmote {
                    id: text("emote_id")?,
                    name: text("emote_name").unwrap_or_default(),
                    provider: text("provider").unwrap_or_default(),
                    url: text("image_url")?,
                    count: num(Some(r), "count").max(0.0) as u64,
                })
            })
            .take(TOP_EMOTES)
            .collect()
    };

    MemberProfile { relics: Vec::new(), stats: tiles, accolades, top_emotes }
}

const TOP_EMOTES: usize = 8;

// ---- StreamNook badges and relics -------------------------------------------------

/// Their StreamNook badges: the one they wear (the plain mark when none), then
/// every other badge they own that has art, in catalog order. Atmospheres,
/// frames and relics show in their own places.
pub fn streamnook_badges(worn: Option<&registry::SnBadge>, owned: &BTreeSet<String>, catalog: &[CatalogEntry]) -> Vec<ProfileBadge> {
    let Some(worn) = worn else { return Vec::new() };
    let worn_slug = worn.slug.clone().unwrap_or_else(|| registry::DEFAULT_SLUG.to_string());
    let mut out = vec![ProfileBadge {
        id: worn_slug.clone(),
        url: worn.image_url.clone(),
        title: worn.label.clone(),
        description: catalog.iter().find(|c| c.slug == worn_slug).and_then(|c| c.description.clone()).filter(|d| !d.is_empty()),
        provider: Some("streamnook".into()),
        ..Default::default()
    }];
    let mut rest: Vec<&CatalogEntry> = catalog
        .iter()
        .filter(|c| c.slug != worn_slug && owned.contains(&c.slug) && registry::kind_is_badge(&c.kind))
        .collect();
    rest.sort_by_key(|c| c.sort_order);
    out.extend(rest.into_iter().filter_map(|c| {
        Some(ProfileBadge {
            id: c.slug.clone(),
            url: registry::badge_art(c, false)?,
            title: c.name.clone(),
            description: c.description.clone().filter(|d| !d.is_empty()),
            provider: Some("streamnook".into()),
            ..Default::default()
        })
    }));
    out
}

/// Their relics with art, the one they feature first (else the first owned).
pub fn relics(owned: &BTreeSet<String>, featured: Option<&str>, catalog: &[CatalogEntry]) -> Vec<ProfileBadge> {
    let mut list: Vec<ProfileBadge> = catalog
        .iter()
        .filter(|c| c.kind == "relic" && owned.contains(&c.slug))
        .filter_map(|c| {
            Some(ProfileBadge {
                id: c.slug.clone(),
                url: registry::badge_art(c, false)?,
                title: c.name.clone(),
                description: c.description.clone().filter(|d| !d.is_empty()),
                provider: Some("streamnook".into()),
                ..Default::default()
            })
        })
        .collect();
    if let Some(at) = featured.and_then(|slug| list.iter().position(|r| r.id == slug)) {
        let lead = list.remove(at);
        list.insert(0, lead);
    }
    list
}

// ---- Reads ----------------------------------------------------------------------

fn digits(id: &str) -> String {
    id.chars().filter(char::is_ascii_digit).collect()
}

async fn first_row(table: &str, query: String) -> Option<Value> {
    crate::services::streamnook_backend::select(table, &query)
        .await
        .map_err(|e| log::debug!("[MemberProfile] {table} unavailable: {e}"))
        .ok()?
        .into_iter()
        .next()
}

/// Profiles counted this session, so reopening one does not count again.
static COUNTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Count one view of someone else's profile (never your own, once a session),
/// answering the new total. A failed count may try again on the next open.
async fn count_view(member_id: &str) -> Option<u64> {
    let viewer = crate::services::account_roster::twitch_user_id()?;
    if viewer == member_id {
        return None;
    }
    let first = COUNTED.lock().ok()?.get_or_insert_with(HashSet::new).insert(member_id.to_string());
    if !first {
        return None;
    }
    match crate::services::streamnook_backend::rpc("increment_profile_view", serde_json::json!({ "p_user_id": member_id })).await {
        Ok(total) => total.as_u64(),
        Err(e) => {
            log::debug!("[MemberProfile] view not counted: {e}");
            if let Ok(mut g) = COUNTED.lock() {
                g.get_or_insert_with(HashSet::new).remove(member_id);
            }
            None
        }
    }
}

/// What a member's sheet adds for them: the member sections (relics included),
/// their StreamNook badges, and whether they hide their badge collection (which
/// hides every badge group, as on desktop).
pub struct MemberSheet {
    pub profile: MemberProfile,
    pub streamnook: Vec<ProfileBadge>,
    pub hide_badges: bool,
}

/// A member's reads, waiting on the 7TV count the sheet reads beside them.
pub struct MemberReads {
    facts: MemberFacts,
    featured: Option<String>,
    member_id: String,
}

impl MemberReads {
    /// Shape the sheet's member part; `seventv_cosmetics` counts their 7TV
    /// paints and badges (Collector and Drip Lord count them).
    pub fn finish(mut self, seventv_cosmetics: u64) -> MemberSheet {
        let owned = registry::owned_slugs(&self.member_id);
        let catalog = registry::catalog();
        self.facts.cosmetics = seventv_cosmetics + owned.len() as u64;
        let hidden = &self.facts.hidden;
        let mut profile = build(&self.facts);
        if !is_stat_hidden(hidden, "relics") {
            profile.relics = relics(&owned, self.featured.as_deref(), &catalog);
        }
        MemberSheet {
            profile,
            streamnook: streamnook_badges(registry::member_badge(&self.member_id, false).as_ref(), &owned, &catalog),
            hide_badges: is_stat_hidden(hidden, "badge_collection"),
        }
    }
}

/// Everything a member's sheet reads for them, each read failing soft. None
/// when they are not a member, or when their preferences could not be read:
/// what they hide must never show.
pub async fn member_reads(member_id: &str, login: &str) -> Option<MemberReads> {
    // A cold launch may open a sheet before the member list has loaded.
    registry::ready(std::time::Duration::from_secs(3)).await;
    let number = registry::member_number(member_id)?;
    let id = digits(member_id);
    let equipment_query = format!("select=slot,cosmetic_slug&twitch_user_id=eq.{id}");
    let emotes_query =
        format!("select=emote_id,emote_name,provider,image_url,count&twitch_user_id=eq.{id}&order=count.desc&limit={TOP_EMOTES}");
    let (prefs, stats, favorite, saved, equipment, emotes, ivr, counted) = tokio::join!(
        crate::services::member_theme::try_profile_prefs(member_id),
        first_row("user_stats", format!("select=*&user_id=eq.{id}")),
        first_row(
            "user_channel_watch",
            format!("select=channel_login,channel_name,minutes&twitch_user_id=eq.{id}&order=minutes.desc&limit=1"),
        ),
        crate::services::profile_view::accolades(member_id),
        crate::services::streamnook_backend::select("user_cosmetic_equipment", &equipment_query),
        crate::services::streamnook_backend::select("user_emote_usage", &emotes_query),
        async {
            if login.is_empty() {
                None
            } else {
                crate::services::ivr::user_summary(login).await.ok().flatten()
            }
        },
        count_view(member_id),
    );
    let prefs = prefs.map_err(|e| log::warn!("[MemberProfile] prefs unavailable, member sections left out: {e}")).ok()?;
    let featured = equipment.ok().and_then(|rows| {
        rows.iter()
            .find(|r| r.get("slot").and_then(Value::as_str) == Some("relic_primary"))
            .and_then(|r| r.get("cosmetic_slug").and_then(Value::as_str).map(String::from))
    });
    Some(MemberReads {
        facts: MemberFacts {
            number: Some(number),
            hidden: prefs.hidden_sections,
            stats,
            favorite,
            saved_accolades: saved.unwrap_or_default().into_iter().collect(),
            ivr,
            emotes: emotes.unwrap_or_default(),
            cosmetics: 0,
            counted_views: counted,
            today: None,
        },
        featured,
        member_id: member_id.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn keys(stored: &[&str]) -> Vec<String> {
        stored.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_section_key_alone_hides_its_section_and_a_stat_key_narrows_it() {
        assert!(is_stat_hidden(&keys(&["lifetime"]), "messages"));
        assert!(is_stat_hidden(&keys(&["lifetime"]), "favorite_channel"));
        // Once one stat of the section is saved on its own, the section key
        // only speaks to older apps.
        let narrowed = keys(&["lifetime", "messages"]);
        assert!(is_stat_hidden(&narrowed, "messages"));
        assert!(!is_stat_hidden(&narrowed, "streams"));
        assert!(is_stat_hidden(&keys(&["views"]), "views"));
        assert!(is_stat_hidden(&keys(&["relics"]), "relics"));
        assert!(!is_stat_hidden(&keys(&["roast"]), "relics"));
        assert!(!is_stat_hidden(&[], "hours"));
    }

    fn facts() -> MemberFacts {
        MemberFacts {
            number: Some(1),
            stats: Some(json!({
                "hours_watched": 1200.4, "recap_hours": 99.7, "messages_sent": 9500, "recap_messages": "600",
                "streams_watched": 300, "recap_subs_gifted": 5, "profile_views": 4321,
            })),
            favorite: Some(json!({ "channel_login": "xqc", "channel_name": "xQc", "minutes": 18_750 })),
            ivr: Some(IvrUserSummary {
                followers: Some(1_234_567),
                created_at: Some("2014-03-20T10:00:00Z".into()),
                is_affiliate: false,
                is_partner: false,
                is_staff: false,
            }),
            emotes: vec![
                json!({ "emote_id": "e1", "emote_name": "OMEGALUL", "provider": "bttv", "image_url": "https://x/1", "count": 90 }),
                json!({ "emote_id": "e2", "emote_name": "nourl", "provider": "7tv", "image_url": null, "count": 50 }),
            ],
            cosmetics: 6,
            today: NaiveDate::from_ymd_opt(2026, 10, 17),
            ..Default::default()
        }
    }

    #[test]
    fn the_overview_reads_like_the_desktop_card() {
        let p = build(&facts());
        let shown: Vec<(&str, &str, Option<&str>)> =
            p.stats.iter().map(|s| (s.key.as_str(), s.value.as_str(), s.caption.as_deref())).collect();
        assert_eq!(
            shown,
            vec![
                ("hours", "1,300", None),
                ("messages", "10,100", None),
                ("streams", "300", None),
                ("favorite_channel", "xQc", Some("313h watched")),
                ("subs_gifted", "5", None),
                ("twitch_age", "12 yrs, 6 mos", Some("Cake day in 154 days")),
                ("followers", "1,234,567", None),
                ("views", "4,321", None),
            ]
        );
        let mut counted = facts();
        counted.counted_views = Some(4322);
        assert_eq!(build(&counted).stats.last().map(|s| s.value.as_str()), Some("4,322"), "the count this open made wins");
        assert_eq!(p.top_emotes.len(), 1, "an emote without art is left out");
        assert_eq!(p.top_emotes[0].name, "OMEGALUL");
    }

    #[test]
    fn hidden_stats_never_reach_the_sheet() {
        let mut f = facts();
        f.hidden = keys(&["twitch", "views", "emotes", "accolades", "favorite_channel", "lifetime"]);
        let p = build(&f);
        let shown: Vec<&str> = p.stats.iter().map(|s| s.key.as_str()).collect();
        // `lifetime` is narrowed by `favorite_channel`, so only that one goes.
        assert_eq!(shown, vec!["hours", "messages", "streams", "subs_gifted"]);
        assert!(p.top_emotes.is_empty());
        assert!(p.accolades.is_empty());
    }

    #[test]
    fn accolades_light_from_public_numbers_or_the_saved_set() {
        let mut f = facts();
        f.saved_accolades = ["halloween", "dropgoblin"].iter().map(|s| s.to_string()).collect();
        let ids: Vec<String> = build(&f).accolades.into_iter().map(|a| a.id).collect();
        assert_eq!(
            ids,
            vec![
                "chatterbox", "motormouth", "marathoner", "nosleep", "surfer", "lurker", "dropgoblin", "collector", "veteran",
                "ancient", "og", "halloween",
            ]
        );
        // The secret ones show only once earned.
        assert!(!ids.contains(&"completionist".to_string()));
        assert!(!ids.contains(&"palindrome".to_string()));
    }

    #[test]
    fn famous_numbers_and_completionist() {
        let t = Tally { number: Some(69), ..Default::default() };
        let ids: Vec<String> = earned_accolades(&t, &HashSet::new()).into_iter().map(|a| a.id).collect();
        assert_eq!(ids, vec!["og", "nice"]);
        let t = Tally { number: Some(1221), ..Default::default() };
        assert_eq!(earned_accolades(&t, &HashSet::new()).into_iter().map(|a| a.id).collect::<Vec<_>>(), vec!["palindrome"]);
        let saved: HashSet<String> = BASE.iter().map(|d| d.id.to_string()).collect();
        let all = earned_accolades(&Tally::default(), &saved);
        assert!(all.iter().any(|a| a.id == "completionist"));
        // The long single words break at a syllable, never mid-letter.
        let label = all.iter().find(|a| a.id == "completionist").map(|a| a.label.replace('\u{ad}', ""));
        assert_eq!(label.as_deref(), Some("Completionist"));
        assert_eq!(all[0].colors, ["#6aa9ff".to_string(), "#3b6fd4".to_string()]);
    }

    #[test]
    fn account_age_and_cake_day() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(account_age(d(2024, 10, 17), d(2026, 10, 17)), ("2 yrs".into(), 2, 0));
        assert_eq!(account_age(d(2026, 2, 20), d(2026, 10, 17)), ("7 mos".into(), 0, 126));
        assert_eq!(account_age(d(2025, 10, 18), d(2026, 10, 17)), ("11 mos".into(), 0, 1));
        // A leap-day account has its cake day on Mar 1 in the other years.
        assert_eq!(account_age(d(2020, 2, 29), d(2026, 2, 28)).2, 1);
        assert_eq!(cake_caption(0), "Cake day is today");
    }

    #[test]
    fn numbers_are_grouped() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    fn entry(slug: &str, kind: &str, order: i64, url: &str) -> CatalogEntry {
        CatalogEntry {
            slug: slug.into(),
            name: slug.to_uppercase(),
            kind: kind.into(),
            sort_order: order,
            asset_path: url.into(),
            ..Default::default()
        }
    }

    #[test]
    fn streamnook_badges_lead_with_the_worn_one_and_skip_other_kinds() {
        let catalog = vec![
            entry("b2", "badge", 2, "https://x/b2.png"),
            entry("b1", "", 1, "https://x/b1.png"),
            entry("worn", "badge", 0, "https://x/w.png"),
            entry("aurora", "atmosphere", 0, "https://x/a.png"),
            entry("noart", "badge", 3, ""),
            entry("gem", "relic", 4, "https://x/gem.png"),
        ];
        let owned: BTreeSet<String> = ["b1", "b2", "worn", "aurora", "noart", "gem"].iter().map(|s| s.to_string()).collect();
        let worn = registry::SnBadge { slug: Some("worn".into()), image_url: "https://x/w.png".into(), label: "StreamNook WORN".into() };
        let ids: Vec<String> = streamnook_badges(Some(&worn), &owned, &catalog).into_iter().map(|b| b.id).collect();
        assert_eq!(ids, vec!["worn", "b1", "b2"]);
        assert!(streamnook_badges(None, &owned, &catalog).is_empty(), "not a member, no group");
        let plain = registry::SnBadge { slug: None, image_url: "streamnook-house:streamnook-default".into(), label: "StreamNook".into() };
        let first = &streamnook_badges(Some(&plain), &owned, &catalog)[0];
        assert_eq!((first.id.as_str(), first.url.as_str()), (registry::DEFAULT_SLUG, "streamnook-house:streamnook-default"));
    }

    #[test]
    fn the_featured_relic_leads() {
        let catalog = vec![entry("r1", "relic", 0, "https://x/1"), entry("r2", "relic", 1, "https://x/2"), entry("b", "badge", 0, "https://x/b")];
        let owned: BTreeSet<String> = ["r1", "r2", "b"].iter().map(|s| s.to_string()).collect();
        let ids = |f: Option<&str>| relics(&owned, f, &catalog).into_iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(Some("r2")), vec!["r2", "r1"]);
        assert_eq!(ids(None), vec!["r1", "r2"]);
        assert_eq!(ids(Some("gone")), vec!["r1", "r2"]);
    }

    #[test]
    fn the_shape_keeps_the_keys_both_phones_decode() {
        let v = serde_json::to_value(build(&facts())).unwrap();
        for key in ["relics", "stats", "accolades", "top_emotes"] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert_eq!(v["stats"][0]["key"], "hours");
        assert!(v["stats"][0].get("caption").is_none());
        assert!(v["accolades"][0]["colors"].is_array());
    }
}
