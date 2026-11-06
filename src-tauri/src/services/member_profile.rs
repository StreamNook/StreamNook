//! A StreamNook member's public profile, as the phone profile sheets show it:
//! the hours hero and its roast, the two stat sections, their views, relics,
//! most-used emotes with counts and totals, and the accolade wall, with
//! everything they chose to hide left out.
//!
//! The desktop member card (`components/profile/memberProfile.ts`,
//! `settings/ProfileOverview.tsx`, `settings/accoladeWall.ts`) works the same
//! answer out in TypeScript. The accolade list, the stat labels, the member
//! number names and the roast pool are one JSON source read by both
//! (`src/utils/accolades.catalog.json`, `hoursRoasts.json`), and the rules are
//! pinned by shared cases (`accolades.cases.json`, `hoursRoasts.cases.json`) that
//! both sides run. Both phones draw this one shape and compute nothing.
//!
//! Every read the profile needs is a part with its own state (`Part`): a part
//! that did not answer is `Failed`, never zero, and a build takes a failed part's
//! last answer from the cached sheet (`merge_facts`) before it shapes anything.

use crate::services::cosmetics_registry::{self as registry, CatalogEntry};
use crate::services::ivr::IvrUserSummary;
use crate::services::profile_view::{member_rank, ProfileBadge};
use crate::services::shared_lookup::{Lookup, SharedLookup};
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

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

// ---- The catalog ---------------------------------------------------------------

/// `src/utils/accolades.catalog.json`: every accolade in the desktop wall's
/// order, the stat tiles' labels, the member number names and the section
/// titles. Strict: a key this file does not know is an error, so a one-sided
/// edit to the JSON fails the first test that loads it.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub v: u32,
    pub titles: Titles,
    pub stats: Vec<StatDef>,
    pub member_number_names: NumberNames,
    pub accolades: Vec<AccoladeDef>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct Titles {
    pub hours: String,
    pub hours_unit: String,
    pub twitch: String,
    pub twitch_own: String,
    pub lifetime: String,
    pub top_emotes: String,
    pub accolades: String,
    pub relics: String,
    pub hidden_secret_hint: String,
    pub collected: String,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct StatDef {
    pub key: String,
    pub label: String,
    pub icon: String,
    #[serde(default)]
    pub caption: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct NumberNames {
    /// Member number (as text) to its name.
    pub exact: std::collections::HashMap<String, String>,
    /// The name of any other number ending in 67.
    pub ends_in_67: String,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Base,
    Secret,
    Offline,
    Seasonal,
    Cake,
    Event,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StatKind {
    Messages,
    Hours,
    Streams,
    Drops,
    Points,
    Cosmetics,
    Years,
}

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Special {
    FirstHundred,
    Completionist,
    Triple,
    Palindrome,
    Nice,
    Saved,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct StatRule {
    pub stat: StatKind,
    pub n: u64,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
pub struct SpecialRule {
    pub special: Special,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
pub enum RuleDef {
    Stat(StatRule),
    Special(SpecialRule),
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct Window {
    pub from_m: u32,
    pub from_d: u32,
    pub to_m: u32,
    pub to_d: u32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct AccoladeDef {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub colors: [String; 2],
    pub kind: Kind,
    pub rule: RuleDef,
    #[serde(default)]
    pub progress: Option<String>,
    #[serde(default)]
    pub goal: String,
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub hint_earned: Option<String>,
    #[serde(default)]
    pub window: Option<Window>,
    #[serde(default)]
    pub ended: bool,
}

impl Catalog {
    pub fn parse(text: &str) -> Result<Catalog, String> {
        let cat: Catalog = serde_json::from_str(text).map_err(|e| format!("accolade catalog: {e}"))?;
        if cat.v != 1 {
            return Err(format!("accolade catalog version {} is not supported", cat.v));
        }
        Ok(cat)
    }

    /// A stat tile's definition. An unknown key is a bug in the caller: it
    /// panics in a debug build and falls back to the key itself in a release one.
    pub fn stat(&self, key: &str) -> StatDef {
        self.stats.iter().find(|s| s.key == key).cloned().unwrap_or_else(|| {
            debug_assert!(false, "no stat tile named {key} in the catalog");
            StatDef { key: key.to_string(), label: key.to_string(), icon: String::new(), caption: None }
        })
    }
}

pub static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    Catalog::parse(include_str!("../../../src/utils/accolades.catalog.json")).expect("accolades.catalog.json")
});

// ---- The hours roast -----------------------------------------------------------

#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Roast,
    Neutral,
    Cute,
}

impl Tone {
    pub fn as_str(self) -> &'static str {
        match self {
            Tone::Roast => "roast",
            Tone::Neutral => "neutral",
            Tone::Cute => "cute",
        }
    }
}

/// The unit a templated line fills in. The declaration order is the order of
/// `Roasts::units`.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Days,
    Weeks,
    Movies,
    Seasons,
    Flights,
}

impl Unit {
    fn key(self) -> &'static str {
        match self {
            Unit::Days => "days",
            Unit::Weeks => "weeks",
            Unit::Movies => "movies",
            Unit::Seasons => "seasons",
            Unit::Flights => "flights",
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Divisors {
    days: f64,
    weeks: f64,
    movies: f64,
    seasons: f64,
    flights: f64,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct RoastLine {
    tone: Tone,
    text: String,
    #[serde(default)]
    requires: Option<Unit>,
}

/// `src/utils/hoursRoasts.json`.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Roasts {
    divisors: Divisors,
    low_hours_max: f64,
    low_hours: Vec<String>,
    pool: Vec<RoastLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Roast {
    pub text: String,
    pub tone: Tone,
}

fn index(r: f64, len: usize) -> usize {
    ((r * len as f64).floor().max(0.0) as usize).min(len.saturating_sub(1))
}

fn differs(candidate: &Roast, last: Option<&str>) -> bool {
    Some(candidate.text.as_str()) != last
}

impl Roasts {
    pub fn parse(text: &str) -> Result<Roasts, String> {
        let roasts: Roasts = serde_json::from_str(text).map_err(|e| format!("hours roasts: {e}"))?;
        if roasts.low_hours.is_empty() {
            return Err("hours roasts: no low-hours lines".to_string());
        }
        if !roasts.pool.iter().any(|l| l.requires.is_none()) {
            return Err("hours roasts: the pool needs a line that asks for no unit".to_string());
        }
        Ok(roasts)
    }

    fn is_low(&self, hours: f64) -> bool {
        hours.is_nan() || hours < self.low_hours_max
    }

    /// Hours as days, weeks, movies, seasons and flights, rounded.
    fn units(&self, hours: f64) -> [f64; 5] {
        let d = &self.divisors;
        [
            (hours / d.days).round(),
            (hours / d.weeks).round(),
            (hours / d.movies).round(),
            (hours / d.seasons).round(),
            (hours / d.flights).round(),
        ]
    }

    /// Every line that may be said for these hours, templates filled in: the
    /// low-hours lines for almost no watch time, else every line whose unit
    /// rounds to two or more (so "1 full days" is never said).
    pub fn candidates(&self, hours: f64) -> Vec<Roast> {
        if self.is_low(hours) {
            return self.low_hours.iter().map(|t| Roast { text: t.clone(), tone: Tone::Roast }).collect();
        }
        let units = self.units(hours);
        self.pool
            .iter()
            .filter_map(|line| match line.requires {
                None => Some(Roast { text: line.text.clone(), tone: line.tone }),
                Some(unit) => {
                    let n = units[unit as usize];
                    (n >= 2.0).then(|| Roast {
                        text: line.text.replacen(&format!("{{{}}}", unit.key()), &grouped(n as u64), 1),
                        tone: line.tone,
                    })
                }
            })
            .collect()
    }

    /// One line for these hours. `rand` is asked once for a tone and once for the
    /// pick (low hours ask once, for the pick); `last` is the line shown before,
    /// avoided while another will do.
    pub fn pick(&self, hours: f64, rand: &mut dyn FnMut() -> f64, last: Option<&str>) -> Roast {
        let eligible = self.candidates(hours);
        if self.is_low(hours) {
            let fresh: Vec<&Roast> = eligible.iter().filter(|c| differs(c, last)).collect();
            let list: Vec<&Roast> = if fresh.is_empty() { eligible.iter().collect() } else { fresh };
            return list[index(rand(), list.len())].clone();
        }
        let r = rand();
        let tone = if r < 0.6 {
            Tone::Roast
        } else if r < 0.9 {
            Tone::Neutral
        } else {
            Tone::Cute
        };
        let mut list: Vec<&Roast> = eligible.iter().filter(|c| c.tone == tone && differs(c, last)).collect();
        if list.is_empty() {
            list = eligible.iter().filter(|c| differs(c, last)).collect();
        }
        if list.is_empty() {
            list = eligible.iter().collect();
        }
        list[index(rand(), list.len())].clone()
    }
}

pub static ROASTS: LazyLock<Roasts> =
    LazyLock::new(|| Roasts::parse(include_str!("../../../src/utils/hoursRoasts.json")).expect("hoursRoasts.json"));

/// A roast line for these hours: `last` is the line shown before; `rand` yields
/// numbers in `[0, 1)`.
pub fn roast_line(hours: f64, rand: &mut dyn FnMut() -> f64, last: Option<&str>) -> Option<String> {
    Some(ROASTS.pick(hours, rand, last).text)
}

/// Whether a line may still be said for these hours (a kept line stops being
/// eligible when the hours it was picked for move past what it counted).
pub fn roast_is_eligible(line: &str, hours: f64) -> bool {
    ROASTS.candidates(hours).iter().any(|c| c.text == line)
}

// ---- The accolade wall -----------------------------------------------------------

/// What the wall's rules read. On a public profile `drops` and `points` are 0
/// (the public view never reads them).
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct WallInput {
    pub messages: u64,
    pub hours: f64,
    pub streams: u64,
    pub drops: u64,
    pub points: u64,
    pub cosmetics: u64,
    pub years: i64,
    pub member_number: Option<u32>,
    pub saved: Vec<String>,
    pub hidden: Vec<String>,
    pub today: String,
    pub created_at: Option<String>,
}

/// One accolade, drawn as a medallion.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AccoladeMedal {
    pub id: String,
    /// With soft hyphens where the name is too long for a medallion's column.
    pub label: String,
    /// The medallion's icon as a lucide name (`message-circle`).
    pub icon: String,
    /// The medallion's two gradient stops, `#rrggbb`.
    pub colors: [String; 2],
    pub earned: bool,
    /// A secret accolade (its hint is the neutral one on a public wall).
    pub secret: bool,
    pub hint: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AccoladeWall {
    pub earned: u32,
    pub total: u32,
    /// Earned first, then locked, each in the catalog's order.
    pub items: Vec<AccoladeMedal>,
}

fn is_palindrome(n: Option<u32>) -> bool {
    n.is_some_and(|n| {
        n >= 10 && {
            let s = n.to_string();
            s.chars().eq(s.chars().rev())
        }
    })
}

/// The name a meme member number goes by, if it is one.
fn number_name(cat: &Catalog, n: Option<u32>) -> Option<String> {
    let n = n.filter(|n| *n > 0)?;
    if let Some(name) = cat.member_number_names.exact.get(&n.to_string()) {
        return Some(name.clone());
    }
    (n % 100 == 67).then(|| cat.member_number_names.ends_in_67.clone())
}

/// The stat's visibility key, for the stats a public wall reads. Drops and
/// points are never read publicly, so they have none.
fn hidden_key(stat: StatKind) -> Option<&'static str> {
    match stat {
        StatKind::Messages => Some("messages"),
        StatKind::Hours => Some("hours"),
        StatKind::Streams => Some("streams"),
        StatKind::Cosmetics => Some("cosmetics"),
        StatKind::Years => Some("twitch_age"),
        StatKind::Drops | StatKind::Points => None,
    }
}

fn shows_progress(stat: StatKind) -> bool {
    hidden_key(stat).is_some()
}

fn stat_value(input: &WallInput, stat: StatKind) -> f64 {
    match stat {
        StatKind::Messages => input.messages as f64,
        StatKind::Hours => input.hours,
        StatKind::Streams => input.streams as f64,
        StatKind::Drops => input.drops as f64,
        StatKind::Points => input.points as f64,
        StatKind::Cosmetics => input.cosmetics as f64,
        StatKind::Years => input.years as f64,
    }
}

fn progress_text(def: &AccoladeDef, rule: &StatRule, input: &WallInput) -> Option<String> {
    let label = def.progress.as_ref()?;
    let value = match rule.stat {
        StatKind::Hours => grouped(input.hours.max(0.0).round() as u64),
        StatKind::Messages => grouped(input.messages),
        StatKind::Streams => grouped(input.streams),
        StatKind::Cosmetics => grouped(input.cosmetics),
        StatKind::Years => grouped(input.years.max(0) as u64),
        StatKind::Drops => grouped(input.drops),
        StatKind::Points => grouped(input.points),
    };
    Some(format!("{label}: {value} / {}", grouped(rule.n)))
}

/// What a public wall says under a medallion; never a hidden number, never one
/// the public view does not read, never how to earn a secret.
fn public_hint(cat: &Catalog, def: &AccoladeDef, earned: bool, input: &WallInput, unknown: &[StatKind]) -> String {
    let hint = def.hint.clone().unwrap_or_default();
    match def.kind {
        Kind::Secret => cat.titles.hidden_secret_hint.clone(),
        Kind::Seasonal | Kind::Cake => {
            if earned {
                cat.titles.collected.clone()
            } else {
                hint
            }
        }
        Kind::Event => {
            if earned {
                def.hint_earned.clone().unwrap_or(hint)
            } else {
                hint
            }
        }
        Kind::Offline => hint,
        Kind::Base => match &def.rule {
            RuleDef::Stat(rule)
                if shows_progress(rule.stat)
                    && !unknown.contains(&rule.stat)
                    && !hidden_key(rule.stat).is_some_and(|k| is_stat_hidden(&input.hidden, k)) =>
            {
                progress_text(def, rule, input).unwrap_or_else(|| def.goal.clone())
            }
            _ => def.goal.clone(),
        },
    }
}

/// Whose wall it is. A public wall is what anyone sees of a member: locked
/// secrets and accolades whose input never answered are left off, and a hint
/// never carries a hidden number or one the public view does not read. An own
/// wall is the owner's: every locked accolade is listed (a locked secret as
/// `???` with a lock and its cryptic hint), hints carry the owner's real numbers
/// (drops and points too), hidden switches are ignored, and the seasons and the
/// cake day light from today's date alone (the server grants them at sign-in).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WallMode {
    Public,
    Own,
}

/// What an own wall shows in place of a locked secret's name and medal.
const MYSTERY_LABEL: &str = "???";
const MYSTERY_ICON: &str = "lock";

/// Whether a month-and-day (`month * 100 + day`) falls in a seasonal window,
/// which may wrap the new year.
fn in_window(md: u32, w: &Window) -> bool {
    let from = w.from_m * 100 + w.from_d;
    let to = w.to_m * 100 + w.to_d;
    if from <= to {
        md >= from && md <= to
    } else {
        md >= from || md <= to
    }
}

/// The accolades an own wall lights from `today` alone: every season whose window
/// holds it, and Cake Day when the account's creation month and day (read in the
/// zone `created_at` is written in) is today's.
fn optimistic_ids(cat: &Catalog, input: &WallInput) -> Vec<String> {
    let Ok(today) = NaiveDate::parse_from_str(&input.today, "%Y-%m-%d") else { return Vec::new() };
    let md = today.month() * 100 + today.day();
    let created_md = input
        .created_at
        .as_deref()
        .and_then(|c| DateTime::parse_from_rfc3339(c).ok())
        .map(|c| c.month() * 100 + c.day());
    cat.accolades
        .iter()
        .filter(|d| match d.kind {
            Kind::Seasonal => d.window.as_ref().is_some_and(|w| in_window(md, w)),
            Kind::Cake => created_md == Some(md),
            _ => false,
        })
        .map(|d| d.id.clone())
        .collect()
}

/// What an own wall says under a medallion: the owner's real numbers for every
/// stat they can read, the goal alone for one whose input never answered, and a
/// secret's own cryptic hint.
fn own_hint(cat: &Catalog, def: &AccoladeDef, earned: bool, input: &WallInput, unknown: &[StatKind]) -> String {
    let hint = def.hint.clone().unwrap_or_default();
    match def.kind {
        Kind::Secret | Kind::Offline => hint,
        Kind::Seasonal | Kind::Cake => {
            if earned {
                cat.titles.collected.clone()
            } else {
                hint
            }
        }
        Kind::Event => {
            if earned {
                def.hint_earned.clone().unwrap_or(hint)
            } else {
                hint
            }
        }
        Kind::Base => match &def.rule {
            RuleDef::Stat(rule) if !unknown.contains(&rule.stat) => {
                progress_text(def, rule, input).unwrap_or_else(|| def.goal.clone())
            }
            _ => def.goal.clone(),
        },
    }
}

/// The accolade wall for a member from the shipped catalog, public or the
/// owner's own.
pub fn accolade_wall(input: &WallInput, mode: WallMode) -> AccoladeWall {
    wall_core(&CATALOG, input, &[], mode)
}

/// The public accolade wall against a given catalog. Locked secrets and
/// accolades nobody can earn any more (events that are over, member-number ones
/// the number does not qualify for) are left off unless held.
pub fn wall_with(cat: &Catalog, input: &WallInput) -> AccoladeWall {
    wall_with_unknowns(cat, input, &[])
}

/// `wall_with` for a build where some inputs could not be read (years when IVR
/// failed, cosmetics when the 7TV inventory failed) and no cached copy stands in.
/// An accolade whose rule reads an unknown stat is lit if it was saved and
/// otherwise left off the wall, never shown locked at 0; a saved one says its
/// goal, not a progress number. The counts follow the items.
pub fn wall_with_unknowns(cat: &Catalog, input: &WallInput, unknown: &[StatKind]) -> AccoladeWall {
    wall_core(cat, input, unknown, WallMode::Public)
}

/// The wall in either mode. `unknown` lists the stats whose input never
/// answered: public leaves their accolades off unless saved, own lists them
/// locked with the goal text (lit if saved), and Triple Threat is never reached
/// while Drops is unknown.
pub fn wall_core(cat: &Catalog, input: &WallInput, unknown: &[StatKind], mode: WallMode) -> AccoladeWall {
    let own = mode == WallMode::Own;
    let optimistic: Vec<String> = if own { optimistic_ids(cat, input) } else { Vec::new() };
    let mut saved: HashSet<&str> = input.saved.iter().map(String::as_str).collect();
    saved.extend(optimistic.iter().map(String::as_str));
    let number = input.member_number;
    let name = number_name(cat, number);
    let unknown_input = |def: &AccoladeDef| matches!(&def.rule, RuleDef::Stat(rule) if unknown.contains(&rule.stat));
    let reached = |def: &AccoladeDef, all_base: bool| -> bool {
        match &def.rule {
            RuleDef::Stat(rule) => !unknown.contains(&rule.stat) && stat_value(input, rule.stat) >= rule.n as f64,
            RuleDef::Special(rule) => match rule.special {
                Special::FirstHundred => number.is_some_and(|n| n <= 100),
                Special::Completionist => all_base,
                // The desktop's Triple Threat: messages, hours and drops all run deep.
                Special::Triple => {
                    input.messages >= 1_000
                        && input.hours >= 100.0
                        && !unknown.contains(&StatKind::Drops)
                        && input.drops >= 25
                }
                Special::Palindrome => is_palindrome(number),
                Special::Nice => name.is_some(),
                Special::Saved => false,
            },
        }
    };
    let lit = |def: &AccoladeDef, all_base: bool| saved.contains(def.id.as_str()) || reached(def, all_base);
    let out_of_reach = |def: &AccoladeDef| -> bool {
        def.ended
            || match &def.rule {
                RuleDef::Special(rule) => match rule.special {
                    Special::FirstHundred => !number.is_some_and(|n| n <= 100),
                    Special::Palindrome => !is_palindrome(number),
                    Special::Nice => name.is_none(),
                    _ => false,
                },
                RuleDef::Stat(_) => false,
            }
    };
    // Completionist counts only the base accolades this member can still earn; one
    // whose input is unknown and that is not saved keeps it from lighting.
    let all_base =
        cat.accolades.iter().filter(|d| d.kind == Kind::Base).all(|d| lit(d, false) || out_of_reach(d));
    let mut items = Vec::new();
    for def in &cat.accolades {
        let earned = lit(def, all_base);
        if (!earned && out_of_reach(def)) || (!own && !earned && (unknown_input(def) || def.kind == Kind::Secret)) {
            continue;
        }
        let mystery = own && def.kind == Kind::Secret && !earned;
        let label = match &def.rule {
            _ if mystery => MYSTERY_LABEL.to_string(),
            RuleDef::Special(rule) if rule.special == Special::Nice => name.clone().unwrap_or_else(|| def.label.clone()),
            _ => def.label.clone(),
        };
        items.push(AccoladeMedal {
            id: def.id.clone(),
            label,
            icon: if mystery { MYSTERY_ICON.to_string() } else { def.icon.clone() },
            colors: def.colors.clone(),
            earned,
            secret: def.kind == Kind::Secret,
            hint: if own {
                own_hint(cat, def, earned, input, unknown)
            } else {
                public_hint(cat, def, earned, input, unknown)
            },
        });
    }
    items.sort_by_key(|m| !m.earned);
    let earned = items.iter().filter(|m| m.earned).count() as u32;
    AccoladeWall { earned, total: items.len() as u32, items }
}

// ---- The viewer's clock ------------------------------------------------------------

/// The zone "today" and a join date are read in.
#[derive(Clone, Copy, Debug)]
pub enum Zone {
    /// The platform's own zone (DST-correct where its rules are readable).
    Local,
    /// A fixed offset the viewer's device reported.
    Fixed(FixedOffset),
}

impl Zone {
    /// The zone for a viewer who reports `tz_offset_min` (minutes east of UTC;
    /// None from a caller that does not say). The platform's own zone is used
    /// while its current offset equals the reported one; when it differs, the
    /// platform cannot read its zone (an Android or iPhone core), so the
    /// reported offset stands.
    pub fn viewer(tz_offset_min: Option<i32>) -> Zone {
        let Some(minutes) = tz_offset_min else { return Zone::Local };
        let seconds = minutes.saturating_mul(60);
        if Local::now().offset().local_minus_utc() == seconds {
            return Zone::Local;
        }
        Zone::Fixed(FixedOffset::east_opt(seconds).unwrap_or_else(|| FixedOffset::east_opt(0).expect("zero offset")))
    }

    /// The calendar date an instant falls on in this zone.
    pub fn date(&self, at: DateTime<Utc>) -> NaiveDate {
        match self {
            Zone::Local => at.with_timezone(&Local).date_naive(),
            Zone::Fixed(offset) => at.with_timezone(offset).date_naive(),
        }
    }
}

/// An instant and the zone to read it in.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub now: DateTime<Utc>,
    pub zone: Zone,
}

impl Clock {
    pub fn system(tz_offset_min: Option<i32>) -> Clock {
        Clock { now: Utc::now(), zone: Zone::viewer(tz_offset_min) }
    }

    pub fn today(&self) -> NaiveDate {
        self.zone.date(self.now)
    }
}

// ---- The shape the phones draw -----------------------------------------------------

/// One overview tile.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MemberStat {
    /// The stat's visibility key (`messages`, `streams`, `favorite_channel`,
    /// `subs_gifted`, `twitch_age`, `followers`, `account_type`, `cosmetics`,
    /// `member_rank`), which also picks its icon.
    pub key: String,
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// Only on `twitch_age`: "Joined March 20, 2014".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A titled card of tiles: `twitch` or `lifetime`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StatSection {
    pub key: String,
    pub title: String,
    pub tiles: Vec<MemberStat>,
}

/// The big watch-time number and the line under it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HoursHero {
    /// "Hours watched", the catalog's `titles.hours`.
    pub label: String,
    pub value: String,
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roast: Option<String>,
}

/// One of their most-used emotes.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TopEmote {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub url: String,
    pub count: u64,
    /// `count` grouped ("1,234"), for the tap panel.
    #[serde(default)]
    pub count_text: String,
}

/// The member sections of a profile sheet. Each part is absent (or empty) when
/// the member hides it, has nothing in it, or its read did not answer.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct MemberProfile {
    /// Their relics with art, the one they feature first.
    #[serde(default)]
    pub relics: Vec<ProfileBadge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hours: Option<HoursHero>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twitch: Option<StatSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime: Option<StatSection>,
    /// How many times other people opened their profile, grouped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub views: Option<String>,
    /// At most sixteen with art, most used first (the phones show twelve after dropping any art that fails).
    #[serde(default)]
    pub top_emotes: Vec<TopEmote>,
    /// "12 emotes · 345 used".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emote_totals: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accolades: Option<AccoladeWall>,
}

// ---- Parts and facts --------------------------------------------------------------

/// One read of a member's profile and whether it answered. `Failed` is never
/// zero: a section built from it is left out, or taken from the cached sheet.
#[derive(Clone, Debug, PartialEq)]
pub enum Part<T> {
    Answered(T),
    Failed,
}

impl<T> Default for Part<T> {
    fn default() -> Self {
        Part::Failed
    }
}

impl<T> Part<T> {
    pub fn from_result(read: Result<T, String>, what: &str) -> Self {
        match read {
            Ok(v) => Part::Answered(v),
            Err(e) => {
                log::debug!("[MemberProfile] {what} unavailable: {e}");
                Part::Failed
            }
        }
    }

    pub fn as_answer(&self) -> Option<&T> {
        match self {
            Part::Answered(v) => Some(v),
            Part::Failed => None,
        }
    }

    pub fn is_answered(&self) -> bool {
        matches!(self, Part::Answered(_))
    }

    /// A part that did not answer takes the cached copy's state and value.
    pub fn or_cached(&mut self, cached: &Part<T>)
    where
        T: Clone,
    {
        if !self.is_answered() {
            *self = cached.clone();
        }
    }
}

/// Everything read for one member, before it is shaped.
#[derive(Clone, Debug, Default)]
pub struct MemberFacts {
    pub member_id: String,
    pub number: Option<u32>,
    pub hidden: Vec<String>,
    /// Their `user_stats` row. `Answered(None)` is a member with no row yet:
    /// zeros, as the desktop card shows.
    pub stats: Part<Option<Value>>,
    /// Their most-watched `user_channel_watch` row (`Answered(None)`: none yet).
    pub favorite: Part<Option<Value>>,
    pub saved: Part<HashSet<String>>,
    pub ivr: Part<Option<IvrUserSummary>>,
    /// Their `user_emote_usage` rows, most used first.
    pub emotes: Part<Vec<Value>>,
    /// How many emotes they have used and how many times: (unique, sum).
    pub totals: Part<(u64, u64)>,
    /// Their `user_cosmetic_equipment` rows.
    pub equipment: Part<Vec<Value>>,
    /// 7TV paints and badges owned: (paints, badges). Filled by the sheet build.
    pub seventv: Part<(u64, u64)>,
    /// The count the view itself returned, when this open counted one.
    pub counted_views: Option<u64>,
}

/// A fresh read with the cached copy's answers for any part that failed this
/// time. Visibility (`hidden`) and the member number always come from the fresh
/// read: it is the one that answered the preferences.
pub fn merge_facts(mut new: MemberFacts, old: &MemberFacts) -> MemberFacts {
    new.stats.or_cached(&old.stats);
    new.favorite.or_cached(&old.favorite);
    new.saved.or_cached(&old.saved);
    new.ivr.or_cached(&old.ivr);
    new.emotes.or_cached(&old.emotes);
    new.totals.or_cached(&old.totals);
    new.equipment.or_cached(&old.equipment);
    new.seventv.or_cached(&old.seventv);
    new
}

/// The slug equipped in a slot (`frame`, `relic_primary`), when the equipment
/// read answered and the slot holds one.
pub fn equipped_slot(f: &MemberFacts, slot: &str) -> Option<String> {
    f.equipment
        .as_answer()?
        .iter()
        .find(|r| r.get("slot").and_then(Value::as_str) == Some(slot))
        .and_then(|r| r.get("cosmetic_slug").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// The inputs to shaping that are not facts about the member.
pub struct ShapeCtx<'a> {
    pub display_name: &'a str,
    pub login: &'a str,
    /// Helix's `broadcaster_type` for the account, "" when it said nothing.
    pub broadcaster_type: &'a str,
    pub clock: &'a Clock,
    pub catalog: &'a Catalog,
}

/// What shaping a member's facts gives back.
pub struct Shaped {
    pub profile: MemberProfile,
    /// The unrounded hours the roast is picked for; None when the hours or the
    /// roast line are hidden (or the hours are unknown).
    pub roast_hours: Option<f64>,
    /// Whether they hide their badge collection (which hides every badge group).
    pub hide_badges: bool,
}

// ---- The overview ------------------------------------------------------------------

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

/// A stat tile's number: grouped up to 9,999,999, then three significant digits
/// ("12.3M", "123M", "1.23B") so a tile never ends in an ellipsis. Truncated,
/// never rounded up: 99,960,000 reads "99.9M", not "100.0M".
pub fn tile_number(n: u64) -> String {
    match n {
        0..=9_999_999 => grouped(n),
        10_000_000..=99_999_999 => format!("{}.{}M", n / 1_000_000, n / 100_000 % 10),
        100_000_000..=999_999_999 => format!("{}M", n / 1_000_000),
        1_000_000_000..=9_999_999_999 => format!("{}.{:02}B", n / 1_000_000_000, n / 10_000_000 % 100),
        10_000_000_000..=99_999_999_999 => format!("{}.{}B", n / 1_000_000_000, n / 100_000_000 % 10),
        _ => format!("{}B", n / 1_000_000_000),
    }
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
        0 => "Cake day is today. Make a wish.".into(),
        1 => "Cake day tomorrow".into(),
        n => format!("Cake day in {n} days"),
    }
}

/// "Tyler's Twitch", "Chris' Twitch": the catalog's title with the possessive
/// of the name filled in.
fn twitch_title(template: &str, name: &str) -> String {
    let possessive = if name.ends_with('s') || name.ends_with('S') { format!("{name}'") } else { format!("{name}'s") };
    if template.contains("{name}'s") {
        template.replace("{name}'s", &possessive)
    } else {
        template.replace("{name}", name)
    }
}

/// The account type the tile shows: Twitch's own roles first (IVR is reliable),
/// then Helix's often-empty `broadcaster_type`.
pub fn account_type(ivr: Option<&IvrUserSummary>, broadcaster_type: &str) -> &'static str {
    let (staff, partner, affiliate) = ivr.map_or((false, false, false), |i| (i.is_staff, i.is_partner, i.is_affiliate));
    if staff {
        "Twitch Staff"
    } else if partner || broadcaster_type == "partner" {
        "Partner"
    } else if affiliate || broadcaster_type == "affiliate" {
        "Affiliate"
    } else {
        "Viewer"
    }
}

const TOP_EMOTES: usize = 16;
/// Emote rows read, so a few without art still leave sixteen to send.
const EMOTES_READ: usize = 32;

fn emote_of(r: &Value) -> Option<TopEmote> {
    let text = |k: &str| r.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from);
    let count = num(Some(r), "count").max(0.0) as u64;
    Some(TopEmote {
        id: text("emote_id")?,
        name: text("emote_name").unwrap_or_default(),
        provider: text("provider").unwrap_or_default(),
        url: text("image_url")?,
        count,
        count_text: grouped(count),
    })
}

/// How many emotes they have used and how many times, from their usage rows.
pub fn totals_of(rows: &[Value]) -> (u64, u64) {
    (rows.len() as u64, rows.iter().map(|r| num(Some(r), "count").max(0.0) as u64).sum())
}

/// The wall's inputs from a member's facts, with the dates read in the clock's
/// zone. `drops` and `points` are None when their read never answered. The second
/// value lists the stats whose input never answered (years when IVR failed,
/// cosmetics when the 7TV inventory did, and the two here), for `wall_core`. The
/// public sheet passes `Some(0)` for both: it never reads them.
pub fn wall_input(f: &MemberFacts, clock: &Clock, drops: Option<u64>, points: Option<u64>) -> (WallInput, Vec<StatKind>) {
    let row: Option<&Value> = f.stats.as_answer().and_then(|r| r.as_ref());
    let ivr: Option<&IvrUserSummary> = f.ivr.as_answer().and_then(|i| i.as_ref());
    let today = clock.today();
    let created: Option<NaiveDate> = ivr
        .and_then(|i| i.created_at.as_deref())
        .and_then(|c| DateTime::parse_from_rfc3339(c).ok())
        .map(|c| clock.zone.date(c.with_timezone(&Utc)));
    let input = WallInput {
        messages: (num(row, "messages_sent") + num(row, "recap_messages")).max(0.0) as u64,
        hours: (num(row, "hours_watched") + num(row, "recap_hours")).max(0.0),
        streams: num(row, "streams_watched").max(0.0) as u64,
        drops: drops.unwrap_or(0),
        points: points.unwrap_or(0),
        cosmetics: f.seventv.as_answer().map_or(0, |(p, b)| p + b) + registry::owned_slugs(&f.member_id).len() as u64,
        years: created.map_or(0, |c| account_age(c, today).1),
        member_number: f.number,
        saved: f.saved.as_answer().map(|s| s.iter().cloned().collect()).unwrap_or_default(),
        hidden: f.hidden.clone(),
        today: today.to_string(),
        created_at: created.map(|c| format!("{c}T00:00:00Z")),
    };
    let mut unknown = Vec::new();
    if !f.ivr.is_answered() {
        unknown.push(StatKind::Years);
    }
    if !f.seventv.is_answered() {
        unknown.push(StatKind::Cosmetics);
    }
    if drops.is_none() {
        unknown.push(StatKind::Drops);
    }
    if points.is_none() {
        unknown.push(StatKind::Points);
    }
    (input, unknown)
}

/// Works a member's facts into what the sheet shows. A part that did not answer
/// leaves its tiles out; nothing here turns a failed read into a zero.
pub fn shape(f: &MemberFacts, ctx: &ShapeCtx) -> Shaped {
    let cat = ctx.catalog;
    let hidden = |k: &str| is_stat_hidden(&f.hidden, k);
    let row: Option<&Value> = f.stats.as_answer().and_then(|r| r.as_ref());
    let stats_known = f.stats.is_answered();
    let hours_f = (num(row, "hours_watched") + num(row, "recap_hours")).max(0.0);
    let messages = (num(row, "messages_sent") + num(row, "recap_messages")).max(0.0) as u64;
    let streams = num(row, "streams_watched").max(0.0) as u64;
    let subs_gifted = num(row, "recap_subs_gifted").max(0.0) as u64;
    let today = ctx.clock.today();
    let ivr: Option<&IvrUserSummary> = f.ivr.as_answer().and_then(|i| i.as_ref());
    let created: Option<NaiveDate> = ivr
        .and_then(|i| i.created_at.as_deref())
        .and_then(|c| DateTime::parse_from_rfc3339(c).ok())
        .map(|c| ctx.clock.zone.date(c.with_timezone(&Utc)));
    let age = created.map(|c| account_age(c, today));
    let owned = registry::owned_slugs(&f.member_id);
    let entries = registry::catalog();
    let tile = |key: &str, value: String, caption: Option<String>, detail: Option<String>| -> MemberStat {
        let def = cat.stat(key);
        MemberStat { key: key.to_string(), label: def.label, value, caption: caption.or(def.caption), detail }
    };

    let hours = (stats_known && !hidden("hours")).then(|| HoursHero {
        label: cat.titles.hours.clone(),
        value: grouped(hours_f.round() as u64),
        unit: cat.titles.hours_unit.clone(),
        roast: None,
    });
    let roast_hours = (hours.is_some() && !hidden("roast_line")).then_some(hours_f);

    let mut twitch_tiles = Vec::new();
    if let (Some((label, _, days)), Some(c)) = (age.as_ref(), created) {
        if !hidden("twitch_age") {
            twitch_tiles.push(tile(
                "twitch_age",
                label.clone(),
                Some(cake_caption(*days)),
                Some(format!("Joined {}", c.format("%B %-d, %Y"))),
            ));
        }
    }
    if let Some(followers) = ivr.and_then(|i| i.followers) {
        if !hidden("followers") {
            twitch_tiles.push(tile("followers", tile_number(followers.max(0) as u64), None, None));
        }
    }
    let bt = ctx.broadcaster_type;
    if (f.ivr.is_answered() || bt == "partner" || bt == "affiliate") && !hidden("account_type") {
        twitch_tiles.push(tile("account_type", account_type(ivr, bt).to_string(), None, None));
    }
    let name = if ctx.display_name.is_empty() { ctx.login } else { ctx.display_name };
    let twitch = (!twitch_tiles.is_empty()).then(|| StatSection {
        key: "twitch".to_string(),
        title: twitch_title(&cat.titles.twitch, name),
        tiles: twitch_tiles,
    });

    let mut lifetime_tiles = Vec::new();
    if stats_known {
        if !hidden("messages") {
            lifetime_tiles.push(tile("messages", tile_number(messages), None, None));
        }
        if !hidden("streams") {
            lifetime_tiles.push(tile("streams", tile_number(streams), None, None));
        }
        if subs_gifted > 0 && !hidden("subs_gifted") {
            lifetime_tiles.push(tile("subs_gifted", tile_number(subs_gifted), None, None));
        }
    }
    if let Some((paints, badges)) = f.seventv.as_answer() {
        if !hidden("cosmetics") {
            let sn = owned.len() as u64;
            lifetime_tiles.push(tile(
                "cosmetics",
                tile_number(paints + badges + sn),
                Some(format!("{}, {}, {sn} StreamNook", plural(*paints as i64, "paint"), plural(*badges as i64, "badge"))),
                None,
            ));
        }
    }
    if let Some(Some(fav)) = f.favorite.as_answer() {
        let text = |k: &str| fav.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        if let Some(channel) = text("channel_name").or_else(|| text("channel_login")) {
            if !hidden("favorite_channel") {
                let watched = (num(Some(fav), "minutes") / 60.0).round().max(0.0) as u64;
                lifetime_tiles.push(tile(
                    "favorite_channel",
                    channel.to_string(),
                    Some(format!("{}h watched", grouped(watched))),
                    None,
                ));
            }
        }
    }
    if let Some(number) = f.number {
        if !hidden("member_rank") {
            lifetime_tiles.push(tile(
                "member_rank",
                format!("#{}", grouped(u64::from(number))),
                Some(member_rank(number).label),
                None,
            ));
        }
    }
    let lifetime = (!lifetime_tiles.is_empty()).then(|| StatSection {
        key: "lifetime".to_string(),
        title: cat.titles.lifetime.clone(),
        tiles: lifetime_tiles,
    });

    let views = if hidden("views") {
        None
    } else {
        f.counted_views.or_else(|| row.and_then(|s| s.get("profile_views")).and_then(Value::as_u64)).map(grouped)
    };

    let top_emotes: Vec<TopEmote> = if hidden("emotes") {
        Vec::new()
    } else {
        f.emotes.as_answer().map(|rows| rows.iter().filter_map(emote_of).take(TOP_EMOTES).collect()).unwrap_or_default()
    };
    let emote_totals = (!hidden("emotes"))
        .then(|| f.totals.as_answer().copied())
        .flatten()
        .filter(|(unique, _)| *unique > 0)
        .map(|(unique, used)| format!("{} emotes · {} used", grouped(unique), grouped(used)));

    let accolades = (!hidden("accolades") && stats_known && f.saved.is_answered()).then(|| {
        // An input that never answered (and has no cached copy) is not read as 0.
        let (input, unknown) = wall_input(f, ctx.clock, Some(0), Some(0));
        wall_with_unknowns(cat, &input, &unknown)
    });

    let relics = if hidden("relics") {
        Vec::new()
    } else {
        relics(&owned, equipped_slot(f, "relic_primary").as_deref(), &entries)
    };

    Shaped {
        profile: MemberProfile {
            relics,
            hours,
            twitch,
            lifetime,
            views,
            top_emotes,
            emote_totals,
            accolades,
        },
        roast_hours,
        hide_badges: hidden("badge_collection"),
    }
}

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

/// The first row of a read: an error when the read failed, `Ok(None)` when it
/// answered with no row (a member who has none yet).
pub(crate) async fn first_row(table: &str, query: String) -> Result<Option<Value>, String> {
    Ok(crate::services::streamnook_backend::select(table, &query).await?.into_iter().next())
}

/// Emote totals, per member: one read at a time, kept ten minutes, a slow read
/// finishing in its own task. The count is the number of rows, the sum their
/// `count` column, paged in a stable order (the table's key is
/// `(twitch_user_id, emote_id)`); PostgREST aggregates are off for the public key.
static TOTALS: LazyLock<SharedLookup<(u64, u64)>> = LazyLock::new(|| SharedLookup::new(512));
const TOTALS_TTL: Duration = Duration::from_secs(600);
const TOTALS_BUDGET: Duration = Duration::from_secs(2);

async fn read_totals(id: String) -> Result<(u64, u64), String> {
    let rows = crate::services::streamnook_backend::select_all(
        "user_emote_usage",
        &format!("select=count&twitch_user_id=eq.{id}&order=emote_id.asc"),
    )
    .await?;
    Ok(totals_of(&rows))
}

async fn emote_totals(id: &str) -> Part<(u64, u64)> {
    let owned = id.to_string();
    match TOTALS.lookup(&format!("totals:{id}"), TOTALS_TTL, TOTALS_BUDGET, move || read_totals(owned)).await {
        Lookup::Fresh(v) | Lookup::Stale(v) => Part::Answered(v),
        Lookup::Failed(e) => {
            log::debug!("[MemberProfile] emote totals unavailable: {e}");
            Part::Failed
        }
        Lookup::Pending => Part::Failed,
    }
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

/// How a member's reads ended.
pub enum MemberRead {
    /// Not a member: no member half, and nothing cached stands in for it.
    NotMember,
    /// Could not tell (the member list never loaded) or their preferences did
    /// not answer: what they hide must never show, so the build has no member
    /// half (a cached one was shaped by what they hid then).
    Unknown,
    Read(MemberFacts),
}

/// Everything a member's sheet reads for them, each read its own part.
pub async fn member_reads(member_id: &str, login: &str) -> MemberRead {
    // A cold launch may open a sheet before the member list has loaded.
    registry::ready(Duration::from_secs(3)).await;
    if !registry::members_loaded() {
        return MemberRead::Unknown;
    }
    let Some(number) = registry::member_number(member_id) else { return MemberRead::NotMember };
    let id = digits(member_id);
    let equipment_query = format!("select=slot,cosmetic_slug&twitch_user_id=eq.{id}");
    let emotes_query =
        format!("select=emote_id,emote_name,provider,image_url,count&twitch_user_id=eq.{id}&order=count.desc&limit={EMOTES_READ}");
    let (prefs, stats, favorite, saved, equipment, emotes, ivr, totals, counted) = tokio::join!(
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
                Err("no login to ask about".to_string())
            } else {
                crate::services::ivr::user_summary(login).await
            }
        },
        emote_totals(&id),
        count_view(member_id),
    );
    let prefs = match prefs {
        Ok(prefs) => prefs,
        Err(e) => {
            log::warn!("[MemberProfile] prefs unavailable, member sections left out: {e}");
            return MemberRead::Unknown;
        }
    };
    MemberRead::Read(MemberFacts {
        member_id: member_id.to_string(),
        number: Some(number),
        hidden: prefs.hidden_sections,
        stats: Part::from_result(stats, "user_stats"),
        favorite: Part::from_result(favorite, "user_channel_watch"),
        saved: Part::from_result(saved.map(|ids| ids.into_iter().collect()), "user_accolades"),
        ivr: Part::from_result(ivr, "ivr"),
        emotes: Part::from_result(emotes, "user_emote_usage"),
        totals,
        equipment: Part::from_result(equipment, "user_cosmetic_equipment"),
        seventv: Part::Failed,
        counted_views: counted,
    })
}

// ---- Tests -------------------------------------------------------------------------

/// A small catalog the tests (and the golden sheet in `profile_view`) run against,
/// so they hold whatever the shipped JSON says. Same shape, same strictness.
#[cfg(test)]
pub(crate) const TEST_CATALOG_JSON: &str = r##"{
  "v": 1,
  "titles": {
    "hours": "Hours watched", "hours_unit": "hrs", "twitch": "{name}'s Twitch", "twitch_own": "Your Twitch",
    "lifetime": "Lifetime in StreamNook", "top_emotes": "Top Emotes", "accolades": "Accolades",
    "relics": "Relics", "hidden_secret_hint": "A hidden accolade.", "collected": "Collected"
  },
  "stats": [
    { "key": "hours", "label": "Hours watched", "icon": "clock" },
    { "key": "twitch_age", "label": "Twitch age", "icon": "cake" },
    { "key": "followers", "label": "Followers", "icon": "users" },
    { "key": "account_type", "label": "Account type", "icon": "award" },
    { "key": "messages", "label": "Messages sent", "icon": "message-square", "caption": "into the void" },
    { "key": "streams", "label": "Streams watched", "icon": "tv" },
    { "key": "subs_gifted", "label": "Subs gifted", "icon": "hand-heart" },
    { "key": "cosmetics", "label": "Cosmetics", "icon": "palette" },
    { "key": "favorite_channel", "label": "Favorite channel", "icon": "heart" },
    { "key": "member_rank", "label": "Member rank", "icon": "sparkles" },
    { "key": "views", "label": "Profile views", "icon": "eye" }
  ],
  "member_number_names": { "exact": { "69": "Nice", "420": "Blaze It" }, "ends_in_67": "Six Seven-ish" },
  "accolades": [
    { "id": "chatterbox", "label": "Chatterbox", "icon": "message-circle", "colors": ["#6aa9ff", "#3b6fd4"], "kind": "base",
      "rule": { "stat": "messages", "n": 1000 }, "progress": "Messages sent", "goal": "Send 1,000 messages" },
    { "id": "marathoner", "label": "Marathoner", "icon": "timer", "colors": ["#f59e0b", "#b45309"], "kind": "base",
      "rule": { "stat": "hours", "n": 100 }, "progress": "Hours watched", "goal": "Watch 100 hours" },
    { "id": "dropgoblin", "label": "Drop Goblin", "icon": "package", "colors": ["#22c55e", "#15803d"], "kind": "base",
      "rule": { "stat": "drops", "n": 25 }, "progress": "Drops claimed", "goal": "Claim 25 drops" },
    { "id": "collector", "label": "Collector", "icon": "shapes", "colors": ["#ec4899", "#be185d"], "kind": "base",
      "rule": { "stat": "cosmetics", "n": 5 }, "progress": "Cosmetics collected", "goal": "Collect 5 cosmetics" },
    { "id": "veteran", "label": "Veteran", "icon": "medal", "colors": ["#fb923c", "#c2410c"], "kind": "base",
      "rule": { "stat": "years", "n": 5 }, "progress": "Years on Twitch", "goal": "Reach 5 years on Twitch" },
    { "id": "og", "label": "OG", "icon": "crown", "colors": ["#fbbf24", "#d97706"], "kind": "base",
      "rule": { "special": "first_hundred" }, "goal": "Be one of the first 100 StreamNook members" },
    { "id": "completionist", "label": "Completionist", "icon": "star", "colors": ["#f472b6", "#db2777"], "kind": "secret",
      "rule": { "special": "completionist" }, "hint": "When there's nothing else left to earn, this one shows up." },
    { "id": "triple", "label": "Triple Threat", "icon": "layers", "colors": ["#34d399", "#0f766e"], "kind": "secret",
      "rule": { "special": "triple" }, "hint": "Talk, watch, and collect until all three run deep." },
    { "id": "nice", "label": "Nice", "icon": "smile", "colors": ["#facc15", "#ca8a04"], "kind": "secret",
      "rule": { "special": "nice" }, "hint": "Land on a number the internet never lets you forget." },
    { "id": "halloween", "label": "Halloween", "icon": "skull", "colors": ["#fb923c", "#c2410c"], "kind": "seasonal",
      "rule": { "special": "saved" }, "hint": "Open StreamNook around Halloween (Oct 31)",
      "window": { "from_m": 10, "from_d": 30, "to_m": 11, "to_d": 1 } },
    { "id": "cs2_major_cologne_2026", "label": "CS2 Major Cologne 2026", "icon": "trophy", "colors": ["#f6e3a0", "#9c7528"],
      "kind": "event", "rule": { "special": "saved" }, "hint": "Watch Counter-Strike in StreamNook during the CS2 Major",
      "hint_earned": "Tuned in during the CS2 Major", "ended": true }
  ]
}"##;

/// A small roast pool for the tests, in the shipped file's shape.
#[cfg(test)]
pub(crate) const TEST_ROASTS_JSON: &str = r#"{
  "divisors": { "days": 24, "weeks": 40, "movies": 2, "seasons": 9, "flights": 5 },
  "low_hours_max": 2,
  "low_hours": ["Low a", "Low b"],
  "pool": [
    { "tone": "roast", "requires": "days", "text": "R {days}" },
    { "tone": "roast", "text": "R flat" },
    { "tone": "neutral", "requires": "weeks", "text": "N {weeks}" },
    { "tone": "cute", "text": "C flat" }
  ]
}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::cosmetics_registry::{entry_for_test, set_for_test, TEST_LOCK};
    use chrono::TimeZone;
    use serde_json::json;

    fn keys(stored: &[&str]) -> Vec<String> {
        stored.iter().map(|s| s.to_string()).collect()
    }

    fn catalog() -> Catalog {
        Catalog::parse(TEST_CATALOG_JSON).expect("the test catalog")
    }

    fn utc() -> Zone {
        Zone::Fixed(FixedOffset::east_opt(0).unwrap())
    }

    fn clock(y: i32, m: u32, d: u32) -> Clock {
        Clock { now: Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap(), zone: utc() }
    }

    /// Member "100" (number 7) owns the default badge and one relic.
    fn registry_fixture() {
        set_for_test(
            &[("100", 7)],
            vec![
                entry_for_test(registry::DEFAULT_SLUG, "badge", "streamnook-logo.png", true),
                entry_for_test("relic-gem", "relic", "https://cdn.streamnook.app/relics/gem.webp", false),
            ],
            &[("100", "relic-gem")],
            &[],
            vec![],
        );
    }

    fn ivr(created: &str) -> IvrUserSummary {
        IvrUserSummary {
            followers: Some(1_234_567),
            created_at: Some(created.into()),
            is_affiliate: false,
            is_partner: false,
            is_staff: false,
        }
    }

    fn facts() -> MemberFacts {
        MemberFacts {
            member_id: "100".into(),
            number: Some(7),
            stats: Part::Answered(Some(json!({
                "hours_watched": 1200.4, "recap_hours": 99.7, "messages_sent": 9500, "recap_messages": "600",
                "streams_watched": 300, "recap_subs_gifted": 5, "profile_views": 4321,
            }))),
            favorite: Part::Answered(Some(json!({ "channel_login": "xqc", "channel_name": "xQc", "minutes": 18_750 }))),
            saved: Part::Answered(HashSet::new()),
            ivr: Part::Answered(Some(ivr("2014-03-20T10:00:00Z"))),
            emotes: Part::Answered(vec![
                json!({ "emote_id": "e1", "emote_name": "OMEGALUL", "provider": "bttv", "image_url": "https://x/1", "count": 90 }),
                json!({ "emote_id": "e2", "emote_name": "nourl", "provider": "7tv", "image_url": null, "count": 50 }),
            ]),
            totals: Part::Answered((12, 345)),
            equipment: Part::Answered(vec![]),
            seventv: Part::Answered((3, 2)),
            ..Default::default()
        }
    }

    fn shaped(f: &MemberFacts, clock: &Clock) -> Shaped {
        let cat = catalog();
        let ctx = ShapeCtx { display_name: "Tyler", login: "tyler", broadcaster_type: "", clock, catalog: &cat };
        shape(f, &ctx)
    }

    fn tiles(section: &Option<StatSection>) -> Vec<(&str, &str, Option<&str>)> {
        section
            .as_ref()
            .map(|s| s.tiles.iter().map(|t| (t.key.as_str(), t.value.as_str(), t.caption.as_deref())).collect())
            .unwrap_or_default()
    }

    fn seq(values: &[f64]) -> impl FnMut() -> f64 + '_ {
        let mut i = 0;
        move || {
            let v = values[i];
            i += 1;
            v
        }
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

    #[test]
    fn the_shipped_catalog_parses_strictly_and_names_every_tile_the_sheet_builds() {
        let cat = &*CATALOG;
        for key in [
            "hours", "twitch_age", "followers", "account_type", "messages", "streams", "subs_gifted", "cosmetics",
            "favorite_channel", "member_rank", "views",
        ] {
            assert!(cat.stats.iter().any(|s| s.key == key), "no stat tile {key} in the catalog");
        }
        let mut ids = HashSet::new();
        for a in &cat.accolades {
            assert!(ids.insert(a.id.as_str()), "{} is listed twice", a.id);
            assert!(a.colors.iter().all(|c| c.len() == 7 && c.starts_with('#')), "{} colors", a.id);
            match (&a.rule, a.kind) {
                (RuleDef::Stat(r), Kind::Base) if shows_progress(r.stat) => {
                    assert!(a.progress.is_some(), "{} needs a progress label", a.id)
                }
                (_, Kind::Base) => assert!(!a.goal.is_empty(), "{} needs a goal", a.id),
                (_, Kind::Seasonal) => assert!(a.window.is_some() && a.hint.is_some(), "{} needs a window and hint", a.id),
                (_, Kind::Event | Kind::Offline | Kind::Secret) => assert!(a.hint.is_some(), "{} needs a hint", a.id),
                _ => {}
            }
        }
        assert!(cat.accolades.iter().any(|a| a.kind == Kind::Secret));
        assert!(!cat.titles.hidden_secret_hint.is_empty() && !cat.titles.collected.is_empty());
    }

    #[test]
    fn an_unknown_key_in_the_catalog_or_the_pool_is_an_error_not_ignored() {
        let extra = TEST_CATALOG_JSON.replacen("\"hidden_secret_hint\"", "\"surprise\": \"x\", \"hidden_secret_hint\"", 1);
        assert!(Catalog::parse(&extra).is_err());
        let wrong_rule = TEST_CATALOG_JSON.replacen("\"first_hundred\"", "\"hundredth\"", 1);
        assert!(Catalog::parse(&wrong_rule).is_err());
        assert!(Catalog::parse(&TEST_CATALOG_JSON.replacen("\"v\": 1", "\"v\": 2", 1)).is_err());
        let loose = TEST_ROASTS_JSON.replacen("\"low_hours_max\"", "\"extra\": 1, \"low_hours_max\"", 1);
        assert!(Roasts::parse(&loose).is_err());
        assert!(Roasts::parse(&TEST_ROASTS_JSON.replacen("[\"Low a\", \"Low b\"]", "[]", 1)).is_err());
        assert!(Roasts::parse(&TEST_ROASTS_JSON.replace("\"tone\": \"roast\", \"text\"", "\"tone\": \"roast\", \"requires\": \"days\", \"text\"")
            .replace("\"tone\": \"cute\", \"text\"", "\"tone\": \"cute\", \"requires\": \"days\", \"text\"")).is_err());
    }

    #[test]
    fn the_overview_reads_like_the_desktop_card() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let out = shaped(&facts(), &clock(2026, 10, 17));
        let p = &out.profile;
        let hero = p.hours.as_ref().expect("the hours hero");
        assert_eq!(
            (hero.label.as_str(), hero.value.as_str(), hero.unit.as_str(), hero.roast.as_deref()),
            ("Hours watched", "1,300", "hrs", None)
        );
        assert_eq!(out.roast_hours, Some(1200.4 + 99.7));
        assert_eq!(p.twitch.as_ref().map(|s| (s.key.as_str(), s.title.as_str())), Some(("twitch", "Tyler's Twitch")));
        assert_eq!(
            tiles(&p.twitch),
            vec![
                ("twitch_age", "12 yrs, 6 mos", Some("Cake day in 154 days")),
                ("followers", "1,234,567", None),
                ("account_type", "Viewer", None),
            ]
        );
        assert_eq!(p.twitch.as_ref().unwrap().tiles[0].detail.as_deref(), Some("Joined March 20, 2014"));
        assert_eq!(p.lifetime.as_ref().map(|s| s.title.as_str()), Some("Lifetime in StreamNook"));
        assert_eq!(
            tiles(&p.lifetime),
            vec![
                ("messages", "10,100", Some("into the void")),
                ("streams", "300", None),
                ("subs_gifted", "5", None),
                ("cosmetics", "7", Some("3 paints, 2 badges, 2 StreamNook")),
                ("favorite_channel", "xQc", Some("313h watched")),
                ("member_rank", "#7", Some("Mythic")),
            ]
        );
        assert_eq!(p.views.as_deref(), Some("4,321"));
        assert_eq!(p.top_emotes.len(), 1, "an emote without art is left out");
        assert_eq!((p.top_emotes[0].name.as_str(), p.top_emotes[0].count, p.top_emotes[0].count_text.as_str()), ("OMEGALUL", 90, "90"));
        assert_eq!(p.emote_totals.as_deref(), Some("12 emotes · 345 used"));
        assert_eq!(p.relics.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["relic-gem"]);
        assert!(!out.hide_badges);
        let mut counted = facts();
        counted.counted_views = Some(4322);
        assert_eq!(shaped(&counted, &clock(2026, 10, 17)).profile.views.as_deref(), Some("4,322"), "the count this open made wins");
    }

    #[test]
    fn hidden_stats_never_reach_the_sheet() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let mut f = facts();
        f.hidden = keys(&["twitch", "views", "emotes", "accolades", "favorite_channel", "lifetime", "badge_collection"]);
        let out = shaped(&f, &clock(2026, 10, 17));
        let p = &out.profile;
        assert!(p.twitch.is_none(), "every tile of the section is hidden");
        assert!(p.views.is_none() && p.emote_totals.is_none() && p.top_emotes.is_empty() && p.accolades.is_none());
        // `lifetime` is narrowed by `favorite_channel`, so only that one goes.
        let kept: Vec<&str> = p.lifetime.as_ref().unwrap().tiles.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(kept, vec!["messages", "streams", "subs_gifted", "cosmetics", "member_rank"]);
        assert!(p.hours.is_some());
        assert!(out.hide_badges);
        let mut relics_hidden = facts();
        relics_hidden.hidden = keys(&["relics"]);
        assert!(shaped(&relics_hidden, &clock(2026, 10, 17)).profile.relics.is_empty());
    }

    #[test]
    fn the_hours_hero_and_its_roast_hide_with_their_own_keys() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let with = |hidden: &[&str]| {
            let mut f = facts();
            f.hidden = keys(hidden);
            shaped(&f, &clock(2026, 10, 17))
        };
        let both = with(&[]);
        assert!(both.profile.hours.is_some() && both.roast_hours.is_some());
        let no_roast = with(&["roast_line"]);
        assert!(no_roast.profile.hours.is_some() && no_roast.roast_hours.is_none());
        let no_hours = with(&["hours"]);
        assert!(no_hours.profile.hours.is_none() && no_hours.roast_hours.is_none());
        let whole_section = with(&["roast"]);
        assert!(whole_section.profile.hours.is_none() && whole_section.roast_hours.is_none());
    }

    #[test]
    fn a_read_that_failed_leaves_its_tiles_out_and_never_shows_zeros() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let mut f = facts();
        f.stats = Part::Failed;
        f.saved = Part::Failed;
        f.ivr = Part::Failed;
        f.favorite = Part::Failed;
        f.totals = Part::Failed;
        let p = shaped(&f, &clock(2026, 10, 17)).profile;
        assert!(p.hours.is_none() && p.views.is_none() && p.twitch.is_none() && p.accolades.is_none());
        assert!(p.emote_totals.is_none());
        let kept: Vec<&str> = p.lifetime.as_ref().unwrap().tiles.iter().map(|t| t.key.as_str()).collect();
        assert_eq!(kept, vec!["cosmetics", "member_rank"]);
        assert_eq!(p.top_emotes.len(), 1, "the emotes read answered");
        // A count this open made still shows without the stats row.
        f.counted_views = Some(5);
        assert_eq!(shaped(&f, &clock(2026, 10, 17)).profile.views.as_deref(), Some("5"));
        // Stats and saved answered, the rest not: the wall is built.
        let mut partial = facts();
        partial.ivr = Part::Failed;
        let wall = shaped(&partial, &clock(2026, 10, 17)).profile.accolades.expect("the wall is built");
        // IVR is down and nothing is cached: Veteran is left off, not shown locked at 0 years.
        assert!(wall.items.iter().all(|m| m.id != "veteran"));
        assert!(wall.items.iter().any(|m| m.id == "collector"), "the cosmetics input answered");
        // Without the saved set there is no wall to build.
        partial.saved = Part::Failed;
        assert!(shaped(&partial, &clock(2026, 10, 17)).profile.accolades.is_none());
        // The 7TV inventory is down instead: Collector is left off, Veteran stays.
        let mut no_inventory = facts();
        no_inventory.seventv = Part::Failed;
        let wall = shaped(&no_inventory, &clock(2026, 10, 17)).profile.accolades.expect("the wall is built");
        assert!(wall.items.iter().all(|m| m.id != "collector"));
        assert!(wall.items.iter().any(|m| m.id == "veteran" && m.earned));
        assert_eq!(wall.total as usize, wall.items.len(), "the counts follow the items");
        assert_eq!(wall.earned as usize, wall.items.iter().filter(|m| m.earned).count());
    }

    #[test]
    fn a_member_with_no_stats_row_yet_shows_zeros_as_the_desktop_card_does() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let mut f = facts();
        f.stats = Part::Answered(None);
        f.favorite = Part::Answered(None);
        let p = shaped(&f, &clock(2026, 10, 17)).profile;
        assert_eq!(p.hours.as_ref().map(|h| h.value.as_str()), Some("0"));
        let lifetime = tiles(&p.lifetime);
        assert_eq!(&lifetime[..2], &[("messages", "0", Some("into the void")), ("streams", "0", None)]);
        assert!(lifetime.iter().all(|t| t.0 != "favorite_channel" && t.0 != "subs_gifted"));
        assert!(p.views.is_none());
    }

    #[test]
    fn a_failed_part_takes_the_cached_copy_and_an_answered_one_never_does() {
        let mut fresh = facts();
        fresh.stats = Part::Failed;
        fresh.totals = Part::Answered((1, 2));
        fresh.hidden = keys(&["views"]);
        let mut cached = facts();
        cached.stats = Part::Answered(Some(json!({ "hours_watched": 5 })));
        cached.totals = Part::Answered((99, 99));
        cached.hidden = keys(&["emotes"]);
        cached.ivr = Part::Failed;
        let merged = merge_facts(fresh, &cached);
        assert_eq!(merged.stats, cached.stats, "the failed part took the cached answer");
        assert_eq!(merged.totals, Part::Answered((1, 2)), "an answered part is kept");
        assert_eq!(merged.hidden, keys(&["views"]), "what they hide comes from the fresh read");
        assert_eq!(merged.ivr, facts().ivr, "an answered part is kept");
        let mut both_failed = facts();
        both_failed.favorite = Part::Failed;
        let mut cached_failed = facts();
        cached_failed.favorite = Part::Failed;
        assert!(!merge_facts(both_failed, &cached_failed).favorite.is_answered());
    }

    #[test]
    fn account_type_follows_the_roles_then_the_broadcaster_type() {
        let roles = |staff, partner, affiliate| IvrUserSummary {
            followers: None,
            created_at: None,
            is_staff: staff,
            is_partner: partner,
            is_affiliate: affiliate,
        };
        assert_eq!(account_type(Some(&roles(true, true, true)), "partner"), "Twitch Staff");
        assert_eq!(account_type(Some(&roles(false, true, false)), ""), "Partner");
        assert_eq!(account_type(Some(&roles(false, false, true)), ""), "Affiliate");
        assert_eq!(account_type(Some(&roles(false, false, false)), ""), "Viewer");
        // IVR may be empty while Helix says what the account is.
        assert_eq!(account_type(Some(&roles(false, false, false)), "partner"), "Partner");
        assert_eq!(account_type(None, "affiliate"), "Affiliate");
        assert_eq!(account_type(None, ""), "Viewer");
    }

    #[test]
    fn the_account_type_tile_needs_an_answer_to_say_viewer() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let cat = catalog();
        let mut f = facts();
        f.ivr = Part::Failed;
        let ask = |bt: &str, f: &MemberFacts| -> Vec<(String, String)> {
            let at = clock(2026, 10, 17);
            let ctx = ShapeCtx { display_name: "Tyler", login: "tyler", broadcaster_type: bt, clock: &at, catalog: &cat };
            shape(f, &ctx)
                .profile
                .twitch
                .map(|s| s.tiles.into_iter().map(|t| (t.key, t.value)).collect())
                .unwrap_or_default()
        };
        assert!(ask("", &f).is_empty(), "no answer, no Viewer");
        assert_eq!(ask("partner", &f), vec![("account_type".to_string(), "Partner".to_string())]);
        assert_eq!(twitch_title("{name}'s Twitch", "Tyler"), "Tyler's Twitch");
        assert_eq!(twitch_title("{name}'s Twitch", "Chris"), "Chris' Twitch");
        assert_eq!(twitch_title("{name}'s Twitch", "XQS"), "XQS' Twitch");
    }

    #[test]
    fn the_joined_date_is_read_in_the_viewers_zone() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let mut f = facts();
        f.ivr = Part::Answered(Some(ivr("2014-03-20T01:00:00Z")));
        let joined = |zone: Zone| {
            let c = Clock { now: Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap(), zone };
            shaped(&f, &c).profile.twitch.unwrap().tiles[0].detail.clone().unwrap()
        };
        assert_eq!(joined(utc()), "Joined March 20, 2014");
        assert_eq!(joined(Zone::Fixed(FixedOffset::east_opt(-8 * 3600).unwrap())), "Joined March 19, 2014");
        // And "today" moves with the zone too.
        let late = Utc.with_ymd_and_hms(2026, 10, 18, 3, 0, 0).unwrap();
        assert_eq!(Clock { now: late, zone: Zone::Fixed(FixedOffset::east_opt(-8 * 3600).unwrap()) }.today().to_string(), "2026-10-17");
        assert_eq!(Clock { now: late, zone: utc() }.today().to_string(), "2026-10-18");
    }

    #[test]
    fn the_reported_offset_stands_only_when_the_platform_zone_differs_from_it() {
        let here = Local::now().offset().local_minus_utc() / 60;
        assert!(matches!(Zone::viewer(None), Zone::Local));
        assert!(matches!(Zone::viewer(Some(here)), Zone::Local));
        assert!(matches!(Zone::viewer(Some(here + 60)), Zone::Fixed(_)));
        let at = Utc.with_ymd_and_hms(2026, 11, 5, 3, 0, 0).unwrap();
        assert_eq!(Zone::Fixed(FixedOffset::east_opt(-8 * 3600).unwrap()).date(at).to_string(), "2026-11-04");
        assert_eq!(Zone::Fixed(FixedOffset::east_opt(9 * 3600).unwrap()).date(at).to_string(), "2026-11-05");
    }

    #[test]
    fn account_age_and_cake_day() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(account_age(d(2024, 10, 17), d(2026, 10, 17)), ("2 yrs".into(), 2, 0));
        assert_eq!(account_age(d(2026, 2, 20), d(2026, 10, 17)), ("7 mos".into(), 0, 126));
        assert_eq!(account_age(d(2025, 10, 18), d(2026, 10, 17)), ("11 mos".into(), 0, 1));
        // A leap-day account has its cake day on Mar 1 in the other years.
        assert_eq!(account_age(d(2020, 2, 29), d(2026, 2, 28)).2, 1);
        assert_eq!(cake_caption(0), "Cake day is today. Make a wish.");
    }

    #[test]
    fn numbers_are_grouped() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1000), "1,000");
        assert_eq!(grouped(1_234_567), "1,234,567");
        // A tile keeps its number to four characters past 9,999,999.
        assert_eq!(tile_number(9_999_999), "9,999,999");
        assert_eq!(tile_number(12_345_678), "12.3M");
        assert_eq!(tile_number(99_960_000), "99.9M");
        assert_eq!(tile_number(123_456_789), "123M");
        assert_eq!(tile_number(1_234_567_890), "1.23B");
        assert_eq!(tile_number(12_345_678_901), "12.3B");
        assert_eq!(tile_number(1_150_000_000), "1.15B");
        assert_eq!(tile_number(10_000_000), "10.0M");
    }

    #[test]
    fn emote_totals_count_rows_and_sum_their_counts() {
        assert_eq!(totals_of(&[]), (0, 0));
        let rows = vec![json!({ "count": 5 }), json!({ "count": "7" }), json!({ "count": 0 })];
        assert_eq!(totals_of(&rows), (3, 12));
    }

    #[test]
    fn the_wall_follows_the_rules_public_walls_have() {
        let cat = catalog();
        let wall = |input: WallInput| wall_with(&cat, &input);
        let ids = |w: &AccoladeWall| w.items.iter().map(|m| m.id.as_str().to_string()).collect::<Vec<_>>();
        let base = WallInput { member_number: Some(1000), today: "2026-11-05".into(), ..Default::default() };

        // Nobody's secrets show until earned, and out-of-reach ones never show.
        let plain = wall(base.clone());
        assert_eq!(ids(&plain), vec!["chatterbox", "marathoner", "dropgoblin", "collector", "veteran", "halloween"]);
        assert_eq!((plain.earned, plain.total), (0, 6));

        // Meme numbers: one accolade, labelled with the number's own name.
        for (n, label) in [(69, "Nice"), (420, "Blaze It"), (167, "Six Seven-ish")] {
            let w = wall(WallInput { member_number: Some(n), ..base.clone() });
            let nice = w.items.iter().find(|m| m.id == "nice").unwrap_or_else(|| panic!("{n} earns Nice"));
            assert_eq!((nice.label.as_str(), nice.earned, nice.secret), (label, true, true));
            assert_eq!(nice.hint, "A hidden accolade.", "a public wall never says how a secret is earned");
        }
        assert!(wall(WallInput { member_number: Some(68), ..base.clone() }).items.iter().all(|m| m.id != "nice"));

        // Completionist counts the base accolades still reachable: OG is out of reach past 100.
        let all = wall(WallInput {
            messages: 2000,
            hours: 150.0,
            cosmetics: 5,
            years: 5,
            saved: vec!["dropgoblin".into()],
            ..base.clone()
        });
        assert!(all.items.iter().any(|m| m.id == "completionist" && m.earned), "Completionist beyond member 100");
        assert!(all.items.iter().all(|m| m.id != "og"), "OG is out of reach and not held");
        let early = wall(WallInput {
            member_number: Some(50),
            messages: 2000,
            hours: 150.0,
            cosmetics: 5,
            years: 5,
            saved: vec!["dropgoblin".into()],
            ..base.clone()
        });
        assert!(early.items.iter().any(|m| m.id == "og" && m.earned));
        assert!(early.items.iter().any(|m| m.id == "completionist" && m.earned));
        // A held accolade that is out of reach still shows.
        let held = wall(WallInput { saved: vec!["og".into()], ..base.clone() });
        assert!(held.items.iter().any(|m| m.id == "og" && m.earned));

        // An event that is over: absent unless held, then its earned words.
        assert!(plain.items.iter().all(|m| m.id != "cs2_major_cologne_2026"));
        let cs2 = wall(WallInput { saved: vec!["cs2_major_cologne_2026".into()], ..base.clone() });
        let medal = cs2.items.iter().find(|m| m.id == "cs2_major_cologne_2026").unwrap();
        assert_eq!(medal.hint, "Tuned in during the CS2 Major");

        // Seasons: locked shows the window hint, earned says Collected.
        let locked = plain.items.iter().find(|m| m.id == "halloween").unwrap();
        assert_eq!(locked.hint, "Open StreamNook around Halloween (Oct 31)");
        let held = wall(WallInput { saved: vec!["halloween".into()], ..base.clone() });
        assert_eq!(held.items.iter().find(|m| m.id == "halloween").unwrap().hint, "Collected");

        // Earned first, then locked, each in catalog order.
        let mixed = wall(WallInput { messages: 5000, saved: vec!["halloween".into()], ..base.clone() });
        assert_eq!(ids(&mixed), vec!["chatterbox", "halloween", "marathoner", "dropgoblin", "collector", "veteran"]);
        assert_eq!((mixed.earned, mixed.total), (2, 6));
    }

    #[test]
    fn the_public_hints_never_say_a_hidden_number_or_one_the_public_view_never_reads() {
        let cat = catalog();
        let hint_of = |input: &WallInput, id: &str| {
            wall_with(&cat, input).items.into_iter().find(|m| m.id == id).map(|m| m.hint).unwrap_or_default()
        };
        let input = WallInput { messages: 500, hours: 99.6, drops: 30, member_number: Some(1000), ..Default::default() };
        assert_eq!(hint_of(&input, "chatterbox"), "Messages sent: 500 / 1,000");
        // Hours round for the words and not for the rule, as on the desktop card.
        assert_eq!(hint_of(&input, "marathoner"), "Hours watched: 100 / 100");
        assert!(!wall_with(&cat, &input).items.iter().any(|m| m.id == "marathoner" && m.earned));
        // A hidden stat says its goal only.
        let hidden = WallInput { hidden: keys(&["messages"]), ..input.clone() };
        assert_eq!(hint_of(&hidden, "chatterbox"), "Send 1,000 messages");
        let hidden_section = WallInput { hidden: keys(&["roast"]), ..input.clone() };
        assert_eq!(hint_of(&hidden_section, "marathoner"), "Watch 100 hours");
        // Drops are never read publicly: the goal, whatever the input holds.
        assert_eq!(hint_of(&input, "dropgoblin"), "Claim 25 drops");
        // A special base says its goal.
        assert_eq!(hint_of(&WallInput { member_number: Some(5), ..input.clone() }, "og"), "Be one of the first 100 StreamNook members");
    }

    #[test]
    fn an_accolade_whose_input_never_answered_is_lit_if_saved_and_otherwise_left_off() {
        let cat = catalog();
        let unknown = [StatKind::Years, StatKind::Cosmetics];
        let base = WallInput { member_number: Some(1000), today: "2026-11-05".into(), ..Default::default() };
        let ids = |w: &AccoladeWall| w.items.iter().map(|m| m.id.clone()).collect::<Vec<_>>();
        // Left off: not shown locked at 0 years or 0 cosmetics. The known inputs still show.
        let off = wall_with_unknowns(&cat, &base, &unknown);
        assert_eq!(ids(&off), vec!["chatterbox", "marathoner", "dropgoblin", "halloween"]);
        assert_eq!((off.earned, off.total), (0, 4), "the counts follow the items");
        // Saved: lit, and it says its goal rather than a progress number it cannot know.
        let saved = wall_with_unknowns(&cat, &WallInput { saved: vec!["veteran".into()], ..base.clone() }, &unknown);
        let veteran = saved.items.iter().find(|m| m.id == "veteran").expect("a saved accolade shows");
        assert!(veteran.earned);
        assert_eq!(veteran.hint, "Reach 5 years on Twitch");
        assert_eq!((saved.earned, saved.total), (1, 5));
        // Completionist cannot light from an unknown base accolade.
        let everything = WallInput { messages: 2000, hours: 150.0, saved: vec!["dropgoblin".into()], ..base.clone() };
        assert!(wall_with_unknowns(&cat, &everything, &unknown).items.iter().all(|m| m.id != "completionist"));
        // The same input with every source answered is the ordinary wall.
        let known = WallInput { cosmetics: 5, years: 5, ..everything };
        assert!(wall_with_unknowns(&cat, &known, &[]).items.iter().any(|m| m.id == "completionist" && m.earned));
        assert_eq!(ids(&wall_with(&cat, &base)), ids(&wall_with_unknowns(&cat, &base, &[])));
    }

    #[test]
    fn the_shared_wall_cases_pass_on_the_rust_side() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Case {
            name: String,
            own: bool,
            input: WallInput,
            expect: Value,
            #[serde(default)]
            site: Option<Value>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Cases {
            v: u32,
            cases: Vec<Case>,
        }
        let file: Cases = serde_json::from_str(include_str!("../../../src/utils/accolades.cases.json")).expect("accolades.cases.json");
        assert_eq!(file.v, 1);
        assert!(file.cases.iter().any(|c| !c.own), "the cases file holds public cases");
        assert!(file.cases.iter().any(|c| c.own), "the cases file holds own cases");
        for case in &file.cases {
            let mode = if case.own { WallMode::Own } else { WallMode::Public };
            let wall = serde_json::to_value(accolade_wall(&case.input, mode)).unwrap();
            assert_eq!(wall, case.expect, "case: {}", case.name);
        }
    }

    #[test]
    fn the_roast_follows_its_rules() {
        let pool = Roasts::parse(TEST_ROASTS_JSON).unwrap();
        let pick = |hours: f64, rand: &[f64], last: Option<&str>| pool.pick(hours, &mut seq(rand), last);
        // Almost no watch time: the low-hours lines, one random number for the pick.
        assert_eq!(pick(1.6, &[0.0], None).text, "Low a");
        assert_eq!(pick(1.6, &[0.99], None).text, "Low b");
        assert_eq!(pick(1.6, &[0.0], Some("Low a")).text, "Low b");
        assert_eq!(pick(0.0, &[0.0], None).tone, Tone::Roast);
        assert_eq!(pick(f64::NAN, &[0.0], None).text, "Low a");
        // Tone from the first number: below 0.6 roast, below 0.9 neutral, else cute.
        assert_eq!(pick(60.0, &[0.59, 0.0], None), Roast { text: "R 3".into(), tone: Tone::Roast });
        assert_eq!(pick(60.0, &[0.6, 0.0], None).text, "N 2");
        assert_eq!(pick(60.0, &[0.9, 0.0], None).text, "C flat");
        assert_eq!(pick(60.0, &[0.999, 0.5], None).text, "C flat");
        // The line shown last is avoided while another will do, then given up.
        assert_eq!(pick(60.0, &[0.1, 0.0], Some("R 3")).text, "R flat");
        assert_eq!(pick(30.0, &[0.1, 0.0], Some("R flat")).text, "C flat");
        // A templated line is eligible only when its unit rounds to two or more: 30 hours is one day and one work week.
        assert_eq!(pick(30.0, &[0.1, 0.0], None).text, "R flat");
        assert_eq!(pick(30.0, &[0.7, 0.0], None).text, "R flat", "no neutral line fits, so any eligible one");
        // Units are grouped with a fixed locale.
        assert_eq!(pick(1_000_000.0, &[0.1, 0.0], None).text, "R 41,667");
        // Eligibility of a kept line follows the hours.
        assert!(pool.candidates(60.0).iter().any(|c| c.text == "R 3"));
        assert!(!pool.candidates(30.0).iter().any(|c| c.text.starts_with("R ") && c.text != "R flat"));
    }

    #[test]
    fn the_shared_roast_cases_pass_on_the_rust_side() {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Expect {
            text: String,
            tone: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Case {
            name: String,
            hours: f64,
            rand: Vec<f64>,
            last: Option<String>,
            expect: Expect,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Cases {
            v: u32,
            cases: Vec<Case>,
        }
        let file: Cases = serde_json::from_str(include_str!("../../../src/utils/hoursRoasts.cases.json")).expect("hoursRoasts.cases.json");
        assert_eq!(file.v, 1);
        assert!(!file.cases.is_empty());
        for case in &file.cases {
            let got = ROASTS.pick(case.hours, &mut seq(&case.rand), case.last.as_deref());
            assert_eq!((got.text.as_str(), got.tone.as_str()), (case.expect.text.as_str(), case.expect.tone.as_str()), "case: {}", case.name);
        }
    }

    #[test]
    fn streamnook_badges_lead_with_the_worn_one_and_skip_other_kinds() {
        let entry = |slug: &str, kind: &str, order: i64, url: &str| CatalogEntry {
            slug: slug.into(),
            name: slug.to_uppercase(),
            kind: kind.into(),
            sort_order: order,
            asset_path: url.into(),
            ..Default::default()
        };
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
        let entry = |slug: &str, kind: &str, order: i64, url: &str| CatalogEntry {
            slug: slug.into(),
            name: slug.to_uppercase(),
            kind: kind.into(),
            sort_order: order,
            asset_path: url.into(),
            ..Default::default()
        };
        let catalog = vec![entry("r1", "relic", 0, "https://x/1"), entry("r2", "relic", 1, "https://x/2"), entry("b", "badge", 0, "https://x/b")];
        let owned: BTreeSet<String> = ["r1", "r2", "b"].iter().map(|s| s.to_string()).collect();
        let ids = |f: Option<&str>| relics(&owned, f, &catalog).into_iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(Some("r2")), vec!["r2", "r1"]);
        assert_eq!(ids(None), vec!["r1", "r2"]);
        assert_eq!(ids(Some("gone")), vec!["r1", "r2"]);
    }

    #[test]
    fn the_equipped_relic_leads_the_shaped_sheet_and_an_unread_slot_changes_nothing() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_for_test(
            &[("100", 7)],
            vec![
                entry_for_test("relic-a", "relic", "https://x/a", false),
                entry_for_test("relic-b", "relic", "https://x/b", false),
            ],
            &[("100", "relic-a"), ("100", "relic-b")],
            &[],
            vec![],
        );
        let mut f = facts();
        f.equipment = Part::Answered(vec![json!({ "slot": "relic_primary", "cosmetic_slug": "relic-b" })]);
        let p = shaped(&f, &clock(2026, 10, 17)).profile;
        assert_eq!(p.relics.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["relic-b", "relic-a"]);
        assert_eq!(equipped_slot(&f, "relic_primary").as_deref(), Some("relic-b"));
        assert_eq!(equipped_slot(&f, "frame"), None);
        f.equipment = Part::Failed;
        let unknown = shaped(&f, &clock(2026, 10, 17)).profile;
        assert_eq!(unknown.relics.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), vec!["relic-a", "relic-b"]);
    }

    #[test]
    fn the_shape_keeps_the_keys_both_phones_decode() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let v = serde_json::to_value(shaped(&facts(), &clock(2026, 10, 17)).profile).unwrap();
        for key in ["relics", "hours", "twitch", "lifetime", "views", "top_emotes", "emote_totals", "accolades"] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert!(v.get("stats").is_none(), "the flat stats list is gone");
        assert_eq!(v["hours"]["label"], "Hours watched");
        assert_eq!(v["hours"]["unit"], "hrs");
        assert!(v["hours"].get("roast").is_none(), "a missing key, never null");
        assert!(v.get("featured_relic").is_none());
        // Every optional key is missing, never null, anywhere in the answer.
        fn no_nulls(v: &Value, at: &str) {
            match v {
                Value::Null => panic!("{at} is null"),
                Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| no_nulls(x, &format!("{at}[{i}]"))),
                Value::Object(o) => o.iter().for_each(|(k, x)| no_nulls(x, &format!("{at}.{k}"))),
                _ => {}
            }
        }
        no_nulls(&v, "member_profile");
        assert_eq!(v["twitch"]["key"], "twitch");
        assert_eq!(v["twitch"]["tiles"][0]["detail"], "Joined March 20, 2014");
        assert!(v["twitch"]["tiles"][1].get("detail").is_none() && v["twitch"]["tiles"][2].get("caption").is_none());
        assert_eq!(v["top_emotes"][0]["count_text"], "90");
        assert!(v["accolades"]["items"][0]["colors"].is_array());
        assert!(v["accolades"]["items"][0].get("secret").is_some());
        let empty = serde_json::to_value(MemberProfile::default()).unwrap();
        let mut present: Vec<&str> = empty.as_object().unwrap().keys().map(String::as_str).collect();
        present.sort_unstable();
        assert_eq!(present, vec!["relics", "top_emotes"]);
    }

    #[test]
    fn up_to_sixteen_emotes_with_art_are_sent() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let rows: Vec<Value> = (0..30)
            .map(|i| {
                // Every fifth emote has no art.
                let art = if i % 5 == 4 { Value::Null } else { json!(format!("https://x/{i}")) };
                json!({ "emote_id": format!("e{i}"), "emote_name": format!("E{i}"), "provider": "7tv", "image_url": art, "count": 100 - i })
            })
            .collect();
        let mut f = facts();
        f.emotes = Part::Answered(rows);
        let p = shaped(&f, &clock(2026, 10, 17)).profile;
        assert_eq!(p.top_emotes.len(), 16);
        assert!(p.top_emotes.iter().all(|e| !e.url.is_empty()));
        assert_eq!(p.top_emotes[0].id, "e0");
        assert!(p.top_emotes.iter().all(|e| e.id != "e4"), "the one without art is skipped");
    }

    fn own_input() -> WallInput {
        WallInput {
            member_number: Some(69),
            messages: 500,
            hours: 99.6,
            drops: 12,
            points: 7,
            today: "2026-11-05".into(),
            created_at: Some("2014-03-20T00:00:00Z".into()),
            hidden: keys(&["messages", "roast"]),
            ..Default::default()
        }
    }

    #[test]
    fn an_own_wall_lists_everything_with_the_owners_numbers_and_masks_locked_secrets() {
        let cat = catalog();
        let own = wall_core(&cat, &own_input(), &[], WallMode::Own);
        let find = |id: &str| own.items.iter().find(|m| m.id == id).unwrap_or_else(|| panic!("{id} is on an own wall")).clone();
        // Hidden switches are the public's business: the owner sees every number.
        assert_eq!(find("chatterbox").hint, "Messages sent: 500 / 1,000");
        assert_eq!(find("marathoner").hint, "Hours watched: 100 / 100");
        assert_eq!(find("dropgoblin").hint, "Drops claimed: 12 / 25");
        assert!(!find("dropgoblin").earned);
        // A locked secret is a mystery with its cryptic hint; an earned one shows itself.
        let triple = find("triple");
        assert_eq!((triple.label.as_str(), triple.icon.as_str(), triple.secret, triple.earned), ("???", "lock", true, false));
        assert_eq!(triple.hint, "Talk, watch, and collect until all three run deep.");
        let nice = find("nice");
        assert_eq!((nice.label.as_str(), nice.icon.as_str(), nice.earned), ("Nice", "smile", true));
        assert_eq!(nice.hint, "Land on a number the internet never lets you forget.");
        // The public wall of the same input has none of that.
        let public = wall_core(&cat, &own_input(), &[], WallMode::Public);
        assert!(public.items.iter().all(|m| m.id != "triple" && m.id != "completionist"));
        assert_eq!(public.items.iter().find(|m| m.id == "nice").unwrap().hint, "A hidden accolade.");
        assert!(public.items.iter().all(|m| m.label != "???"));
        assert_eq!(public, wall_with(&cat, &own_input()), "the public entry points are unchanged");
    }

    #[test]
    fn an_own_wall_lights_the_seasons_from_todays_date_and_a_public_one_never_does() {
        let cat = catalog();
        let on = |today: &str| WallInput { today: today.into(), member_number: Some(1000), ..Default::default() };
        let halloween = |w: &AccoladeWall| w.items.iter().find(|m| m.id == "halloween").map(|m| (m.earned, m.hint.clone()));
        assert_eq!(halloween(&wall_core(&cat, &on("2026-10-31"), &[], WallMode::Own)), Some((true, "Collected".to_string())));
        assert_eq!(
            halloween(&wall_core(&cat, &on("2026-11-05"), &[], WallMode::Own)),
            Some((false, "Open StreamNook around Halloween (Oct 31)".to_string()))
        );
        assert_eq!(halloween(&wall_core(&cat, &on("2026-10-31"), &[], WallMode::Public)).map(|h| h.0), Some(false));
        // The window wraps the new year, and an unreadable date lights nothing.
        let wraps = Window { from_m: 12, from_d: 31, to_m: 1, to_d: 1 };
        assert!(in_window(101, &wraps) && in_window(1231, &wraps) && !in_window(1230, &wraps) && !in_window(102, &wraps));
        assert!(optimistic_ids(&cat, &on("not a date")).is_empty());
    }

    #[test]
    fn an_own_wall_keeps_an_accolade_whose_input_never_answered_locked_with_its_goal_never_at_zero() {
        let cat = catalog();
        let base = WallInput {
            member_number: Some(1000),
            messages: 2000,
            hours: 150.0,
            today: "2026-11-05".into(),
            ..Default::default()
        };
        let own = |input: &WallInput, unknown: &[StatKind]| wall_core(&cat, input, unknown, WallMode::Own);
        let find = |w: &AccoladeWall, id: &str| w.items.iter().find(|m| m.id == id).cloned();
        // Drops unknown (no drops token for this account): Drop Goblin is listed locked with its goal.
        let w = own(&base, &[StatKind::Drops]);
        let goblin = find(&w, "dropgoblin").expect("listed, not omitted");
        assert_eq!((goblin.earned, goblin.hint.as_str()), (false, "Claim 25 drops"));
        // Triple Threat reads Drops from that state: messages and hours are past their lines and it is
        // still locked (a mystery with its cryptic hint) until the drops read answers.
        let triple = find(&w, "triple").expect("a locked secret is listed on an own wall");
        assert_eq!((triple.earned, triple.label.as_str(), triple.icon.as_str()), (false, "???", "lock"));
        assert_eq!(triple.hint, "Talk, watch, and collect until all three run deep.");
        // Saved: lit, and it says its goal rather than a number it cannot know.
        let saved = WallInput { saved: vec!["dropgoblin".into(), "triple".into()], ..base.clone() };
        let w = own(&saved, &[StatKind::Drops]);
        assert_eq!(find(&w, "dropgoblin").map(|m| (m.earned, m.hint)), Some((true, "Claim 25 drops".to_string())));
        assert_eq!(find(&w, "triple").map(|m| (m.earned, m.label)), Some((true, "Triple Threat".to_string())));
        // The drops read answered with 30: Triple Threat is earned from the numbers alone.
        let known = WallInput { drops: 30, ..base.clone() };
        assert_eq!(find(&own(&known, &[]), "triple").map(|m| m.earned), Some(true));
        assert_eq!(find(&own(&known, &[]), "dropgoblin").map(|m| m.hint), Some("Drops claimed: 30 / 25".to_string()));
        // Years and cosmetics work the same way on an own wall: listed locked with the goal.
        let w = own(&base, &[StatKind::Years, StatKind::Cosmetics]);
        assert_eq!(find(&w, "veteran").map(|m| (m.earned, m.hint)), Some((false, "Reach 5 years on Twitch".to_string())));
        assert_eq!(find(&w, "collector").map(|m| m.hint), Some("Collect 5 cosmetics".to_string()));
        // Completionist cannot light while a base accolade's input is unknown.
        let everything = WallInput { saved: vec!["dropgoblin".into()], ..base.clone() };
        assert_eq!(find(&own(&everything, &[StatKind::Years]), "completionist").map(|m| m.earned), Some(false));
    }

    #[test]
    fn the_wall_input_reads_the_same_numbers_the_sheet_does() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        registry_fixture();
        let (input, unknown) = wall_input(&facts(), &clock(2026, 10, 17), Some(0), Some(0));
        assert_eq!((input.messages, input.streams, input.cosmetics, input.years), (10_100, 300, 7, 12));
        assert_eq!(input.hours, 1200.4 + 99.7);
        assert_eq!((input.member_number, input.today.as_str()), (Some(7), "2026-10-17"));
        assert_eq!(input.created_at.as_deref(), Some("2014-03-20T00:00:00Z"));
        assert!(unknown.is_empty());
        // None means the read never answered, and says so.
        let (input, unknown) = wall_input(&facts(), &clock(2026, 10, 17), None, Some(5_000));
        assert_eq!((input.drops, input.points), (0, 5_000));
        assert_eq!(unknown, vec![StatKind::Drops]);
        let mut down = facts();
        down.ivr = Part::Failed;
        down.seventv = Part::Failed;
        assert_eq!(wall_input(&down, &clock(2026, 10, 17), Some(0), Some(0)).1, vec![StatKind::Years, StatKind::Cosmetics]);
        // The creation date is read in the viewer's zone.
        let mut late = facts();
        late.ivr = Part::Answered(Some(ivr("2014-03-20T01:00:00Z")));
        let west = Clock { now: Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap(), zone: Zone::Fixed(FixedOffset::east_opt(-8 * 3600).unwrap()) };
        assert_eq!(wall_input(&late, &west, Some(0), Some(0)).0.created_at.as_deref(), Some("2014-03-19T00:00:00Z"));
    }
}
