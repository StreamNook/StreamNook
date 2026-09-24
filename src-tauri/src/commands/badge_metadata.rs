use crate::services::universal_cache_service::{cache_item, get_cached_item, CacheType};
use log::debug;
use once_cell::sync::Lazy;
use regex::Regex;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

/// Cache `source` marking a `metadata:{set}-v{ver}` entry as relay-supplied
/// enrichment (campaign-grounded, richer than the badgebase scrape). Entries
/// with this source are served as-is and never re-scraped over.
pub const ENRICHMENT_SOURCE: &str = "socket-enrichment";

/// Discord-only markup that must never reach the desktop: custom emoji
/// `<:name:id>` / `<a:name:id>` and role/user/channel mentions `<@…>` / `<#…>`.
/// The relay composes some fields (e.g. `related`) with these for its Discord
/// post, and that same text rides into the drop payload.
static DISCORD_TOKEN_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<a?:\w+:\d+>|<[@#][!&]?\d+>").unwrap());

/// Strip Discord markup and collapse whitespace so relay-supplied text renders
/// cleanly in the desktop badge panel.
fn clean_text(s: &str) -> String {
    let stripped = DISCORD_TOKEN_RE.replace_all(s, " ");
    stripped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fractional seconds in an ISO timestamp (`...:00.000Z`).
static ISO_FRACTION_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\.\d+(Z|[+-]\d{2}:?\d{2})").unwrap());

/// Drop fractional seconds so the timestamp is second-precision (`...:00Z`). The
/// panel's inline date converter only matches to seconds, so a millisecond
/// timestamp otherwise loses its `Z` (parsed as local, not UTC) and leaves a
/// literal `.000Z` in the rendered text.
fn normalize_iso(s: &str) -> String {
    ISO_FRACTION_RE.replace(s, "$1").into_owned()
}

const MONTH_NAMES: &str =
    "january|february|march|april|may|june|july|august|september|october|november|december";

/// A prose date RANGE: "July 23 through July 25", "July 23 - August 2",
/// "July 23, 2026 to July 25". An optional ", YYYY" after each day is tolerated
/// and ignored — a range renders year-less so the app applies the current year.
static PROSE_RANGE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?i)\b({m})\s+(\d{{1,2}})(?:st|nd|rd|th)?(?:,?\s*\d{{4}})?\s*(?:through|thru|to|until|[-–—])\s*(?:({m})\s+)?(\d{{1,2}})(?:st|nd|rd|th)?\b",
        m = MONTH_NAMES
    ))
    .unwrap()
});

/// A single EXPLICIT date with a year (and optional time): "July 25, 2026",
/// "July 25 2026 at 17:00 UTC". Rendered as ISO so the app reads it as UTC.
static PROSE_SINGLE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?i)\b({m})\s+(\d{{1,2}})(?:st|nd|rd|th)?,?\s+(\d{{4}})(?:\s+(?:at\s+)?(\d{{1,2}}):(\d{{2}}))?",
        m = MONTH_NAMES
    ))
    .unwrap()
});

/// 1-based month from a full month name, any casing. Shared with the badge
/// window classifier so there is one month table for full names.
pub(crate) fn month_index(m: &str) -> Option<u32> {
    [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ]
    .iter()
    .position(|&n| n == m.to_lowercase())
    .map(|i| i as u32 + 1)
}

fn month_abbr(m: &str) -> &'static str {
    const ABBR: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    month_index(m).map(|i| ABBR[(i - 1) as usize]).unwrap_or("")
}

/// Today's date as "D Month YYYY" (e.g. "23 July 2026") — the same human shape
/// badgebase uses for Date of Addition, so it both displays cleanly and sorts.
fn today_date_string() -> String {
    use chrono::{Datelike, Local};
    let dt = Local::now();
    const MONTHS: [&str; 12] = [
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
    format!("{} {} {}", dt.day(), MONTHS[(dt.month() - 1) as usize], dt.year())
}

/// Recover an earn window from our own enricher prose so a badge with no
/// authoritative campaign window can still be placed in time. Tries a range
/// first (rendered "Mon D - Mon D"; the app fills the current year), then a
/// single explicit date with a year (rendered ISO; the app reads it as UTC).
/// Returns a value ready to follow "Event duration: ".
fn extract_prose_window(text: &str) -> Option<String> {
    if let Some(c) = PROSE_RANGE_RE.captures(text) {
        let m1 = month_abbr(c.get(1)?.as_str());
        if !m1.is_empty() {
            let d1 = c.get(2)?.as_str();
            let m2 = c
                .get(3)
                .map(|m| month_abbr(m.as_str()))
                .filter(|s| !s.is_empty())
                .unwrap_or(m1);
            let d2 = c.get(4)?.as_str();
            return Some(format!("{} {} - {} {}", m1, d1, m2, d2));
        }
    }
    if let Some(c) = PROSE_SINGLE_RE.captures(text) {
        if let Some(mo) = month_index(c.get(1)?.as_str()) {
            let day: u32 = c.get(2)?.as_str().parse().ok()?;
            let year: i32 = c.get(3)?.as_str().parse().ok()?;
            let hour: u32 = c.get(4).and_then(|h| h.as_str().parse().ok()).unwrap_or(0);
            let min: u32 = c.get(5).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
            return Some(format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:00Z",
                year, mo, day, hour, min
            ));
        }
    }
    None
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BadgeMetadata {
    pub date_added: Option<String>,
    pub usage_stats: Option<String>,
    pub more_info: Option<String>,
    /// Raw relay enrichment object (campaign facts, siblings, window), so the
    /// detail panel can render structured sections. Absent for badgebase badges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrichment: Option<serde_json::Value>,
    #[serde(skip_serializing)]
    pub info_url: String,
}

/// Badge metadata for caching (without URL)
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BadgeMetadataCached {
    pub date_added: Option<String>,
    pub usage_stats: Option<String>,
    pub more_info: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrichment: Option<serde_json::Value>,
}

/// Fetch additional badge metadata information
#[tauri::command]
pub async fn fetch_badge_metadata(
    badge_set_id: String,
    badge_version: String,
    force: Option<bool>,
) -> Result<BadgeMetadata, String> {
    // Create cache key with metadata prefix to distinguish from badge data.
    let cache_key = format!("metadata:{}-v{}", badge_set_id, badge_version);

    // Construct the info URL for response
    let url = format!(
        "https://badgebase.co/badges/{}-v{}/",
        badge_set_id, badge_version
    );

    // Relay enrichment is richer than the scrape, so it wins even over a force
    // refresh. Checked ahead of the force branch so the gallery's refresh
    // button cannot scrape over it.
    if let Ok(Some(cached)) = get_cached_item(CacheType::Badge, &cache_key).await {
        if cached.metadata.source == ENRICHMENT_SOURCE {
            if let Ok(cached_info) = serde_json::from_value::<BadgeMetadataCached>(cached.data) {
                return Ok(BadgeMetadata {
                    date_added: cached_info.date_added,
                    usage_stats: cached_info.usage_stats,
                    more_info: cached_info.more_info,
                    enrichment: cached_info.enrichment,
                    info_url: url,
                });
            }
        }
    }

    // Check universal cache first (unless force refresh is requested)
    let should_force = force.unwrap_or(false);
    if !should_force {
        debug!("[BadgeMetadata] Checking cache for: {}", cache_key);
        if let Ok(Some(cached)) = get_cached_item(CacheType::Badge, &cache_key).await {
            debug!("[BadgeMetadata] Found in cache: {}", cache_key);
            if let Ok(cached_info) = serde_json::from_value::<BadgeMetadataCached>(cached.data) {
                if is_more_info_stale(cached_info.more_info.as_deref()) {
                    debug!(
                        "[BadgeMetadata] Cached more_info looks stale, refetching: {}",
                        cache_key
                    );
                } else if is_usage_stats_missing(cached_info.usage_stats.as_deref()) {
                    debug!(
                        "[BadgeMetadata] Cached usage_stats missing, refetching: {}",
                        cache_key
                    );
                } else {
                    return Ok(BadgeMetadata {
                        date_added: cached_info.date_added,
                        usage_stats: cached_info.usage_stats,
                        more_info: cached_info.more_info,
                        enrichment: cached_info.enrichment,
                        info_url: url,
                    });
                }
            }
        }
    } else {
        debug!("[BadgeMetadata] Force refresh requested for: {}", cache_key);
    }

    debug!("[BadgeMetadata] Fetching info from: {}", url);

    // Fetch the HTML page
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch badge metadata page: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Badge metadata source returned status: {}",
            response.status()
        ));
    }

    let html_content = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response body: {}", e))?;

    // Parse the HTML and extract data in a separate scope
    let (date_added, usage_stats, more_info) = {
        let document = Html::parse_document(&html_content);
        let date_added = extract_date_added(&document);
        let usage_stats = extract_usage_stats(&document);
        let more_info = extract_more_info(&document);
        (date_added, usage_stats, more_info)
    }; // document is dropped here

    // Create cached version without URL
    let cached_info = BadgeMetadataCached {
        date_added: date_added.clone(),
        usage_stats: usage_stats.clone(),
        more_info: more_info.clone(),
        enrichment: None,
    };

    // Cache the result permanently (expiry_days = 0 means never expire)
    if let Ok(json_value) = serde_json::to_value(&cached_info) {
        let _ = cache_item(
            CacheType::Badge,
            cache_key,
            json_value,
            "badgebase".to_string(),
            0, // Never expire
        )
        .await;
        debug!("[BadgeMetadata] Cached badge info permanently");
    }

    // Return full info with URL
    Ok(BadgeMetadata {
        date_added,
        usage_stats,
        more_info,
        enrichment: None,
        info_url: url,
    })
}

/// Compose the badge More Info fields from a relay-pushed `enrichment` object
/// and cache them under the same `metadata:{set}-v{ver}` key the More Info panel
/// reads, marked `ENRICHMENT_SOURCE` so a badgebase re-scrape never overwrites
/// it. The window is emitted as ISO timestamps so the panel highlights it and
/// derives Available / Coming Soon / Expired. Best-effort; a no-content
/// enrichment is skipped so the panel falls back to badgebase.
pub async fn store_enrichment_metadata(
    badge_set_id: &str,
    badge_version: &str,
    enrichment: &serde_json::Value,
) {
    let field = |k: &str| {
        enrichment
            .get(k)
            .and_then(|v| v.as_str())
            .map(clean_text)
            .filter(|s| !s.is_empty())
    };

    let mut parts: Vec<String> = Vec::new();
    let body = field("how_to_earn").or_else(|| field("action"));
    let body_lc = body.as_deref().unwrap_or("").to_lowercase();
    if let Some(b) = &body {
        parts.push(b.clone());
    }
    // Surface highlight/caveats only when they say something the main paragraph
    // doesn't already (the campaign-grounded how_to_earn usually folds the Prime
    // note and channel detail in, so these would just repeat it).
    for key in ["highlight", "caveats"] {
        if let Some(v) = field(key) {
            let v_lc = v.to_lowercase();
            // Redundant if the paragraph already contains it, or both are the
            // Prime-exclusion note phrased differently.
            let redundant =
                body_lc.contains(&v_lc) || (v_lc.contains("prime") && body_lc.contains("prime"));
            if !redundant {
                parts.push(v);
            }
        }
    }
    // distribution/footnote intentionally omitted: low-value on desktop and they
    // render as orphan fragments (a bare "Twitch Drops" line).
    let start = field("starts_utc");
    let end = field("ends_utc");
    match (&start, &end) {
        (Some(s), Some(e)) => parts.push(format!(
            "Event duration: {} - {}",
            normalize_iso(s),
            normalize_iso(e)
        )),
        (Some(s), None) => parts.push(format!("Event duration: from {}", normalize_iso(s))),
        (None, Some(e)) => parts.push(format!("Event duration: until {}", normalize_iso(e))),
        (None, None) => {
            // No authoritative window; recover one from our own prose (e.g. "July
            // 23 through July 25") so the app can still place the badge in time.
            if let Some(window) = body.as_deref().and_then(extract_prose_window) {
                parts.push(format!("Event duration: {}", window));
            }
        }
    }
    // Siblings LAST so the panel can split them off for chips while the date pills
    // stay in the paragraph. Newline-separated, may carry Discord emoji markup.
    if let Some(related_raw) = enrichment.get("related").and_then(|v| v.as_str()) {
        let items: Vec<String> = related_raw
            .split('\n')
            .map(clean_text)
            .filter(|s| !s.is_empty())
            .collect();
        if !items.is_empty() {
            parts.push(format!("Also part of this event: {}", items.join(", ")));
        }
    }

    // Drop duplicate lines (the relay often repeats the Prime note as both
    // `highlight` and `footnote`).
    let mut seen = std::collections::HashSet::new();
    parts.retain(|p| seen.insert(p.clone()));

    if parts.is_empty() {
        return;
    }

    let cache_key = format!("metadata:{}-v{}", badge_set_id, badge_version);

    // Carry forward what badgebase knew and the relay does not send.
    //
    // Date of Addition: keep a genuine human date, else stamp today's (never the
    // ISO earn-window) so a fresh drop sorts to the top of the date-newest
    // gallery. Non-ISO values survive later backfills, so the stamp is stable.
    //
    // Usage statistics: the relay has none, and nulling them would break the
    // most/least-used sort for every badge the feed touches.
    let existing = match get_cached_item(CacheType::Badge, &cache_key).await {
        Ok(Some(c)) => serde_json::from_value::<BadgeMetadataCached>(c.data).ok(),
        _ => None,
    };
    let date_added = Some(
        existing
            .as_ref()
            .and_then(|m| m.date_added.clone())
            .filter(|d| !d.contains('T'))
            .unwrap_or_else(today_date_string),
    );
    let usage_stats = existing.and_then(|m| m.usage_stats);

    let cached = BadgeMetadataCached {
        date_added,
        usage_stats,
        more_info: Some(parts.join("\n\n")),
        enrichment: Some(enrichment.clone()),
    };

    if let Ok(json_value) = serde_json::to_value(&cached) {
        let _ = cache_item(
            CacheType::Badge,
            cache_key,
            json_value,
            ENRICHMENT_SOURCE.to_string(),
            0, // Never expire
        )
        .await;
        debug!("[BadgeMetadata] Stored relay enrichment for {}-v{}", badge_set_id, badge_version);
    }
}

/// Returns true when `more_info` appears to describe an event window but is
/// missing the ISO 8601 timestamps the UI needs to classify the badge as
/// Available / Coming Soon / Expired. Caused by older entries scraped before
/// the timezone-converter element extractor accepted `<time datetime="…">`.
pub fn is_more_info_stale(more_info: Option<&str>) -> bool {
    let text = match more_info {
        Some(t) if !t.is_empty() => t,
        _ => return false,
    };
    let has_iso = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}")
        .map(|re| re.is_match(text))
        .unwrap_or(false);
    if has_iso {
        return false;
    }
    let lower = text.to_lowercase();
    lower.contains("event duration") || lower.contains(" utc")
}

/// Returns true when a cached badge has no usage statistics recorded. Older
/// entries were scraped before the source page served the "Usage Statistics"
/// figure, leaving the field null; those need a re-scrape so the most/least-used
/// sort has a value to order by. A genuinely zero-usage badge still scrapes to a
/// non-null string ("None users seen with this badge"), so this never loops.
pub fn is_usage_stats_missing(usage_stats: Option<&str>) -> bool {
    match usage_stats {
        Some(s) => s.trim().is_empty(),
        None => true,
    }
}

fn extract_date_added(document: &Html) -> Option<String> {
    // Look for the "Date of addition" label and get the next span
    let selector = Selector::parse("li").ok()?;

    for element in document.select(&selector) {
        let text = element.text().collect::<String>();
        if text.contains("Date of addition") {
            // Extract the date from the text
            let parts: Vec<&str> = text.split("Date of addition").collect();
            if parts.len() > 1 {
                return Some(parts[1].trim().to_string());
            }
        }
    }

    None
}

fn extract_usage_stats(document: &Html) -> Option<String> {
    // Look for the "Usage Statistics" section
    let selector = Selector::parse("li").ok()?;

    for element in document.select(&selector) {
        let text = element.text().collect::<String>();
        if text.contains("Usage Statistics") {
            // Extract the usage stats text
            let parts: Vec<&str> = text.split("Usage Statistics").collect();
            if parts.len() > 1 {
                let stats = parts[1].trim();
                // Remove "View All Statistics" link text if present
                let stats = stats.replace("View All Statistics", "").trim().to_string();
                return Some(stats);
            }
        }
    }

    None
}

fn extract_more_info(document: &Html) -> Option<String> {
    // Look for the h2 or h6 with "More Info From Us" and get the following div.text content
    let heading_selector = Selector::parse("h2.h6.text-primary, h6.text-primary").ok()?;

    for heading in document.select(&heading_selector) {
        let heading_text = heading.text().collect::<String>();
        if heading_text.contains("More Info From Us") {
            // Get the next sibling div with class "text"
            if let Some(parent) = heading.parent() {
                let div_selector = Selector::parse("div.text").ok()?;
                if let Some(div) = parent.children().find_map(|child| {
                    child.value().as_element()?;
                    let element = scraper::ElementRef::wrap(child)?;
                    if div_selector.matches(&element) {
                        Some(element)
                    } else {
                        None
                    }
                }) {
                    // Extract text but preserve data-original timestamps from timezone-converter spans
                    let mut result = String::new();
                    extract_text_with_timestamps(&div, &mut result);
                    return Some(result.trim().to_string());
                }
            }

            // Alternative: try to find the next div.text element in the document
            let mut found_heading = false;
            let all_selector = Selector::parse("*").ok()?;
            for element in document.select(&all_selector) {
                if found_heading {
                    if element.value().name() == "div" {
                        if let Some(class) = element.value().attr("class") {
                            if class.contains("text") {
                                let mut result = String::new();
                                extract_text_with_timestamps(&element, &mut result);
                                return Some(result.trim().to_string());
                            }
                        }
                    }
                }

                if element.value().name() == "h2" || element.value().name() == "h6" {
                    let text = element.text().collect::<String>();
                    if text.contains("More Info From Us") {
                        found_heading = true;
                    }
                }
            }
        }
    }

    None
}

fn extract_text_with_timestamps(element: &scraper::ElementRef, result: &mut String) {
    use scraper::node::Node;

    for child in element.children() {
        match child.value() {
            Node::Text(text) => {
                // Decode HTML entities in text
                result.push_str(&decode_html_entities(text));
            }
            Node::Element(_) => {
                if let Some(child_element) = scraper::ElementRef::wrap(child) {
                    // Two known timezone-converter shapes:
                    //   legacy: <span class="timezone-converter" data-original="2025-12-04T15:00:00Z">
                    //   new:    <time class="timezone-converter" datetime="2026-05-21T16:00:00Z">21 May, 16:00 UTC</time>
                    // Accept either tag and prefer either attribute so the ISO timestamp
                    // ends up in `more_info` instead of the year-less rendered text.
                    if let Some(class) = child_element.value().attr("class") {
                        if class.contains("timezone-converter") {
                            if let Some(iso) = child_element
                                .value()
                                .attr("data-original")
                                .or_else(|| child_element.value().attr("datetime"))
                            {
                                result.push_str(&decode_html_entities(iso));
                                continue;
                            }
                        }
                    }
                    // Recursively process child elements
                    extract_text_with_timestamps(&child_element, result);
                }
            }
            _ => {}
        }
    }
}

/// Decode HTML entities like &#8211; → – and &amp; → &
fn decode_html_entities(text: &str) -> String {
    let mut result = text.to_string();

    // Decode numeric HTML entities (&#NNNN;)
    // Match decimal entities like &#8211;
    if let Ok(decimal_re) = Regex::new(r"&#(\d+);") {
        let temp = result.clone();
        let mut last_end = 0;
        let mut new_result = String::new();

        for caps in decimal_re.captures_iter(&temp) {
            if let (Some(full_match), Some(num_str)) = (caps.get(0), caps.get(1)) {
                // Add text before this match
                new_result.push_str(&temp[last_end..full_match.start()]);

                // Try to decode the entity
                if let Ok(code_point) = num_str.as_str().parse::<u32>() {
                    if let Some(c) = char::from_u32(code_point) {
                        new_result.push(c);
                    } else {
                        new_result.push_str(full_match.as_str());
                    }
                } else {
                    new_result.push_str(full_match.as_str());
                }

                last_end = full_match.end();
            }
        }
        new_result.push_str(&temp[last_end..]);
        result = new_result;
    }

    // Match hex entities like &#x2013;
    if let Ok(hex_re) = Regex::new(r"&#x([0-9a-fA-F]+);") {
        let temp = result.clone();
        let mut last_end = 0;
        let mut new_result = String::new();

        for caps in hex_re.captures_iter(&temp) {
            if let (Some(full_match), Some(hex_str)) = (caps.get(0), caps.get(1)) {
                // Add text before this match
                new_result.push_str(&temp[last_end..full_match.start()]);

                // Try to decode the entity
                if let Ok(code_point) = u32::from_str_radix(hex_str.as_str(), 16) {
                    if let Some(c) = char::from_u32(code_point) {
                        new_result.push(c);
                    } else {
                        new_result.push_str(full_match.as_str());
                    }
                } else {
                    new_result.push_str(full_match.as_str());
                }

                last_end = full_match.end();
            }
        }
        new_result.push_str(&temp[last_end..]);
        result = new_result;
    }

    // Decode common named HTML entities
    result = result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
        .replace("&ndash;", "\u{2013}") // en-dash
        .replace("&mdash;", "\u{2014}") // em-dash
        .replace("&lsquo;", "\u{2018}") // left single quote
        .replace("&rsquo;", "\u{2019}") // right single quote
        .replace("&ldquo;", "\u{201C}") // left double quote
        .replace("&rdquo;", "\u{201D}") // right double quote
        .replace("&bull;", "\u{2022}") // bullet
        .replace("&hellip;", "\u{2026}") // ellipsis
        .replace("&copy;", "\u{00A9}") // copyright
        .replace("&reg;", "\u{00AE}") // registered
        .replace("&trade;", "\u{2122}"); // trademark

    result
}
