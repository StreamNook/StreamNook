//! Does a rule fire for a row: the one matcher behind live chat, badge drops
//! and Try it.
//!
//! Pure over the compiled rules, the row's facts and a context (this
//! device's away and the local time). With `trace` on, every condition is
//! checked and explained (Try it); with it off, the first failure ends the
//! rule (the live path). Whether the stream is live, whether a chat is open in
//! a window and which stream you are watching are checked by the executor
//! when the rule acts, never here.

use chrono::{Datelike, NaiveDateTime, Timelike};
use serde::Serialize;

use super::badges;
use super::compile::{Compiled, CompiledRule};
use super::facts::{Facts, RowKind};
use crate::models::automations::{parse_hhmm, Away, ChannelScope, EventKind, Hours, Trigger, Weekday};
use crate::services::chat_rules::mentions_login;

/// A Shared Chat partner's row never starts an event or first-message rule.
pub const PARTNER_EVENTS: &str = "A Shared Chat partner's row never starts this kind of rule";
const NOT_A_MESSAGE: &str = "Not a chat message";
const NOT_AN_EVENT: &str = "Not a channel event";

/// The local time a rule's hours are judged at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// Minutes past local midnight.
    pub minute: u32,
    pub weekday: Weekday,
}

impl LocalTime {
    pub fn from_naive(t: NaiveDateTime) -> LocalTime {
        LocalTime { minute: t.hour() * 60 + t.minute(), weekday: weekday_of(t.weekday()) }
    }
}

fn weekday_of(day: chrono::Weekday) -> Weekday {
    match day {
        chrono::Weekday::Mon => Weekday::Mon,
        chrono::Weekday::Tue => Weekday::Tue,
        chrono::Weekday::Wed => Weekday::Wed,
        chrono::Weekday::Thu => Weekday::Thu,
        chrono::Weekday::Fri => Weekday::Fri,
        chrono::Weekday::Sat => Weekday::Sat,
        chrono::Weekday::Sun => Weekday::Sun,
    }
}

fn day_before(day: Weekday) -> Weekday {
    match day {
        Weekday::Mon => Weekday::Sun,
        Weekday::Tue => Weekday::Mon,
        Weekday::Wed => Weekday::Tue,
        Weekday::Thu => Weekday::Wed,
        Weekday::Fri => Weekday::Thu,
        Weekday::Sat => Weekday::Fri,
        Weekday::Sun => Weekday::Sat,
    }
}

/// What the matcher reads besides the row.
pub struct Ctx<'a> {
    /// This device's away, if set.
    pub away: Option<&'a Away>,
    pub local: LocalTime,
    /// Explain every condition (Try it) instead of stopping at the first miss.
    pub trace: bool,
}

/// One condition, explained.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    pub step: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// One rule's answer for one row.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    /// Index into `Compiled::rules`.
    pub rule: usize,
    pub rule_id: String,
    pub fired: bool,
    /// The first condition that missed.
    pub reason: Option<String>,
    /// Every condition, when traced.
    pub trace: Vec<Trace>,
}

struct Run<'v> {
    v: &'v mut Verdict,
    trace: bool,
}

impl Run<'_> {
    /// Record one condition. False when the rule is done: it missed and
    /// nothing is being traced.
    fn check(&mut self, step: &'static str, ok: bool, pass: impl FnOnce() -> String, fail: impl FnOnce() -> String) -> bool {
        if ok {
            if self.trace {
                self.v.trace.push(Trace { step, ok: true, detail: pass() });
            }
            return true;
        }
        let why = fail();
        if self.trace {
            self.v.trace.push(Trace { step, ok: false, detail: why.clone() });
        }
        if self.v.reason.is_none() {
            self.v.reason = Some(why);
        }
        self.v.fired = false;
        self.trace
    }
}

/// The verdicts of the running rules the row can reach, in rule order.
pub fn evaluate(compiled: &Compiled, facts: &Facts, ctx: &Ctx) -> Vec<Verdict> {
    compiled.candidates(facts).into_iter().map(|i| evaluate_rule(compiled, i, facts, ctx)).collect()
}

/// Try it: every rule, running or not, wherever it listens.
pub fn try_all(compiled: &Compiled, facts: &Facts, ctx: &Ctx) -> Vec<Verdict> {
    (0..compiled.rules.len()).map(|i| evaluate_rule(compiled, i, facts, ctx)).collect()
}

pub fn evaluate_rule(compiled: &Compiled, index: usize, facts: &Facts, ctx: &Ctx) -> Verdict {
    let cr = &compiled.rules[index];
    let mut verdict = Verdict { rule: index, rule_id: cr.rule.id.clone(), fired: true, reason: None, trace: Vec::new() };
    let mut run = Run { v: &mut verdict, trace: ctx.trace };
    check_rule(cr, facts, ctx, &mut run);
    verdict
}

fn check_rule(cr: &CompiledRule, facts: &Facts, ctx: &Ctx, run: &mut Run) {
    let rule = &cr.rule;
    let when = trigger_fit(cr, facts);
    let when_ok = when.is_ok();
    if !run.check(
        "When",
        when_ok,
        || when.clone().map(str::to_string).unwrap_or_default(),
        || when.clone().err().unwrap_or_default(),
    ) {
        return;
    }
    if matches!(facts.kind, RowKind::Message | RowKind::Notice) {
        let partner_ok = !facts.shared_partner || rule.only_if.shared_chat;
        if !run.check(
            "Shared Chat",
            partner_ok,
            || if facts.shared_partner { "From a Shared Chat partner's chat" } else { "From this chat" }.to_string(),
            || "From a Shared Chat partner's chat, and this rule leaves those out".to_string(),
        ) {
            return;
        }
        let channels = channel_fit(cr, facts);
        if !run.check(
            "Channels",
            channels.is_ok(),
            || channels.unwrap_or_default().to_string(),
            || channels.err().unwrap_or_default().to_string(),
        ) {
            return;
        }
        let roles = &rule.only_if.roles;
        let role_ok = roles.is_empty() || facts.roles.iter().any(|r| roles.contains(r));
        if !run.check(
            "From",
            role_ok,
            || if roles.is_empty() { "Anyone" } else { "A role this rule answers" }.to_string(),
            || "Not a role this rule answers".to_string(),
        ) {
            return;
        }
        let login = facts.user_login.as_str();
        let skipped = rule.only_if.skip_users.iter().any(|u| u == login);
        if !run.check("Skip", !skipped, || "Not on the skip list".to_string(), || format!("@{login} is on the skip list")) {
            return;
        }
        let only = &rule.only_if.only_users;
        let listed = only.is_empty() || only.iter().any(|u| u == login);
        if !run.check(
            "Only",
            listed,
            || if only.is_empty() { "Anyone" } else { "On this rule's list" }.to_string(),
            || format!("@{login} isn't on this rule's list"),
        ) {
            return;
        }
    }
    if let (Some(program), Some(message)) = (&cr.filter, &facts.message) {
        let mentioned = cr.owner_login.is_empty() == false && mentions_login(&facts.text, &cr.owner_login);
        if !run.check(
            "Expression",
            program.matches(message, mentioned),
            || "The expression holds".to_string(),
            || "The expression doesn't hold".to_string(),
        ) {
            return;
        }
    } else if cr.filter.is_some() && run.trace {
        run.v.trace.push(Trace { step: "Expression", ok: true, detail: "Checked on a real chat row".to_string() });
    }
    if rule.only_if.only_away
        && !run.check(
            "Away",
            ctx.away.is_some(),
            || "You're away on this device".to_string(),
            || "You're not away on this device".to_string(),
        )
    {
        return;
    }
    if let Some(hours) = &rule.only_if.hours {
        if !run.check(
            "Hours",
            in_hours(hours, ctx.local),
            || format!("Inside {} to {}", hours.from, hours.to),
            || format!("Outside {} to {}", hours.from, hours.to),
        ) {
            return;
        }
    }
    if rule.only_if.only_live && run.trace {
        run.v.trace.push(Trace { step: "Live", ok: true, detail: "Checked when it acts".to_string() });
    }
}

fn trigger_fit(cr: &CompiledRule, facts: &Facts) -> Result<&'static str, String> {
    let rule = &cr.rule;
    let chat_row = facts.kind == RowKind::Message;
    match &rule.when {
        Trigger::Message { .. } => {
            if !chat_row {
                return Err(NOT_A_MESSAGE.to_string());
            }
            if cr.test.matches(facts) {
                Ok("The message matches")
            } else {
                Err("The message doesn't match".to_string())
            }
        }
        Trigger::Mention { replies_too } => {
            if !chat_row {
                return Err(NOT_A_MESSAGE.to_string());
            }
            if cr.owner_login.is_empty() {
                return Err("Doesn't mention you".to_string());
            }
            if mentions_login(&facts.text, &cr.owner_login) {
                return Ok("Mentions you");
            }
            if *replies_too && !facts.reply_parent_user_id.is_empty() && facts.reply_parent_user_id == rule.owner.id {
                return Ok("Replies to you");
            }
            Err(format!("Doesn't mention @{}", cr.owner_login))
        }
        Trigger::FirstMessage {} => {
            if !chat_row {
                return Err(NOT_A_MESSAGE.to_string());
            }
            if facts.shared_partner {
                return Err(PARTNER_EVENTS.to_string());
            }
            if facts.first_message {
                Ok("Their first message here")
            } else {
                Err("Not their first message here".to_string())
            }
        }
        Trigger::ChannelEvent { events, min_months, min_gifts, min_viewers, min_bits } => {
            if facts.kind == RowKind::Badge {
                return Err(NOT_AN_EVENT.to_string());
            }
            if facts.shared_partner {
                return Err(PARTNER_EVENTS.to_string());
            }
            let Some(event) = &facts.event else {
                return Err(NOT_AN_EVENT.to_string());
            };
            if !events.contains(&event.kind) {
                return Err("Not one of this rule's events".to_string());
            }
            let (have, need, what) = match event.kind {
                EventKind::Sub | EventKind::Resub => (event.months, *min_months, "months"),
                EventKind::Gift | EventKind::GiftBomb => (event.gifts, *min_gifts, "gifted subs"),
                EventKind::Raid => (event.viewers, *min_viewers, "viewers"),
                EventKind::Cheer => (event.bits, *min_bits, "bits"),
                EventKind::Announcement => (0, 0, ""),
            };
            if have >= need {
                Ok("One of this rule's events")
            } else {
                Err(format!("Fewer than {need} {what}"))
            }
        }
        Trigger::BadgeDrop { on, .. } => match &facts.badge {
            Some(seen) if badges::wants(*on, seen) => Ok("Badge news this rule posts"),
            Some(_) => Err("Not the badge news this rule posts".to_string()),
            None => Err("Not a badge drop".to_string()),
        },
        Trigger::Whisper { text } => {
            if facts.kind != RowKind::Whisper {
                return Err("Not a whisper".to_string());
            }
            match text.as_ref().filter(|t| !t.pattern.trim().is_empty()) {
                None => Ok("A whisper"),
                Some(_) if cr.test.matches(facts) => Ok("The whisper matches"),
                Some(_) => Err("The whisper doesn't match".to_string()),
            }
        }
        Trigger::Timer { .. } | Trigger::Clock { .. } | Trigger::Uptime { .. } | Trigger::Delay { .. } => {
            if facts.kind == RowKind::Timed {
                Ok("Its time")
            } else {
                Err("Not a timed fire".to_string())
            }
        }
    }
}

fn channel_fit(cr: &CompiledRule, facts: &Facts) -> Result<&'static str, &'static str> {
    match &cr.rule.only_if.channels {
        ChannelScope::Own {} => {
            if !cr.own_key.is_empty() && facts.channel_key == cr.own_key {
                Ok("Your channel")
            } else {
                Err("Not your channel")
            }
        }
        ChannelScope::These { .. } => {
            if cr.these_keys.iter().any(|k| *k == facts.channel_key) {
                Ok("One of this rule's channels")
            } else {
                Err("Not one of this rule's channels")
            }
        }
        ChannelScope::AnyOpen {} => Ok("Any chat you have open (checked when it acts)"),
        ChannelScope::Watching {} => Ok("The stream you're watching (checked when it acts)"),
    }
}

/// Whether the local time is inside a rule's hours. `to` before `from`
/// crosses midnight, and the hours after midnight belong to the day they
/// started on (Friday 22:00 to 02:00 covers Saturday 01:30). Equal times mean
/// all day; no days means every day. Hours that cannot be read never hold a
/// rule back (saving refuses them).
pub fn in_hours(hours: &Hours, local: LocalTime) -> bool {
    let (Some(from), Some(to)) = (parse_hhmm(&hours.from), parse_hhmm(&hours.to)) else {
        return true;
    };
    let m = local.minute;
    let day = if from == to {
        Some(local.weekday)
    } else if from < to {
        (m >= from && m < to).then_some(local.weekday)
    } else if m >= from {
        Some(local.weekday)
    } else if m < to {
        Some(day_before(local.weekday))
    } else {
        None
    };
    match day {
        Some(day) => hours.days.is_empty() || hours.days.contains(&day),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::{BadgeLeg, BadgeLink, ChannelScope, MatchMode, Rule, RunState, SenderRole, TextMatch};
    use crate::services::account_automations::badges::{BadgeInfo, BadgePhase, BadgeSeen, Presence};
    use crate::services::account_automations::compile::fixtures::{chat, compiled_with, reply, rule, send, ME};
    use crate::services::account_automations::facts::{from_badge, from_chat, rows};

    fn noon() -> LocalTime {
        LocalTime { minute: 12 * 60, weekday: Weekday::Wed }
    }

    fn ctx() -> Ctx<'static> {
        Ctx { away: None, local: noon(), trace: false }
    }

    fn row(text: &str) -> Facts {
        from_chat(&rows::privmsg("m1", "me", ME, "fan", "300", text)).unwrap()
    }

    fn notice(msg_id: &str, params: &[(&str, &str)]) -> Facts {
        from_chat(&rows::usernotice("n1", "me", ME, "fan", "300", msg_id, params)).unwrap()
    }

    fn fires_in(r: Rule, facts: &Facts, ctx: &Ctx) -> bool {
        let c = compiled_with(&[r], RunState::On);
        evaluate_rule(&c, 0, facts, ctx).fired
    }

    fn fires(r: Rule, facts: &Facts) -> bool {
        fires_in(r, facts, &ctx())
    }

    fn mention(replies_too: bool) -> Rule {
        rule("r", Trigger::Mention { replies_too }, vec![reply("hi")])
    }

    fn events(kinds: Vec<EventKind>) -> Rule {
        rule(
            "r",
            Trigger::ChannelEvent { events: kinds, min_months: 0, min_gifts: 0, min_viewers: 0, min_bits: 0 },
            vec![reply("thanks")],
        )
    }

    #[test]
    fn message_rules_read_chat_messages_only() {
        let r = rule(
            "r",
            Trigger::Message { text: TextMatch { mode: MatchMode::AnyWord, pattern: "hello".into(), case_sensitive: false } },
            vec![reply("hi")],
        );
        assert!(fires(r.clone(), &row("well Hello there")));
        assert!(!fires(r.clone(), &row("goodbye")));
        assert!(!fires(r, &notice("resub", &[])));
    }

    #[test]
    fn mentions_follow_the_owners_current_login_and_replies_their_id() {
        // The stored login is stale; the compile reads the current one by id.
        let mut r = mention(true);
        r.owner.login = "oldname".into();
        assert!(fires(r.clone(), &row("hey @Me!")));
        assert!(!fires(r.clone(), &row("hey @oldname")));
        assert!(!fires(r.clone(), &row("hey @meme")));
        let mut reply_row = row("sure thing");
        reply_row.reply_parent_user_id = ME.into();
        assert!(fires(r.clone(), &reply_row));
        assert!(!fires(mention(false), &reply_row));
        let mut to_other = row("sure");
        to_other.reply_parent_user_id = "555".into();
        assert!(!fires(r, &to_other));
    }

    #[test]
    fn first_messages_fire_but_never_from_a_partner() {
        let r = rule("r", Trigger::FirstMessage {}, vec![reply("Welcome")]);
        let mut first = row("hi all");
        first.first_message = true;
        assert!(fires(r.clone(), &first));
        assert!(!fires(r.clone(), &row("hi again")));
        let mut partner = first.clone();
        partner.shared_partner = true;
        let mut sharing = r;
        sharing.only_if.shared_chat = true;
        assert!(!fires(sharing, &partner));
    }

    #[test]
    fn channel_events_match_their_kinds_and_thresholds() {
        let mut subs = events(vec![EventKind::Sub, EventKind::Resub]);
        if let Trigger::ChannelEvent { min_months, .. } = &mut subs.when {
            *min_months = 6;
        }
        assert!(fires(subs.clone(), &notice("resub", &[("msg-param-cumulative-months", "12")])));
        assert!(!fires(subs.clone(), &notice("resub", &[("msg-param-cumulative-months", "3")])));
        assert!(!fires(subs, &notice("raid", &[("msg-param-viewerCount", "500")])));

        let mut raids = events(vec![EventKind::Raid]);
        if let Trigger::ChannelEvent { min_viewers, .. } = &mut raids.when {
            *min_viewers = 10;
        }
        assert!(fires(raids.clone(), &notice("raid", &[("msg-param-viewerCount", "10")])));
        assert!(!fires(raids, &notice("raid", &[("msg-param-viewerCount", "9")])));

        // A gift bomb is thanked once, never per gifted sub.
        let gifts = events(vec![EventKind::Gift, EventKind::GiftBomb]);
        assert!(fires(gifts.clone(), &notice("submysterygift", &[("msg-param-mass-gift-count", "5")])));
        assert!(!fires(gifts.clone(), &notice("subgift", &[("msg-param-community-gift-id", "77")])));
        assert!(fires(gifts.clone(), &notice("subgift", &[])));
        assert!(fires(events(vec![EventKind::Announcement]), &notice("announcement", &[])));

        // A cheer is a chat message with bits.
        let mut cheers = events(vec![EventKind::Cheer]);
        if let Trigger::ChannelEvent { min_bits, .. } = &mut cheers.when {
            *min_bits = 100;
        }
        let cheer = |bits: u32| {
            let mut m = rows::privmsg("c1", "me", ME, "fan", "300", "Cheer");
            m.metadata.bits_amount = Some(bits);
            from_chat(&m).unwrap()
        };
        assert!(fires(cheers.clone(), &cheer(500)));
        assert!(!fires(cheers, &cheer(50)));

        // A partner's notice never starts an event rule, even with Shared Chat on.
        let mut shared = gifts.clone();
        shared.only_if.shared_chat = true;
        shared.only_if.channels = ChannelScope::AnyOpen {};
        let partner = from_chat(&rows::shared(
            rows::usernotice("n2", "partner", "600", "fan", "300", "submysterygift", &[("msg-param-mass-gift-count", "5")]),
            "n-origin",
            "500",
        ))
        .unwrap();
        assert!(!fires(shared, &partner));
        // An event rule never fires on a plain message.
        assert!(!fires(gifts, &row("hello")));
    }

    #[test]
    fn channels_own_these_and_any_open() {
        let own = mention(false);
        let elsewhere = from_chat(&rows::privmsg("m2", "friend", "700", "fan", "300", "@me hi")).unwrap();
        assert!(fires(own.clone(), &row("@me hi")));
        assert!(!fires(own.clone(), &elsewhere));
        let mut these = own.clone();
        these.only_if.channels = ChannelScope::These { list: vec![chat("friend", "700")] };
        assert!(fires(these.clone(), &elsewhere));
        assert!(!fires(these, &row("@me hi")));
        let mut open = own;
        open.only_if.channels = ChannelScope::AnyOpen {};
        assert!(fires(open.clone(), &elsewhere) && fires(open, &row("@me hi")));
    }

    #[test]
    fn roles_and_people_lists() {
        let plain = row("@me hi");
        let mut m = rows::privmsg("m3", "me", ME, "amod", "301", "@me hi");
        m.badges = vec![rows::badge("moderator"), rows::badge("subscriber")];
        let modrow = from_chat(&m).unwrap();

        let mut mods = mention(false);
        mods.only_if.roles = vec![SenderRole::Moderator, SenderRole::Broadcaster];
        assert!(fires(mods.clone(), &modrow) && !fires(mods, &plain));
        let mut viewers = mention(false);
        viewers.only_if.roles = vec![SenderRole::Viewer];
        assert!(fires(viewers.clone(), &plain) && !fires(viewers, &modrow));
        assert!(fires(mention(false), &plain) && fires(mention(false), &modrow));

        let mut skip = mention(false);
        skip.only_if.skip_users = vec!["@Fan".into()];
        assert!(!fires(skip.clone(), &plain) && fires(skip, &modrow));
        let mut only = mention(false);
        only.only_if.only_users = vec!["amod".into()];
        assert!(fires(only.clone(), &modrow) && !fires(only, &plain));
    }

    #[test]
    fn away_rules_answer_only_while_this_device_is_away() {
        let mut r = mention(false);
        r.only_if.only_away = true;
        assert!(!fires(r.clone(), &row("@me hi")));
        let away = Away { message: "brb".into(), since_ms: 0 };
        assert!(fires_in(r, &row("@me hi"), &Ctx { away: Some(&away), local: noon(), trace: false }));
    }

    #[test]
    fn hours_cross_midnight_and_count_the_day_they_started() {
        let at = |h: u32, m: u32, weekday: Weekday| LocalTime { minute: h * 60 + m, weekday };
        let night = Hours { from: "22:00".into(), to: "02:00".into(), days: vec![Weekday::Fri] };
        assert!(in_hours(&night, at(23, 0, Weekday::Fri)));
        assert!(in_hours(&night, at(1, 30, Weekday::Sat)));
        assert!(!in_hours(&night, at(2, 0, Weekday::Sat)));
        assert!(!in_hours(&night, at(1, 30, Weekday::Fri)));
        assert!(!in_hours(&night, at(23, 0, Weekday::Sat)));
        assert!(!in_hours(&night, at(12, 0, Weekday::Fri)));
        let day = Hours { from: "09:00".into(), to: "17:00".into(), days: Vec::new() };
        assert!(in_hours(&day, at(9, 0, Weekday::Sun)));
        assert!(!in_hours(&day, at(17, 0, Weekday::Sun)));
        assert!(!in_hours(&day, at(8, 59, Weekday::Mon)));
        let all_day = Hours { from: "00:00".into(), to: "00:00".into(), days: vec![Weekday::Mon] };
        assert!(in_hours(&all_day, at(5, 0, Weekday::Mon)) && !in_hours(&all_day, at(5, 0, Weekday::Tue)));

        let mut r = mention(false);
        r.only_if.hours = Some(day);
        assert!(fires_in(r.clone(), &row("@me"), &Ctx { away: None, local: at(10, 0, Weekday::Mon), trace: false }));
        assert!(!fires_in(r, &row("@me"), &Ctx { away: None, local: at(20, 0, Weekday::Mon), trace: false }));

        let friday_night = chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap().and_hms_opt(23, 30, 0).unwrap();
        assert_eq!(LocalTime::from_naive(friday_night), at(23, 30, Weekday::Fri));
    }

    #[test]
    fn partner_chat_rows_reach_only_rules_that_take_shared_chat() {
        let mut r = mention(false);
        r.only_if.channels = ChannelScope::AnyOpen {};
        let partner =
            from_chat(&rows::shared(rows::privmsg("copy", "partner", "600", "fan", "300", "@me hi"), "origin", "500")).unwrap();
        assert!(!fires(r.clone(), &partner));
        r.only_if.shared_chat = true;
        assert!(fires(r, &partner));
    }

    #[test]
    fn badge_rules_fire_on_the_legs_they_want() {
        let seen = |announced: bool, earnable: bool| BadgeSeen {
            badge: BadgeInfo::default(),
            feed_id: "x-v1".into(),
            phase: if earnable { BadgePhase::Earnable } else { BadgePhase::Announced },
            announced,
            earnable,
            ts_ms: 0,
            arrived_ms: 0,
            push: true,
            in_gallery: Presence::No,
        };
        let badge_rule = |on: BadgeLeg| {
            rule("b", Trigger::BadgeDrop { on, link: BadgeLink::default(), batch_text: String::new() }, vec![send("{badge}")])
        };
        let announced = from_badge(&seen(true, false));
        let opened = from_badge(&seen(false, true));
        let at_once = from_badge(&seen(true, true));
        for (on, a, o) in [(BadgeLeg::Announced, true, false), (BadgeLeg::Earnable, false, true), (BadgeLeg::Both, true, true)] {
            assert_eq!(fires(badge_rule(on), &announced), a, "{on:?}");
            assert_eq!(fires(badge_rule(on), &opened), o, "{on:?}");
            assert!(fires(badge_rule(on), &at_once), "{on:?}");
        }
        // A badge rule never fires on a chat row, and a chat rule never on a badge.
        assert!(!fires(badge_rule(BadgeLeg::Both), &row("hi")));
        assert!(!fires(mention(true), &announced));
    }

    #[test]
    fn traces_explain_every_condition_for_try_it() {
        let mut r = mention(false);
        r.only_if.roles = vec![SenderRole::Moderator];
        r.only_if.only_away = true;
        r.only_if.only_live = true;
        let c = compiled_with(&[r], RunState::On);
        let quiet = evaluate_rule(&c, 0, &row("no mention here"), &ctx());
        assert!(!quiet.fired);
        assert!(quiet.trace.is_empty());
        assert_eq!(quiet.reason.as_deref(), Some("Doesn't mention @me"));
        let traced = evaluate_rule(&c, 0, &row("no mention here"), &Ctx { away: None, local: noon(), trace: true });
        let steps: Vec<(&str, bool)> = traced.trace.iter().map(|t| (t.step, t.ok)).collect();
        assert_eq!(
            steps,
            vec![
                ("When", false),
                ("Shared Chat", true),
                ("Channels", true),
                ("From", false),
                ("Skip", true),
                ("Only", true),
                ("Away", false),
                ("Live", true),
            ]
        );
        assert_eq!(traced.reason.as_deref(), Some("Doesn't mention @me"));
        assert!(!traced.fired);
        // A rule that fires explains why each condition passed.
        let c = compiled_with(&[mention(false)], RunState::On);
        let ok = evaluate_rule(&c, 0, &row("@me hi"), &Ctx { away: None, local: noon(), trace: true });
        assert!(ok.fired && ok.reason.is_none());
        assert_eq!(ok.trace[0], Trace { step: "When", ok: true, detail: "Mentions you".into() });
    }

    #[test]
    fn evaluate_reaches_only_the_rules_of_the_rows_chat() {
        let own = rule("own", Trigger::Mention { replies_too: false }, vec![reply("hi")]);
        let mut friend = rule("friend", Trigger::Mention { replies_too: false }, vec![reply("hi")]);
        friend.only_if.channels = ChannelScope::These { list: vec![chat("friend", "700")] };
        let c = compiled_with(&[own, friend], RunState::On);
        let verdicts = evaluate(&c, &row("@me hi"), &ctx());
        assert_eq!(verdicts.iter().map(|v| (v.rule_id.as_str(), v.fired)).collect::<Vec<_>>(), vec![("own", true)]);
        // Try it reads every rule.
        assert_eq!(try_all(&c, &row("@me hi"), &ctx()).len(), 2);
        // With every rule Off nothing is reached, and Try it still explains them.
        let off = compiled_with(&[mention(false)], RunState::Off);
        assert!(evaluate(&off, &row("@me hi"), &ctx()).is_empty());
        assert!(try_all(&off, &row("@me hi"), &ctx())[0].fired);
    }
}
