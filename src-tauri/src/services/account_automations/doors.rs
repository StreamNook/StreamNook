//! The non-chat step subset (D273) for Command, moderation (Timeout, Delete,
//! Pin, Shoutout), Whisper (and a Whisper rule's Reply), a Kick actor's
//! Timeout and Delete, plus the Announce preflight and the control actions
//! (JoinChat, LeaveChat, Log).
//!
//! In order: render and parse (a failed parse skips before any door), verify
//! `send_as` (step 2), the action's own budget (whisper: `knows`, the local 40
//! check, the day hold; moderation: a slot of the 10 a minute reserved under
//! the lock and the holds; Command: none), a `SetGame`'s category read, the
//! final checks (step 5), the claim (step 7), verify again plus the final
//! checks with no await before the call (step 9 without a ticket), then the
//! one door call. They book no ticket, never call `window_fits_engine` and
//! never re-book after a slow claim. A reserved slot is given back on every
//! skip and on a Refused (a `Guard` that gives back on drop); Sent and Unknown
//! keep it. Outcome and release rules are § 4.5's door table.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::models::automations::{is_kick_sender, Action, SenderRole};
use crate::services::chat_send_limiter::Role;
use crate::services::providers::kick_door::KickOutcome;
use crate::services::twitch_service::{DoorOutcome, SearchFail};
use crate::services::whisper_recipients;
use crate::services::whisper_send::Outcome as WhisperOutcome;

use super::activity::EntryKind;
use super::claims::ClassPlan;
use super::command::{parse_command, CommandAction};
use super::compile::Compiled;
use super::executor::{self, Claimed, Job, JobRun};
use super::facts::Facts;
use super::outward::WhisperTo;
use super::{lock, now_ms, sign_in, token_words, Engine};

/// Moderation actions across rules in a minute.
pub const MOD_PER_MINUTE: usize = 10;
/// A 429 holds that kind in that chat this long.
pub const HOLD_429_MS: i64 = 10_000;
/// A 429 on a whisper holds whispers this long.
pub const WHISPER_429_MS: i64 = 60_000;
/// Twitch allows one announcement per 2 s.
pub const ANNOUNCE_GAP_MS: i64 = 2_000;
/// Shoutouts: one per room per 2 minutes, one per (room, target) per hour.
pub const SHOUT_ROOM_MS: i64 = 120_000;
pub const SHOUT_TARGET_MS: i64 = 3_600_000;
/// A Kick timeout lasts whole minutes up to 7 days.
pub const KICK_TIMEOUT_MAX_S: u32 = 604_800;

/// What a reserved shoutout replaced, to give it back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShoutPrior {
    room: Option<i64>,
    target: Option<i64>,
}

/// Local holds and budgets for the door classes, on the steady clock. Every
/// budget is reserved at its check (under the engine's lock) so two jobs that
/// both check before either sends can never both pass.
#[derive(Default)]
pub struct Holds {
    kind_until: HashMap<(String, String), i64>,
    whisper_until: i64,
    /// New whisper recipients are held until this wall ms after a `day` loss.
    whisper_day_until: i64,
    announce_next: HashMap<String, i64>,
    shout_room: HashMap<String, i64>,
    shout_target: HashMap<(String, String), i64>,
    mod_times: VecDeque<i64>,
}

impl Holds {
    fn prune_mod(&mut self, now: i64) {
        while self.mod_times.front().is_some_and(|t| now - *t >= 60_000) {
            self.mod_times.pop_front();
        }
    }

    /// Reserve one of the 10 moderation slots a minute; the 11th in 60 s is
    /// skipped. Answers the reservation's stamp (to give back or re-check).
    pub fn reserve_mod(&mut self, now: i64) -> Result<i64, String> {
        self.prune_mod(now);
        if self.mod_times.len() >= MOD_PER_MINUTE {
            return Err("Moderation is limited to 10 actions a minute".into());
        }
        // A strictly increasing stamp, so two reservations in one ms stay two.
        let stamp = self.mod_times.back().map_or(now, |last| now.max(*last + 1));
        self.mod_times.push_back(stamp);
        Ok(stamp)
    }

    pub fn give_back_mod(&mut self, stamp: i64) {
        if let Some(i) = self.mod_times.iter().position(|t| *t == stamp) {
            self.mod_times.remove(i);
        }
    }

    /// Step 9's re-check: the reservation is still held and nothing paused the kind.
    pub fn mod_still_ok(&mut self, stamp: i64, kind: &str, room: &str, now: i64) -> bool {
        self.prune_mod(now);
        self.mod_times.contains(&stamp) && !self.kind_held(kind, room, now)
    }

    pub fn hold_kind(&mut self, kind: &str, room: &str, until: i64) {
        self.kind_until
            .insert((kind.to_string(), room.to_string()), until);
    }

    pub fn kind_held(&self, kind: &str, room: &str, now: i64) -> bool {
        self.kind_until
            .get(&(kind.to_string(), room.to_string()))
            .is_some_and(|u| *u > now)
    }

    pub fn hold_whispers(&mut self, until: i64) {
        self.whisper_until = until;
    }

    pub fn whispers_held(&self, now: i64) -> bool {
        self.whisper_until > now
    }

    /// After a `day` loss: new recipients wait until `retry_after_ms` is over.
    pub fn hold_new_recipients_until(&mut self, wall_ms: i64) {
        self.whisper_day_until = wall_ms;
    }

    pub fn new_recipients_held(&self, wall_ms: i64) -> bool {
        self.whisper_day_until > wall_ms
    }

    /// Reserve the room's next Announce slot (Twitch allows one per 2 s) and
    /// answer (how long to wait before booking, the slot's steady ms): the
    /// caller waits first (step 3), so nothing sleeps after step 9. Two
    /// Announces in one room get slots 2 s apart; another room and other kinds
    /// of send are not held.
    pub fn reserve_announce(&mut self, room: &str, now: i64) -> (i64, i64) {
        let slot = self
            .announce_next
            .get(room)
            .copied()
            .unwrap_or(now)
            .max(now);
        self.announce_next
            .insert(room.to_string(), slot + ANNOUNCE_GAP_MS);
        (slot - now, slot)
    }

    /// A skipped or Refused Announce gives its slot back, when no later Announce
    /// has reserved behind it (then the gap stays: it only costs a wait).
    pub fn give_back_announce(&mut self, room: &str, slot: i64) {
        if self.announce_next.get(room) == Some(&(slot + ANNOUNCE_GAP_MS)) {
            self.announce_next.insert(room.to_string(), slot);
        }
    }

    /// Twitch's own limits, per broadcaster: reserve the room (120 s) and the
    /// (room, target) hour at the check. Answers what to give back.
    pub fn reserve_shout(
        &mut self,
        room: &str,
        target: &str,
        now: i64,
    ) -> Result<ShoutPrior, String> {
        let key = (room.to_string(), target.to_string());
        if self.shout_room.get(room).is_some_and(|u| *u > now) {
            return Err("Shoutouts are limited to one every 2 minutes".into());
        }
        if self.shout_target.get(&key).is_some_and(|u| *u > now) {
            return Err("They were already shouted out in the last hour".into());
        }
        let prior = ShoutPrior {
            room: self.shout_room.get(room).copied(),
            target: self.shout_target.get(&key).copied(),
        };
        self.shout_room
            .insert(room.to_string(), now + SHOUT_ROOM_MS);
        self.shout_target.insert(key, now + SHOUT_TARGET_MS);
        Ok(prior)
    }

    pub fn give_back_shout(&mut self, room: &str, target: &str, prior: ShoutPrior) {
        let key = (room.to_string(), target.to_string());
        match prior.room {
            Some(u) => self.shout_room.insert(room.to_string(), u),
            None => self.shout_room.remove(room),
        };
        match prior.target {
            Some(u) => self.shout_target.insert(key, u),
            None => self.shout_target.remove(&key),
        };
    }
}

/// A reserved slot that gives itself back on drop unless `keep` was called
/// (Sent and Unknown keep it; every skip and a Refused give it back).
struct Reserved<'a> {
    holds: &'a Mutex<Holds>,
    mod_stamp: Option<i64>,
    shout: Option<(String, String, ShoutPrior)>,
    kept: bool,
}

impl Reserved<'_> {
    fn keep(&mut self) {
        self.kept = true;
    }
}

impl Drop for Reserved<'_> {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        let mut holds = lock(self.holds);
        if let Some(stamp) = self.mod_stamp {
            holds.give_back_mod(stamp);
        }
        if let Some((room, target, prior)) = self.shout.take() {
            holds.give_back_shout(&room, &target, prior);
        }
    }
}

/// The Helix settings body of one chat-settings command (today's arms' JSON).
pub fn settings_body(action: &CommandAction) -> Option<Value> {
    Some(match action {
        CommandAction::Slow(s) => json!({ "slow_mode": true, "slow_mode_wait_time": s }),
        CommandAction::SlowOff => json!({ "slow_mode": false }),
        CommandAction::Followers(m) => {
            json!({ "follower_mode": true, "follower_mode_duration": m })
        }
        CommandAction::FollowersOff => json!({ "follower_mode": false }),
        CommandAction::Subscribers => json!({ "subscriber_mode": true }),
        CommandAction::SubscribersOff => json!({ "subscriber_mode": false }),
        CommandAction::UniqueChat => json!({ "unique_chat_mode": true }),
        CommandAction::UniqueChatOff => json!({ "unique_chat_mode": false }),
        CommandAction::EmoteOnly => json!({ "emote_mode": true }),
        CommandAction::EmoteOnlyOff => json!({ "emote_mode": false }),
        CommandAction::Marker(_) | CommandAction::SetTitle(_) | CommandAction::SetGame(_) => {
            return None
        }
    })
}

/// The id `SetGame` writes, from the read's rows: the row whose name equals
/// the query ignoring case, with an id that is neither empty nor "0" (Twitch
/// reads both as unsetting the category). `pick_category`'s `""` fallback is
/// never sent.
pub fn exact_category(rows: &[Value], query: &str) -> Option<String> {
    let wanted = query.trim();
    rows.iter()
        .find(|r| {
            r["name"]
                .as_str()
                .is_some_and(|n| n.trim().eq_ignore_ascii_case(wanted))
        })
        .and_then(|r| r["id"].as_str())
        .filter(|id| !id.is_empty() && *id != "0")
        .map(str::to_string)
}

/// Timeout, Delete and Pin never act on a moderator, VIP, the broadcaster, any
/// of your accounts or a partner row; Shoutout's only target rail is "not one
/// of your accounts" (a VIP or a raider is a normal target). Pure.
pub fn target_rail(compiled: &Compiled, facts: &Facts, shoutout: bool) -> Result<(), String> {
    if compiled.is_mine(&facts.user_id) {
        return Err("That's one of your accounts".into());
    }
    if shoutout {
        return Ok(());
    }
    if facts.shared_partner {
        return Err("That message is from a Shared Chat partner".into());
    }
    if facts.roles.iter().any(|r| {
        matches!(
            r,
            SenderRole::Broadcaster | SenderRole::Moderator | SenderRole::Vip
        )
    }) {
        return Err("Moderators, VIPs and the broadcaster can't be moderated".into());
    }
    Ok(())
}

/// Only where the acting account is a moderator or the broadcaster there.
pub fn actor_may_moderate(role: Option<Role>) -> bool {
    matches!(role, Some(Role::Moderator | Role::Broadcaster))
}

/// The person a moderation or shoutout targets: the author of a chat row, or a
/// raid's raider (`user-id`).
pub fn target_of(facts: &Facts) -> Option<&str> {
    (!facts.user_id.is_empty()).then_some(facts.user_id.as_str())
}

/// The whisper recipient: the explicit `to`, else (a Whisper rule's Reply form
/// and an empty `to`) the sender.
pub fn whisper_recipient(
    action: &Action,
    facts: &Facts,
    rule_is_whisper: bool,
) -> Option<WhisperTo> {
    match action {
        Action::Whisper { to: Some(t), .. } if !t.user_id.trim().is_empty() => Some(WhisperTo {
            user_id: t.user_id.trim().to_string(),
            login: t.login.clone(),
            name: t.name.clone(),
        }),
        Action::Whisper { .. } | Action::Reply { .. }
            if rule_is_whisper && !facts.user_id.is_empty() =>
        {
            Some(WhisperTo {
                user_id: facts.user_id.clone(),
                login: facts.user_login.clone(),
                name: facts.user_name.clone(),
            })
        }
        _ => None,
    }
}

fn skip(engine: &std::sync::Arc<Engine>, job: &Job, reason: &str) {
    executor::skip(engine, job, reason);
}

/// Whether `action` runs on this module's path (a Reply only on a Whisper rule).
pub fn is_door_action(action: &Action, rule_is_whisper: bool) -> bool {
    match action {
        Action::Command { .. }
        | Action::Whisper { .. }
        | Action::Timeout { .. }
        | Action::Delete { .. }
        | Action::Pin { .. }
        | Action::Shoutout { .. } => true,
        Action::Reply { .. } => rule_is_whisper,
        _ => false,
    }
}

/// A 401 from a door or the category read: the cached token goes and the
/// account's failure is remembered for a minute (the real doors stay one line).
fn unauthorized(engine: &Engine, send_as: &str, token: &super::outward::VerifiedToken) {
    engine.outward.drop_token(token);
    engine.outward.record_send_401(send_as);
}

/// The door the action reaches, resolved to the typed call.
enum Call {
    Settings {
        body: Value,
    },
    Marker {
        description: Option<String>,
    },
    Title(String),
    Game(String),
    Timeout {
        target: String,
        seconds: u32,
        reason: Option<String>,
    },
    Delete {
        message_id: String,
    },
    Pin {
        message_id: String,
        seconds: Option<u32>,
    },
    Shoutout {
        target: String,
    },
    Whisper {
        to: WhisperTo,
        text: String,
    },
}

impl Call {
    fn moderation(&self) -> bool {
        matches!(
            self,
            Call::Timeout { .. } | Call::Delete { .. } | Call::Pin { .. } | Call::Shoutout { .. }
        )
    }
}

/// The chat key a job's room reads under: `kick:<slug>` for a Kick actor.
fn room_chat_key(job: &Job, login: &str) -> String {
    if is_kick_sender(&job.rule().rule) {
        crate::services::providers::key::make_key("kick", login)
    } else {
        executor::chat_key_of(login)
    }
}

/// A Kick actor's Timeout or Delete runs the Kick path (P4's door), never the
/// Twitch verify (D277).
fn is_kick_moderation(job: &Job, action: &Action) -> bool {
    is_kick_sender(&job.rule().rule)
        && matches!(action, Action::Timeout { .. } | Action::Delete { .. })
}

/// Run one door action through the D273 subset.
pub(crate) async fn run_door_action(
    engine: &std::sync::Arc<Engine>,
    job: &Job,
    action: &Action,
    plan: &ClassPlan,
    run: &mut JobRun,
) {
    if is_kick_moderation(job, action) {
        return run_kick_moderation(engine, job, action, plan, run).await;
    }
    let cr = job.rule();
    let rule = &cr.rule;
    let send_as = rule.send_as.id.clone();
    let class = plan.class.key();
    let room = job.facts.room_id.clone();
    let login = job.login();
    let chat_key = room_chat_key(job, &login);

    // Render: the chatter-text rule was checked at save; a variable fills here.
    let call_text = match action {
        Action::Command { line, .. } => match executor::expand_plain(engine, job, line).await {
            Ok(expanded) => Some(expanded),
            Err(reason) => return skip(engine, job, &reason),
        },
        Action::Whisper { .. } | Action::Reply { .. } => {
            match executor::render(engine, job, action, &[]).await {
                Ok(text) => Some(text),
                Err(reason) => return skip(engine, job, &reason),
            }
        }
        Action::Timeout { reason, .. } => match executor::expand_plain(engine, job, reason).await {
            Ok(text) => Some(text),
            Err(r) => return skip(engine, job, &r),
        },
        _ => None,
    };

    // Resolve the typed call before any door (a failed parse skips here).
    let call = match action {
        Action::Command { .. } => {
            let parsed = match parse_command(call_text.as_deref().unwrap_or("")) {
                Ok(p) => p,
                Err(e) => return skip(engine, job, &e),
            };
            match &parsed {
                CommandAction::Marker(d) => Call::Marker {
                    description: d.clone(),
                },
                CommandAction::SetTitle(t) => Call::Title(t.clone()),
                CommandAction::SetGame(g) => Call::Game(g.clone()),
                other => Call::Settings {
                    body: settings_body(other).unwrap_or(Value::Null),
                },
            }
        }
        Action::Whisper { .. } | Action::Reply { .. } => {
            let Some(to) = whisper_recipient(action, &job.facts, rule.is_whisper_rule()) else {
                return skip(engine, job, "Pick a person you have whispered with");
            };
            Call::Whisper {
                to,
                text: call_text.clone().unwrap_or_default(),
            }
        }
        Action::Timeout { seconds, .. } => {
            let Some(target) = target_of(&job.facts) else {
                return skip(engine, job, "No person to time out");
            };
            Call::Timeout {
                target: target.to_string(),
                seconds: *seconds,
                reason: call_text.clone().filter(|r| !r.trim().is_empty()),
            }
        }
        Action::Delete { .. } => {
            if job.facts.message_id.is_empty() {
                return skip(engine, job, "No message to delete");
            }
            Call::Delete {
                message_id: job.facts.message_id.clone(),
            }
        }
        Action::Pin { seconds, .. } => {
            if job.facts.message_id.is_empty() {
                return skip(engine, job, "No message to pin");
            }
            Call::Pin {
                message_id: job.facts.message_id.clone(),
                seconds: *seconds,
            }
        }
        Action::Shoutout { .. } => {
            let Some(target) = target_of(&job.facts) else {
                return skip(engine, job, "No one to shout out");
            };
            Call::Shoutout {
                target: target.to_string(),
            }
        }
        _ => return,
    };
    // The title and a marker are bounded again after expansion, before any request.
    match &call {
        Call::Title(t) if t.chars().count() > super::command::TITLE_MAX => {
            return skip(engine, job, "Titles can be at most 140 characters")
        }
        Call::Marker {
            description: Some(d),
        } if d.chars().count() > super::command::MARKER_MAX => {
            return skip(engine, job, "Keep a marker to 140 characters")
        }
        _ => {}
    }

    // Step 2: verify send_as (the owner is verified for the claim below).
    let Some(budget) = job.left() else {
        return skip(engine, job, "Skipped: too late to answer");
    };
    let acting = match engine.verified(&send_as, budget).await {
        Ok(t) => t,
        Err(why) => return skip(engine, job, &token_words(&why, &rule.send_as.login)),
    };
    if engine.outward.primary_id().as_deref() != Some(send_as.as_str()) {
        return skip(
            engine,
            job,
            &format!(
                "Waiting for @{} to be your main account",
                rule.send_as.login
            ),
        );
    }
    if matches!(call, Call::Title(_) | Call::Game(_)) && room != acting.user_id {
        return skip(
            engine,
            job,
            "Only the broadcaster's own channel can change the title or category",
        );
    }

    // The action's own budget, reserved under the lock and given back on any skip.
    let now = engine.now();
    let mut reserved = Reserved {
        holds: &engine.door_holds,
        mod_stamp: None,
        shout: None,
        kept: false,
    };
    match &call {
        Call::Timeout { .. } | Call::Delete { .. } | Call::Pin { .. } | Call::Shoutout { .. } => {
            let shout = matches!(call, Call::Shoutout { .. });
            if let Err(why) = target_rail(&engine.compiled(), &job.facts, shout) {
                return skip(engine, job, &why);
            }
            if !actor_may_moderate(engine.local.own_role(&rule.send_as.login, &login))
                && room != acting.user_id
            {
                return skip(engine, job, "You're not a moderator in this chat");
            }
            let mut holds = lock(&engine.door_holds);
            if holds.kind_held(&class, &room, now) {
                return skip(
                    engine,
                    job,
                    "Held for a few seconds: Twitch asked for a pause",
                );
            }
            match holds.reserve_mod(now) {
                Ok(stamp) => reserved.mod_stamp = Some(stamp),
                Err(why) => return skip(engine, job, &why),
            }
            if let Call::Shoutout { target } = &call {
                match holds.reserve_shout(&room, target, now) {
                    Ok(prior) => reserved.shout = Some((room.clone(), target.clone(), prior)),
                    Err(why) => return skip(engine, job, &why),
                }
            }
        }
        Call::Whisper { to, .. } => {
            let wall = chrono::Utc::now().timestamp_millis();
            if lock(&engine.door_holds).whispers_held(now) {
                return skip(engine, job, "Held for a minute: Twitch asked for a pause");
            }
            if !engine
                .outward
                .whisper_door()
                .knows(&send_as, &to.user_id)
                .await
            {
                return skip(engine, job, "You haven't whispered with them before");
            }
            let recipients = whisper_recipients::snapshot();
            if !recipients.has(&send_as, &to.user_id, wall)
                && recipients.counted(&send_as, wall) >= whisper_recipients::TWITCH_PER_DAY
            {
                return skip(engine, job, "Twitch allows whispering 40 new people a day");
            }
            if lock(&engine.door_holds).new_recipients_held(wall)
                && !recipients.has(&send_as, &to.user_id, wall)
            {
                return skip(
                    engine,
                    job,
                    "Whisper people a day reached across your devices",
                );
            }
        }
        _ => {}
    }

    // A SetGame's category read: after verify, before the final checks, so a
    // failed read skips before any claim. Never in the write mapping.
    let mut game_id: Option<String> = None;
    if let Call::Game(query) = &call {
        match engine
            .outward
            .command_doors()
            .search_categories(&acting.token, query)
            .await
        {
            Ok(rows) => match exact_category(&rows, query) {
                Some(id) => game_id = Some(id),
                None => return skip(engine, job, &format!("No category named {query}")),
            },
            Err(SearchFail::Unauthorized) => {
                unauthorized(engine, &send_as, &acting);
                return skip(engine, job, "Twitch rejected the sign-in (401)");
            }
            Err(SearchFail::Other(why)) => {
                return skip(
                    engine,
                    job,
                    &format!("Couldn't look up the category: {why}"),
                )
            }
        }
    }

    // Step 5: the final checks (a whisper reads only local state; a room job
    // also checks the chat's own conditions).
    let booked_away = engine.away();
    if matches!(call, Call::Whisper { .. }) {
        if let Err(reason) = executor::local_checks(engine, job, &chat_key, &booked_away, true) {
            return skip(engine, job, &reason);
        }
    } else if let Err(reason) = executor::final_checks(engine, job, &login, &booked_away).await {
        return skip(engine, job, &reason);
    }

    // Step 7: the claim (the owner's verified token is the bearer).
    let owner = match executor::verify_accounts(engine, job).await {
        Ok((_, owner)) => owner,
        Err(reason) => return skip(engine, job, &reason),
    };
    let first_of_class = !run.classes.get(&class).is_some_and(|c| c.won);
    if first_of_class {
        match executor::claim_class(engine, job, plan, &login, &booked_away, None, &owner).await {
            Claimed::Skip(reason) => return skip(engine, job, &reason),
            Claimed::Won { call: won, .. } => {
                let entry = run.classes.entry(class.clone()).or_default();
                entry.won = true;
                entry.calls.extend(won);
            }
        }
    }

    // Step 9: verify again (a cache hit), then every check with no await
    // between the last check and the write.
    let Some(budget9) = job.left() else {
        return skip(engine, job, "Skipped: too late to answer");
    };
    let token = match engine.verified(&send_as, budget9).await {
        Ok(t) => t,
        Err(why) => {
            run.stopped = true;
            return skip(engine, job, &token_words(&why, &rule.send_as.login));
        }
    };
    // From here to the door call there is no await.
    if token.user_id != send_as {
        return skip(engine, job, &sign_in(&rule.send_as.login));
    }
    if engine.outward.primary_id().as_deref() != Some(send_as.as_str()) {
        return skip(
            engine,
            job,
            &format!(
                "Waiting for @{} to be your main account",
                rule.send_as.login
            ),
        );
    }
    if matches!(call, Call::Title(_) | Call::Game(_)) && room != token.user_id {
        return skip(
            engine,
            job,
            "Only the broadcaster's own channel can change the title or category",
        );
    }
    if let Err(reason) = executor::local_checks(engine, job, &chat_key, &booked_away, true) {
        return skip(engine, job, &reason);
    }
    if let Some(stamp) = reserved.mod_stamp {
        if !lock(&engine.door_holds).mod_still_ok(stamp, &class, &room, engine.now()) {
            return skip(
                engine,
                job,
                "Held for a few seconds: Twitch asked for a pause",
            );
        }
    }
    let started = engine.now();
    let mut rate_limited = false;
    let outcome = match &call {
        Call::Settings { body } => {
            engine
                .outward
                .command_doors()
                .chat_settings(&token.token, &token.user_id, &room, body)
                .await
        }
        Call::Marker { description } => {
            engine
                .outward
                .command_doors()
                .marker(&token.token, &room, description.as_deref())
                .await
        }
        Call::Title(t) => {
            engine
                .outward
                .command_doors()
                .channel_info(&token.token, &room, Some(t), None)
                .await
        }
        Call::Game(_) => {
            engine
                .outward
                .command_doors()
                .channel_info(&token.token, &room, None, game_id.as_deref())
                .await
        }
        Call::Timeout {
            target,
            seconds,
            reason,
        } => {
            engine
                .outward
                .mod_doors()
                .timeout(
                    &token.token,
                    &token.user_id,
                    &room,
                    target,
                    *seconds,
                    reason.as_deref(),
                )
                .await
        }
        Call::Delete { message_id } => {
            engine
                .outward
                .mod_doors()
                .delete(&token.token, &token.user_id, &room, message_id)
                .await
        }
        Call::Pin {
            message_id,
            seconds,
        } => {
            engine
                .outward
                .mod_doors()
                .pin(&token.token, &token.user_id, &room, message_id, *seconds)
                .await
        }
        Call::Shoutout { target } => {
            engine
                .outward
                .mod_doors()
                .shoutout(&token.token, &token.user_id, &room, target)
                .await
        }
        Call::Whisper { to, text } => {
            let (result, w) = engine.outward.whisper_door().send(&send_as, to, text).await;
            // Twitch's "Sending too fast" text carries neither "429" nor "too many": the flag is what holds whispers 60 s.
            rate_limited = matches!(w, WhisperOutcome::RateLimited);
            whisper_outcome(&result, &w)
        }
    };
    settle(
        engine,
        job,
        run,
        &class,
        &call,
        &send_as,
        &token,
        &room,
        &login,
        started,
        outcome,
        rate_limited,
        &mut reserved,
    )
    .await;
}

/// A whisper's `Outcome` by the door table: `Sent` and `Unknown` keep the
/// claim; `RateLimited` (after its retries), `Refused` (before connect,
/// signed out, a 4xx) release. A 429's words carry "too many".
pub fn whisper_outcome(result: &Result<(), String>, outcome: &WhisperOutcome) -> DoorOutcome {
    match outcome {
        WhisperOutcome::Sent => DoorOutcome::Sent,
        WhisperOutcome::Unknown => DoorOutcome::Unknown,
        WhisperOutcome::RateLimited | WhisperOutcome::Refused(_) => {
            DoorOutcome::Refused(result.clone().err().unwrap_or_default())
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn settle(
    engine: &std::sync::Arc<Engine>,
    job: &Job,
    run: &mut JobRun,
    class: &str,
    call: &Call,
    send_as: &str,
    token: &super::outward::VerifiedToken,
    room: &str,
    login: &str,
    started: i64,
    outcome: DoorOutcome,
    rate_limited: bool,
    reserved: &mut Reserved<'_>,
) {
    let rule = &job.rule().rule;
    let chat_key = room_chat_key(job, login);
    // Read what the outcome says before it is moved into the match arms.
    let unauthorized_outcome = matches!(outcome, DoorOutcome::RefusedUnauthorized(_));
    match outcome {
        DoorOutcome::Sent | DoorOutcome::Unknown => {
            reserved.keep();
            let unknown = matches!(outcome, DoorOutcome::Unknown);
            run.classes
                .entry(class.to_string())
                .or_default()
                .sent_or_unknown = true;
            lock(&engine.guards).stamp(rule, class, &job.room, job.person(), started);
            lock(&engine.counts).note_fired(&rule.id, now_ms());
            engine.note_counts_changed();
            if unknown {
                run.stopped = true;
                return skip(engine, job, "Could not confirm it went through");
            }
            let line = match call {
                Call::Settings { .. } | Call::Marker { .. } | Call::Title(_) | Call::Game(_) => {
                    format!("Ran a command in #{login}")
                }
                Call::Timeout { seconds, .. } => format!(
                    "Timed out @{} for {seconds} seconds in #{login}",
                    job.facts.user_login
                ),
                Call::Delete { .. } => {
                    format!("Deleted @{}'s message in #{login}", job.facts.user_login)
                }
                Call::Pin { .. } => {
                    format!("Pinned @{}'s message in #{login}", job.facts.user_login)
                }
                Call::Shoutout { .. } => {
                    format!("Shouted out @{} in #{login}", job.facts.user_login)
                }
                Call::Whisper { to, .. } => format!("Whispered @{}", to.login),
            };
            engine.log(executor::entry(job, EntryKind::Acted, &chat_key, line));
        }
        DoorOutcome::Refused(text) | DoorOutcome::RefusedUnauthorized(text) => {
            // The reservation drops (given back) with `reserved`.
            run.stopped = true;
            if unauthorized_outcome {
                unauthorized(engine, send_as, token);
            }
            if rate_limited
                || text.contains("429")
                || text.to_ascii_lowercase().contains("too many")
            {
                // A 429 holds that kind in that chat 10 s (whispers 60 s).
                let mut holds = lock(&engine.door_holds);
                if matches!(call, Call::Whisper { .. }) {
                    holds.hold_whispers(engine.now() + WHISPER_429_MS);
                } else {
                    holds.hold_kind(class, room, engine.now() + HOLD_429_MS);
                }
            }
            skip(engine, job, &format!("Twitch refused it: {text}"));
        }
    }
}

/// A Kick actor's Timeout or Delete (C6): verify the owner's Twitch token (the
/// claim's bearer; never a Twitch verify of a Kick `send_as`, D277), check the
/// connected Kick account is `send_as` (P4's `kick_is_sender`, read again at
/// step 9 with no await before the call), apply Kick's target rail and the
/// 10-a-minute budget, run the final checks, claim, then P4's
/// `kick_moderation_call` (the Kick door `Outward::kick_moderate`).
async fn run_kick_moderation(
    engine: &std::sync::Arc<Engine>,
    job: &Job,
    action: &Action,
    plan: &ClassPlan,
    run: &mut JobRun,
) {
    let rule = &job.rule().rule;
    let class = plan.class.key();
    let login = job.login();
    let chat_key = room_chat_key(job, &login);
    let room = job.facts.room_id.clone();
    if job
        .facts
        .message_id
        .starts_with(crate::services::providers::kick::KICK_LOCAL_ID)
    {
        run.stopped = true;
        return skip(engine, job, "Skipped: no shared message id");
    }
    if let Action::Timeout { seconds, .. } = action {
        if *seconds == 0 || *seconds > KICK_TIMEOUT_MAX_S {
            return skip(engine, job, "Kick timeouts last up to 7 days");
        }
    }
    // Step 2: the owner's token (the claim's bearer) and the Kick account.
    let Some(budget) = job.left() else {
        return skip(engine, job, "Skipped: too late to answer");
    };
    let owner = match engine.verified(&rule.owner.id, budget).await {
        Ok(t) => t,
        Err(why) => return skip(engine, job, &token_words(&why, &rule.owner.login)),
    };
    if engine.outward.kick_account_id().as_deref() != Some(rule.send_as.id.as_str()) {
        return skip(engine, job, "Connect Kick to run this rule");
    }
    // The target rail and the budget, reserved under the lock.
    if let Err(why) = crate::services::account_automations::provider_facts::kick_target_rail(
        &engine.compiled(),
        &job.facts,
    ) {
        return skip(engine, job, why);
    }
    let now = engine.now();
    let mut reserved = Reserved {
        holds: &engine.door_holds,
        mod_stamp: None,
        shout: None,
        kept: false,
    };
    {
        let mut holds = lock(&engine.door_holds);
        if holds.kind_held(&class, &room, now) {
            return skip(
                engine,
                job,
                "Held for a few seconds: Kick asked for a pause",
            );
        }
        match holds.reserve_mod(now) {
            Ok(stamp) => reserved.mod_stamp = Some(stamp),
            Err(why) => return skip(engine, job, &why),
        }
    }
    // Step 5: the final checks. Step 7: the claim.
    let booked_away = engine.away();
    if let Err(reason) = executor::final_checks(engine, job, &login, &booked_away).await {
        return skip(engine, job, &reason);
    }
    if !run.classes.get(&class).is_some_and(|c| c.won) {
        match executor::claim_class(engine, job, plan, &login, &booked_away, None, &owner).await {
            Claimed::Skip(reason) => return skip(engine, job, &reason),
            Claimed::Won { call, .. } => {
                let entry = run.classes.entry(class.clone()).or_default();
                entry.won = true;
                entry.calls.extend(call);
            }
        }
    }
    // Step 9: the connected account, every local check, then the call: nothing
    // awaits between the account read and `kick_moderate`.
    if engine.outward.kick_account_id().as_deref() != Some(rule.send_as.id.as_str()) {
        return skip(engine, job, "Connect Kick to run this rule");
    }
    if let Err(reason) = executor::local_checks(engine, job, &chat_key, &booked_away, true) {
        return skip(engine, job, &reason);
    }
    if let Some(stamp) = reserved.mod_stamp {
        if !lock(&engine.door_holds).mod_still_ok(stamp, &class, &room, engine.now()) {
            return skip(
                engine,
                job,
                "Held for a few seconds: Kick asked for a pause",
            );
        }
    }
    let started = engine.now();
    let outcome = match executor::kick_moderation_call(engine, job, action).await {
        Ok(outcome) => outcome,
        Err(words) => return skip(engine, job, &words),
    };
    match outcome {
        KickOutcome::Sent { .. } | KickOutcome::Unknown => {
            reserved.keep();
            run.classes
                .entry(class.clone())
                .or_default()
                .sent_or_unknown = true;
            lock(&engine.guards).stamp(rule, &class, &job.room, job.person(), started);
            lock(&engine.counts).note_fired(&rule.id, now_ms());
            engine.note_counts_changed();
            if matches!(outcome, KickOutcome::Unknown) {
                run.stopped = true;
                return skip(engine, job, "Could not confirm it went through");
            }
            let line = match action {
                Action::Timeout { seconds, .. } => format!(
                    "Timed out @{} for {seconds} seconds in {login}",
                    job.facts.user_login
                ),
                _ => format!("Deleted @{}'s message in {login}", job.facts.user_login),
            };
            engine.log(executor::entry(job, EntryKind::Acted, &chat_key, line));
        }
        KickOutcome::Refused { reason, status } => {
            run.stopped = true;
            if let Some(hold) = crate::services::provider_pace::hold_for_status(status) {
                lock(&engine.door_holds).hold_kind(
                    &class,
                    &room,
                    engine.now() + hold.as_millis() as i64,
                );
            }
            skip(engine, job, &format!("Kick refused it: {reason}"));
        }
    }
}

/// An Announce's rails at step 2 and again at step 9 (sync): it needs a chat
/// message trigger (checked at save), `send_as` equal to the primary, and the
/// primary a moderator or the broadcaster in that room.
pub(crate) fn announce_preflight(engine: &Engine, job: &Job) -> Result<(), String> {
    let rule = &job.rule().rule;
    if engine.outward.primary_id().as_deref() != Some(rule.send_as.id.as_str()) {
        return Err(format!(
            "Waiting for @{} to be your main account",
            rule.send_as.login
        ));
    }
    let login = job.login();
    if job.facts.room_id != rule.send_as.id
        && !actor_may_moderate(engine.local.own_role(&rule.send_as.login, &login))
    {
        return Err("You're not a moderator in this chat".into());
    }
    Ok(())
}

/// Test mode: what a door action would do, never a claim, a token or a request.
pub(crate) async fn test_door_action(
    engine: &std::sync::Arc<Engine>,
    job: &Job,
    action: &Action,
    plan: &ClassPlan,
) {
    let words = match action {
        Action::Command { line, .. } => match executor::expand_plain(engine, job, line).await {
            Ok(expanded) => match parse_command(&expanded) {
                Ok(_) => format!("Would run {}", expanded.trim()),
                Err(e) => return skip(engine, job, &e),
            },
            Err(e) => return skip(engine, job, &e),
        },
        Action::Whisper { .. } | Action::Reply { .. } => {
            match executor::render(engine, job, action, &[]).await {
                Ok(text) => format!("Would whisper: {text}"),
                Err(e) => return skip(engine, job, &e),
            }
        }
        Action::Timeout { seconds, .. } => format!(
            "Would time @{} out for {seconds} seconds",
            job.facts.user_login
        ),
        Action::Delete { .. } => format!("Would delete @{}'s message", job.facts.user_login),
        Action::Pin { .. } => format!("Would pin @{}'s message", job.facts.user_login),
        Action::Shoutout { .. } => format!("Would shout out @{}", job.facts.user_login),
        _ => return,
    };
    let login = job.login();
    engine.log(executor::entry(
        job,
        EntryKind::Test,
        &room_chat_key(job, &login),
        words.clone(),
    ));
    lock(&engine.guards).stamp(
        &job.rule().rule,
        &plan.class.key(),
        &job.room,
        job.person(),
        engine.now(),
    );
    engine.chat_note(&job.rule().rule.id, &login, &words).await;
}

/// The control actions: JoinChat, LeaveChat, Log. (Wait is the executor's:
/// it re-queues the rest of the job.)
pub(crate) async fn run_control_action(
    engine: &std::sync::Arc<Engine>,
    job: &Job,
    action: &Action,
    text_for_log: &str,
) {
    let rule = &job.rule().rule;
    match action {
        Action::JoinChat { channel, .. } => {
            let login = channel
                .key
                .split_once(':')
                .map_or(channel.key.as_str(), |(_, l)| l)
                .to_string();
            if job.live {
                lock(&engine.action_joins).insert(login.clone());
                engine.request_refresh();
            }
            engine.log(executor::entry(
                job,
                if job.live {
                    EntryKind::Acted
                } else {
                    EntryKind::Test
                },
                &channel.key,
                format!(
                    "{} keeping #{login} joined",
                    if job.live { "Now" } else { "Would start" }
                ),
            ));
        }
        Action::LeaveChat { channel, .. } => {
            let login = channel
                .key
                .split_once(':')
                .map_or(channel.key.as_str(), |(_, l)| l)
                .to_string();
            if job.live {
                lock(&engine.action_joins).remove(&login);
                engine.request_refresh();
            }
            engine.log(executor::entry(
                job,
                if job.live {
                    EntryKind::Acted
                } else {
                    EntryKind::Test
                },
                &channel.key,
                format!(
                    "{} keeping #{login} joined",
                    if job.live { "Stopped" } else { "Would stop" }
                ),
            ));
        }
        Action::Log { to_file, .. } => {
            let line = format!(
                "{} [{}] {}",
                chrono::Local::now().format("%H:%M:%S"),
                rule.name,
                text_for_log
            );
            if *to_file && job.live {
                engine.logfile.append(&line);
            }
            engine.log(executor::entry(
                job,
                if job.live {
                    EntryKind::Acted
                } else {
                    EntryKind::Test
                },
                &job.facts.channel_key,
                format!("Logged: {text_for_log}"),
            ));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::automations::SenderRole;

    #[test]
    fn settings_bodies_are_todays_arms_json() {
        assert_eq!(
            settings_body(&CommandAction::Slow(60)),
            Some(json!({ "slow_mode": true, "slow_mode_wait_time": 60 }))
        );
        assert_eq!(
            settings_body(&CommandAction::SlowOff),
            Some(json!({ "slow_mode": false }))
        );
        assert_eq!(
            settings_body(&CommandAction::Followers(10)),
            Some(json!({ "follower_mode": true, "follower_mode_duration": 10 }))
        );
        assert_eq!(
            settings_body(&CommandAction::FollowersOff),
            Some(json!({ "follower_mode": false }))
        );
        assert_eq!(
            settings_body(&CommandAction::UniqueChat),
            Some(json!({ "unique_chat_mode": true }))
        );
        assert_eq!(
            settings_body(&CommandAction::EmoteOnlyOff),
            Some(json!({ "emote_mode": false }))
        );
        assert_eq!(settings_body(&CommandAction::Marker(None)), None);
    }

    #[test]
    fn a_category_needs_an_exact_row_with_a_real_id() {
        let rows = vec![
            json!({ "name": "Artifact", "id": "1" }),
            json!({ "name": "Art", "id": "509660" }),
        ];
        assert_eq!(exact_category(&rows, "art"), Some("509660".into()));
        assert_eq!(exact_category(&rows, "Arts"), None);
        assert_eq!(
            exact_category(&[json!({ "name": "Art", "id": "" })], "Art"),
            None
        );
        assert_eq!(
            exact_category(&[json!({ "name": "Art", "id": "0" })], "Art"),
            None
        );
    }

    #[test]
    fn the_target_rail_matches_the_plan() {
        let compiled = Compiled::empty();
        let mut facts = Facts {
            user_id: "5".into(),
            roles: vec![SenderRole::Viewer],
            ..Facts::default()
        };
        assert!(target_rail(&compiled, &facts, false).is_ok());
        for role in [
            SenderRole::Moderator,
            SenderRole::Vip,
            SenderRole::Broadcaster,
        ] {
            facts.roles = vec![role];
            assert!(target_rail(&compiled, &facts, false).is_err(), "{role:?}");
            assert!(
                target_rail(&compiled, &facts, true).is_ok(),
                "a VIP or moderator is a normal Shoutout target"
            );
        }
        facts.roles = vec![SenderRole::Viewer];
        facts.shared_partner = true;
        assert!(target_rail(&compiled, &facts, false).is_err());
    }

    #[test]
    fn the_mod_rate_reserves_under_the_lock_and_gives_back() {
        let mut h = Holds::default();
        let stamps: Vec<i64> = (0..10).map(|i| h.reserve_mod(i).unwrap()).collect();
        assert!(
            h.reserve_mod(11).is_err(),
            "the 11th in 60 s, even before any of the 10 sent"
        );
        h.give_back_mod(stamps[3]);
        assert!(
            h.reserve_mod(12).is_ok(),
            "a skipped or Refused action gave its slot back"
        );
        assert!(h.reserve_mod(60_100).is_ok(), "slots age out");
        assert!(
            !h.mod_still_ok(stamps[3], "timeout:1", "1", 13),
            "a given-back slot is no longer reserved"
        );
    }

    #[test]
    fn a_429_hold_is_per_kind_and_room() {
        let mut h = Holds::default();
        h.hold_kind("timeout:1", "1", 10_000);
        assert!(h.kind_held("timeout:1", "1", 5_000));
        assert!(!h.kind_held("timeout:1", "2", 5_000));
        assert!(!h.kind_held("delete:1", "1", 5_000));
        assert!(!h.kind_held("timeout:1", "1", 10_000));
    }

    #[test]
    fn shoutout_reservations_are_per_broadcaster_and_given_back() {
        let mut h = Holds::default();
        let prior = h.reserve_shout("room1", "t1", 0).unwrap();
        assert!(
            h.reserve_shout("room1", "t2", 60_000).is_err(),
            "any target inside 120 s"
        );
        assert!(
            h.reserve_shout("room1", "t1", 130_000).is_err(),
            "the same target inside an hour"
        );
        assert!(
            h.reserve_shout("room2", "t1", 1_000).is_ok(),
            "another room is not held"
        );
        h.give_back_shout("room1", "t1", prior);
        assert!(
            h.reserve_shout("room1", "t1", 1_000).is_ok(),
            "a skipped shoutout spent nothing"
        );
    }

    #[test]
    fn announce_slots_are_reserved_two_seconds_apart_per_room() {
        let mut h = Holds::default();
        assert_eq!(h.reserve_announce("r1", 1000), (0, 1000));
        assert_eq!(
            h.reserve_announce("r1", 1000),
            (2000, 3000),
            "the second waits for its own slot, reserved at once"
        );
        assert_eq!(h.reserve_announce("r1", 1000), (4000, 5000));
        let (_, slot) = h.reserve_announce("r3", 1000);
        h.give_back_announce("r3", slot);
        assert_eq!(
            h.reserve_announce("r3", 1000),
            (0, 1000),
            "a given-back slot is free again"
        );
        assert_eq!(
            h.reserve_announce("r2", 1000).0,
            0,
            "another room is not held"
        );
        assert_eq!(h.reserve_announce("r1", 9000).0, 0);
    }

    #[test]
    fn a_whisper_five_hundred_is_unknown_and_before_connect_is_refused() {
        use crate::services::whisper_send::{classify, SendFail};
        assert_eq!(
            whisper_outcome(
                &Ok(()),
                &classify(&Err(SendFail::Status(503, String::new())))
            ),
            DoorOutcome::Unknown
        );
        assert!(matches!(
            whisper_outcome(
                &Err("x".into()),
                &classify(&Err(SendFail::BeforeConnect("dns".into())))
            ),
            DoorOutcome::Refused(_)
        ));
        assert!(matches!(
            whisper_outcome(&Err("x".into()), &classify(&Err(SendFail::SignedOut))),
            DoorOutcome::Refused(_)
        ));
        assert!(matches!(
            whisper_outcome(
                &Err("x".into()),
                &classify(&Err(SendFail::Status(429, String::new())))
            ),
            DoorOutcome::Refused(_)
        ));
        assert_eq!(
            whisper_outcome(&Ok(()), &classify(&Ok(()))),
            DoorOutcome::Sent
        );
    }

    #[test]
    fn a_whisper_rules_reply_goes_to_the_sender() {
        let facts = Facts {
            user_id: "9".into(),
            user_login: "pal".into(),
            user_name: "Pal".into(),
            ..Facts::default()
        };
        let reply = Action::Reply {
            id: String::new(),
            texts: vec![],
            pick: Default::default(),
            thread: false,
        };
        assert_eq!(
            whisper_recipient(&reply, &facts, true).unwrap().user_id,
            "9"
        );
        assert!(whisper_recipient(&reply, &facts, false).is_none());
        assert!(is_door_action(&reply, true) && !is_door_action(&reply, false));
    }
}
