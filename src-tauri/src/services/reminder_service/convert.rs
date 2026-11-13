//! "Make it a rule", "Turn back on" and the Reminders writer (§ 11.4, D281).
//!
//! Reminders stay reminders: same storage (`extra["reminders"]`), same editor, same `/remind`.
//! This module adds what the Automations page needs: the list it shows
//! ([`summaries`]), the field-level writer the iPhone uses ([`apply_op`]), and the two writes
//! that move a reminder into a rule and back ([`convert_in`], [`restore_in`]). Each takes
//! `&mut Settings` and runs inside `commands::settings::commit_settings`, so one closure changes
//! `automations` and `reminders` together under the settings lock and a refusal changes nothing.
//!
//! The convert never runs on its own, never migrates anything, and builds the rule as a raw JSON
//! value read back through `Rule::from_value` (so it compiles whatever triggers a phase has added).

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{reminder_name, spec_of, Reminder};
use crate::models::automations::{
    fill_ids, validate, AccountRef, Automations, ClaimAs, FieldError, Rule, ValidateCtx,
};
use crate::models::settings::Settings;
use crate::services::account_automations::reminder_claim::ReminderKind;

/// One reminder as the Automations group and the iPhone's Reminders section list it. The same
/// fields on every platform.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ReminderSummary {
    pub id: String,
    /// The label, else the message.
    pub name: String,
    /// "Every 10 minutes", "Daily at 21:00", "When someone says \"hello\"".
    pub schedule: String,
    /// "#channel" or "Current channel".
    pub chat: String,
    /// On and not turned into a rule.
    pub enabled: bool,
    /// The id is in `converted_reminders`: the reminder is a rule now.
    pub converted: bool,
    /// When it last posted since the app started.
    pub last_fired_ms: Option<i64>,
}

/// The Reminders writer's op. On the wire: `{ "op": "upsert", "reminder": {..} }`,
/// `{ "op": "remove", "id": ".." }`, `{ "op": "set_enabled", "id": "..", "enabled": true }`
/// (an internally tagged enum needs struct variants, D289).
#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ReminderOp {
    Upsert {
        reminder: Value,
    },
    Remove {
        id: String,
    },
    SetEnabled {
        id: String,
        enabled: bool,
    },
    /// D294: put the reminder at 0-based position `to` (clamped to the last place); the order
    /// the list is saved in is the order the page and the engine read.
    Move {
        id: String,
        to: usize,
    },
}

/// One line of "What changes".
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Change {
    pub field: String,
    pub words: String,
}

/// What the convert answers.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Conversion {
    pub rule_id: String,
    pub changes: Vec<Change>,
    pub errors: Vec<FieldError>,
}

/// What the convert needs from outside the settings.
pub struct ConvertEnv {
    /// `ChatRules::own_identity()` as an account: the owner, the claim account and who posts.
    pub owner: AccountRef,
    pub now_ms: i64,
    /// The room id of a pinned chat the reminder only knew by login, looked up before the lock.
    pub room_id: Option<String>,
    /// `automation_prefs.other_channels`, for the words about another streamer's chat.
    pub other_channels_on: bool,
}

fn id_of(v: &Value) -> &str {
    v.get("id").and_then(Value::as_str).unwrap_or_default()
}

fn read_list(settings: &Settings) -> Vec<Value> {
    settings
        .extra
        .get("reminders")
        .and_then(|g| g.get("reminders"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Write the list back, keeping any other key the group holds.
fn write_list(settings: &mut Settings, list: Vec<Value>) {
    let group = settings
        .extra
        .entry("reminders".to_string())
        .or_insert_with(|| json!({}));
    if let Value::Object(map) = group {
        map.insert("reminders".to_string(), Value::Array(list));
    } else {
        *group = json!({ "reminders": list });
    }
}

// ---- the list ----------------------------------------------------------------------------------

/// "1 hour 30 minutes", "10 minutes", "45 seconds".
pub(crate) fn long_duration(total: u64) -> String {
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    let part = |n: u64, unit: &str| {
        (n > 0).then(|| format!("{n} {unit}{}", if n == 1 { "" } else { "s" }))
    };
    let parts: Vec<String> = [part(h, "hour"), part(m, "minute"), part(s, "second")]
        .into_iter()
        .flatten()
        .collect();
    if parts.is_empty() {
        "0 seconds".to_string()
    } else {
        parts.join(" ")
    }
}

fn schedule_words(r: &Reminder) -> String {
    match r.trigger.as_str() {
        "interval" => format!("Every {}", long_duration(r.interval())),
        "delay" => format!("{} after you join", long_duration(r.delay())),
        "clock" => format!("Daily at {}", r.clock_time.as_deref().unwrap_or("?").trim()),
        "uptime" => format!("At {} of uptime", long_duration(r.uptime())),
        "keyword" => format!(
            "When someone says \"{}\"",
            r.keyword.as_deref().unwrap_or_default().trim()
        ),
        _ => "Unknown trigger".to_string(),
    }
}

fn chat_words(r: &Reminder) -> String {
    match r.pinned_channel() {
        Some(login) if !login.is_empty() => format!("#{login}"),
        _ => "Current channel".to_string(),
    }
}

/// Every reminder, in the order the settings hold them (a reminder this build cannot read is left out).
pub fn summaries(settings: &Settings) -> Vec<ReminderSummary> {
    let converted = &settings.automations.converted_reminders;
    read_list(settings)
        .into_iter()
        .filter_map(|raw| {
            let r: Reminder = serde_json::from_value(raw).ok()?;
            let is_converted = converted.contains(&r.id);
            Some(ReminderSummary {
                name: reminder_name(&r),
                schedule: schedule_words(&r),
                chat: chat_words(&r),
                enabled: r.enabled && !is_converted,
                converted: is_converted,
                last_fired_ms: super::last_fired(&r.id),
                id: r.id,
            })
        })
        .collect()
}

// ---- the writer ----------------------------------------------------------------------------------

/// Merge an upserted reminder over the stored one with the same id key by key (a `null` removes
/// the key, unknown keys stay), or append it. Answers whether anything changed.
fn upsert_in(list: &mut Vec<Value>, reminder: Value) -> Result<bool, String> {
    let Value::Object(incoming) = reminder else {
        return Err("A reminder is an object".into());
    };
    let id = incoming
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("A reminder needs an id")?
        .to_string();
    match list.iter_mut().find(|r| id_of(r) == id) {
        Some(stored) => {
            let before = stored.clone();
            if let Value::Object(map) = &mut *stored {
                for (key, value) in incoming {
                    if value.is_null() {
                        map.remove(&key);
                    } else {
                        map.insert(key, value);
                    }
                }
            }
            Ok(*stored != before)
        }
        None => {
            let clean: Map<String, Value> =
                incoming.into_iter().filter(|(_, v)| !v.is_null()).collect();
            list.push(Value::Object(clean));
            Ok(true)
        }
    }
}

fn move_in(list: &mut Vec<Value>, id: &str, to: usize) -> Result<bool, String> {
    let from = list
        .iter()
        .position(|r| id_of(r) == id)
        .ok_or("That reminder is gone")?;
    let to = to.min(list.len() - 1);
    if from == to {
        return Ok(false);
    }
    let item = list.remove(from);
    list.insert(to, item);
    Ok(true)
}

fn set_enabled_in(list: &mut [Value], id: &str, enabled: bool) -> Result<bool, String> {
    let stored = list
        .iter_mut()
        .find(|r| id_of(r) == id)
        .ok_or("That reminder is gone")?;
    if stored.get("enabled").and_then(Value::as_bool) == Some(enabled) {
        return Ok(false);
    }
    match stored.as_object_mut() {
        Some(map) => {
            map.insert("enabled".to_string(), Value::Bool(enabled));
            Ok(true)
        }
        None => Err("That reminder is damaged".into()),
    }
}

/// Apply one op to `extra["reminders"]`. Answers whether anything changed. The `converted_reminders`
/// list is never touched here: only [`restore_in`] removes an id, and a reminder in the list stays
/// off whatever this writes into its `enabled`.
pub fn apply_op(settings: &mut Settings, op: ReminderOp) -> Result<bool, String> {
    let mut list = read_list(settings);
    let changed = match op {
        ReminderOp::Upsert { reminder } => upsert_in(&mut list, reminder)?,
        ReminderOp::Remove { id } => {
            let before = list.len();
            list.retain(|r| id_of(r) != id);
            list.len() != before
        }
        ReminderOp::SetEnabled { id, enabled } => set_enabled_in(&mut list, &id, enabled)?,
        ReminderOp::Move { id, to } => move_in(&mut list, &id, to)?,
    };
    if changed {
        write_list(settings, list);
    }
    Ok(changed)
}

// ---- Make it a rule ----------------------------------------------------------------------------

/// What a reminder knows by one name and a rule by another.
const RENAMES: &[(&str, &str)] = &[
    ("user", "me"),
    ("stream.title", "title"),
    ("stream.game", "game"),
    ("stream.uptime", "uptime"),
];

/// `{user}` to `{me}`, `{stream.title}` to `{title}`, `{stream.game}` to `{game}`, `{stream.uptime}`
/// to `{uptime}`, every name matched case-insensitively; `{{` stays literal. Answers the new text and
/// each (name as written, new name) it renamed, once each.
pub(crate) fn rewrite_variables(text: &str) -> (String, Vec<(String, String)>) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut renamed: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' && chars.get(i + 1) == Some(&'{') {
            out.push_str("{{");
            i += 2;
            continue;
        }
        if chars[i] == '{' {
            if let Some(end) = chars[i + 1..].iter().position(|&c| c == '}') {
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                if let Some((_, to)) = RENAMES
                    .iter()
                    .find(|(from, _)| name.eq_ignore_ascii_case(from))
                {
                    out.push_str(&format!("{{{to}}}"));
                    if !renamed
                        .iter()
                        .any(|(was, _)| was.eq_ignore_ascii_case(&name))
                    {
                        renamed.push((name, (*to).to_string()));
                    }
                    i += end + 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    (out, renamed)
}

fn change(field: &str, words: impl Into<String>) -> Change {
    Change {
        field: field.to_string(),
        words: words.into(),
    }
}

/// The rule's text, and what changed about it.
fn prepare_text(message: &str) -> (String, Vec<Change>) {
    let mut changes = Vec::new();
    let mut text = message.trim().to_string();
    let lower = text.to_ascii_lowercase();
    if lower == "/me" || lower.starts_with("/me ") {
        text = text[3..].trim_start().to_string();
        changes.push(change(
            "actions[0].texts",
            "A rule can't start a message with /. The /me is dropped and the rest posts as plain text.",
        ));
    } else if text.starts_with('/') {
        changes.push(change(
            "actions[0].texts",
            "A rule can't post a line that starts with /. Change the message to turn the rule on.",
        ));
    }
    let (rewritten, renamed) = rewrite_variables(&text);
    if !renamed.is_empty() {
        let list: Vec<String> = renamed
            .iter()
            .map(|(was, now)| format!("{{{was}}} is now {{{now}}}"))
            .collect();
        changes.push(change(
            "actions[0].texts",
            format!("Renamed for rules: {}.", list.join(", ")),
        ));
    }
    if rewritten.contains("{message}") {
        changes.push(change(
            "actions[0].texts",
            "A rule may repeat what a chatter wrote only in your own channel, for moderators and you.",
        ));
    }
    (rewritten, changes)
}

struct Built {
    rule: Value,
    changes: Vec<Change>,
    kind: ReminderKind,
    /// The interval or delay in seconds; 0 for the others.
    seconds: u64,
}

/// The rule, field by field. The wire shapes of P2's triggers are the plan's (§ 11.1): a Timer is
/// `{ every_seconds, min_messages }`, a Clock `{ at, days }` (empty `days` = every day), an Uptime
/// `{ seconds, late_ok }`, a Delay `{ seconds }`.
fn build_rule(r: &Reminder, env: &ConvertEnv) -> Result<Built, String> {
    let kind = ReminderKind::from_word(r.trigger.as_str())
        .ok_or("This reminder's trigger isn't one a rule can use")?;
    let mut changes: Vec<Change> = Vec::new();
    let (text, text_changes) = prepare_text(&r.message);
    changes.extend(text_changes);

    let repeats = super::clamp_repeat(r.repeat_count);
    if repeats > 1 {
        changes.push(change(
            "repeat_count",
            format!("A rule posts once each time. This reminder posts {repeats} copies."),
        ));
    }

    // The trigger.
    let (when, seconds) = match kind {
        ReminderKind::Interval => {
            let secs = r.interval();
            (
                json!({ "kind": "timer", "every_seconds": secs, "min_messages": 0 }),
                secs,
            )
        }
        ReminderKind::Delay => {
            let secs = r.delay();
            (json!({ "kind": "delay", "seconds": secs }), secs)
        }
        ReminderKind::Clock => (
            json!({ "kind": "clock", "at": r.clock_time.as_deref().unwrap_or_default().trim(), "days": [] }),
            0,
        ),
        ReminderKind::Uptime => {
            // Converted rules keep the reminder's seconds and its "any time after, once per
            // broadcast" firing: `late_ok` lifts the 2-minute window for this rule.
            let secs = r.uptime();
            if secs > 172_800 {
                changes.push(change(
                    "when.seconds",
                    format!(
                        "An uptime of {} is more than the 48 hours (172,800 seconds) a rule can wait. Pick a shorter one to turn the rule on.",
                        long_duration(secs)
                    ),
                ));
            }
            (
                json!({ "kind": "uptime", "seconds": secs, "late_ok": true }),
                secs,
            )
        }
        ReminderKind::Keyword => {
            let mode = match r.keyword_match.as_deref() {
                Some("exact") => "exact",
                // `Phrase` matches whole words on the reminder's own boundary.
                Some("word") => "phrase",
                _ => "contains",
            };
            let pattern = r.keyword.as_deref().unwrap_or_default().trim();
            (
                json!({ "kind": "message", "text": { "mode": mode, "pattern": pattern, "case_sensitive": false } }),
                0,
            )
        }
    };
    if kind == ReminderKind::Interval && !(300..=43_200).contains(&seconds) {
        changes.push(change(
            "when.every_seconds",
            format!(
                "Every {} is outside a rule's range (5 minutes to 12 hours). Pick a time in that range to turn the rule on.",
                long_duration(seconds)
            ),
        ));
    }

    // The chat: `current` is exactly the stream you are watching; a pinned chat is These (Own when it is yours).
    let pinned = r.pinned_channel();
    let owner_login = env.owner.login.to_lowercase();
    let (channels, other_chat) = match &pinned {
        None => (json!({ "kind": "watching" }), None),
        Some(login) if login.is_empty() => return Err("Pick the reminder's channel first".into()),
        Some(login) if *login == owner_login => (json!({ "kind": "own" }), None),
        Some(login) => {
            let room = env
                .room_id
                .clone()
                .or_else(|| r.channel_id.clone())
                .unwrap_or_default();
            let list = json!([{ "id": "", "key": format!("twitch:{login}"), "room_id": room, "name": login }]);
            (
                json!({ "kind": "these", "list": list }),
                Some(login.clone()),
            )
        }
    };
    if let Some(login) = &other_chat {
        let words = if env.other_channels_on {
            format!("#{login} isn't your chat. A rule posts there at most once per gap (Other channels, above).")
        } else {
            format!("#{login} isn't your chat. A rule posts there only while Other channels is on in Automations, and then at most once per gap.")
        };
        changes.push(change("only_if.channels", words));
    }

    // Who may trigger a keyword rule, and the cooldown.
    let spec = spec_of(r);
    let (roles, lead_excluded) = if kind == ReminderKind::Keyword {
        match r.keyword_from.as_deref() {
            // The reminder's own reading: the broadcaster and moderators, no lead moderator.
            Some("mods") => (json!(["moderator", "broadcaster"]), true),
            Some("broadcaster") => (json!(["broadcaster"]), false),
            _ => (json!([]), false),
        }
    } else {
        (json!([]), false)
    };
    if kind == ReminderKind::Keyword {
        changes.push(change(
            "when",
            "Messages from any of your signed-in accounts no longer trigger it. A reminder only ignored your main account's.",
        ));
        if spec.cooldown_s == 0 {
            changes.push(change(
                "limits.rule_cooldown_seconds",
                "No cooldown now meets a rule's rate hold (6 in 60 seconds) and at most 120 an hour.",
            ));
        }
    }

    changes.push(change(
        "send_as",
        format!(
            "Posts as @{} from now on. A reminder posts as whichever account is your main one when it fires.",
            env.owner.login
        ),
    ));
    changes.push(change(
        "enabled",
        "Rules follow the Off, Test and On status at the top of this page and pause while the phone is in the background. Reminders follow their own switches.",
    ));

    let owner = serde_json::to_value(&env.owner).unwrap_or(Value::Null);
    let only_if = json!({
        "channels": channels,
        "roles": roles,
        "skip_users": [],
        "only_users": [],
        // Reminders are not live-gated.
        "only_live": false,
        "only_away": false,
        // The reminder answers Shared Chat partner rows.
        "shared_chat": kind == ReminderKind::Keyword,
        "hours": null,
        // Posts only where a reminder would; reconcile never joins a chat for it.
        "only_while_open": true,
        "lead_moderator_excluded": lead_excluded,
        // An uptime reminder pinned to a chat also requires that chat to be the one being watched.
        "only_while_watching": kind == ReminderKind::Uptime && pinned.is_some(),
    });
    let base = {
        let label = r.label.as_deref().unwrap_or_default().trim();
        if label.is_empty() {
            r.message.trim()
        } else {
            label
        }
    };
    let name: String = format!("Reminder: {base}").chars().take(60).collect();
    let rule = json!({
        "id": "",
        "name": name.trim(),
        "enabled": true,
        "live": false,
        "owner": owner,
        "send_as": owner,
        "when": when,
        "only_if": only_if,
        "actions": [{ "kind": "send", "id": "", "to": null, "texts": [{ "id": "", "text": text }], "pick": "random" }],
        "limits": {
            // The keyword cooldown (whole minutes) is the rule cooldown; the reminder had no other.
            "rule_cooldown_seconds": if kind == ReminderKind::Keyword { spec.cooldown_s } else { 0 },
            "user_cooldown_seconds": 0,
            "max_per_hour": 120,
            "max_wait_seconds": 10
        },
        "vary": false,
        "origin": format!("reminder:{}", r.id),
        "created_ms": 0,
        "updated_ms": 0
    });
    Ok(Built {
        rule,
        changes,
        kind,
        seconds,
    })
}

/// "Make it a rule" for the reminder `id`, inside one settings write. Refuses ("Already a rule")
/// a reminder that has a rule with `origin: "reminder:<id>"` or whose id is listed in
/// `converted_reminders`, so converting twice never makes two rules. On success: the rule is saved
/// in Test (with its field errors shown and Live blocked until they are fixed), `claim_as` is set,
/// the id joins `converted_reminders` (the reminder is off while it is listed, whatever its
/// `enabled`), and the reminder's own `enabled` is written false so a build from before P5 never
/// runs both after a downgrade. A reminder that cannot convert changes nothing and keeps running.
pub fn convert_in(
    settings: &mut Settings,
    id: &str,
    env: &ConvertEnv,
    context: &dyn Fn(&Automations, &str) -> ValidateCtx,
) -> Result<Conversion, String> {
    let mut list = read_list(settings);
    let index = list
        .iter()
        .position(|r| id_of(r) == id)
        .ok_or("That reminder is gone")?;
    let reminder: Reminder = serde_json::from_value(list[index].clone())
        .map_err(|_| "This reminder can't be read".to_string())?;
    let origin = format!("reminder:{id}");
    let group = &settings.automations;
    let rule_exists = group
        .rules
        .iter()
        .any(|raw| raw.get("origin").and_then(Value::as_str) == Some(origin.as_str()));
    if rule_exists || group.converted_reminders.iter().any(|c| c == id) {
        return Err("Already a rule".into());
    }

    let built = build_rule(&reminder, env)?;
    let mut rule = Rule::from_value(&built.rule)
        .map_err(|_| "This reminder can't become a rule".to_string())?;
    let spec = spec_of(&reminder);
    let (trigger_kind, scope_kind, scope_key, send_to) = ClaimAs::shape_of(&rule);
    rule.claim_as = Some(ClaimAs {
        account_id: env.owner.id.clone(),
        kind: built.kind.word().to_string(),
        family_hash: spec.hash,
        seconds: built.seconds,
        cooldown_s: spec.cooldown_s,
        trigger_kind,
        scope_kind,
        scope_key,
        send_to,
    });
    rule.created_ms = env.now_ms;
    rule.updated_ms = env.now_ms;
    rule.live = false;
    rule.enabled = true;
    fill_ids(&mut rule);
    let errors = validate(&mut rule, &context(group, ""));
    if let Some(full) = errors.iter().find(|e| e.path == "rules") {
        return Err(full.message.clone());
    }
    let rule_id = rule.id.clone();

    let group = &mut settings.automations;
    group.rules.push(rule.to_value());
    group.converted_reminders.push(id.to_string());
    if let Some(map) = list[index].as_object_mut() {
        map.insert("enabled".to_string(), Value::Bool(false));
    }
    write_list(settings, list);
    Ok(Conversion {
        rule_id,
        changes: built.changes,
        errors,
    })
}

/// "Turn back on": in one settings write the id leaves `converted_reminders`, the reminder's
/// `enabled` is true again and its rule (when it still exists) is disabled. The only thing that
/// ever removes an id. Answers whether anything changed (a reminder not listed changes nothing).
pub fn restore_in(settings: &mut Settings, id: &str) -> Result<bool, String> {
    let mut list = read_list(settings);
    let index = list
        .iter()
        .position(|r| id_of(r) == id)
        .ok_or("That reminder is gone")?;
    if !settings
        .automations
        .converted_reminders
        .iter()
        .any(|c| c == id)
    {
        return Ok(false);
    }
    let group = &mut settings.automations;
    group.converted_reminders.retain(|c| c != id);
    let origin = format!("reminder:{id}");
    for raw in group.rules.iter_mut() {
        if raw.get("origin").and_then(Value::as_str) == Some(origin.as_str()) {
            if let Some(map) = raw.as_object_mut() {
                map.insert("enabled".to_string(), Value::Bool(false));
            }
        }
    }
    if let Some(map) = list[index].as_object_mut() {
        map.insert("enabled".to_string(), Value::Bool(true));
    }
    write_list(settings, list);
    Ok(true)
}

/// The login of a pinned reminder that has no room id of its own, which the command looks up
/// before it takes the settings lock.
pub fn room_lookup_needed(settings: &Settings, id: &str) -> Option<String> {
    let raw = read_list(settings).into_iter().find(|r| id_of(r) == id)?;
    let r: Reminder = serde_json::from_value(raw).ok()?;
    let login = r.pinned_channel().filter(|l| !l.is_empty())?;
    let known = r
        .channel_id
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    (!known).then_some(login)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::{ChannelScope, SenderRole, Trigger};

    fn settings_with(reminders: Vec<Value>) -> Settings {
        let mut s = Settings::default();
        write_list(&mut s, reminders);
        s
    }

    fn env() -> ConvertEnv {
        ConvertEnv {
            owner: AccountRef {
                provider: "twitch".into(),
                id: "100".into(),
                login: "me".into(),
                name: "me".into(),
            },
            now_ms: 1_000,
            room_id: None,
            other_channels_on: false,
        }
    }

    fn no_context(_: &Automations, _: &str) -> ValidateCtx {
        ValidateCtx::default()
    }

    fn base(trigger: &str, extra: Value) -> Value {
        let mut v = json!({ "id": "r1", "enabled": true, "trigger": trigger, "message": "Plug the Discord", "channel_scope": "current" });
        if let (Some(map), Some(more)) = (v.as_object_mut(), extra.as_object()) {
            map.extend(more.clone());
        }
        v
    }

    /// Convert `reminder` in fresh settings; the settings and the answer.
    fn convert(reminder: Value) -> (Settings, Conversion) {
        let mut s = settings_with(vec![reminder]);
        let answer = convert_in(&mut s, "r1", &env(), &no_context).expect("converts");
        (s, answer)
    }

    fn the_rule(s: &Settings) -> Rule {
        assert_eq!(s.automations.rules.len(), 1);
        Rule::from_value(&s.automations.rules[0]).expect("the stored rule reads")
    }

    fn has(c: &Conversion, field: &str) -> bool {
        c.changes.iter().any(|ch| ch.field == field)
    }

    // ---- the wire shapes ----

    #[test]
    fn the_op_shapes_round_trip_on_the_wire() {
        let op = |v: Value| serde_json::from_value::<ReminderOp>(v).expect("reads");
        assert_eq!(
            op(json!({ "op": "remove", "id": "a" })),
            ReminderOp::Remove { id: "a".into() }
        );
        assert_eq!(
            op(json!({ "op": "set_enabled", "id": "a", "enabled": true })),
            ReminderOp::SetEnabled {
                id: "a".into(),
                enabled: true
            }
        );
        assert_eq!(
            op(json!({ "op": "upsert", "reminder": { "id": "a" } })),
            ReminderOp::Upsert {
                reminder: json!({ "id": "a" })
            }
        );
        // The internal tag is the only form: a bare string or an externally tagged object is refused.
        assert!(serde_json::from_value::<ReminderOp>(json!("remove")).is_err());
        assert!(serde_json::from_value::<ReminderOp>(json!({ "remove": { "id": "a" } })).is_err());
    }

    #[test]
    fn the_answers_carry_exactly_the_planned_fields() {
        let (s, answer) = convert(base("interval", json!({ "interval_seconds": 900 })));
        let v = serde_json::to_value(&answer).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["changes", "errors", "rule_id"]);
        assert!(v["changes"][0].get("field").is_some() && v["changes"][0].get("words").is_some());
        let list = serde_json::to_value(summaries(&s)).unwrap();
        let mut keys: Vec<&str> = list[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "chat",
                "converted",
                "enabled",
                "id",
                "last_fired_ms",
                "name",
                "schedule"
            ]
        );
    }

    #[test]
    fn the_three_commands_are_registered_in_the_shared_handler_and_allowed() {
        let lib = include_str!("../../lib.rs");
        let toml = include_str!("../../../permissions/app-commands.toml");
        for name in [
            "automations_set_reminders",
            "automations_convert_reminder",
            "automations_restore_reminder",
        ] {
            assert!(
                lib.contains(&format!("commands::account_automations::{name},")),
                "{name} in generate_handler!"
            );
            assert!(
                toml.contains(&format!("\"{name}\",")),
                "{name} in app-commands.toml"
            );
        }
    }

    // ---- the Reminders writer and the list ----

    #[test]
    fn the_writer_upserts_by_merge_removes_and_switches() {
        let mut s = settings_with(vec![base(
            "interval",
            json!({ "interval_seconds": 600, "label": "Old name", "future": 1 }),
        )]);
        // A merge: the null removes `label`, the unknown key stays, the new field lands.
        let changed = apply_op(
            &mut s,
            ReminderOp::Upsert {
                reminder: json!({ "id": "r1", "label": null, "message": "New text" }),
            },
        )
        .unwrap();
        assert!(changed);
        let list = read_list(&s);
        assert_eq!(
            (
                list[0]["message"].clone(),
                list[0].get("label").cloned(),
                list[0]["future"].clone()
            ),
            (json!("New text"), None, json!(1))
        );
        // The same upsert again changes nothing.
        assert!(!apply_op(
            &mut s,
            ReminderOp::Upsert {
                reminder: json!({ "id": "r1", "message": "New text" })
            }
        )
        .unwrap());
        // A new id appends (nulls dropped); no id is refused.
        assert!(apply_op(
            &mut s,
            ReminderOp::Upsert {
                reminder: json!({ "id": "r2", "trigger": "delay", "x": null })
            }
        )
        .unwrap());
        assert_eq!(read_list(&s).len(), 2);
        assert!(read_list(&s)[1].get("x").is_none());
        assert!(apply_op(
            &mut s,
            ReminderOp::Upsert {
                reminder: json!({ "message": "no id" })
            }
        )
        .is_err());
        // SetEnabled and Remove.
        assert!(apply_op(
            &mut s,
            ReminderOp::SetEnabled {
                id: "r1".into(),
                enabled: false
            }
        )
        .unwrap());
        assert!(!apply_op(
            &mut s,
            ReminderOp::SetEnabled {
                id: "r1".into(),
                enabled: false
            }
        )
        .unwrap());
        assert!(apply_op(
            &mut s,
            ReminderOp::SetEnabled {
                id: "gone".into(),
                enabled: true
            }
        )
        .is_err());
        assert!(apply_op(&mut s, ReminderOp::Remove { id: "r2".into() }).unwrap());
        assert_eq!(
            read_list(&s).iter().map(id_of).collect::<Vec<_>>(),
            vec!["r1"]
        );
    }

    #[test]
    fn move_puts_a_reminder_at_an_index_and_clamps() {
        let ids = |s: &Settings| {
            read_list(s)
                .iter()
                .map(id_of)
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let mut s = settings_with(vec![
            json!({ "id": "a" }),
            json!({ "id": "b" }),
            json!({ "id": "c" }),
        ]);
        assert!(
            apply_op(
                &mut s,
                ReminderOp::Move {
                    id: "a".into(),
                    to: 2
                }
            )
            .unwrap(),
            "first to last"
        );
        assert_eq!(ids(&s), vec!["b", "c", "a"]);
        assert!(
            apply_op(
                &mut s,
                ReminderOp::Move {
                    id: "a".into(),
                    to: 0
                }
            )
            .unwrap(),
            "last to first"
        );
        assert_eq!(ids(&s), vec!["a", "b", "c"]);
        assert!(
            apply_op(
                &mut s,
                ReminderOp::Move {
                    id: "a".into(),
                    to: 99
                }
            )
            .unwrap(),
            "out of range clamps to the end"
        );
        assert_eq!(ids(&s), vec!["b", "c", "a"]);
        assert!(
            !apply_op(
                &mut s,
                ReminderOp::Move {
                    id: "a".into(),
                    to: 99
                }
            )
            .unwrap(),
            "already there changes nothing"
        );
        assert!(
            apply_op(
                &mut s,
                ReminderOp::Move {
                    id: "gone".into(),
                    to: 0
                }
            )
            .is_err(),
            "an unknown id is refused"
        );
        assert_eq!(ids(&s), vec!["b", "c", "a"]);
        assert_eq!(
            serde_json::from_value::<ReminderOp>(json!({ "op": "move", "id": "a", "to": 1 }))
                .unwrap(),
            ReminderOp::Move {
                id: "a".into(),
                to: 1
            }
        );
    }

    #[test]
    fn the_list_says_what_each_reminder_does_in_words() {
        let s = settings_with(vec![
            base(
                "interval",
                json!({ "id": "a", "interval_seconds": 5400, "label": "Discord" }),
            ),
            base(
                "keyword",
                json!({ "id": "b", "keyword": "hello", "channel_scope": "specific", "channel_login": "Streamer" }),
            ),
            base(
                "clock",
                json!({ "id": "c", "clock_time": "21:00", "enabled": false }),
            ),
            base("delay", json!({ "id": "d", "delay_seconds": 1 })),
        ]);
        let list = summaries(&s);
        assert_eq!(
            list.iter()
                .map(|r| (
                    r.id.as_str(),
                    r.name.as_str(),
                    r.schedule.as_str(),
                    r.chat.as_str(),
                    r.enabled
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "a",
                    "Discord",
                    "Every 1 hour 30 minutes",
                    "Current channel",
                    true
                ),
                (
                    "b",
                    "Plug the Discord",
                    "When someone says \"hello\"",
                    "#streamer",
                    true
                ),
                (
                    "c",
                    "Plug the Discord",
                    "Daily at 21:00",
                    "Current channel",
                    false
                ),
                (
                    "d",
                    "Plug the Discord",
                    "1 second after you join",
                    "Current channel",
                    true
                ),
            ]
        );
        assert_eq!(long_duration(45), "45 seconds");
        assert_eq!(long_duration(600), "10 minutes");
    }

    // ---- Make it a rule: one test per field ----

    #[test]
    fn each_trigger_becomes_its_rule_trigger() {
        let (s, _) = convert(base("interval", json!({ "interval_seconds": 900 })));
        let w = s.automations.rules[0]["when"].clone();
        assert_eq!(
            (
                w["kind"].clone(),
                w["every_seconds"].clone(),
                w["min_messages"].clone()
            ),
            (json!("timer"), json!(900), json!(0))
        );
        let (s, _) = convert(base("clock", json!({ "clock_time": "21:30" })));
        assert_eq!(s.automations.rules[0]["when"]["kind"], "clock");
        assert_eq!(s.automations.rules[0]["when"]["at"], "21:30");
        let (s, _) = convert(base("delay", json!({ "delay_seconds": 120 })));
        assert_eq!(
            (
                s.automations.rules[0]["when"]["kind"].clone(),
                s.automations.rules[0]["when"]["seconds"].clone()
            ),
            (json!("delay"), json!(120))
        );
        // Uptime 5,430 s keeps its seconds and fires any time after, once per broadcast.
        let (s, _) = convert(base("uptime", json!({ "uptime_seconds": 5430 })));
        let w = s.automations.rules[0]["when"].clone();
        assert_eq!(
            (
                w["kind"].clone(),
                w["seconds"].clone(),
                w["late_ok"].clone()
            ),
            (json!("uptime"), json!(5430), json!(true))
        );
    }

    #[test]
    fn the_chat_maps_current_specific_and_your_own() {
        let (s, _) = convert(base("interval", json!({ "interval_seconds": 900 })));
        assert_eq!(the_rule(&s).only_if.channels, ChannelScope::Watching {});
        let (s, c) = convert(base(
            "interval",
            json!({ "interval_seconds": 900, "channel_scope": "specific", "channel_login": "Streamer", "channel_id": "9" }),
        ));
        match the_rule(&s).only_if.channels {
            ChannelScope::These { list } => {
                assert_eq!(
                    (list[0].key.as_str(), list[0].room_id.as_str()),
                    ("twitch:streamer", "9")
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(
            has(&c, "only_if.channels"),
            "another streamer's chat is listed (Other channels, the gap)"
        );
        let (s, c) = convert(base(
            "interval",
            json!({ "interval_seconds": 900, "channel_scope": "specific", "channel_login": "ME" }),
        ));
        assert_eq!(the_rule(&s).only_if.channels, ChannelScope::Own {});
        assert!(!has(&c, "only_if.channels"));
    }

    #[test]
    fn a_keyword_reads_as_the_reminder_reads_it() {
        let words = |mode: &str| {
            let (s, _) = convert(base(
                "keyword",
                json!({ "keyword": " Hello ", "keyword_match": mode }),
            ));
            let t = s.automations.rules[0]["when"]["text"].clone();
            (
                t["mode"].clone(),
                t["pattern"].clone(),
                t["case_sensitive"].clone(),
            )
        };
        assert_eq!(
            words("contains"),
            (json!("contains"), json!("Hello"), json!(false))
        );
        assert_eq!(
            words("exact"),
            (json!("exact"), json!("Hello"), json!(false))
        );
        assert_eq!(
            words("word"),
            (json!("phrase"), json!("Hello"), json!(false))
        );
        let (s, _) = convert(base("keyword", json!({ "keyword": "x" })));
        assert_eq!(
            the_rule(&s).only_if.shared_chat,
            true,
            "the reminder answers Shared Chat partner rows"
        );
    }

    #[test]
    fn keyword_from_maps_to_roles_without_lead_moderators() {
        let roles = |from: &str| {
            let (s, _) = convert(base(
                "keyword",
                json!({ "keyword": "x", "keyword_from": from }),
            ));
            let r = the_rule(&s);
            (r.only_if.roles, r.only_if.lead_moderator_excluded)
        };
        assert_eq!(roles("anyone"), (vec![], false));
        assert_eq!(
            roles("mods"),
            (vec![SenderRole::Moderator, SenderRole::Broadcaster], true)
        );
        assert_eq!(roles("broadcaster"), (vec![SenderRole::Broadcaster], false));
    }

    #[test]
    fn rule_defaults_are_set_to_match_the_reminder() {
        let (s, _) = convert(base(
            "keyword",
            json!({ "keyword": "x", "keyword_cooldown_minutes": 3 }),
        ));
        let r = the_rule(&s);
        assert_eq!(
            (
                r.limits.rule_cooldown_seconds,
                r.limits.user_cooldown_seconds,
                r.limits.max_per_hour
            ),
            (180, 0, 120),
            "a 3 minute cooldown is 180 seconds"
        );
        assert!(
            !r.only_if.only_live && r.only_if.only_while_open,
            "reminders are not live-gated and post only in chats a window holds"
        );
        assert_eq!(r.origin.as_deref(), Some("reminder:r1"));
        assert!(!r.live && r.enabled, "saved in Test");
        // Only a pinned uptime reminder also needs its chat to be the one watched.
        let (s, _) = convert(base(
            "uptime",
            json!({ "uptime_seconds": 60, "channel_scope": "specific", "channel_login": "streamer" }),
        ));
        assert!(the_rule(&s).only_if.only_while_watching);
        let (s, _) = convert(base("uptime", json!({ "uptime_seconds": 60 })));
        assert!(!the_rule(&s).only_if.only_while_watching);
    }

    #[test]
    fn what_changes_lists_every_field_whose_behavior_differs() {
        let (_, c) = convert(base(
            "interval",
            json!({ "interval_seconds": 900, "repeat_count": 3 }),
        ));
        assert!(has(&c, "repeat_count") && has(&c, "send_as") && has(&c, "enabled"));
        assert!(c
            .changes
            .iter()
            .any(|ch| ch.field == "send_as" && ch.words.contains("@me")));
        // Intervals outside 5 minutes to 12 hours are listed, and shown as field errors with Live blocked.
        for secs in [60, 50_000] {
            let (_, c) = convert(base("interval", json!({ "interval_seconds": secs })));
            assert!(has(&c, "when.every_seconds"), "{secs}");
            assert!(
                !c.errors.is_empty(),
                "{secs}: a field error, so Live is blocked until it is fixed"
            );
        }
        // An uptime over 48 h (172,800 s) fails P2's bound: listed, and a field error.
        let (_, long) = convert(base("uptime", json!({ "uptime_seconds": 172_801 })));
        assert!(has(&long, "when.seconds") && !long.errors.is_empty());
        let (_, edge) = convert(base("uptime", json!({ "uptime_seconds": 172_800 })));
        assert!(!has(&edge, "when.seconds") && edge.errors.is_empty());
        let (_, ok) = convert(base("interval", json!({ "interval_seconds": 900 })));
        assert!(!has(&ok, "when.every_seconds") && ok.errors.is_empty());
        // Alts and a 0-minute cooldown, for a keyword.
        let (_, c) = convert(base(
            "keyword",
            json!({ "keyword": "x", "keyword_cooldown_minutes": 0 }),
        ));
        assert!(has(&c, "when") && has(&c, "limits.rule_cooldown_seconds"));
        let (_, c) = convert(base("keyword", json!({ "keyword": "x" })));
        assert!(has(&c, "when") && !has(&c, "limits.rule_cooldown_seconds"));
    }

    #[test]
    fn text_is_rewritten_for_rules_and_the_changes_say_so() {
        let (s, c) = convert(base(
            "interval",
            json!({ "interval_seconds": 900, "message": "/me hi {user}, {Stream.Title} for {stream.uptime}" }),
        ));
        let text = s.automations.rules[0]["actions"][0]["texts"][0]["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(text, "hi {me}, {title} for {uptime}");
        assert!(
            c.changes
                .iter()
                .filter(|ch| ch.field == "actions[0].texts")
                .count()
                >= 2,
            "the /me and the renames are listed"
        );
        // Another / line is kept and refused by rule cleaning (a field error).
        let (_, c) = convert(base(
            "interval",
            json!({ "interval_seconds": 900, "message": "/clear" }),
        ));
        assert!(c
            .changes
            .iter()
            .any(|ch| ch.words.contains("starts with /")));
        assert!(!c.errors.is_empty());
        assert_eq!(
            rewrite_variables("{{user}} {USER} {stream.GAME} {stream.uptime}").0,
            "{{user}} {me} {game} {uptime}"
        );
        assert_eq!(rewrite_variables("a {unknown} b").0, "a {unknown} b");
    }

    #[test]
    fn converting_twice_is_refused_and_a_reminder_that_cannot_convert_keeps_running() {
        let mut s = settings_with(vec![base("interval", json!({ "interval_seconds": 900 }))]);
        convert_in(&mut s, "r1", &env(), &no_context).unwrap();
        let again = convert_in(&mut s, "r1", &env(), &no_context);
        assert_eq!(again.unwrap_err(), "Already a rule");
        assert_eq!(s.automations.rules.len(), 1, "one rule");
        // A rule with this origin and no listed id also counts.
        let mut t = settings_with(vec![base("interval", json!({ "interval_seconds": 900 }))]);
        t.automations
            .rules
            .push(json!({ "id": "r-x", "origin": "reminder:r1" }));
        assert_eq!(
            convert_in(&mut t, "r1", &env(), &no_context).unwrap_err(),
            "Already a rule"
        );
        // An unknown trigger cannot convert: nothing changes, the reminder still runs.
        let mut u = settings_with(vec![base("telepathy", json!({}))]);
        let before = u.extra.clone();
        assert!(convert_in(&mut u, "r1", &env(), &no_context).is_err());
        assert_eq!(
            (
                u.extra == before,
                u.automations.rules.len(),
                u.automations.converted_reminders.len()
            ),
            (true, 0, 0)
        );
        assert!(convert_in(&mut u, "missing", &env(), &no_context).is_err());
    }

    // ---- stale writers, restore, claim_as ----

    #[test]
    fn a_converted_reminder_stays_off_through_a_stale_write_and_a_rule_delete() {
        let (mut s, answer) = convert(base("interval", json!({ "interval_seconds": 900 })));
        assert_eq!(s.automations.converted_reminders, vec!["r1".to_string()]);
        assert_eq!(
            read_list(&s)[0]["enabled"],
            json!(false),
            "the convert writes the reminder's own enabled: false too"
        );
        // A stale Reminders window writes the whole list back with enabled: true, then the rule is deleted.
        write_list(
            &mut s,
            vec![base(
                "interval",
                json!({ "interval_seconds": 900, "enabled": true }),
            )],
        );
        s.automations
            .rules
            .retain(|r| r["id"] != json!(answer.rule_id));
        let (keywords, timed) = super::super::compile_lists(&s);
        assert!(
            keywords.is_empty() && timed.is_empty(),
            "nothing fires: the id is read from settings, never from the rules"
        );
        assert!(super::super::find_active(&s, "r1").is_none());
        // The same for a keyword reminder (a convert followed by an immediate tick before the engine compiles).
        let (s, _) = convert(base("keyword", json!({ "keyword": "x" })));
        assert!(super::super::compile_lists(&s).0.is_empty());
    }

    #[test]
    fn restore_is_the_only_way_back_and_it_changes_both_groups() {
        let (mut s, answer) = convert(base("interval", json!({ "interval_seconds": 900 })));
        // The writer cannot take the id off the list.
        apply_op(
            &mut s,
            ReminderOp::SetEnabled {
                id: "r1".into(),
                enabled: true,
            },
        )
        .unwrap();
        apply_op(
            &mut s,
            ReminderOp::Upsert {
                reminder: json!({ "id": "r1", "enabled": true }),
            },
        )
        .unwrap();
        assert_eq!(s.automations.converted_reminders, vec!["r1".to_string()]);
        assert!(super::super::find_active(&s, "r1").is_none());
        assert!(summaries(&s)[0].converted && !summaries(&s)[0].enabled);
        // Restore: the id leaves, the reminder is on, the rule is off.
        assert!(restore_in(&mut s, "r1").unwrap());
        assert!(s.automations.converted_reminders.is_empty());
        assert_eq!(read_list(&s)[0]["enabled"], json!(true));
        let rule = s
            .automations
            .rules
            .iter()
            .find(|r| r["id"] == json!(answer.rule_id))
            .unwrap();
        assert_eq!(rule["enabled"], json!(false));
        assert!(super::super::find_active(&s, "r1").is_some());
        // Idempotent; a deleted rule is fine; a missing reminder is an error.
        assert!(!restore_in(&mut s, "r1").unwrap());
        let (mut t, answer) = convert(base("interval", json!({ "interval_seconds": 900 })));
        t.automations
            .rules
            .retain(|r| r["id"] != json!(answer.rule_id));
        assert!(restore_in(&mut t, "r1").unwrap());
        assert!(restore_in(&mut t, "nope").is_err());
    }

    #[test]
    fn claim_as_is_built_from_the_reminder_and_applies_only_while_the_shape_holds() {
        let r: Reminder = serde_json::from_value(base(
            "keyword",
            json!({ "keyword": "x", "keyword_cooldown_minutes": 3 }),
        ))
        .unwrap();
        let (s, _) = convert(base(
            "keyword",
            json!({ "keyword": "x", "keyword_cooldown_minutes": 3 }),
        ));
        let mut rule = the_rule(&s);
        let claim = rule
            .claim_as
            .clone()
            .expect("claim_as is set by the convert");
        assert_eq!(
            (
                claim.kind.as_str(),
                claim.family_hash.as_str(),
                claim.cooldown_s
            ),
            ("keyword", spec_of(&r).hash.as_str(), 180)
        );
        assert_eq!(
            (
                claim.account_id.as_str(),
                claim.scope_kind.as_str(),
                claim.send_to.clone()
            ),
            ("100", "watching", None)
        );
        assert!(claim.applies_to(&rule));
        // Text edits keep it; an owner, scope, trigger or Send target change clears it.
        if let Some(crate::models::automations::Action::Send { texts, .. }) =
            rule.actions.first_mut()
        {
            texts[0].text = "Different words".into();
        }
        assert!(claim.applies_to(&rule));
        let mut other = rule.clone();
        other.owner.id = "200".into();
        assert!(
            !claim.applies_to(&other),
            "the claim account no longer matches"
        );
        let mut scope = rule.clone();
        scope.only_if.channels = ChannelScope::Own {};
        assert!(!claim.applies_to(&scope));
        let mut trigger = rule.clone();
        trigger.when = Trigger::FirstMessage {};
        assert!(!claim.applies_to(&trigger));
        // The pure save rule: `save_rule_in` keeps `claim_as` through `.filter(applies_to)`.
        assert!(rule
            .claim_as
            .clone()
            .filter(|c| c.applies_to(&other))
            .is_none());
        assert!(rule
            .claim_as
            .clone()
            .filter(|c| c.applies_to(&rule))
            .is_some());
    }
}
