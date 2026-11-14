//! Badge drops for rules: the sighting a rule reads, and the pure step
//! machines around it. No network and no clock here: every time is passed in.
//!
//! - [`CatchUp`]: which badge feed frames are the start-up catch-up (never
//!   posted), per source.
//! - [`ServerClock`]: the feed's clock from HTTP `Date` headers, so every
//!   device judges a drop's age and its phase at the same moment.
//! - [`screen`]: a stale frame, or a badge the gallery already had, posts
//!   nothing.
//! - [`presence`] and [`RecentArrivals`]: whether a badge was in the gallery
//!   before the feed brought it.
//! - [`GalleryAge`]: the gallery is refreshed before a batch when it is more
//!   than 6 hours old.
//! - [`Batcher`]: drops collected into one message per batch.
//! - [`LinkWait`]: a badge link is posted once it answers, or the rule's
//!   fallback after 10 minutes.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::claims::{self, Element, KeyMode};
use crate::models::automations::{BadgeLeg, BadgeLink, LinkFallback, LinkPreset};
use crate::services::badge_polling_service::BadgeNotification;
use crate::services::slash_dispatch::expand_with;

/// Characters of a badge's description `{badge.about}` carries.
pub const ABOUT_CHARS: usize = 120;
/// A frame whose drop is older than this (at server time) is stale; a `drop`
/// push needs no age check.
pub const STALE_MS: i64 = 15 * 60_000;
/// A drop older than this (since it arrived) is never posted.
pub const MAX_POST_AGE_MS: i64 = 25 * 60_000;
/// A gallery read younger than this cannot tell old badges from new ones.
pub const TRUST_FEED_MS: i64 = 30 * 60_000;
/// A badge a gallery read added within this time is new, not old.
pub const RECENT_MS: i64 = 30 * 60_000;
/// How long a recorded arrival is kept.
pub const ARRIVALS_KEEP_MS: i64 = 2 * 3_600_000;
/// The oldest gallery a batch is judged against without a refresh.
pub const GALLERY_MAX_AGE_MS: i64 = 6 * 3_600_000;
/// After a failed gallery refresh, the next try.
pub const GALLERY_RETRY_MS: i64 = 5 * 60_000;
/// A batch closes after this long with no new drop...
pub const QUIET_MS: i64 = 15_000;
/// ...or this long after it opened.
pub const BATCH_CAP_MS: i64 = 60_000;
/// The most drops one batch holds.
pub const MAX_BATCH_DROPS: usize = 55;
/// The most bytes a batch's drop elements take in its claim body.
pub const MAX_BATCH_BYTES: usize = 7_680;
/// The site's claim body cap.
pub const CLAIM_BODY_CAP: usize = 16 * 1024;
/// A link that is not ready is checked again after this...
pub const LINK_RETRY_MS: i64 = 60_000;
/// ...for at most this long; then the rule's fallback is used.
pub const LINK_WAIT_MS: i64 = 10 * 60_000;
/// The pending file's version.
pub const PENDING_VERSION: u32 = 1;

/// Activity words when the gallery could not be refreshed.
pub const GALLERY_TOO_OLD: &str = "Skipped: badge list too old";
pub const NOT_A_LINK: &str = "This isn't a link";
pub const HTTPS_ONLY: &str = "Links must start with https://";
pub const LOCAL_LINK: &str = "Links can't point at an IP address or a private network";

/// Which moment of a badge a sighting is: announced, or its earn window open.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BadgePhase {
    Announced,
    Earnable,
}

impl BadgePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            BadgePhase::Announced => "announced",
            BadgePhase::Earnable => "earnable",
        }
    }
}

/// A badge's details as rules post them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct BadgeInfo {
    pub set_id: String,
    pub version: String,
    pub name: String,
    /// The description, at most 120 characters.
    pub about: String,
    pub image: String,
    /// The relay's date line.
    pub when: String,
    /// How to earn it, in one line.
    pub how: String,
    /// The game or category it belongs to.
    pub game: String,
    /// The event it belongs to.
    pub event: String,
}

impl BadgeInfo {
    /// Read from one feed drop.
    pub fn from_notification(badge: &BadgeNotification) -> BadgeInfo {
        let field = |key: &str| {
            badge
                .enrichment
                .as_ref()
                .and_then(|e| e.get(key))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        BadgeInfo {
            set_id: badge.badge_set_id.clone(),
            version: badge.badge_version.clone(),
            name: badge.badge_name.trim().to_string(),
            about: badge.badge_description.as_deref().unwrap_or("").trim().chars().take(ABOUT_CHARS).collect(),
            image: badge.badge_image_url.clone(),
            when: badge.date_info.as_deref().unwrap_or("").trim().to_string(),
            how: field("action").or_else(|| field("how_to_earn").map(|h| first_sentence(&h))).unwrap_or_default(),
            game: field("category").unwrap_or_default(),
            event: field("event").unwrap_or_default(),
        }
    }
}

fn first_sentence(text: &str) -> String {
    match text.find(". ") {
        Some(end) => text[..=end].to_string(),
        None => text.to_string(),
    }
}

/// A badge variable's value (`{badge}`, `{badge.id}`, ...); None for a name
/// that is not one. `{badge.link}`, `{badge.count}` and `{badges}` are filled
/// by the poster.
pub fn badge_value(badge: &BadgeInfo, name: &str) -> Option<String> {
    Some(match name.to_ascii_lowercase().as_str() {
        "badge" | "badge.name" => badge.name.clone(),
        "badge.id" => badge.set_id.clone(),
        "badge.version" => badge.version.clone(),
        "badge.about" => badge.about.clone(),
        "badge.image" => badge.image.clone(),
        "badge.when" => badge.when.clone(),
        "badge.how" => badge.how.clone(),
        "badge.game" => badge.game.clone(),
        "badge.event" => badge.event.clone(),
        _ => return None,
    })
}

/// Whether the gallery had a badge before the feed brought it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    /// The gallery was read too recently to tell.
    #[default]
    Unknown,
    Yes,
    No,
}

/// One badge, seen once, as rules read it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct BadgeSeen {
    pub badge: BadgeInfo,
    /// The relay's drop id.
    pub feed_id: String,
    pub phase: BadgePhase,
    /// First seen now.
    pub announced: bool,
    /// Its earn window is open now (at server time).
    pub earnable: bool,
    /// The relay's time for the drop (server ms).
    pub ts_ms: i64,
    /// When this device received it (local wall clock ms).
    pub arrived_ms: i64,
    /// It came as a `drop` push (never stale).
    pub push: bool,
    pub in_gallery: Presence,
}

/// A sighting from the feed's classification: `first` when the drop is new,
/// `available` when a known badge's window just opened, `window_open` when the
/// window is open at server time. Some((phase, announced, earnable)) when
/// rules hear of it. A badge first seen already earnable is one sighting with
/// phase earnable, so a Both rule posts once; a window that opens later is a
/// second sighting.
pub fn sighting(first: bool, available: bool, window_open: bool) -> Option<(BadgePhase, bool, bool)> {
    let opened = available || (first && window_open);
    if !(first || opened) {
        return None;
    }
    let phase = if opened { BadgePhase::Earnable } else { BadgePhase::Announced };
    Some((phase, first, opened))
}

/// Whether a rule's legs want a sighting.
pub fn wants(on: BadgeLeg, seen: &BadgeSeen) -> bool {
    match on {
        BadgeLeg::Announced => seen.announced,
        BadgeLeg::Earnable => seen.earnable,
        BadgeLeg::Both => seen.announced || seen.earnable,
    }
}

// ---------------------------------------------------------------------------
// Catch-up and the server's clock
// ---------------------------------------------------------------------------

/// Where a feed frame came from, for rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// The start-up catch-up: never posted.
    CatchUp,
    Live,
}

/// Which frames are the catch-up, per source: the first poll after the feed
/// starts a session (process start, or back from hidden), and the first
/// `history` frame of each socket session. A failed poll counts as landed.
/// Everything else is live.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CatchUp {
    poll_open: bool,
    history_open: bool,
}

impl CatchUp {
    /// Nothing open yet; usable in a `static`.
    pub const fn new() -> Self {
        CatchUp { poll_open: false, history_open: false }
    }

    /// The feed starts a session: its first poll is the catch-up.
    pub fn session_started(&mut self) {
        self.poll_open = true;
    }

    /// A socket session opened: its first `history` frame is the catch-up.
    pub fn socket_opened(&mut self) {
        self.history_open = true;
    }

    /// A poll ended, with drops or failed.
    pub fn poll_landed(&mut self) -> Origin {
        if std::mem::take(&mut self.poll_open) {
            Origin::CatchUp
        } else {
            Origin::Live
        }
    }

    /// A socket frame arrived; `history` for a `history` frame.
    pub fn frame(&mut self, history: bool) -> Origin {
        if history && std::mem::take(&mut self.history_open) {
            Origin::CatchUp
        } else {
            Origin::Live
        }
    }
}

/// The badge feed's clock, from the `Date` header of its answers (the latest
/// wins), so a device whose clock is off still judges ages and windows the way
/// every other device does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServerClock {
    offset_ms: i64,
}

impl ServerClock {
    /// No offset known yet; usable in a `static`.
    pub const fn new() -> Self {
        ServerClock { offset_ms: 0 }
    }

    /// Note an answer's `Date` header, received at `local_ms`. False when the
    /// header cannot be read (the offset is kept).
    pub fn note_date(&mut self, date: &str, local_ms: i64) -> bool {
        match parse_http_date(date) {
            Some(server_ms) => {
                self.offset_ms = server_ms - local_ms;
                true
            }
            None => false,
        }
    }

    pub fn offset_ms(&self) -> i64 {
        self.offset_ms
    }

    pub fn server_now(&self, local_ms: i64) -> i64 {
        local_ms + self.offset_ms
    }
}

/// An HTTP date (`Wed, 12 Nov 2026 22:00:00 GMT`) as epoch ms.
pub fn parse_http_date(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc2822(text.trim()).ok().map(|d| d.timestamp_millis())
}

/// What happens to a sighting before any rule sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Keep,
    /// The frame is older than 15 minutes at server time.
    Stale,
    /// A first sighting of a badge the gallery already had.
    InGallery,
}

pub fn screen(seen: &BadgeSeen, offset_ms: i64) -> Screen {
    if !seen.push && seen.arrived_ms + offset_ms - seen.ts_ms > STALE_MS {
        return Screen::Stale;
    }
    if seen.announced && seen.in_gallery == Presence::Yes {
        return Screen::InGallery;
    }
    Screen::Keep
}

// ---------------------------------------------------------------------------
// The gallery
// ---------------------------------------------------------------------------

/// Whether the gallery had a badge before this ingest: Unknown while the
/// gallery read is under 30 minutes old (it may already hold the new badge),
/// else present and not a recent arrival.
pub fn presence(cached_at_ms: i64, now_ms: i64, present: bool, recent: bool) -> Presence {
    if now_ms - cached_at_ms < TRUST_FEED_MS {
        Presence::Unknown
    } else if present && !recent {
        Presence::Yes
    } else {
        Presence::No
    }
}

/// One (set, version) a gallery read added.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Arrival {
    pub set_id: String,
    pub version: String,
    pub at_ms: i64,
}

/// The badges gallery reads added recently, kept 2 hours.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct RecentArrivals {
    pub entries: Vec<Arrival>,
}

impl RecentArrivals {
    pub fn record(&mut self, pairs: Vec<(String, String)>, now_ms: i64) {
        for (set_id, version) in pairs {
            self.entries.retain(|a| !(a.set_id == set_id && a.version == version));
            self.entries.push(Arrival { set_id, version, at_ms: now_ms });
        }
        self.prune(now_ms);
    }

    /// Added within the last 30 minutes.
    pub fn is_recent(&self, set_id: &str, version: &str, now_ms: i64) -> bool {
        self.entries.iter().any(|a| a.set_id == set_id && a.version == version && now_ms - a.at_ms < RECENT_MS)
    }

    pub fn prune(&mut self, now_ms: i64) {
        self.entries.retain(|a| now_ms - a.at_ms < ARRIVALS_KEEP_MS);
    }
}

/// The (set, version) pairs a gallery read added: none when there was no
/// previous read to compare with.
pub fn new_pairs(previous: Option<&HashSet<(String, String)>>, current: &HashSet<(String, String)>) -> Vec<(String, String)> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let mut added: Vec<(String, String)> = current.difference(previous).cloned().collect();
    added.sort();
    added
}

/// What to do about the gallery's age before a batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GalleryStep {
    Fresh,
    /// Refresh it now, then go on.
    Refresh,
    /// A refresh failed less than 5 minutes ago: skip with [`GALLERY_TOO_OLD`].
    TooOld,
}

/// The gallery refresh's retry state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GalleryAge {
    retry_at_ms: Option<i64>,
}

impl GalleryAge {
    pub fn check(&self, cached_at_ms: Option<i64>, now_ms: i64) -> GalleryStep {
        if cached_at_ms.is_some_and(|at| now_ms - at <= GALLERY_MAX_AGE_MS) {
            return GalleryStep::Fresh;
        }
        match self.retry_at_ms {
            Some(at) if now_ms < at => GalleryStep::TooOld,
            _ => GalleryStep::Refresh,
        }
    }

    pub fn refreshed(&mut self) {
        self.retry_at_ms = None;
    }

    pub fn failed(&mut self, now_ms: i64) {
        self.retry_at_ms = Some(now_ms + GALLERY_RETRY_MS);
    }
}

// ---------------------------------------------------------------------------
// Batches
// ---------------------------------------------------------------------------

/// Drops posted together as one message.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Batch {
    pub drops: Vec<BadgeSeen>,
    pub opened_ms: i64,
    pub last_ms: i64,
    /// The drop elements' bytes in the claim body.
    pub bytes: usize,
}

/// Collects drops until 15 s pass with no new one, 60 s pass since the first,
/// it holds 55, or the next drop would take its claim elements past 7.5 KB
/// (that drop opens the next batch).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Batcher {
    pub open: Option<Batch>,
}

/// The bytes one drop's event element takes in a claim body, for a room id
/// of `room_len` digits.
pub fn drop_bytes(seen: &BadgeSeen, room_len: usize) -> usize {
    let element = Element::Event {
        key: claims::badge_key(&seen.feed_id, seen.phase),
        family: format!("badge:|post:{}", "0".repeat(room_len.max(1))),
        mode: KeyMode::Exact,
        window_s: None,
        sends: 0,
        to_person: false,
        settle_ms: claims::MAX_SETTLE_MS,
    };
    serde_json::to_string(&element).map_or(0, |s| s.len()) + 1
}

impl Batcher {
    /// Add a drop at `now_ms`; `room_len` is the longest owner room id among
    /// the badge rules. Returns a batch this push closed. The same drop and
    /// phase twice in one batch counts once.
    pub fn push(&mut self, seen: BadgeSeen, now_ms: i64, room_len: usize) -> Option<Batch> {
        if let Some(open) = &self.open {
            if open.drops.iter().any(|d| d.feed_id == seen.feed_id && d.phase == seen.phase) {
                return None;
            }
        }
        let bytes = drop_bytes(&seen, room_len);
        let full = self
            .open
            .as_ref()
            .is_some_and(|open| open.drops.len() >= MAX_BATCH_DROPS || open.bytes + bytes > MAX_BATCH_BYTES);
        let mut closed = if full { self.open.take() } else { None };
        let batch = self.open.get_or_insert_with(|| Batch { drops: Vec::new(), opened_ms: now_ms, last_ms: now_ms, bytes: 0 });
        batch.drops.push(seen);
        batch.last_ms = now_ms;
        batch.bytes += bytes;
        let reached = batch.drops.len() >= MAX_BATCH_DROPS;
        if closed.is_none() && reached {
            closed = self.open.take();
        }
        closed
    }

    /// When the open batch closes by time.
    pub fn due_ms(&self) -> Option<i64> {
        self.open.as_ref().map(|b| (b.last_ms + QUIET_MS).min(b.opened_ms + BATCH_CAP_MS))
    }

    /// The open batch, when its time is up.
    pub fn tick(&mut self, now_ms: i64) -> Option<Batch> {
        if self.due_ms().is_some_and(|due| now_ms >= due) {
            self.open.take()
        } else {
            None
        }
    }
}

/// Young enough to post (25 minutes since it arrived).
pub fn post_age_ok(seen: &BadgeSeen, now_ms: i64) -> bool {
    now_ms - seen.arrived_ms <= MAX_POST_AGE_MS
}

/// The drops of a batch one rule posts: the legs it wants, young enough, and
/// not posted by this rule on this device before (`posted`).
pub fn drops_for<'a>(
    batch: &'a Batch,
    on: BadgeLeg,
    posted: impl Fn(&str, BadgePhase) -> bool,
    now_ms: i64,
) -> Vec<&'a BadgeSeen> {
    batch
        .drops
        .iter()
        .filter(|d| wants(on, d) && post_age_ok(d, now_ms) && !posted(&d.feed_id, d.phase))
        .collect()
}

/// What a rule posts for the drops it won: its Send lines for one, its batch
/// text for more.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostShape<'a> {
    One(&'a BadgeSeen),
    Many { count: usize, names: String },
}

pub fn post_shape<'a>(drops: &[&'a BadgeSeen]) -> Option<PostShape<'a>> {
    match drops {
        [] => None,
        [one] => Some(PostShape::One(*one)),
        many => Some(PostShape::Many { count: many.len(), names: badge_names(many) }),
    }
}

/// `{badges}`: "A", "A and B", "A, B and C".
pub fn badge_names(drops: &[&BadgeSeen]) -> String {
    let names: Vec<&str> = drops
        .iter()
        .map(|d| match d.badge.name.trim() {
            "" => d.feed_id.as_str(),
            name => name,
        })
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The batcher and the batches waiting to post, kept in
/// `automation_pending.json` with the recent arrivals.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(default)]
pub struct Pending {
    pub version: u32,
    pub batcher: Batcher,
    /// Closed batches not yet posted (a link loading, a claim pending).
    pub waiting: Vec<Batch>,
    pub arrivals: RecentArrivals,
}

impl Pending {
    pub fn from_json(text: &str) -> Result<Pending, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(&Pending { version: PENDING_VERSION, ..self.clone() }).unwrap_or_default()
    }

    /// Drop what can no longer post, and old arrivals.
    pub fn prune(&mut self, now_ms: i64) {
        for batch in &mut self.waiting {
            batch.drops.retain(|d| post_age_ok(d, now_ms));
        }
        self.waiting.retain(|b| !b.drops.is_empty());
        let empty = match &mut self.batcher.open {
            Some(open) => {
                open.drops.retain(|d| post_age_ok(d, now_ms));
                open.drops.is_empty()
            }
            None => false,
        };
        if empty {
            self.batcher.open = None;
        }
        self.arrivals.prune(now_ms);
    }
}

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------

fn encode(text: &str) -> String {
    urlencoding::encode(text).into_owned()
}

/// The StreamNook badge page; the version is named only when it is not 1.
pub fn streamnook_link(badge: &BadgeInfo) -> String {
    let mut url = format!("https://streamnook.app/badge/{}", encode(&badge.set_id));
    let version = badge.version.trim();
    if !version.is_empty() && version != "1" {
        url.push('/');
        url.push_str(&encode(version));
    }
    url
}

pub fn streamdatabase_link(badge: &BadgeInfo) -> String {
    format!(
        "https://www.streamdatabase.com/twitch/global-badges/{}/{}",
        encode(&badge.set_id),
        encode(&badge.version)
    )
}

/// A custom link with the badge's variables filled in, URL-encoded.
pub fn custom_link(template: &str, badge: &BadgeInfo) -> String {
    expand_with(template.trim(), &[], |name| badge_value(badge, name).map(|v| encode(&v))).text
}

/// The link a rule's preset names; None for no link.
pub fn link_for(link: &BadgeLink, badge: &BadgeInfo) -> Option<String> {
    match link.preset {
        LinkPreset::StreamNook => Some(streamnook_link(badge)),
        LinkPreset::StreamDatabase => Some(streamdatabase_link(badge)),
        LinkPreset::Custom => Some(custom_link(&link.custom, badge)).filter(|u| !u.is_empty()),
        LinkPreset::None => None,
    }
}

/// The link posted when the rule's link is still not ready after 10 minutes.
pub fn fallback_link(link: &BadgeLink, badge: &BadgeInfo) -> Option<String> {
    match link.if_not_ready {
        LinkFallback::StreamNook => Some(streamnook_link(badge)),
        LinkFallback::NoLink => None,
    }
}

/// Whether a link may be checked and posted: https only, a named host with a
/// dot, never an IP address, localhost or a local-network name.
pub fn check_url(text: &str) -> Result<(), &'static str> {
    let url = url::Url::parse(text.trim()).map_err(|_| NOT_A_LINK)?;
    if url.scheme() != "https" {
        return Err(HTTPS_ONLY);
    }
    match url.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            let local = host == "localhost"
                || host.ends_with(".localhost")
                || host.ends_with(".local")
                || host.ends_with(".internal")
                || host.ends_with(".lan")
                || host.ends_with(".home.arpa")
                || !host.contains('.');
            if local {
                Err(LOCAL_LINK)
            } else {
                Ok(())
            }
        }
        Some(_) => Err(LOCAL_LINK),
        None => Err(NOT_A_LINK),
    }
}

/// One answer to a link check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// The answer's status (redirects are not followed: a 3xx is not ready).
    Status(u16),
    /// No answer (a timeout or a network error).
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkStep {
    Ready,
    RetryAt(i64),
    /// 10 minutes passed: post with the rule's fallback.
    GiveUp,
}

/// Waiting for a badge link to answer 200.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkWait {
    pub url: String,
    pub started_ms: i64,
}

impl LinkWait {
    pub fn start(url: String, now_ms: i64) -> Result<LinkWait, &'static str> {
        check_url(&url)?;
        Ok(LinkWait { url, started_ms: now_ms })
    }

    pub fn on_probe(&self, probe: Probe, now_ms: i64) -> LinkStep {
        if probe == Probe::Status(200) {
            return LinkStep::Ready;
        }
        let next = now_ms + LINK_RETRY_MS;
        if next - self.started_ms > LINK_WAIT_MS {
            LinkStep::GiveUp
        } else {
            LinkStep::RetryAt(next)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::{AutomationPrefs, Trigger};
    use crate::services::account_automations::claims::{class_elements, classes_for, event_family, trigger_family, CallInput};
    use crate::services::account_automations::compile::fixtures::{rule, send};
    use crate::services::account_automations::counts::Counts;
    use crate::services::account_automations::facts::from_badge;
    use crate::services::badge_polling_service::BadgeNotificationStatus;

    const MIN: i64 = 60_000;

    fn posted_by(counts: &Counts) -> impl Fn(&str, BadgePhase) -> bool + '_ {
        move |feed: &str, phase: BadgePhase| counts.has_posted("r", feed, phase)
    }

    fn drop_of(feed_id: &str, phase: BadgePhase, arrived_ms: i64) -> BadgeSeen {
        BadgeSeen {
            badge: BadgeInfo { set_id: feed_id.into(), version: "1".into(), name: format!("Badge {feed_id}"), ..BadgeInfo::default() },
            feed_id: feed_id.into(),
            phase,
            announced: phase == BadgePhase::Announced,
            earnable: phase == BadgePhase::Earnable,
            ts_ms: arrived_ms,
            arrived_ms,
            push: true,
            in_gallery: Presence::No,
        }
    }

    #[test]
    fn catch_up_covers_the_first_poll_and_first_history_in_either_order() {
        for poll_first in [true, false] {
            let mut c = CatchUp::default();
            c.session_started();
            c.socket_opened();
            if poll_first {
                assert_eq!(c.poll_landed(), Origin::CatchUp);
                assert_eq!(c.frame(true), Origin::CatchUp);
            } else {
                assert_eq!(c.frame(true), Origin::CatchUp);
                assert_eq!(c.poll_landed(), Origin::CatchUp);
            }
            // Everything after is live.
            assert_eq!(c.poll_landed(), Origin::Live);
            assert_eq!(c.frame(true), Origin::Live);
            assert_eq!(c.frame(false), Origin::Live);
        }
    }

    #[test]
    fn a_failed_poll_closes_its_catch_up_and_a_kept_socket_stays_live() {
        // The const constructors (for statics) equal the defaults.
        assert_eq!(CatchUp::new(), CatchUp::default());
        assert_eq!(ServerClock::new(), ServerClock::default());
        let mut c = CatchUp::new();
        c.session_started();
        // The poll failed: it still counts as landed.
        assert_eq!(c.poll_landed(), Origin::CatchUp);
        assert_eq!(c.poll_landed(), Origin::Live);
        // Back from a short background that kept the socket: a new session
        // makes the next poll the catch-up, but the socket's next drop push is live.
        c.socket_opened();
        assert_eq!(c.frame(true), Origin::CatchUp);
        c.session_started();
        assert_eq!(c.frame(false), Origin::Live);
        assert_eq!(c.poll_landed(), Origin::CatchUp);
        // A drop before the first history frame is live, and history still catches up.
        let mut c = CatchUp::default();
        c.socket_opened();
        assert_eq!(c.frame(false), Origin::Live);
        assert_eq!(c.frame(true), Origin::CatchUp);
    }

    #[test]
    fn age_is_judged_at_server_time_whatever_the_device_clock() {
        let t = 1_800_000_000_000;
        let mut clock = ServerClock::default();
        assert!(clock.note_date("Fri, 15 Jan 2027 08:00:00 GMT", 1_000));
        assert_eq!(clock.server_now(1_000), parse_http_date("Fri, 15 Jan 2027 08:00:00 GMT").unwrap());
        assert!(!clock.note_date("not a date", 5));
        for skew in [-20 * MIN, 0, 20 * MIN] {
            // A frame (not a push) that reached the device 1 minute after the relay stamped it.
            let mut seen = drop_of("x-v1", BadgePhase::Announced, t + MIN + skew);
            seen.ts_ms = t;
            seen.push = false;
            assert_eq!(screen(&seen, -skew), Screen::Keep, "skew {skew}");
            // The same frame 16 minutes late is stale on every device.
            seen.arrived_ms = t + 16 * MIN + skew;
            assert_eq!(screen(&seen, -skew), Screen::Stale, "skew {skew}");
            // A drop push is never stale.
            seen.push = true;
            assert_eq!(screen(&seen, -skew), Screen::Keep);
        }
    }

    #[test]
    fn a_badge_the_gallery_already_had_posts_nothing() {
        let now = 10 * 3_600_000;
        let mut seen = drop_of("x-v1", BadgePhase::Announced, now);
        seen.in_gallery = presence(now - 2 * 3_600_000, now, true, false);
        assert_eq!(seen.in_gallery, Presence::Yes);
        assert_eq!(screen(&seen, 0), Screen::InGallery);
        // A window opening on a badge already in the gallery still posts.
        let mut opening = drop_of("x-v1", BadgePhase::Earnable, now);
        opening.in_gallery = Presence::Yes;
        assert_eq!(screen(&opening, 0), Screen::Keep);
        // A gallery read under 30 minutes old may already hold the new badge: trust the feed.
        assert_eq!(presence(now - 10 * MIN, now, true, false), Presence::Unknown);
        // A badge a gallery read added recently is new.
        assert_eq!(presence(now - 2 * 3_600_000, now, true, true), Presence::No);
        assert_eq!(presence(now - 2 * 3_600_000, now, false, false), Presence::No);
    }

    #[test]
    fn arrivals_are_recorded_against_the_previous_read_only() {
        let pair = |s: &str| (s.to_string(), "1".to_string());
        let current: HashSet<(String, String)> = [pair("a"), pair("b")].into_iter().collect();
        assert!(new_pairs(None, &current).is_empty());
        let previous: HashSet<(String, String)> = [pair("a")].into_iter().collect();
        assert_eq!(new_pairs(Some(&previous), &current), vec![pair("b")]);
        let mut arrivals = RecentArrivals::default();
        arrivals.record(vec![pair("b")], 0);
        assert!(arrivals.is_recent("b", "1", 29 * MIN));
        assert!(!arrivals.is_recent("b", "1", 31 * MIN));
        assert!(!arrivals.is_recent("a", "1", 0));
        arrivals.prune(2 * 3_600_000);
        assert!(arrivals.entries.is_empty());
    }

    #[test]
    fn the_gallery_is_refreshed_when_old_and_retried_after_a_failure() {
        let mut age = GalleryAge::default();
        let now = 100 * 3_600_000;
        assert_eq!(age.check(Some(now - 3_600_000), now), GalleryStep::Fresh);
        assert_eq!(age.check(Some(now - 7 * 3_600_000), now), GalleryStep::Refresh);
        assert_eq!(age.check(None, now), GalleryStep::Refresh);
        age.failed(now);
        assert_eq!(age.check(None, now + MIN), GalleryStep::TooOld);
        assert_eq!(age.check(None, now + 5 * MIN), GalleryStep::Refresh);
        age.refreshed();
        assert_eq!(age.check(None, now + MIN), GalleryStep::Refresh);
    }

    #[test]
    fn legs_and_phases() {
        // First seen already earnable: one sighting, phase earnable, every leg fires once.
        let (phase, announced, earnable) = sighting(true, false, true).unwrap();
        assert_eq!((phase, announced, earnable), (BadgePhase::Earnable, true, true));
        let mut seen = drop_of("x-v1", phase, 0);
        seen.announced = announced;
        seen.earnable = earnable;
        for leg in [BadgeLeg::Announced, BadgeLeg::Earnable, BadgeLeg::Both] {
            assert!(wants(leg, &seen), "{leg:?}");
        }
        // Announced first, the window opening later: two sightings, two keys.
        assert_eq!(sighting(true, false, false), Some((BadgePhase::Announced, true, false)));
        assert_eq!(sighting(false, true, false), Some((BadgePhase::Earnable, false, true)));
        assert_eq!(sighting(false, false, true), None);
        let first = drop_of("x-v1", BadgePhase::Announced, 0);
        let later = drop_of("x-v1", BadgePhase::Earnable, 0);
        assert!(wants(BadgeLeg::Announced, &first) && !wants(BadgeLeg::Announced, &later));
        assert!(!wants(BadgeLeg::Earnable, &first) && wants(BadgeLeg::Earnable, &later));
        assert_ne!(from_badge(&first).event_key, from_badge(&later).event_key);
    }

    #[test]
    fn one_window_moment_is_one_phase_on_every_device() {
        // The window opens at server time `open`. Device A's clock runs 2 minutes
        // fast, B's is right; both see the drop at the same server moment.
        let open = 1_800_000_000_000;
        let a = ServerClock { offset_ms: -2 * MIN };
        let b = ServerClock { offset_ms: 0 };
        for moment in [open - 10_000, open + 10_000] {
            let at_a = a.server_now(moment + 2 * MIN);
            let at_b = b.server_now(moment);
            assert_eq!(at_a, at_b);
            assert_eq!(sighting(true, false, at_a >= open), sighting(true, false, at_b >= open));
        }
    }

    #[test]
    fn a_batch_closes_on_quiet_on_its_cap_and_a_straggler_opens_the_next() {
        let mut b = Batcher::default();
        for i in 0..3 {
            assert!(b.push(drop_of(&format!("d{i}-v1"), BadgePhase::Announced, i * 1_000), i * 1_000, 3).is_none());
        }
        assert_eq!(b.due_ms(), Some(2_000 + QUIET_MS));
        assert!(b.tick(16_999).is_none());
        let first = b.tick(17_000).unwrap();
        assert_eq!(first.drops.len(), 3);
        assert_eq!(post_shape(&first.drops.iter().collect::<Vec<_>>()).map(|s| matches!(s, PostShape::Many { count: 3, .. })), Some(true));
        // A straggler 20 s after the batch opens a batch of its own.
        assert!(b.push(drop_of("late-v1", BadgePhase::Announced, 22_000), 22_000, 3).is_none());
        assert_eq!(b.open.as_ref().unwrap().opened_ms, 22_000);
        assert_eq!(b.tick(37_000).unwrap().drops.len(), 1);
        // A drop every 10 s never goes quiet: the batch closes 60 s after it opened.
        let mut b = Batcher::default();
        for i in 0..7 {
            let at = i * 10_000;
            b.push(drop_of(&format!("s{i}-v1"), BadgePhase::Announced, at), at, 3);
        }
        assert_eq!(b.due_ms(), Some(BATCH_CAP_MS));
        assert_eq!(b.tick(BATCH_CAP_MS).unwrap().drops.len(), 7);
        // The same drop and phase twice counts once.
        let mut b = Batcher::default();
        b.push(drop_of("x-v1", BadgePhase::Announced, 0), 0, 3);
        b.push(drop_of("x-v1", BadgePhase::Announced, 1), 1, 3);
        b.push(drop_of("x-v1", BadgePhase::Earnable, 2), 2, 3);
        assert_eq!(b.open.as_ref().unwrap().drops.len(), 2);
    }

    #[test]
    fn fifty_six_drops_close_as_two_batches_the_first_at_55() {
        let mut b = Batcher::default();
        let mut closed = Vec::new();
        for i in 0..56 {
            if let Some(batch) = b.push(drop_of(&i.to_string(), BadgePhase::Announced, 0), 0, 3) {
                closed.push(batch);
            }
        }
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].drops.len(), 55);
        assert!(closed[0].bytes <= MAX_BATCH_BYTES);
        assert_eq!(b.open.as_ref().unwrap().drops.len(), 1);
    }

    #[test]
    fn long_feed_ids_close_a_batch_before_its_claim_body_passes_the_limit() {
        let room = "1234567890";
        let mut b = Batcher::default();
        let mut closed = Vec::new();
        for i in 0..80 {
            let feed = format!("{i:02}-twitchcon-rotterdam-2026-community-v1");
            assert_eq!(feed.len(), 40);
            if let Some(batch) = b.push(drop_of(&feed, BadgePhase::Announced, 0), 0, room.len()) {
                closed.push(batch);
            }
        }
        assert!(!closed.is_empty());
        let r = {
            let mut r = rule(
                "r-badge",
                Trigger::BadgeDrop { on: BadgeLeg::Both, link: BadgeLink::default(), batch_text: "{badges}".into() },
                vec![send("{badge}")],
            );
            r.owner.id = room.into();
            r.send_as.id = room.into();
            r
        };
        let prefs = AutomationPrefs::default();
        for batch in &closed {
            assert!(batch.drops.len() < MAX_BATCH_DROPS);
            assert!(batch.bytes <= MAX_BATCH_BYTES);
            // The whole call, as the claim client sends it, stays under the site's cap.
            let facts = from_badge(&batch.drops[0]);
            let (plans, _) = classes_for(&r, room);
            let family = event_family(&trigger_family(&r.when, &facts).unwrap(), &plans[0].class);
            let keys: Vec<String> = batch.drops.iter().map(|d| claims::badge_key(&d.feed_id, d.phase)).collect();
            let elements = class_elements(&CallInput {
                rule: &r,
                prefs: &prefs,
                plan: &plans[0],
                family: &family,
                event_keys: &keys,
                event_room: room,
                person: None,
            })
            .unwrap();
            let body = serde_json::json!({
                "action": "claim",
                "call_id": "0123456789abcdef",
                "session": "0123456789abcdef",
                "device_hash": "0123456789abcdef",
                "name": "A forty character device name for tests",
                "reclaim": true,
                "elements": elements,
            });
            let size = serde_json::to_string(&body).unwrap().len();
            assert!(size < CLAIM_BODY_CAP, "{size}");
            assert!(elements.len() <= 60);
        }
    }

    #[test]
    fn a_rule_posts_the_legs_it_wants_once_per_device() {
        let now = 10 * MIN;
        let batch = Batch {
            drops: vec![
                drop_of("a-v1", BadgePhase::Announced, now),
                drop_of("b-v1", BadgePhase::Earnable, now),
                drop_of("old-v1", BadgePhase::Announced, now - 26 * MIN),
            ],
            opened_ms: now,
            last_ms: now,
            bytes: 0,
        };
        let mut counts = Counts::default();
        let ids = |drops: Vec<&BadgeSeen>| drops.iter().map(|d| d.feed_id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(drops_for(&batch, BadgeLeg::Both, posted_by(&counts), now)), vec!["a-v1", "b-v1"]);
        assert_eq!(ids(drops_for(&batch, BadgeLeg::Announced, posted_by(&counts), now)), vec!["a-v1"]);
        assert_eq!(ids(drops_for(&batch, BadgeLeg::Earnable, posted_by(&counts), now)), vec!["b-v1"]);
        // Once posted, a re-ingest of the same drop posts nothing.
        counts.mark_posted("r", &[("a-v1".to_string(), BadgePhase::Announced)], now);
        assert_eq!(ids(drops_for(&batch, BadgeLeg::Both, posted_by(&counts), now)), vec!["b-v1"]);
        // One drop posts the Send lines; more post the batch text.
        let one = drops_for(&batch, BadgeLeg::Earnable, posted_by(&counts), now);
        assert_eq!(post_shape(&one), Some(PostShape::One(&batch.drops[1])));
        let all: Vec<&BadgeSeen> = batch.drops.iter().collect();
        assert_eq!(badge_names(&all), "Badge a-v1, Badge b-v1 and Badge old-v1");
        assert_eq!(post_shape(&[]), None);
    }

    #[test]
    fn badge_details_come_from_the_feed_drop() {
        let drop = BadgeNotification {
            badge_name: " Rotterdam ".into(),
            badge_set_id: "twitchcon-rotterdam".into(),
            badge_version: "2".into(),
            badge_image_url: "https://static-cdn.jtvnw.net/badges/v1/x/3".into(),
            badge_description: Some("x".repeat(200)),
            status: BadgeNotificationStatus::New,
            date_info: Some("Dec 1-12".into()),
            enrichment: Some(serde_json::json!({
                "how_to_earn": "Watch 3 hours of any eligible channel. Prime subs do not count.",
                "category": "Pok\u{e9}mon GO",
                "event": "TwitchCon"
            })),
            window: None,
            first_seen_ms: None,
            first_seen_p: None,
        };
        let info = BadgeInfo::from_notification(&drop);
        assert_eq!(info.name, "Rotterdam");
        assert_eq!(info.about.chars().count(), ABOUT_CHARS);
        assert_eq!(info.how, "Watch 3 hours of any eligible channel.");
        assert_eq!(info.game, "Pok\u{e9}mon GO");
        assert_eq!(info.event, "TwitchCon");
        assert_eq!(info.when, "Dec 1-12");
        assert_eq!(badge_value(&info, "badge"), Some("Rotterdam".to_string()));
        assert_eq!(badge_value(&info, "Badge.Version"), Some("2".to_string()));
        assert_eq!(badge_value(&info, "badge.link"), None);
        // A short action wins over the long description.
        let mut short = drop.clone();
        short.enrichment = Some(serde_json::json!({"action": "Subscribe to a co-streamer", "how_to_earn": "Long."}));
        assert_eq!(BadgeInfo::from_notification(&short).how, "Subscribe to a co-streamer");
    }

    #[test]
    fn links_per_preset() {
        let one = BadgeInfo { set_id: "twitchcon rotterdam".into(), version: "1".into(), name: "R&D".into(), ..BadgeInfo::default() };
        let two = BadgeInfo { version: "2".into(), ..one.clone() };
        assert_eq!(streamnook_link(&one), "https://streamnook.app/badge/twitchcon%20rotterdam");
        assert_eq!(streamnook_link(&two), "https://streamnook.app/badge/twitchcon%20rotterdam/2");
        assert_eq!(
            streamdatabase_link(&one),
            "https://www.streamdatabase.com/twitch/global-badges/twitchcon%20rotterdam/1"
        );
        let custom = BadgeLink {
            preset: LinkPreset::Custom,
            custom: "https://example.com/b?id={badge.id}&n={badge}".into(),
            if_not_ready: LinkFallback::NoLink,
        };
        assert_eq!(link_for(&custom, &one).as_deref(), Some("https://example.com/b?id=twitchcon%20rotterdam&n=R%26D"));
        let none = BadgeLink { preset: LinkPreset::None, ..custom.clone() };
        assert_eq!(link_for(&none, &one), None);
        assert_eq!(fallback_link(&custom, &one), None);
        let back = BadgeLink { if_not_ready: LinkFallback::StreamNook, ..custom };
        assert_eq!(fallback_link(&back, &one).as_deref(), Some("https://streamnook.app/badge/twitchcon%20rotterdam"));
    }

    #[test]
    fn link_readiness_waits_retries_and_falls_back() {
        // 404, 404, then 200: posted with the link.
        let wait = LinkWait::start("https://streamnook.app/badge/x".into(), 0).unwrap();
        assert_eq!(wait.on_probe(Probe::Status(404), 0), LinkStep::RetryAt(MIN));
        assert_eq!(wait.on_probe(Probe::Status(404), MIN), LinkStep::RetryAt(2 * MIN));
        assert_eq!(wait.on_probe(Probe::Status(200), 2 * MIN), LinkStep::Ready);
        // A redirect is not ready; neither is no answer.
        assert_eq!(wait.on_probe(Probe::Status(302), 0), LinkStep::RetryAt(MIN));
        assert_eq!(wait.on_probe(Probe::Failed, 0), LinkStep::RetryAt(MIN));
        // Ten minutes of 404: the fallback.
        let mut at = 0;
        let mut steps = 0;
        loop {
            match wait.on_probe(Probe::Status(404), at) {
                LinkStep::RetryAt(next) => at = next,
                LinkStep::GiveUp => break,
                LinkStep::Ready => unreachable!(),
            }
            steps += 1;
            assert!(steps < 20);
        }
        assert_eq!(at, LINK_WAIT_MS);
        // Only https links to named public hosts are ever checked.
        assert_eq!(LinkWait::start("http://streamnook.app/badge/x".into(), 0), Err(HTTPS_ONLY));
        for local in [
            "https://localhost/x",
            "https://printer.local/x",
            "https://intranet/x",
            "https://127.0.0.1/x",
            "https://10.0.0.5/x",
            "https://192.168.1.1/x",
            "https://[::1]/x",
            "https://0x7f.1/x",
        ] {
            assert_eq!(check_url(local), Err(LOCAL_LINK), "{local}");
        }
        assert_eq!(check_url("not a link"), Err(NOT_A_LINK));
        assert_eq!(check_url("https://www.streamdatabase.com/twitch/global-badges/x/1"), Ok(()));
    }

    #[test]
    fn the_pending_store_round_trips_and_forgets_what_can_no_longer_post() {
        let mut pending = Pending::default();
        pending.batcher.push(drop_of("a-v1", BadgePhase::Announced, 0), 0, 3);
        pending.waiting.push(Batch { drops: vec![drop_of("b-v1", BadgePhase::Earnable, 20 * MIN)], opened_ms: 0, last_ms: 0, bytes: 0 });
        pending.arrivals.record(vec![("c".into(), "1".into())], 0);
        let text = pending.to_json();
        let back = Pending::from_json(&text).unwrap();
        assert_eq!(back.version, PENDING_VERSION);
        assert_eq!(back.batcher, pending.batcher);
        assert_eq!(back.waiting, pending.waiting);
        let mut later = back;
        later.prune(30 * MIN);
        assert!(later.batcher.open.is_none());
        assert_eq!(later.waiting.len(), 1);
        later.prune(46 * MIN);
        assert!(later.waiting.is_empty());
        assert!(Pending::from_json("not json").is_err());
        assert_eq!(Pending::from_json("{}").unwrap(), Pending::default());
    }
}
