//! A rule's message: its variables, the chatter-text rule, cleaning and the
//! invisible variation.
//!
//! Expansion uses the custom-command grammar (`slash_dispatch::expand_with`):
//! `{{` and `}}` are literal, an unknown name stays as typed, and an empty
//! value leaves no double space. Every outward text is cleaned before it is
//! booked: newlines become spaces, the ends are trimmed, it is cut to 500
//! characters (498 with `vary`, so the variation still fits), and a text that
//! starts with `/` or `.` is refused, so a rule never runs a chat command.

use chrono::NaiveDateTime;
use serde::Serialize;

use super::badges::{self, badge_value, BadgeInfo, BadgeSeen};
use super::facts::{EventFacts, Facts, MESSAGE_VAR_CHARS};
use crate::models::automations::{is_chatter_text, placeholders, Away, ChannelScope, EventKind, Rule, SenderRole};
use crate::services::chat_burst::VARY_SUFFIX;
use crate::services::slash_dispatch::expand_with;

/// Characters in one chat message.
pub const MAX_TEXT_CHARS: usize = 500;
/// Characters in a message that may get the variation appended.
pub const MAX_VARIED_CHARS: usize = 498;
/// Activity words when a text with the chatter's words cannot go out here.
pub const CHATTER_TEXT_SKIP: &str =
    "Skipped: the chatter's words go out only in your own channel, from a rule that answers only you and your moderators";

/// Every value a rule's text can name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vars {
    pub user: String,
    pub user_login: String,
    pub user_id: String,
    /// The sending account's display name.
    pub me: String,
    pub channel: String,
    pub channel_login: String,
    pub channel_id: String,
    /// Times this rule acted (Live).
    pub count: u64,
    pub event: String,
    pub months: Option<u32>,
    pub gifts: Option<u32>,
    pub viewers: Option<u32>,
    pub bits: Option<u32>,
    pub tier: String,
    pub title: String,
    pub game: String,
    pub uptime: String,
    /// This device's away message; empty while not away.
    pub away: String,
    /// How long ago away was set ("12 minutes").
    pub away_since: String,
    pub time: String,
    pub date: String,
    /// The chatter's text, at most 200 characters.
    pub message: String,
    /// The chatter's words, for `{1}`, `{2+}` and `{*}`.
    pub args: Vec<String>,
    pub badge: Option<BadgeInfo>,
    pub badge_link: String,
    pub badge_count: usize,
    /// The batch's badge names ("A, B and C").
    pub badges: String,
}

impl Vars {
    /// The values the row itself gives; the executor adds the rest (`me`,
    /// `count`, the stream's title, game and uptime, away, the time, the link).
    pub fn from_facts(facts: &Facts) -> Vars {
        let mut vars = Vars {
            user: facts.user_name.clone(),
            user_login: facts.user_login.clone(),
            user_id: facts.user_id.clone(),
            channel: facts.channel_login.clone(),
            channel_login: facts.channel_login.clone(),
            channel_id: facts.room_id.clone(),
            message: facts.text.trim().chars().take(MESSAGE_VAR_CHARS).collect(),
            args: facts.text.split_whitespace().map(str::to_string).collect(),
            badge: facts.badge.as_ref().map(|seen| seen.badge.clone()),
            ..Vars::default()
        };
        if let Some(event) = &facts.event {
            vars.event = event_words(event);
            vars.tier = event.tier.clone();
            match event.kind {
                EventKind::Sub | EventKind::Resub => vars.months = Some(event.months),
                EventKind::Gift | EventKind::GiftBomb => vars.gifts = Some(event.gifts),
                EventKind::Raid => vars.viewers = Some(event.viewers),
                EventKind::Cheer => vars.bits = Some(event.bits),
                EventKind::Announcement => {}
            }
        }
        vars
    }

    /// `{away}` and `{away.since}` from this device's away.
    pub fn set_away(&mut self, away: Option<&Away>, now_ms: i64) {
        match away {
            Some(away) => {
                self.away = away.message.clone();
                self.away_since = since_words(away.since_ms, now_ms);
            }
            None => {
                self.away.clear();
                self.away_since.clear();
            }
        }
    }

    /// `{time}` and `{date}` from the local time.
    pub fn set_local(&mut self, local: NaiveDateTime) {
        self.time = time_words(local);
        self.date = date_words(local);
    }

    /// `{badge.count}` and `{badges}` for the drops a batch posts.
    pub fn set_batch(&mut self, drops: &[&BadgeSeen]) {
        self.badge_count = drops.len();
        self.badges = badges::badge_names(drops);
        if let [one] = drops {
            self.badge = Some(one.badge.clone());
        }
    }

    /// A name's value, in any case; None for a name that is not a variable
    /// (it stays as typed).
    pub fn lookup(&self, name: &str) -> Option<String> {
        let name = name.to_ascii_lowercase();
        let number = |n: Option<u32>| n.map_or_else(String::new, |n| n.to_string());
        Some(match name.as_str() {
            "user" | "user.name" => self.user.clone(),
            "user.login" => self.user_login.clone(),
            "user.id" => self.user_id.clone(),
            "me" => self.me.clone(),
            "channel" | "channel.name" => self.channel.clone(),
            "channel.login" => self.channel_login.clone(),
            "channel.id" => self.channel_id.clone(),
            "count" => self.count.to_string(),
            "event" => self.event.clone(),
            "months" => number(self.months),
            "gifts" => number(self.gifts),
            "viewers" => number(self.viewers),
            "bits" => number(self.bits),
            "tier" => self.tier.clone(),
            "title" => self.title.clone(),
            "game" => self.game.clone(),
            "uptime" => self.uptime.clone(),
            "away" => self.away.clone(),
            "away.since" => self.away_since.clone(),
            "time" => self.time.clone(),
            "date" => self.date.clone(),
            "message" => self.message.clone(),
            "badge.link" => self.badge_link.clone(),
            "badge.count" => self.badge_count.to_string(),
            "badges" => self.badges.clone(),
            other => return self.badge.as_ref().and_then(|badge| badge_value(badge, other)),
        })
    }
}

/// `text` with its variables filled in.
pub fn expand(text: &str, vars: &Vars) -> String {
    expand_with(text, &vars.args, |name| vars.lookup(name)).text
}

/// Why a text cannot go out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CleanError {
    Empty,
    Command,
}

impl CleanError {
    pub fn words(self) -> &'static str {
        match self {
            CleanError::Empty => "Skipped: nothing to send once the variables were filled in",
            CleanError::Command => "Skipped: the message would start with / or ., which runs a chat command",
        }
    }
}

/// An outward text as it is booked and sent: one line, trimmed, at most 500
/// characters (498 with `vary`), never starting with `/` or `.`.
pub fn clean(text: &str, vary: bool) -> Result<String, CleanError> {
    let one_line: String = text.replace("\r\n", " ").chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect();
    let trimmed = one_line.trim();
    if trimmed.is_empty() {
        return Err(CleanError::Empty);
    }
    if trimmed.starts_with('/') || trimmed.starts_with('.') {
        return Err(CleanError::Command);
    }
    let cap = if vary { MAX_VARIED_CHARS } else { MAX_TEXT_CHARS };
    let cut: String = trimmed.chars().take(cap).collect();
    Ok(cut.trim_end().to_string())
}

/// The invisible variation: a booking refused as a duplicate of the last
/// message books once more with it.
pub fn varied(text: &str) -> String {
    format!("{text}{VARY_SUFFIX}")
}

/// The text names the chatter's own words: `{message}`, `{N}`, `{N+}`, `{*}`.
pub fn uses_chatter_text(text: &str) -> bool {
    placeholders(text).iter().any(|name| is_chatter_text(name))
}

/// The text names this device's away (`{away}` or `{away.since}`).
pub fn uses_away(text: &str) -> bool {
    placeholders(text).iter().any(|name| matches!(name.to_ascii_lowercase().as_str(), "away" | "away.since"))
}

/// The chatter's words may go out only in your own channel, from a rule that
/// answers only you and your moderators, and never from a Shared Chat
/// partner's row (checked as the rule acts; saving checks the rest).
pub fn chatter_text_allowed(rule: &Rule, facts: &Facts) -> bool {
    matches!(rule.only_if.channels, ChannelScope::Own {})
        && !rule.only_if.roles.is_empty()
        && rule.only_if.roles.iter().all(|r| matches!(r, SenderRole::Broadcaster | SenderRole::Moderator))
        && !facts.shared_partner
}

/// Whether an outward template may go out for this row. Notify texts are
/// local and always may.
pub fn outward_text_ok(rule: &Rule, facts: &Facts, template: &str) -> Result<(), &'static str> {
    if uses_chatter_text(template) && !chatter_text_allowed(rule, facts) {
        Err(CHATTER_TEXT_SKIP)
    } else {
        Ok(())
    }
}

fn count_words(n: u32, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// `{event}`: "subscribed for 12 months", "gifted 5 subs", "raided with 120
/// viewers", "cheered 500 bits".
pub fn event_words(event: &EventFacts) -> String {
    match event.kind {
        EventKind::Sub | EventKind::Resub if event.months > 1 => format!("subscribed for {} months", event.months),
        EventKind::Sub | EventKind::Resub => "subscribed".to_string(),
        EventKind::Gift => "gifted a sub".to_string(),
        EventKind::GiftBomb if event.gifts == 1 => "gifted a sub".to_string(),
        EventKind::GiftBomb => format!("gifted {} subs", event.gifts),
        EventKind::Raid => format!("raided with {}", count_words(event.viewers, "viewer")),
        EventKind::Cheer => format!("cheered {}", count_words(event.bits, "bit")),
        EventKind::Announcement => "made an announcement".to_string(),
    }
}

fn count_long(n: i64, unit: &str) -> String {
    if n == 1 {
        format!("1 {unit}")
    } else {
        format!("{n} {unit}s")
    }
}

/// `{away.since}`: "12 minutes", "2 hours 5 minutes", "3 days".
pub fn since_words(since_ms: i64, now_ms: i64) -> String {
    let minutes = (now_ms - since_ms).max(0) / 60_000;
    if minutes < 1 {
        return "less than a minute".to_string();
    }
    if minutes < 60 {
        return count_long(minutes, "minute");
    }
    let (hours, rest) = (minutes / 60, minutes % 60);
    if hours < 24 {
        return if rest == 0 { count_long(hours, "hour") } else { format!("{} {}", count_long(hours, "hour"), count_long(rest, "minute")) };
    }
    count_long(hours / 24, "day")
}

/// `{uptime}`: "2h 5m", "45m".
pub fn uptime_words(started_ms: i64, now_ms: i64) -> String {
    let minutes = (now_ms - started_ms).max(0) / 60_000;
    let (hours, rest) = (minutes / 60, minutes % 60);
    if hours > 0 {
        format!("{hours}h {rest}m")
    } else {
        format!("{rest}m")
    }
}

/// `{time}`: "3:45 PM".
pub fn time_words(local: NaiveDateTime) -> String {
    local.format("%-I:%M %p").to_string()
}

/// `{date}`: "Nov 13".
pub fn date_words(local: NaiveDateTime) -> String {
    local.format("%b %-d").to_string()
}

/// Where a variable may go.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VariableScope {
    Any,
    /// Notify; outward only for rules in your own channel.
    OwnChannel,
    /// Notify; outward only in your own channel, from a rule that answers
    /// only you and your moderators, never from a Shared Chat partner.
    Chatter,
    /// Badge rules only.
    Badge,
}

/// One row of the variables the editors show (served by `automations_variables`).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct VariableRow {
    pub names: &'static [&'static str],
    pub value: &'static str,
    pub scope: VariableScope,
    pub allowed: &'static str,
}

const ANYWHERE: &str = "Any rule";
const OWN_ONLY: &str = "Notifications; messages only from rules in your own channel";
const CHATTER_ONLY: &str = "Notifications; messages only from rules in your own channel that answer only you and your moderators, never from a Shared Chat partner";
const BADGE_ONLY: &str = "Badge rules";

/// The variables, the same on desktop, Android and iPhone.
pub fn variable_table() -> Vec<VariableRow> {
    use VariableScope::*;
    vec![
        VariableRow { names: &["user", "user.login", "user.id"], value: "The person who made it fire", scope: Any, allowed: ANYWHERE },
        VariableRow { names: &["me"], value: "The name of the account the rule sends as", scope: Any, allowed: ANYWHERE },
        VariableRow { names: &["channel", "channel.login", "channel.id"], value: "The chat", scope: Any, allowed: ANYWHERE },
        VariableRow { names: &["count"], value: "How many times this rule has acted", scope: Any, allowed: ANYWHERE },
        VariableRow {
            names: &["event"],
            value: "What they did: \"subscribed for 12 months\", \"gifted 5 subs\", \"raided with 120 viewers\", \"cheered 500 bits\"",
            scope: Any,
            allowed: ANYWHERE,
        },
        VariableRow { names: &["months", "gifts", "viewers", "bits", "tier"], value: "The event's numbers", scope: Any, allowed: ANYWHERE },
        VariableRow { names: &["title", "game"], value: "The stream's title and game", scope: OwnChannel, allowed: OWN_ONLY },
        VariableRow { names: &["uptime"], value: "How long the stream has been live", scope: Any, allowed: ANYWHERE },
        VariableRow {
            names: &["away", "away.since"],
            value: "Your away message on this device, and how long ago you set it (\"12 minutes\")",
            scope: Any,
            allowed: ANYWHERE,
        },
        VariableRow { names: &["time", "date"], value: "The time and date on this device", scope: Any, allowed: ANYWHERE },
        VariableRow {
            names: &["message", "1", "2", "2+", "*"],
            value: "What the chatter wrote (up to 200 characters), one word of it, the words from one on, or all of it",
            scope: Chatter,
            allowed: CHATTER_ONLY,
        },
        VariableRow {
            names: &[
                "badge",
                "badge.id",
                "badge.version",
                "badge.about",
                "badge.image",
                "badge.when",
                "badge.how",
                "badge.game",
                "badge.event",
                "badge.link",
            ],
            value: "The new badge: its name, id, version, description (up to 120 characters), picture, dates, how to earn it, game, event and link",
            scope: Badge,
            allowed: BADGE_ONLY,
        },
        VariableRow {
            names: &["badge.count", "badges"],
            value: "For several badges in one message: how many, and their names",
            scope: Badge,
            allowed: BADGE_ONLY,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::{is_badge_variable, is_channel_state, Trigger};
    use crate::services::account_automations::badges::{BadgePhase, Presence};
    use crate::services::account_automations::compile::fixtures::{reply, rule, ME};
    use crate::services::account_automations::facts::{from_badge, from_chat, rows};

    fn row(text: &str) -> Facts {
        let mut m = rows::privmsg("m1", "me", ME, "fan", "300", text);
        m.display_name = "Fan".into();
        from_chat(&m).unwrap()
    }

    fn notice(msg_id: &str, params: &[(&str, &str)]) -> Vars {
        Vars::from_facts(&from_chat(&rows::usernotice("n1", "me", ME, "fan", "300", msg_id, params)).unwrap())
    }

    #[test]
    fn variables_fill_from_the_row_and_the_rule() {
        let mut vars = Vars::from_facts(&row("!so @friend now"));
        vars.me = "Me".into();
        vars.count = 4;
        assert_eq!(
            expand("Hi {user} (@{user.login}, {user.id}) from {me} in {channel} #{channel.id}: {count}", &vars),
            "Hi Fan (@fan, 300) from Me in me #100: 4"
        );
        assert_eq!(expand("{1} {2+} | {*} | {message}", &vars), "!so @friend now | !so @friend now | !so @friend now");
        // An unknown name stays as typed, doubled braces are literal, and an
        // empty value leaves no gap.
        assert_eq!(expand("{nope} {{x}} a {title} b", &vars), "{nope} {x} a b");
        assert_eq!(expand("{USER}", &vars), "Fan");
        // `{message}` carries at most 200 characters.
        let long = Vars::from_facts(&row(&"w".repeat(300)));
        assert_eq!(expand("{message}", &long).chars().count(), MESSAGE_VAR_CHARS);
    }

    #[test]
    fn events_read_as_words_and_numbers() {
        let resub = notice("resub", &[("msg-param-cumulative-months", "12"), ("msg-param-sub-plan", "2000")]);
        assert_eq!(expand("{event} {months} tier {tier}", &resub), "subscribed for 12 months 12 tier 2");
        assert_eq!(expand("{event}", &notice("sub", &[])), "subscribed");
        assert_eq!(expand("{event} {gifts}", &notice("submysterygift", &[("msg-param-mass-gift-count", "5")])), "gifted 5 subs 5");
        assert_eq!(expand("{event}", &notice("subgift", &[])), "gifted a sub");
        assert_eq!(expand("{event} ({viewers})", &notice("raid", &[("msg-param-viewerCount", "120")])), "raided with 120 viewers (120)");
        assert_eq!(expand("{event}", &notice("raid", &[("msg-param-viewerCount", "1")])), "raided with 1 viewer");
        assert_eq!(expand("{event}", &notice("announcement", &[])), "made an announcement");
        let mut m = rows::privmsg("c1", "me", ME, "fan", "300", "Cheer500");
        m.metadata.bits_amount = Some(500);
        assert_eq!(expand("{event} {bits}", &Vars::from_facts(&from_chat(&m).unwrap())), "cheered 500 bits 500");
        // Numbers an event does not have are empty.
        assert_eq!(expand("[{months}{gifts}{viewers}{bits}]", &resub), "[12]");
    }

    #[test]
    fn away_reads_this_devices_away() {
        let mut vars = Vars::from_facts(&row("@me hi"));
        vars.set_away(None, 0);
        assert_eq!(expand("I'm away right now: {away}", &vars), "I'm away right now:");
        let away = Away { message: "brb, dinner".into(), since_ms: 0 };
        vars.set_away(Some(&away), 12 * 60_000);
        assert_eq!(expand("{away} (since {away.since})", &vars), "brb, dinner (since 12 minutes)");
        vars.set_away(None, 0);
        assert_eq!(expand("[{away}{away.since}]", &vars), "[]");
        assert!(uses_away("x {Away.since}") && uses_away("{away}") && !uses_away("{awayx} {{away}}"));
        assert_eq!(since_words(0, 30_000), "less than a minute");
        assert_eq!(since_words(0, 60_000), "1 minute");
        assert_eq!(since_words(0, 120 * 60_000), "2 hours");
        assert_eq!(since_words(0, 125 * 60_000), "2 hours 5 minutes");
        assert_eq!(since_words(0, 3 * 24 * 3_600_000), "3 days");
        assert_eq!(since_words(10, 0), "less than a minute");
    }

    #[test]
    fn outward_text_is_cleaned_before_it_is_booked() {
        assert_eq!(clean("  Line one\r\nLine two\n", false), Ok("Line one Line two".to_string()));
        assert_eq!(clean(&"a".repeat(600), false).unwrap().chars().count(), MAX_TEXT_CHARS);
        assert_eq!(clean(&"a".repeat(600), true).unwrap().chars().count(), MAX_VARIED_CHARS);
        assert_eq!(clean(&"\u{e9}".repeat(600), false).unwrap().chars().count(), MAX_TEXT_CHARS);
        assert_eq!(clean("   ", false), Err(CleanError::Empty));
        // A filled-in message that would run a chat command is refused.
        let vars = Vars::from_facts(&row("/ban someone"));
        assert_eq!(clean(&expand("{message}", &vars), false), Err(CleanError::Command));
        assert_eq!(clean(" .mods", false), Err(CleanError::Command));
        assert_eq!(clean("ok /ban", false), Ok("ok /ban".to_string()));
        assert!(CleanError::Command.words().starts_with("Skipped"));
    }

    #[test]
    fn vary_fits_its_suffix_inside_the_limit() {
        let text = clean(&"x".repeat(700), true).unwrap();
        let again = varied(&text);
        assert_eq!(again.chars().count(), MAX_TEXT_CHARS);
        assert_ne!(again, text);
        assert!(again.starts_with(&text) && again.ends_with(VARY_SUFFIX));
    }

    #[test]
    fn the_chatters_words_go_out_only_where_the_rule_allows() {
        let mut mods = rule("r", Trigger::Mention { replies_too: false }, vec![reply("{message}")]);
        mods.only_if.roles = vec![SenderRole::Moderator, SenderRole::Broadcaster];
        let facts = row("@me hi");
        assert_eq!(outward_text_ok(&mods, &facts, "said: {message}"), Ok(()));
        assert_eq!(outward_text_ok(&mods, &facts, "said: {1}"), Ok(()));
        // A Shared Chat partner's row never posts their words.
        let mut partner = facts.clone();
        partner.shared_partner = true;
        assert_eq!(outward_text_ok(&mods, &partner, "said: {*}"), Err(CHATTER_TEXT_SKIP));
        // From anyone, or in another channel: never.
        let anyone = rule("r", Trigger::Mention { replies_too: false }, vec![reply("{message}")]);
        assert_eq!(outward_text_ok(&anyone, &facts, "{message}"), Err(CHATTER_TEXT_SKIP));
        let mut elsewhere = mods.clone();
        elsewhere.only_if.channels = ChannelScope::AnyOpen {};
        assert_eq!(outward_text_ok(&elsewhere, &facts, "{2+}"), Err(CHATTER_TEXT_SKIP));
        let mut with_vips = mods.clone();
        with_vips.only_if.roles.push(SenderRole::Vip);
        assert_eq!(outward_text_ok(&with_vips, &facts, "{message}"), Err(CHATTER_TEXT_SKIP));
        // A text without the chatter's words always goes.
        assert_eq!(outward_text_ok(&anyone, &partner, "Hi {user}"), Ok(()));
        assert!(uses_chatter_text("{3+}") && !uses_chatter_text("{user} {{message}}"));
    }

    #[test]
    fn badge_variables_fill_one_drop_or_a_batch() {
        let seen = BadgeSeen {
            badge: BadgeInfo {
                set_id: "x".into(),
                version: "2".into(),
                name: "Rotterdam".into(),
                how: "Watch 3 hours".into(),
                ..BadgeInfo::default()
            },
            feed_id: "x-v2".into(),
            phase: BadgePhase::Announced,
            announced: true,
            earnable: false,
            ts_ms: 0,
            arrived_ms: 0,
            push: true,
            in_gallery: Presence::No,
        };
        let mut vars = Vars::from_facts(&from_badge(&seen));
        vars.badge_link = "https://streamnook.app/badge/x/2".into();
        assert_eq!(
            expand("New Twitch badge just dropped: {badge}! {badge.how} {badge.link}", &vars),
            "New Twitch badge just dropped: Rotterdam! Watch 3 hours https://streamnook.app/badge/x/2"
        );
        assert_eq!(expand("{badge.id} v{badge.version} {badge.game}", &vars), "x v2");
        let other = BadgeSeen { badge: BadgeInfo { name: "Cologne".into(), ..BadgeInfo::default() }, feed_id: "y-v1".into(), ..seen.clone() };
        let mut batch = Vars::default();
        batch.set_batch(&[&seen, &other]);
        assert_eq!(
            expand("{badge.count} new Twitch badges just dropped: {badges}!", &batch),
            "2 new Twitch badges just dropped: Rotterdam and Cologne!"
        );
        // A chat rule has no badge, so badge names stay as typed (saving refuses them).
        assert_eq!(expand("{badge.how}", &Vars::from_facts(&row("hi"))), "{badge.how}");
    }

    #[test]
    fn local_time_and_uptime_words() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 11, 13).unwrap().and_hms_opt(15, 45, 0).unwrap();
        let mut vars = Vars::default();
        vars.set_local(t);
        assert_eq!(expand("{time} on {date}", &vars), "3:45 PM on Nov 13");
        assert_eq!(uptime_words(0, 125 * 60_000), "2h 5m");
        assert_eq!(uptime_words(0, 45 * 60_000), "45m");
    }

    #[test]
    fn the_variable_table_matches_the_save_rules() {
        let table = variable_table();
        let vars = Vars { badge: Some(BadgeInfo::default()), ..Vars::default() };
        for row in &table {
            for name in row.names {
                match row.scope {
                    VariableScope::Chatter => assert!(is_chatter_text(name), "{name}"),
                    VariableScope::OwnChannel => assert!(is_channel_state(name), "{name}"),
                    VariableScope::Badge => assert!(is_badge_variable(name), "{name}"),
                    VariableScope::Any => {
                        assert!(!is_chatter_text(name) && !is_channel_state(name) && !is_badge_variable(name), "{name}")
                    }
                }
                if row.scope != VariableScope::Chatter || *name == "message" {
                    assert!(vars.lookup(name).is_some(), "{name}");
                }
            }
        }
        for name in [
            "user", "me", "channel", "count", "event", "months", "title", "uptime", "away", "away.since", "time", "date",
            "message", "badge", "badge.link", "badges",
        ] {
            assert!(table.iter().any(|r| r.names.contains(&name)), "{name}");
        }
    }
}
