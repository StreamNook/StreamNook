//! Rules as the engine runs them, compiled once whenever settings, accounts
//! or the main account change, then read on every row without a lock.
//!
//! Compile reads the stored raw rules, this device's preferences and your
//! accounts (handed in by the caller from synchronous reads: the registry's
//! accounts and the profile's id; nothing here reads a file or waits on the
//! network) and builds an immutable
//! [`Compiled`]: each rule typed with its text test and summary, the chat
//! index, the trigger families in use, your own account ids (your own
//! messages never trigger a rule), the chats to keep joined and the chats the
//! header dot marks. A rule this build cannot read, or whose accounts are not
//! signed in, is held with the reason its row shows. Whether an account's
//! token really is that account's is never decided here: the job verifies it
//! with Twitch before it claims.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::{Regex, RegexBuilder};
use serde_json::Value;

use super::facts::{fold, nfc, Facts, RowKind};
use crate::models::automations::{
    Action, Automations, AutomationPrefs, BadgeLeg, ChannelRef, ChannelScope, EventKind, MatchMode, Rule, RunState,
    TextMatch, Trigger, MAX_ENGINE_CHATS, REGEX_SIZE_LIMIT,
};
use crate::services::chat_rules::channel_filter_key;

/// Trigger families with a running rule: chat rows (messages, mentions,
/// first messages, channel events). Later phases add whispers, timers and
/// live changes as further bits.
pub const FAMILY_CHAT: u32 = 1;
/// Badge drops.
pub const FAMILY_BADGE: u32 = 1 << 1;
/// The `my_ids` key of Twitch ids.
pub const TWITCH: &str = "twitch";

/// The row of a rule a newer StreamNook wrote.
pub const NEWER_BUILD: &str = "Made in a newer StreamNook";

/// How a running rule acts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// It writes what it would do to Activity and sends nothing.
    Test,
    Live,
}

/// A rule's mode under the run state: none while Off, every rule Test while
/// Test, and each rule's own Test or Live while On.
pub fn mode_for(run: RunState, live: bool) -> Option<Mode> {
    match run {
        RunState::Off => None,
        RunState::Test => Some(Mode::Test),
        RunState::On => Some(if live { Mode::Live } else { Mode::Test }),
    }
}

/// A Message trigger's test, built once. Every mode but Regex compares the
/// message in NFC, lowercased unless case-sensitive, the way the pattern was
/// read. Words and phrases match whole words.
#[derive(Clone, Debug)]
pub enum TextTest {
    /// Not a Message rule, or an empty pattern.
    Never,
    AnyWord { words: Vec<String>, cs: bool },
    AllWords { words: Vec<String>, cs: bool },
    Phrase { phrase: String, cs: bool },
    Contains { needle: String, cs: bool },
    StartsWith { prefix: String, cs: bool },
    Exact { text: String, cs: bool },
    /// Built with the 64 KiB size limit; matched against the text as written.
    Regex(Regex),
}

impl TextTest {
    pub fn build(text: &TextMatch) -> Result<TextTest, &'static str> {
        let cs = text.case_sensitive;
        if text.pattern.trim().is_empty() {
            return Ok(TextTest::Never);
        }
        let read = || if cs { nfc(text.pattern.trim()).into_owned() } else { fold(text.pattern.trim()) };
        let words = || read().split_whitespace().map(str::to_string).collect::<Vec<_>>();
        Ok(match text.mode {
            MatchMode::Regex => TextTest::Regex(
                RegexBuilder::new(&text.pattern)
                    .case_insensitive(!cs)
                    .size_limit(REGEX_SIZE_LIMIT)
                    .build()
                    .map_err(|e| match e {
                        regex::Error::CompiledTooBig(_) => "This pattern is too large to run",
                        _ => "This isn't a valid regular expression",
                    })?,
            ),
            MatchMode::AnyWord => TextTest::AnyWord { words: words(), cs },
            MatchMode::AllWords => TextTest::AllWords { words: words(), cs },
            MatchMode::Phrase => TextTest::Phrase { phrase: words().join(" "), cs },
            MatchMode::Contains => TextTest::Contains { needle: read(), cs },
            MatchMode::StartsWith => TextTest::StartsWith { prefix: read(), cs },
            MatchMode::Exact => TextTest::Exact { text: read(), cs },
        })
    }

    pub fn matches(&self, facts: &Facts) -> bool {
        let hay = |cs: bool| if cs { facts.text_nfc.as_str() } else { facts.text_folded.as_str() };
        match self {
            TextTest::Never => false,
            TextTest::AnyWord { words, cs } => words.iter().any(|w| bounded(hay(*cs), w)),
            TextTest::AllWords { words, cs } => !words.is_empty() && words.iter().all(|w| bounded(hay(*cs), w)),
            TextTest::Phrase { phrase, cs } => bounded(hay(*cs), phrase),
            TextTest::Contains { needle, cs } => hay(*cs).contains(needle.as_str()),
            TextTest::StartsWith { prefix, cs } => hay(*cs).trim_start().starts_with(prefix.as_str()),
            TextTest::Exact { text, cs } => hay(*cs).trim() == text.as_str(),
            TextTest::Regex(re) => re.is_match(&facts.text),
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `needle` appears in `hay` as whole words: no letter, digit or `_` touches
/// it on a side where the needle itself starts or ends with one.
pub fn bounded(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let first_word = needle.chars().next().is_some_and(is_word);
    let last_word = needle.chars().next_back().is_some_and(is_word);
    let step = needle.chars().next().map_or(1, char::len_utf8);
    let mut from = 0;
    while let Some(pos) = hay[from..].find(needle) {
        let start = from + pos;
        let end = start + needle.len();
        let before = !first_word || hay[..start].chars().next_back().map_or(true, |c| !is_word(c));
        let after = !last_word || hay[end..].chars().next().map_or(true, |c| !is_word(c));
        if before && after {
            return true;
        }
        from = start + step;
    }
    false
}

/// One stored rule, compiled.
#[derive(Clone, Debug)]
pub struct CompiledRule {
    /// The rule with its forced values applied (a badge rule's channel,
    /// only-live and cooldown; send as the owner when unset; tidy people lists).
    pub rule: Rule,
    /// Its place in the stored list.
    pub position: usize,
    /// How it runs now; None while it does not (switched off, held, or Off).
    pub mode: Option<Mode>,
    /// Why it cannot run, for its row; None when it runs or is only switched off.
    pub held: Option<String>,
    /// The owner's current login (by id), lowercase.
    pub owner_login: String,
    pub send_as_login: String,
    /// `{me}`: the sending account's display name.
    pub send_as_name: String,
    /// `twitch:<owner login>`: the chat "your channel" means.
    pub own_key: String,
    /// The chats a These rule listens in, as `twitch:<login>`.
    pub these_keys: Vec<String>,
    pub test: TextTest,
    pub summary: String,
    /// It has an action that posts.
    pub outward: bool,
}

/// A stored rule this build cannot read; kept as it is and shown read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unreadable {
    pub position: usize,
    pub id: String,
    pub name: String,
    pub reason: String,
}

/// Every rule, compiled; immutable once built.
#[derive(Clone, Debug, Default)]
pub struct Compiled {
    pub run: RunState,
    /// This device's preferences, inside their ranges.
    pub prefs: AutomationPrefs,
    pub rules: Vec<CompiledRule>,
    pub unreadable: Vec<Unreadable>,
    /// Running chat rules by the chat they listen in.
    pub chat_index: HashMap<String, Vec<usize>>,
    /// Running chat rules for any open chat or the watched one.
    pub chat_any: Vec<usize>,
    pub badge_rules: Vec<usize>,
    /// [`FAMILY_CHAT`] and [`FAMILY_BADGE`] bits with a running rule.
    pub families: u32,
    /// Your own ids per provider (`twitch`, `kick`, ...). The Twitch set is
    /// the registry's accounts, the profile's id and the Twitch rules' owner
    /// and send-as ids; another provider's rule ids go into that provider's
    /// set, so a Kick id never makes a Twitch chatter with the same numeric
    /// id "you". Messages from an id here never trigger a rule; the profile's
    /// and the rules' ids never make a rule run.
    pub my_ids: HashMap<String, HashSet<String>>,
    /// Signed-in accounts (the registry's) and their logins.
    pub logins: HashMap<String, String>,
    /// Logins of the chats running rules listen in (at most 50).
    pub needed_chats: Vec<String>,
    /// Logins of the chats of running only-live rules.
    pub only_live_chats: Vec<String>,
    /// `twitch:<login>` keys where a Live rule with an outward action listens.
    pub live_chats: Vec<String>,
    /// A Live outward rule listens in any open chat or the watched one.
    pub live_any: bool,
    /// The longest owner room id among running badge rules.
    pub badge_room_len: usize,
}

impl Compiled {
    /// Nothing compiled yet: no rule runs.
    pub fn empty() -> Compiled {
        Compiled::default()
    }

    /// One of your Twitch ids (every Twitch check reads only this set).
    pub fn is_mine(&self, id: &str) -> bool {
        self.is_mine_on(TWITCH, id)
    }

    /// One of your ids on `provider`.
    pub fn is_mine_on(&self, provider: &str, id: &str) -> bool {
        self.my_ids.get(provider).is_some_and(|ids| ids.contains(id))
    }

    /// Your Twitch ids (the CLEARCHAT hold, alt badges, moderation targets).
    pub fn twitch_ids(&self) -> &HashSet<String> {
        static EMPTY: OnceLock<HashSet<String>> = OnceLock::new();
        self.my_ids.get(TWITCH).unwrap_or_else(|| EMPTY.get_or_init(HashSet::new))
    }

    pub fn login(&self, id: &str) -> Option<&str> {
        self.logins.get(id).map(String::as_str)
    }

    pub fn rule(&self, id: &str) -> Option<&CompiledRule> {
        self.rules.iter().find(|r| r.rule.id == id)
    }

    pub fn running(&self) -> impl Iterator<Item = &CompiledRule> + '_ {
        self.rules.iter().filter(|r| r.mode.is_some())
    }

    /// The running rules a row can reach, in rule order: the rules of its
    /// chat and the rules for any chat; the badge rules for a badge drop.
    pub fn candidates(&self, facts: &Facts) -> Vec<usize> {
        if facts.kind == RowKind::Badge {
            return self.badge_rules.clone();
        }
        let mut out = self.chat_index.get(&facts.channel_key).cloned().unwrap_or_default();
        out.extend(self.chat_any.iter().copied());
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// What a compile reads. Every account input is a synchronous read the
/// caller made just before: no network, no wait.
pub struct CompileInput<'a> {
    pub automations: &'a Automations,
    pub prefs: &'a AutomationPrefs,
    /// The registry's accounts as (id, login), from its list (the registry
    /// is saved atomically, so the list is never half-written).
    pub own_accounts: &'a [(String, String)],
    /// The id the profile read names, verified or not.
    pub profile_user_id: Option<&'a str>,
}

fn clean_login(login: &str) -> String {
    login.trim().trim_start_matches('@').to_lowercase()
}

/// An account's provider as a `my_ids` key: lowercase, Twitch when unset.
fn provider_of(provider: &str) -> String {
    match provider.trim() {
        "" => TWITCH.to_string(),
        name => name.to_ascii_lowercase(),
    }
}

pub fn compile(input: &CompileInput) -> Compiled {
    let mut out = Compiled { run: input.automations.run, prefs: input.prefs.clamped(), ..Compiled::default() };
    for (id, login) in input.own_accounts {
        let id = id.trim();
        if !id.is_empty() {
            out.logins.insert(id.to_string(), clean_login(login));
        }
    }
    let mut twitch: HashSet<String> = out.logins.keys().cloned().collect();
    if let Some(profile) = input.profile_user_id.map(str::trim).filter(|p| !p.is_empty()) {
        twitch.insert(profile.to_string());
    }
    out.my_ids.insert(TWITCH.to_string(), twitch);
    let mut needed: Vec<String> = Vec::new();
    let mut only_live: Vec<String> = Vec::new();
    for (position, raw) in input.automations.rules.iter().enumerate() {
        let mut rule = match Rule::from_value(raw) {
            Ok(rule) => rule,
            Err(_) => {
                out.unreadable.push(Unreadable {
                    position,
                    id: text_field(raw, "id"),
                    name: text_field(raw, "name"),
                    reason: NEWER_BUILD.to_string(),
                });
                continue;
            }
        };
        force(&mut rule);
        let owner_login = known_login(&out, &rule.owner.id, &rule.owner.login);
        let send_as_login = known_login(&out, &rule.send_as.id, &rule.send_as.login);
        let send_as_name = match rule.send_as.name.trim() {
            "" => send_as_login.clone(),
            name => name.to_string(),
        };
        let own_key = if owner_login.is_empty() { String::new() } else { channel_filter_key("twitch", &owner_login) };
        let these_keys: Vec<String> = match &rule.only_if.channels {
            ChannelScope::These { list } => list.iter().filter_map(chat_key_of).collect(),
            _ => Vec::new(),
        };
        // Signed in means the registry names the account; the profile's id
        // alone never runs a rule.
        let mut held = if !out.logins.contains_key(&rule.owner.id) || owner_login.is_empty() {
            Some(sign_in(&owner_login))
        } else if !out.logins.contains_key(&rule.send_as.id) {
            Some(sign_in(&send_as_login))
        } else {
            None
        };
        let test = match &rule.when {
            Trigger::Message { text } => match TextTest::build(text) {
                Ok(test) => test,
                Err(why) => {
                    if held.is_none() {
                        held = Some(why.to_string());
                    }
                    TextTest::Never
                }
            },
            _ => TextTest::Never,
        };
        let mode = if rule.enabled && held.is_none() { mode_for(out.run, rule.live) } else { None };
        let outward = rule.actions.iter().any(Action::is_outward);
        let index = out.rules.len();
        if let Some(mode) = mode {
            let listened: Vec<String> = match &rule.only_if.channels {
                ChannelScope::Own {} => vec![own_key.clone()],
                ChannelScope::These { .. } => these_keys.clone(),
                ChannelScope::AnyOpen {} | ChannelScope::Watching {} => Vec::new(),
            };
            if rule.is_badge_rule() {
                out.badge_rules.push(index);
                out.families |= FAMILY_BADGE;
                out.badge_room_len = out.badge_room_len.max(rule.owner.id.trim().len());
            } else {
                out.families |= FAMILY_CHAT;
                if listened.is_empty() {
                    out.chat_any.push(index);
                } else {
                    for key in &listened {
                        out.chat_index.entry(key.clone()).or_default().push(index);
                    }
                }
            }
            for key in &listened {
                push_unique(&mut needed, login_of_key(key));
                if rule.only_if.only_live {
                    push_unique(&mut only_live, login_of_key(key));
                }
            }
            if mode == Mode::Live && outward {
                if listened.is_empty() {
                    out.live_any = true;
                } else {
                    for key in &listened {
                        push_unique(&mut out.live_chats, key);
                    }
                }
            }
        }
        let summary = summarize(&rule);
        out.rules.push(CompiledRule {
            rule,
            position,
            mode,
            held,
            owner_login,
            send_as_login,
            send_as_name,
            own_key,
            these_keys,
            test,
            summary,
            outward,
        });
    }
    // Every account a rule names is yours: its messages never trigger a rule,
    // even right after a sign-in whose registry update failed. An id here
    // only keeps rules from answering it; it never makes a rule run.
    // Each id joins its own provider's set.
    let rule_ids: Vec<(String, String)> = out
        .rules
        .iter()
        .flat_map(|r| [&r.rule.owner, &r.rule.send_as])
        .map(|account| (provider_of(&account.provider), account.id.trim().to_string()))
        .filter(|(_, id)| !id.is_empty())
        .collect();
    for (provider, id) in rule_ids {
        out.my_ids.entry(provider).or_default().insert(id);
    }
    needed.truncate(MAX_ENGINE_CHATS);
    out.needed_chats = needed;
    out.only_live_chats = only_live;
    out
}

/// The values every rule runs with, whatever was stored: a badge rule posts
/// in your own channel, only while live, with no rule cooldown; a rule sends
/// as its owner unless it names another account; people lists are logins.
fn force(rule: &mut Rule) {
    if rule.send_as.id.trim().is_empty() {
        rule.send_as = rule.owner.clone();
    }
    if rule.is_badge_rule() {
        rule.only_if.channels = ChannelScope::Own {};
        rule.only_if.only_live = true;
        rule.limits.rule_cooldown_seconds = 0;
    }
    for list in [&mut rule.only_if.skip_users, &mut rule.only_if.only_users] {
        for user in list.iter_mut() {
            *user = user.trim().trim_start_matches('@').to_ascii_lowercase();
        }
        list.retain(|u| !u.is_empty());
    }
}

fn known_login(compiled: &Compiled, id: &str, stored: &str) -> String {
    compiled
        .logins
        .get(id)
        .filter(|login| !login.is_empty())
        .cloned()
        .unwrap_or_else(|| stored.trim().trim_start_matches('@').to_lowercase())
}

fn sign_in(login: &str) -> String {
    if login.is_empty() {
        "Sign in with this rule's account to run it".to_string()
    } else {
        format!("Sign in as @{login} to run this rule")
    }
}

fn text_field(raw: &Value, key: &str) -> String {
    raw.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

fn chat_key_of(chat: &ChannelRef) -> Option<String> {
    let raw = match chat.key.trim() {
        "" => chat.name.trim(),
        key => key,
    };
    let key = channel_filter_key("twitch", raw);
    (!login_of_key(&key).is_empty()).then_some(key)
}

fn login_of_key(key: &str) -> &str {
    key.split_once(':').map_or(key, |(_, login)| login).trim()
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if !value.is_empty() && !list.iter().any(|v| v == value) {
        list.push(value.to_string());
    }
}

// ---------------------------------------------------------------------------
// Summaries
// ---------------------------------------------------------------------------

/// The one line a rule's row shows: when, where, while, then what it does.
pub fn summarize(rule: &Rule) -> String {
    let mut out = when_words(&rule.when);
    if rule.is_badge_rule() {
        out.push_str(" in your channel");
    } else {
        out.push_str(&where_words(&rule.only_if.channels));
    }
    if rule.only_if.only_live {
        out.push_str(" while you're live");
    }
    if rule.only_if.only_away {
        out.push_str(" while you're away");
    }
    out.push_str(": ");
    out.push_str(&action_words(&rule.actions));
    out
}

fn mode_words(mode: MatchMode) -> &'static str {
    match mode {
        MatchMode::AnyWord => "has any of the words",
        MatchMode::AllWords => "has all of the words",
        MatchMode::Phrase => "has the phrase",
        MatchMode::Contains => "contains",
        MatchMode::StartsWith => "starts with",
        MatchMode::Exact => "is exactly",
        MatchMode::Regex => "matches",
    }
}

fn event_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Sub => "a new sub",
        EventKind::Resub => "a resub",
        EventKind::Gift => "a gifted sub",
        EventKind::GiftBomb => "a batch of gifted subs",
        EventKind::Raid => "a raid",
        EventKind::Cheer => "a cheer",
        EventKind::Announcement => "an announcement",
    }
}

fn when_words(when: &Trigger) -> String {
    match when {
        Trigger::Message { text } => {
            let pattern: String = text.pattern.trim().chars().take(40).collect();
            format!("When a message {} \"{pattern}\"", mode_words(text.mode))
        }
        Trigger::Mention { replies_too: true } => "When someone mentions you or replies to you".to_string(),
        Trigger::Mention { replies_too: false } => "When someone mentions you".to_string(),
        Trigger::FirstMessage {} => "When someone chats for the first time".to_string(),
        Trigger::ChannelEvent { events, .. } => {
            let names: Vec<String> = events.iter().map(|e| event_name(*e).to_string()).collect();
            if names.is_empty() {
                "On a channel event".to_string()
            } else {
                format!("On {}", list_words(&names, "or"))
            }
        }
        Trigger::BadgeDrop { on, .. } => match on {
            BadgeLeg::Announced => "When a new Twitch badge is announced",
            BadgeLeg::Earnable => "When a new Twitch badge can be earned",
            BadgeLeg::Both => "When a new Twitch badge is announced or can be earned",
        }
        .to_string(),
    }
}

fn chat_name(chat: &ChannelRef) -> String {
    let name = chat.name.trim().trim_start_matches('#');
    if name.is_empty() {
        login_of_key(&chat.key).to_string()
    } else {
        name.to_string()
    }
}

fn where_words(scope: &ChannelScope) -> String {
    match scope {
        ChannelScope::Own {} => " in your channel".to_string(),
        ChannelScope::These { list } => {
            let names: Vec<String> = list.iter().take(3).map(|c| format!("#{}", chat_name(c))).collect();
            let more = list.len().saturating_sub(3);
            if names.is_empty() {
                " in the channels you pick".to_string()
            } else if more > 0 {
                format!(" in {} and {more} more", names.join(", "))
            } else {
                format!(" in {}", list_words(&names, "and"))
            }
        }
        ChannelScope::AnyOpen {} => " in any chat you have open".to_string(),
        ChannelScope::Watching {} => " in the chat of the stream you're watching".to_string(),
    }
}

fn action_words(actions: &[Action]) -> String {
    let words: Vec<String> = actions
        .iter()
        .map(|a| match a {
            Action::Reply { .. } => "reply".to_string(),
            Action::Send { to: None, .. } => "send a message".to_string(),
            Action::Send { to: Some(chat), .. } => format!("send a message in #{}", chat_name(chat)),
            Action::Notify { .. } => "notify you".to_string(),
        })
        .collect();
    if words.is_empty() {
        "nothing yet".to_string()
    } else {
        list_words(&words, "and")
    }
}

/// "a", "a and b", "a, b and c".
fn list_words(items: &[String], last: &str) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., tail] => format!("{} {last} {tail}", rest.join(", ")),
    }
}

/// Rules and accounts for the engine's tests.
#[cfg(test)]
pub(crate) mod fixtures {
    use super::{compile, CompileInput, Compiled};
    use crate::models::automations::{
        AccountRef, Action, Automations, AutomationPrefs, ChannelRef, Line, Pick, Rule, RunState, Trigger,
    };

    /// The main account's id (and its room).
    pub(crate) const ME: &str = "100";
    /// A linked account's id.
    pub(crate) const ALT: &str = "200";

    pub(crate) fn me() -> AccountRef {
        AccountRef { provider: "twitch".into(), id: ME.into(), login: "me".into(), name: "Me".into() }
    }

    pub(crate) fn alt() -> AccountRef {
        AccountRef { provider: "twitch".into(), id: ALT.into(), login: "myalt".into(), name: "MyAlt".into() }
    }

    /// The registry's accounts: the main and one linked account.
    pub(crate) fn accounts() -> Vec<(String, String)> {
        vec![(ME.into(), "me".into()), (ALT.into(), "myalt".into())]
    }

    pub(crate) fn line(text: &str) -> Line {
        Line { id: String::new(), text: text.to_string() }
    }

    pub(crate) fn reply(text: &str) -> Action {
        Action::Reply { id: "a-reply".into(), texts: vec![line(text)], pick: Pick::Random, thread: false }
    }

    pub(crate) fn send(text: &str) -> Action {
        Action::Send { id: "a-send".into(), to: None, texts: vec![line(text)], pick: Pick::Random }
    }

    pub(crate) fn chat(login: &str, room: &str) -> ChannelRef {
        ChannelRef { id: format!("c-{login}"), key: format!("twitch:{login}"), room_id: room.into(), name: login.into() }
    }

    pub(crate) fn send_to(login: &str, room: &str, text: &str) -> Action {
        Action::Send { id: format!("a-send-{login}"), to: Some(chat(login, room)), texts: vec![line(text)], pick: Pick::Random }
    }

    pub(crate) fn notify(title: &str) -> Action {
        Action::Notify {
            id: "a-notify".into(),
            title: title.into(),
            body: String::new(),
            toast: true,
            system: false,
            sound: None,
            volume: 1.0,
        }
    }

    /// A Live, switched-on rule of the main account in its own channel.
    pub(crate) fn rule(id: &str, when: Trigger, actions: Vec<Action>) -> Rule {
        Rule {
            id: id.into(),
            name: format!("Rule {id}"),
            enabled: true,
            live: true,
            owner: me(),
            send_as: me(),
            when,
            actions,
            ..Rule::default()
        }
    }

    /// A compile with this registry and no profile id.
    pub(crate) fn compiled_for(
        rules: &[Rule],
        run: RunState,
        prefs: &AutomationPrefs,
        accounts: &[(String, String)],
    ) -> Compiled {
        let automations = Automations { run, rules: rules.iter().map(Rule::to_value).collect(), ..Automations::default() };
        compile(&CompileInput { automations: &automations, prefs, own_accounts: accounts, profile_user_id: None })
    }

    /// A compile of these rules from these account reads.
    pub(crate) fn compiled_accounts(rules: &[Rule], own_accounts: &[(String, String)], profile_user_id: Option<&str>) -> Compiled {
        let automations =
            Automations { run: RunState::On, rules: rules.iter().map(Rule::to_value).collect(), ..Automations::default() };
        compile(&CompileInput { automations: &automations, prefs: &AutomationPrefs::default(), own_accounts, profile_user_id })
    }

    pub(crate) fn compiled_with(rules: &[Rule], run: RunState) -> Compiled {
        compiled_for(rules, run, &AutomationPrefs::default(), &accounts())
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::models::automations::AccountRef;
    use crate::services::account_automations::facts::{from_chat, rows};

    fn mention() -> Trigger {
        Trigger::Mention { replies_too: true }
    }

    fn modes(c: &Compiled) -> Vec<Option<Mode>> {
        c.rules.iter().map(|r| r.mode).collect()
    }

    fn says(test: &TextTest, text: &str) -> bool {
        test.matches(&from_chat(&rows::privmsg("m1", "me", ME, "fan", "300", text)).unwrap())
    }

    fn test_of(mode: MatchMode, pattern: &str, cs: bool) -> TextTest {
        TextTest::build(&TextMatch { mode, pattern: pattern.into(), case_sensitive: cs }).unwrap()
    }

    #[test]
    fn your_own_ids_are_the_registry_the_profile_and_every_rules_accounts() {
        let c = compiled_with(&[], RunState::On);
        assert!(c.is_mine(ME) && c.is_mine(ALT));
        assert_eq!(c.login(ALT), Some("myalt"));
        assert!(!c.is_mine("999"));
        // An account nothing names is not yours.
        let alt_only = vec![(ALT.to_string(), "myalt".to_string())];
        assert!(!compiled_accounts(&[], &alt_only, None).is_mine(ME));
        // The profile's id is yours, but runs no rule.
        let profiled = compiled_accounts(&[], &[], Some("400"));
        assert!(profiled.is_mine("400"));
        assert_eq!(profiled.login("400"), None);
        // Every rule's owner and send-as ids are yours, with nothing listed
        // (a sign-in whose registry update failed), and naming an account
        // never signs it in: the rule is still held.
        let mut as_alt = rule("r", mention(), vec![reply("hi")]);
        as_alt.send_as = alt();
        let named = compiled_accounts(&[as_alt], &[], None);
        assert!(named.is_mine(ME) && named.is_mine(ALT));
        assert_eq!(named.rules[0].held.as_deref(), Some("Sign in as @me to run this rule"));
        assert_eq!(named.twitch_ids(), &[ME.to_string(), ALT.to_string()].into_iter().collect::<HashSet<String>>());
        assert!(Compiled::empty().my_ids.is_empty() && Compiled::empty().twitch_ids().is_empty());
        assert_eq!(Compiled::empty().families, 0);
    }

    #[test]
    fn a_kick_id_never_makes_a_twitch_chatter_with_the_same_number_yours() {
        // A Kick rule sends as Kick account 300; Twitch user 300 is a stranger.
        let mut kick = rule("k", mention(), vec![reply("hi")]);
        kick.send_as = AccountRef { provider: "Kick".into(), id: "300".into(), login: "mykick".into(), name: "MyKick".into() };
        let c = compiled_with(&[kick], RunState::On);
        assert!(c.is_mine_on("kick", "300"));
        assert!(!c.is_mine("300"));
        assert!(!c.twitch_ids().contains("300"));
        let chatter = from_chat(&rows::privmsg("m1", "me", ME, "fan", "300", "@me hi")).unwrap();
        assert!(!c.is_mine(&chatter.user_id));
        // The Twitch owner of that rule is still yours on Twitch.
        assert!(c.is_mine(ME) && !c.is_mine_on("kick", ME));
    }

    #[test]
    fn a_linked_accounts_typed_message_never_fires_a_rule() {
        let c = compiled_with(&[rule("r", mention(), vec![reply("hi")])], RunState::On);
        let typed = from_chat(&rows::privmsg("m1", "me", ME, "myalt", ALT, "@me hi")).unwrap();
        assert!(c.is_mine(&typed.user_id));
        let stranger = from_chat(&rows::privmsg("m2", "me", ME, "fan", "300", "@me hi")).unwrap();
        assert!(!c.is_mine(&stranger.user_id));
    }

    #[test]
    fn off_runs_nothing_and_test_or_on_pick_each_rules_mode() {
        let live = rule("r-live", mention(), vec![reply("hi")]);
        let mut test = rule("r-test", mention(), vec![reply("hi")]);
        test.live = false;
        let mut off = rule("r-off", mention(), vec![reply("hi")]);
        off.enabled = false;
        let rules = [live, test, off];
        let c = compiled_with(&rules, RunState::Off);
        assert_eq!(c.families, 0);
        assert_eq!(modes(&c), vec![None, None, None]);
        assert!(c.needed_chats.is_empty());
        assert!(c.chat_index.is_empty());
        let c = compiled_with(&rules, RunState::Test);
        assert_eq!(modes(&c), vec![Some(Mode::Test), Some(Mode::Test), None]);
        assert_eq!(c.families, FAMILY_CHAT);
        assert!(c.live_chats.is_empty());
        let c = compiled_with(&rules, RunState::On);
        assert_eq!(modes(&c), vec![Some(Mode::Live), Some(Mode::Test), None]);
        assert_eq!(c.rules[2].held, None);
        assert_eq!(c.running().count(), 2);
        assert_eq!(c.live_chats, vec!["twitch:me"]);
        // A Notify-only Live rule marks no chat with the dot.
        let c = compiled_with(&[rule("r-note", mention(), vec![notify("x")])], RunState::On);
        assert!(c.live_chats.is_empty() && !c.live_any);
    }

    #[test]
    fn rules_are_indexed_by_the_chats_they_listen_in() {
        let own = rule("r-own", mention(), vec![reply("hi")]);
        let mut these = rule("r-these", mention(), vec![reply("hi")]);
        these.only_if.channels = ChannelScope::These {
            list: vec![
                ChannelRef { id: "c-1".into(), key: "twitch:Friend".into(), room_id: "700".into(), name: "Friend".into() },
                ChannelRef { id: "c-2".into(), key: String::new(), room_id: String::new(), name: "#Other".into() },
            ],
        };
        let mut open = rule("r-open", mention(), vec![reply("hi")]);
        open.only_if.channels = ChannelScope::AnyOpen {};
        let c = compiled_with(&[own, these, open], RunState::On);
        assert_eq!(c.chat_index.get("twitch:me"), Some(&vec![0]));
        assert_eq!(c.chat_index.get("twitch:friend"), Some(&vec![1]));
        assert_eq!(c.chat_index.get("twitch:other"), Some(&vec![1]));
        assert_eq!(c.chat_any, vec![2]);
        let at = |channel: &str, room: &str| from_chat(&rows::privmsg("m1", channel, room, "fan", "300", "hi")).unwrap();
        assert_eq!(c.candidates(&at("me", ME)), vec![0, 2]);
        assert_eq!(c.candidates(&at("friend", "700")), vec![1, 2]);
        assert_eq!(c.candidates(&at("stranger", "800")), vec![2]);
        assert_eq!(c.needed_chats, vec!["me", "friend", "other"]);
        assert!(c.live_any);
        assert_eq!(c.live_chats, vec!["twitch:me", "twitch:friend", "twitch:other"]);
    }

    #[test]
    fn a_rule_whose_account_is_gone_is_held_with_a_reason() {
        let mut gone = rule("r-gone", mention(), vec![reply("hi")]);
        gone.owner = AccountRef { provider: "twitch".into(), id: "900".into(), login: "OldMe".into(), name: "OldMe".into() };
        gone.send_as = gone.owner.clone();
        let mut alt_gone = rule("r-alt", mention(), vec![reply("hi")]);
        alt_gone.send_as = AccountRef { provider: "twitch".into(), id: "901".into(), login: "lostalt".into(), name: "LostAlt".into() };
        let mut as_alt = rule("r-as-alt", mention(), vec![reply("hi")]);
        as_alt.send_as = alt();
        let c = compiled_with(&[gone, alt_gone, as_alt], RunState::On);
        assert_eq!(c.rules[0].held.as_deref(), Some("Sign in as @oldme to run this rule"));
        assert_eq!(c.rules[1].held.as_deref(), Some("Sign in as @lostalt to run this rule"));
        assert_eq!(modes(&c), vec![None, None, Some(Mode::Live)]);
        assert_eq!(c.rules[2].send_as_name, "MyAlt");
        assert_eq!(c.rules[2].send_as_login, "myalt");
    }

    #[test]
    fn a_rule_from_a_newer_build_is_held_and_the_rest_still_run() {
        let automations = Automations {
            run: RunState::On,
            rules: vec![
                serde_json::json!({"id": "r-new", "name": "Future", "when": {"kind": "hype_train"}}),
                rule("r-ok", mention(), vec![reply("hi")]).to_value(),
            ],
            ..Automations::default()
        };
        let c = compile(&CompileInput {
            automations: &automations,
            prefs: &AutomationPrefs::default(),
            own_accounts: &accounts(),
            profile_user_id: None,
        });
        assert_eq!(
            c.unreadable,
            vec![Unreadable { position: 0, id: "r-new".into(), name: "Future".into(), reason: NEWER_BUILD.into() }]
        );
        assert_eq!(c.rules.len(), 1);
        assert_eq!(c.rules[0].position, 1);
        assert_eq!(c.rules[0].mode, Some(Mode::Live));
        assert_eq!(c.chat_index.get("twitch:me"), Some(&vec![0]));
    }

    #[test]
    fn logins_are_refreshed_by_id() {
        let mut stale = rule("r", mention(), vec![reply("hi")]);
        stale.owner.login = "oldname".into();
        let accounts = vec![(ME.to_string(), "NewName".to_string())];
        let c = compiled_for(&[stale], RunState::On, &AutomationPrefs::default(), &accounts);
        assert_eq!(c.rules[0].owner_login, "newname");
        assert_eq!(c.rules[0].own_key, "twitch:newname");
        assert_eq!(c.needed_chats, vec!["newname"]);
        // Listed, but with no login anywhere: the rule asks for a sign-in.
        let nameless = vec![(ME.to_string(), String::new())];
        let mut unnamed = rule("r", mention(), vec![reply("hi")]);
        unnamed.owner.login = String::new();
        let c = compiled_for(&[unnamed], RunState::On, &AutomationPrefs::default(), &nameless);
        assert_eq!(c.rules[0].held.as_deref(), Some("Sign in with this rule's account to run it"));
        assert_eq!(c.rules[0].mode, None);
    }

    #[test]
    fn a_pattern_that_cannot_run_holds_its_rule() {
        let bad = rule(
            "r",
            Trigger::Message { text: TextMatch { mode: MatchMode::Regex, pattern: "(".into(), case_sensitive: false } },
            vec![reply("x")],
        );
        let c = compiled_with(&[bad], RunState::On);
        assert_eq!(c.rules[0].held.as_deref(), Some("This isn't a valid regular expression"));
        assert_eq!(c.rules[0].mode, None);
    }

    #[test]
    fn badge_rules_are_forced_to_your_channel_while_live_with_no_rule_cooldown() {
        let mut raw = rule(
            "r-badge",
            Trigger::BadgeDrop { on: BadgeLeg::Both, link: Default::default(), batch_text: String::new() },
            vec![send("{badge}")],
        );
        raw.only_if.only_live = false;
        raw.only_if.channels = ChannelScope::AnyOpen {};
        raw.limits.rule_cooldown_seconds = 90;
        let c = compiled_with(&[raw], RunState::On);
        let r = &c.rules[0].rule;
        assert!(r.only_if.only_live);
        assert_eq!(r.only_if.channels, ChannelScope::Own {});
        assert_eq!(r.limits.rule_cooldown_seconds, 0);
        assert_eq!(c.families, FAMILY_BADGE);
        assert_eq!(c.badge_rules, vec![0]);
        assert_eq!(c.badge_room_len, ME.len());
        assert_eq!(c.only_live_chats, vec!["me"]);
        assert_eq!(c.live_chats, vec!["twitch:me"]);
        assert!(c.chat_index.is_empty() && c.chat_any.is_empty());
        assert_eq!(c.rules[0].summary, "When a new Twitch badge is announced or can be earned in your channel while you're live: send a message");
    }

    #[test]
    fn people_lists_are_tidied_and_send_as_defaults_to_the_owner() {
        let mut r = rule("r", mention(), vec![reply("hi")]);
        r.send_as = AccountRef::default();
        r.only_if.skip_users = vec![" @NightBot ".into(), String::new()];
        r.only_if.only_users = vec!["@Friend".into()];
        let c = compiled_with(&[r], RunState::On);
        let r = &c.rules[0];
        assert_eq!(r.rule.send_as.id, ME);
        assert_eq!(r.rule.only_if.skip_users, vec!["nightbot"]);
        assert_eq!(r.rule.only_if.only_users, vec!["friend"]);
        assert_eq!(r.send_as_name, "Me");
    }

    #[test]
    fn needed_chats_are_capped_at_fifty() {
        let rules: Vec<Rule> = (0..30)
            .map(|i| {
                let mut r = rule(&format!("r{i}"), mention(), vec![reply("hi")]);
                r.only_if.channels = ChannelScope::These { list: vec![chat(&format!("a{i}"), ""), chat(&format!("b{i}"), "")] };
                r
            })
            .collect();
        let c = compiled_with(&rules, RunState::On);
        assert_eq!(c.needed_chats.len(), MAX_ENGINE_CHATS);
        assert_eq!(c.chat_index.len(), 60);
    }

    #[test]
    fn text_tests_match_by_mode() {
        let any = test_of(MatchMode::AnyWord, "Hello hi", false);
        assert!(says(&any, "well HI there"));
        assert!(!says(&any, "this is high"));
        assert!(says(&any, "hi!") && !says(&any, "chi"));
        let all = test_of(MatchMode::AllWords, "hello world", false);
        assert!(says(&all, "World, hello!") && !says(&all, "hello there"));
        let phrase = test_of(MatchMode::Phrase, "good   morning", false);
        assert!(says(&phrase, "Good morning all") && !says(&phrase, "goodmorning"));
        assert!(says(&test_of(MatchMode::Contains, "ell", false), "HELLO"));
        let starts = test_of(MatchMode::StartsWith, "!lurk", false);
        assert!(says(&starts, "  !LURK now") && !says(&starts, "no !lurk"));
        let exact = test_of(MatchMode::Exact, "gg", false);
        assert!(says(&exact, " GG ") && !says(&exact, "gg wp"));
        // Case-sensitive tests compare as typed.
        let cs = test_of(MatchMode::Contains, "Hi", true);
        assert!(says(&cs, "Hi there") && !says(&cs, "hi there"));
        // A Regex is never lowercased, and is case-insensitive unless asked.
        let re = test_of(MatchMode::Regex, r"^Hi\b", true);
        assert!(says(&re, "Hi all") && !says(&re, "hi all"));
        assert!(says(&test_of(MatchMode::Regex, r"^Hi\b", false), "hi all"));
        // One spelling composed or decomposed matches either way.
        assert!(says(&test_of(MatchMode::AnyWord, "caf\u{e9}", false), "CAFE\u{301} time"));
        // An empty pattern never matches.
        assert!(!says(&test_of(MatchMode::AllWords, "  ", false), "anything"));
        assert!(matches!(test_of(MatchMode::Regex, " ", false), TextTest::Never));
    }

    #[test]
    fn summaries_say_what_a_rule_does() {
        let mut away = rule("r", mention(), vec![reply("hi"), notify("x")]);
        away.only_if.channels = ChannelScope::AnyOpen {};
        away.only_if.only_away = true;
        assert_eq!(
            summarize(&away),
            "When someone mentions you or replies to you in any chat you have open while you're away: reply and notify you"
        );
        let events = rule(
            "e",
            Trigger::ChannelEvent {
                events: vec![EventKind::Sub, EventKind::Resub, EventKind::Raid],
                min_months: 0,
                min_gifts: 0,
                min_viewers: 0,
                min_bits: 0,
            },
            vec![send_to("friend", "700", "x")],
        );
        assert_eq!(summarize(&events), "On a new sub, a resub or a raid in your channel: send a message in #friend");
        let mut words = rule(
            "w",
            Trigger::Message { text: TextMatch { mode: MatchMode::AnyWord, pattern: "hello hi".into(), case_sensitive: false } },
            Vec::new(),
        );
        words.only_if.channels =
            ChannelScope::These { list: vec![chat("a", ""), chat("b", ""), chat("c", ""), chat("d", "")] };
        assert_eq!(summarize(&words), "When a message has any of the words \"hello hi\" in #a, #b, #c and 1 more: nothing yet");
        let mut first = rule("f", Trigger::FirstMessage {}, vec![reply("Welcome")]);
        first.only_if.only_live = true;
        assert_eq!(summarize(&first), "When someone chats for the first time in your channel while you're live: reply");
    }

    #[test]
    fn the_pure_modules_reach_nothing_outside_the_app() {
        let sources = [
            ("activity.rs", include_str!("activity.rs")),
            ("badges.rs", include_str!("badges.rs")),
            ("claims.rs", include_str!("claims.rs")),
            ("compile.rs", include_str!("compile.rs")),
            ("counts.rs", include_str!("counts.rs")),
            ("facts.rs", include_str!("facts.rs")),
            ("guards.rs", include_str!("guards.rs")),
            ("matcher.rs", include_str!("matcher.rs")),
            ("template.rs", include_str!("template.rs")),
            ("templates.rs", include_str!("templates.rs")),
            ("verify.rs", include_str!("verify.rs")),
        ];
        // Built from parts, so this test's own source does not name them.
        let banned = [
            ["Account", "Store"].concat(),
            ["account", "_store::"].concat(),
            ["get", "_token"].concat(),
            ["primary", "("].concat(),
            ["twitch_identity", "_of"].concat(),
            ["user_by", "_login"].concat(),
            ["post", "_json"].concat(),
            ["send_message", "_helix"].concat(),
            ["reqwest", "::"].concat(),
            ["tokio", "::"].concat(),
            ["std::", "fs"].concat(),
            ["tauri", "::"].concat(),
        ];
        // The token verifier alone runs its work under the job's deadline.
        let runtime = ["tokio", "::"].concat();
        for (name, text) in sources {
            for needle in &banned {
                if name == "verify.rs" && *needle == runtime {
                    continue;
                }
                assert!(!text.contains(needle.as_str()), "{name} names {needle}");
            }
        }
    }
}
