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
//!
//! Channels are bare lowercase Twitch logins (no `#`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
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
    last_text: Option<(String, Instant)>,
    /// Booked slots `acquire` handed out whose send has not been recorded yet.
    pending: VecDeque<Instant>,
}

#[derive(Default)]
struct Limiter {
    /// Every send and booked slot of the account, oldest first, each with the
    /// window budget of the role it went out under. A later booking must never
    /// push an earlier-booked send over its own budget.
    sends: VecDeque<(Instant, usize)>,
    channels: HashMap<String, ChannelFacts>,
    own_login: Option<String>,
}

fn key(channel: &str) -> String {
    channel.trim().trim_start_matches('#').to_ascii_lowercase()
}

impl Limiter {
    fn facts(&mut self, channel: &str) -> &mut ChannelFacts {
        self.channels.entry(key(channel)).or_default()
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

    /// Sends counted in the window that ends at `end`: (end - WINDOW, end].
    fn in_window(&self, end: Instant) -> usize {
        self.sends
            .iter()
            .filter(|(t, _)| *t <= end && end.saturating_duration_since(*t) < WINDOW)
            .count()
    }

    fn insert_send(&mut self, at: Instant, limit: usize) {
        let pos = self.sends.iter().position(|(t, _)| *t > at).unwrap_or(self.sends.len());
        self.sends.insert(pos, (at, limit));
    }

    fn prune(&mut self, now: Instant) {
        while let Some((front, _)) = self.sends.front() {
            if now.saturating_duration_since(*front) >= WINDOW && *front <= now {
                self.sends.pop_front();
            } else {
                break;
            }
        }
        for facts in self.channels.values_mut() {
            while let Some(front) = facts.pending.front() {
                // A booked slot nobody sent on is dropped once it is well past.
                if now.saturating_duration_since(*front) > Duration::from_secs(10) {
                    facts.pending.pop_front();
                } else {
                    break;
                }
            }
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
        if modes.subs_only == Some(true) && !facts.map(|f| f.viewer_is_sub).unwrap_or(false) {
            return Some(Refusal::SubsOnly);
        }
        if modes.emote_only == Some(true) {
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

    /// The earliest instant at or after `now` a send in `channel` may go.
    fn earliest(&self, channel: &str, now: Instant) -> Instant {
        let k = key(channel);
        let role = self.role_of(&k);
        let facts = self.channels.get(&k);
        let mut spacing = if role.privileged() { PRIVILEGED_SPACING } else { VIEWER_SPACING };
        if !role.privileged() {
            if let Some(slow) = facts.and_then(|f| f.modes.slow_seconds).filter(|s| *s > 0) {
                // A quarter second of margin over the room's own wait.
                spacing = spacing.max(Duration::from_secs(slow) + Duration::from_millis(250));
            }
        }
        let mut at = now;
        if let Some(last) = facts.and_then(|f| f.last_slot) {
            at = at.max(last + spacing);
        }
        let limit = if role.privileged() { PRIVILEGED_PER_WINDOW } else { VIEWER_PER_WINDOW };
        loop {
            // Fewer than `limit` sends may sit in (at - WINDOW, at].
            if self.in_window(at) >= limit {
                // The oldest send inside the window leaves it WINDOW after it went.
                let oldest = self
                    .sends
                    .iter()
                    .map(|(t, _)| *t)
                    .filter(|t| *t <= at && at.saturating_duration_since(*t) < WINDOW)
                    .min()
                    .unwrap_or(at);
                at = oldest + WINDOW;
                continue;
            }
            // A send already booked for later than `at` (and less than a
            // window later) will count this one when it goes out, so it must
            // still have room for one more. When it has none, this send goes
            // after it.
            let blocked = self
                .sends
                .iter()
                .filter(|(u, _)| *u > at && u.saturating_duration_since(at) < WINDOW)
                .find(|(u, budget)| self.in_window(*u) >= *budget)
                .map(|(u, _)| *u);
            match blocked {
                Some(u) => at = u + Duration::from_millis(1),
                None => return at,
            }
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
        facts.last_text = Some((text.to_string(), at));
        facts.pending.push_back(at);
        Ok(wait)
    }

    fn record(&mut self, channel: &str, text: &str, now: Instant) {
        self.prune(now);
        let facts = self.facts(channel);
        // A send on a slot `acquire` booked is already counted.
        if let Some(i) = facts
            .pending
            .iter()
            .position(|t| *t <= now + Duration::from_secs(2))
        {
            facts.pending.remove(i);
            facts.last_text = Some((text.to_string(), now));
            return;
        }
        facts.last_slot = Some(facts.last_slot.map_or(now, |l| l.max(now)));
        facts.last_text = Some((text.to_string(), now));
        let limit = self.per_window(channel);
        self.insert_send(now, limit);
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
    let mut l = limiter();
    let facts = l.facts(channel);
    let role = Some(Role::from_badges(badges));
    let is_sub = badges
        .split(',')
        .any(|b| b.starts_with("subscriber/") || b.starts_with("founder/"));
    let changed = facts.role != role || facts.viewer_is_sub != is_sub;
    facts.role = role;
    facts.viewer_is_sub = is_sub;
    changed
}

/// The channel's user id, from ROOMSTATE's `room-id`.
pub fn note_room_id(channel: &str, room_id: &str) {
    let room_id = room_id.trim();
    if !room_id.is_empty() {
        limiter().facts(channel).room_id = Some(room_id.to_string());
    }
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
#[derive(Serialize)]
pub struct PacedSend {
    pub is_sent: bool,
    pub drop_reason: Option<String>,
}

/// Send one scripted line (`/timer`, `/chain`, `/foreach`, `/runin`) to a
/// Twitch channel: wait for a slot, then send as the signed-in account. Plain
/// text goes over Helix, which needs no joined chat, so a timer still posts
/// after its chat was closed or switched away from. `channel_id` saves a
/// lookup when the caller knows it. Errors carry the refusal in words.
#[tauri::command]
pub async fn chat_send_paced(channel: String, channel_id: Option<String>, text: String) -> Result<PacedSend, String> {
    let login = key(&channel);
    if login.is_empty() {
        return Err(Refusal::EmptyChannel.text());
    }
    if text.trim().is_empty() {
        return Err("Nothing to send".into());
    }
    let sender_id = match crate::services::account_store::AccountStore::primary() {
        Some(account) => account.user_id,
        None => {
            crate::services::twitch_service::TwitchService::get_user_info()
                .await
                .map_err(|_| "Sign in to Twitch to send messages".to_string())?
                .id
        }
    };
    let broadcaster_id = match channel_id.map(|id| id.trim().to_string()).filter(|id| !id.is_empty()) {
        Some(id) => id,
        None => crate::services::slash_commands::user_by_login(&login)
            .await?
            .and_then(|u| u["id"].as_str().map(str::to_string))
            .ok_or_else(|| format!("There is no Twitch channel named {login}"))?,
    };
    acquire(&login, &text).await.map_err(|r| r.text())?;
    let sent = crate::services::chat_service::ChatService::send_message(
        &text,
        None,
        Some(&login),
        Some(&broadcaster_id),
        Some(&sender_id),
        None,
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok(PacedSend { is_sent: sent.is_sent, drop_reason: sent.drop_reason })
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
