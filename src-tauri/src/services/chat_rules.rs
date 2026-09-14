//! Rust-owned chat rule engine: highlights, mentions, ignores, and saved
//! message filters, evaluated ONCE per message on the IRC lane and stamped
//! onto the wire frame's metadata. The React row reads the stamp; it no longer
//! runs a regex loop per row per window.
//!
//! Rules come from the frontend-managed settings groups that ride the
//! flattened `extra` map on `Settings` (`chat_highlights`, `chat_filters`,
//! `chat_query`). They are deserialized here into local structs with serde
//! defaults, so an unknown field is ignored rather than dropped on save: the
//! "settings both sides" trap only bites fields nested inside TYPED structs on
//! `Settings`, and none of these groups are typed there.
//!
//! Hot-path contract: `evaluate` takes an `Arc<CompiledRules>` snapshot (one
//! RwLock read, no allocation on the miss path) and runs a `RegexSet` pass
//! over the message text. Recompilation happens only on a settings save whose
//! rule groups actually changed (JSON hash compare).
//!
//! The saved-filter expression language is Chatterino's, so a user can paste a
//! filter straight from Chatterino settings. Variables that StreamNook has no
//! data for (`author.external_badges`, `channel.watching`, `channel.live`,
//! `flags.automod`, `flags.restricted`, `flags.monitored`, `flags.whisper`,
//! `flags.similar`, `reward.title`, `reward.cost`) evaluate to their empty
//! value and `validate_filter` lists them so the editor can say so.

use crate::models::chat_layout::{BuiltInStamp, ChatMessage, HighlightStamp};
use crate::models::settings::Settings;
use regex::{Regex, RegexSet};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock, RwLock};

// ---------------------------------------------------------------------------
// Settings shapes (mirror src/types/index.ts; every field defaulted)
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default, Clone)]
struct HighlightPhrase {
    #[serde(default)]
    id: String,
    #[serde(default)]
    pattern: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    case_sensitive: bool,
    #[serde(default)]
    whole_word: bool,
    #[serde(default)]
    is_regex: bool,
    #[serde(default)]
    color: String,
    #[serde(default)]
    sound_id: Option<String>,
    #[serde(default)]
    cooldown_seconds: Option<f64>,
}

#[derive(Deserialize, Default, Clone)]
struct HighlightUser {
    #[serde(default)]
    id: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    username: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    sound_id: Option<String>,
    #[serde(default)]
    cooldown_seconds: Option<f64>,
}

#[derive(Deserialize, Default, Clone)]
struct HighlightBadge {
    #[serde(default)]
    id: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    badge_key: String,
    #[serde(default)]
    color: String,
    #[serde(default)]
    sound_id: Option<String>,
    #[serde(default)]
    cooldown_seconds: Option<f64>,
}

#[derive(Deserialize, Default, Clone)]
struct BuiltInRule {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Deserialize, Default, Clone)]
struct BuiltInSettings {
    #[serde(default)]
    first_time_chatter: Option<BuiltInRule>,
    #[serde(default)]
    returning_chatter: Option<BuiltInRule>,
    #[serde(default)]
    self_message: Option<BuiltInRule>,
    #[serde(default)]
    raider: Option<BuiltInRule>,
}

#[derive(Deserialize, Default, Clone)]
struct ChatHighlightSettings {
    #[serde(default)]
    phrases: Vec<HighlightPhrase>,
    #[serde(default)]
    built_in: Option<BuiltInSettings>,
    #[serde(default)]
    users: Vec<HighlightUser>,
    #[serde(default)]
    badges: Vec<HighlightBadge>,
}

#[derive(Deserialize, Default, Clone)]
struct IgnoredPhrase {
    #[serde(default)]
    id: String,
    #[serde(default)]
    pattern: String,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    case_sensitive: bool,
    #[serde(default)]
    whole_word: bool,
    #[serde(default)]
    is_regex: bool,
}

#[derive(Deserialize, Default, Clone)]
struct ChatFilterSettings {
    #[serde(default)]
    hide_bots: bool,
    #[serde(default)]
    hidden_users: Vec<String>,
    #[serde(default)]
    per_channel: HashMap<String, Vec<String>>,
    /// New in the rule engine: phrases that hide a message everywhere.
    #[serde(default)]
    ignored_phrases: Vec<IgnoredPhrase>,
}

#[derive(Deserialize, Default, Clone)]
struct SavedFilter {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    expr: String,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Deserialize, Default, Clone)]
struct ChatQuerySettings {
    #[serde(default)]
    filters: Vec<SavedFilter>,
    #[serde(default)]
    history_cap: Option<usize>,
}

fn default_true() -> bool {
    true
}

const DEFAULT_COOLDOWN_SECONDS: f64 = 3.0;
const VALID_SOUND_IDS: [&str; 5] = ["boop", "tick", "soft", "whisper", "gentle"];

fn normalize_sound(raw: &Option<String>) -> Option<String> {
    let s = raw.as_deref()?;
    if VALID_SOUND_IDS.contains(&s) || (s.starts_with("file:") && s.len() > 5 && s.len() < 64) {
        Some(s.to_string())
    } else {
        None
    }
}

fn cooldown_ms(secs: Option<f64>) -> u64 {
    let s = secs.unwrap_or(DEFAULT_COOLDOWN_SECONDS);
    if s.is_finite() && s > 0.0 {
        (s * 1000.0) as u64
    } else {
        0
    }
}

/// Same list as src/utils/knownBots.ts. Keep the two in step.
const KNOWN_BOTS: &[&str] = &[
    "nightbot",
    "streamelements",
    "streamlabs",
    "moobot",
    "fossabot",
    "wizebot",
    "sery_bot",
    "commanderroot",
    "soundtrackbot",
    "streamlootsbot",
    "pretzelrocks",
    "tangiabot",
    "blerp",
    "kofistreambot",
    "own3d",
    "botrixoficial",
    "coebot",
    "phantombot",
    "thepositivebot",
    "streamstickers",
    "lattemotte",
    "restreambot",
    "supibot",
    "anotherttvviewer",
    "streamdatabase",
    "streamdbbot",
    "potatbotat",
    "pajbot",
    "titlechange_bot",
    "buttsbot",
    "snusbot",
    "deepbot",
    "ankhbot",
    "vivbot",
    "revlobot",
    "dixperbro",
    "botisimo",
    "mikuia",
    "wzbot",
    "own3dpro_bot",
    "playwithviewersbot",
    "thepixelbot",
    "cloudbot",
    "9gag",
];

// ---------------------------------------------------------------------------
// Compiled rules
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct RuleMeta {
    id: String,
    color: String,
    sound_id: Option<String>,
    cooldown_ms: u64,
}

struct BadgeRule {
    meta: RuleMeta,
    /// Lowercased `name/version`, or `name/` when the rule is a wildcard.
    key: String,
    wildcard: bool,
}

struct BuiltIn {
    enabled: bool,
    color: String,
}

pub struct CompiledFilter {
    pub id: String,
    pub name: String,
    expr: Expr,
}

#[derive(Default)]
pub struct CompiledRules {
    /// Every enabled phrase pattern in list order, evaluated as one set.
    phrase_set: Option<RegexSet>,
    /// Parallel to the set's pattern indices.
    phrase_meta: Vec<RuleMeta>,
    users: HashMap<String, RuleMeta>,
    badges: Vec<BadgeRule>,
    first_time: BuiltIn,
    returning: BuiltIn,
    self_message: BuiltIn,
    raider: BuiltIn,
    hide_bots: bool,
    hidden_global: HashSet<String>,
    hidden_per_channel: HashMap<String, HashSet<String>>,
    ignore_set: Option<RegexSet>,
    pub filters: Vec<CompiledFilter>,
    /// Patterns that failed to compile, for the settings UI.
    pub errors: Vec<RuleError>,
    pub history_cap: usize,
}

impl Default for BuiltIn {
    fn default() -> Self {
        BuiltIn {
            enabled: false,
            color: String::new(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RuleError {
    pub scope: String,
    pub id: String,
    pub error: String,
}

pub const DEFAULT_HISTORY_CAP: usize = 1000;
pub const MIN_HISTORY_CAP: usize = 200;
pub const MAX_HISTORY_CAP: usize = 5000;

fn escape_regex(s: &str) -> String {
    regex::escape(s)
}

/// Build the Rust pattern for a phrase-style rule (highlight or ignore),
/// mirroring `buildRegex` in the retired JS matcher: escaped unless regex,
/// `\b` wrapped when whole-word, `(?i)` unless case-sensitive.
fn phrase_pattern(pattern: &str, is_regex: bool, whole_word: bool, case_sensitive: bool) -> String {
    let body = if is_regex {
        pattern.to_string()
    } else {
        let escaped = escape_regex(pattern);
        if whole_word {
            format!(r"\b{}\b", escaped)
        } else {
            escaped
        }
    };
    if case_sensitive {
        body
    } else {
        format!("(?i:{})", body)
    }
}

fn built_in(rule: &Option<BuiltInRule>, default_enabled: bool, default_color: &str) -> BuiltIn {
    let enabled = rule
        .as_ref()
        .and_then(|r| r.enabled)
        .unwrap_or(default_enabled);
    let color = rule
        .as_ref()
        .and_then(|r| r.color.clone())
        .filter(|c| !c.trim().is_empty())
        .unwrap_or_else(|| default_color.to_string());
    BuiltIn { enabled, color }
}

fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase().trim_start_matches('@').to_string()
}

/// Composite `provider:channel` key, lowercased, matching `makeKey` on the
/// frontend for the case-insensitive providers. A bare login is Twitch.
pub fn channel_filter_key(provider: &str, channel: &str) -> String {
    let ch = channel.trim().trim_start_matches('#');
    if let Some(idx) = ch.find(':') {
        let (p, c) = ch.split_at(idx);
        return format!("{}:{}", p.to_lowercase(), c[1..].to_lowercase());
    }
    let p = if provider.is_empty() { "twitch" } else { provider };
    format!("{}:{}", p.to_lowercase(), ch.to_lowercase())
}

impl CompiledRules {
    fn compile(
        hl: &ChatHighlightSettings,
        cf: &ChatFilterSettings,
        cq: &ChatQuerySettings,
    ) -> CompiledRules {
        let mut errors = Vec::new();

        // Phrases: one RegexSet over the enabled, non-empty, compilable rules.
        let mut patterns = Vec::new();
        let mut phrase_meta = Vec::new();
        for p in &hl.phrases {
            if !p.enabled || p.pattern.trim().is_empty() {
                continue;
            }
            let pat = phrase_pattern(&p.pattern, p.is_regex, p.whole_word, p.case_sensitive);
            match Regex::new(&pat) {
                Ok(_) => {
                    patterns.push(pat);
                    phrase_meta.push(RuleMeta {
                        id: p.id.clone(),
                        color: p.color.clone(),
                        sound_id: normalize_sound(&p.sound_id),
                        cooldown_ms: cooldown_ms(p.cooldown_seconds),
                    });
                }
                Err(e) => errors.push(RuleError {
                    scope: "highlight_phrase".into(),
                    id: p.id.clone(),
                    error: e.to_string(),
                }),
            }
        }
        let phrase_set = if patterns.is_empty() {
            None
        } else {
            match RegexSet::new(&patterns) {
                Ok(set) => Some(set),
                Err(e) => {
                    errors.push(RuleError {
                        scope: "highlight_phrase".into(),
                        id: String::new(),
                        error: e.to_string(),
                    });
                    None
                }
            }
        };

        let mut users = HashMap::new();
        for u in &hl.users {
            if !u.enabled {
                continue;
            }
            let key = u.username.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            // First rule in list order wins, like the JS matcher.
            users.entry(key).or_insert(RuleMeta {
                id: u.id.clone(),
                color: u.color.clone(),
                sound_id: normalize_sound(&u.sound_id),
                cooldown_ms: cooldown_ms(u.cooldown_seconds),
            });
        }

        let mut badges = Vec::new();
        for b in &hl.badges {
            if !b.enabled {
                continue;
            }
            let key = b.badge_key.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            let wildcard = key.ends_with("/*");
            let key = if wildcard {
                key[..key.len() - 1].to_string()
            } else {
                key
            };
            badges.push(BadgeRule {
                meta: RuleMeta {
                    id: b.id.clone(),
                    color: b.color.clone(),
                    sound_id: normalize_sound(&b.sound_id),
                    cooldown_ms: cooldown_ms(b.cooldown_seconds),
                },
                key,
                wildcard,
            });
        }

        let bi = hl.built_in.clone().unwrap_or_default();

        let mut hidden_global = HashSet::new();
        for n in &cf.hidden_users {
            let n = normalize_name(n);
            if !n.is_empty() {
                hidden_global.insert(n);
            }
        }
        let mut hidden_per_channel: HashMap<String, HashSet<String>> = HashMap::new();
        for (raw_key, names) in &cf.per_channel {
            let key = channel_filter_key("twitch", raw_key);
            let set = hidden_per_channel.entry(key).or_default();
            for n in names {
                let n = normalize_name(n);
                if !n.is_empty() {
                    set.insert(n);
                }
            }
        }
        hidden_per_channel.retain(|_, s| !s.is_empty());

        let mut ignore_patterns = Vec::new();
        for p in &cf.ignored_phrases {
            if !p.enabled || p.pattern.trim().is_empty() {
                continue;
            }
            let pat = phrase_pattern(&p.pattern, p.is_regex, p.whole_word, p.case_sensitive);
            match Regex::new(&pat) {
                Ok(_) => ignore_patterns.push(pat),
                Err(e) => errors.push(RuleError {
                    scope: "ignored_phrase".into(),
                    id: p.id.clone(),
                    error: e.to_string(),
                }),
            }
        }
        let ignore_set = if ignore_patterns.is_empty() {
            None
        } else {
            RegexSet::new(&ignore_patterns).ok()
        };

        let mut filters = Vec::new();
        for f in &cq.filters {
            if !f.enabled || f.expr.trim().is_empty() {
                continue;
            }
            match parse_filter(&f.expr) {
                Ok(expr) => filters.push(CompiledFilter {
                    id: f.id.clone(),
                    name: f.name.clone(),
                    expr,
                }),
                Err(e) => errors.push(RuleError {
                    scope: "filter".into(),
                    id: f.id.clone(),
                    error: e,
                }),
            }
        }

        let history_cap = cq
            .history_cap
            .unwrap_or(DEFAULT_HISTORY_CAP)
            .clamp(MIN_HISTORY_CAP, MAX_HISTORY_CAP);

        CompiledRules {
            phrase_set,
            phrase_meta,
            users,
            badges,
            first_time: built_in(&bi.first_time_chatter, true, "#a855f7"),
            returning: built_in(&bi.returning_chatter, false, "#22d3ee"),
            self_message: built_in(&bi.self_message, false, "#facc15"),
            raider: built_in(&bi.raider, false, "#ef4444"),
            hide_bots: cf.hide_bots,
            hidden_global,
            hidden_per_channel,
            ignore_set,
            filters,
            errors,
            history_cap,
        }
    }

    fn is_empty_highlights(&self) -> bool {
        self.phrase_set.is_none() && self.users.is_empty() && self.badges.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Message facts: everything the rules and the history ring read, built once
// ---------------------------------------------------------------------------

pub const F_ACTION: u32 = 1 << 0;
pub const F_FIRST: u32 = 1 << 1;
pub const F_CHEER: u32 = 1 << 2;
pub const F_HIGHLIGHTED: u32 = 1 << 3;
pub const F_REDEEMED: u32 = 1 << 4;
pub const F_REPLY: u32 = 1 << 5;
pub const F_SHARED: u32 = 1 << 6;
pub const F_SUB: u32 = 1 << 7;
pub const F_SYSTEM: u32 = 1 << 8;
pub const F_STREAK: u32 = 1 << 9;
pub const F_RETURNING: u32 = 1 << 10;
pub const F_RAID: u32 = 1 << 11;
pub const F_LINK: u32 = 1 << 12;
pub const F_MENTION: u32 = 1 << 13;
pub const F_HIGHLIGHT_RULE: u32 = 1 << 14;
pub const F_DELETED: u32 = 1 << 15;
pub const F_MONITORED: u32 = 1 << 16;
pub const F_RESTRICTED: u32 = 1 << 17;

const SUB_TYPES: &[&str] = &[
    "sub",
    "resub",
    "subgift",
    "submysterygift",
    "anonsubgift",
    "anonsubmysterygift",
    "giftpaidupgrade",
    "primepaidupgrade",
    "standardpayforward",
    "communitypayforward",
    "anongiftpaidupgrade",
    "onetapstreakexpired",
];

pub struct MessageFacts<'a> {
    pub login: String,
    pub display_lower: String,
    pub user_id: &'a str,
    pub color: &'a str,
    /// `name/version` lowercased.
    pub badge_keys: Vec<String>,
    pub badge_names: Vec<String>,
    pub sub_length: i64,
    pub subbed: bool,
    pub bits: i64,
    pub channel_key: String,
    pub filter_channel_key: String,
    pub content: &'a str,
    pub flags: u32,
    pub reward_id: String,
    pub reply_parent_user_id: &'a str,
    pub msg_type: &'a str,
    pub ts_ms: i64,
    /// "monitored" | "restricted" when the sender is flagged in this channel.
    pub suspicious: Option<String>,
}

impl<'a> MessageFacts<'a> {
    pub fn from_message(msg: &'a ChatMessage) -> MessageFacts<'a> {
        let tag = |k: &str| msg.tags.get(k).map(|s| s.as_str()).unwrap_or("");
        let login = if msg.username.is_empty() {
            tag("login").to_lowercase()
        } else {
            msg.username.to_lowercase()
        };
        let display_lower = msg.display_name.to_lowercase();
        let badge_keys: Vec<String> = msg
            .badges
            .iter()
            .map(|b| format!("{}/{}", b.name.to_lowercase(), b.version.to_lowercase()))
            .collect();
        let badge_names: Vec<String> = msg.badges.iter().map(|b| b.name.to_lowercase()).collect();
        let subbed = badge_names
            .iter()
            .any(|n| n == "subscriber" || n == "founder");
        let sub_length = tag("badge-info")
            .split(',')
            .filter_map(|e| {
                let (name, ver) = e.split_once('/')?;
                if name == "subscriber" || name == "founder" {
                    ver.parse::<i64>().ok()
                } else {
                    None
                }
            })
            .next()
            .unwrap_or(0);
        let bits = msg
            .metadata
            .bits_amount
            .map(|b| b as i64)
            .or_else(|| tag("bits").parse::<i64>().ok())
            .unwrap_or(0);
        let msg_type = msg.metadata.msg_type.as_deref().unwrap_or("");
        let msg_id_tag = tag("msg-id");
        let reward_id = tag("custom-reward-id").to_string();
        let channel_key = msg.channel.trim_start_matches('#').to_lowercase();
        let filter_channel_key = channel_filter_key(&msg.provider, &channel_key);

        let mut flags = 0u32;
        if msg.metadata.is_action {
            flags |= F_ACTION;
        }
        if msg.metadata.is_first_message || tag("first-msg") == "1" {
            flags |= F_FIRST;
        }
        if tag("returning-chatter") == "1" {
            flags |= F_RETURNING;
        }
        if bits > 0 {
            flags |= F_CHEER;
        }
        if msg_id_tag == "highlighted-message" {
            flags |= F_HIGHLIGHTED | F_REDEEMED;
        }
        if !reward_id.is_empty() {
            flags |= F_REDEEMED;
        }
        if msg.metadata.reply_info.is_some() {
            flags |= F_REPLY;
        }
        if msg.metadata.is_from_shared_chat {
            flags |= F_SHARED;
        }
        if SUB_TYPES.contains(&msg_type) || SUB_TYPES.contains(&msg_id_tag) {
            flags |= F_SUB;
        }
        if msg_type == "raid" || msg_id_tag == "raid" {
            flags |= F_RAID;
        }
        if msg_type == "viewermilestone" || msg_id_tag == "viewermilestone" {
            flags |= F_STREAK;
        }
        if (!msg_type.is_empty() || msg.metadata.system_message.is_some())
            && msg.content.trim().is_empty()
        {
            flags |= F_SYSTEM;
        }
        if msg.user_id == "tw-system" {
            flags |= F_SYSTEM;
        }
        if has_link(&msg.content) {
            flags |= F_LINK;
        }
        let suspicious =
            crate::services::suspicious_users::SuspiciousUsers::status(&channel_key, &msg.user_id);
        match suspicious.as_deref() {
            Some("restricted") => flags |= F_RESTRICTED,
            Some("monitored") => flags |= F_MONITORED,
            _ => {}
        }
        let ts_ms = tag("tmi-sent-ts")
            .parse::<i64>()
            .ok()
            .or_else(|| msg.timestamp.parse::<i64>().ok())
            .unwrap_or(0);

        MessageFacts {
            login,
            display_lower,
            user_id: &msg.user_id,
            color: msg.color.as_deref().unwrap_or(""),
            badge_keys,
            badge_names,
            sub_length,
            subbed,
            bits,
            channel_key,
            filter_channel_key,
            content: &msg.content,
            flags,
            reward_id,
            reply_parent_user_id: msg
                .metadata
                .reply_info
                .as_ref()
                .map(|r| r.parent_user_id.as_str())
                .unwrap_or(""),
            msg_type,
            ts_ms,
            suspicious,
        }
    }
}

fn has_link(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("http://") || lower.contains("https://") || lower.contains("www.")
}

/// `@login` followed by end-of-text or a non-word character, case-insensitive.
/// Same boundary rule the React row used.
pub fn mentions_login(content: &str, login: &str) -> bool {
    if login.is_empty() {
        return false;
    }
    let lower = content.to_lowercase();
    let needle = format!("@{}", login.to_lowercase());
    let mut from = 0;
    while let Some(pos) = lower[from..].find(&needle) {
        let after = from + pos + needle.len();
        match lower[after..].chars().next() {
            None => return true,
            Some(c) if !(c.is_ascii_alphanumeric() || c == '_') => return true,
            _ => from = after,
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Engine entry points
// ---------------------------------------------------------------------------

static RULES: OnceLock<RwLock<Arc<CompiledRules>>> = OnceLock::new();
static RULES_HASH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static OWN: OnceLock<RwLock<Option<(String, String)>>> = OnceLock::new();

fn rules_cell() -> &'static RwLock<Arc<CompiledRules>> {
    RULES.get_or_init(|| RwLock::new(Arc::new(CompiledRules::default())))
}

fn own_cell() -> &'static RwLock<Option<(String, String)>> {
    OWN.get_or_init(|| RwLock::new(None))
}

pub struct ChatRules;

pub struct Verdict {
    /// The message must not reach any window.
    pub drop: bool,
    pub matched_highlight: bool,
    pub mentioned: bool,
}

/// Full result of one evaluation, applied to the message after the borrowed
/// facts are released.
struct Evaluation {
    drop: bool,
    mentioned: bool,
    reply_to_me: bool,
    highlight: Option<HighlightStamp>,
    built_in: Option<BuiltInStamp>,
    filter_ids: Vec<String>,
    suspicious: Option<String>,
}

impl Evaluation {
    fn dropped() -> Self {
        Evaluation {
            drop: true,
            mentioned: false,
            reply_to_me: false,
            highlight: None,
            built_in: None,
            filter_ids: Vec::new(),
            suspicious: None,
        }
    }
}

impl ChatRules {
    /// Current compiled snapshot. One RwLock read + Arc clone.
    pub fn snapshot() -> Arc<CompiledRules> {
        rules_cell()
            .read()
            .map(|g| g.clone())
            .unwrap_or_else(|_| Arc::new(CompiledRules::default()))
    }

    /// (login, user id) of the IRC account, for mention and self detection.
    pub fn set_own_identity(login: &str, user_id: &str) {
        if let Ok(mut g) = own_cell().write() {
            *g = Some((login.to_lowercase(), user_id.to_string()));
        }
    }

    fn own_identity() -> Option<(String, String)> {
        own_cell().read().ok().and_then(|g| g.clone())
    }

    /// Recompile from the live settings when the rule groups changed. Cheap
    /// to call on every settings save: it hashes the three JSON groups first.
    pub fn refresh(settings: &Settings) {
        // Timestamp clock lives next to the rules: same settings event.
        crate::services::irc_service::TIMESTAMP_24H.store(
            settings.chat_design.timestamp_format == "24h",
            std::sync::atomic::Ordering::Relaxed,
        );
        let hl_v = settings.extra.get("chat_highlights");
        let cf_v = settings.extra.get("chat_filters");
        let cq_v = settings.extra.get("chat_query");
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for v in [hl_v, cf_v, cq_v] {
            match v {
                Some(v) => v.to_string().hash(&mut hasher),
                None => 0u8.hash(&mut hasher),
            }
        }
        let h = hasher.finish().max(1);
        if RULES_HASH.load(std::sync::atomic::Ordering::Acquire) == h {
            return;
        }
        let hl: ChatHighlightSettings = hl_v
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let cf: ChatFilterSettings = cf_v
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let cq: ChatQuerySettings = cq_v
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let compiled = CompiledRules::compile(&hl, &cf, &cq);
        for e in &compiled.errors {
            log::warn!(
                "[ChatRules] {} rule {} skipped: {}",
                e.scope,
                e.id,
                e.error
            );
        }
        log::info!(
            "[ChatRules] compiled: {} phrases, {} users, {} badges, {} ignore phrases, {} filters, {} hidden users, cap {}",
            compiled.phrase_meta.len(),
            compiled.users.len(),
            compiled.badges.len(),
            compiled.ignore_set.as_ref().map(|s| s.len()).unwrap_or(0),
            compiled.filters.len(),
            compiled.hidden_global.len(),
            compiled.history_cap
        );
        crate::services::chat_history::ChatHistory::set_cap(compiled.history_cap);
        if let Ok(mut g) = rules_cell().write() {
            *g = Arc::new(compiled);
        }
        RULES_HASH.store(h, std::sync::atomic::Ordering::Release);
    }

    /// Evaluate every rule against one message and stamp the metadata. Returns
    /// the verdict; the caller decides whether a dropped message still feeds
    /// logs and history (it does: the record is not the display).
    pub fn evaluate(msg: &mut ChatMessage, rules: &CompiledRules) -> Verdict {
        let ev = {
            let facts = MessageFacts::from_message(msg);
            let ev = Self::evaluate_facts(&facts, rules);
            if !ev.drop {
                crate::services::chat_history::ChatHistory::push(
                    msg,
                    &facts,
                    ev.highlight.is_some(),
                    ev.mentioned,
                );
            }
            ev
        };
        let verdict = Verdict {
            drop: ev.drop,
            matched_highlight: ev.highlight.is_some(),
            mentioned: ev.mentioned,
        };
        if !ev.drop {
            Self::apply(msg, ev);
        }
        verdict
    }

    fn apply(msg: &mut ChatMessage, ev: Evaluation) {
        msg.metadata.is_mentioned = ev.mentioned;
        msg.metadata.is_reply_to_me = ev.reply_to_me;
        msg.metadata.highlight = ev.highlight;
        msg.metadata.built_in = ev.built_in;
        msg.metadata.filter_ids = ev.filter_ids;
        msg.metadata.suspicious = ev.suspicious;
        msg.metadata.rules_evaluated = true;
    }

    fn evaluate_facts(facts: &MessageFacts<'_>, rules: &CompiledRules) -> Evaluation {
        let own = Self::own_identity();
        let is_own = own
            .as_ref()
            .map(|(_, id)| !id.is_empty() && id == facts.user_id)
            .unwrap_or(false);

        // 1. Ignores. Own messages are exempt so hiding yourself can never
        //    eat your sends (same rule the JS gate had).
        if !is_own {
            if rules.hide_bots
                && (KNOWN_BOTS.contains(&facts.login.as_str())
                    || KNOWN_BOTS.contains(&facts.display_lower.as_str()))
            {
                return Evaluation::dropped();
            }
            if !rules.hidden_global.is_empty()
                && (rules.hidden_global.contains(&facts.login)
                    || rules.hidden_global.contains(&facts.display_lower))
            {
                return Evaluation::dropped();
            }
            if let Some(set) = rules.hidden_per_channel.get(&facts.filter_channel_key) {
                if set.contains(&facts.login) || set.contains(&facts.display_lower) {
                    return Evaluation::dropped();
                }
            }
            if let Some(set) = &rules.ignore_set {
                if set.is_match(facts.content) {
                    return Evaluation::dropped();
                }
            }
        }

        // 2. Mention and reply-to-me.
        let mentioned = own
            .as_ref()
            .map(|(login, _)| !is_own && mentions_login(facts.content, login))
            .unwrap_or(false);
        let reply_to_me = own
            .as_ref()
            .map(|(_, id)| !id.is_empty() && !is_own && facts.reply_parent_user_id == id)
            .unwrap_or(false);

        // 3. Highlight rules: phrase, then user, then badge; first wins. A
        //    mention or reply suppresses them so sound and animation never
        //    double-fire (mirrors the retired JS matcher).
        let mut highlight: Option<HighlightStamp> = None;
        if !mentioned && !reply_to_me && !rules.is_empty_highlights() {
            if let Some(set) = &rules.phrase_set {
                if let Some(idx) = set.matches(facts.content).iter().next() {
                    let m = &rules.phrase_meta[idx];
                    highlight = Some(stamp("phrase", m));
                }
            }
            if highlight.is_none() && !rules.users.is_empty() {
                let m = rules
                    .users
                    .get(&facts.display_lower)
                    .or_else(|| rules.users.get(&facts.login));
                if let Some(m) = m {
                    highlight = Some(stamp("user", m));
                }
            }
            if highlight.is_none() && !rules.badges.is_empty() {
                'outer: for b in &rules.badges {
                    for k in &facts.badge_keys {
                        let hit = if b.wildcard {
                            k.starts_with(&b.key)
                        } else {
                            k == &b.key
                        };
                        if hit {
                            highlight = Some(stamp("badge", &b.meta));
                            break 'outer;
                        }
                    }
                }
            }
        }

        // 4. Built-in event tint: raid > returning > first-time > self.
        let built_in = if facts.flags & F_RAID != 0 && rules.raider.enabled {
            Some(BuiltInStamp {
                kind: "raider".into(),
                color: rules.raider.color.clone(),
                label: "Raid".into(),
            })
        } else if facts.flags & F_RETURNING != 0 && rules.returning.enabled {
            Some(BuiltInStamp {
                kind: "returning".into(),
                color: rules.returning.color.clone(),
                label: "Returning chatter".into(),
            })
        } else if facts.flags & F_FIRST != 0 && rules.first_time.enabled {
            Some(BuiltInStamp {
                kind: "first_time".into(),
                color: rules.first_time.color.clone(),
                label: "First message in chat".into(),
            })
        } else if is_own && rules.self_message.enabled {
            Some(BuiltInStamp {
                kind: "self".into(),
                color: rules.self_message.color.clone(),
                label: "You".into(),
            })
        } else {
            None
        };

        // 5. Saved filters: every enabled one, ids of the matches.
        let mut filter_ids = Vec::new();
        if !rules.filters.is_empty() {
            let ctx = EvalCtx { facts, mentioned, highlighted: highlight.is_some() };
            for f in &rules.filters {
                if eval(&f.expr, &ctx).truthy() {
                    filter_ids.push(f.id.clone());
                }
            }
        }

        Evaluation {
            drop: false,
            mentioned,
            reply_to_me,
            highlight,
            built_in,
            filter_ids,
            suspicious: facts.suspicious.clone(),
        }
    }

    /// Chatterino-syntax filter check for the settings editor. Ok(warnings)
    /// lists variables StreamNook cannot populate.
    pub fn validate_filter(expr: &str) -> Result<Vec<String>, String> {
        let ast = parse_filter(expr)?;
        let mut unsupported = Vec::new();
        collect_unsupported(&ast, &mut unsupported);
        unsupported.sort();
        unsupported.dedup();
        Ok(unsupported)
    }

    /// Rust-side regex validation for highlight and ignore phrases, so the
    /// settings UI reports the dialect that actually runs.
    pub fn validate_phrase(
        pattern: &str,
        is_regex: bool,
        whole_word: bool,
        case_sensitive: bool,
    ) -> Result<(), String> {
        if pattern.trim().is_empty() {
            return Ok(());
        }
        Regex::new(&phrase_pattern(pattern, is_regex, whole_word, case_sensitive))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

fn stamp(kind: &str, m: &RuleMeta) -> HighlightStamp {
    HighlightStamp {
        rule_id: m.id.clone(),
        kind: kind.to_string(),
        color: m.color.clone(),
        sound_id: m.sound_id.clone(),
        cooldown_ms: m.cooldown_ms,
    }
}

// ---------------------------------------------------------------------------
// Filter expression language (Chatterino syntax)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<Value>),
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Str(s) => !s.is_empty(),
            Value::List(l) => !l.is_empty(),
        }
    }
    fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
    fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }
    fn display(&self) -> String {
        match self {
            Value::Null => String::new(),
            Value::Bool(b) => b.to_string(),
            Value::Int(i) => i.to_string(),
            Value::Str(s) => s.clone(),
            Value::List(l) => l.iter().map(|v| v.display()).collect::<Vec<_>>().join(","),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BinOp {
    And,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Contains,
    StartsWith,
    EndsWith,
    In,
    Match,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

#[derive(Debug)]
pub enum Expr {
    Lit(Value),
    Var(String),
    List(Vec<Expr>),
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    /// `lhs match "literal"`, regex compiled at parse time.
    MatchRe(Box<Expr>, Regex),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(i64),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Comma,
    Op(String),
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            let mut s = String::new();
            i += 1;
            let mut closed = false;
            while i < chars.len() {
                let d = chars[i];
                if d == '\\' && i + 1 < chars.len() {
                    let e = chars[i + 1];
                    s.push(match e {
                        'n' => '\n',
                        't' => '\t',
                        other => other,
                    });
                    i += 2;
                    continue;
                }
                if d == '"' {
                    closed = true;
                    i += 1;
                    break;
                }
                s.push(d);
                i += 1;
            }
            if !closed {
                return Err("unterminated string literal".into());
            }
            out.push(Tok::Str(s));
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            out.push(Tok::Num(text.parse::<i64>().map_err(|e| e.to_string())?));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            out.push(Tok::Ident(text));
            continue;
        }
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        if ["&&", "||", "==", "!=", "<=", ">="].contains(&two.as_str()) {
            out.push(Tok::Op(two));
            i += 2;
            continue;
        }
        match c {
            '(' => out.push(Tok::LParen),
            ')' => out.push(Tok::RParen),
            '{' => out.push(Tok::LBrace),
            '}' => out.push(Tok::RBrace),
            ',' => out.push(Tok::Comma),
            '!' | '<' | '>' | '+' | '-' | '*' | '/' | '%' => out.push(Tok::Op(c.to_string())),
            other => return Err(format!("unexpected character '{}'", other)),
        }
        i += 1;
    }
    Ok(out)
}

const KNOWN_VARS: &[&str] = &[
    "author.badges",
    "author.external_badges",
    "author.color",
    "author.name",
    "author.user_id",
    "author.no_color",
    "author.subbed",
    "author.sub_length",
    "bits.amount",
    "channel.name",
    "channel.watching",
    "channel.live",
    "flags.action",
    "flags.automod",
    "flags.cheer_message",
    "flags.first_message",
    "flags.highlighted",
    "flags.points_redeemed",
    "flags.reply",
    "flags.restricted",
    "flags.monitored",
    "flags.reward_message",
    "flags.shared",
    "flags.sub_message",
    "flags.system_message",
    "flags.whisper",
    "flags.similar",
    "flags.watch_streak",
    "flags.mention",
    "flags.highlight_rule",
    "message.content",
    "message.length",
    "reward.title",
    "reward.cost",
    "reward.id",
];

/// Variables Chatterino populates that StreamNook has no data for (yet).
const UNSUPPORTED_VARS: &[&str] = &[
    "author.external_badges",
    "channel.watching",
    "channel.live",
    "flags.automod",
    "flags.whisper",
    "flags.similar",
    "reward.title",
    "reward.cost",
];

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.pos).cloned();
        self.pos += 1;
        t
    }
    fn eat_op(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Op(o)) if o == op) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_and()?;
        while self.eat_op("||") {
            let rhs = self.parse_and()?;
            lhs = Expr::Bin(BinOp::Or, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_cmp()?;
        while self.eat_op("&&") {
            let rhs = self.parse_cmp()?;
            lhs = Expr::Bin(BinOp::And, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }
    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let lhs = self.parse_add()?;
        let op = match self.peek() {
            Some(Tok::Op(o)) => match o.as_str() {
                "==" => Some(BinOp::Eq),
                "!=" => Some(BinOp::Ne),
                "<" => Some(BinOp::Lt),
                "<=" => Some(BinOp::Le),
                ">" => Some(BinOp::Gt),
                ">=" => Some(BinOp::Ge),
                _ => None,
            },
            Some(Tok::Ident(w)) => match w.as_str() {
                "contains" => Some(BinOp::Contains),
                "startswith" => Some(BinOp::StartsWith),
                "endswith" => Some(BinOp::EndsWith),
                "in" => Some(BinOp::In),
                "match" => Some(BinOp::Match),
                _ => None,
            },
            _ => None,
        };
        let Some(op) = op else { return Ok(lhs) };
        self.pos += 1;
        let rhs = self.parse_add()?;
        if op == BinOp::Match {
            return match rhs {
                Expr::Lit(Value::Str(pat)) => {
                    let re = Regex::new(&pat).map_err(|e| format!("bad regex: {}", e))?;
                    Ok(Expr::MatchRe(Box::new(lhs), re))
                }
                Expr::List(items) => {
                    // Chatterino's `{regex, group}` form: honour the regex, ignore the group.
                    match items.into_iter().next() {
                        Some(Expr::Lit(Value::Str(pat))) => {
                            let re = Regex::new(&pat).map_err(|e| format!("bad regex: {}", e))?;
                            Ok(Expr::MatchRe(Box::new(lhs), re))
                        }
                        _ => Err("match needs a string literal regex".into()),
                    }
                }
                _ => Err("match needs a string literal regex".into()),
            };
        }
        Ok(Expr::Bin(op, Box::new(lhs), Box::new(rhs)))
    }
    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_mul()?;
        loop {
            if self.eat_op("+") {
                let rhs = self.parse_mul()?;
                lhs = Expr::Bin(BinOp::Add, Box::new(lhs), Box::new(rhs));
            } else if self.eat_op("-") {
                let rhs = self.parse_mul()?;
                lhs = Expr::Bin(BinOp::Sub, Box::new(lhs), Box::new(rhs));
            } else {
                return Ok(lhs);
            }
        }
    }
    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_unary()?;
        loop {
            if self.eat_op("*") {
                let rhs = self.parse_unary()?;
                lhs = Expr::Bin(BinOp::Mul, Box::new(lhs), Box::new(rhs));
            } else if self.eat_op("/") {
                let rhs = self.parse_unary()?;
                lhs = Expr::Bin(BinOp::Div, Box::new(lhs), Box::new(rhs));
            } else if self.eat_op("%") {
                let rhs = self.parse_unary()?;
                lhs = Expr::Bin(BinOp::Mod, Box::new(lhs), Box::new(rhs));
            } else {
                return Ok(lhs);
            }
        }
    }
    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.eat_op("!") {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        if self.eat_op("-") {
            return Ok(Expr::Neg(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }
    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Some(Tok::Num(n)) => Ok(Expr::Lit(Value::Int(n))),
            Some(Tok::Str(s)) => Ok(Expr::Lit(Value::Str(s))),
            Some(Tok::Ident(w)) => match w.as_str() {
                "true" => Ok(Expr::Lit(Value::Bool(true))),
                "false" => Ok(Expr::Lit(Value::Bool(false))),
                "contains" | "startswith" | "endswith" | "in" | "match" => {
                    Err(format!("'{}' needs a left-hand value", w))
                }
                _ => {
                    if KNOWN_VARS.contains(&w.as_str()) {
                        Ok(Expr::Var(w))
                    } else {
                        Err(format!("unknown variable '{}'", w))
                    }
                }
            },
            Some(Tok::LParen) => {
                let e = self.parse_or()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err("expected ')'".into()),
                }
            }
            Some(Tok::LBrace) => {
                let mut items = Vec::new();
                if matches!(self.peek(), Some(Tok::RBrace)) {
                    self.pos += 1;
                    return Ok(Expr::List(items));
                }
                loop {
                    items.push(self.parse_or()?);
                    match self.next() {
                        Some(Tok::Comma) => continue,
                        Some(Tok::RBrace) => break,
                        _ => return Err("expected ',' or '}' in list".into()),
                    }
                }
                Ok(Expr::List(items))
            }
            Some(t) => Err(format!("unexpected token {:?}", t)),
            None => Err("unexpected end of expression".into()),
        }
    }
}

pub fn parse_filter(src: &str) -> Result<Expr, String> {
    let toks = tokenize(src)?;
    if toks.is_empty() {
        return Err("empty filter".into());
    }
    let mut p = Parser { toks, pos: 0 };
    let e = p.parse_or()?;
    if p.pos != p.toks.len() {
        return Err(format!("unexpected trailing token {:?}", p.toks[p.pos]));
    }
    Ok(e)
}

fn collect_unsupported(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::Var(v) => {
            if UNSUPPORTED_VARS.contains(&v.as_str()) {
                out.push(v.clone());
            }
        }
        Expr::List(items) => items.iter().for_each(|i| collect_unsupported(i, out)),
        Expr::Not(i) | Expr::Neg(i) => collect_unsupported(i, out),
        Expr::Bin(_, a, b) => {
            collect_unsupported(a, out);
            collect_unsupported(b, out);
        }
        Expr::MatchRe(a, _) => collect_unsupported(a, out),
        Expr::Lit(_) => {}
    }
}

struct EvalCtx<'a, 'b> {
    facts: &'b MessageFacts<'a>,
    mentioned: bool,
    highlighted: bool,
}

fn var(name: &str, ctx: &EvalCtx<'_, '_>) -> Value {
    let f = ctx.facts;
    let flag = |bit: u32| Value::Bool(f.flags & bit != 0);
    match name {
        "author.badges" => Value::List(f.badge_names.iter().cloned().map(Value::Str).collect()),
        "author.external_badges" => Value::List(Vec::new()),
        "author.color" => Value::Str(f.color.to_string()),
        "author.name" => Value::Str(f.login.clone()),
        "author.user_id" => Value::Str(f.user_id.to_string()),
        "author.no_color" => Value::Bool(f.color.is_empty()),
        "author.subbed" => Value::Bool(f.subbed),
        "author.sub_length" => Value::Int(f.sub_length),
        "bits.amount" => Value::Int(f.bits),
        "channel.name" => Value::Str(f.channel_key.clone()),
        "channel.watching" | "channel.live" => Value::Bool(false),
        "flags.action" => flag(F_ACTION),
        "flags.automod" | "flags.whisper" | "flags.similar" => Value::Bool(false),
        "flags.restricted" => flag(F_RESTRICTED),
        "flags.monitored" => flag(F_MONITORED),
        "flags.cheer_message" => flag(F_CHEER),
        "flags.first_message" => flag(F_FIRST),
        "flags.highlighted" => flag(F_HIGHLIGHTED),
        "flags.points_redeemed" => flag(F_REDEEMED),
        "flags.reply" => flag(F_REPLY),
        "flags.reward_message" => Value::Bool(!f.reward_id.is_empty()),
        "flags.shared" => flag(F_SHARED),
        "flags.sub_message" => flag(F_SUB),
        "flags.system_message" => flag(F_SYSTEM),
        "flags.watch_streak" => flag(F_STREAK),
        "flags.mention" => Value::Bool(ctx.mentioned),
        "flags.highlight_rule" => Value::Bool(ctx.highlighted),
        "message.content" => Value::Str(f.content.to_string()),
        "message.length" => Value::Int(f.content.chars().count() as i64),
        "reward.title" => Value::Str(String::new()),
        "reward.cost" => Value::Int(0),
        "reward.id" => Value::Str(f.reward_id.clone()),
        _ => Value::Null,
    }
}

fn str_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x.eq_ignore_ascii_case(y),
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Bool(x), Value::Int(y)) | (Value::Int(y), Value::Bool(x)) => (*x as i64) == *y,
        (Value::List(x), Value::List(y)) => x.len() == y.len() && x.iter().zip(y).all(|(a, b)| str_eq(a, b)),
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

fn list_contains(list: &[Value], needle: &Value) -> bool {
    list.iter().any(|v| str_eq(v, needle))
}

fn eval(e: &Expr, ctx: &EvalCtx<'_, '_>) -> Value {
    match e {
        Expr::Lit(v) => v.clone(),
        Expr::Var(name) => var(name, ctx),
        Expr::List(items) => Value::List(items.iter().map(|i| eval(i, ctx)).collect()),
        Expr::Not(i) => Value::Bool(!eval(i, ctx).truthy()),
        Expr::Neg(i) => match eval(i, ctx) {
            Value::Int(n) => Value::Int(-n),
            _ => Value::Null,
        },
        Expr::MatchRe(lhs, re) => match eval(lhs, ctx) {
            Value::Str(s) => Value::Bool(re.is_match(&s)),
            Value::List(items) => Value::Bool(items.iter().any(|v| v.as_str().map(|s| re.is_match(s)).unwrap_or(false))),
            other => Value::Bool(re.is_match(&other.display())),
        },
        Expr::Bin(op, a, b) => {
            match op {
                BinOp::And => {
                    let l = eval(a, ctx);
                    if !l.truthy() {
                        return Value::Bool(false);
                    }
                    return Value::Bool(eval(b, ctx).truthy());
                }
                BinOp::Or => {
                    let l = eval(a, ctx);
                    if l.truthy() {
                        return Value::Bool(true);
                    }
                    return Value::Bool(eval(b, ctx).truthy());
                }
                _ => {}
            }
            let l = eval(a, ctx);
            let r = eval(b, ctx);
            match op {
                BinOp::Eq => Value::Bool(str_eq(&l, &r)),
                BinOp::Ne => Value::Bool(!str_eq(&l, &r)),
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => match (l.as_int(), r.as_int()) {
                    (Some(x), Some(y)) => Value::Bool(match op {
                        BinOp::Lt => x < y,
                        BinOp::Le => x <= y,
                        BinOp::Gt => x > y,
                        _ => x >= y,
                    }),
                    _ => Value::Bool(false),
                },
                BinOp::Contains => match (&l, &r) {
                    (Value::List(list), needle) => Value::Bool(list_contains(list, needle)),
                    (Value::Str(hay), Value::Str(needle)) => {
                        Value::Bool(hay.to_lowercase().contains(&needle.to_lowercase()))
                    }
                    (Value::Str(hay), other) => Value::Bool(hay.to_lowercase().contains(&other.display().to_lowercase())),
                    _ => Value::Bool(false),
                },
                BinOp::StartsWith => match (&l, &r) {
                    (Value::Str(hay), needle) => {
                        Value::Bool(hay.to_lowercase().starts_with(&needle.display().to_lowercase()))
                    }
                    (Value::List(list), needle) => Value::Bool(list.first().map(|v| str_eq(v, needle)).unwrap_or(false)),
                    _ => Value::Bool(false),
                },
                BinOp::EndsWith => match (&l, &r) {
                    (Value::Str(hay), needle) => {
                        Value::Bool(hay.to_lowercase().ends_with(&needle.display().to_lowercase()))
                    }
                    (Value::List(list), needle) => Value::Bool(list.last().map(|v| str_eq(v, needle)).unwrap_or(false)),
                    _ => Value::Bool(false),
                },
                BinOp::In => match &r {
                    Value::List(list) => Value::Bool(list_contains(list, &l)),
                    Value::Str(hay) => Value::Bool(hay.to_lowercase().contains(&l.display().to_lowercase())),
                    _ => Value::Bool(false),
                },
                BinOp::Add => match (&l, &r) {
                    (Value::Int(x), Value::Int(y)) => Value::Int(x.wrapping_add(*y)),
                    (Value::Str(_), _) | (_, Value::Str(_)) => Value::Str(format!("{}{}", l.display(), r.display())),
                    _ => Value::Null,
                },
                BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod => match (l.as_int(), r.as_int()) {
                    (Some(x), Some(y)) => match op {
                        BinOp::Sub => Value::Int(x.wrapping_sub(y)),
                        BinOp::Mul => Value::Int(x.wrapping_mul(y)),
                        BinOp::Div => {
                            if y == 0 {
                                Value::Null
                            } else {
                                Value::Int(x / y)
                            }
                        }
                        _ => {
                            if y == 0 {
                                Value::Null
                            } else {
                                Value::Int(x % y)
                            }
                        }
                    },
                    _ => Value::Null,
                },
                BinOp::And | BinOp::Or | BinOp::Match => Value::Null,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::chat_layout::{Badge, ChatMessage};

    fn msg(login: &str, content: &str) -> ChatMessage {
        let mut m = ChatMessage {
            id: "m1".into(),
            user_id: "42".into(),
            username: login.into(),
            display_name: login.to_uppercase(),
            color: Some("#ff0000".into()),
            badges: vec![Badge {
                name: "subscriber".into(),
                version: "12".into(),
                image_url_1x: None,
                image_url_2x: None,
                image_url_4x: None,
                title: None,
                description: None,
            }],
            timestamp: "0".into(),
            content: content.into(),
            provider: "twitch".into(),
            channel: "#Forsen".into(),
            emotes: vec![],
            tags: HashMap::new(),
            layout: Default::default(),
            segments: vec![],
            metadata: Default::default(),
        };
        m.tags.insert("badge-info".into(), "subscriber/13".into());
        m
    }

    fn rules_from(hl: &str, cf: &str, cq: &str) -> CompiledRules {
        let hl: ChatHighlightSettings = serde_json::from_str(hl).unwrap();
        let cf: ChatFilterSettings = serde_json::from_str(cf).unwrap();
        let cq: ChatQuerySettings = serde_json::from_str(cq).unwrap();
        CompiledRules::compile(&hl, &cf, &cq)
    }

    /// Evaluate and stamp without touching the global history ring.
    fn eval_with(rules: &CompiledRules, m: &mut ChatMessage) -> Verdict {
        let ev = {
            let facts = MessageFacts::from_message(m);
            ChatRules::evaluate_facts(&facts, rules)
        };
        let v = Verdict {
            drop: ev.drop,
            matched_highlight: ev.highlight.is_some(),
            mentioned: ev.mentioned,
        };
        if !ev.drop {
            ChatRules::apply(m, ev);
        }
        v
    }

    #[test]
    fn phrase_highlight_first_in_list_order_wins() {
        let rules = rules_from(
            r##"{"phrases":[
                {"id":"a","pattern":"hello","enabled":true,"color":"#1"},
                {"id":"b","pattern":"world","enabled":true,"color":"#2"}
            ]}"##,
            "{}",
            "{}",
        );
        let mut m = msg("bob", "well HELLO world");
        let v = eval_with(&rules, &mut m);
        assert!(!v.drop);
        let hl = m.metadata.highlight.as_ref().unwrap();
        assert_eq!(hl.rule_id, "a");
        assert_eq!(hl.kind, "phrase");
        assert_eq!(hl.cooldown_ms, 3000);
    }

    #[test]
    fn whole_word_and_case_sensitive_phrases() {
        let rules = rules_from(
            r##"{"phrases":[
                {"id":"w","pattern":"cat","enabled":true,"whole_word":true,"color":"#1"},
                {"id":"c","pattern":"Dog","enabled":true,"case_sensitive":true,"color":"#2"}
            ]}"##,
            "{}",
            "{}",
        );
        let mut m = msg("bob", "concatenate");
        eval_with(&rules, &mut m);
        assert!(m.metadata.highlight.is_none());
        let mut m = msg("bob", "a cat sat");
        eval_with(&rules, &mut m);
        assert_eq!(m.metadata.highlight.as_ref().unwrap().rule_id, "w");
        let mut m = msg("bob", "hot dog");
        eval_with(&rules, &mut m);
        assert!(m.metadata.highlight.is_none());
        let mut m = msg("bob", "Dog park");
        eval_with(&rules, &mut m);
        assert_eq!(m.metadata.highlight.as_ref().unwrap().rule_id, "c");
    }

    #[test]
    fn invalid_regex_is_reported_not_fatal() {
        let rules = rules_from(
            r##"{"phrases":[{"id":"bad","pattern":"(","enabled":true,"is_regex":true,"color":"#1"}]}"##,
            "{}",
            "{}",
        );
        assert_eq!(rules.errors.len(), 1);
        assert_eq!(rules.errors[0].id, "bad");
        assert!(rules.phrase_set.is_none());
    }

    #[test]
    fn user_and_badge_rules_with_wildcard() {
        let rules = rules_from(
            r##"{"users":[{"id":"u","enabled":true,"username":"Bob","color":"#u"}],
                "badges":[{"id":"b","enabled":true,"badge_key":"subscriber/*","color":"#b"}]}"##,
            "{}",
            "{}",
        );
        let mut m = msg("bob", "hi");
        eval_with(&rules, &mut m);
        assert_eq!(m.metadata.highlight.as_ref().unwrap().rule_id, "u");
        let mut m = msg("alice", "hi");
        eval_with(&rules, &mut m);
        assert_eq!(m.metadata.highlight.as_ref().unwrap().rule_id, "b");
    }

    #[test]
    fn mention_suppresses_rule_highlight_and_sets_flag() {
        ChatRules::set_own_identity("Brandon", "999");
        let rules = rules_from(
            r##"{"phrases":[{"id":"a","pattern":"hey","enabled":true,"color":"#1"}]}"##,
            "{}",
            "{}",
        );
        let mut m = msg("bob", "hey @brandon!");
        let v = eval_with(&rules, &mut m);
        assert!(v.mentioned);
        assert!(m.metadata.is_mentioned);
        assert!(m.metadata.highlight.is_none());
        // Not a mention when the login continues as a longer word.
        let mut m = msg("bob", "hey @brandonx");
        eval_with(&rules, &mut m);
        assert!(!m.metadata.is_mentioned);
        assert!(m.metadata.highlight.is_some());
    }

    #[test]
    fn ignores_drop_bots_users_channels_and_phrases() {
        ChatRules::set_own_identity("brandon", "999");
        let rules = rules_from(
            "{}",
            r##"{"hide_bots":true,"hidden_users":["@Alice"],
                "per_channel":{"forsen":["carl"],"kick:xqc":["dave"]},
                "ignored_phrases":[{"id":"p","pattern":"buy followers","enabled":true}]}"##,
            "{}",
        );
        assert!(eval_with(&rules, &mut msg("nightbot", "x")).drop);
        assert!(eval_with(&rules, &mut msg("alice", "x")).drop);
        assert!(eval_with(&rules, &mut msg("carl", "x")).drop);
        assert!(!eval_with(&rules, &mut msg("dave", "x")).drop);
        let mut kick = msg("dave", "x");
        kick.provider = "kick".into();
        kick.channel = "xqc".into();
        assert!(eval_with(&rules, &mut kick).drop);
        assert!(eval_with(&rules, &mut msg("eve", "BUY FOLLOWERS now")).drop);
        assert!(!eval_with(&rules, &mut msg("eve", "hello")).drop);
        // Own messages are never dropped.
        let mut own = msg("alice", "buy followers");
        own.user_id = "999".into();
        assert!(!eval_with(&rules, &mut own).drop);
    }

    #[test]
    fn built_in_precedence_and_defaults() {
        ChatRules::set_own_identity("brandon", "999");
        let rules = rules_from(r##"{"built_in":{"raider":{"enabled":true}}}"##, "{}", "{}");
        let mut m = msg("bob", "hi");
        m.tags.insert("first-msg".into(), "1".into());
        eval_with(&rules, &mut m);
        assert_eq!(m.metadata.built_in.as_ref().unwrap().kind, "first_time");
        assert_eq!(m.metadata.built_in.as_ref().unwrap().color, "#a855f7");
        m.tags.insert("msg-id".into(), "raid".into());
        eval_with(&rules, &mut m);
        assert_eq!(m.metadata.built_in.as_ref().unwrap().kind, "raider");
        let mut own = msg("brandon", "hi");
        own.user_id = "999".into();
        eval_with(&rules, &mut own);
        assert!(own.metadata.built_in.is_none(), "self is off by default");
    }

    #[test]
    fn filter_grammar_matches_chatterino_examples() {
        let rules = rules_from(
            "{}",
            "{}",
            r##"{"filters":[
                {"id":"subs","expr":"author.subbed && author.sub_length >= 12","enabled":true},
                {"id":"mods","expr":"author.badges contains \"moderator\"","enabled":true},
                {"id":"long","expr":"message.length > 5 && !flags.reply","enabled":true},
                {"id":"names","expr":"author.name in {\"bob\", \"alice\"}","enabled":true},
                {"id":"re","expr":"message.content match \"(?i)^hel+o\"","enabled":true},
                {"id":"bits","expr":"bits.amount / 100 >= 1 || flags.cheer_message","enabled":true},
                {"id":"chan","expr":"channel.name == \"forsen\" && author.color startswith \"#ff\"","enabled":true}
            ]}"##,
        );
        assert!(rules.errors.is_empty(), "{:?}", rules.errors);
        let mut m = msg("bob", "helllo there");
        eval_with(&rules, &mut m);
        let ids = m.metadata.filter_ids.clone();
        assert!(ids.contains(&"subs".to_string()));
        assert!(!ids.contains(&"mods".to_string()));
        assert!(ids.contains(&"long".to_string()));
        assert!(ids.contains(&"names".to_string()));
        assert!(ids.contains(&"re".to_string()));
        assert!(!ids.contains(&"bits".to_string()));
        assert!(ids.contains(&"chan".to_string()));
    }

    #[test]
    fn filter_validation_reports_errors_and_unsupported_vars() {
        assert!(ChatRules::validate_filter("author.name ==").is_err());
        assert!(ChatRules::validate_filter("author.nmae == \"x\"").is_err());
        assert!(ChatRules::validate_filter("(flags.reply").is_err());
        let warn = ChatRules::validate_filter("flags.automod || channel.live").unwrap();
        assert_eq!(warn, vec!["channel.live".to_string(), "flags.automod".to_string()]);
        assert!(ChatRules::validate_filter("message.content match \"(\"").is_err());
    }

    #[test]
    fn channel_filter_key_normalizes_bare_and_composite() {
        assert_eq!(channel_filter_key("twitch", "#Forsen"), "twitch:forsen");
        assert_eq!(channel_filter_key("", "Forsen"), "twitch:forsen");
        assert_eq!(channel_filter_key("kick", "XQC"), "kick:xqc");
        assert_eq!(channel_filter_key("twitch", "kick:XQC"), "kick:xqc");
    }

    #[test]
    fn mention_boundary_rules() {
        assert!(mentions_login("gg @Brandon", "brandon"));
        assert!(mentions_login("@brandon, hi", "brandon"));
        assert!(!mentions_login("@brandonfan hi", "brandon"));
        assert!(mentions_login("@brandonfan and @brandon", "brandon"));
        assert!(!mentions_login("brandon", "brandon"));
    }
}
