//! The timed and whisper sources of jobs: the engine's counters (outside
//! `Compiled`, D271), the tick task that feeds `timers::Machine` (driven by
//! the engine's own steady clock), the whisper admission, and the age bound a
//! timed job counts from its deadline.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::reminder_claim::{delay_window_s, interval_window_s, offset_text, ReminderSlot};
use crate::models::automations::{ChannelScope, Trigger};
use crate::services::chat_rules::channel_filter_key;

use super::claims;
use super::compile::{Compiled, CompiledRule};
use super::counts::TimedFired;
use super::facts::{self, Facts};
use super::matcher;
use super::timers::{self, Fire, Spec, SpecKind, SystemZone};
use super::{lock, now_ms, Engine, AUTOMATIONS_LABEL};

/// How often the tick task looks.
pub const TICK: Duration = Duration::from_secs(1);
/// A whisper older than this by Twitch time never fires a rule.
pub const WHISPER_MAX_AGE_MS: i64 = 30_000;
/// One room lookup may take this long before the fire is skipped.
pub const ROOM_LOOKUP: Duration = Duration::from_secs(3);

/// Other people's chat lines per chat key and what each Timer saw at its last
/// run. One lock-free counter per chat once created (a read-mostly map).
#[derive(Default)]
pub struct Counters {
    per_chat: Mutex<HashMap<String, Arc<AtomicU64>>>,
    seen: Mutex<HashMap<(String, String), u64>>,
}

impl Counters {
    fn cell(&self, chat_key: &str) -> Arc<AtomicU64> {
        lock(&self.per_chat)
            .entry(chat_key.to_string())
            .or_default()
            .clone()
    }

    pub fn bump(&self, chat_key: &str) {
        self.cell(chat_key).fetch_add(1, Ordering::Relaxed);
    }

    pub fn value(&self, chat_key: &str) -> u64 {
        self.cell(chat_key).load(Ordering::Relaxed)
    }

    /// Lines since the Timer's last run.
    pub fn since(&self, rule_id: &str, chat_key: &str) -> u64 {
        let seen = lock(&self.seen)
            .get(&(rule_id.to_string(), chat_key.to_string()))
            .copied()
            .unwrap_or(0);
        self.value(chat_key).saturating_sub(seen)
    }

    /// The Timer ran: it has now seen every line so far.
    pub fn note_run(&self, rule_id: &str, chat_key: &str) {
        let now = self.value(chat_key);
        lock(&self.seen).insert((rule_id.to_string(), chat_key.to_string()), now);
    }
}

/// When a window joined each chat, as (steady ms from `Engine::now`, wall ms):
/// the Delay trigger's start. Engine-owned, never drained by reconcile (the
/// `rec.window_claims` map is, and holds adoptions only). Set from
/// `note_window_claim_on` (E19f) and by the tick when it finds a chat a
/// window already holds; removed when the last window lets go.
#[derive(Default)]
pub struct ChatJoins {
    map: Mutex<HashMap<String, (i64, i64)>>,
}

impl ChatJoins {
    pub fn note(&self, login: &str, steady_ms: i64, wall_ms: i64) {
        lock(&self.map)
            .entry(login.trim_start_matches('#').to_lowercase())
            .or_insert((steady_ms, wall_ms));
    }

    pub fn forget(&self, login: &str) {
        lock(&self.map).remove(&login.trim_start_matches('#').to_lowercase());
    }

    pub fn get(&self, login: &str) -> Option<(i64, i64)> {
        lock(&self.map)
            .get(&login.trim_start_matches('#').to_lowercase())
            .copied()
    }
}

/// The chats a timed rule acts in now, as (chat key, login).
fn chats_of(engine: &Engine, cr: &CompiledRule) -> Vec<(String, String)> {
    match &cr.rule.only_if.channels {
        ChannelScope::Own {} if !cr.owner_login.is_empty() => {
            vec![(cr.own_key.clone(), cr.owner_login.clone())]
        }
        ChannelScope::These { .. } => cr
            .these_keys
            .iter()
            .map(|k| {
                (
                    k.clone(),
                    k.split_once(':').map_or(k.clone(), |(_, l)| l.to_string()),
                )
            })
            .collect(),
        ChannelScope::Watching {} => engine
            .local
            .current_watch()
            .map(|login| vec![(channel_filter_key("twitch", &login), login)])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn spec_of(
    engine: &Engine,
    cr: &CompiledRule,
    chat_key: &str,
    started_ms: Option<i64>,
    joined: Option<(i64, i64)>,
) -> Option<Spec> {
    let rule = &cr.rule;
    let fired = lock(&engine.counts).timed(&rule.id, chat_key);
    let kind = match &rule.when {
        Trigger::Timer { every_seconds, .. } => SpecKind::Timer {
            every_s: *every_seconds,
        },
        Trigger::Clock { at, days } => SpecKind::Clock {
            at_min: crate::models::automations::parse_hhmm(at)?,
            days: days.clone(),
        },
        Trigger::Uptime { seconds, late_ok } => SpecKind::Uptime {
            seconds: *seconds,
            started_ms,
            late_ok: *late_ok,
        },
        Trigger::Delay { seconds } => SpecKind::Delay {
            seconds: *seconds,
            joined_steady_ms: joined.map(|j| j.0),
            joined_id: joined.map(|j| j.1),
        },
        _ => return None,
    };
    Some(Spec {
        rule_id: rule.id.clone(),
        chat_key: chat_key.to_string(),
        kind,
        fired,
    })
}

/// One tick: gather specs for every running timed rule, step the machine and
/// admit what is due. Off compiles no timed rule. `steady_ms` is
/// `Engine::now()`, the clock the join stamps use.
pub(crate) async fn tick(engine: &Arc<Engine>, steady_ms: i64, wall_ms: i64) {
    let compiled = engine.compiled();
    if compiled.timed_rules.is_empty() || !engine.foreground() {
        return;
    }
    let twitch_now = crate::services::whisper_sync::twitch_now(wall_ms);
    let mut specs: Vec<Spec> = Vec::new();
    let mut meta: Vec<(usize, String)> = Vec::new();
    for &index in &compiled.timed_rules {
        let cr = &compiled.rules[index];
        for (chat_key, login) in chats_of(engine, cr) {
            // A rule that is only-live arms at go-live: it has no spec (and no
            // phase) while the chat is offline.
            let live = engine.local.is_live(&login).await;
            if cr.rule.only_if.only_live && live != Some(true) {
                lock(&engine.timed).forget(&cr.rule.id, &chat_key);
                continue;
            }
            let started = match &cr.rule.when {
                Trigger::Uptime { .. } => engine
                    .local
                    .channel_info(&login)
                    .await
                    .and_then(|i| i.started_at)
                    .and_then(|s| {
                        chrono::DateTime::parse_from_rfc3339(&s)
                            .ok()
                            .map(|d| d.timestamp_millis())
                    }),
                _ => None,
            };
            // Delay counts from when a window opened the chat: a window holds it now
            // (a label other than the engine's) or it does not.
            let joined = if matches!(cr.rule.when, Trigger::Delay { .. }) {
                let open = engine
                    .local
                    .holders(&login)
                    .await
                    .iter()
                    .any(|h| h != AUTOMATIONS_LABEL);
                if open {
                    engine.chat_joins.note(&login, steady_ms, wall_ms);
                } else {
                    engine.chat_joins.forget(&login);
                }
                engine.chat_joins.get(&login)
            } else {
                None
            };
            if let Some(spec) = spec_of(engine, cr, &chat_key, started, joined) {
                specs.push(spec);
                meta.push((index, login));
            }
        }
    }
    lock(&engine.timed).retain(&specs);
    let fires: Vec<Fire> =
        lock(&engine.timed).tick(steady_ms, wall_ms, twitch_now, &specs, &SystemZone);
    // Each room lookup is bounded and they run together, so one slow lookup
    // never delays the others or trips the wall-gap `resume()`.
    let jobs = fires.into_iter().map(|fire| {
        let (index, login) = meta[fire.spec].clone();
        let spec = specs[fire.spec].clone();
        let engine = engine.clone();
        let compiled = compiled.clone();
        async move { admit_fire(&engine, &compiled, index, &spec, &login, &fire, steady_ms).await }
    });
    futures::future::join_all(jobs).await;
}

async fn admit_fire(
    engine: &Arc<Engine>,
    compiled: &Arc<Compiled>,
    index: usize,
    spec: &Spec,
    login: &str,
    fire: &Fire,
    steady_ms: i64,
) {
    let cr = &compiled.rules[index];
    let rule = &cr.rule;
    // Stamp first: a missed or skipped fire is never retried (a deadline stamped is not re-fired).
    lock(&engine.counts).set_timed(&rule.id, &spec.chat_key, fire.stamp.clone());
    engine.note_counts_changed();
    if let SpecKind::Timer { every_s } = spec.kind {
        lock(&engine.timed).moved(&rule.id, &spec.chat_key, steady_ms, every_s);
    }
    if fire.missed {
        engine.skip(rule, &spec.chat_key, "Skipped: too late for this time");
        return;
    }
    // A Timer skips below `min_messages` other people's lines since its last run.
    if let Trigger::Timer { min_messages, .. } = &rule.when {
        let since = engine.counters.since(&rule.id, &spec.chat_key);
        if since < u64::from(*min_messages) {
            engine.skip(
                rule,
                &spec.chat_key,
                &format!("Skipped: only {since} messages since last time"),
            );
            return;
        }
        engine.counters.note_run(&rule.id, &spec.chat_key);
    }
    let room_id = match tokio::time::timeout(ROOM_LOOKUP, engine.outward.room_id(login)).await {
        Ok(Ok(id)) => id,
        Ok(Err(why)) => {
            engine.skip(rule, &spec.chat_key, &why);
            return;
        }
        Err(_) => {
            engine.skip(
                rule,
                &spec.chat_key,
                "Skipped: couldn't find the chat in time",
            );
            return;
        }
    };
    // PLAN § 4.8 keys, on the chat key; the Clock key carries the UTC offset in
    // force at the scheduled time (`clock_slot_at`, the reminders' own reader).
    let key = match (&rule.when, &fire.local) {
        (Trigger::Timer { .. }, _) => claims::timer_key(&spec.chat_key),
        (Trigger::Clock { .. }, Some((date, hhmm))) => {
            let offset_s = clock_offset_s(date, hhmm);
            claims::clock_key(&spec.chat_key, date, hhmm, offset_s)
        }
        (Trigger::Uptime { seconds, .. }, _) => {
            claims::uptime_key(&spec.chat_key, fire.stamp.fired_for, *seconds)
        }
        (Trigger::Delay { .. }, _) => claims::delay_key(&spec.chat_key),
        _ => return,
    };
    let facts = Arc::new(facts::from_timed(key, login, &room_id, now_ms()));
    let away = engine.away();
    let ctx = engine.match_ctx(away.as_ref(), false);
    let verdict = matcher::evaluate_rule(compiled, index, &facts, &ctx);
    if !verdict.fired {
        engine.skip(
            rule,
            &spec.chat_key,
            verdict.reason.as_deref().unwrap_or("Skipped"),
        );
        return;
    }
    // Age counts from the deadline (D283); a `late_ok` Uptime from admission.
    let Some(bound) = timers::job_age_bound(&spec.kind, rule.limits.max_wait_seconds, fire.late_ms)
    else {
        engine.skip(rule, &spec.chat_key, "Skipped: too late to answer");
        return;
    };
    engine.admit_rule(
        compiled,
        index,
        facts,
        Vec::new(),
        HashMap::new(),
        engine.now(),
        Some(bound),
    );
}

/// The UTC offset in force at a Clock's scheduled local time (the offset
/// before the gap for a missing time), through the reminders' reader.
fn clock_offset_s(date: &str, hhmm: &str) -> i32 {
    let day = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok();
    let (h, m) = hhmm
        .split_once(':')
        .and_then(|(h, m)| Some((h.parse().ok()?, m.parse().ok()?)))
        .unwrap_or((0, 0));
    match day.map(|d| crate::services::reminder_service::clock_slot_at(d, h, m)) {
        Some(ReminderSlot::Clock { offset_s, .. }) => offset_s,
        _ => 0,
    }
}

/// A timed claim's window by trigger (PLAN § 4.8): a Timer `ceil(0.9 x every)`,
/// a Delay `max(seconds, 600)`; the reminders' own functions.
pub fn timed_window_s(when: &Trigger) -> Option<u32> {
    match when {
        Trigger::Timer { every_seconds, .. } => Some(interval_window_s(u64::from(*every_seconds))),
        Trigger::Delay { seconds } => Some(delay_window_s(u64::from(*seconds))),
        _ => None,
    }
}

/// `+HH:MM` for a Clock key, re-exported so `claims.rs` stays pure.
pub fn offset_words(offset_s: i32) -> String {
    offset_text(offset_s)
}

/// The whisper gates, then one job per Whisper rule that fires: a duplicate
/// `whisper_id`, a whisper older than 30 s by Twitch time, one of your own
/// accounts as the sender, an owner that is not the primary (the compile hold)
/// or a sender `knows` says no to fire nothing. A device whose deep sync is
/// not Complete and whose `you_wrote` is not true reads "Waiting for your
/// whisper history" (the door's `knows` is false and the list is not read);
/// once the list is read, a stranger simply does not fire. Whoever filed it
/// first, the engine sees every whisper.
pub(crate) async fn admit_whisper_now(
    engine: &Arc<Engine>,
    compiled: &Arc<Compiled>,
    facts: Arc<Facts>,
    twitch_ms: i64,
    twitch_now_ms: i64,
) {
    if twitch_now_ms - twitch_ms > WHISPER_MAX_AGE_MS {
        return;
    }
    if compiled.is_mine(&facts.user_id) {
        return;
    }
    if !lock(&engine.counts).note_whisper(&facts.message_id) {
        return;
    }
    engine.note_counts_changed();
    for &index in &compiled.whisper_rules {
        let cr = &compiled.rules[index];
        let door = engine.outward.whisper_door();
        if !door.knows(&cr.rule.owner.id, &facts.user_id).await {
            if !door.history_complete(&cr.rule.owner.id).await {
                engine.skip(&cr.rule, "whisper", "Waiting for your whisper history");
            }
            continue;
        }
        if engine.outward.primary_id().as_deref() != Some(cr.rule.owner.id.as_str()) {
            continue;
        }
        let away = engine.away();
        let ctx = engine.match_ctx(away.as_ref(), false);
        if matcher::evaluate_rule(compiled, index, &facts, &ctx).fired {
            engine.admit_rule(
                compiled,
                index,
                facts.clone(),
                Vec::new(),
                HashMap::new(),
                engine.now(),
                None,
            );
        }
    }
}

/// The tick task: once a second on the engine's own steady clock (and
/// `Machine::resume` on a wall-clock gap). It runs on the engine's spawner.
pub(crate) fn start(engine: &Arc<Engine>) {
    let task = engine.clone();
    engine.spawn(async move {
        let engine = task;
        let mut last_wall = chrono::Utc::now().timestamp_millis();
        loop {
            tokio::time::sleep(TICK).await;
            let wall = chrono::Utc::now().timestamp_millis();
            if (wall - last_wall - TICK.as_millis() as i64).abs() > 30_000 {
                lock(&engine.timed).resume();
            }
            last_wall = wall;
            tick(&engine, engine.now(), wall).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_survive_a_recompile_and_count_the_difference() {
        let c = Counters::default();
        for _ in 0..4 {
            c.bump("twitch:a");
        }
        assert_eq!(c.since("t", "twitch:a"), 4);
        c.note_run("t", "twitch:a");
        for _ in 0..6 {
            c.bump("twitch:a");
        }
        // A recompile builds a new `Compiled`; the counters live in the Engine.
        assert_eq!(c.since("t", "twitch:a"), 6);
        assert_eq!(
            c.since("other-rule", "twitch:a"),
            10,
            "each Timer keeps its own last-seen value"
        );
        assert_eq!(c.since("t", "twitch:b"), 0);
    }

    #[test]
    fn a_join_is_kept_until_forgotten_and_never_restamped() {
        let j = ChatJoins::default();
        j.note("#Chan", 1000, 5000);
        j.note("chan", 9999, 9999);
        assert_eq!(j.get("CHAN"), Some((1000, 5000)));
        j.forget("chan");
        assert_eq!(j.get("chan"), None);
    }

    #[test]
    fn the_windows_are_the_reminders() {
        assert_eq!(
            timed_window_s(&Trigger::Timer {
                every_seconds: 900,
                min_messages: 0
            }),
            Some(810)
        );
        assert_eq!(timed_window_s(&Trigger::Delay { seconds: 60 }), Some(600));
        assert_eq!(timed_window_s(&Trigger::Delay { seconds: 900 }), Some(900));
        assert_eq!(
            timed_window_s(&Trigger::Clock {
                at: "19:00".into(),
                days: vec![]
            }),
            None
        );
    }
}
