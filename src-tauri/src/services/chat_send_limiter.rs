//! Twitch's chat send limits, applied before a paced message leaves the app.
//!
//! Twitch ignores an account for an hour when it sends too fast, so anything
//! that sends more than one message on its own (a repeated reminder, a plugin's
//! send, a scripted command) asks this module for a slot first and waits for
//! it. Ordinary typed messages are not held back; they are only counted, so a
//! paced job and the user's own chatting share one budget.
//!
//! The rules, per channel and account:
//!
//! | Sender in the channel            | Window budget  | Spacing in the channel |
//! | -------------------------------- | -------------- | ---------------------- |
//! | viewer                           | 20 per 30 s    | 1 per second, or the slow-mode wait when longer |
//! | VIP, moderator or broadcaster    | 100 per 30 s   | none from Twitch; a short gap keeps order |
//!
//! The window counts every send of the account across all channels, which is
//! the conservative reading of Twitch's rule. Room modes come from ROOMSTATE
//! and the sender's role from USERSTATE badges; until USERSTATE arrives the
//! sender counts as a viewer, unless the channel is their own.
//!
//! Refusals (returned instead of a wait): sub-only chat for a non-subscriber,
//! emote-only chat, and a viewer repeating their last message in the channel
//! inside 30 s (or ever, in unique-chat mode). Followers-only mode is reported
//! in the budget but never refused here: whether the account follows long
//! enough is Twitch's call, and its send result carries the reason.
//!
//! API for other modules:
//! - `acquire(channel, text).await`: wait for a slot, then the caller sends.
//! - `reserve(channel, text)`: the same decision without waiting; returns the wait.
//! - `record_sent(channel, text)`: count a send that did not go through `acquire`
//!   (`ChatService::send_message` calls it for every Twitch send).
//! - `budget(channel)`: the role, limits and room modes in force, for sizing a job.
//! - `room_state_fields(channel)`: the merged room modes plus their words for
//!   the message box (`room_modes`), shipped on every ROOMSTATE frame.
//! - `note_room_state` / `note_own_badges` / `note_room_id`: fed by the IRC reader.
//! - `own_follow_wanted` / `note_own_follow` / `own_follow_tick`: the account's
//!   own follow of a followers-only room, looked up once per channel, so the
//!   message box can say whether it holds the account back.
//! - `acquire_within_as(account, channel, text, max_wait)`: an automation's
//!   booking. It never waits: it books a slot and returns a [`Ticket`] that
//!   ends through `record_ticket` or `cancel_booking` (or its `Drop`). An
//!   automation's entry counts for [`ENGINE_WINDOW`] (Twitch's 30 s plus how
//!   late its send can reach Twitch) for every booker, and automation
//!   bookings stop at [`ENGINE_SHARE`] entries in any window: rules on this
//!   device use at most 12 of the 20 messages per 30 seconds, and typing,
//!   `/spam`, timers, scripts and reminder copies share the other 8.
//! - `record_sent_as` / `note_alt_badges`: the same counting for a linked
//!   account, which has its own limiter.
//! - `paced_send`: one scripted line through a [`PacedIo`], so a test runs it
//!   against a fake; `chat_send_paced` is its command.
//!
//! Channels are bare lowercase Twitch logins (no `#`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::services::room_modes::{describe_with, ModesIn, OwnFollow, Standing};

/// Twitch's rolling window for the message budget.
pub const WINDOW: Duration = Duration::from_secs(30);
/// Messages per window for a viewer.
pub const VIEWER_PER_WINDOW: usize = 20;
/// Messages per window for a VIP, moderator or broadcaster.
pub const PRIVILEGED_PER_WINDOW: usize = 100;
/// A viewer's minimum gap between two messages in one channel. Twitch's rule is
/// one per second; the extra 100 ms absorbs clock and network jitter.
pub const VIEWER_SPACING: Duration = Duration::from_millis(1100);
/// A privileged sender's gap, only so copies land in order.
pub const PRIVILEGED_SPACING: Duration = Duration::from_millis(300);
/// How far ahead slots may be booked. A job that would wait longer is refused
/// rather than left hanging. Longer than Twitch's slowest slow mode (120 s)
/// plus the margin added to it, so slow mode alone never refuses a line.
pub const MAX_WAIT: Duration = Duration::from_secs(130);
/// Twitch rejects a viewer's identical message inside this span.
pub const DUPLICATE_SPAN: Duration = Duration::from_secs(30);
/// How far off its booked slot an automation's send may start before it
/// books again; the next booking in that chat adds it to the spacing.
pub const REBOOK_TOLERANCE: Duration = Duration::from_millis(250);
/// The automation HTTP client's total timeout.
pub const ENGINE_SEND_TIMEOUT: Duration = Duration::from_secs(5);
/// How late an automation's send can reach Twitch after its booked slot.
pub const ENGINE_DRIFT: Duration =
    Duration::from_millis(REBOOK_TOLERANCE.as_millis() as u64 + ENGINE_SEND_TIMEOUT.as_millis() as u64);
/// How long an automation's entry counts: Twitch's window plus the drift
/// (35.25 s), for every booker.
pub const ENGINE_WINDOW: Duration = Duration::from_millis(WINDOW.as_millis() as u64 + ENGINE_DRIFT.as_millis() as u64);
/// The most entries an automation booking leaves in any window, for the
/// account on this device, in every chat.
pub const ENGINE_SHARE: usize = 12;

/// The sender's standing in one channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Vip,
    Moderator,
    Broadcaster,
}

impl Role {
    /// VIPs, moderators and the broadcaster get the high limit and skip slow mode.
    pub fn privileged(self) -> bool {
        !matches!(self, Role::Viewer)
    }

    /// From a USERSTATE `badges` tag (`name/version,...`).
    pub fn from_badges(badges: &str) -> Role {
        let has = |name: &str| {
            badges
                .split(',')
                .any(|b| b.split('/').next().map(str::trim) == Some(name))
        };
        if has("broadcaster") {
            Role::Broadcaster
        } else if has("moderator") || has("lead_moderator") {
            Role::Moderator
        } else if has("vip") {
            Role::Vip
        } else {
            Role::Viewer
        }
    }
}

/// Chat settings from the channel's ROOMSTATE. `None` = not seen yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RoomModes {
    pub slow_seconds: Option<u64>,
    /// -1 = off, 0 = any follower, N = followed at least N minutes.
    pub followers_only_minutes: Option<i64>,
    pub subs_only: Option<bool>,
    pub emote_only: Option<bool>,
    pub r9k: Option<bool>,
}

/// Why a paced send may not go out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    SubsOnly,
    EmoteOnly,
    Duplicate,
    TooLong { wait_secs: u64 },
    EmptyChannel,
    /// An automation's booking: this device's rules already used their 12.
    ShareFull { wait_secs: u64 },
}

impl Refusal {
    /// Words for the person who asked for the send.
    pub fn text(&self) -> String {
        match self {
            Refusal::SubsOnly => "Chat is in sub-only mode, so these messages would not be accepted".into(),
            Refusal::EmoteOnly => "Chat is in emote-only mode, so text messages would not be accepted".into(),
            Refusal::Duplicate => "Twitch refuses the same message twice in a row. Change it a little and try again".into(),
            Refusal::TooLong { wait_secs } => format!(
                "Twitch's message limit is full for the next {wait_secs} seconds. Try again shortly"
            ),
            Refusal::EmptyChannel => "No channel to send to".into(),
            Refusal::ShareFull { wait_secs } => {
                format!("Automations' share of the message limit is full for {wait_secs} seconds")
            }
        }
    }
}

/// What is in force for a channel, for sizing a job or showing a limit.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Budget {
    pub channel: String,
    pub role: Role,
    pub privileged: bool,
    pub per_window: usize,
    pub window_seconds: u64,
    pub spacing_ms: u64,
    /// Sends already counted in the current window, across all channels.
    pub used_in_window: usize,
    pub slow_seconds: u64,
    pub subs_only: bool,
    pub viewer_is_sub: bool,
    pub emote_only: bool,
    pub r9k: bool,
    pub followers_only_minutes: i64,
}

/// The account's own follow of a channel, learned for followers-only chat.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FollowFact {
    #[default]
    NotAsked,
    Asking,
    /// `None` = not following.
    Known(Option<DateTime<Utc>>),
    /// The lookup had no answer; not asked again for this account.
    Unanswered,
}

#[derive(Default)]
struct ChannelFacts {
    modes: RoomModes,
    role: Option<Role>,
    viewer_is_sub: bool,
    /// The channel's user id, from ROOMSTATE's `room-id`.
    room_id: Option<String>,
    follow: FollowFact,
    /// Bumped whenever `follow` changes, so a countdown started for an older
    /// answer stops.
    follow_gen: u64,
    /// When the next send in this channel may go (booked or sent).
    last_slot: Option<Instant>,
    /// `last_slot` is an automation's slot: that send may start up to
    /// [`REBOOK_TOLERANCE`] late, so the next booking adds it to the spacing.
    last_slot_ticketed: bool,
    last_text: Option<(String, Instant)>,
    /// When the latest send in this channel started. Only ever moves forward.
    last_sent: Option<Instant>,
    /// The automation booking whose send set `last_sent`; None for any other.
    last_sent_by: Option<u64>,
    /// What `last_sent` was before an automation's send start moved it (the
    /// booking id, the prior instant and its sender), so a refused send that
    /// never posted leaves no spacing behind.
    sent_before: Option<(u64, Option<Instant>, Option<u64>)>,
    /// Booked slots whose send has not been recorded yet, oldest first, each
    /// with its automation booking id (None for a slot `acquire` handed out).
    pending: VecDeque<(Instant, Option<u64>)>,
}

struct Limiter {
    /// Every send and booked slot of the account, oldest first, each with the
    /// window budget of the role it went out under. A later booking must never
    /// push an earlier-booked send over its own budget.
    sends: VecDeque<(Instant, usize)>,
    /// The automation booking id of each `sends` entry, in lockstep with
    /// `sends` (same length, same order); None for every other entry.
    send_ids: VecDeque<Option<u64>>,
    channels: HashMap<String, ChannelFacts>,
    own_login: Option<String>,
    /// Room ids where the account's own sub (or a gift to it) showed in chat.
    /// Kept for the account, so a chat that learns its room id later, or a
    /// USERSTATE whose badges still lag the sub, keeps the account subscribed.
    own_subs: HashSet<String>,
    /// Drawn from a process-wide counter when the limiter is made, so a
    /// booking from a replaced limiter never acts on its successor.
    gen: u64,
    /// A linked account's limiter: Twitch answers for sub-only and
    /// emote-only chat, so those are never refused here.
    is_alt: bool,
}

/// Limiter generations, process-wide: a new limiter never repeats one.
static NEXT_GEN: AtomicU64 = AtomicU64::new(1);
/// Automation booking ids, process-wide.
static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

impl Default for Limiter {
    fn default() -> Self {
        Limiter {
            sends: VecDeque::new(),
            send_ids: VecDeque::new(),
            channels: HashMap::new(),
            own_login: None,
            own_subs: HashSet::new(),
            gen: NEXT_GEN.fetch_add(1, Ordering::Relaxed),
            is_alt: false,
        }
    }
}

/// How long an entry counts: an automation's for [`ENGINE_WINDOW`] (its send
/// can reach Twitch up to [`ENGINE_DRIFT`] after its slot), any other for
/// [`WINDOW`].
fn counted_for(id: Option<u64>) -> Duration {
    if id.is_some() {
        ENGINE_WINDOW
    } else {
        WINDOW
    }
}

/// Insert a booked slot keeping the list ordered by time.
fn insert_pending(pending: &mut VecDeque<(Instant, Option<u64>)>, at: Instant, id: Option<u64>) {
    let pos = pending.iter().position(|(t, _)| *t > at).unwrap_or(pending.len());
    pending.insert(pos, (at, id));
}

/// A send started in the channel at `at`: `last_sent` moves to it unless a
/// later send already set it.
fn note_sent(facts: &mut ChannelFacts, at: Instant, by: Option<u64>) {
    if facts.last_sent.is_none_or(|sent| at >= sent) {
        facts.last_sent = Some(at);
        facts.last_sent_by = by;
    }
}

/// Who a slot search is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Booker {
    /// `reserve`, `acquire`: the role's budget over today's window.
    Plain,
    /// An automation: at most [`ENGINE_SHARE`] entries in any window.
    Engine,
}

/// A slot search's answer.
#[derive(Clone, Copy, Debug)]
struct Found {
    at: Instant,
    /// How many times the search moved or checked the slot.
    passes: usize,
    /// The automations' 12 (not Twitch's limit) moved the slot.
    share_held: bool,
}

/// An automation's booking as the limiter recorded it: plain data, held by a
/// [`Ticket`].
#[derive(Clone, Debug)]
struct Booked {
    id: u64,
    gen: u64,
    channel: String,
    at: Instant,
    booked_at: Instant,
    limit: usize,
    prior_text: Option<(String, Instant)>,
    text: String,
    wait: Duration,
}

fn key(channel: &str) -> String {
    channel.trim().trim_start_matches('#').to_ascii_lowercase()
}

impl Limiter {
    /// A linked account's limiter.
    fn alt(login: Option<String>) -> Limiter {
        Limiter { own_login: login, is_alt: true, ..Limiter::default() }
    }

    fn facts(&mut self, channel: &str) -> &mut ChannelFacts {
        self.channels.entry(key(channel)).or_default()
    }

    /// Marks the account subscribed in every open channel whose user id is
    /// `room_id`; the channels whose standing changed.
    fn own_sub(&mut self, room_id: &str) -> Vec<String> {
        self.own_subs.insert(room_id.to_string());
        let mut changed = Vec::new();
        for (channel, facts) in self.channels.iter_mut() {
            if facts.room_id.as_deref() == Some(room_id) && !facts.viewer_is_sub {
                facts.viewer_is_sub = true;
                changed.push(channel.clone());
            }
        }
        changed
    }

    /// Whether the account's own sub showed in `channel`'s room.
    fn own_sub_seen(&self, channel: &str) -> bool {
        self.channels
            .get(&key(channel))
            .and_then(|f| f.room_id.as_ref())
            .is_some_and(|id| self.own_subs.contains(id))
    }

    /// The account's own badges in `channel`; true when its standing changed.
    fn own_badges(&mut self, channel: &str, badges: &str) -> bool {
        let seen = self.own_sub_seen(channel);
        let facts = self.facts(channel);
        let role = Some(Role::from_badges(badges));
        let is_sub = seen
            || badges
                .split(',')
                .any(|b| b.starts_with("subscriber/") || b.starts_with("founder/"));
        let changed = facts.role != role || facts.viewer_is_sub != is_sub;
        facts.role = role;
        facts.viewer_is_sub = is_sub;
        changed
    }

    /// `channel`'s user id; true when an own sub already seen in that room
    /// changed the account's standing there.
    fn room_id(&mut self, channel: &str, room_id: &str) -> bool {
        let sub = self.own_subs.contains(room_id);
        let facts = self.facts(channel);
        facts.room_id = Some(room_id.to_string());
        if sub && !facts.viewer_is_sub {
            facts.viewer_is_sub = true;
            return true;
        }
        false
    }

    fn role_of(&self, channel: &str) -> Role {
        let k = key(channel);
        if let Some(role) = self.channels.get(&k).and_then(|f| f.role) {
            return role;
        }
        // The account's own channel, before USERSTATE says so.
        if self.own_login.as_deref() == Some(k.as_str()) {
            return Role::Broadcaster;
        }
        Role::Viewer
    }

    fn per_window(&self, channel: &str) -> usize {
        if self.role_of(channel).privileged() {
            PRIVILEGED_PER_WINDOW
        } else {
            VIEWER_PER_WINDOW
        }
    }

    /// Sends counted in the window that ends at `end`: an automation's entry
    /// within [`ENGINE_WINDOW`] before `end`, any other within [`WINDOW`].
    fn in_window(&self, end: Instant) -> usize {
        self.in_window_except(end, None)
    }

    /// `in_window`, leaving out the entry of automation booking `except`.
    fn in_window_except(&self, end: Instant, except: Option<u64>) -> usize {
        self.sends
            .iter()
            .zip(self.send_ids.iter())
            .filter(|((t, _), id)| {
                (except.is_none() || **id != except) && *t <= end && end.saturating_duration_since(*t) < counted_for(**id)
            })
            .count()
    }

    /// The earliest instant an entry counted at `end` leaves the window:
    /// always later than `end`.
    fn next_leave(&self, end: Instant) -> Instant {
        self.sends
            .iter()
            .zip(self.send_ids.iter())
            .filter(|((t, _), id)| *t <= end && end.saturating_duration_since(*t) < counted_for(**id))
            .map(|((t, _), id)| *t + counted_for(*id))
            .min()
            .unwrap_or(end + Duration::from_millis(1))
    }

    fn insert_send(&mut self, at: Instant, limit: usize) {
        self.insert_entry(at, limit, None);
    }

    /// Insert an entry keeping `sends` ordered by time (a booking may land
    /// before later bookings of another channel), its id at the same place.
    fn insert_entry(&mut self, at: Instant, limit: usize, id: Option<u64>) {
        let pos = self.sends.iter().position(|(t, _)| *t > at).unwrap_or(self.sends.len());
        self.sends.insert(pos, (at, limit));
        self.send_ids.insert(pos, id);
    }

    /// Remove automation booking `id`'s entry; whether it was there.
    fn remove_entry(&mut self, id: u64) -> bool {
        match self.send_ids.iter().position(|e| *e == Some(id)) {
            Some(pos) => {
                self.sends.remove(pos);
                self.send_ids.remove(pos);
                true
            }
            None => false,
        }
    }

    fn prune(&mut self, now: Instant) {
        // Oldest first; each entry leaves once its own span has passed.
        let mut i = 0;
        while i < self.sends.len() {
            let at = self.sends[i].0;
            if at > now {
                break;
            }
            if now.saturating_duration_since(at) >= counted_for(self.send_ids[i]) {
                self.sends.remove(i);
                self.send_ids.remove(i);
            } else {
                i += 1;
            }
        }
        for facts in self.channels.values_mut() {
            // A booked slot nobody sent on is dropped once it is well past.
            // An automation's slot ends only through its booking's owner.
            facts
                .pending
                .retain(|(t, id)| id.is_some() || now.saturating_duration_since(*t) <= Duration::from_secs(10));
        }
    }

    /// The refusal for `text` in `channel`, if any.
    fn refusal(&self, channel: &str, text: &str, now: Instant) -> Option<Refusal> {
        let k = key(channel);
        if k.is_empty() {
            return Some(Refusal::EmptyChannel);
        }
        let role = self.role_of(&k);
        if role.privileged() {
            return None;
        }
        let facts = self.channels.get(&k);
        let modes = facts.map(|f| f.modes).unwrap_or_default();
        if !self.is_alt && modes.subs_only == Some(true) && !facts.map(|f| f.viewer_is_sub).unwrap_or(false) {
            return Some(Refusal::SubsOnly);
        }
        if !self.is_alt && modes.emote_only == Some(true) {
            return Some(Refusal::EmoteOnly);
        }
        if let Some((last, at)) = facts.and_then(|f| f.last_text.as_ref()) {
            let same = last.trim() == text.trim();
            let recent = now.saturating_duration_since(*at) < DUPLICATE_SPAN;
            if same && (recent || modes.r9k == Some(true)) {
                return Some(Refusal::Duplicate);
            }
        }
        None
    }

    /// The gap two sends in `channel` keep: a second for a viewer (or the
    /// slow-mode wait plus a quarter second of margin, when longer), a short
    /// gap for a privileged sender.
    fn spacing(&self, channel: &str) -> Duration {
        let k = key(channel);
        if self.role_of(&k).privileged() {
            return PRIVILEGED_SPACING;
        }
        match self.channels.get(&k).and_then(|f| f.modes.slow_seconds).filter(|s| *s > 0) {
            Some(slow) => VIEWER_SPACING.max(Duration::from_secs(slow) + Duration::from_millis(250)),
            None => VIEWER_SPACING,
        }
    }

    /// The earliest instant at or after `now` a send in `channel` may go.
    fn earliest(&self, channel: &str, now: Instant) -> Instant {
        self.search(channel, now, Booker::Plain).at
    }

    /// The slot search every booker shares. A slot fits when its own window
    /// holds fewer than the booker's cap (the role's budget; for an
    /// automation at most [`ENGINE_SHARE`]) and every entry booked later,
    /// whose window the new entry would join, still has room for it. When it
    /// does not fit, the slot moves to the earliest instant an entry counted
    /// in its window leaves (an automation's entry [`ENGINE_WINDOW`] after its
    /// slot, any other [`WINDOW`]), or just past the later entry that has no
    /// room. Every pass moves the slot strictly later, so the search ends.
    fn search(&self, channel: &str, now: Instant, booker: Booker) -> Found {
        let k = key(channel);
        let facts = self.channels.get(&k);
        let mut at = now;
        if let Some(last) = facts.and_then(|f| f.last_slot) {
            let late = if facts.is_some_and(|f| f.last_slot_ticketed) { REBOOK_TOLERANCE } else { Duration::ZERO };
            at = at.max(last + late + self.spacing(&k));
        }
        let limit = self.per_window(&k);
        let (cap, reach) = match booker {
            Booker::Plain => (limit, WINDOW),
            Booker::Engine => (ENGINE_SHARE.min(limit), ENGINE_WINDOW),
        };
        let mut found = Found { at, passes: 0, share_held: false };
        loop {
            found.passes += 1;
            let before = at;
            let count = self.in_window(at);
            if count >= cap {
                if count < limit {
                    found.share_held = true;
                }
                at = self.next_leave(at);
            } else {
                // An entry booked later whose window this one would join must
                // still have room for one more: the role's budget for a plain
                // entry, the automations' 12 for an automation's entry when an
                // automation books.
                let blocked = self
                    .sends
                    .iter()
                    .zip(self.send_ids.iter())
                    .filter(|((u, _), _)| *u > at && u.saturating_duration_since(at) < reach)
                    .find(|((u, budget), id)| {
                        let room = if booker == Booker::Engine && id.is_some() { ENGINE_SHARE } else { *budget };
                        self.in_window(*u) >= room
                    })
                    .map(|((u, _), id)| (*u, id.is_some()));
                match blocked {
                    Some((u, ticketed)) => {
                        if booker == Booker::Engine && ticketed {
                            found.share_held = true;
                        }
                        at = u + Duration::from_millis(1);
                    }
                    None => {
                        found.at = at;
                        return found;
                    }
                }
            }
            debug_assert!(at > before, "every pass moves the slot later");
        }
    }

    fn book(&mut self, channel: &str, text: &str, now: Instant) -> Result<Duration, Refusal> {
        self.prune(now);
        if let Some(refusal) = self.refusal(channel, text, now) {
            return Err(refusal);
        }
        let at = self.earliest(channel, now);
        let wait = at.saturating_duration_since(now);
        if wait > MAX_WAIT {
            return Err(Refusal::TooLong { wait_secs: wait.as_secs() });
        }
        // Keep `sends` ordered: a booking may land before later bookings of
        // another channel.
        let limit = self.per_window(channel);
        self.insert_send(at, limit);
        let facts = self.facts(channel);
        facts.last_slot = Some(at);
        facts.last_slot_ticketed = false;
        facts.last_text = Some((text.to_string(), at));
        insert_pending(&mut facts.pending, at, None);
        Ok(wait)
    }

    /// An automation's booking: books nothing on a refusal or when the slot
    /// is more than `max_wait` away.
    fn book_engine(&mut self, channel: &str, text: &str, now: Instant, max_wait: Duration) -> Result<Booked, Refusal> {
        self.prune(now);
        if let Some(refusal) = self.refusal(channel, text, now) {
            return Err(refusal);
        }
        let found = self.search(channel, now, Booker::Engine);
        let wait = found.at.saturating_duration_since(now);
        if wait > max_wait.min(MAX_WAIT) {
            let wait_secs = wait.as_millis().div_ceil(1000) as u64;
            return Err(if found.share_held {
                Refusal::ShareFull { wait_secs }
            } else {
                Refusal::TooLong { wait_secs }
            });
        }
        let id = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
        let limit = self.per_window(channel);
        self.insert_entry(found.at, limit, Some(id));
        let gen = self.gen;
        let facts = self.facts(channel);
        let prior_text = facts.last_text.clone();
        facts.last_slot = Some(found.at);
        facts.last_slot_ticketed = true;
        facts.last_text = Some((text.to_string(), found.at));
        insert_pending(&mut facts.pending, found.at, Some(id));
        Ok(Booked {
            id,
            gen,
            channel: key(channel),
            at: found.at,
            booked_at: now,
            limit,
            prior_text,
            text: text.to_string(),
            wait,
        })
    }

    /// The automation's send for booking `b` starts now.
    fn mark(&mut self, b: &Booked, now: Instant) {
        if b.gen != self.gen {
            return;
        }
        let facts = self.facts(&b.channel);
        let prior = (facts.last_sent, facts.last_sent_by);
        note_sent(facts, now, Some(b.id));
        if facts.last_sent_by == Some(b.id) {
            facts.sent_before = Some((b.id, prior.0, prior.1));
        }
        if facts.last_slot.is_none_or(|last| last < now) {
            facts.last_slot = Some(now);
            facts.last_slot_ticketed = true;
        }
    }

    /// The automation's send for booking `b` was answered (or may have
    /// posted): its entry stays at its booked slot and keeps counting for
    /// [`ENGINE_WINDOW`]; nothing is inserted.
    fn record_booked(&mut self, b: &Booked, sent_at: Instant) {
        if b.gen != self.gen {
            return;
        }
        let facts = self.facts(&b.channel);
        facts.pending.retain(|(_, id)| *id != Some(b.id));
        note_sent(facts, sent_at, Some(b.id));
    }

    /// Booking `b` never posted: its entry goes, a send start it marked is
    /// undone while it is still the latest, the chat's next slot is the
    /// latest remaining booking or send, and its text stops counting as the
    /// last one while it still is.
    fn cancel(&mut self, b: &Booked) {
        if b.gen != self.gen {
            return;
        }
        self.remove_entry(b.id);
        let facts = self.facts(&b.channel);
        facts.pending.retain(|(_, id)| *id != Some(b.id));
        if facts.last_sent_by == Some(b.id) {
            if let Some((_, sent, by)) = facts.sent_before.filter(|(id, _, _)| *id == b.id) {
                facts.last_sent = sent;
                facts.last_sent_by = by;
            }
        }
        let latest = facts.pending.iter().max_by_key(|(t, _)| *t).copied();
        let sent = facts.last_sent.map(|t| (t, facts.last_sent_by));
        (facts.last_slot, facts.last_slot_ticketed) = match (latest, sent) {
            (Some((p, pid)), Some((s, _))) if p >= s => (Some(p), pid.is_some()),
            (Some((p, pid)), None) => (Some(p), pid.is_some()),
            (_, Some((s, by))) => (Some(s), by.is_some()),
            (None, None) => (None, false),
        };
        if facts.last_text.as_ref().is_some_and(|(t, at)| *t == b.text && *at == b.at) {
            facts.last_text = b.prior_text.clone();
        }
    }

    /// Booking `b` now sends `text`.
    fn retext_booked(&mut self, b: &Booked, text: &str) {
        if b.gen != self.gen {
            return;
        }
        if let Some(facts) = self.channels.get_mut(&b.channel) {
            if facts.last_text.as_ref().is_some_and(|(t, at)| *t == b.text && *at == b.at) {
                facts.last_text = Some((text.to_string(), b.at));
            }
        }
    }

    /// A send or another booking in booking `b`'s chat, after `b` was
    /// booked, now lies within the chat's spacing of `now`.
    fn spacing_moved(&self, b: &Booked, now: Instant) -> bool {
        if b.gen != self.gen {
            return false;
        }
        let Some(facts) = self.channels.get(&b.channel) else { return false };
        let spacing = self.spacing(&b.channel);
        let sent_close = facts.last_sent.is_some_and(|sent| {
            facts.last_sent_by != Some(b.id) && sent > b.booked_at && now.saturating_duration_since(sent) < spacing
        });
        let booked_close = facts.pending.iter().any(|(slot, id)| {
            let gap = if *slot >= now { slot.duration_since(now) } else { now.duration_since(*slot) };
            *id != Some(b.id) && gap < spacing
        });
        sent_close || booked_close
    }

    /// Whether booking `b` may send now: the window ending now holds fewer
    /// than [`ENGINE_SHARE`] entries besides its own (typed entries included),
    /// and nothing moved the chat's spacing.
    fn fits_engine(&mut self, b: &Booked, now: Instant) -> bool {
        if b.gen != self.gen {
            return false;
        }
        self.prune(now);
        self.in_window_except(now, Some(b.id)) < ENGINE_SHARE.min(self.per_window(&b.channel)) && !self.spacing_moved(b, now)
    }

    fn record(&mut self, channel: &str, text: &str, now: Instant) {
        self.prune(now);
        let facts = self.facts(channel);
        note_sent(facts, now, None);
        // A send on a slot `acquire` booked is already counted. An
        // automation's slot is never taken here: it ends through its booking.
        if let Some(i) = facts
            .pending
            .iter()
            .position(|(t, id)| id.is_none() && *t <= now + Duration::from_secs(2))
        {
            facts.pending.remove(i);
            facts.last_text = Some((text.to_string(), now));
            return;
        }
        if facts.last_slot.is_none_or(|last| last < now) {
            facts.last_slot = Some(now);
            facts.last_slot_ticketed = false;
        }
        facts.last_text = Some((text.to_string(), now));
        let limit = self.per_window(channel);
        self.insert_send(now, limit);
    }

    /// The role USERSTATE last gave `as_login` in `channel`, or None before
    /// one came or while the limiter speaks for another account.
    fn known_role(&self, as_login: &str, channel: &str) -> Option<Role> {
        if self.own_login.as_deref() != Some(as_login.trim().to_ascii_lowercase().as_str()) {
            return None;
        }
        self.channels.get(&key(channel)).and_then(|f| f.role)
    }

    /// The account's standing in `channel` for the room-mode words: unknown
    /// until USERSTATE says, except in the account's own channel.
    fn standing(&self, channel: &str) -> Standing {
        let k = key(channel);
        let facts = self.channels.get(&k);
        let role = facts
            .and_then(|f| f.role)
            .or_else(|| (self.own_login.as_deref() == Some(k.as_str())).then_some(Role::Broadcaster));
        match role {
            None => Standing::Unknown,
            Some(Role::Broadcaster) => Standing::Broadcaster,
            Some(Role::Moderator) => Standing::Moderator,
            Some(Role::Vip) => Standing::Vip,
            Some(Role::Viewer) => Standing::Viewer { is_sub: facts.map(|f| f.viewer_is_sub).unwrap_or(false) },
        }
    }

    /// The account's follow of `channel` as the room-mode words take it.
    fn own_follow(&self, channel: &str, now: DateTime<Utc>) -> OwnFollow {
        match self.channels.get(&key(channel)).map(|f| f.follow) {
            Some(FollowFact::Known(None)) => OwnFollow::NotFollowing,
            Some(FollowFact::Known(Some(since))) => OwnFollow::ForSecs((now - since).num_seconds().max(0)),
            _ => OwnFollow::Unknown,
        }
    }

    /// How long until the followers-only countdown for `channel` next changes
    /// its words (a whole minute passes, or the wait ends). `None` when there
    /// is no countdown, or `gen` belongs to an older follow answer.
    fn follow_tick(&self, channel: &str, gen: u64, now: DateTime<Utc>) -> Option<Duration> {
        let facts = self.channels.get(&key(channel))?;
        if facts.follow_gen != gen || self.role_of(channel).privileged() {
            return None;
        }
        let FollowFact::Known(Some(since)) = facts.follow else { return None };
        let need = facts.modes.followers_only_minutes.filter(|m| *m > 0)? * 60;
        let left = need - (now - since).num_seconds();
        if left <= 0 {
            return None;
        }
        // The words count whole minutes rounded up, so they change each time
        // `left` crosses a minute; one more second lands just past it.
        let to_change = match left % 60 {
            0 => 60,
            r => r,
        };
        Some(Duration::from_secs(to_change as u64 + 1))
    }

    /// The room's merged ROOMSTATE fields and their words (`modes`), or `None`
    /// before any ROOMSTATE arrived for `channel`.
    fn room_fields(&self, channel: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
        let m = self.channels.get(&key(channel))?.modes;
        if m == RoomModes::default() {
            return None;
        }
        let mut out = serde_json::Map::new();
        if let Some(v) = m.followers_only_minutes {
            out.insert("followers_only".into(), v.into());
        }
        if let Some(v) = m.slow_seconds {
            out.insert("slow".into(), v.into());
        }
        if let Some(v) = m.subs_only {
            out.insert("subs_only".into(), v.into());
        }
        if let Some(v) = m.emote_only {
            out.insert("emote_only".into(), v.into());
        }
        if let Some(v) = m.r9k {
            out.insert("r9k".into(), v.into());
        }
        let mut view = describe_with(
            &ModesIn {
                slow_seconds: m.slow_seconds.unwrap_or(0),
                followers_only_minutes: m.followers_only_minutes.unwrap_or(-1),
                subs_only: m.subs_only.unwrap_or(false),
                emote_only: m.emote_only.unwrap_or(false),
                unique: m.r9k.unwrap_or(false),
            },
            self.standing(channel),
            self.own_follow(channel, Utc::now()),
        );
        view.warning = self
            .channels
            .get(&key(channel))
            .and_then(|f| f.room_id.as_deref())
            .and_then(crate::services::chat_warning::view);
        out.insert("modes".into(), serde_json::to_value(view).unwrap_or_default());
        Some(out)
    }

    fn budget(&mut self, channel: &str, now: Instant) -> Budget {
        self.prune(now);
        let k = key(channel);
        let role = self.role_of(&k);
        let facts = self.channels.get(&k);
        let modes = facts.map(|f| f.modes).unwrap_or_default();
        let used = self.in_window(now);
        Budget {
            channel: k.clone(),
            role,
            privileged: role.privileged(),
            per_window: if role.privileged() { PRIVILEGED_PER_WINDOW } else { VIEWER_PER_WINDOW },
            window_seconds: WINDOW.as_secs(),
            spacing_ms: if role.privileged() {
                PRIVILEGED_SPACING.as_millis() as u64
            } else {
                VIEWER_SPACING.as_millis() as u64
            },
            used_in_window: used,
            slow_seconds: modes.slow_seconds.unwrap_or(0),
            subs_only: modes.subs_only.unwrap_or(false),
            viewer_is_sub: facts.map(|f| f.viewer_is_sub).unwrap_or(false),
            emote_only: modes.emote_only.unwrap_or(false),
            r9k: modes.r9k.unwrap_or(false),
            followers_only_minutes: modes.followers_only_minutes.unwrap_or(-1),
        }
    }
}

static LIMITER: OnceLock<Mutex<Limiter>> = OnceLock::new();

fn limiter() -> std::sync::MutexGuard<'static, Limiter> {
    LIMITER
        .get_or_init(|| Mutex::new(Limiter::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn own_login_hint(l: &mut Limiter) {
    if l.own_login.is_none() {
        l.own_login = crate::services::account_store::AccountStore::primary().map(|a| a.login.to_ascii_lowercase());
    }
}

/// One limiter per linked account, by account id.
static ALT_LIMITERS: OnceLock<Mutex<HashMap<String, Limiter>>> = OnceLock::new();

fn alt_limiters() -> MutexGuard<'static, HashMap<String, Limiter>> {
    ALT_LIMITERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Which limiter a booking belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LimiterKey {
    /// The main account (the IRC connection's).
    Primary,
    /// A linked account, by id.
    Alt(String),
}

impl LimiterKey {
    pub fn is_primary(&self) -> bool {
        matches!(self, LimiterKey::Primary)
    }
}

/// Run `f` on the limiter `account` names, under its lock. `f` gets and
/// returns plain data, never a booking's owner: a [`Ticket`]'s `Drop` takes
/// this same lock, so none may be dropped inside.
fn with_limiter<R>(account: &LimiterKey, f: impl FnOnce(&mut Limiter) -> R) -> R {
    match account {
        LimiterKey::Primary => {
            let mut l = limiter();
            own_login_hint(&mut l);
            f(&mut l)
        }
        LimiterKey::Alt(id) => {
            let mut alts = alt_limiters();
            f(alts.entry(id.clone()).or_insert_with(|| Limiter::alt(None)))
        }
    }
}

fn limiter_key_for(account_id: &str) -> LimiterKey {
    let primary = crate::services::account_store::AccountStore::primary().map(|p| p.user_id);
    if primary.as_deref() == Some(account_id) {
        LimiterKey::Primary
    } else {
        LimiterKey::Alt(account_id.to_string())
    }
}

/// A linked account's own channel counts as the broadcaster's before any
/// USERSTATE: learn its login once.
fn note_alt_login(id: &str) {
    let known = alt_limiters().get(id).is_some_and(|l| l.own_login.is_some());
    if known {
        return;
    }
    if let Some(login) = crate::services::account_store::AccountStore::get(id).map(|a| a.login.to_ascii_lowercase()) {
        with_limiter(&LimiterKey::Alt(id.to_string()), |l| l.own_login = Some(login));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TicketState {
    Booked,
    /// `mark_sending` ran: the send may have posted.
    Sending,
    /// Recorded or cancelled.
    Done,
}

/// An automation's booked send slot. It ends through [`record_ticket`] (the
/// send was answered or may have posted) or [`cancel_booking`] (it never
/// posted). Dropped unsettled, it cancels; dropped after [`mark_sending`], it
/// records at its booked slot, because the send may have posted.
///
/// Never drop a `Ticket` while holding the limiter's lock or the linked
/// limiters' lock: its `Drop` takes that lock.
#[must_use = "a ticket that is dropped cancels its booking"]
pub struct Ticket {
    account: LimiterKey,
    booked: Booked,
    sent_at: Option<Instant>,
    state: TicketState,
}

impl Ticket {
    fn new(account: LimiterKey, booked: Booked) -> Ticket {
        Ticket { account, booked, sent_at: None, state: TicketState::Booked }
    }

    /// The booking's id, process-wide.
    pub fn id(&self) -> u64 {
        self.booked.id
    }

    /// The limiter it booked on: the main account's or a linked account's.
    pub fn account(&self) -> &LimiterKey {
        &self.account
    }

    /// That limiter's generation when it booked.
    pub fn gen(&self) -> u64 {
        self.booked.gen
    }

    pub fn channel(&self) -> &str {
        &self.booked.channel
    }

    /// The booked slot.
    pub fn at(&self) -> Instant {
        self.booked.at
    }

    /// When it was booked.
    pub fn booked_at(&self) -> Instant {
        self.booked.booked_at
    }

    /// The window budget of the role it booked under.
    pub fn limit(&self) -> usize {
        self.booked.limit
    }

    /// The chat's last text before this booking.
    pub fn prior_text(&self) -> Option<&(String, Instant)> {
        self.booked.prior_text.as_ref()
    }

    /// The text it will send.
    pub fn text(&self) -> &str {
        &self.booked.text
    }

    /// Recorded or cancelled: nothing more happens on drop.
    pub fn is_settled(&self) -> bool {
        self.state == TicketState::Done
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        match self.state {
            TicketState::Booked => cancel_booking(self),
            TicketState::Sending => record_ticket(self),
            TicketState::Done => {}
        }
    }
}

/// Book an automation's send for `account_id` in `channel` without waiting:
/// the slot and its ticket, or the refusal (and nothing booked). A linked
/// account books on its own limiter, with the room modes the main
/// connection saw. Over `max_wait` it refuses with `ShareFull` when this
/// device's 12 held it, else `TooLong`.
pub fn acquire_within_as(account_id: &str, channel: &str, text: &str, max_wait: Duration) -> Result<(Duration, Ticket), Refusal> {
    let account = limiter_key_for(account_id);
    if let LimiterKey::Alt(id) = &account {
        note_alt_login(id);
    }
    acquire_within_on(account, channel, text, max_wait)
}

/// `acquire_within_as` on a known limiter.
pub(crate) fn acquire_within_on(account: LimiterKey, channel: &str, text: &str, max_wait: Duration) -> Result<(Duration, Ticket), Refusal> {
    let now = Instant::now();
    let modes = match &account {
        LimiterKey::Alt(_) => limiter().channels.get(&key(channel)).map(|f| f.modes),
        LimiterKey::Primary => None,
    };
    let booked = with_limiter(&account, |l| {
        if let Some(modes) = modes {
            l.facts(channel).modes = modes;
        }
        l.book_engine(channel, text, now, max_wait)
    })?;
    let wait = booked.wait;
    Ok((wait, Ticket::new(account, booked)))
}

/// The automation's send starts now; there must be no await between this
/// and the request.
pub fn mark_sending(ticket: &mut Ticket) {
    if ticket.state != TicketState::Booked {
        return;
    }
    let now = Instant::now();
    with_limiter(&ticket.account, |l| l.mark(&ticket.booked, now));
    ticket.sent_at = Some(now);
    ticket.state = TicketState::Sending;
}

/// The automation's send was answered or may have posted: it stays counted
/// at its booked slot. `last_sent` keeps the instant the send started.
pub fn record_ticket(ticket: &mut Ticket) {
    if ticket.state == TicketState::Done {
        return;
    }
    let sent_at = ticket.sent_at.unwrap_or(ticket.booked.at);
    with_limiter(&ticket.account, |l| l.record_booked(&ticket.booked, sent_at));
    ticket.state = TicketState::Done;
}

/// The automation's send never posted (a refusal, a skip): its booking goes
/// and the chat's spacing and duplicate check return to what remains.
pub fn cancel_booking(ticket: &mut Ticket) {
    if ticket.state == TicketState::Done {
        return;
    }
    with_limiter(&ticket.account, |l| l.cancel(&ticket.booked));
    ticket.state = TicketState::Done;
}

/// The ticket now sends `text` (it changed after booking).
pub fn retext(ticket: &mut Ticket, text: &str) {
    if ticket.state == TicketState::Done {
        return;
    }
    with_limiter(&ticket.account, |l| l.retext_booked(&ticket.booked, text));
    ticket.booked.text = text.to_string();
}

/// Whether the ticket may send now: fewer than 12 entries besides its own
/// in the window ending now, and nothing moved the chat's spacing.
pub fn window_fits_engine(ticket: &Ticket, now: Instant) -> bool {
    if ticket.state == TicketState::Done {
        return false;
    }
    with_limiter(&ticket.account, |l| l.fits_engine(&ticket.booked, now))
}

/// A send or another booking in the ticket's chat, after it was booked, now
/// lies within the chat's spacing of `now` (a message you typed while the
/// automation waited).
pub fn spacing_moved(ticket: &Ticket, now: Instant) -> bool {
    with_limiter(&ticket.account, |l| l.spacing_moved(&ticket.booked, now))
}

/// Count a send made as `account_id` (the main account's through
/// `record_sent`, a linked account's on its own limiter).
pub fn record_sent_as(account_id: &str, channel: &str, text: &str) {
    if key(channel).is_empty() {
        return;
    }
    match limiter_key_for(account_id) {
        LimiterKey::Primary => record_sent(channel, text),
        alt => record_on(&alt, channel, text),
    }
}

pub(crate) fn record_on(account: &LimiterKey, channel: &str, text: &str) {
    with_limiter(account, |l| l.record(channel, text, Instant::now()));
}

/// A linked account's own badges in `channel`, seen on the main connection:
/// its role there. True when its standing changed.
pub fn note_alt_badges(account_id: &str, channel: &str, badges: &str) -> bool {
    with_limiter(&LimiterKey::Alt(account_id.to_string()), |l| l.own_badges(channel, badges))
}

/// A linked account's limiter entries and booked slots, for tests elsewhere.
#[cfg(test)]
pub(crate) fn alt_entries(id: &str) -> (usize, usize) {
    alt_limiters()
        .get(id)
        .map_or((0, 0), |l| (l.sends.len(), l.channels.values().map(|f| f.pending.len()).sum()))
}

/// Book the next slot for `text` in `channel`; returns how long to wait before
/// sending. The slot is counted from now on, so call this only for a send that
/// will happen.
pub fn reserve(channel: &str, text: &str) -> Result<Duration, Refusal> {
    let mut l = limiter();
    own_login_hint(&mut l);
    l.book(channel, text, Instant::now())
}

/// Wait for the next slot in `channel`, then return; the caller sends right
/// after. Returns how long it waited.
pub async fn acquire(channel: &str, text: &str) -> Result<Duration, Refusal> {
    let wait = reserve(channel, text)?;
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
    Ok(wait)
}

/// Count a send that went out without `acquire` (or claim the slot it booked).
pub fn record_sent(channel: &str, text: &str) {
    if key(channel).is_empty() {
        return;
    }
    limiter().record(channel, text, Instant::now());
}

/// What USERSTATE last said `as_login` is in `channel`; None before it said,
/// or when chat is signed in as someone else.
pub fn known_role(as_login: &str, channel: &str) -> Option<Role> {
    let mut l = limiter();
    own_login_hint(&mut l);
    l.known_role(as_login, channel)
}

/// The limits and room modes in force for `channel`.
pub fn budget(channel: &str) -> Budget {
    let mut l = limiter();
    own_login_hint(&mut l);
    l.budget(channel, Instant::now())
}

/// A ROOMSTATE frame for `channel`. Twitch sends the full set on join and only
/// the changed tags afterwards, so `None` fields keep what was known.
pub fn note_room_state(channel: &str, update: RoomModes) {
    let mut l = limiter();
    let modes = &mut l.facts(channel).modes;
    if update.slow_seconds.is_some() {
        modes.slow_seconds = update.slow_seconds;
    }
    if update.followers_only_minutes.is_some() {
        modes.followers_only_minutes = update.followers_only_minutes;
    }
    if update.subs_only.is_some() {
        modes.subs_only = update.subs_only;
    }
    if update.emote_only.is_some() {
        modes.emote_only = update.emote_only;
    }
    if update.r9k.is_some() {
        modes.r9k = update.r9k;
    }
}

/// The account's own badges in `channel`, from USERSTATE. True when the
/// account's standing there changed (a new role, or subscribed or not).
pub fn note_own_badges(channel: &str, badges: &str) -> bool {
    limiter().own_badges(channel, badges)
}

/// The account subscribed in the channel whose user id is `room_id` (its own
/// sub, or a gift to it, seen in that chat). Twitch resends USERSTATE only
/// after an own message, which sub-only chat refuses, so this is how an open
/// chat learns the account may talk there now. The channels whose standing
/// changed.
pub fn note_own_sub(room_id: &str) -> Vec<String> {
    limiter().own_sub(room_id)
}

/// The channel's user id, from ROOMSTATE's `room-id`. True when an own sub
/// seen there before the id was known changed the account's standing.
pub fn note_room_id(channel: &str, room_id: &str) -> bool {
    let room_id = room_id.trim();
    !room_id.is_empty() && limiter().room_id(channel, room_id)
}

/// `channel`'s user id, once a ROOMSTATE named it.
pub fn room_id_of(channel: &str) -> Option<String> {
    limiter().channels.get(&key(channel)).and_then(|f| f.room_id.clone())
}

/// Every open channel whose user id is `room_id` (a chat can be open under
/// more than one key, a popout beside the main window).
pub fn channels_with_room_id(room_id: &str) -> Vec<String> {
    limiter()
        .channels
        .iter()
        .filter(|(_, f)| f.room_id.as_deref() == Some(room_id))
        .map(|(channel, _)| channel.clone())
        .collect()
}

/// Whether the account's follow of `channel` should be looked up now: the room
/// is followers-only, the account is not known to be privileged there, and it
/// has not been asked. Returns the channel's user id and marks it asked, so
/// one lookup runs per channel and account.
pub fn own_follow_wanted(channel: &str) -> Option<String> {
    let mut l = limiter();
    own_login_hint(&mut l);
    if l.role_of(channel).privileged() {
        return None;
    }
    let facts = l.facts(channel);
    if facts.modes.followers_only_minutes.unwrap_or(-1) < 0 || facts.follow != FollowFact::NotAsked {
        return None;
    }
    let room_id = facts.room_id.clone()?;
    facts.follow = FollowFact::Asking;
    Some(room_id)
}

/// The lookup's answer for `channel`: `Some(since)` following, `Some(None)`
/// not following, `None` no answer. Returns the countdown generation.
pub fn note_own_follow(channel: &str, answer: Option<Option<DateTime<Utc>>>) -> u64 {
    let mut l = limiter();
    let facts = l.facts(channel);
    facts.follow = match answer {
        Some(since) => FollowFact::Known(since),
        None => FollowFact::Unanswered,
    };
    facts.follow_gen += 1;
    facts.follow_gen
}

/// The account followed (or unfollowed) the channel whose user id is
/// `room_id` from inside the app. Returns each open channel it applies to with
/// its countdown generation, so their rooms can be worded again.
pub fn note_own_follow_by_room_id(room_id: &str, following: bool) -> Vec<(String, u64)> {
    let mut l = limiter();
    let now = Utc::now();
    let mut touched = Vec::new();
    for (channel, facts) in l.channels.iter_mut() {
        if facts.room_id.as_deref() == Some(room_id) {
            facts.follow = FollowFact::Known(following.then_some(now));
            facts.follow_gen += 1;
            touched.push((channel.clone(), facts.follow_gen));
        }
    }
    touched
}

/// How long until `channel`'s followers-only countdown next changes its words;
/// `None` once it is over or a newer answer replaced generation `gen`.
pub fn own_follow_tick(channel: &str, gen: u64) -> Option<Duration> {
    let mut l = limiter();
    own_login_hint(&mut l);
    l.follow_tick(channel, gen, Utc::now())
}

/// The room's ROOMSTATE fields as merged so far, plus `modes`: the active
/// modes in words and what they mean for the signed-in account
/// (`room_modes::RoomModeView`). `None` before any ROOMSTATE for `channel`.
/// The IRC reader ships this on every ROOMSTATE frame, so the page never has
/// to merge partial frames to know the room.
pub fn room_state_fields(channel: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut l = limiter();
    own_login_hint(&mut l);
    l.room_fields(channel)
}

/// The signed-in account changed: roles and history belong to the old one.
pub fn reset_account() {
    *limiter() = Limiter::default();
    alt_limiters().clear();
    crate::services::chat_warning::reset_account();
}

/// Wait for a send slot in `channel` (bare login). Errors carry the refusal in
/// words. Returns the milliseconds waited.
#[tauri::command]
pub async fn chat_send_acquire(channel: String, text: String) -> Result<u64, String> {
    acquire(&channel, &text)
        .await
        .map(|w| w.as_millis() as u64)
        .map_err(|r| r.text())
}

/// The limits and room modes in force for `channel` (bare login).
#[tauri::command]
pub fn chat_send_budget(channel: String) -> Budget {
    budget(&channel)
}

/// How a paced send went: Twitch's verdict on the line.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct PacedSend {
    pub is_sent: bool,
    pub drop_reason: Option<String>,
}

/// Why a paced send did not finish. `Before`: nothing can have reached
/// Twitch (the room lookup, sign-in, an empty line, the limiter's refusal,
/// or a send error from before the request left). `After`: the send failed
/// once the request may have left, so the line may have posted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PacedFail {
    Before(String),
    After(String),
}

/// The words a paced send's failure shows, the same for both kinds.
pub fn fail_text(fail: &PacedFail) -> String {
    match fail {
        PacedFail::Before(text) | PacedFail::After(text) => text.clone(),
    }
}

/// One copy of a paced post.
pub type CopyResult = Result<PacedSend, PacedFail>;

/// Whether a post may let its claim go: only when nothing can have posted,
/// that is every attempted copy failed before its request left, or Twitch
/// answered `is_sent: false` with a reason. No copies (a post that returned
/// early) also lets it go.
pub fn release_after(copies: &[CopyResult]) -> bool {
    copies.iter().all(|copy| match copy {
        Err(PacedFail::Before(_)) => true,
        Err(PacedFail::After(_)) => false,
        Ok(sent) => !sent.is_sent && sent.drop_reason.as_deref().is_some_and(|r| !r.trim().is_empty()),
    })
}

/// What a paced send needs from outside: the sender, the room lookup, the
/// slot and the send. The real one reaches Twitch and panics in a test build.
#[async_trait::async_trait]
pub trait PacedIo: Send + Sync {
    /// The signed-in account's user id, or "Sign in to Twitch to send messages".
    async fn sender_id(&self) -> Result<String, String>;
    async fn user_by_login(&self, login: &str) -> Result<Option<serde_json::Value>, String>;
    async fn acquire(&self, channel: &str, text: &str) -> Result<Duration, Refusal>;
    async fn send(
        &self,
        text: &str,
        login: &str,
        broadcaster_id: &str,
        sender_id: &str,
    ) -> Result<crate::services::chat_service::SendResult, crate::services::chat_service::SendError>;
}

/// The paced send the app runs.
pub struct RealPacedIo;

fn real_only() {
    if cfg!(test) {
        panic!("the real paced send reaches Twitch; tests use a fake");
    }
}

#[async_trait::async_trait]
impl PacedIo for RealPacedIo {
    async fn sender_id(&self) -> Result<String, String> {
        real_only();
        match crate::services::account_store::AccountStore::primary() {
            Some(account) => Ok(account.user_id),
            None => crate::services::twitch_service::TwitchService::get_user_info()
                .await
                .map(|u| u.id)
                .map_err(|_| "Sign in to Twitch to send messages".to_string()),
        }
    }

    async fn user_by_login(&self, login: &str) -> Result<Option<serde_json::Value>, String> {
        real_only();
        crate::services::slash_commands::user_by_login(login).await
    }

    async fn acquire(&self, channel: &str, text: &str) -> Result<Duration, Refusal> {
        real_only();
        acquire(channel, text).await
    }

    async fn send(
        &self,
        text: &str,
        login: &str,
        broadcaster_id: &str,
        sender_id: &str,
    ) -> Result<crate::services::chat_service::SendResult, crate::services::chat_service::SendError> {
        real_only();
        crate::services::chat_service::ChatService::send_message(text, None, Some(login), Some(broadcaster_id), Some(sender_id), None)
            .await
    }
}

/// Send one scripted line to a Twitch channel: wait for a slot, then send as
/// the signed-in account. `channel_id` saves a lookup when the caller knows
/// it.
pub async fn paced_send<I: PacedIo>(io: &I, channel: &str, channel_id: Option<String>, text: &str) -> Result<PacedSend, PacedFail> {
    let login = key(channel);
    if login.is_empty() {
        return Err(PacedFail::Before(Refusal::EmptyChannel.text()));
    }
    if text.trim().is_empty() {
        return Err(PacedFail::Before("Nothing to send".into()));
    }
    let sender_id = io.sender_id().await.map_err(PacedFail::Before)?;
    let broadcaster_id = match channel_id.map(|id| id.trim().to_string()).filter(|id| !id.is_empty()) {
        Some(id) => id,
        None => io
            .user_by_login(&login)
            .await
            .map_err(PacedFail::Before)?
            .and_then(|u| u["id"].as_str().map(str::to_string))
            .ok_or_else(|| PacedFail::Before(format!("There is no Twitch channel named {login}")))?,
    };
    io.acquire(&login, text).await.map_err(|r| PacedFail::Before(r.text()))?;
    let sent = io.send(text, &login, &broadcaster_id, &sender_id).await.map_err(|e| {
        if e.may_have_left {
            PacedFail::After(e.to_string())
        } else {
            PacedFail::Before(e.to_string())
        }
    })?;
    Ok(PacedSend { is_sent: sent.is_sent, drop_reason: sent.drop_reason })
}

/// Send one scripted line (`/timer`, `/chain`, `/foreach`, `/runin`) to a
/// Twitch channel: wait for a slot, then send as the signed-in account. Plain
/// text goes over Helix, which needs no joined chat, so a timer still posts
/// after its chat was closed or switched away from. `channel_id` saves a
/// lookup when the caller knows it. Errors carry the refusal in words.
#[tauri::command]
pub async fn chat_send_paced(channel: String, channel_id: Option<String>, text: String) -> Result<PacedSend, String> {
    paced_send(&RealPacedIo, &channel, channel_id, &text).await.map_err(|f| fail_text(&f))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lim() -> Limiter {
        Limiter::default()
    }

    #[test]
    fn role_from_badges() {
        assert_eq!(Role::from_badges("broadcaster/1,subscriber/0"), Role::Broadcaster);
        assert_eq!(Role::from_badges("moderator/1"), Role::Moderator);
        assert_eq!(Role::from_badges("lead_moderator/1"), Role::Moderator);
        assert_eq!(Role::from_badges("vip/1,subscriber/12"), Role::Vip);
        assert_eq!(Role::from_badges("subscriber/12,premium/1"), Role::Viewer);
        assert_eq!(Role::from_badges(""), Role::Viewer);
        // A badge whose name only starts with a role name is not that role.
        assert_eq!(Role::from_badges("vipers/1"), Role::Viewer);
    }

    #[test]
    fn viewer_gets_one_per_second_per_channel() {
        let mut l = lim();
        let now = Instant::now();
        assert_eq!(l.book("chan", "a", now).unwrap(), Duration::ZERO);
        assert_eq!(l.book("chan", "b", now).unwrap(), VIEWER_SPACING);
        assert_eq!(l.book("chan", "c", now).unwrap(), VIEWER_SPACING * 2);
        // Another channel is not spaced against this one.
        assert_eq!(l.book("other", "a", now).unwrap(), Duration::ZERO);
    }

    #[test]
    fn privileged_goes_faster() {
        let mut l = lim();
        note_role(&mut l, "chan", "moderator/1");
        let now = Instant::now();
        l.book("chan", "a", now).unwrap();
        assert_eq!(l.book("chan", "a", now).unwrap(), PRIVILEGED_SPACING);
    }

    fn note_role(l: &mut Limiter, channel: &str, badges: &str) {
        let f = l.facts(channel);
        f.role = Some(Role::from_badges(badges));
        f.viewer_is_sub = badges.contains("subscriber/");
    }

    #[test]
    fn viewer_window_caps_at_twenty_per_thirty_seconds() {
        let mut l = lim();
        let now = Instant::now();
        // Twenty sends in twenty channels at once fill the window.
        for i in 0..VIEWER_PER_WINDOW {
            assert_eq!(l.book(&format!("c{i}"), "x", now).unwrap(), Duration::ZERO);
        }
        // The 21st waits until the first leaves the window.
        assert_eq!(l.book("c99", "x", now).unwrap(), WINDOW);
    }

    #[test]
    fn privileged_window_is_one_hundred() {
        let mut l = lim();
        let now = Instant::now();
        for i in 0..VIEWER_PER_WINDOW {
            l.book(&format!("c{i}"), "x", now).unwrap();
        }
        note_role(&mut l, "mine", "broadcaster/1");
        assert_eq!(l.book("mine", "x", now).unwrap(), Duration::ZERO);
    }

    #[test]
    fn slow_mode_stretches_a_viewers_spacing_only() {
        let mut l = lim();
        l.facts("chan").modes.slow_seconds = Some(10);
        let now = Instant::now();
        l.book("chan", "a", now).unwrap();
        assert_eq!(l.book("chan", "b", now).unwrap(), Duration::from_millis(10_250));
        note_role(&mut l, "vipchan", "vip/1");
        l.facts("vipchan").modes.slow_seconds = Some(10);
        l.book("vipchan", "a", now).unwrap();
        assert_eq!(l.book("vipchan", "b", now).unwrap(), PRIVILEGED_SPACING);
    }

    #[test]
    fn room_modes_refuse_viewers() {
        let mut l = lim();
        let now = Instant::now();
        l.facts("subs").modes.subs_only = Some(true);
        assert_eq!(l.book("subs", "hi", now), Err(Refusal::SubsOnly));
        note_role(&mut l, "subs", "subscriber/3");
        assert!(l.book("subs", "hi", now).is_ok());
        l.facts("emotes").modes.emote_only = Some(true);
        assert_eq!(l.book("emotes", "hi", now), Err(Refusal::EmoteOnly));
        note_role(&mut l, "emotes", "moderator/1");
        assert!(l.book("emotes", "hi", now).is_ok());
    }

    #[test]
    fn a_sub_seen_in_chat_opens_sub_only_chat() {
        let mut l = lim();
        let now = Instant::now();
        note_role(&mut l, "subs", "");
        l.facts("subs").modes.subs_only = Some(true);
        l.facts("subs").room_id = Some("42".into());
        l.facts("other").room_id = Some("7".into());
        assert_eq!(l.book("subs", "hi", now), Err(Refusal::SubsOnly));
        assert_eq!(l.own_sub("42"), vec!["subs".to_string()]);
        assert!(l.book("subs", "hi", now).is_ok());
        assert!(matches!(l.standing("subs"), Standing::Viewer { is_sub: true }));
        assert!(!l.channels["other"].viewer_is_sub);
        // Already known: nothing to resend.
        assert!(l.own_sub("42").is_empty());
    }

    #[test]
    fn a_sub_seen_before_the_room_id_counts_once_the_id_arrives() {
        let mut l = lim();
        let now = Instant::now();
        // The sub shows before this chat's ROOMSTATE named its room.
        assert!(l.own_sub("42").is_empty());
        assert!(l.own_badges("subs", ""));
        l.facts("subs").modes.subs_only = Some(true);
        assert_eq!(l.book("subs", "hi", now), Err(Refusal::SubsOnly));
        assert!(l.room_id("subs", "42"));
        assert!(l.book("subs", "hi", now).is_ok());
        // A later USERSTATE whose badges still lag the sub keeps it.
        assert!(!l.own_badges("subs", "premium/1"));
        assert!(l.channels["subs"].viewer_is_sub);
        // Another room is untouched.
        assert!(!l.room_id("other", "7"));
        assert!(!l.channels["other"].viewer_is_sub);
    }

    #[test]
    fn duplicate_refused_for_viewers_within_thirty_seconds() {
        let mut l = lim();
        let now = Instant::now();
        l.book("chan", "same", now).unwrap();
        assert_eq!(l.book("chan", "same", now), Err(Refusal::Duplicate));
        assert!(l.book("chan", "same \u{E0000}", now).is_ok());
        // Long after, the same text is fine again.
        let later = now + Duration::from_secs(40);
        assert!(l.book("chan", "same", later).is_ok());
    }

    #[test]
    fn r9k_refuses_repeats_at_any_age() {
        let mut l = lim();
        l.facts("chan").modes.r9k = Some(true);
        let now = Instant::now();
        l.book("chan", "same", now).unwrap();
        assert_eq!(l.book("chan", "same", now + Duration::from_secs(90)), Err(Refusal::Duplicate));
    }

    #[test]
    fn recorded_sends_count_against_the_window() {
        let mut l = lim();
        let now = Instant::now();
        l.record("chan", "typed", now);
        // A paced send right after a typed one waits the per-channel gap.
        assert_eq!(l.book("chan", "paced", now).unwrap(), VIEWER_SPACING);
    }

    #[test]
    fn a_booked_slot_is_claimed_not_double_counted() {
        let mut l = lim();
        let now = Instant::now();
        l.book("chan", "a", now).unwrap();
        l.record("chan", "a", now);
        assert_eq!(l.sends.len(), 1);
        assert!(l.channels["chan"].pending.is_empty());
    }

    #[test]
    fn a_wait_past_the_cap_is_refused() {
        let mut l = lim();
        let now = Instant::now();
        l.facts("chan").modes.slow_seconds = Some(200);
        l.book("chan", "a", now).unwrap();
        assert!(matches!(l.book("chan", "b", now), Err(Refusal::TooLong { .. })));
    }

    #[test]
    fn the_slowest_slow_mode_waits_instead_of_refusing() {
        let mut l = lim();
        let now = Instant::now();
        l.facts("chan").modes.slow_seconds = Some(120);
        l.book("chan", "a", now).unwrap();
        assert_eq!(l.book("chan", "b", now).unwrap(), Duration::from_millis(120_250));
    }

    /// The most sends inside any one 30 s window, over every booked instant.
    fn fullest_window(l: &Limiter) -> usize {
        l.sends.iter().map(|(end, _)| l.in_window(*end)).max().unwrap_or(0)
    }

    #[test]
    fn a_new_booking_never_overfills_a_window_that_ends_later() {
        let mut l = lim();
        let now = Instant::now();
        // Twenty lines paced ahead in one channel: 0, 1.1 ... 20.9 s.
        for i in 0..VIEWER_PER_WINDOW {
            l.book("a", &format!("line {i}"), now).unwrap();
        }
        // A line in another channel may not join that window: it waits until
        // the first of the twenty has left it.
        assert_eq!(l.book("b", "other", now).unwrap(), WINDOW);
        assert!(fullest_window(&l) <= VIEWER_PER_WINDOW, "{}", fullest_window(&l));
    }

    #[test]
    fn a_privileged_booking_respects_a_viewer_send_booked_later() {
        let mut l = lim();
        let now = Instant::now();
        for i in 0..VIEWER_PER_WINDOW {
            l.book("viewerchan", &format!("line {i}"), now).unwrap();
        }
        note_role(&mut l, "mine", "broadcaster/1");
        // The broadcaster's own channel has room for 100, but going now would
        // put a 21st send in the window of the last viewer line.
        let wait = l.book("mine", "x", now).unwrap();
        assert!(wait > VIEWER_SPACING * 19, "{wait:?}");
        for (end, budget) in &l.sends {
            assert!(l.in_window(*end) <= *budget);
        }
    }

    #[test]
    fn known_role_waits_for_userstate_and_the_right_account() {
        let mut l = lim();
        l.own_login = Some("me".into());
        assert_eq!(l.known_role("me", "chan"), None);
        l.facts("chan").role = Some(Role::Moderator);
        assert_eq!(l.known_role("Me", "#Chan"), Some(Role::Moderator));
        assert_eq!(l.known_role("someoneelse", "chan"), None);
    }

    #[test]
    fn own_channel_counts_as_broadcaster_before_userstate() {
        let mut l = lim();
        l.own_login = Some("me".into());
        assert_eq!(l.role_of("#Me"), Role::Broadcaster);
        assert_eq!(l.role_of("someone"), Role::Viewer);
    }

    #[test]
    fn budget_reports_modes_and_use() {
        let mut l = lim();
        let now = Instant::now();
        l.facts("chan").modes = RoomModes {
            slow_seconds: Some(5),
            followers_only_minutes: Some(10),
            subs_only: Some(false),
            emote_only: Some(false),
            r9k: Some(true),
        };
        l.record("chan", "x", now);
        let b = l.budget("chan", now);
        assert_eq!(b.role, Role::Viewer);
        assert_eq!(b.per_window, VIEWER_PER_WINDOW);
        assert_eq!(b.used_in_window, 1);
        assert_eq!(b.slow_seconds, 5);
        assert_eq!(b.followers_only_minutes, 10);
        assert!(b.r9k);
    }

    #[test]
    fn room_fields_merge_partial_frames_and_word_them_for_the_account() {
        let mut l = lim();
        assert!(l.room_fields("chan").is_none(), "nothing before a ROOMSTATE");
        // The full frame on join, then a partial one turning slow mode on.
        l.facts("chan").modes = RoomModes {
            slow_seconds: Some(0),
            followers_only_minutes: Some(10),
            subs_only: Some(false),
            emote_only: Some(false),
            r9k: Some(false),
        };
        l.facts("chan").modes.slow_seconds = Some(30);
        let f = l.room_fields("#Chan").unwrap();
        assert_eq!(f["followers_only"], 10);
        assert_eq!(f["slow"], 30);
        assert_eq!(f["r9k"], false);
        let modes = &f["modes"];
        assert_eq!(modes["chip"], "Followers 10m");
        assert_eq!(modes["exempt"], false);
        assert_eq!(modes["prompt"], "Followers-only (10m)");
        assert_eq!(modes["prompt_short"], "Followers 10m");
        assert_eq!(modes["modes"][0]["kind"], "followers_only");
        assert_eq!(modes["modes"][1]["kind"], "slow");
        assert_eq!(modes["modes"][0]["applies_to_you"], serde_json::Value::Null, "role unknown before USERSTATE");
        assert_eq!(modes["slow_seconds"], 30);

        note_role(&mut l, "chan", "moderator/1");
        let f = l.room_fields("chan").unwrap();
        assert_eq!(f["modes"]["standing"], "You can chat as a moderator");
        assert_eq!(f["modes"]["prompt"], serde_json::Value::Null);
        assert_eq!(f["modes"]["slow_seconds"], 0);

        note_role(&mut l, "chan", "subscriber/3");
        let f = l.room_fields("chan").unwrap();
        assert_eq!(f["modes"]["modes"][1]["applies_to_you"], true, "slow mode holds a viewer back");
        assert_eq!(f["modes"]["slow_seconds"], 30);
    }

    // ---- automations' bookings ----------------------------------------------

    fn base() -> Instant {
        Instant::now() + Duration::from_secs(100)
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn engine(l: &mut Limiter, channel: &str, text: &str, now: Instant) -> Booked {
        l.book_engine(channel, text, now, MAX_WAIT).expect("a slot")
    }

    /// The most entries in any window ending at an entry.
    fn fullest(l: &Limiter) -> usize {
        l.sends.iter().map(|(end, _)| l.in_window(*end)).max().unwrap_or(0)
    }

    fn lockstep(l: &Limiter) {
        assert_eq!(l.sends.len(), l.send_ids.len(), "ids stay in lockstep with entries");
    }

    #[test]
    fn the_engine_constants_add_up() {
        assert_eq!(ENGINE_DRIFT, REBOOK_TOLERANCE + ENGINE_SEND_TIMEOUT);
        assert_eq!(ENGINE_WINDOW, WINDOW + ENGINE_DRIFT);
        assert_eq!(ENGINE_WINDOW, Duration::from_millis(35_250));
        assert_eq!(ENGINE_SHARE, 12);
        assert_eq!(
            Refusal::ShareFull { wait_secs: 7 }.text(),
            "Automations' share of the message limit is full for 7 seconds"
        );
    }

    #[test]
    fn twelve_automation_sends_then_the_thirteenth_waits() {
        let mut l = lim();
        let t = base();
        let booked: Vec<Booked> = (0..ENGINE_SHARE).map(|i| engine(&mut l, "chan", &format!("line {i}"), t)).collect();
        assert_eq!(booked[1].at - booked[0].at, VIEWER_SPACING + REBOOK_TOLERANCE);
        let thirteenth = engine(&mut l, "chan", "line 12", t);
        assert_eq!(thirteenth.at, t + ENGINE_WINDOW);
        // Its own entry is left out of its count.
        assert_eq!(l.in_window_except(booked[0].at, Some(booked[0].id)), 0);
        assert!(l.fits_engine(&booked[0], booked[0].at));
        // Past the rule's wait, it refuses in the automations' own words and
        // books nothing.
        let before = l.sends.len();
        assert_eq!(l.book_engine("other", "x", t + ms(15_000), Duration::from_secs(10)).unwrap_err(), Refusal::ShareFull { wait_secs: 22 });
        assert_eq!(l.sends.len(), before);
        lockstep(&l);
    }

    #[test]
    fn a_refused_booking_books_nothing() {
        let mut l = lim();
        let t = base();
        engine(&mut l, "chan", "same", t);
        let (sends, pending) = (l.sends.len(), l.channels["chan"].pending.len());
        assert_eq!(l.book_engine("chan", "same", t, MAX_WAIT).unwrap_err(), Refusal::Duplicate);
        l.facts("slow").modes.slow_seconds = Some(60);
        engine(&mut l, "slow", "a", t);
        assert!(matches!(l.book_engine("slow", "b", t, Duration::from_secs(10)), Err(Refusal::TooLong { .. })));
        assert_eq!(l.sends.len(), sends + 1);
        assert_eq!(l.channels["chan"].pending.len(), pending);
        lockstep(&l);
    }

    #[test]
    fn a_privileged_chat_still_stops_automations_at_twelve() {
        let mut l = lim();
        let t = base();
        for i in 0..ENGINE_SHARE {
            note_role(&mut l, &format!("m{i}"), "moderator/1");
            engine(&mut l, &format!("m{i}"), "x", t);
        }
        note_role(&mut l, "mine", "broadcaster/1");
        assert_eq!(engine(&mut l, "mine", "x", t).at, t + ENGINE_WINDOW);
    }

    #[test]
    fn a_typed_message_counts_against_the_twelve() {
        let mut l = lim();
        let t = base();
        for i in 0..ENGINE_SHARE - 1 {
            engine(&mut l, &format!("e{i}"), "x", t);
        }
        l.record("typed", "hi", t);
        // The typed entry leaves first, after Twitch's 30 s.
        assert_eq!(engine(&mut l, "late", "x", t).at, t + WINDOW);
    }

    #[test]
    fn later_automation_entries_keep_their_window_at_twelve() {
        let mut l = lim();
        let t = base();
        for i in 0..ENGINE_SHARE {
            engine(&mut l, &format!("ahead{i}"), "x", t + ms(20_000 + 800 * i as u64));
        }
        for i in 0..5 {
            engine(&mut l, &format!("now{i}"), "x", t);
        }
        assert!(fullest(&l) <= ENGINE_SHARE, "{}", fullest(&l));
        let mut l = lim();
        engine(&mut l, "ahead", "x", t + ms(19_250));
        for i in 0..ENGINE_SHARE {
            engine(&mut l, &format!("now{i}"), "x", t);
        }
        assert!(fullest(&l) <= ENGINE_SHARE, "{}", fullest(&l));
        lockstep(&l);
    }

    #[test]
    fn a_plain_entry_booked_ahead_is_never_pushed_past_its_budget() {
        // A script line at t+25 whose window holds 19, counting itself.
        let mut l = lim();
        let t = base();
        for i in 1..=18 {
            l.insert_send(t + ms(1_000 * i as u64), VIEWER_PER_WINDOW);
        }
        l.insert_send(t + ms(25_000), VIEWER_PER_WINDOW);
        engine(&mut l, "e1", "x", t);
        engine(&mut l, "e2", "x", t);
        for (i, (end, budget)) in l.sends.iter().enumerate() {
            if l.send_ids[i].is_none() {
                assert!(l.in_window(*end) <= *budget, "{} at {i}", l.in_window(*end));
            }
        }
        // A plain entry at t+31 whose window holds 20, counting itself: the
        // automation's entry would count there up to 35.25 s ahead.
        let mut l = lim();
        for i in 0..19 {
            l.insert_send(t + ms(2_000 + 1_400 * i as u64), VIEWER_PER_WINDOW);
        }
        l.insert_send(t + ms(31_000), VIEWER_PER_WINDOW);
        assert_eq!(l.in_window(t + ms(31_000)), VIEWER_PER_WINDOW);
        let booked = engine(&mut l, "e", "x", t);
        assert!(booked.at > t + ms(31_000));
        assert!(l.in_window(t + ms(31_000)) <= VIEWER_PER_WINDOW);
    }

    #[test]
    fn a_script_line_waits_behind_late_automation_entries_only_as_long_as_they_count() {
        let mut l = lim();
        let t0 = base();
        for i in 0..ENGINE_SHARE {
            engine(&mut l, &format!("e{i}"), "x", t0);
        }
        for i in 0..8 {
            l.record(&format!("p{i}"), "typed", t0 + ms(20_000));
        }
        // Today's oldest-plus-30 jump would land it at t0+50.
        let found = l.search("script", t0 + ms(31_000), Booker::Plain);
        assert_eq!(found.at, t0 + ENGINE_WINDOW);
        assert!(found.passes <= 3, "{}", found.passes);
        assert_eq!(l.book("script", "line", t0 + ms(31_000)), Ok(Duration::from_millis(4_250)));
    }

    #[test]
    fn a_full_window_ends_in_a_slot_or_a_refusal_quickly() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut l = Limiter::default();
            let t0 = base();
            for i in 0..ENGINE_SHARE {
                l.book_engine(&format!("e{i}"), "x", t0, MAX_WAIT).unwrap();
            }
            for i in 0..8 {
                l.record(&format!("p{i}"), "typed", t0 + ms(20_000));
            }
            let script = l.book("script", "line", t0 + ms(31_000));
            let passes = l.search("other", t0 + ms(31_000), Booker::Plain).passes;
            let refused = l.book_engine("late", "x", t0 + ms(31_000), Duration::ZERO).map(|b| b.at);
            let _ = tx.send((script, passes, refused));
        });
        let (script, passes, refused) = rx.recv_timeout(Duration::from_secs(1)).expect("the search ends");
        assert_eq!(script, Ok(Duration::from_millis(4_250)));
        assert!(passes <= 4, "{passes}");
        assert!(matches!(refused, Err(Refusal::TooLong { .. })), "Twitch's own 20 held it, not the automations' 12");
    }

    #[test]
    fn automation_entries_count_for_the_longer_window() {
        let mut l = lim();
        let t0 = base();
        for i in 0..ENGINE_SHARE {
            engine(&mut l, &format!("e{i}"), "x", t0);
        }
        assert_eq!(engine(&mut l, "late", "x", t0 + ms(30_200)).wait, Duration::from_millis(5_050));
        let mut l = lim();
        engine(&mut l, "e", "x", t0);
        l.record("p", "typed", t0);
        l.prune(t0 + ms(30_500));
        assert_eq!(l.sends.len(), 1);
        assert!(l.send_ids[0].is_some(), "the automation's entry stays");
        assert_eq!(l.budget("e", t0 + ms(32_000)).used_in_window, 1);
        l.prune(t0 + ENGINE_WINDOW + Duration::from_millis(1));
        assert!(l.sends.is_empty() && l.send_ids.is_empty());
    }

    #[test]
    fn an_entry_is_found_by_its_id_never_its_instant() {
        let mut l = lim();
        let t = base();
        let a = engine(&mut l, "a", "x", t);
        let b = engine(&mut l, "b", "x", t);
        l.record("c", "typed", t);
        assert_eq!((a.at, b.at), (t, t));
        assert_eq!(l.in_window(t), 3);
        assert_eq!(l.in_window_except(t, Some(a.id)), 2);
        l.cancel(&a);
        assert_eq!(l.sends.len(), 2);
        assert!(l.send_ids.contains(&Some(b.id)) && l.send_ids.contains(&None) && !l.send_ids.contains(&Some(a.id)));
        lockstep(&l);
        // A booking that lands before an existing entry is inserted in the middle.
        let mut l = lim();
        l.record("p", "typed", t + ms(20_000));
        let first = engine(&mut l, "e1", "x", t);
        let second = engine(&mut l, "e2", "x", t + ms(5_000));
        l.cancel(&first);
        let pairs: Vec<(Instant, Option<u64>)> = l.sends.iter().map(|(at, _)| *at).zip(l.send_ids.iter().copied()).collect();
        assert_eq!(pairs, vec![(t + ms(5_000), Some(second.id)), (t + ms(20_000), None)]);
        assert_eq!(l.in_window(t + ms(5_000)), 1);
        assert_eq!(l.in_window_except(t + ms(5_000), Some(second.id)), 0);
    }

    #[test]
    fn a_recorded_automation_entry_keeps_its_id_until_it_leaves() {
        let mut l = lim();
        let t = base();
        let b = engine(&mut l, "chan", "x", t);
        l.mark(&b, t);
        l.record_booked(&b, t);
        assert_eq!(l.sends.len(), 1, "no second entry");
        assert_eq!(l.in_window(t + ms(32_000)), 1, "still counted at 32 s");
        assert!(l.channels["chan"].pending.is_empty());
        l.prune(t + ENGINE_WINDOW);
        assert!(l.sends.is_empty() && l.send_ids.is_empty());
    }

    #[test]
    fn a_held_booking_survives_prune_and_counts_once_at_its_slot() {
        let mut l = lim();
        let t = base();
        let b = engine(&mut l, "chan", "x", t);
        let plain = l.book("other", "y", t).unwrap();
        assert_eq!(plain, Duration::ZERO);
        l.prune(t + ms(12_000));
        assert_eq!(l.channels["chan"].pending.len(), 1, "an automation's slot is never dropped by age");
        assert!(l.channels["other"].pending.is_empty(), "an untracked slot still drops after 10 s");
        l.record_booked(&b, t + ms(12_000));
        assert_eq!(l.sends.iter().filter(|(at, _)| *at == t).count(), 2);
        assert!(l.send_ids.contains(&Some(b.id)));
        // A typed record never takes an automation's slot.
        let mut l = lim();
        let c = engine(&mut l, "chan", "auto", t);
        l.record("chan", "typed", t);
        assert_eq!(l.channels["chan"].pending.iter().copied().collect::<Vec<_>>(), vec![(t, Some(c.id))]);
        assert_eq!(l.sends.len(), 2);
    }

    fn cancel_cases(mut l: Limiter) {
        let t = base();
        // Book A then B; cancel A: B keeps its slot and its duplicate check.
        let a = engine(&mut l, "chan", "a-text", t);
        let b = engine(&mut l, "chan", "b-text", t);
        l.cancel(&a);
        assert_eq!(l.book_engine("chan", "b-text", t, MAX_WAIT).unwrap_err(), Refusal::Duplicate);
        let c = engine(&mut l, "chan", "c-text", t);
        assert_eq!(c.at, b.at + VIEWER_SPACING + REBOOK_TOLERANCE);
        // Book A, book B, B sends, cancel A: the next slot follows B's send.
        let mut fresh = Limiter { is_alt: l.is_alt, ..Limiter::default() };
        let a = engine(&mut fresh, "chan", "a", t);
        let b = engine(&mut fresh, "chan", "b", t);
        fresh.mark(&b, b.at);
        fresh.record_booked(&b, b.at);
        fresh.cancel(&a);
        assert_eq!(fresh.channels["chan"].last_slot, Some(b.at));
        assert_eq!(engine(&mut fresh, "chan", "c", t).at, b.at + VIEWER_SPACING + REBOOK_TOLERANCE);
        // With nothing booked after it, a cancel leaves what was there before.
        let mut alone = Limiter { is_alt: l.is_alt, ..Limiter::default() };
        alone.record("chan", "typed", t);
        let a = engine(&mut alone, "chan", "auto", t + ms(5_000));
        alone.cancel(&a);
        let facts = &alone.channels["chan"];
        assert_eq!(facts.last_slot, Some(t));
        assert!(!facts.last_slot_ticketed);
        assert_eq!(facts.last_text, Some(("typed".to_string(), t)));
        assert_eq!(alone.sends.len(), 1);
        lockstep(&alone);
    }

    #[test]
    fn a_cancel_never_undoes_a_later_booking() {
        cancel_cases(lim());
        cancel_cases(Limiter::alt(None));
    }

    #[test]
    fn a_linked_account_skips_sub_only_and_emote_only() {
        let t = base();
        let mut main = lim();
        main.facts("subs").modes.subs_only = Some(true);
        main.facts("emotes").modes.emote_only = Some(true);
        assert_eq!(main.book_engine("subs", "x", t, MAX_WAIT).unwrap_err(), Refusal::SubsOnly);
        assert_eq!(main.book_engine("emotes", "x", t, MAX_WAIT).unwrap_err(), Refusal::EmoteOnly);
        let mut alt = Limiter::alt(Some("alt".into()));
        alt.facts("subs").modes.subs_only = Some(true);
        alt.facts("emotes").modes.emote_only = Some(true);
        assert!(alt.book_engine("subs", "x", t, MAX_WAIT).is_ok());
        assert!(alt.book_engine("emotes", "x", t, MAX_WAIT).is_ok());
        assert_eq!(alt.role_of("alt"), Role::Broadcaster, "its own channel");
        assert!(alt.own_badges("chan", "vip/1"));
        assert_eq!(alt.role_of("chan"), Role::Vip);
    }

    #[test]
    fn a_booking_from_a_replaced_limiter_touches_nothing() {
        let t = base();
        let mut old = lim();
        let b = engine(&mut old, "chan", "x", t);
        let mut new = lim();
        assert_ne!(old.gen, new.gen, "every limiter draws its own generation");
        new.record("chan", "typed", t);
        let before = (new.channels["chan"].last_slot, new.channels["chan"].last_text.clone(), new.sends.len());
        new.cancel(&b);
        new.retext_booked(&b, "other");
        new.mark(&b, t);
        new.record_booked(&b, t);
        assert!(!new.fits_engine(&b, t));
        assert!(!new.spacing_moved(&b, t));
        assert_eq!((new.channels["chan"].last_slot, new.channels["chan"].last_text.clone(), new.sends.len()), before);
    }

    #[test]
    fn last_sent_only_moves_forward() {
        let mut l = lim();
        let t = base();
        let b = engine(&mut l, "chan", "auto", t);
        l.mark(&b, t);
        l.record("chan", "typed", t + ms(500));
        l.record_booked(&b, t);
        let facts = &l.channels["chan"];
        assert_eq!(facts.last_sent, Some(t + ms(500)));
        assert_eq!(facts.last_sent_by, None);
        assert_eq!(engine(&mut l, "chan", "next", t + ms(800)).at, t + ms(500) + VIEWER_SPACING);
    }

    #[test]
    fn two_automation_sends_in_one_chat_keep_their_spacing_when_the_first_is_late() {
        let t = base();
        for (slow, gap) in [(None, VIEWER_SPACING), (Some(10), Duration::from_millis(10_500))] {
            let mut l = lim();
            l.facts("chan").modes.slow_seconds = slow;
            let a = engine(&mut l, "chan", "a", t);
            let started = a.at + Duration::from_millis(240);
            l.mark(&a, started);
            l.record_booked(&a, started);
            let b = engine(&mut l, "chan", "b", t + ms(300));
            assert!(b.at - started >= gap, "{:?}", b.at - started);
        }
    }

    #[test]
    fn a_message_typed_during_the_wait_moves_the_spacing() {
        let mut l = lim();
        let t = base();
        l.record("chan", "earlier", t);
        let a = engine(&mut l, "chan", "auto", t);
        assert_eq!(a.at, t + VIEWER_SPACING);
        assert!(l.fits_engine(&a, a.at));
        l.record("chan", "typed", t + ms(800));
        assert!(l.spacing_moved(&a, a.at));
        assert!(!l.fits_engine(&a, a.at));
        l.cancel(&a);
        let again = engine(&mut l, "chan", "auto", a.at);
        assert_eq!(again.at, t + ms(800) + VIEWER_SPACING);
        assert!(l.fits_engine(&again, again.at));
        // A retext keeps the duplicate check on the text that goes out.
        l.retext_booked(&again, "auto, changed");
        assert_eq!(l.channels["chan"].last_text, Some(("auto, changed".to_string(), again.at)));
    }

    /// A small fixed-seed generator, so every run sees the same draws.
    struct Draws(u64);

    impl Draws {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }

    /// The most arrivals inside any `span`.
    fn most_in_any(arrivals: &mut [Instant], span: Duration) -> usize {
        arrivals.sort();
        let (mut most, mut j) = (0, 0);
        for i in 0..arrivals.len() {
            while arrivals[i].duration_since(arrivals[j]) >= span {
                j += 1;
            }
            most = most.max(i - j + 1);
        }
        most
    }

    #[test]
    fn automation_sends_arriving_late_never_pass_twelve_in_a_twitch_window() {
        for seed in 0..2000u64 {
            let mut draws = Draws(seed.wrapping_mul(2_654_435_761).wrapping_add(1));
            let mut l = lim();
            let mut now = base();
            let mut arrivals = Vec::new();
            for step in 0..40 {
                now += Duration::from_millis(draws.below(4_000));
                for k in 0..draws.below(5) {
                    let channel = format!("c{}", draws.below(6));
                    if let Ok(b) = l.book_engine(&channel, &format!("s{step} k{k}"), now, MAX_WAIT) {
                        arrivals.push(b.at + Duration::from_millis(draws.below(5_251)));
                    }
                }
            }
            assert!(most_in_any(&mut arrivals, WINDOW) <= ENGINE_SHARE, "seed {seed}");
        }
        // The first batch arrives as late as it can, the second on time.
        let mut l = lim();
        let t = base();
        let mut arrivals: Vec<Instant> = (0..ENGINE_SHARE).map(|i| engine(&mut l, &format!("a{i}"), "x", t).at + ENGINE_DRIFT).collect();
        arrivals.extend((0..ENGINE_SHARE).map(|i| engine(&mut l, &format!("b{i}"), "x", t).at));
        assert!(most_in_any(&mut arrivals, WINDOW) <= ENGINE_SHARE);
    }

    #[test]
    fn a_script_run_after_late_automation_sends_never_passes_twenty() {
        let mut l = lim();
        let t = base();
        let mut arrivals: Vec<Instant> =
            (0..ENGINE_SHARE).map(|i| engine(&mut l, &format!("e{i}"), "x", t).at + Duration::from_secs(5)).collect();
        let run = t + WINDOW;
        for i in 0..20 {
            let wait = l.book("run", &format!("line {i}"), run).unwrap();
            arrivals.push(run + wait);
        }
        assert!(most_in_any(&mut arrivals, WINDOW) <= VIEWER_PER_WINDOW);
    }

    // ---- tickets on the real limiters (each test its own linked account) ----

    fn alt(id: &str) -> LimiterKey {
        LimiterKey::Alt(id.to_string())
    }

    #[test]
    fn a_dropped_ticket_cancels_and_one_dropped_while_sending_records() {
        let (_, ticket) = acquire_within_on(alt("t-drop-cancel"), "chan", "hi", MAX_WAIT).unwrap();
        assert_eq!(alt_entries("t-drop-cancel"), (1, 1));
        drop(ticket);
        assert_eq!(alt_entries("t-drop-cancel"), (0, 0));
        let (_, mut ticket) = acquire_within_on(alt("t-drop-record"), "chan", "hi", MAX_WAIT).unwrap();
        mark_sending(&mut ticket);
        drop(ticket);
        assert_eq!(alt_entries("t-drop-record"), (1, 0), "counted at its slot, no longer pending");
        assert!(alt_limiters()["t-drop-record"].channels["chan"].last_sent.is_some());
    }

    #[test]
    fn a_refused_send_frees_its_slot_and_its_text() {
        let (_, mut ticket) = acquire_within_on(alt("t-refused"), "chan", "hi", MAX_WAIT).unwrap();
        mark_sending(&mut ticket);
        cancel_booking(&mut ticket);
        assert!(ticket.is_settled());
        let (wait, again) = acquire_within_on(alt("t-refused"), "chan", "hi", MAX_WAIT).expect("not a duplicate");
        assert_eq!(wait, Duration::ZERO, "no spacing from a send that never posted");
        drop(again);
        assert_eq!(alt_entries("t-refused"), (0, 0));
    }

    #[test]
    fn a_ticket_settles_once() {
        let (_, mut ticket) = acquire_within_on(alt("t-once"), "chan", "hi", MAX_WAIT).unwrap();
        assert!(window_fits_engine(&ticket, ticket.at()));
        retext(&mut ticket, "hi there");
        assert_eq!(ticket.text(), "hi there");
        mark_sending(&mut ticket);
        record_ticket(&mut ticket);
        cancel_booking(&mut ticket);
        assert!(!window_fits_engine(&ticket, ticket.at()));
        drop(ticket);
        assert_eq!(alt_entries("t-once"), (1, 0), "the cancel after the record changed nothing");
    }

    #[test]
    fn each_linked_account_has_its_own_limiter() {
        for i in 0..VIEWER_PER_WINDOW {
            record_on(&alt("t-iso-a"), &format!("iso-c{i}"), "x");
        }
        let (wait, a) = acquire_within_on(alt("t-iso-a"), "iso-c99", "y", MAX_WAIT).unwrap();
        assert!(wait > Duration::from_secs(25), "{wait:?}");
        let (wait, b) = acquire_within_on(alt("t-iso-b"), "iso-c99", "y", MAX_WAIT).unwrap();
        assert_eq!(wait, Duration::ZERO);
        assert!(limiter().channels.get("iso-c0").is_none(), "the main account's limiter saw none of it");
        drop((a, b));
    }

    #[test]
    fn no_ticket_is_dropped_under_a_limiter_lock() {
        let src = include_str!("chat_send_limiter.rs").replace("\r\n", "\n");
        let code = src.split("#[cfg(test)]\nmod tests {").next().unwrap();
        assert!(code.contains("impl Drop for Ticket"));
        // The one place a ticket is made is after the lock was let go.
        assert_eq!(code.matches("Ticket::new(").count(), 1);
        assert!(code.contains("    })?;\n    let wait = booked.wait;\n    Ok((wait, Ticket::new(account, booked)))"));
        // What runs under the lock never holds a ticket.
        let with = code.split("fn with_limiter<R>").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(!with.contains("Ticket"));
        let limiter_impl = code.split("impl Limiter {").nth(1).unwrap().split("\n}\n").next().unwrap();
        assert!(!limiter_impl.contains("Ticket"));
        // Every ticket operation borrows its ticket; none takes it by value.
        for op in [
            "pub fn mark_sending(ticket: &mut Ticket)",
            "pub fn record_ticket(ticket: &mut Ticket)",
            "pub fn cancel_booking(ticket: &mut Ticket)",
            "pub fn retext(ticket: &mut Ticket, text: &str)",
            "pub fn window_fits_engine(ticket: &Ticket, now: Instant) -> bool",
            "pub fn spacing_moved(ticket: &Ticket, now: Instant) -> bool",
        ] {
            assert!(code.contains(op), "{op}");
        }
    }

    // ---- the paced send through a fake --------------------------------------

    use crate::services::chat_service::{SendError, SendResult};
    use std::sync::atomic::AtomicUsize;

    struct FakeIo {
        sender: Result<String, String>,
        room: Result<Option<serde_json::Value>, String>,
        slot: Result<Duration, Refusal>,
        send: fn() -> Result<SendResult, SendError>,
        sent: AtomicUsize,
    }

    impl FakeIo {
        fn ok() -> FakeIo {
            FakeIo {
                sender: Ok("1".into()),
                room: Ok(Some(serde_json::json!({ "id": "42" }))),
                slot: Ok(Duration::ZERO),
                send: || Ok(SendResult { message_id: Some("m".into()), is_sent: true, drop_reason: None }),
                sent: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl PacedIo for FakeIo {
        async fn sender_id(&self) -> Result<String, String> {
            self.sender.clone()
        }
        async fn user_by_login(&self, _login: &str) -> Result<Option<serde_json::Value>, String> {
            self.room.clone()
        }
        async fn acquire(&self, _channel: &str, _text: &str) -> Result<Duration, Refusal> {
            self.slot.clone()
        }
        async fn send(&self, _text: &str, _login: &str, _broadcaster_id: &str, _sender_id: &str) -> Result<SendResult, SendError> {
            self.sent.fetch_add(1, Ordering::SeqCst);
            (self.send)()
        }
    }

    #[tokio::test]
    async fn every_step_before_the_send_fails_before() {
        let before = |t: &str| -> Result<PacedSend, PacedFail> { Err(PacedFail::Before(t.to_string())) };
        assert_eq!(paced_send(&FakeIo::ok(), "  ", None, "hi").await, before(&Refusal::EmptyChannel.text()));
        assert_eq!(paced_send(&FakeIo::ok(), "chan", None, "  ").await, before("Nothing to send"));
        let signed_out = FakeIo { sender: Err("Sign in to Twitch to send messages".into()), ..FakeIo::ok() };
        assert_eq!(paced_send(&signed_out, "chan", None, "hi").await, before("Sign in to Twitch to send messages"));
        let lookup = FakeIo { room: Err("Twitch didn't answer".into()), ..FakeIo::ok() };
        assert_eq!(paced_send(&lookup, "chan", None, "hi").await, before("Twitch didn't answer"));
        let missing = FakeIo { room: Ok(None), ..FakeIo::ok() };
        assert_eq!(paced_send(&missing, "chan", None, "hi").await, before("There is no Twitch channel named chan"));
        let refused = FakeIo { slot: Err(Refusal::Duplicate), ..FakeIo::ok() };
        assert_eq!(paced_send(&refused, "chan", None, "hi").await, before(&Refusal::Duplicate.text()));
        for io in [signed_out, lookup, missing, refused] {
            assert_eq!(io.sent.load(Ordering::SeqCst), 0);
        }
        // A known channel id skips the lookup.
        let known = FakeIo { room: Err("never asked".into()), ..FakeIo::ok() };
        assert_eq!(paced_send(&known, "chan", Some("42".into()), "hi").await, Ok(PacedSend { is_sent: true, drop_reason: None }));
    }

    #[tokio::test]
    async fn a_send_error_is_before_or_after_by_where_it_failed() {
        let early = FakeIo {
            send: || Err(SendError { source: anyhow::anyhow!("IRC connection not established"), may_have_left: false }),
            ..FakeIo::ok()
        };
        let early = paced_send(&early, "chan", None, "hi").await.unwrap_err();
        assert_eq!(early, PacedFail::Before("IRC connection not established".into()));
        assert!(release_after(&[Err(early.clone())]));
        let late = FakeIo {
            send: || Err(SendError { source: anyhow::anyhow!("Helix send returned HTTP 503: null"), may_have_left: true }),
            ..FakeIo::ok()
        };
        let late = paced_send(&late, "chan", None, "hi").await.unwrap_err();
        assert_eq!(late, PacedFail::After("Helix send returned HTTP 503: null".into()));
        assert!(!release_after(&[Err(late.clone())]));
        assert_eq!(fail_text(&early), "IRC connection not established");
        assert_eq!(fail_text(&PacedFail::After("same".into())), fail_text(&PacedFail::Before("same".into())));
    }

    #[test]
    fn a_post_lets_its_claim_go_only_when_nothing_can_have_posted() {
        let dropped = |reason: Option<&str>| Ok(PacedSend { is_sent: false, drop_reason: reason.map(str::to_string) });
        let sent = Ok(PacedSend { is_sent: true, drop_reason: None });
        assert!(release_after(&[]), "an early return");
        assert!(release_after(&[dropped(Some("msg_duplicate")), dropped(Some("Your message was not sent"))]));
        assert!(release_after(&[Err(PacedFail::Before("There is no Twitch channel named x".into()))]));
        assert!(!release_after(&[Err(PacedFail::After("timed out".into()))]));
        assert!(!release_after(&[dropped(Some("msg_duplicate")), sent.clone()]));
        assert!(!release_after(&[dropped(None)]), "no reason: it may have posted");
        assert!(!release_after(&[sent]));
    }

    #[tokio::test]
    #[should_panic(expected = "tests use a fake")]
    async fn the_real_paced_send_panics_in_tests() {
        let _ = RealPacedIo.sender_id().await;
    }

    #[test]
    fn the_own_channel_reads_as_the_broadcaster_in_room_words() {
        let mut l = lim();
        l.own_login = Some("me".into());
        l.facts("me").modes.emote_only = Some(true);
        let f = l.room_fields("me").unwrap();
        assert_eq!(f["modes"]["standing"], "You can chat as the broadcaster");
        assert_eq!(f["emote_only"], true);
        assert!(f.get("slow").is_none(), "a tag never seen is left out");
    }
}
