//! The Automations page's commands, and the one writer of
//! `Settings.automations`.
//!
//! Every change to the group goes through [`edit`]: under the settings lock
//! it clones the group, runs the change, and writes it back only when the
//! change says something changed; then every save's side effects run, the
//! file is written and every window hears `streamnook-settings-updated`
//! with `keys: ["automations"]`. The changes themselves are pure functions
//! over `Automations`, so the read-modify-write rules are tested without a
//! settings file.

use serde::{Deserialize, Serialize};

use crate::models::automations::{
    self, listened_chats, merge_raw, validate, AccountRef, Action, Automations, Away, ChannelScope, EventKind, FieldError,
    Rule, RunState, SenderRole, ValidateCtx, DEFAULT_AWAY_MESSAGE, MAX_AWAY_CHARS,
};
use crate::rt::{AppHandle, Window};
use crate::services::account_automations::compile::{compile, CompileInput, Compiled};
use crate::services::account_automations::{self as engine, activity, badges, facts, matcher, now_ms, template, templates};

/// While the engine has not started yet (the first moments after launch).
const STARTING: &str = "Automations are still starting";

/// Change `Settings.automations` under the settings lock. `change` answers
/// whether it changed anything; nothing is written when it did not.
pub(crate) fn edit(app: &AppHandle, change: impl FnOnce(&mut Automations) -> Result<bool, String>) -> Result<bool, String> {
    use tauri::Manager;
    let state = app.state::<crate::models::settings::AppState>();
    crate::commands::settings::commit_automations(app, &state, change)
}

/// A restored or imported file never brings rules back live, never turns
/// automations on, and never brings back an old away (from another device or
/// an old `/afk`). Reminders are not touched.
pub fn pause_restored(a: &mut Automations) {
    for rule in a.rules.iter_mut() {
        if let Some(object) = rule.as_object_mut() {
            object.insert("live".into(), serde_json::Value::Bool(false));
        }
    }
    if a.run == RunState::On {
        a.run = RunState::Test;
    }
    a.away = None;
}

/// Set the run state; Test and On are also what Resume returns to.
pub fn apply_run(a: &mut Automations, run: RunState) -> bool {
    let changed = a.run != run || (run != RunState::Off && a.last_active != run);
    a.run = run;
    if run != RunState::Off {
        a.last_active = run;
    }
    changed
}

/// Set (Some) or clear (None) this device's away. An empty message uses the
/// last one, else "Back soon".
pub fn apply_away(a: &mut Automations, message: Option<&str>, now_ms: i64) -> bool {
    match message {
        None => a.away.take().is_some(),
        Some(text) => {
            let typed: String = text.trim().chars().take(MAX_AWAY_CHARS).collect();
            let message = if !typed.is_empty() {
                typed
            } else if !a.last_away_message.is_empty() {
                a.last_away_message.clone()
            } else {
                DEFAULT_AWAY_MESSAGE.to_string()
            };
            if !message.is_empty() {
                a.last_away_message = message.clone();
            }
            a.away = Some(Away { message, since_ms: now_ms });
            true
        }
    }
}

fn id_of(raw: &serde_json::Value) -> &str {
    raw.get("id").and_then(|v| v.as_str()).unwrap_or_default()
}

/// What a save answers: the rule as stored (ids filled, forced fields set)
/// and its field errors; `saved` is false when there were errors.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SaveAnswer {
    pub rule: Rule,
    pub errors: Vec<FieldError>,
    pub saved: bool,
}

/// The other enabled rules' chats and how many rules there are besides `id`.
fn context(a: &Automations, id: &str) -> ValidateCtx {
    let mut ctx = ValidateCtx::default();
    for raw in a.rules.iter().filter(|r| id_of(r) != id) {
        ctx.other_rules += 1;
        if let Ok(rule) = Rule::from_value(raw) {
            if rule.enabled {
                ctx.other_chats.extend(listened_chats(&rule));
            }
        }
    }
    ctx
}

/// Insert a new rule (fresh id, Test, on) or merge an edit over the stored
/// raw rule, keeping its switch, its Test/Live and its creation time. An edit
/// made from a copy older than the stored rule is refused.
pub fn save_rule_in(a: &mut Automations, mut rule: Rule, expected_updated_ms: Option<i64>, now: i64) -> Result<SaveAnswer, String> {
    let index = (!rule.id.trim().is_empty()).then(|| a.rules.iter().position(|r| id_of(r) == rule.id)).flatten();
    match index {
        None => {
            rule.id = String::new();
            rule.enabled = true;
            rule.live = false;
            rule.created_ms = now;
            rule.updated_ms = now;
            let errors = validate(&mut rule, &context(a, ""));
            if !errors.is_empty() {
                return Ok(SaveAnswer { rule, errors, saved: false });
            }
            a.rules.push(rule.to_value());
            Ok(SaveAnswer { rule, errors, saved: true })
        }
        Some(i) => {
            let stored = Rule::from_value(&a.rules[i]).map_err(|_| "Made in a newer StreamNook; it can't be changed here".to_string())?;
            if expected_updated_ms != Some(stored.updated_ms) {
                return Err("Changed in another window".into());
            }
            rule.enabled = stored.enabled;
            rule.live = stored.live;
            rule.created_ms = stored.created_ms;
            rule.updated_ms = now.max(stored.updated_ms + 1);
            let id = rule.id.clone();
            let errors = validate(&mut rule, &context(a, &id));
            if !errors.is_empty() {
                return Ok(SaveAnswer { rule, errors, saved: false });
            }
            a.rules[i] = merge_raw(&a.rules[i], &rule.to_value());
            Ok(SaveAnswer { rule, errors, saved: true })
        }
    }
}

pub fn delete_rule_in(a: &mut Automations, id: &str) -> bool {
    let before = a.rules.len();
    a.rules.retain(|r| id_of(r) != id);
    a.rules.len() != before
}

/// Move rule `id` before `before_id` (None: to the end).
pub fn move_rule_in(a: &mut Automations, id: &str, before_id: Option<&str>) -> bool {
    let Some(from) = a.rules.iter().position(|r| id_of(r) == id) else { return false };
    let rule = a.rules.remove(from);
    let to = before_id.and_then(|b| a.rules.iter().position(|r| id_of(r) == b)).unwrap_or(a.rules.len());
    a.rules.insert(to, rule);
    to != from
}

/// The switch and Test/Live. `updated_ms` stays, so an open editor's save
/// is not refused because of a flip.
pub fn set_flags_in(a: &mut Automations, id: &str, enabled: Option<bool>, live: Option<bool>) -> Result<bool, String> {
    let raw = a.rules.iter_mut().find(|r| id_of(r) == id).ok_or("That rule is gone")?;
    if Rule::from_value(raw).is_err() {
        return Err("Made in a newer StreamNook; it can't be changed here".into());
    }
    let object = raw.as_object_mut().ok_or("That rule is damaged")?;
    let mut changed = false;
    for (key, value) in [("enabled", enabled), ("live", live)] {
        if let Some(value) = value {
            changed |= object.get(key) != Some(&serde_json::Value::Bool(value));
            object.insert(key.into(), serde_json::Value::Bool(value));
        }
    }
    Ok(changed)
}

/// Fill what a rule names by id: account logins and names from your
/// accounts, and room ids for named chats (looked up once, at save).
async fn complete(rule: &mut Rule) {
    let Some(engine) = engine::engine() else { return };
    let accounts = engine.outward.own_accounts();
    for account in [&mut rule.owner, &mut rule.send_as] {
        if let Some((_, login)) = accounts.iter().find(|(id, _)| *id == account.id) {
            if account.login != *login {
                account.login = login.clone();
            }
            if account.name.trim().is_empty() {
                account.name = login.clone();
            }
        }
    }
    let mut chats: Vec<&mut automations::ChannelRef> = Vec::new();
    if let ChannelScope::These { list } = &mut rule.only_if.channels {
        chats.extend(list.iter_mut());
    }
    for action in rule.actions.iter_mut() {
        if let Action::Send { to: Some(chat), .. } = action {
            chats.push(chat);
        }
    }
    for chat in chats {
        if !chat.room_id.trim().is_empty() {
            continue;
        }
        let key = if chat.key.trim().is_empty() { chat.name.clone() } else { chat.key.clone() };
        let login = key.split_once(':').map_or(key.as_str(), |(_, l)| l).trim().trim_start_matches('#').to_lowercase();
        if login.is_empty() {
            continue;
        }
        if let Ok(id) = engine.outward.room_id(&login).await {
            chat.room_id = id;
        }
    }
}

#[tauri::command]
pub fn automations_status() -> Result<engine::StatusSnapshot, String> {
    engine::engine().map(|e| e.status()).ok_or_else(|| STARTING.to_string())
}

#[tauri::command]
pub async fn automations_save_rule(app: AppHandle, rule: Rule, expected_updated_ms: Option<i64>) -> Result<SaveAnswer, String> {
    let mut rule = rule;
    complete(&mut rule).await;
    let mut answer = None;
    edit(&app, |a| {
        let saved = save_rule_in(a, rule, expected_updated_ms, now_ms())?;
        let changed = saved.saved;
        answer = Some(saved);
        Ok(changed)
    })?;
    answer.ok_or_else(|| "Nothing was saved".to_string())
}

#[tauri::command]
pub fn automations_delete_rule(app: AppHandle, id: String) -> Result<(), String> {
    edit(&app, |a| Ok(delete_rule_in(a, &id))).map(|_| ())
}

#[tauri::command]
pub fn automations_move_rule(app: AppHandle, id: String, before_id: Option<String>) -> Result<(), String> {
    edit(&app, |a| Ok(move_rule_in(a, &id, before_id.as_deref()))).map(|_| ())
}

#[tauri::command]
pub fn automations_set_rule_flags(app: AppHandle, id: String, enabled: Option<bool>, live: Option<bool>) -> Result<(), String> {
    edit(&app, |a| set_flags_in(a, &id, enabled, live)).map(|_| ())
}

#[tauri::command]
pub fn automations_set_run(app: AppHandle, state: RunState) -> Result<(), String> {
    edit(&app, |a| Ok(apply_run(a, state)))?;
    if let Some(engine) = engine::engine() {
        engine.set_run_now(state);
    }
    Ok(())
}

/// Set (`message` given, possibly empty) or clear (null) this device's away.
#[tauri::command]
pub fn automations_set_away(app: AppHandle, message: Option<String>) -> Result<Option<Away>, String> {
    let mut away = None;
    edit(&app, |a| {
        let changed = apply_away(a, message.as_deref(), now_ms());
        away = a.away.clone();
        Ok(changed)
    })?;
    if let Some(engine) = engine::engine() {
        engine.set_away_now(away.clone());
    }
    Ok(away)
}

/// An event for Try it: the kind and the number each kind carries (the
/// others are ignored).
#[derive(Deserialize, Clone, Copy, Debug)]
pub struct TryEvent {
    pub kind: EventKind,
    #[serde(default)]
    pub months: u32,
    #[serde(default)]
    pub gifts: u32,
    #[serde(default)]
    pub viewers: u32,
    #[serde(default)]
    pub bits: u32,
}

/// What `automations_test` takes (the wire shape every client sends; Try it
/// and the editor's preview). `draft` is an unsaved rule tried instead of the
/// stored ones.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct TryRequest {
    pub text: String,
    /// A Twitch login.
    pub channel: String,
    /// The sender's login.
    pub sender: String,
    pub roles: Vec<SenderRole>,
    pub first_message: bool,
    pub event: Option<TryEvent>,
    /// A recent drop (`automations_badge_samples`); None with a badge rule
    /// means the newest one.
    pub badge_feed_id: Option<String>,
    pub draft: Option<Rule>,
}

/// One action's text, as it would go out.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct TryText {
    pub action_id: String,
    /// `reply`, `send` or `notify`.
    pub kind: &'static str,
    pub text: String,
}

/// One rule's answer for the sample.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TryRow {
    pub rule_id: String,
    pub name: String,
    pub fired: bool,
    /// Each check, in order, in your words.
    pub trace: Vec<String>,
    /// Filled for a rule that fired, and always for a draft.
    pub texts: Vec<TryText>,
    /// A badge rule's filled link for the drop tried.
    pub link: Option<String>,
    /// A draft's field errors; empty for stored rules.
    pub errors: Vec<FieldError>,
}

/// `automations_test`'s answer.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TryAnswer {
    pub rules: Vec<TryRow>,
}

/// The sample as `facts` reads it.
pub fn try_sample(request: &TryRequest) -> facts::TrySample {
    let (event, amount) = match request.event {
        Some(e) => (
            Some(e.kind),
            match e.kind {
                EventKind::Sub | EventKind::Resub => e.months,
                EventKind::Gift | EventKind::GiftBomb => e.gifts,
                EventKind::Raid => e.viewers,
                EventKind::Cheer => e.bits,
                EventKind::Announcement => 0,
            },
        ),
        None => (None, 0),
    };
    facts::TrySample {
        text: request.text.clone(),
        channel: request.channel.clone(),
        user_login: request.sender.clone(),
        roles: request.roles.clone(),
        first_message: request.first_message,
        event,
        amount,
        ..facts::TrySample::default()
    }
}

fn action_text(action: &Action, vars: &template::Vars, vary: bool) -> Option<TryText> {
    let (id, kind, raw) = match action {
        Action::Reply { id, texts, .. } => (id, "reply", texts.first()?.text.clone()),
        Action::Send { id, texts, .. } => (id, "send", texts.first()?.text.clone()),
        Action::Notify { id, title, body, .. } => (id, "notify", if body.trim().is_empty() { title.clone() } else { body.clone() }),
    };
    let filled = template::expand(&raw, vars);
    let text = if kind == "notify" { filled } else { template::clean(&filled, vary).unwrap_or_else(|e| e.words().to_string()) };
    Some(TryText { action_id: id.clone(), kind, text })
}

/// Every compiled rule's row for the sample. A badge rule reads the drop
/// (`seen`), the others the made-up message or event.
pub fn try_rows(
    compiled: &Compiled,
    request: &TryRequest,
    seen: Option<&badges::BadgeSeen>,
    away: Option<&Away>,
    now: i64,
    local: chrono::NaiveDateTime,
    draft_errors: Option<Vec<FieldError>>,
) -> Vec<TryRow> {
    let ctx = matcher::Ctx { away, local: matcher::LocalTime::from_naive(local), trace: true };
    let sample_facts = facts::from_sample(&try_sample(request));
    let sample_verdicts = matcher::try_all(compiled, &sample_facts, &ctx);
    let drop_facts = seen.map(facts::from_badge);
    let drop_verdicts = drop_facts.as_ref().map(|f| matcher::try_all(compiled, f, &ctx));
    let is_draft = draft_errors.is_some();
    compiled
        .rules
        .iter()
        .enumerate()
        .map(|(i, cr)| {
            let badge = cr.rule.is_badge_rule();
            let (facts, verdicts) = match (&drop_facts, &drop_verdicts) {
                (Some(f), Some(v)) if badge => (f, v),
                _ => (&sample_facts, &sample_verdicts),
            };
            let verdict = verdicts.iter().find(|v| v.rule == i);
            let fired = verdict.is_some_and(|v| v.fired);
            let trace = verdict.map(|v| v.trace.iter().map(|t| t.detail.clone()).collect()).unwrap_or_default();
            let mut vars = template::Vars::from_facts(facts);
            vars.me = cr.send_as_name.clone();
            vars.set_away(away, now);
            vars.set_local(local);
            let mut link = None;
            if let (automations::Trigger::BadgeDrop { link: rule_link, .. }, Some(seen)) = (&cr.rule.when, seen) {
                link = badges::link_for(rule_link, &seen.badge);
                vars.badge_link = link.clone().unwrap_or_default();
                vars.set_batch(&[seen]);
            }
            let texts = if fired || is_draft {
                cr.rule.actions.iter().filter_map(|a| action_text(a, &vars, cr.rule.vary)).collect()
            } else {
                Vec::new()
            };
            TryRow {
                rule_id: cr.rule.id.clone(),
                name: cr.rule.name.clone(),
                fired,
                trace,
                texts,
                link,
                errors: draft_errors.clone().unwrap_or_default(),
            }
        })
        .collect()
}

/// Try it: every rule's verdict, trace and texts for a sample; no guard,
/// count, index or claim. With a `draft`, only that rule is tried (compiled
/// as Test, switched on) and its field errors come back with it.
#[tauri::command]
pub fn automations_test(sample: TryRequest) -> Result<TryAnswer, String> {
    let engine = engine::engine().ok_or(STARTING)?;
    let away = engine.away();
    let drops = engine.badge_samples();
    let seen = match sample.badge_feed_id.as_deref() {
        Some(id) => drops.iter().find(|b| b.feed_id == id),
        None => drops.first(),
    };
    let now = now_ms();
    let local = chrono::Local::now().naive_local();
    let (compiled, errors) = match &sample.draft {
        None => (engine.compiled(), None),
        Some(draft) => {
            let (mut group, prefs) = engine.local.settings();
            let mut rule = draft.clone();
            if rule.id.trim().is_empty() {
                rule.id = "draft".into();
            }
            rule.enabled = true;
            let id = rule.id.clone();
            let errors = validate(&mut rule, &context(&group, &id));
            group.rules = vec![rule.to_value()];
            group.run = RunState::Test;
            let own = engine.outward.own_accounts();
            let profile = engine.outward.profile_user_id();
            let compiled = compile(&CompileInput {
                automations: &group,
                prefs: &prefs,
                own_accounts: &own,
                profile_user_id: profile.as_deref(),
            });
            (std::sync::Arc::new(compiled), Some(errors))
        }
    };
    Ok(TryAnswer { rules: try_rows(&compiled, &sample, seen, away.as_ref(), now, local, errors) })
}

/// A recent drop for "Try with".
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct BadgeSample {
    pub feed_id: String,
    pub name: String,
    pub phase: badges::BadgePhase,
}

#[tauri::command]
pub fn automations_badge_samples() -> Vec<BadgeSample> {
    let seen = engine::engine().map(|e| e.badge_samples()).unwrap_or_default();
    seen.into_iter().map(|b| BadgeSample { feed_id: b.feed_id, name: b.badge.name, phase: b.phase }).collect()
}

#[tauri::command]
pub fn automations_templates() -> Vec<templates::TemplateView> {
    templates::templates()
}

/// Add a starter in Test, owned by and sending as the main account.
#[tauri::command]
pub async fn automations_add_template(app: AppHandle, id: String) -> Result<SaveAnswer, String> {
    let engine = engine::engine().ok_or(STARTING)?;
    let main = engine.main_id().ok_or("Sign in to add a rule")?;
    let login = engine
        .outward
        .own_accounts()
        .into_iter()
        .find(|(id, _)| *id == main)
        .map(|(_, login)| login)
        .ok_or("Sign in to add a rule")?;
    let owner = AccountRef { provider: "twitch".into(), id: main, login: login.clone(), name: login };
    let mut rule = templates::template_rule(&id, &owner, now_ms()).ok_or("There is no such template")?;
    complete(&mut rule).await;
    let mut answer = None;
    edit(&app, |a| {
        let saved = save_rule_in(a, rule, None, now_ms())?;
        let changed = saved.saved;
        answer = Some(saved);
        Ok(changed)
    })?;
    answer.ok_or_else(|| "Nothing was saved".to_string())
}

#[tauri::command]
pub fn automations_variables() -> Vec<template::VariableRow> {
    template::variable_table()
}

#[tauri::command]
pub fn automations_activity(filter: Option<activity::Filter>, limit: Option<usize>, before: Option<u64>) -> Vec<activity::Entry> {
    let Some(engine) = engine::engine() else { return Vec::new() };
    engine.activity_list(filter.unwrap_or_default(), limit.unwrap_or(100).min(500), before)
}

#[tauri::command]
pub fn automations_clear_activity() {
    if let Some(engine) = engine::engine() {
        engine.clear_activity();
    }
}

/// The calling window wants Activity lines while its page is open.
#[tauri::command]
pub fn automations_watch_activity(window: Window, watch: bool) {
    if let Some(engine) = engine::engine() {
        engine.watch_activity(window.label(), watch);
    }
}

#[tauri::command]
pub fn automations_set_device_name(name: String) -> Result<String, String> {
    engine::engine().ok_or(STARTING)?.set_device_name(&name)
}

/// The editor's "Check it": the same rules as a badge post's link check.
#[tauri::command]
pub async fn automations_check_link(url: String) -> bool {
    let Some(engine) = engine::engine() else { return false };
    badges::check_url(&url).is_ok() && engine.local.probe_link(url.trim()).await == badges::Probe::Status(200)
}

/// A window heard notification `id`. True: this window plays it.
#[tauri::command]
pub fn automations_notify_claim(id: u64) -> bool {
    engine::engine().is_some_and(|e| e.notifier.claim(id, e.timings.notify_confirm))
}

/// The iPhone shell's picture in picture started (true) or stopped.
#[tauri::command]
pub fn automations_pip(active: bool) {
    engine::on_pip(active);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn owner() -> serde_json::Value {
        json!({ "provider": "twitch", "id": "1", "login": "me", "name": "Me" })
    }

    fn raw_rule(id: &str) -> serde_json::Value {
        json!({
            "id": id, "name": "Thanks", "enabled": true, "live": true, "owner": owner(), "send_as": owner(),
            "when": { "kind": "mention", "replies_too": true },
            "only_if": { "channels": { "kind": "own" }, "roles": [], "skip_users": [], "only_users": [],
                         "only_live": false, "only_away": false, "shared_chat": false,
                         "hours": { "from": "09:00", "to": "17:00", "days": [] } },
            "actions": [
                { "kind": "reply", "id": "a-1", "texts": [{ "id": "l-1", "text": "hi {user}" }, { "id": "l-2", "text": "hey {user}" }],
                  "pick": "random", "thread": false, "future_field": 7 },
                { "kind": "send", "id": "a-2", "to": { "id": "c-1", "key": "twitch:other", "room_id": "9", "name": "other" },
                  "texts": [{ "id": "l-3", "text": "bye" }], "pick": "random" }
            ],
            "limits": { "rule_cooldown_seconds": 30, "user_cooldown_seconds": 300, "max_per_hour": 30, "max_wait_seconds": 10, "later": true },
            "vary": false, "origin": null, "created_ms": 5, "updated_ms": 10
        })
    }

    #[test]
    fn a_try_request_reads_the_clients_shape_and_picks_the_events_number() {
        let request: TryRequest = serde_json::from_value(json!({
            "text": "", "channel": "me", "sender": "viewer", "roles": ["vip"], "first_message": false,
            "event": { "kind": "raid", "months": 12, "gifts": 5, "viewers": 120, "bits": 500 },
            "badge_feed_id": null, "draft": null
        }))
        .unwrap();
        let sample = try_sample(&request);
        assert_eq!((sample.event, sample.amount, sample.user_login.as_str()), (Some(EventKind::Raid), 120, "viewer"));
        assert!(request.draft.is_none() && request.badge_feed_id.is_none());
    }

    fn newer_rule() -> serde_json::Value {
        json!({ "id": "r-newer", "name": "From the future", "enabled": true, "live": true,
                "when": { "kind": "hype_train" }, "actions": [], "mystery": [1, 2] })
    }

    fn group() -> Automations {
        Automations { rules: vec![raw_rule("r-1"), newer_rule()], ..Automations::default() }
    }

    fn typed(a: &Automations, id: &str) -> Rule {
        Rule::from_value(a.rules.iter().find(|r| id_of(r) == id).unwrap()).unwrap()
    }

    #[test]
    fn a_rule_edit_keeps_unknown_fields_and_never_touches_a_newer_rule() {
        let mut a = group();
        let mut rule = typed(&a, "r-1");
        rule.name = "Thanks a lot".into();
        let answer = save_rule_in(&mut a, rule, Some(10), 1_000).unwrap();
        assert!(answer.saved, "{:?}", answer.errors);
        let stored = &a.rules[0];
        assert_eq!(stored["name"], "Thanks a lot");
        assert_eq!(stored["actions"][0]["future_field"], 7);
        assert_eq!(stored["limits"]["later"], true);
        assert_eq!(a.rules[1], newer_rule());
    }

    #[test]
    fn nulls_clear_hours_and_the_send_target() {
        let mut a = group();
        let mut rule = typed(&a, "r-1");
        rule.only_if.hours = None;
        if let Action::Send { to, .. } = &mut rule.actions[1] {
            *to = None;
        }
        save_rule_in(&mut a, rule, Some(10), 1_000).unwrap();
        assert!(a.rules[0]["only_if"]["hours"].is_null());
        assert!(a.rules[0]["actions"][1]["to"].is_null());
    }

    #[test]
    fn reordering_actions_and_deleting_a_line_keep_each_elements_fields() {
        let mut a = group();
        let mut rule = typed(&a, "r-1");
        rule.actions.swap(0, 1);
        if let Action::Reply { texts, .. } = &mut rule.actions[1] {
            texts.remove(0);
        }
        save_rule_in(&mut a, rule, Some(10), 1_000).unwrap();
        let actions = &a.rules[0]["actions"];
        assert_eq!(actions[0]["id"], "a-2");
        assert_eq!(actions[1]["id"], "a-1");
        assert_eq!(actions[1]["future_field"], 7);
        assert_eq!(actions[1]["texts"].as_array().unwrap().len(), 1);
        assert_eq!(actions[1]["texts"][0]["id"], "l-2");
    }

    #[test]
    fn a_kind_change_leaves_nothing_of_the_old_action() {
        let mut a = group();
        let mut rule = typed(&a, "r-1");
        rule.actions[0] = Action::Send { id: "a-1".into(), to: None, texts: vec![automations::Line { id: "l-9".into(), text: "hello".into() }], pick: automations::Pick::Random };
        save_rule_in(&mut a, rule, Some(10), 1_000).unwrap();
        let first = a.rules[0]["actions"][0].as_object().unwrap();
        assert_eq!(first["kind"], "send");
        assert!(!first.contains_key("thread"));
        assert!(!first.contains_key("future_field"));
    }

    #[test]
    fn a_stale_editor_is_refused_and_changes_nothing() {
        let mut a = group();
        let before = a.clone();
        let mut rule = typed(&a, "r-1");
        rule.name = "Changed".into();
        assert_eq!(save_rule_in(&mut a, rule, Some(9), 1_000), Err("Changed in another window".into()));
        assert_eq!(a, before);
    }

    #[test]
    fn a_save_never_changes_the_switch_or_test_and_live() {
        let mut a = group();
        let mut rule = typed(&a, "r-1");
        rule.enabled = false;
        rule.live = false;
        save_rule_in(&mut a, rule, Some(10), 1_000).unwrap();
        let stored = typed(&a, "r-1");
        assert!(stored.enabled && stored.live);
        assert_eq!(stored.created_ms, 5);
        assert_eq!(stored.updated_ms, 1_000);
    }

    #[test]
    fn flags_keep_updated_ms_and_a_new_rule_starts_in_test() {
        let mut a = group();
        assert_eq!(set_flags_in(&mut a, "r-1", Some(false), Some(false)), Ok(true));
        let stored = typed(&a, "r-1");
        assert!(!stored.enabled && !stored.live);
        assert_eq!(stored.updated_ms, 10);
        assert!(set_flags_in(&mut a, "r-newer", Some(false), None).is_err());
        let mut fresh = typed(&a, "r-1");
        fresh.id = String::new();
        fresh.live = true;
        fresh.enabled = false;
        let answer = save_rule_in(&mut a, fresh, None, 2_000).unwrap();
        assert!(answer.saved);
        assert!(answer.rule.id.starts_with("r-"));
        assert!(answer.rule.enabled && !answer.rule.live);
        assert_eq!(a.rules.len(), 3);
    }

    #[test]
    fn delete_and_move_work_on_raw_rules() {
        let mut a = group();
        assert!(move_rule_in(&mut a, "r-newer", Some("r-1")));
        assert_eq!(id_of(&a.rules[0]), "r-newer");
        assert!(move_rule_in(&mut a, "r-newer", None));
        assert_eq!(id_of(&a.rules[1]), "r-newer");
        assert!(delete_rule_in(&mut a, "r-newer"));
        assert!(!delete_rule_in(&mut a, "r-newer"));
        assert_eq!(a.rules.len(), 1);
    }

    #[test]
    fn every_restore_pauses() {
        let mut a = group();
        a.run = RunState::On;
        a.away = Some(Away { message: "old afk".into(), since_ms: 1 });
        a.converted_reminders = vec!["rem-1".into()];
        pause_restored(&mut a);
        assert_eq!(a.run, RunState::Test);
        assert!(a.away.is_none());
        assert!(a.rules.iter().all(|r| r["live"] == false));
        assert_eq!(a.rules[1]["mystery"], json!([1, 2]), "a newer rule keeps everything else");
        assert_eq!(a.converted_reminders, vec!["rem-1".to_string()]);
        let mut off = Automations { run: RunState::Off, ..Automations::default() };
        pause_restored(&mut off);
        assert_eq!(off.run, RunState::Off);
    }

    #[test]
    fn away_sets_and_clears_with_the_remembered_message() {
        let mut a = Automations::default();
        assert!(apply_away(&mut a, Some(""), 5));
        assert_eq!(a.away.as_ref().unwrap().message, DEFAULT_AWAY_MESSAGE);
        assert!(apply_away(&mut a, Some(" lunch "), 6));
        assert_eq!(a.away, Some(Away { message: "lunch".into(), since_ms: 6 }));
        assert_eq!(a.last_away_message, "lunch");
        assert!(apply_away(&mut a, None, 7));
        assert!(a.away.is_none());
        assert!(!apply_away(&mut a, None, 8));
        assert!(apply_away(&mut a, Some(""), 9));
        assert_eq!(a.away.as_ref().unwrap().message, "lunch");
    }

    #[test]
    fn run_state_remembers_what_resume_returns_to() {
        let mut a = Automations::default();
        assert!(apply_run(&mut a, RunState::On));
        assert_eq!(a.last_active, RunState::On);
        assert!(apply_run(&mut a, RunState::Off));
        assert_eq!(a.last_active, RunState::On);
        assert!(!apply_run(&mut a, RunState::Off));
    }

    #[test]
    fn every_command_here_is_registered_and_allowed() {
        let lib = include_str!("../lib.rs").replace("\r\n", "\n");
        let acl = include_str!("../../permissions/app-commands.toml").replace("\r\n", "\n");
        let src = include_str!("account_automations.rs").replace("\r\n", "\n");
        let code = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        let mut found = 0;
        for chunk in code.split("#[tauri::command]").skip(1) {
            let name = chunk.split("fn ").nth(1).and_then(|s| s.split('(').next()).unwrap().trim();
            assert!(lib.contains(&format!("commands::account_automations::{name},")), "{name} missing from generate_handler!");
            assert!(acl.contains(&format!("\"{name}\",")), "{name} missing from app-commands.toml");
            found += 1;
        }
        assert_eq!(found, 19);
    }
}
