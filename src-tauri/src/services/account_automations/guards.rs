//! The rails every rule runs inside, on this device: the rule's own rate,
//! its cooldowns, the conversation and pace limits, the other-channel gap,
//! holds where Twitch says an account can't post, the echo ring, paced chat
//! notes and the circuit breaker.
//!
//! Pure over an injected `now` (milliseconds on one steady clock; only
//! [`row_fresh`] reads the wall clock's row stamp). Test mode runs every rail
//! exactly as Live; only the breaker differs, and in Test it only says what
//! it would do. The rails that span devices (gap, cooldowns, conversation,
//! pace) are checked here first and then refereed by the claim: this is the
//! local view, never the only one.

use std::collections::{HashMap, HashSet, VecDeque};

use super::claims::ClassPlan;
use super::facts::{Facts, MAX_ROW_AGE_MS};
use crate::models::automations::{AutomationPrefs, Rule};
use crate::services::chat_burst::{is_cannot_post, CANNOT_POST_NOTICES};

/// The lane of a rule's local actions (Notify), beside its action classes.
pub const LOCAL_LANE: &str = "local";
pub const HOUR_MS: i64 = 3_600_000;
/// A rule's sixth fire inside a minute holds it.
pub const BURST_FIRES: usize = 6;
pub const BURST_MS: i64 = 60_000;
pub const RATE_HOLD_MS: i64 = 600_000;
pub const CONV_LIMIT: usize = 3;
pub const CONV_MS: i64 = 300_000;
pub const PACE_MS: i64 = 60_000;
pub const ECHO_SIZE: usize = 64;
pub const ECHO_MS: i64 = 60_000;
/// At most one chat note per rule per chat in this time.
pub const NOTE_GAP_MS: i64 = 30_000;
pub const BREAKER_WINDOW_MS: i64 = 120_000;
/// How long a "can't post here" refusal holds an account in a chat.
pub const REFUSAL_HOLD_MS: i64 = 600_000;

pub const BREAKER_OFF: &str = "Automations turned off: rules were firing too fast";
pub const BREAKER_WOULD: &str = "Would turn automations off";
pub const RATE_HELD: &str = "Held for 10 minutes: this rule fired too often";
pub const COOLING: &str = "Cooling down";
pub const HELD_HERE: &str = "Twitch says this account can't post here right now";
pub const NOT_YOUR_CHANNEL: &str = "Not your channel, and Other channels is off";
pub const CONV_FULL: &str = "Already answered this person 3 times in 5 minutes";

/// The lanes a job's cooldowns are kept per: each action class's key, and
/// [`LOCAL_LANE`] when the rule has a local action.
pub fn lanes_for(rule: &Rule, plans: &[ClassPlan]) -> Vec<String> {
    let mut lanes: Vec<String> = plans.iter().map(|p| p.class.key()).collect();
    if rule.actions.iter().any(|a| !a.is_outward()) {
        lanes.push(LOCAL_LANE.to_string());
    }
    lanes
}

/// A row may fire a rule: it was not dropped by your chat filters, did not
/// come from history, and was read off the socket at most 30 s ago (by this
/// device's wall clock, never Twitch's).
pub fn row_fresh(facts: &Facts, dropped: bool, now_ms: i64) -> bool {
    !dropped && !facts.from_backfill && facts.received_at_ms.is_some_and(|at| now_ms - at <= MAX_ROW_AGE_MS)
}

/// Admission's answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admit {
    /// The rule runs; `lanes` are the lanes not cooling down.
    Admitted { lanes: Vec<String> },
    /// Skipped; `newly_held` when this fire put the rule on its 10-minute
    /// hold (Activity and the status say so once).
    Skipped { reason: String, newly_held: bool },
}

/// What the breaker decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trip {
    /// Live jobs ran too fast: turn rules Off (a spawned write, after every
    /// lock is released) and notify.
    TurnOff,
    /// Test jobs did: log only.
    WouldTurnOff,
}

impl Trip {
    pub fn words(self) -> &'static str {
        match self {
            Trip::TurnOff => BREAKER_OFF,
            Trip::WouldTurnOff => BREAKER_WOULD,
        }
    }
}

/// How long a hold lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldUntil {
    At(i64),
    /// Until the next settings or account save (a ban, a suspension).
    NextSave,
}

/// One class's sends, as the local rails see them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendCheck<'a> {
    pub owner: &'a str,
    pub send_as: &'a str,
    /// `twitch:<login>` of the chat it posts in.
    pub chat_key: &'a str,
    pub room_id: &'a str,
    /// The chat is not the owner's.
    pub other_channel: bool,
    pub sends: u32,
    /// The sends answer the person.
    pub to_person: bool,
    pub person: Option<&'a str>,
}

/// Admitted jobs per 2-minute window (the last three).
#[derive(Clone, Debug, Default)]
struct Windows {
    counts: VecDeque<(i64, i64)>,
}

impl Windows {
    fn add(&mut self, at: i64, delta: i64) {
        let window = at.div_euclid(BREAKER_WINDOW_MS);
        if let Some(entry) = self.counts.iter_mut().find(|(w, _)| *w == window) {
            entry.1 += delta;
            return;
        }
        if delta > 0 && self.counts.back().map_or(true, |(w, _)| *w < window) {
            self.counts.push_back((window, delta));
            while self.counts.len() > 3 {
                self.counts.pop_front();
            }
        }
    }

    fn count(&self, window: i64) -> i64 {
        self.counts.iter().find(|(w, _)| *w == window).map_or(0, |(_, c)| *c)
    }
}

fn echo_key(chat_key: &str, text: &str) -> String {
    let words: String = text.chars().filter(|c| *c != '\u{e0000}').collect();
    let normalized = words.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    format!("{chat_key}\n{normalized}")
}

fn timeout_seconds(message: &str) -> Option<i64> {
    message.split(|c: char| !c.is_ascii_digit()).find(|part| !part.is_empty()).and_then(|n| n.parse().ok())
}

/// This device's rail state.
#[derive(Clone, Debug, Default)]
pub struct Guards {
    fires: HashMap<String, VecDeque<i64>>,
    rate_held: HashMap<String, i64>,
    rule_cd: HashMap<(String, String, String), i64>,
    person_cd: HashMap<(String, String, String), i64>,
    pace: HashMap<String, VecDeque<i64>>,
    conv: HashMap<(String, String), VecDeque<i64>>,
    gap: HashMap<String, i64>,
    live: Windows,
    test: Windows,
    echo: VecDeque<(String, i64)>,
    holds: HashMap<(String, String), HoldUntil>,
    notes: HashMap<(String, String), i64>,
}

impl Guards {
    pub fn new() -> Guards {
        Guards::default()
    }

    /// Admit one fire of `rule` in `room` (the event's room) for `person`.
    /// Counts toward the rule's rate in Test and Live alike; a fire above the
    /// rule's hourly limit, or its sixth inside a minute, holds the rule for
    /// 10 minutes. Lanes still cooling down are left out; when every lane is,
    /// the fire is skipped.
    pub fn admit(&mut self, rule: &Rule, lanes: &[String], room: &str, person: Option<&str>, now: i64) -> Admit {
        if let Some(until) = self.rate_held.get(&rule.id).copied() {
            if now < until {
                return Admit::Skipped { reason: RATE_HELD.to_string(), newly_held: false };
            }
            self.rate_held.remove(&rule.id);
        }
        let max = rule.limits.max_per_hour.max(1) as usize;
        let fires = self.fires.entry(rule.id.clone()).or_default();
        while fires.front().is_some_and(|t| now - *t >= HOUR_MS) {
            fires.pop_front();
        }
        let in_minute = fires.iter().filter(|t| now - **t < BURST_MS).count();
        if fires.len() >= max || in_minute + 1 >= BURST_FIRES {
            fires.clear();
            self.rate_held.insert(rule.id.clone(), now + RATE_HOLD_MS);
            return Admit::Skipped { reason: RATE_HELD.to_string(), newly_held: true };
        }
        let open: Vec<String> =
            lanes.iter().filter(|lane| !self.cooling(&rule.id, lane.as_str(), room, person, now)).cloned().collect();
        if open.is_empty() && !lanes.is_empty() {
            return Admit::Skipped { reason: COOLING.to_string(), newly_held: false };
        }
        self.fires.entry(rule.id.clone()).or_default().push_back(now);
        Admit::Admitted { lanes: open }
    }

    /// When the rule's hold ends, while it is held.
    pub fn rate_held_until(&self, rule_id: &str, now: i64) -> Option<i64> {
        self.rate_held.get(rule_id).copied().filter(|until| now < *until)
    }

    fn cooling(&self, rule_id: &str, lane: &str, room: &str, person: Option<&str>, now: i64) -> bool {
        let key = (rule_id.to_string(), lane.to_string(), room.to_string());
        if self.rule_cd.get(&key).is_some_and(|until| now < *until) {
            return true;
        }
        match person {
            Some(p) => self
                .person_cd
                .get(&(rule_id.to_string(), lane.to_string(), p.to_string()))
                .is_some_and(|until| now < *until),
            None => false,
        }
    }

    /// The lane acted (Live sent, or Test would have): its rule cooldown in
    /// this chat and its person cooldown start now. A lost exact event stamps
    /// the same way.
    pub fn stamp(&mut self, rule: &Rule, lane: &str, room: &str, person: Option<&str>, now: i64) {
        let rule_cd = i64::from(rule.limits.rule_cooldown_seconds) * 1000;
        let user_cd = i64::from(rule.limits.user_cooldown_seconds) * 1000;
        self.stamp_both(rule, lane, room, person, (rule_cd > 0).then_some(now + rule_cd), (user_cd > 0).then_some(now + user_cd));
    }

    /// Another device holds a windowed rail of this lane until `until` (local
    /// time, through the server offset): cool down until then.
    pub fn stamp_until(&mut self, rule: &Rule, lane: &str, room: &str, person: Option<&str>, until: i64) {
        let rule_until = (rule.limits.rule_cooldown_seconds > 0).then_some(until);
        let person_until = (rule.limits.user_cooldown_seconds > 0).then_some(until);
        self.stamp_both(rule, lane, room, person, rule_until, person_until);
    }

    fn stamp_both(
        &mut self,
        rule: &Rule,
        lane: &str,
        room: &str,
        person: Option<&str>,
        rule_until: Option<i64>,
        person_until: Option<i64>,
    ) {
        if let Some(until) = rule_until {
            let entry = self.rule_cd.entry((rule.id.clone(), lane.to_string(), room.to_string())).or_insert(until);
            *entry = (*entry).max(until);
        }
        if let (Some(until), Some(person)) = (person_until, person) {
            let entry = self.person_cd.entry((rule.id.clone(), lane.to_string(), person.to_string())).or_insert(until);
            *entry = (*entry).max(until);
        }
    }

    /// The local view of the rails right before a class books: a hold, other
    /// channels, the other-channel gap, the pace and the conversation limit.
    pub fn outward_ok(&self, check: &SendCheck, prefs: &AutomationPrefs, now: i64) -> Result<(), String> {
        let prefs = prefs.clamped();
        if self.held(check.send_as, check.chat_key, now) {
            return Err(HELD_HERE.to_string());
        }
        if check.other_channel {
            if !prefs.other_channels {
                return Err(NOT_YOUR_CHANNEL.to_string());
            }
            let gap = i64::from(prefs.other_channel_gap_seconds) * 1000;
            if self.gap.get(check.room_id).is_some_and(|at| now - at < gap) {
                return Err(format!("Already posted in this chat in the last {} seconds", prefs.other_channel_gap_seconds));
            }
        }
        let sends = check.sends as usize;
        let paced = self.pace.get(check.owner).map_or(0, |q| q.iter().filter(|t| now - **t < PACE_MS).count());
        if paced + sends > prefs.sends_per_minute as usize {
            return Err(format!("Pace reached: {} messages a minute", prefs.sends_per_minute));
        }
        if let (true, Some(person)) = (check.to_person, check.person) {
            let talked = self
                .conv
                .get(&(check.send_as.to_string(), person.to_string()))
                .map_or(0, |q| q.iter().filter(|t| now - **t < CONV_MS).count());
            if talked + sends > CONV_LIMIT {
                return Err(CONV_FULL.to_string());
            }
        }
        Ok(())
    }

    /// The class sent (or may have): count it on the pace, the conversation
    /// and the other-channel gap.
    pub fn note_sent(&mut self, check: &SendCheck, now: i64) {
        let pace = self.pace.entry(check.owner.to_string()).or_default();
        for _ in 0..check.sends {
            pace.push_back(now);
        }
        while pace.front().is_some_and(|t| now - *t >= PACE_MS) {
            pace.pop_front();
        }
        if let (true, Some(person)) = (check.to_person, check.person) {
            let talked = self.conv.entry((check.send_as.to_string(), person.to_string())).or_default();
            for _ in 0..check.sends {
                talked.push_back(now);
            }
            while talked.front().is_some_and(|t| now - *t >= CONV_MS) {
                talked.pop_front();
            }
        }
        if check.other_channel {
            self.gap.insert(check.room_id.to_string(), now);
        }
    }

    /// One admitted job, Live or Test. Trips when the admitted jobs (minus
    /// those that lost their event to another device) are above twice the
    /// pace in each of two 2-minute windows in a row, so a raid two devices
    /// share never trips it and a rule looping on one device does.
    pub fn note_admitted(&mut self, live: bool, sends_per_minute: u32, now: i64) -> Option<Trip> {
        let windows = if live { &mut self.live } else { &mut self.test };
        windows.add(now, 1);
        let window = now.div_euclid(BREAKER_WINDOW_MS);
        let limit = 2 * i64::from(sends_per_minute.clamp(1, 20));
        if windows.count(window) > limit && windows.count(window - 1) > limit {
            windows.counts.clear();
            return Some(if live { Trip::TurnOff } else { Trip::WouldTurnOff });
        }
        None
    }

    /// A job admitted at `admitted_at` lost its event element to another device.
    pub fn note_event_lost(&mut self, live: bool, admitted_at: i64) {
        let windows = if live { &mut self.live } else { &mut self.test };
        windows.add(admitted_at, -1);
    }

    /// Remember an automated send, so the same text coming back into the chat
    /// within a minute never triggers a rule.
    pub fn note_echo(&mut self, chat_key: &str, text: &str, now: i64) {
        self.echo.push_back((echo_key(chat_key, text), now));
        while self.echo.len() > ECHO_SIZE {
            self.echo.pop_front();
        }
    }

    pub fn is_echo(&self, chat_key: &str, text: &str, now: i64) -> bool {
        let key = echo_key(chat_key, text);
        self.echo.iter().any(|(k, at)| now - at < ECHO_MS && *k == key)
    }

    pub fn hold(&mut self, account: &str, chat_key: &str, until: HoldUntil) {
        let key = (account.to_string(), chat_key.to_string());
        let next = match (self.holds.get(&key).copied(), until) {
            (Some(HoldUntil::NextSave), _) | (_, HoldUntil::NextSave) => HoldUntil::NextSave,
            (Some(HoldUntil::At(a)), HoldUntil::At(b)) => HoldUntil::At(a.max(b)),
            (None, at) => at,
        };
        self.holds.insert(key, next);
    }

    pub fn held(&self, account: &str, chat_key: &str, now: i64) -> bool {
        match self.holds.get(&(account.to_string(), chat_key.to_string())) {
            Some(HoldUntil::NextSave) => true,
            Some(HoldUntil::At(until)) => now < *until,
            None => false,
        }
    }

    /// A settings or account save lifts the holds that last until one.
    pub fn clear_save_holds(&mut self) {
        self.holds.retain(|_, hold| !matches!(hold, HoldUntil::NextSave));
    }

    /// Twitch refused a send with words that mean this account can't post in
    /// this chat: hold it there for 10 minutes. True when it held.
    pub fn hold_after_refusal(&mut self, send_as: &str, chat_key: &str, reason: &str, now: i64) -> bool {
        if !is_cannot_post(reason) {
            return false;
        }
        self.hold(send_as, chat_key, HoldUntil::At(now + REFUSAL_HOLD_MS));
        true
    }

    /// A chat NOTICE to the main account that it cannot post: a timeout holds
    /// for its seconds, a ban or suspension until the next save.
    pub fn hold_after_notice(&mut self, account: &str, chat_key: &str, code: &str, message: &str, now: i64) -> bool {
        if !CANNOT_POST_NOTICES.contains(&code) {
            return false;
        }
        let until = if code == "msg_timedout" {
            HoldUntil::At(now + timeout_seconds(message).map_or(REFUSAL_HOLD_MS, |s| s * 1000))
        } else {
            HoldUntil::NextSave
        };
        self.hold(account, chat_key, until);
        true
    }

    /// A CLEARCHAT naming one of your accounts (any signed-in one) holds that
    /// account in that chat for the ban's duration, or until the next save.
    pub fn hold_after_clearchat(
        &mut self,
        my_ids: &HashSet<String>,
        target_id: &str,
        chat_key: &str,
        ban_duration_s: Option<i64>,
        now: i64,
    ) -> bool {
        if !my_ids.contains(target_id) {
            return false;
        }
        let until = match ban_duration_s {
            Some(seconds) if seconds > 0 => HoldUntil::At(now + seconds * 1000),
            _ => HoldUntil::NextSave,
        };
        self.hold(target_id, chat_key, until);
        true
    }

    /// Whether a chat note may go out now (one per rule per chat per 30 s);
    /// true stamps it.
    pub fn note_ok(&mut self, rule_id: &str, chat_key: &str, now: i64) -> bool {
        let key = (rule_id.to_string(), chat_key.to_string());
        if self.notes.get(&key).is_some_and(|at| now - at < NOTE_GAP_MS) {
            return false;
        }
        self.notes.insert(key, now);
        true
    }

    /// Forget rules that no longer exist.
    pub fn retain_rules(&mut self, ids: &HashSet<String>) {
        self.fires.retain(|id, _| ids.contains(id));
        self.rate_held.retain(|id, _| ids.contains(id));
        self.rule_cd.retain(|(id, _, _), _| ids.contains(id));
        self.person_cd.retain(|(id, _, _), _| ids.contains(id));
        self.notes.retain(|(id, _), _| ids.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::Trigger;
    use crate::services::account_automations::claims::classes_for;
    use crate::services::account_automations::compile::fixtures::{notify, reply, rule, send_to, ALT, ME};
    use crate::services::account_automations::facts::{from_chat, rows};
    use crate::services::chat_burst::VARY_SUFFIX;

    const SEC: i64 = 1_000;

    fn lanes(r: &Rule) -> Vec<String> {
        lanes_for(r, &classes_for(r, ME).0)
    }

    fn mention_rule() -> Rule {
        rule("r", Trigger::Mention { replies_too: false }, vec![reply("hi"), notify("x")])
    }

    fn admitted(a: &Admit) -> bool {
        matches!(a, Admit::Admitted { .. })
    }

    fn check<'a>(person: Option<&'a str>, other: bool, room: &'a str) -> SendCheck<'a> {
        SendCheck {
            owner: ME,
            send_as: ME,
            chat_key: if other { "twitch:other" } else { "twitch:me" },
            room_id: room,
            other_channel: other,
            sends: 1,
            to_person: person.is_some(),
            person,
        }
    }

    #[test]
    fn lanes_are_the_classes_and_the_local_lane() {
        let r = rule("r", Trigger::Mention { replies_too: false }, vec![reply("hi"), send_to("other", "700", "x"), notify("x")]);
        assert_eq!(lanes(&r), vec![format!("post:{ME}"), "post:700".to_string(), LOCAL_LANE.to_string()]);
        assert_eq!(lanes(&rule("n", Trigger::Mention { replies_too: false }, vec![notify("x")])), vec![LOCAL_LANE.to_string()]);
    }

    #[test]
    fn a_rule_that_fires_too_often_is_held_for_ten_minutes() {
        let mut r = mention_rule();
        r.limits.rule_cooldown_seconds = 0;
        r.limits.user_cooldown_seconds = 0;
        r.limits.max_per_hour = 3;
        let l = lanes(&r);
        let mut g = Guards::new();
        for i in 0..3 {
            assert!(admitted(&g.admit(&r, &l, ME, Some("300"), i * 120 * SEC)));
        }
        assert_eq!(g.admit(&r, &l, ME, Some("300"), 400 * SEC), Admit::Skipped { reason: RATE_HELD.into(), newly_held: true });
        assert_eq!(g.rate_held_until(&r.id, 401 * SEC), Some(400 * SEC + RATE_HOLD_MS));
        assert_eq!(g.admit(&r, &l, ME, Some("300"), 500 * SEC), Admit::Skipped { reason: RATE_HELD.into(), newly_held: false });
        assert!(admitted(&g.admit(&r, &l, ME, Some("300"), 400 * SEC + RATE_HOLD_MS)));
        // The sixth fire inside a minute holds it, whatever the hourly limit.
        let mut fast = r.clone();
        fast.limits.max_per_hour = 120;
        let mut g = Guards::new();
        for i in 0..5 {
            assert!(admitted(&g.admit(&fast, &l, ME, None, i * SEC)));
        }
        assert!(matches!(g.admit(&fast, &l, ME, None, 5 * SEC), Admit::Skipped { newly_held: true, .. }));
        // Five a minute, minute after minute, never does.
        let mut g = Guards::new();
        for i in 0..100 {
            assert!(admitted(&g.admit(&fast, &l, ME, None, i * 12 * SEC)), "{i}");
        }
    }

    #[test]
    fn cooldowns_hold_each_lane_per_chat_and_per_person() {
        let r = mention_rule();
        let l = lanes(&r);
        let mut g = Guards::new();
        assert!(admitted(&g.admit(&r, &l, ME, Some("300"), 0)));
        for lane in &l {
            g.stamp(&r, lane, ME, Some("300"), 0);
        }
        // The same chat inside the rule's 30 s: cooling down.
        assert_eq!(g.admit(&r, &l, ME, Some("301"), 10 * SEC), Admit::Skipped { reason: COOLING.into(), newly_held: false });
        // Another chat has its own cooldown.
        assert!(admitted(&g.admit(&r, &l, "700", Some("301"), 10 * SEC)));
        // After 30 s another person passes; the same person waits out 300 s.
        assert!(admitted(&g.admit(&r, &l, ME, Some("301"), 31 * SEC)));
        assert_eq!(g.admit(&r, &l, ME, Some("300"), 32 * SEC), Admit::Skipped { reason: COOLING.into(), newly_held: false });
        assert!(admitted(&g.admit(&r, &l, ME, Some("300"), 301 * SEC)));

        // A lane cooling down leaves the rule's other lanes open.
        let mut g = Guards::new();
        g.stamp(&r, &l[0], ME, Some("300"), 0);
        assert_eq!(g.admit(&r, &l, ME, Some("300"), SEC), Admit::Admitted { lanes: vec![LOCAL_LANE.to_string()] });
        // A windowed rail another device holds cools the lane until its free time.
        let mut g = Guards::new();
        g.stamp_until(&r, &l[0], ME, Some("300"), 90 * SEC);
        assert_eq!(g.admit(&r, &l, ME, Some("300"), 80 * SEC), Admit::Admitted { lanes: vec![LOCAL_LANE.to_string()] });
        assert_eq!(g.admit(&r, &l, ME, Some("300"), 91 * SEC), Admit::Admitted { lanes: l.clone() });
        // Cooldowns of 0 stamp nothing.
        let mut free = r.clone();
        free.limits.rule_cooldown_seconds = 0;
        free.limits.user_cooldown_seconds = 0;
        let mut g = Guards::new();
        g.stamp(&free, &l[0], ME, Some("300"), 0);
        g.stamp_until(&free, &l[0], ME, Some("300"), 90 * SEC);
        assert_eq!(g.admit(&free, &l, ME, Some("300"), 1), Admit::Admitted { lanes: l.clone() });
    }

    #[test]
    fn pace_conversation_gap_and_other_channels() {
        let two_a_minute = AutomationPrefs { sends_per_minute: 2, ..AutomationPrefs::default() };
        let mut g = Guards::new();
        let first = check(Some("300"), false, ME);
        assert_eq!(g.outward_ok(&first, &two_a_minute, 0), Ok(()));
        g.note_sent(&first, 0);
        g.note_sent(&check(Some("301"), false, ME), SEC);
        assert_eq!(
            g.outward_ok(&check(Some("302"), false, ME), &two_a_minute, 2 * SEC),
            Err("Pace reached: 2 messages a minute".to_string())
        );
        assert_eq!(g.outward_ok(&check(Some("302"), false, ME), &two_a_minute, 61 * SEC), Ok(()));
        // A class with two sends counts both.
        let two = SendCheck { sends: 2, ..check(Some("303"), false, ME) };
        assert_eq!(Guards::new().outward_ok(&two, &two_a_minute, 0), Ok(()));
        let three = SendCheck { sends: 3, ..two };
        assert!(Guards::new().outward_ok(&three, &two_a_minute, 0).is_err());

        // Three answers to one person in 5 minutes, then no more; someone else still gets one.
        let busy = AutomationPrefs { sends_per_minute: 20, ..AutomationPrefs::default() };
        let mut g = Guards::new();
        for i in 0..3 {
            let c = check(Some("300"), false, ME);
            assert_eq!(g.outward_ok(&c, &busy, i * SEC), Ok(()));
            g.note_sent(&c, i * SEC);
        }
        assert_eq!(g.outward_ok(&check(Some("300"), false, ME), &busy, 10 * SEC), Err(CONV_FULL.to_string()));
        assert_eq!(g.outward_ok(&check(Some("301"), false, ME), &busy, 10 * SEC), Ok(()));
        assert_eq!(g.outward_ok(&check(Some("300"), false, ME), &busy, 301 * SEC), Ok(()));
        // A message that does not answer the person never counts toward them.
        let mut g = Guards::new();
        for i in 0..5 {
            g.note_sent(&check(None, false, ME), i * 13 * SEC);
        }
        assert_eq!(g.outward_ok(&check(Some("300"), false, ME), &busy, 70 * SEC), Ok(()));

        // Other channels are off by default; with them on, one message per chat per gap.
        let mut g = Guards::new();
        let away = check(Some("300"), true, "700");
        assert_eq!(g.outward_ok(&away, &AutomationPrefs::default(), 0), Err(NOT_YOUR_CHANNEL.to_string()));
        let open = AutomationPrefs { other_channels: true, other_channel_gap_seconds: 30, ..AutomationPrefs::default() };
        assert_eq!(g.outward_ok(&away, &open, 0), Ok(()));
        g.note_sent(&away, 0);
        assert_eq!(
            g.outward_ok(&check(Some("301"), true, "700"), &open, 5 * SEC),
            Err("Already posted in this chat in the last 30 seconds".to_string())
        );
        assert_eq!(g.outward_ok(&check(Some("301"), true, "800"), &open, 5 * SEC), Ok(()));
        assert_eq!(g.outward_ok(&check(Some("301"), true, "700"), &open, 30 * SEC), Ok(()));
    }

    #[test]
    fn holds_from_refusals_notices_and_clearchat() {
        let prefs = AutomationPrefs::default();
        // Twitch's words for "can't post here" hold that account in that chat for 10 minutes.
        let mut g = Guards::new();
        assert!(g.hold_after_refusal(ALT, "twitch:me", "You are banned from talking in this channel", 0));
        assert!(!g.hold_after_refusal(ALT, "twitch:me", "Your message is identical to the previous one", 0));
        assert!(g.held(ALT, "twitch:me", REFUSAL_HOLD_MS - 1) && !g.held(ALT, "twitch:me", REFUSAL_HOLD_MS));
        assert!(!g.held(ME, "twitch:me", 0));
        let alt_send = SendCheck { send_as: ALT, ..check(None, false, ME) };
        assert_eq!(g.outward_ok(&alt_send, &prefs, 0), Err(HELD_HERE.to_string()));
        assert_eq!(g.outward_ok(&check(None, false, ME), &prefs, 0), Ok(()));
        // A timeout notice holds for its seconds; a ban until the next save.
        let mut g = Guards::new();
        assert!(g.hold_after_notice(ME, "twitch:me", "msg_timedout", "You are timed out for 60 more seconds.", 0));
        assert!(g.held(ME, "twitch:me", 59 * SEC) && !g.held(ME, "twitch:me", 60 * SEC));
        assert!(g.hold_after_notice(ME, "twitch:x", "msg_banned", "You are permanently banned from talking in x.", 0));
        assert!(!g.hold_after_notice(ME, "twitch:y", "msg_ratelimit", "Slow down", 0));
        for code in CANNOT_POST_NOTICES {
            assert!(Guards::new().hold_after_notice(ME, "twitch:z", code, "", 0), "{code}");
        }
        assert!(g.held(ME, "twitch:x", 10 * 24 * HOUR_MS));
        g.clear_save_holds();
        assert!(!g.held(ME, "twitch:x", 0));
        assert!(g.held(ME, "twitch:me", 0));
        // A CLEARCHAT naming any of your accounts (an alt too) holds it there.
        let mine: HashSet<String> = [ME.to_string(), ALT.to_string()].into_iter().collect();
        let mut g = Guards::new();
        assert!(g.hold_after_clearchat(&mine, ALT, "twitch:friend", Some(600), 0));
        assert!(g.held(ALT, "twitch:friend", 599 * SEC) && !g.held(ALT, "twitch:friend", 600 * SEC));
        assert!(g.hold_after_clearchat(&mine, ME, "twitch:friend", None, 0));
        assert!(g.held(ME, "twitch:friend", 365 * 24 * HOUR_MS));
        assert!(!g.hold_after_clearchat(&mine, "999", "twitch:friend", Some(60), 0));
        // A longer hold is never cut short by a shorter one.
        let mut g = Guards::new();
        g.hold(ME, "twitch:me", HoldUntil::At(100));
        g.hold(ME, "twitch:me", HoldUntil::At(50));
        assert!(g.held(ME, "twitch:me", 99));
    }

    #[test]
    fn the_breaker_trips_on_two_busy_windows_in_a_row_only() {
        let w = BREAKER_WINDOW_MS;
        // 2 x spm + 1 in each of two windows in a row turns rules off.
        let mut g = Guards::new();
        for i in 0..21 {
            assert_eq!(g.note_admitted(true, 10, i * 100), None);
        }
        let mut trip = None;
        for i in 0..21 {
            trip = trip.or(g.note_admitted(true, 10, w + i * 100));
        }
        assert_eq!(trip, Some(Trip::TurnOff));
        assert_eq!(Trip::TurnOff.words(), BREAKER_OFF);
        // The pace a minute, all day long, never does.
        let mut g = Guards::new();
        for i in 0..2_000 {
            assert_eq!(g.note_admitted(true, 10, i * 6_000), None, "{i}");
        }
        // One busy 2-minute window alone never does.
        let mut g = Guards::new();
        for i in 0..200 {
            assert_eq!(g.note_admitted(true, 10, i * 10), None);
        }
        for i in 0..20 {
            assert_eq!(g.note_admitted(true, 10, w + 60_000 + i), None);
        }
        // Test only says what it would do.
        let mut g = Guards::new();
        let mut trip = None;
        for i in 0..42 {
            trip = trip.or(g.note_admitted(false, 10, (i / 21) * w + (i % 21) * 100));
        }
        assert_eq!(trip, Some(Trip::WouldTurnOff));
        assert_eq!(Trip::WouldTurnOff.words(), BREAKER_WOULD);
    }

    #[test]
    fn a_raid_shared_by_two_devices_never_trips_it_but_a_local_loop_does() {
        let w = BREAKER_WINDOW_MS;
        // Each device admits all 40 raid events and loses about half to the
        // other device, in two windows in a row.
        let mut g = Guards::new();
        for window in 0..2 {
            for i in 0..40 {
                let at = window * w + i * 500;
                assert_eq!(g.note_admitted(true, 10, at), None, "{window}/{i}");
                if i % 2 == 1 {
                    g.note_event_lost(true, at);
                }
            }
        }
        // A rule looping on one device is refused locally by pace before any
        // claim, so it loses nothing, and it trips.
        let mut g = Guards::new();
        let mut trip = None;
        for i in 0..60 {
            trip = trip.or(g.note_admitted(true, 10, i * 4_000));
        }
        assert_eq!(trip, Some(Trip::TurnOff));
    }

    #[test]
    fn the_echo_ring_remembers_the_last_64_sends_for_a_minute() {
        let mut g = Guards::new();
        g.note_echo("twitch:me", "Welcome in,  Fan!", 0);
        assert!(g.is_echo("twitch:me", "welcome in, fan!", 30 * SEC));
        assert!(g.is_echo("twitch:me", &format!("Welcome in, Fan!{VARY_SUFFIX}"), 30 * SEC));
        assert!(!g.is_echo("twitch:other", "welcome in, fan!", 30 * SEC));
        assert!(!g.is_echo("twitch:me", "welcome in, fan!", 60 * SEC));
        for i in 0..64 {
            g.note_echo("twitch:me", &format!("line {i}"), 10);
        }
        assert!(!g.is_echo("twitch:me", "welcome in, fan!", 20));
        assert!(g.is_echo("twitch:me", "line 0", 20));
    }

    #[test]
    fn chat_notes_are_paced_per_rule_and_chat() {
        let mut g = Guards::new();
        assert!(g.note_ok("r", "twitch:me", 0));
        assert!(!g.note_ok("r", "twitch:me", 29 * SEC));
        assert!(g.note_ok("r", "twitch:other", 29 * SEC));
        assert!(g.note_ok("r2", "twitch:me", 29 * SEC));
        assert!(g.note_ok("r", "twitch:me", 30 * SEC));
    }

    #[test]
    fn old_backfilled_or_dropped_rows_never_fire() {
        let now = 100_000;
        let read_at = |received: Option<i64>| {
            let mut m = rows::privmsg("m1", "me", ME, "fan", "300", "hi");
            m.received_at_ms = received;
            from_chat(&m).unwrap()
        };
        assert!(row_fresh(&read_at(Some(now - 30_000)), false, now));
        assert!(!row_fresh(&read_at(Some(now - 30_001)), false, now));
        assert!(!row_fresh(&read_at(None), false, now));
        assert!(!row_fresh(&read_at(Some(now)), true, now));
        let mut history = read_at(Some(now));
        history.from_backfill = true;
        assert!(!row_fresh(&history, false, now));
    }

    #[test]
    fn forgotten_rules_leave_no_state() {
        let r = mention_rule();
        let l = lanes(&r);
        let mut g = Guards::new();
        assert!(admitted(&g.admit(&r, &l, ME, Some("300"), 0)));
        g.stamp(&r, &l[0], ME, Some("300"), 0);
        assert!(g.note_ok(&r.id, "twitch:me", 0));
        g.retain_rules(&HashSet::new());
        assert_eq!(g.admit(&r, &l, ME, Some("300"), SEC), Admit::Admitted { lanes: l.clone() });
        assert!(g.note_ok(&r.id, "twitch:me", SEC));
    }
}
