//! Badge earn windows: when a Twitch badge can be earned.
//!
//! The single classifier for every surface that asks "is this badge earnable
//! now". Windows arrive as campaign ISO timestamps on the relay enrichment, as
//! ISO stamps written into the More Info copy, or as hand-written prose
//! ("Dec 19 – Jan 01", "December 4, 2025 at 9:00 AM"). This resolves them into
//! runs of epoch milliseconds; comparing a run against the clock is left to the
//! caller, so a badge whose window opens while its payload sits in a cache still
//! reads correctly.
//!
//! Prose dates are local wall-clock times (the copy never names a zone), ISO
//! stamps without a zone suffix are local too, and ISO stamps with one are
//! absolute. The relay's `sub_events` are not read: that field is only
//! campaign-sourced when a campaign matched, and a model-written phase list must
//! not decide whether a badge reads as live.

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WindowStatus {
    Available,
    ComingSoon,
    Expired,
}

/// One span a badge is earnable in. `None` is an open end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowRun {
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

/// Where a window came from, most authoritative first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSource {
    /// `starts_utc` / `ends_utc` on the relay enrichment, taken from the Drops
    /// campaign itself.
    Campaign,
    /// ISO stamps written into the More Info copy.
    CopyIso,
    /// A range read out of prose. Year-less prose assumes the current year.
    Prose,
    /// No dates, but the copy says the badge is available on an ongoing
    /// condition ("available while TwitchCon San Diego 2026 tickets are on
    /// sale"). Open from the start, closed at the end of the latest year the
    /// copy names, so a cached description that is never re-scraped cannot
    /// keep a finished event available forever.
    Ongoing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedWindow {
    pub runs: Vec<WindowRun>,
    pub source: WindowSource,
}

const FULL_MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const ABBR_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Exact-case full month name to a 0-based index ("December" only, not
/// "december"). The "Event duration" forms are machine-written and always
/// capitalised, so an exact match keeps a stray word from reading as a month.
fn full_month_exact(name: &str) -> Option<u32> {
    FULL_MONTHS.iter().position(|m| *m == name).map(|i| i as u32)
}

/// Exact-case three-letter abbreviation to a 0-based index ("Dec").
fn abbr_month_exact(name: &str) -> Option<u32> {
    ABBR_MONTHS.iter().position(|m| *m == name).map(|i| i as u32)
}

// Regex building blocks. ASCII classes throughout: badge copy is English, and a
// Unicode `\d` or `\w` would accept digits and letters the dates never use.
const DASH: &str = "[-–—]";
const ISO_STAMP: &str = r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}(?::[0-9]{2})?Z?";
const MONTH_WORD: &str = "([A-Za-z]{3,9})";
const DAY: &str = "([0-9]{1,2})(?:st|nd|rd|th)?";
const W: &str = "[A-Za-z0-9_]";

fn join() -> String {
    format!(r"(?:\s*{DASH}\s*|\s+(?:to|through|until)\s+|\s+and\s+)")
}

fn re(pattern: String) -> Regex {
    Regex::new(&pattern).expect("badge window regex")
}

static FULL_NAMED_RANGE: Lazy<Regex> = Lazy::new(|| {
    re(format!(
        r"(?i)Event duration:\s*({W}+)\s+([0-9]{{1,2}}),?\s+([0-9]{{4}})\s*{DASH}\s*({W}+)\s+([0-9]{{1,2}}),?\s+([0-9]{{4}})"
    ))
});
static ABBREV_RANGE: Lazy<Regex> = Lazy::new(|| {
    re(format!(
        r"(?i)Event duration:\s*({W}{{3}})\s+([0-9]{{1,2}})\s*{DASH}\s*({W}{{3}})\s+([0-9]{{1,2}})"
    ))
});
static ABBREV_SAME_MONTH: Lazy<Regex> = Lazy::new(|| {
    re(format!(
        r"(?i)Event duration:\s*({W}{{3}})\s+([0-9]{{1,2}})\s*{DASH}\s*([0-9]{{1,2}})"
    ))
});
static ISO_START: Lazy<Regex> =
    Lazy::new(|| re(format!(r"(?i)Event start:\s*({ISO_STAMP})")));
static ISO_END: Lazy<Regex> = Lazy::new(|| re(format!(r"(?i)Event end:\s*({ISO_STAMP})")));
static ISO_RANGE: Lazy<Regex> =
    Lazy::new(|| re(format!(r"({ISO_STAMP})\s*{DASH}\s*({ISO_STAMP})")));
static ISO_ANY: Lazy<Regex> = Lazy::new(|| re(format!("({ISO_STAMP})")));
static NAMED_RANGE_WITH_TIME: Lazy<Regex> = Lazy::new(|| {
    re(format!(
        r"(?i)({W}+)\s+([0-9]{{1,2}}),?\s+([0-9]{{4}})\s+at\s+([0-9]{{1,2}}):([0-9]{{2}})\s*(AM|PM)\s*{DASH}\s*({W}+)\s+([0-9]{{1,2}}),?\s+([0-9]{{4}})\s+at\s+([0-9]{{1,2}}):([0-9]{{2}})\s*(AM|PM)"
    ))
});
static NAMED_START: Lazy<Regex> = Lazy::new(|| {
    re(format!(
        r"(?i)Event start:\s*({W}+)\s+([0-9]{{1,2}}),?\s+([0-9]{{4}})\s+at\s+([0-9]{{1,2}}):([0-9]{{2}})\s*(AM|PM)"
    ))
});
static DURATION_HINT: Lazy<Regex> =
    Lazy::new(|| re(r"(?i)([0-9]+)\s+(minute|hour)s?".to_string()));
static CROSS_MONTH: Lazy<Regex> = Lazy::new(|| {
    re(format!(
        r"(?i){MONTH_WORD}\s+{DAY}{join}{MONTH_WORD}\s+{DAY}(?:,?\s*([0-9]{{4}}))?",
        join = join()
    ))
});
/// The same-month form up to and including the first day and the joiner; the
/// second day, the lookahead and the year are matched by hand in
/// `same_month_range` because the regex crate has no lookahead.
static SAME_MONTH_HEAD: Lazy<Regex> =
    Lazy::new(|| re(format!(r"(?i){MONTH_WORD}\s+{DAY}{join}", join = join())));
static SECOND_DAY: Lazy<Regex> = Lazy::new(|| re(r"^([0-9]{1,2})((?i:st|nd|rd|th))?".into()));
static REJECT_AFTER_DAY: Lazy<Regex> = Lazy::new(|| re(r"^\s*[A-Za-z]{3,9}\s+[0-9]".into()));
static TRAILING_YEAR: Lazy<Regex> = Lazy::new(|| re(r"^,?\s*([0-9]{4})".into()));
/// "available while ...", "available as long as ...", "available until
/// further notice": availability stated as a condition rather than dates.
static ONGOING: Lazy<Regex> = Lazy::new(|| {
    re(r"(?i)\b(?:available|earnable|obtainable)\s+(?:while|as\s+long\s+as|until\s+further\s+notice)\b".into())
});
static YEAR: Lazy<Regex> = Lazy::new(|| re(r"\b(20[0-9]{2})\b".into()));
static NUMERIC_ENTITY: Lazy<Regex> = Lazy::new(|| re(r"&#([0-9]+);".into()));
static HEX_ENTITY: Lazy<Regex> = Lazy::new(|| re(r"&#x([0-9a-fA-F]+);".into()));

/// Month index from a full name, an abbreviation, or a prefix of at least three
/// letters ("Sept"). Case-insensitive; the free-form prose uses every casing.
pub(crate) fn month_num(name: &str) -> Option<u32> {
    if let Some(i) = crate::commands::badge_metadata::month_index(name) {
        return Some(i - 1);
    }
    let key = name.to_lowercase();
    if let Some(i) = ABBR_MONTHS.iter().position(|m| m.to_lowercase() == key) {
        return Some(i as u32);
    }
    if key.len() >= 3 {
        if let Some(i) = FULL_MONTHS
            .iter()
            .position(|m| m.to_lowercase().starts_with(&key))
        {
            return Some(i as u32);
        }
    }
    None
}

/// Scraped copy arrives with entities intact, which breaks the dash regexes.
pub fn decode_html_entities(text: &str) -> String {
    let from_code = |raw: &str, radix: u32| -> String {
        u32::from_str_radix(raw, radix)
            .ok()
            .and_then(char::from_u32)
            .map(String::from)
            .unwrap_or_default()
    };
    let decimal = NUMERIC_ENTITY.replace_all(text, |c: &regex::Captures| from_code(&c[1], 10));
    let mut result = HEX_ENTITY
        .replace_all(&decimal, |c: &regex::Captures| from_code(&c[1], 16))
        .into_owned();
    for (entity, ch) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
        ("&nbsp;", " "),
        ("&ndash;", "–"),
        ("&mdash;", "—"),
    ] {
        result = result.replace(entity, ch);
    }
    result
}

/// A local wall-clock instant, with JavaScript's `new Date(y, m, d, h, mi, s)`
/// semantics: an out-of-range day or minute rolls over rather than failing. A
/// time that falls in a DST gap is read as UTC so the window still exists.
fn local_ms(year: i32, month0: u32, day: i64, hour: i64, minute: i64, second: i64) -> Option<i64> {
    let first = NaiveDate::from_ymd_opt(year, month0 + 1, 1)?;
    let date = first.checked_add_signed(Duration::days(day - 1))?;
    let naive: NaiveDateTime = date.and_hms_opt(0, 0, 0)?
        + Duration::seconds(hour * 3600 + minute * 60 + second);
    naive_local_ms(&naive)
}

fn naive_local_ms(naive: &NaiveDateTime) -> Option<i64> {
    let mapped = Local.from_local_datetime(naive);
    mapped
        .earliest()
        .or_else(|| mapped.latest())
        .map(|dt| dt.timestamp_millis())
        .or_else(|| Some(Utc.from_utc_datetime(naive).timestamp_millis()))
}

/// The last second of the local day `ms` falls on, plus `extra_ms`, matching
/// `date.setHours(23, 59, 59, extra_ms)`.
fn end_of_local_day(ms: i64, extra_ms: i64) -> Option<i64> {
    let local = Local.timestamp_millis_opt(ms).single()?;
    let naive = local.date_naive().and_hms_opt(23, 59, 59)?;
    naive_local_ms(&naive).map(|m| m + extra_ms)
}

/// Parse a timestamp the way `new Date(string)` does for the shapes badge data
/// carries: RFC 3339 with a zone (absolute), a date-time without one (local),
/// or a bare date (UTC midnight).
pub(crate) fn parse_js_date(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    // Accept a lowercase `t` / `z` the way a case-insensitive capture can hand
    // them over, and the space separator some sources use.
    let normalized: String = {
        let mut out = s.to_string();
        if out.len() > 10 && matches!(out.as_bytes()[10], b't' | b' ') {
            out.replace_range(10..11, "T");
        }
        if out.ends_with('z') {
            out.pop();
            out.push('Z');
        }
        out
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&normalized) {
        return Some(dt.timestamp_millis());
    }
    // A zone suffix without seconds ("2026-07-24T07:00Z").
    for fmt in ["%Y-%m-%dT%H:%MZ", "%Y-%m-%dT%H:%M%:z"] {
        if let Ok(dt) = chrono::DateTime::parse_from_str(&normalized, fmt) {
            return Some(dt.timestamp_millis());
        }
        if fmt.ends_with('Z') {
            if let Ok(naive) = NaiveDateTime::parse_from_str(&normalized, fmt) {
                return Some(Utc.from_utc_datetime(&naive).timestamp_millis());
            }
        }
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(&normalized, fmt) {
            return naive_local_ms(&naive);
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(&normalized, "%Y-%m-%d") {
        return Some(Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0)?).timestamp_millis());
    }
    None
}

fn run(start: i64, end: i64) -> WindowRun {
    WindowRun { start_ms: Some(start), end_ms: Some(end) }
}

fn num(s: &str) -> i64 {
    s.parse().unwrap_or(0)
}

fn current_local_year() -> i32 {
    Local::now().year()
}

/// Pull an earn window out of free-form badge copy. Ordered most specific first:
/// an explicit "Event duration" line beats a bare range found anywhere in the
/// prose, and a form carrying a year beats one that must assume the current year.
pub(crate) fn parse_date_range(input: &str, current_year: i32) -> Option<WindowRun> {
    let text = decode_html_entities(input);
    let text = text.as_str();

    // "Event duration: December 6, 2025 – December 7, 2025" (full month, no time)
    if let Some(c) = FULL_NAMED_RANGE.captures(text) {
        if let (Some(sm), Some(em)) = (full_month_exact(&c[1]), full_month_exact(&c[4])) {
            let start = local_ms(num(&c[3]) as i32, sm, num(&c[2]), 0, 0, 0);
            let end = local_ms(num(&c[6]) as i32, em, num(&c[5]), 23, 59, 59);
            if let (Some(s), Some(e)) = (start, end) {
                return Some(run(s, e));
            }
        }
    }

    // "Event duration: Dec 19 – Jan 01" (abbreviated, may cross the year boundary)
    if let Some(c) = ABBREV_RANGE.captures(text) {
        if let (Some(sm), Some(em)) = (abbr_month_exact(&c[1]), abbr_month_exact(&c[3])) {
            // Dec to Jan means the end lands in the following year.
            let end_year = if sm > em { current_year + 1 } else { current_year };
            let start = local_ms(current_year, sm, num(&c[2]), 0, 0, 0);
            let end = local_ms(end_year, em, num(&c[4]), 23, 59, 59);
            if let (Some(s), Some(e)) = (start, end) {
                return Some(run(s, e));
            }
        }
    }

    // "Event duration: Dec 19-25" (same month)
    if let Some(c) = ABBREV_SAME_MONTH.captures(text) {
        if let Some(m) = abbr_month_exact(&c[1]) {
            let start = local_ms(current_year, m, num(&c[2]), 0, 0, 0);
            let end = local_ms(current_year, m, num(&c[3]), 23, 59, 59);
            if let (Some(s), Some(e)) = (start, end) {
                return Some(run(s, e));
            }
        }
    }

    // "Event start: 2025-12-04T15:00:00Z" with an optional matching "Event end:"
    if let Some(c) = ISO_START.captures(text) {
        if let Some(start) = parse_js_date(&c[1]) {
            let end = match ISO_END.captures(text) {
                Some(e) => parse_js_date(&e[1]),
                // No stated end: run to the end of that day.
                None => end_of_local_day(start, 999),
            };
            if let Some(e) = end {
                return Some(run(start, e));
            }
        }
    }

    // "2025-12-04T15:00:00Z – 2025-12-04T23:59:00Z"
    if let Some(c) = ISO_RANGE.captures(text) {
        if let (Some(s), Some(e)) = (parse_js_date(&c[1]), parse_js_date(&c[2])) {
            return Some(run(s, e));
        }
    }

    // "December 4, 2025 at 7:00 AM – December 4, 2025 at 11:59 PM"
    if let Some(c) = NAMED_RANGE_WITH_TIME.captures(text) {
        let start = named_date_time(&c[1], &c[2], &c[3], &c[4], &c[5], &c[6]);
        let end = named_date_time(&c[7], &c[8], &c[9], &c[10], &c[11], &c[12]);
        if let (Some(s), Some(e)) = (start, end) {
            return Some(run(s, e));
        }
    }

    // "Event start: December 4, 2025 at 9:00 AM", optionally with a duration hint
    if let Some(c) = NAMED_START.captures(text) {
        if let Some(start) = named_date_time(&c[1], &c[2], &c[3], &c[4], &c[5], &c[6]) {
            // Rest of that day, unless the copy states a duration.
            let end = match duration_ms(text) {
                Some(d) => Some(start + d),
                None => end_of_local_day(start, 0),
            };
            if let Some(e) = end {
                return Some(run(start, e));
            }
        }
    }

    // Cross-month range in either spelling and any joiner: "Dec 06 - Dec 07",
    // "December 2 - December 13", "between May 29 and June 3",
    // "from February 27 to March 3", "June 24 - July 12, 2025".
    for c in CROSS_MONTH.captures_iter(text) {
        // The month slot matches any word, so skip candidates like "period 2 to
        // June 3" and keep scanning rather than giving up on the first hit.
        let (Some(sm), Some(em)) = (month_num(&c[1]), month_num(&c[3])) else {
            continue;
        };
        let y = c.get(5).map(|m| num(m.as_str()) as i32).unwrap_or(current_year);
        let start = local_ms(y, sm, num(&c[2]), 0, 0, 0);
        // Dec to Jan means the end lands in the following year.
        let end = local_ms(if sm > em { y + 1 } else { y }, em, num(&c[4]), 23, 59, 59);
        if let (Some(s), Some(e)) = (start, end) {
            return Some(run(s, e));
        }
    }

    // Same-month range: "Dec 1-12", "December 3-15", "April 1-4, 2025",
    // "June 28-29".
    same_month_range(text, current_year)
}

/// `MONTH DAY JOIN DAY` not followed by another `MONTH DAY`, with an optional
/// trailing year. The follow-up check is what stops this eating the first half
/// of a cross-month range the branch above declined. The second day is tried
/// longest first and then shorter, the order a backtracking engine would try.
fn same_month_range(text: &str, current_year: i32) -> Option<WindowRun> {
    let mut from = 0;
    while from <= text.len() {
        let c = SAME_MONTH_HEAD.captures_at(text, from)?;
        let whole = c.get(0)?;
        let rest = &text[whole.end()..];
        let mut accepted: Option<(i64, usize)> = None;
        if let Some(day) = SECOND_DAY.captures(rest) {
            let digits = day.get(1)?.as_str();
            let with_suffix = day.get(0)?.end();
            let mut candidates = vec![(digits, with_suffix)];
            if day.get(2).is_some() {
                candidates.push((digits, digits.len()));
            }
            if digits.len() == 2 {
                candidates.push((&digits[..1], 1));
            }
            for (d, consumed) in candidates {
                if !REJECT_AFTER_DAY.is_match(&rest[consumed..]) {
                    accepted = Some((num(d), consumed));
                    break;
                }
            }
        }
        if let Some((end_day, consumed)) = accepted {
            let after = &rest[consumed..];
            let year = TRAILING_YEAR.captures(after);
            if let Some(m) = month_num(&c[1]) {
                let y = year
                    .as_ref()
                    .and_then(|y| y.get(1))
                    .map(|y| num(y.as_str()) as i32)
                    .unwrap_or(current_year);
                let start = local_ms(y, m, num(&c[2]), 0, 0, 0);
                let end = local_ms(y, m, end_day, 23, 59, 59);
                if let (Some(s), Some(e)) = (start, end) {
                    return Some(run(s, e));
                }
            }
            // A match that was not a month: keep scanning after the whole of it,
            // day and year included, as `matchAll` does.
            let year_len = year.and_then(|y| y.get(0)).map(|y| y.end()).unwrap_or(0);
            from = whole.end() + consumed + year_len;
        } else {
            // Nothing after the joiner satisfied the pattern at this start; try
            // the next character, as a regex engine would.
            from = next_char_boundary(text, whole.start());
        }
    }
    None
}

fn next_char_boundary(text: &str, at: usize) -> usize {
    text[at..]
        .chars()
        .next()
        .map(|ch| at + ch.len_utf8())
        .unwrap_or(text.len() + 1)
}

fn named_date_time(
    month_name: &str,
    day: &str,
    year: &str,
    hours: &str,
    minutes: &str,
    meridiem: &str,
) -> Option<i64> {
    let month = full_month_exact(month_name)?;
    let mut h = num(hours);
    let upper = meridiem.to_uppercase();
    if upper == "PM" && h != 12 {
        h += 12;
    } else if upper == "AM" && h == 12 {
        h = 0;
    }
    local_ms(num(year) as i32, month, num(day), h, num(minutes), 0)
}

/// "(N) minute(s)" or "(N) hour(s)" anywhere in the copy, as milliseconds.
fn duration_ms(text: &str) -> Option<i64> {
    let c = DURATION_HINT.captures(text)?;
    let amount = num(&c[1]);
    Some(if c[2].eq_ignore_ascii_case("minute") {
        amount * 60_000
    } else {
        amount * 3_600_000
    })
}

/// The campaign window on a relay enrichment, read the way a JS `Date` reads it.
fn enrichment_window(enrichment: Option<&serde_json::Value>) -> Option<WindowRun> {
    let iso = |key: &str| {
        enrichment?
            .get(key)?
            .as_str()
            .and_then(parse_js_date)
    };
    let start = iso("starts_utc");
    let end = iso("ends_utc");
    if start.is_none() && end.is_none() {
        return None;
    }
    Some(WindowRun { start_ms: start, end_ms: end })
}

/// Resolve every run a badge is earnable in, most authoritative source first:
/// the campaign window on the relay enrichment, then ISO stamps in the copy,
/// then prose. `None` when there is no window, which is correct for permanent
/// badges (subscriber tenure, founder, Prime).
pub fn resolve(
    more_info: Option<&str>,
    enrichment: Option<&serde_json::Value>,
) -> Option<ResolvedWindow> {
    resolve_with_year(more_info, enrichment, current_local_year())
}

pub(crate) fn resolve_with_year(
    more_info: Option<&str>,
    enrichment: Option<&serde_json::Value>,
    current_year: i32,
) -> Option<ResolvedWindow> {
    if let Some(window) = enrichment_window(enrichment) {
        return Some(ResolvedWindow { runs: vec![window], source: WindowSource::Campaign });
    }

    let copy = more_info.filter(|s| !s.is_empty())?;

    // ISO stamps in the copy. One stamp plus a duration hint is a window; one
    // stamp alone runs to the end of that day.
    let stamps: Vec<&str> = ISO_ANY.find_iter(copy).map(|m| m.as_str()).collect();
    if stamps.len() == 1 {
        if let Some(start) = parse_js_date(stamps[0]) {
            let end = match duration_ms(copy) {
                Some(d) => Some(start + d),
                None => end_of_local_day(start, 0),
            };
            if let Some(e) = end {
                return Some(ResolvedWindow { runs: vec![run(start, e)], source: WindowSource::CopyIso });
            }
        }
    } else if stamps.len() >= 2 {
        let ms: Vec<i64> = stamps.iter().filter_map(|s| parse_js_date(s)).collect();
        if ms.len() >= 2 {
            // A campaign can run in separate bursts ("First Release ... Second
            // Release ..."). Treating the stamps as one span would report the
            // badge earnable during the gap, so pair them into runs. Odd counts
            // fall back to the outer span.
            let runs = if ms.len() % 2 == 0 {
                ms.chunks(2)
                    .map(|p| run(p[0].min(p[1]), p[0].max(p[1])))
                    .collect()
            } else {
                vec![run(*ms.iter().min()?, *ms.iter().max()?)]
            };
            return Some(ResolvedWindow { runs, source: WindowSource::CopyIso });
        }
    }

    // Prose formats. Year-less ones assume the current year.
    if let Some(w) = parse_date_range(copy, current_year) {
        return Some(ResolvedWindow { runs: vec![w], source: WindowSource::Prose });
    }

    ongoing_window(copy)
}

/// A badge the copy calls available on an ongoing condition, with no dates.
/// Needs a year in the copy to close the window; without one it stays
/// unknown, because an open window with no end would outlive the event.
fn ongoing_window(copy: &str) -> Option<ResolvedWindow> {
    if !ONGOING.is_match(copy) {
        return None;
    }
    let year: i32 = YEAR
        .captures_iter(copy)
        .filter_map(|c| c[1].parse().ok())
        .max()?;
    let end = local_ms(year, 11, 31, 23, 59, 59)?;
    Some(ResolvedWindow {
        runs: vec![WindowRun { start_ms: None, end_ms: Some(end) }],
        source: WindowSource::Ongoing,
    })
}

/// The campaign window alone, parsed strictly as RFC 3339, which is exactly how
/// the toast and Android notification path has always read it. Those paths must
/// never fire off a prose window: year-less prose ("Dec 1-12") re-matches every
/// year, so an old badge would announce itself again each December.
pub fn campaign_runs(enrichment: Option<&serde_json::Value>) -> Option<Vec<WindowRun>> {
    let iso = |key: &str| -> Option<i64> {
        enrichment?
            .get(key)?
            .as_str()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis())
    };
    let start = iso("starts_utc");
    let end = iso("ends_utc");
    if start.is_none() && end.is_none() {
        return None;
    }
    Some(vec![WindowRun { start_ms: start, end_ms: end }])
}

/// Classify a window against `now_ms`: inside any run is available, a run still
/// ahead is coming soon, otherwise it is over.
pub fn status_at(runs: &[WindowRun], now_ms: i64) -> WindowStatus {
    let start = |r: &WindowRun| r.start_ms.unwrap_or(i64::MIN);
    let end = |r: &WindowRun| r.end_ms.unwrap_or(i64::MAX);
    if runs.iter().any(|r| now_ms >= start(r) && now_ms <= end(r)) {
        WindowStatus::Available
    } else if runs.iter().any(|r| now_ms < start(r)) {
        WindowStatus::ComingSoon
    } else {
        WindowStatus::Expired
    }
}

/// The run containing `now_ms`, if any.
pub fn run_containing(runs: &[WindowRun], now_ms: i64) -> Option<WindowRun> {
    runs.iter()
        .find(|r| {
            now_ms >= r.start_ms.unwrap_or(i64::MIN) && now_ms <= r.end_ms.unwrap_or(i64::MAX)
        })
        .copied()
}

/// The next instant after `now_ms` at which `status_at` can change: the soonest
/// run start or end still ahead. An end is inclusive, so the change lands one
/// millisecond after it.
pub fn next_boundary_after(runs: &[WindowRun], now_ms: i64) -> Option<i64> {
    runs.iter()
        .flat_map(|r| [r.start_ms, r.end_ms.map(|e| e + 1)])
        .flatten()
        .filter(|t| *t > now_ms)
        .min()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;
    use serde_json::json;

    fn at(iso: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(iso).unwrap().timestamp_millis()
    }

    fn local(ms: i64) -> chrono::DateTime<Local> {
        Local.timestamp_millis_opt(ms).single().unwrap()
    }

    fn range(text: &str) -> Option<WindowRun> {
        parse_date_range(text, current_local_year())
    }

    fn status(more: Option<&str>, enrichment: Option<serde_json::Value>, now: i64) -> Option<WindowStatus> {
        resolve(more, enrichment.as_ref()).map(|w| status_at(&w.runs, now))
    }

    fn start(r: &WindowRun) -> chrono::DateTime<Local> {
        local(r.start_ms.unwrap())
    }

    fn end(r: &WindowRun) -> chrono::DateTime<Local> {
        local(r.end_ms.unwrap())
    }

    #[test]
    fn decodes_the_entities_that_scraped_copy_arrives_with() {
        assert_eq!(decode_html_entities("Dec 6 &ndash; Dec 7"), "Dec 6 – Dec 7");
        assert_eq!(decode_html_entities("Chaos Orb &#8220;PoE2&#8221;"), "Chaos Orb “PoE2”");
    }

    #[test]
    fn parses_a_full_month_range_with_years() {
        let r = range("Event duration: December 6, 2025 – December 7, 2025").unwrap();
        assert_eq!(start(&r).year(), 2025);
        assert_eq!(start(&r).month0(), 11);
        assert_eq!(start(&r).day(), 6);
        assert_eq!(end(&r).day(), 7);
    }

    #[test]
    fn parses_an_abbreviated_range_that_crosses_into_the_next_year() {
        let r = range("Event duration: Dec 19 – Jan 01").unwrap();
        assert_eq!(end(&r).year(), start(&r).year() + 1);
    }

    #[test]
    fn parses_an_iso_range() {
        let r = range("Event duration: 2026-07-08T13:00:00Z - 2026-08-24T13:00:00Z").unwrap();
        assert_eq!(r.start_ms, Some(at("2026-07-08T13:00:00Z")));
        assert_eq!(r.end_ms, Some(at("2026-08-24T13:00:00Z")));
    }

    #[test]
    fn parses_an_am_pm_range() {
        let r = range("December 4, 2025 at 7:00 AM – December 4, 2025 at 11:59 PM").unwrap();
        assert_eq!(start(&r).hour(), 7);
        assert_eq!(end(&r).hour(), 23);
    }

    #[test]
    fn parses_an_abbreviated_same_month_range() {
        let r = range("Event duration: Jul 23 - Jul 25").unwrap();
        assert_eq!(start(&r).month0(), 6);
        assert_eq!(start(&r).day(), 23);
        assert_eq!(end(&r).day(), 25);
    }

    #[test]
    fn a_campaign_window_beats_anything_written_in_the_prose() {
        let s = status(
            Some("Event duration: Jan 01 - Jan 02"),
            Some(json!({ "starts_utc": "2026-07-24T07:00:00Z", "ends_utc": "2026-07-27T08:00:00Z" })),
            at("2026-07-24T22:00:00Z"),
        );
        assert_eq!(s, Some(WindowStatus::Available));
    }

    #[test]
    fn reclassifies_a_badge_across_its_window_with_no_new_push() {
        let w = json!({ "starts_utc": "2026-07-24T07:00:00Z", "ends_utc": "2026-07-27T08:00:00Z" });
        assert_eq!(status(None, Some(w.clone()), at("2026-07-24T06:46:00Z")), Some(WindowStatus::ComingSoon));
        assert_eq!(status(None, Some(w.clone()), at("2026-07-24T22:51:00Z")), Some(WindowStatus::Available));
        assert_eq!(status(None, Some(w), at("2026-07-28T00:00:00Z")), Some(WindowStatus::Expired));
    }

    #[test]
    fn an_open_ended_window_is_available_once_it_has_started() {
        let w = json!({ "starts_utc": "2026-07-25T17:00:00Z" });
        assert_eq!(status(None, Some(w.clone()), at("2026-07-26T00:00:00Z")), Some(WindowStatus::Available));
        assert_eq!(status(None, Some(w), at("2026-07-24T00:00:00Z")), Some(WindowStatus::ComingSoon));
    }

    #[test]
    fn falls_back_to_iso_stamps_in_the_copy_when_there_is_no_campaign() {
        let s = status(
            Some("Event duration: 2026-07-08T13:00:00Z - 2026-08-24T13:00:00Z"),
            None,
            at("2026-07-24T00:00:00Z"),
        );
        assert_eq!(s, Some(WindowStatus::Available));
    }

    #[test]
    fn parses_full_month_names_not_just_three_letter_abbreviations() {
        for (text, label) in [
            ("during the campaign period (December 2 – December 13)", "clip-the-halls"),
            ("Event time: July 24 – July 25", "budz"),
            ("during the event: July 10 – July 12", "dreamers"),
            ("Time window: June 24 – July 12, 2025", "league-of-legends-msi-grey"),
        ] {
            assert!(range(text).is_some(), "failed to parse {label}: {text}");
        }
    }

    #[test]
    fn parses_between_and_from_to_joiners() {
        let between = range("share a clip from the category between May 29 and June 3").unwrap();
        assert_eq!(start(&between).month0(), 4);
        assert_eq!(end(&between).month0(), 5);

        let from = range("the campaign ran from February 27 to March 3").unwrap();
        assert_eq!(start(&from).month0(), 1);
        assert_eq!(end(&from).month0(), 2);
    }

    #[test]
    fn parses_ordinal_days_and_same_month_compact_ranges() {
        let ordinal = range("completed the survey from July 26th to July 28th, 2024").unwrap();
        assert_eq!(start(&ordinal).year(), 2024);
        assert_eq!(start(&ordinal).day(), 26);

        let compact = range("the “Together For Good” campaign (December 3–15)").unwrap();
        assert_eq!(start(&compact).day(), 3);
        assert_eq!(end(&compact).day(), 15);

        let trailing = range("Event window : June 28–29 This is the first time").unwrap();
        assert_eq!(end(&trailing).day(), 29);
    }

    #[test]
    fn a_leading_non_month_word_does_not_block_a_later_real_range() {
        let r = range("the campaign period 2 to June 3 was extended, running May 29 to June 3").unwrap();
        assert_eq!(start(&r).month0(), 4);
    }

    #[test]
    fn a_split_campaign_is_not_earnable_in_the_gap_between_its_runs() {
        // borderlands-4-ripper: two disjoint releases, months apart.
        let copy = "First Release: from 2025-06-21T15:00:00Z to 2025-06-22T00:00:00Z \
                    Second Release: from 2025-09-11T12:00:00Z to 2025-09-15T06:59:00Z";
        assert_eq!(status(Some(copy), None, at("2025-06-21T18:00:00Z")), Some(WindowStatus::Available));
        assert_eq!(status(Some(copy), None, at("2025-07-15T00:00:00Z")), Some(WindowStatus::ComingSoon));
        assert_eq!(status(Some(copy), None, at("2025-09-12T00:00:00Z")), Some(WindowStatus::Available));
        assert_eq!(status(Some(copy), None, at("2025-10-01T00:00:00Z")), Some(WindowStatus::Expired));
    }

    #[test]
    fn a_permanent_badge_has_no_window_and_no_status() {
        assert_eq!(resolve(Some("Given to channel subscribers."), None), None);
        assert_eq!(resolve(None, None), None);
        assert_eq!(resolve(None, Some(&json!({}))), None);
    }

    #[test]
    fn ignores_a_malformed_campaign_timestamp_rather_than_inventing_a_status() {
        assert_eq!(resolve(None, Some(&json!({ "starts_utc": "not a date" }))), None);
    }

    // Every shape badge_metadata::store_enrichment_metadata writes into the copy.
    #[test]
    fn reads_every_event_duration_shape_the_enrichment_writer_emits() {
        let prose = resolve_with_year(Some("Watch it.\n\nEvent duration: Jul 23 - Jul 25"), None, 2026).unwrap();
        assert_eq!(prose.source, WindowSource::Prose);
        assert_eq!(start(&prose.runs[0]).day(), 23);
        assert_eq!(end(&prose.runs[0]).day(), 25);

        let iso = resolve(Some("Event duration: 2026-09-24T12:00:00Z - 2026-10-10T11:59:59Z"), None).unwrap();
        assert_eq!(iso.source, WindowSource::CopyIso);
        assert_eq!(iso.runs, vec![run(at("2026-09-24T12:00:00Z"), at("2026-10-10T11:59:59Z"))]);

        // A lone "from" or "until" stamp is one stamp: it runs to the end of that
        // local day, exactly as the TypeScript classifier read it.
        let from = resolve(Some("Event duration: from 2026-09-24T12:00:00Z"), None).unwrap();
        assert_eq!(from.runs[0].start_ms, Some(at("2026-09-24T12:00:00Z")));
        assert_eq!(end(&from.runs[0]).hour(), 23);

        let until = resolve(Some("Event duration: until 2026-09-24T12:00:00Z"), None).unwrap();
        assert_eq!(until.source, WindowSource::CopyIso);
    }

    #[test]
    fn tags_each_source() {
        let campaign = resolve(None, Some(&json!({ "ends_utc": "2026-10-10T11:59:59Z" }))).unwrap();
        assert_eq!(campaign.source, WindowSource::Campaign);
        assert_eq!(campaign.runs, vec![WindowRun { start_ms: None, end_ms: Some(at("2026-10-10T11:59:59Z")) }]);
        assert_eq!(resolve(Some("2026-07-08T13:00:00Z"), None).unwrap().source, WindowSource::CopyIso);
        assert_eq!(resolve(Some("Dec 1-12"), None).unwrap().source, WindowSource::Prose);
    }

    #[test]
    fn a_single_copy_stamp_with_a_duration_hint_lasts_that_long() {
        let w = resolve(Some("Event start: 2026-07-08T13:00:00Z for 90 minutes"), None).unwrap();
        assert_eq!(w.runs, vec![run(at("2026-07-08T13:00:00Z"), at("2026-07-08T14:30:00Z"))]);
    }

    // TwitchCon San Diego 2026's ticket badges, verbatim from badgebase: no
    // dates at all, only a condition. They read as "not available" before.
    const TACO: &str = "Taco badge is a limited-time global chat badge awarded to attendees of         TwitchCon San Diego 2026. The badge is granted to anyone who purchases a 3-day pass for         TwitchCon San Diego 2026. The badge will be available while TwitchCon San Diego 2026         tickets are on sale. Ticket purchase page: https://www.twitchcon.com/san-diego-2026/passes/";

    #[test]
    fn a_badge_available_while_tickets_are_on_sale_is_available_now() {
        let w = resolve(Some(TACO), None).unwrap();
        assert_eq!(w.source, WindowSource::Ongoing);
        assert_eq!(status_at(&w.runs, at("2026-09-24T12:00:00Z")), WindowStatus::Available);
        // Closed at the end of the year it names, so it cannot stay open forever.
        assert_eq!(status_at(&w.runs, at("2027-01-02T12:00:00Z")), WindowStatus::Expired);
        assert_eq!(end(&w.runs[0]).year(), 2026);
    }

    #[test]
    fn an_ongoing_condition_without_a_year_stays_unknown() {
        assert_eq!(resolve(Some("The badge will be available while supplies last."), None), None);
        // Dates, when present, still win over the condition.
        let dated = resolve(Some("Available while the event runs, Event duration: Jul 23 - Jul 25 2026"), None).unwrap();
        assert_eq!(dated.source, WindowSource::Prose);
    }

    #[test]
    fn campaign_runs_ignores_prose_and_copy() {
        assert_eq!(campaign_runs(None), None);
        assert_eq!(campaign_runs(Some(&json!({ "action": "Watch Dec 1-12" }))), None);
        assert_eq!(
            campaign_runs(Some(&json!({ "starts_utc": "2026-07-24T07:00:00Z" }))),
            Some(vec![WindowRun { start_ms: Some(at("2026-07-24T07:00:00Z")), end_ms: None }])
        );
    }

    #[test]
    fn next_boundary_is_the_soonest_change_still_ahead() {
        let runs = vec![
            run(at("2026-07-01T00:00:00Z"), at("2026-07-10T00:00:00Z")),
            run(at("2026-08-01T00:00:00Z"), at("2026-08-10T00:00:00Z")),
        ];
        let now = at("2026-07-05T00:00:00Z");
        assert_eq!(next_boundary_after(&runs, now), Some(at("2026-07-10T00:00:00Z") + 1));
        let later = at("2026-07-20T00:00:00Z");
        assert_eq!(next_boundary_after(&runs, later), Some(at("2026-08-01T00:00:00Z")));
        assert_eq!(next_boundary_after(&runs, at("2026-09-01T00:00:00Z")), None);
        let open = vec![WindowRun { start_ms: None, end_ms: None }];
        assert_eq!(next_boundary_after(&open, now), None);
    }

    #[test]
    fn the_status_flips_exactly_at_the_boundary() {
        let runs = vec![run(1_000, 2_000)];
        assert_eq!(status_at(&runs, 999), WindowStatus::ComingSoon);
        assert_eq!(status_at(&runs, 1_000), WindowStatus::Available);
        assert_eq!(status_at(&runs, 2_000), WindowStatus::Available);
        assert_eq!(status_at(&runs, 2_001), WindowStatus::Expired);
        assert_eq!(next_boundary_after(&runs, 1_500), Some(2_001));
    }

    #[test]
    fn month_num_accepts_every_spelling_the_prose_uses() {
        assert_eq!(month_num("December"), Some(11));
        assert_eq!(month_num("dec"), Some(11));
        assert_eq!(month_num("Sept"), Some(8));
        assert_eq!(month_num("period"), None);
        assert_eq!(month_num("Ma"), None);
    }

    #[test]
    fn a_day_followed_by_another_month_is_not_a_same_month_range() {
        // "June 2 - 13 July 5": the second day must not be taken when a month and
        // a day follow it, so this is not "June 2 to 13".
        let r = same_month_range("June 2 - 13 July 5", 2026);
        assert!(r.map(|w| end(&w).day() != 13).unwrap_or(true));
    }
}
