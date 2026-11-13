//! Jobs: what an admitted rule does, one action after another.
//!
//! Up to 8 jobs run at once; jobs for the same (send_as, chat) run in the
//! order they were admitted. Local actions (Notify) run as they come, in
//! Test and Live, and never claim. Outward actions are grouped by action
//! class (`claims::classes_for`): each first verifies both accounts'
//! tokens with Twitch, books its slot and waits for it, re-runs the final
//! checks, and only then the class claims the event on streamnook.app, so the
//! device that wins is a device that sends at once. Later actions of the class
//! reuse the won claim. A class gives its claim back only when none of its
//! sends was Sent or Unknown: an Unknown may have posted, and a claim given
//! back after a post would let another device answer the same event.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{Notify, Semaphore};

use crate::models::automations::{Action, Away, ChannelScope, Pick, RunState, Trigger};
use crate::services::chat_send_limiter::{self, Refusal, Ticket, REBOOK_TOLERANCE};
use crate::services::chat_service::SendOutcome;

use super::activity::{EntryKind, NewEntry};
use super::badges::{self, BadgeSeen, PostShape};
use super::claim_client::{CallKind, ClaimRequest, ClaimResult, ElementAnswer};
use super::claims::{self, CallInput, ClassPlan, Element};
use super::compile::{Compiled, CompiledRule, Mode};
use super::facts::{Facts, RowKind};
use super::speak::{self, SpeakRequest};
use super::guards::{self, SendCheck};
use super::notify::{self, NotifyRequest};
use super::outward::ChatSend;
use super::template::{self, Vars};
use super::verify::VerifiedToken;
use super::{lock, now_ms, sign_in, token_words, Engine};

/// Jobs that run at once.
pub const JOB_SLOTS: usize = 8;
/// A chat job may still act this long past its rule's longest wait.
const CHAT_ALLOWANCE: Duration = Duration::from_secs(5);
/// An edge job (follow, reward, poll, prediction, live; P3) may act this long
/// after the event arrived (admission is the arrival), so a slow socket or a
/// short queue never drops it. Its claim keeps the normal budget (at most 3 s).
pub const EDGE_ALLOWANCE: Duration = Duration::from_secs(120);
/// A post-claim re-book happens at most this many times.
const MAX_REBOOKS: u32 = 2;
/// A claim waits at most this long for its answer, and at least the floor.
const CLAIM_CAP: Duration = Duration::from_secs(3);
const CLAIM_FLOOR: Duration = Duration::from_millis(500);

/// One admitted rule's work for one event.
pub struct Job {
    pub(crate) compiled: Arc<Compiled>,
    /// The rule, as an index into `compiled.rules`.
    pub(crate) index: usize,
    pub(crate) facts: Arc<Facts>,
    /// A badge job's drops (empty for chat jobs) and their links by feed id.
    pub(crate) drops: Vec<BadgeSeen>,
    pub(crate) links: HashMap<String, Option<String>>,
    /// Live (sends) or Test (logs what it would do).
    pub(crate) live: bool,
    pub(crate) admitted_at: Instant,
    /// Steady ms at admission (the breaker's window).
    pub(crate) admitted_ms: i64,
    /// How long after admission it may still act.
    pub(crate) age_bound: Duration,
    /// The cancel generations it started under: the engine's, send_as's, the owner's.
    pub(crate) gen: u64,
    pub(crate) account_gens: (u64, u64),
    /// The rule's action classes for this event.
    pub(crate) plans: Vec<ClassPlan>,
    /// Actions that cannot run, with their words (a Send with no room id).
    pub(crate) unready: Vec<(usize, &'static str)>,
    /// The lanes admission found not cooling down.
    pub(crate) open_lanes: Vec<String>,
    /// The event's room (`claims::event_room`).
    pub(crate) room: String,
    /// The first action this pass runs (a job resumed after a Wait starts past it).
    pub(crate) start_at: usize,
    /// The claims and flags a Wait carried over to this pass.
    pub(crate) carry: Mutex<Option<JobRun>>,
}

impl Job {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        engine: &Engine,
        compiled: Arc<Compiled>,
        index: usize,
        facts: Arc<Facts>,
        drops: Vec<BadgeSeen>,
        links: HashMap<String, Option<String>>,
        live: bool,
        plans: Vec<ClassPlan>,
        unready: Vec<(usize, &'static str)>,
        open_lanes: Vec<String>,
        room: String,
        now_steady: i64,
    ) -> Job {
        let rule = &compiled.rules[index].rule;
        let age_bound = if facts.kind == RowKind::Own {
            EDGE_ALLOWANCE
        } else if drops.is_empty() {
            Duration::from_secs(u64::from(rule.limits.max_wait_seconds)) + CHAT_ALLOWANCE
        } else {
            let oldest = drops.iter().map(|d| d.arrived_ms).min().unwrap_or_else(now_ms);
            Duration::from_millis((badges::MAX_POST_AGE_MS - (now_ms() - oldest)).max(0) as u64)
        };
        let account_gens = engine.executor.account_gens(&rule.send_as.id, &rule.owner.id);
        Job {
            gen: engine.executor.gen(),
            account_gens,
            compiled,
            index,
            facts,
            drops,
            links,
            live,
            admitted_at: Instant::now(),
            admitted_ms: now_steady,
            age_bound,
            plans,
            unready,
            open_lanes,
            room,
            start_at: 0,
            carry: Mutex::new(None),
        }
    }

    pub(crate) fn rule(&self) -> &CompiledRule {
        &self.compiled.rules[self.index]
    }

    pub(crate) fn plan_of(&self, action: usize) -> Option<&ClassPlan> {
        self.plans.iter().find(|p| p.actions.contains(&action))
    }

    fn badge(&self) -> bool {
        !self.drops.is_empty()
    }

    /// A P3 event: not a chat row. It posts in the owner's own channel.
    fn edge(&self) -> bool {
        self.facts.kind == RowKind::Own
    }

    /// The person the event came from (none for a badge drop).
    pub(crate) fn person(&self) -> Option<&str> {
        (!self.badge() && !self.facts.user_id.is_empty()).then_some(self.facts.user_id.as_str())
    }

    /// The chat the event is in: the owner's for a badge drop.
    pub(crate) fn login(&self) -> String {
        if self.badge() || self.edge() {
            self.rule().owner_login.clone()
        } else {
            self.facts.channel_login.clone()
        }
    }

    /// The channel the event is about, for `{title}`, `{game}` and `{uptime}`
    /// (a live change names a channel that is not the owner's).
    fn info_login(&self) -> String {
        if self.badge() {
            self.rule().owner_login.clone()
        } else {
            self.facts.channel_login.clone()
        }
    }

    /// Time left for a slot wait. An edge job's 120 s age bound would let a
    /// booking wait that long; its wait stays the rule's own (`max_wait` plus
    /// 5 s), inside the age bound.
    fn slot_left(&self) -> Option<Duration> {
        let left = self.left()?;
        if self.edge() {
            let own = Duration::from_secs(u64::from(self.rule().rule.limits.max_wait_seconds)) + CHAT_ALLOWANCE;
            Some(left.min(own))
        } else {
            Some(left)
        }
    }

    pub(crate) fn lane(&self) -> (String, String) {
        (self.rule().rule.send_as.id.clone(), self.login())
    }

    pub(crate) fn accounts(&self) -> Vec<String> {
        let rule = &self.rule().rule;
        let mut ids = vec![rule.send_as.id.clone()];
        if rule.owner.id != rule.send_as.id {
            ids.push(rule.owner.id.clone());
        }
        ids
    }

    /// Time left before the age bound, if any.
    pub(crate) fn left(&self) -> Option<Duration> {
        self.age_bound.checked_sub(self.admitted_at.elapsed()).filter(|d| !d.is_zero())
    }

    /// The holder's longest time to send, for the release queue.
    fn settle_ms(&self) -> u64 {
        u64::from(claims::settle_ms(&self.rule().rule))
    }

    fn uses_away(&self) -> bool {
        let rule = &self.rule().rule;
        rule.only_if.only_away || rule.actions.iter().any(|a| a.lines().iter().any(|l| template::uses_away(&l.text)))
    }
}

/// The executor's shared state.
pub struct Executor {
    slots: Arc<Semaphore>,
    gen: AtomicU64,
    account_gens: Mutex<HashMap<String, u64>>,
    inflight: Mutex<HashMap<String, usize>>,
    idle: Notify,
    cancel: Notify,
    lanes: Mutex<HashMap<(String, String), VecDeque<Job>>>,
    submitted: AtomicU64,
}

impl Default for Executor {
    fn default() -> Self {
        Executor::new()
    }
}

impl Executor {
    pub fn new() -> Executor {
        Executor {
            slots: Arc::new(Semaphore::new(JOB_SLOTS)),
            gen: AtomicU64::new(0),
            account_gens: Mutex::new(HashMap::new()),
            inflight: Mutex::new(HashMap::new()),
            idle: Notify::new(),
            cancel: Notify::new(),
            lanes: Mutex::new(HashMap::new()),
            submitted: AtomicU64::new(0),
        }
    }

    pub fn gen(&self) -> u64 {
        self.gen.load(Ordering::SeqCst)
    }

    /// Stop every job in flight (Off, the background, a primary switch).
    pub fn bump(&self) {
        self.gen.fetch_add(1, Ordering::SeqCst);
        self.cancel.notify_waiters();
    }

    /// Stop every job sending as or owned by one of `ids`.
    pub fn cancel_accounts(&self, ids: &[String]) {
        {
            let mut gens = lock(&self.account_gens);
            for id in ids {
                *gens.entry(id.clone()).or_insert(0) += 1;
            }
        }
        self.cancel.notify_waiters();
    }

    fn account_gens(&self, send_as: &str, owner: &str) -> (u64, u64) {
        let gens = lock(&self.account_gens);
        (gens.get(send_as).copied().unwrap_or(0), gens.get(owner).copied().unwrap_or(0))
    }

    /// A newer generation stopped the job.
    pub(crate) fn canceled(&self, job: &Job) -> bool {
        let rule = &job.rule().rule;
        job.gen != self.gen() || job.account_gens != self.account_gens(&rule.send_as.id, &rule.owner.id)
    }

    pub fn submitted(&self) -> u64 {
        self.submitted.load(Ordering::SeqCst)
    }

    /// No job queued or running.
    pub fn is_idle(&self) -> bool {
        lock(&self.lanes).is_empty() && lock(&self.inflight).is_empty()
    }

    /// Queue a job behind the jobs of its (send_as, chat).
    pub(crate) fn submit(&self, engine: &Arc<Engine>, job: Job) {
        self.submitted.fetch_add(1, Ordering::SeqCst);
        {
            let mut inflight = lock(&self.inflight);
            for id in job.accounts() {
                *inflight.entry(id).or_insert(0) += 1;
            }
        }
        let lane = job.lane();
        let start = {
            let mut lanes = lock(&self.lanes);
            match lanes.get_mut(&lane) {
                Some(queue) => {
                    queue.push_back(job);
                    false
                }
                None => {
                    lanes.insert(lane.clone(), VecDeque::from([job]));
                    true
                }
            }
        };
        if start {
            let task = engine.clone();
            engine.spawn(async move { run_lane(task, lane).await });
        }
    }

    fn next_in_lane(&self, lane: &(String, String)) -> Option<Job> {
        let mut lanes = lock(&self.lanes);
        let job = lanes.get_mut(lane).and_then(VecDeque::pop_front);
        if job.is_none() {
            lanes.remove(lane);
        }
        job
    }

    fn done(&self, job: &Job) {
        {
            let mut inflight = lock(&self.inflight);
            for id in job.accounts() {
                if let Some(n) = inflight.get_mut(&id) {
                    *n -= 1;
                    if *n == 0 {
                        inflight.remove(&id);
                    }
                }
            }
        }
        self.idle.notify_waiters();
    }

    /// Resolves once no job sending as or owned by `ids` is in flight.
    pub(crate) async fn idle_for(&self, ids: &[String]) {
        loop {
            let notified = self.idle.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if !ids.iter().any(|id| lock(&self.inflight).contains_key(id)) {
                return;
            }
            notified.await;
        }
    }

    /// Sleep `dur` unless a newer generation stops the job first. True: the
    /// job may go on.
    pub(crate) async fn pause(&self, job: &Job, dur: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + dur;
        loop {
            let notified = self.cancel.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.canceled(job) {
                return false;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => return !self.canceled(job),
                _ = notified => {}
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn hold_account_for_test(&self, id: &str) {
        *lock(&self.inflight).entry(id.to_string()).or_insert(0) += 1;
    }

    #[cfg(test)]
    pub(crate) fn release_account_for_test(&self, id: &str) {
        let mut inflight = lock(&self.inflight);
        if let Some(n) = inflight.get_mut(id) {
            *n -= 1;
            if *n == 0 {
                inflight.remove(id);
            }
        }
        drop(inflight);
        self.idle.notify_waiters();
    }
}

async fn run_lane(engine: Arc<Engine>, lane: (String, String)) {
    while let Some(job) = engine.executor.next_in_lane(&lane) {
        let permit = engine.executor.slots.clone().acquire_owned().await;
        run_job(&engine, &job).await;
        drop(permit);
        engine.executor.done(&job);
    }
}

/// A won call: given back if nothing of its class went out.
#[derive(Clone, Debug)]
pub(crate) struct WonCall {
    pub call_id: String,
    pub claimed_at: Instant,
}

#[derive(Default)]
pub(crate) struct ClassRun {
    pub(crate) calls: Vec<WonCall>,
    /// A send of the class was Sent or Unknown: the claims are kept.
    pub(crate) sent_or_unknown: bool,
    /// The class claimed (or a cached win stands in for a claim).
    pub(crate) won: bool,
}

#[derive(Default)]
pub(crate) struct JobRun {
    pub(crate) classes: HashMap<String, ClassRun>,
    /// Classes already logged as cooling down.
    pub(crate) cooling: HashSet<String>,
    /// A Refused or Unknown send stopped every later outward action.
    pub(crate) stopped: bool,
    /// A Wait ended the pass: the next action to run and the seconds to wait first.
    pub(crate) resume_at: Option<(usize, u32)>,
}

/// Run every action of `job` in rule order, then give back what it won and
/// never used.
pub(crate) async fn run_job(engine: &Arc<Engine>, job: &Job) {
    let mut run = job.carry.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default();
    for (index, action) in job.rule().rule.actions.iter().enumerate().skip(job.start_at) {
        if engine.executor.canceled(job) {
            break;
        }
        if let Some((_, words)) = job.unready.iter().find(|(i, _)| *i == index) {
            skip(engine, job, words);
            continue;
        }
        match action {
            Action::Notify { .. } => {
                if job.open_lanes.iter().any(|l| l == guards::LOCAL_LANE) {
                    notify_action(engine, job, action).await;
                }
            }
            Action::Speak { .. } => {
                if job.open_lanes.iter().any(|l| l == guards::LOCAL_LANE) {
                    speak_action(engine, job, action).await;
                }
            }
            // A Highlight is not run per event: compile hands it to chat_rules.
            Action::Highlight { .. } => {}
            Action::Announce { .. } => {
                // Ticketed `post:` booking and the full chat path; the door is the
                // engine's Announce door at step 10 (`outward_action` branches on the action).
                if run.stopped {
                    continue;
                }
                let Some(plan) = job.plan_of(index).cloned() else { continue };
                if !job.open_lanes.contains(&plan.class.key()) {
                    if run.cooling.insert(plan.class.key()) {
                        skip(engine, job, guards::COOLING);
                    }
                    continue;
                }
                if job.live {
                    outward_action(engine, job, action, &plan, &mut run).await;
                } else {
                    test_action(engine, job, action, &plan).await;
                }
            }
            // The door classes, by name (a guard would not count toward exhaustiveness):
            // Command, Whisper and the moderation actions, and a Whisper rule's Reply, which
            // whispers back and never reaches the chat path (C5).
            Action::Command { .. } | Action::Whisper { .. } | Action::Timeout { .. } | Action::Delete { .. } | Action::Pin { .. } | Action::Shoutout { .. } => {
                door_arm(engine, job, index, action, &mut run).await;
            }
            Action::Reply { .. } if job.rule().rule.is_whisper_rule() => {
                door_arm(engine, job, index, action, &mut run).await;
            }
            Action::JoinChat { .. } | Action::LeaveChat { .. } | Action::Log { .. } => {
                let text = match action {
                    Action::Log { .. } => job.facts.text.clone(),
                    _ => String::new(),
                };
                super::doors::run_control_action(engine, job, action, &text).await;
            }
            Action::Wait { seconds, .. } => {
                if !job.live {
                    engine.log(entry(job, EntryKind::Test, &chat_key_of(&job.login()), format!("Would wait {seconds} seconds")));
                    continue;
                }
                // Release the permit and the order slot: the rest of the job is
                // re-queued after the wait with a fresh `admitted_at`; its won
                // claims stand and guards and holds run again when it ends.
                run.resume_at = Some((index + 1, *seconds));
                break;
            }
            Action::Reply { .. } | Action::Send { .. } => {
                if run.stopped {
                    continue;
                }
                let Some(plan) = job.plan_of(index).cloned() else { continue };
                let key = plan.class.key();
                if !job.open_lanes.contains(&key) {
                    if run.cooling.insert(key) {
                        skip(engine, job, guards::COOLING);
                    }
                    continue;
                }
                if job.live {
                    outward_action(engine, job, action, &plan, &mut run).await;
                } else {
                    test_action(engine, job, action, &plan).await;
                }
            }
        }
    }
    if let Some((next, seconds)) = run.resume_at.take() {
        resume_after_wait(engine, job, run, next, seconds);
        return;
    }
    finish(engine, job, run).await;
    engine.announce_status();
}

/// One door-class action of `run_job` (Command, Whisper, the moderation
/// actions, a Whisper rule's Reply): the lane's cooling check, then the live
/// door path or the Test line.
async fn door_arm(engine: &Arc<Engine>, job: &Job, index: usize, action: &Action, run: &mut JobRun) {
    if run.stopped {
        return;
    }
    let Some(plan) = job.plan_of(index).cloned() else { return };
    if !job.open_lanes.contains(&plan.class.key()) {
        if run.cooling.insert(plan.class.key()) {
            skip(engine, job, guards::COOLING);
        }
        return;
    }
    if job.live {
        super::doors::run_door_action(engine, job, action, &plan, run).await;
    } else {
        super::doors::test_door_action(engine, job, action, &plan).await;
    }
}

/// A Wait ended the pass: after `seconds` (or a cancel) the rest of the job
/// runs as a new job in the same lane, with a fresh `admitted_at`, its claims
/// carried and the holds and final checks run again (not admission: the original
/// admission stands). A cancel or a switched-off rule gives the claims back.
fn resume_after_wait(engine: &Arc<Engine>, job: &Job, run: JobRun, next: usize, seconds: u32) {
    let engine = engine.clone();
    let spawner = engine.clone();
    let compiled = job.compiled.clone();
    let (index, facts, live, plans, room) = (job.index, job.facts.clone(), job.live, job.plans.clone(), job.room.clone());
    let (drops, links, unready) = (job.drops.clone(), job.links.clone(), job.unready.clone());
    let probe = Job::new(&engine, compiled.clone(), index, facts.clone(), drops.clone(), links.clone(), live, plans.clone(), unready.clone(), job.open_lanes.clone(), room.clone(), engine.now());
    spawner.spawn(async move {
        if !engine.executor.pause(&probe, Duration::from_secs(u64::from(seconds))).await {
            finish(&engine, &probe, run).await;
            return;
        }
        // The original admission stands (M10): a second `admit` would count a second
        // fire against the rule's rate and meet the job's own stamps as a cooldown.
        // After the wait only the holds and the final checks run, and every action
        // runs them again at its own step 5 and 9 (`local_checks` reads `held`,
        // the rule's mode and the cancel generation). Nothing re-stamps.
        // The rule's mode NOW (a recompile may have switched it off or to Test during
        // the wait), never the pre-Wait snapshot's.
        let rule_id = compiled.rules[index].rule.id.clone();
        let wanted = if live { super::compile::Mode::Live } else { super::compile::Mode::Test };
        if engine.compiled().rule(&rule_id).and_then(|c| c.mode) != Some(wanted) || engine.executor.canceled(&probe) {
            finish(&engine, &probe, run).await;
            return;
        }
        let mut job = Job::new(&engine, compiled, index, facts, drops, links, live, plans, unready, probe.open_lanes.clone(), room, engine.now());
        job.start_at = next;
        *job.carry.lock().unwrap_or_else(|e| e.into_inner()) = Some(run);
        engine.executor.submit(&engine, job);
    });
}

/// Give back the claim of every class none of whose sends was Sent or Unknown.
async fn finish(engine: &Arc<Engine>, job: &Job, run: JobRun) {
    let owner = job.rule().rule.owner.id.clone();
    let mut waits = Vec::new();
    for class in run.classes.into_values() {
        if class.sent_or_unknown {
            continue;
        }
        for call in class.calls {
            waits.push(engine.claims.release(&owner, &call.call_id, call.claimed_at, job.settle_ms()));
        }
    }
    let all = futures::future::join_all(waits);
    let _ = tokio::time::timeout(engine.timings.job_release_wait, all).await;
}

// ---- texts ----------------------------------------------------------------------

/// Where an outward action posts: (login, room id, reply parent).
fn target(job: &Job, action: &Action) -> (String, String, Option<String>) {
    let event_room = if job.badge() || job.edge() { job.rule().rule.owner.id.clone() } else { job.facts.room_id.clone() };
    match action {
        Action::Reply { thread, .. } => {
            let parent = (*thread && !job.facts.message_id.is_empty()).then(|| job.facts.message_id.clone());
            (job.login(), event_room, parent)
        }
        Action::Send { to: Some(chat), .. } => {
            let login = chat.key.split_once(':').map_or(chat.key.as_str(), |(_, l)| l).to_string();
            (login, chat.room_id.clone(), None)
        }
        _ => (job.login(), event_room, None),
    }
}

pub(crate) fn chat_key_of(login: &str) -> String {
    crate::services::chat_rules::channel_filter_key("twitch", login)
}

/// The values a rule's text can name for this job.
async fn vars(engine: &Arc<Engine>, job: &Job, drops: &[&BadgeSeen]) -> Vars {
    let cr = job.rule();
    let mut vars = Vars::from_facts(&job.facts);
    vars.me = cr.send_as_name.clone();
    vars.count = lock(&engine.counts).count(&cr.rule.id);
    vars.set_away(engine.away().as_ref(), now_ms());
    vars.set_local(chrono::Local::now().naive_local());
    if let Some(info) = engine.local.channel_info(&job.info_login()).await {
        vars.title = info.title.unwrap_or_default();
        vars.game = info.game_name.unwrap_or_default();
        let started = info
            .started_at
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.timestamp_millis());
        if let Some(started_ms) = started {
            vars.uptime = template::uptime_words(started_ms, now_ms());
        }
    }
    if !drops.is_empty() {
        vars.set_batch(drops);
        if let [one] = drops {
            vars.badge_link = job.links.get(&one.feed_id).cloned().flatten().unwrap_or_default();
        }
    }
    vars
}

/// A text's variables filled, with no line picking and no cleaning of a
/// command's slash: for a Command line, a Timeout reason.
pub(crate) async fn expand_plain(engine: &Arc<Engine>, job: &Job, text: &str) -> Result<String, String> {
    template::outward_text_ok(&job.rule().rule, &job.facts, text).map_err(str::to_string)?;
    let vars = vars(engine, job, &[]).await;
    Ok(template::expand(text, &vars))
}

/// The action's text for this event: one line picked (the batch line for a
/// badge batch of several), variables filled, cleaned.
pub(crate) async fn render(engine: &Arc<Engine>, job: &Job, action: &Action, won: &[usize]) -> Result<String, String> {
    let cr = job.rule();
    let drops: Vec<&BadgeSeen> = won.iter().filter_map(|i| job.drops.get(*i)).collect();
    let lines = action.lines();
    let batch_text = match (&cr.rule.when, badges::post_shape(&drops)) {
        (Trigger::BadgeDrop { batch_text, .. }, Some(PostShape::Many { .. })) if !batch_text.trim().is_empty() => {
            Some(batch_text.clone())
        }
        _ => None,
    };
    let text = match batch_text {
        Some(text) => text,
        None if matches!(action, Action::Announce { .. }) => match action {
            Action::Announce { text, .. } => text.clone(),
            _ => String::new(),
        },
        None => {
            if lines.is_empty() {
                return Err("Skipped: nothing to send".into());
            }
            let pick = match action {
                Action::Reply { pick, .. } | Action::Send { pick, .. } => *pick,
                _ => Pick::Random,
            };
            let index = lock(&engine.counts).pick_line(&cr.rule.id, action.id(), lines.len(), pick, rand::random::<u64>());
            lines[index.min(lines.len() - 1)].text.clone()
        }
    };
    template::outward_text_ok(&cr.rule, &job.facts, &text).map_err(str::to_string)?;
    let vars = vars(engine, job, &drops).await;
    template::clean(&template::expand(&text, &vars), cr.rule.vary).map_err(|e| e.words().to_string())
}

pub(crate) fn entry(job: &Job, kind: EntryKind, channel: &str, text: String) -> NewEntry {
    let rule = &job.rule().rule;
    NewEntry { kind, rule_id: rule.id.clone(), rule_name: rule.name.clone(), channel: channel.to_string(), text }
}

pub(crate) fn skip(engine: &Arc<Engine>, job: &Job, reason: &str) {
    engine.log(entry(job, EntryKind::Skipped, &chat_key_of(&job.login()), reason.to_string()));
}

// ---- local and test actions ---------------------------------------------------------

async fn notify_action(engine: &Arc<Engine>, job: &Job, action: &Action) {
    let Action::Notify { title, body, toast, system, sound, volume, .. } = action else { return };
    let all: Vec<usize> = (0..job.drops.len()).collect();
    let drops: Vec<&BadgeSeen> = all.iter().filter_map(|i| job.drops.get(*i)).collect();
    let vars = vars(engine, job, &drops).await;
    let request = NotifyRequest {
        rule_id: Some(job.rule().rule.id.clone()),
        title: template::expand(title, &vars),
        body: template::expand(body, &vars),
        toast: *toast,
        system: *system,
        sound: sound.clone(),
        volume: *volume,
        channel: job.login(),
        settings_link: false,
    };
    notify::deliver(engine, request).await;
    lock(&engine.guards).stamp(&job.rule().rule, guards::LOCAL_LANE, &job.room, job.person(), engine.now());
}

/// A Speak action: local, no claim, runs in Test and Live. The chatter-text
/// rule applies to what is read aloud; the line is expanded, cleaned and read
/// by one claimed player (`speak::deliver`). It never posts.
async fn speak_action(engine: &Arc<Engine>, job: &Job, action: &Action) {
    let Action::Speak { text, on_stream, .. } = action else { return };
    if let Err(why) = template::outward_text_ok(&job.rule().rule, &job.facts, text) {
        return skip(engine, job, why);
    }
    let vars = vars(engine, job, &[]).await;
    let line = template::clean_speech(&template::expand(text, &vars));
    if line.is_empty() {
        return skip(engine, job, "Skipped: nothing to say once the variables were filled in");
    }
    speak::deliver(
        engine,
        SpeakRequest { rule_id: Some(job.rule().rule.id.clone()), text: line, on_stream: *on_stream, channel: job.info_login() },
    )
    .await;
    lock(&engine.guards).stamp(&job.rule().rule, guards::LOCAL_LANE, &job.room, job.person(), engine.now());
}

async fn test_action(engine: &Arc<Engine>, job: &Job, action: &Action, plan: &ClassPlan) {
    let all: Vec<usize> = (0..job.drops.len()).collect();
    let text = match render(engine, job, action, &all).await {
        Ok(text) => text,
        Err(reason) => return skip(engine, job, &reason),
    };
    let (login, _, _) = target(job, action);
    let line = match action {
        Action::Reply { .. } => format!("Would reply: {text}"),
        _ => format!("Would post: {text}"),
    };
    engine.log(entry(job, EntryKind::Test, &chat_key_of(&login), line.clone()));
    lock(&engine.guards).stamp(&job.rule().rule, &plan.class.key(), &job.room, job.person(), engine.now());
    engine.chat_note(&job.rule().rule.id, &login, &line).await;
}

// ---- the outward path -------------------------------------------------------------------

/// A reserved Announce slot that gives itself back on drop unless an Announce
/// went out (Sent or Unknown keep it).
struct AnnounceSlot<'a> {
    holds: &'a Mutex<super::doors::Holds>,
    room: String,
    slot: i64,
    kept: bool,
}

impl Drop for AnnounceSlot<'_> {
    fn drop(&mut self) {
        if !self.kept {
            lock(self.holds).give_back_announce(&self.room, self.slot);
        }
    }
}

/// An Announce's send: the engine door, settling the booked ticket like the
/// chat door (`record_ticket` on Sent and Unknown, `cancel_booking` on Refused).
/// The room's 2 s spacing was reserved under the lock and waited out before
/// booking (`outward_action`, below), so nothing sleeps here, after step 9's
/// final checks; the rails (send_as is the primary, moderator or broadcaster
/// there) are read once more, synchronously, right before the request.
async fn announce_send(engine: &Arc<Engine>, job: &Job, ticket: &mut Ticket, send: &ChatSend, color: &str) -> SendOutcome {
    if let Err(reason) = super::doors::announce_preflight(engine, job) {
        chat_send_limiter::cancel_booking(ticket);
        return SendOutcome::Refused(reason);
    }
    chat_send_limiter::mark_sending(ticket);
    let outcome = engine.outward.mod_doors().announce(&send.token, &send.send_as, &send.room_id, &send.text, color).await;
    use crate::services::twitch_service::DoorOutcome;
    match outcome {
        DoorOutcome::Sent => {
            chat_send_limiter::record_ticket(ticket);
            SendOutcome::Sent { message_id: None }
        }
        DoorOutcome::Unknown => {
            chat_send_limiter::record_ticket(ticket);
            SendOutcome::Unknown
        }
        DoorOutcome::Refused(text) => {
            chat_send_limiter::cancel_booking(ticket);
            SendOutcome::Refused(text)
        }
        DoorOutcome::RefusedUnauthorized(_) => {
            chat_send_limiter::cancel_booking(ticket);
            SendOutcome::Refused(format!("{} (401)", crate::services::chat_service::UNAUTHORIZED))
        }
    }
}

/// Book a slot for `text` on `send_as`'s limiter (the main account's own when
/// it is the main account); with `vary`, a duplicate books once more with the
/// invisible variation.
fn book(engine: &Engine, job: &Job, login: &str, text: &str) -> Result<(Duration, Ticket, String), String> {
    let rule = &job.rule().rule;
    let main = engine.main_id();
    let max_wait = job.slot_left().ok_or("Skipped: too late to answer")?;
    match chat_send_limiter::acquire_within_as(&rule.send_as.id, main.as_deref(), login, text, max_wait) {
        Ok((wait, ticket)) => Ok((wait, ticket, text.to_string())),
        Err(Refusal::Duplicate) if rule.vary => {
            let varied = template::varied(text);
            chat_send_limiter::acquire_within_as(&rule.send_as.id, main.as_deref(), login, &varied, max_wait)
                .map(|(wait, ticket)| (wait, ticket, varied))
                .map_err(|r| r.text())
        }
        Err(r) => Err(r.text()),
    }
}

/// The final checks that read only local state (no await), in one place so
/// step 9 runs them with no await between them and the request.
pub(crate) fn local_checks(engine: &Arc<Engine>, job: &Job, chat_key: &str, booked_away: &Option<Away>, live_now: bool) -> Result<(), String> {
    let rule = &job.rule().rule;
    if engine.executor.canceled(job) {
        return Err("Stopped".into());
    }
    if engine.run_state() != RunState::On {
        return Err("Skipped: automations aren't on".into());
    }
    let compiled = engine.compiled();
    match compiled.rule(&rule.id) {
        Some(now) if now.mode == Some(Mode::Live) => {}
        Some(now) if now.mode.is_some() => return Err("Skipped: the rule was set to Test".into()),
        _ => return Err("Skipped: the rule was turned off".into()),
    }
    if !engine.foreground() {
        return Err("Skipped: StreamNook went to the background".into());
    }
    if job.left().is_none() {
        return Err("Skipped: too late to answer".into());
    }
    if lock(&engine.guards).held(&rule.send_as.id, chat_key, engine.now()) {
        return Err(guards::HELD_HERE.into());
    }
    if !live_now {
        return Err("Skipped: the channel isn't live".into());
    }
    if job.uses_away() {
        let now = engine.away();
        let moved = match (booked_away, &now) {
            (Some(was), Some(is)) => was.message != is.message,
            (None, None) => false,
            _ => true,
        };
        if moved || (rule.only_if.only_away && now.is_none()) {
            return Err("Skipped: You're back".into());
        }
    }
    for account in [&rule.send_as, &rule.owner] {
        if engine.is_leaving(&account.id) {
            return Err("Skipped: Account being removed".into());
        }
        if compiled.login(&account.id).is_none() {
            return Err(sign_in(&account.login));
        }
    }
    Ok(())
}

/// Every final check (step 5): the awaits (where the chat is open, liveness)
/// first, then the local reads.
pub(crate) async fn final_checks(engine: &Arc<Engine>, job: &Job, login: &str, booked_away: &Option<Away>) -> Result<(), String> {
    match &job.rule().rule.only_if.channels {
        ChannelScope::AnyOpen {} => {
            let adopted = lock(&engine.rec).adopted.contains_key(login);
            let open = adopted || engine.local.holders(login).await.iter().any(|h| h != super::AUTOMATIONS_LABEL);
            if !open {
                return Err("Skipped: not open in a window".into());
            }
        }
        ChannelScope::Watching {} => {
            if engine.local.current_watch().as_deref() != Some(login) {
                return Err("Skipped: not watching a stream".into());
            }
        }
        _ => {}
    }
    let live_now = !job.rule().rule.only_if.only_live || engine.local.is_live(login).await == Some(true);
    local_checks(engine, job, &chat_key_of(login), booked_away, live_now)
}

/// The verified tokens for the job's sender and owner. A token is used for an
/// account only when Twitch says it is that account's.
pub(crate) async fn verify_accounts(engine: &Arc<Engine>, job: &Job) -> Result<(VerifiedToken, VerifiedToken), String> {
    let rule = &job.rule().rule;
    // The checks spend the job's own remaining age, never more.
    let budget = job.left().ok_or("Skipped: too late to answer")?;
    let send_as = engine.verified(&rule.send_as.id, budget).await.map_err(|why| token_words(&why, &rule.send_as.login))?;
    let owner = if rule.owner.id == rule.send_as.id {
        send_as.clone()
    } else {
        engine.verified(&rule.owner.id, budget).await.map_err(|why| token_words(&why, &rule.owner.login))?
    };
    Ok((send_as, owner))
}

/// Steps 4 to 6: wait for the slot, re-run the final checks, and re-book
/// while a message you typed moved the chat's spacing.
async fn settle_slot(
    engine: &Arc<Engine>,
    job: &Job,
    login: &str,
    text: &str,
    ticket: &mut Ticket,
    mut wait: Duration,
    booked_away: &Option<Away>,
) -> Result<(), String> {
    loop {
        if !engine.executor.pause(job, wait).await {
            return Err("Stopped".into());
        }
        final_checks(engine, job, login, booked_away).await?;
        if !chat_send_limiter::spacing_moved(ticket, Instant::now()) {
            return Ok(());
        }
        chat_send_limiter::cancel_booking(ticket);
        let left = job.slot_left().ok_or("Skipped: too late to answer")?;
        let main = engine.main_id();
        let (fresh_wait, fresh) =
            chat_send_limiter::acquire_within_as(&job.rule().rule.send_as.id, main.as_deref(), login, text, left)
                .map_err(|r| r.text())?;
        *ticket = fresh;
        wait = fresh_wait;
    }
}

/// Cancel and book again, then settle the new slot.
async fn rebook(
    engine: &Arc<Engine>,
    job: &Job,
    login: &str,
    text: &str,
    ticket: &mut Ticket,
    booked_away: &Option<Away>,
) -> Result<(), String> {
    chat_send_limiter::cancel_booking(ticket);
    let left = job.slot_left().ok_or("Skipped: too late to answer")?;
    let main = engine.main_id();
    let (wait, fresh) =
        chat_send_limiter::acquire_within_as(&job.rule().rule.send_as.id, main.as_deref(), login, text, left)
            .map_err(|r| r.text())?;
    *ticket = fresh;
    settle_slot(engine, job, login, text, ticket, wait, booked_away).await
}

/// What a claim decided for the class.
pub(crate) enum Claimed {
    /// The class won (for a badge job: these drops).
    Won { call: Option<WonCall>, drops: Vec<usize> },
    /// The class skips with these words.
    Skip(String),
}

/// How one answer reads for the class.
#[derive(Debug, PartialEq)]
pub(crate) enum Reading {
    /// The listed event elements (by their order among the events) won, and every rail.
    Won(Vec<usize>),
    /// A windowed rail was held by another device until `free_at_ms`.
    Cooldown { holder: String, free_at_ms: Option<i64> },
    Pace,
    Conversation,
    /// The whisper budget: this person was whispered in the last hour.
    WhisperHour,
    /// The whisper budget: the people-a-day count is reached.
    /// The server's `retry_after_ms` from this very answer.
    WhisperDay { retry_after_ms: Option<i64> },
    /// Every event element is held by another session.
    Held { holder: String, same_device: bool, age_ms: i64, settle_ms: i64 },
}

/// Read an answer against the call's elements. Event elements answer by
/// their position in the request; their order among the events is the drop
/// order of a badge batch.
pub(crate) fn read_answer(elements: &[Element], answers: &[ElementAnswer]) -> Reading {
    let event_positions: Vec<usize> = elements.iter().enumerate().filter(|(_, e)| e.is_event()).map(|(i, _)| i).collect();
    let by_index: HashMap<usize, &ElementAnswer> = answers.iter().map(|a| (a.index, a)).collect();
    // The whisper budget answers first; neither answer is cached or claimed again.
    if answers.iter().any(|a| !a.won && a.reason.as_deref() == Some("hour")) {
        return Reading::WhisperHour;
    }
    if let Some(a) = answers.iter().find(|a| !a.won && a.reason.as_deref() == Some("day")) {
        return Reading::WhisperDay { retry_after_ms: a.retry_after_ms };
    }
    if let Some(loser) = answers.iter().find(|a| !a.won && matches!(a.reason.as_deref(), Some("gap" | "rulecd" | "person" | "reminder_kw"))) {
        return Reading::Cooldown { holder: loser.holder_name.clone().unwrap_or_default(), free_at_ms: loser.free_at_ms };
    }
    if answers.iter().any(|a| !a.won && a.reason.as_deref() == Some("pace")) {
        return Reading::Pace;
    }
    if answers.iter().any(|a| !a.won && a.reason.as_deref() == Some("conv")) {
        return Reading::Conversation;
    }
    let mut won = Vec::new();
    let mut holder: Option<&ElementAnswer> = None;
    for (n, position) in event_positions.iter().enumerate() {
        match by_index.get(position) {
            Some(a) if a.won => won.push(n),
            Some(a) if holder.is_none() && a.reason.as_deref() == Some("held") => holder = Some(a),
            _ => {}
        }
    }
    if !won.is_empty() {
        return Reading::Won(won);
    }
    let h = holder.or_else(|| event_positions.first().and_then(|p| by_index.get(p).copied()));
    Reading::Held {
        holder: h.and_then(|a| a.holder_name.clone()).unwrap_or_default(),
        same_device: h.is_some_and(|a| a.same_device),
        age_ms: h.and_then(|a| a.holder_age_ms).unwrap_or(i64::MAX),
        settle_ms: h.and_then(|a| a.holder_settle_ms).unwrap_or(0),
    }
}

/// A `day` loss of the whisper budget.
const WHISPER_DAY_FULL: &str = "Whisper people a day reached across your devices";

fn lost_words(holder: &str, same_device: bool) -> String {
    if same_device {
        "Already posted from this device".into()
    } else if holder.is_empty() {
        "Another device answered this".into()
    } else {
        format!("Another device answered this ({holder})")
    }
}

/// Step 7: claim the class's event (and rails) in one call, with the owner's
/// verified token; a lost exact event is claimed again at 1.5, 3, 6 and 12 s
/// from the loss while its holder may still give it back.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn claim_class(
    engine: &Arc<Engine>,
    job: &Job,
    plan: &ClassPlan,
    login: &str,
    booked_away: &Option<Away>,
    only: Option<&[usize]>,
    owner: &VerifiedToken,
) -> Claimed {
    let cr = job.rule();
    let rule = &cr.rule;
    let prefs = engine.compiled().prefs.clone();
    let lane = plan.class.key();
    let person = job.person();
    let Some(trigger) = claims::rule_trigger_family(rule, &job.facts) else {
        return Claimed::Skip(claims::BUILD_FAILED.into());
    };
    let family = claims::event_family(&trigger, &plan.class);
    let keys: Vec<String> = if job.badge() {
        let picked: Vec<&BadgeSeen> = match only {
            Some(keep) => keep.iter().filter_map(|i| job.drops.get(*i)).collect(),
            None => job.drops.iter().collect(),
        };
        picked.iter().map(|d| claims::badge_key(&d.feed_id, d.phase)).collect()
    } else {
        vec![job.facts.event_key.clone()]
    };
    let input = CallInput { rule, prefs: &prefs, plan, family: &family, event_keys: &keys, event_room: &job.room, person };
    let elements = match claims::class_elements(&input) {
        Ok(elements) => elements,
        Err(words) => return Claimed::Skip(words.into()),
    };
    let mut kind = if only.is_some() { CallKind::Reclaim } else { CallKind::Rule };
    let mut lost_at: Option<Instant> = None;
    let mut attempt = 0usize;
    loop {
        let Some(left) = job.left() else { return Claimed::Skip("Skipped: too late to answer".into()) };
        let request = ClaimRequest {
            owner_id: rule.owner.id.clone(),
            bearer: owner.token.clone(),
            elements: elements.clone(),
            kind,
            budget: left.min(CLAIM_CAP).max(CLAIM_FLOOR),
            settle_ms: job.settle_ms(),
            next_fire_in: None,
        };
        let result = engine.claims.claim(request).await;
        engine.announce_status();
        let (call, reading) = match result {
            ClaimResult::Unknown { why, .. } => return Claimed::Skip(why.words().into()),
            ClaimResult::Cached { won: true, .. } => {
                let all = (0..elements.iter().filter(|e| e.is_event()).count()).collect();
                return Claimed::Won { call: None, drops: all };
            }
            ClaimResult::Cached { won: false, holder_name, free_at_ms } => {
                (None, Reading::Cooldown { holder: holder_name, free_at_ms })
            }
            ClaimResult::Answered { call_id, claimed_at, answer } => {
                (Some(WonCall { call_id, claimed_at }), read_answer(&elements, &answer.elements))
            }
        };
        match reading {
            Reading::Won(won) => {
                let won = match only {
                    Some(keep) => won.into_iter().filter_map(|i| keep.get(i).copied()).collect(),
                    None => won,
                };
                return Claimed::Won { call, drops: won };
            }
            Reading::Cooldown { holder, free_at_ms } => {
                let left_ms = free_at_ms.map_or(0, |at| (at - engine.claims.server_now_ms()).max(0));
                lock(&engine.guards).stamp_until(rule, &lane, &job.room, person, engine.now() + left_ms);
                return Claimed::Skip(if holder.is_empty() { "Cooldown from another device".into() } else { format!("Cooldown from {holder}") });
            }
            Reading::Pace => return Claimed::Skip("Pace reached across your devices".into()),
            Reading::Conversation => return Claimed::Skip(guards::CONV_FULL.into()),
            Reading::WhisperHour => {
                return Claimed::Skip(format!("Already whispered @{} in the last hour", job.facts.user_login));
            }
            Reading::WhisperDay { retry_after_ms } => {
                // New recipients wait until the oldest counted recipient's 24 h ends;
                // no claim per event (D272, D289).
                let until = chrono::Utc::now().timestamp_millis() + retry_after_ms.unwrap_or(3_600_000);
                lock(&engine.door_holds).hold_new_recipients_until(until);
                return Claimed::Skip(WHISPER_DAY_FULL.into());
            }
            Reading::Held { holder, same_device, age_ms, settle_ms } => {
                let loss = *lost_at.get_or_insert_with(Instant::now);
                let offsets = engine.timings.claims.reclaim_at;
                if age_ms < settle_ms && !same_device && attempt < offsets.len() {
                    let at = loss + offsets[attempt];
                    attempt += 1;
                    let wait = at.saturating_duration_since(Instant::now());
                    if job.left().is_some_and(|l| l > wait) && engine.claims.take_reclaim() {
                        if !engine.executor.pause(job, wait).await {
                            return Claimed::Skip("Stopped".into());
                        }
                        if let Err(reason) = final_checks(engine, job, login, booked_away).await {
                            return Claimed::Skip(reason);
                        }
                        kind = CallKind::Reclaim;
                        continue;
                    }
                }
                let now = engine.now();
                let mut guards = lock(&engine.guards);
                guards.stamp(rule, &lane, &job.room, person, now);
                guards.note_event_lost(job.live, job.admitted_ms);
                drop(guards);
                return Claimed::Skip(if job.badge() { "Another device posted these".into() } else { lost_words(&holder, same_device) });
            }
        }
    }
}

/// One outward action through steps 2 to 10.
async fn outward_action(engine: &Arc<Engine>, job: &Job, action: &Action, plan: &ClassPlan, run: &mut JobRun) {
    let cr = job.rule();
    let rule = &cr.rule;
    let class = plan.class.key();
    let (login, room_id, parent) = target(job, action);
    let chat_key = chat_key_of(&login);
    let first_of_class = !run.classes.get(&class).is_some_and(|c| c.won);
    let mut drops_won: Vec<usize> = (0..job.drops.len()).collect();
    let check = SendCheck {
        owner: &rule.owner.id,
        send_as: &rule.send_as.id,
        chat_key: &chat_key,
        room_id: &room_id,
        other_channel: chat_key != cr.own_key,
        sends: 1,
        to_person: plan.to_person,
        person: job.person(),
    };

    // Step 2: the text as it will be sent ({away} filled before booking),
    // this device's rails, and both accounts' tokens checked with Twitch.
    let mut text = match render(engine, job, action, &drops_won).await {
        Ok(text) => text,
        Err(reason) => return skip(engine, job, &reason),
    };
    if let Err(reason) = lock(&engine.guards).outward_ok(&check, &engine.compiled().prefs, engine.now()) {
        return skip(engine, job, &reason);
    }
    let owner = match verify_accounts(engine, job).await {
        Ok((_, owner)) => owner,
        Err(reason) => return skip(engine, job, &reason),
    };
    let booked_away = engine.away();

    // An Announce: its rails (send_as the primary, a moderator or the broadcaster
    // there), then the room's next 2 s slot reserved under the lock; the wait is
    // here, before booking, never after step 9. A held Announce re-books after
    // the hold instead of skipping; other sends and other rooms are not held.
    // The slot is given back on every skip and on a Refused (the guard's drop), and kept on Sent and Unknown.
    let mut announce_slot: Option<AnnounceSlot> = None;
    if matches!(action, Action::Announce { .. }) {
        if let Err(reason) = super::doors::announce_preflight(engine, job) {
            return skip(engine, job, &reason);
        }
        let (wait_ms, slot) = lock(&engine.door_holds).reserve_announce(&room_id, engine.now());
        announce_slot = Some(AnnounceSlot { holds: &engine.door_holds, room: room_id.clone(), slot, kept: false });
        if wait_ms > 0 && !engine.executor.pause(job, Duration::from_millis(wait_ms as u64)).await {
            return skip(engine, job, "Stopped");
        }
    }

    // Step 3: book. Steps 4 to 6: wait, check, re-book on moved spacing.
    let (wait, mut ticket, booked_text) = match book(engine, job, &login, &text) {
        Ok(booked) => booked,
        Err(reason) => return skip(engine, job, &reason),
    };
    text = booked_text;
    if let Err(reason) = settle_slot(engine, job, &login, &text, &mut ticket, wait, &booked_away).await {
        chat_send_limiter::cancel_booking(&mut ticket);
        return skip(engine, job, &reason);
    }

    let mut rebooks = 0u32;
    if first_of_class {
        // Step 7: the claim.
        match claim_class(engine, job, plan, &login, &booked_away, None, &owner).await {
            Claimed::Skip(reason) => {
                chat_send_limiter::cancel_booking(&mut ticket);
                return skip(engine, job, &reason);
            }
            Claimed::Won { call, drops } => {
                let entry = run.classes.entry(class.clone()).or_default();
                entry.won = true;
                entry.calls.extend(call);
                drops_won = drops;
            }
        }
        // A badge batch that lost some drops claims them again as one
        // follow-up call while it holds the ones it won.
        if job.badge() && drops_won.len() < job.drops.len() {
            let lost: Vec<usize> = (0..job.drops.len()).filter(|i| !drops_won.contains(i)).collect();
            if let Claimed::Won { call, drops } = claim_class(engine, job, plan, &login, &booked_away, Some(&lost), &owner).await {
                run.classes.entry(class.clone()).or_default().calls.extend(call);
                drops_won.extend(drops);
                drops_won.sort_unstable();
            }
            if drops_won.len() < job.drops.len() {
                skip(engine, job, "Another device posted these");
            }
            match render(engine, job, action, &drops_won).await {
                Ok(fresh) if fresh != text => {
                    chat_send_limiter::retext(&mut ticket, &fresh);
                    text = fresh;
                }
                Ok(_) => {}
                Err(reason) => {
                    chat_send_limiter::cancel_booking(&mut ticket);
                    return skip(engine, job, &reason);
                }
            }
        }
        // Won, then the away the booking used changed: give it back.
        if job.uses_away() {
            if let Err(reason) = local_checks(engine, job, &chat_key, &booked_away, true) {
                chat_send_limiter::cancel_booking(&mut ticket);
                return skip(engine, job, &reason);
            }
        }
        // Step 8: the claim's round trip moved the send off its slot.
        if Instant::now() > ticket.at() + REBOOK_TOLERANCE {
            rebooks += 1;
            if let Err(reason) = rebook(engine, job, &login, &text, &mut ticket, &booked_away).await {
                chat_send_limiter::cancel_booking(&mut ticket);
                return skip(engine, job, &reason);
            }
        }
    }

    // Step 9: the verified token (a cache hit after step 2), then the room
    // id when the event did not carry it, then every check with no await
    // before the send.
    let send_as = rule.send_as.id.clone();
    let (token, room_id) = loop {
        let token = match engine.verified(&send_as, job.left().unwrap_or_default()).await {
            Ok(token) => token,
            Err(why) => {
                chat_send_limiter::cancel_booking(&mut ticket);
                run.stopped = true;
                return skip(engine, job, &token_words(&why, &rule.send_as.login));
            }
        };
        let room = if room_id.is_empty() {
            match engine.outward.room_id(&login).await {
                Ok(id) => id,
                Err(e) => {
                    chat_send_limiter::cancel_booking(&mut ticket);
                    run.stopped = true;
                    return skip(engine, job, &e);
                }
            }
        } else {
            room_id.clone()
        };
        let live_now = !rule.only_if.only_live || engine.local.is_live(&login).await == Some(true);
        // From here to the request there is no await.
        if token.user_id != send_as {
            chat_send_limiter::cancel_booking(&mut ticket);
            return skip(engine, job, &sign_in(&rule.send_as.login));
        }
        if let Err(reason) = local_checks(engine, job, &chat_key, &booked_away, live_now) {
            chat_send_limiter::cancel_booking(&mut ticket);
            return skip(engine, job, &reason);
        }
        let now = Instant::now();
        if now <= ticket.at() + REBOOK_TOLERANCE && chat_send_limiter::window_fits_engine(&ticket, now) {
            break (token, room);
        }
        rebooks += 1;
        if rebooks > MAX_REBOOKS {
            chat_send_limiter::cancel_booking(&mut ticket);
            return skip(engine, job, "Couldn't keep the send slot");
        }
        if let Err(reason) = rebook(engine, job, &login, &text, &mut ticket, &booked_away).await {
            chat_send_limiter::cancel_booking(&mut ticket);
            return skip(engine, job, &reason);
        }
    };

    // Step 10: the send. The door marks, records or cancels the ticket.
    let send = ChatSend { token: token.token, text: text.clone(), parent, login: login.clone(), room_id, send_as: send_as.clone() };
    let outcome = if let Action::Announce { color, .. } = action {
        announce_send(engine, job, &mut ticket, &send, color).await
    } else {
        engine.outward.chat_send(&mut ticket, &send).await
    };
    if let Some(slot) = announce_slot.as_mut() {
        if matches!(outcome, SendOutcome::Sent { .. } | SendOutcome::Unknown) {
            slot.kept = true;
        }
    }
    let now = engine.now();
    match outcome {
        SendOutcome::Sent { .. } => {
            run.classes.entry(class.clone()).or_default().sent_or_unknown = true;
            {
                let mut guards = lock(&engine.guards);
                guards.note_sent(&check, now);
                guards.note_echo(&chat_key, &text, now);
                guards.stamp(rule, &class, &job.room, job.person(), now);
            }
            sent(engine, job, action, &login, &chat_key, &drops_won).await;
        }
        SendOutcome::Unknown => {
            run.classes.entry(class.clone()).or_default().sent_or_unknown = true;
            run.stopped = true;
            {
                let mut guards = lock(&engine.guards);
                guards.note_sent(&check, now);
                guards.stamp(rule, &class, &job.room, job.person(), now);
            }
            lock(&engine.counts).note_fired(&rule.id, now_ms());
            engine.note_counts_changed();
            skip(engine, job, "Could not confirm the send");
        }
        SendOutcome::Refused(reason) => {
            run.stopped = true;
            if crate::services::chat_service::is_unauthorized(&reason) {
                // Twitch refused the token itself: its cached owner goes and
                // the account's failure is remembered for a minute.
                engine.token_unauthorized(&send_as, &send.token);
            }
            lock(&engine.guards).hold_after_refusal(&send_as, &chat_key, &reason, now);
            skip(engine, job, &format!("Twitch refused it: {reason}"));
        }
    }
}

/// Bookkeeping after a Sent: counts, the posted badges, Activity, the chat note.
async fn sent(engine: &Arc<Engine>, job: &Job, action: &Action, login: &str, chat_key: &str, drops_won: &[usize]) {
    let rule = &job.rule().rule;
    {
        let mut counts = lock(&engine.counts);
        counts.note_fired(&rule.id, now_ms());
        let posted: Vec<(String, badges::BadgePhase)> =
            drops_won.iter().filter_map(|i| job.drops.get(*i)).map(|d| (d.feed_id.clone(), d.phase)).collect();
        if !posted.is_empty() {
            counts.mark_posted(&rule.id, &posted, now_ms());
        }
    }
    engine.note_counts_changed();
    let user = job.facts.user_login.clone();
    let line = match action {
        Action::Reply { .. } => format!("Replied to @{user} in #{login}"),
        _ if job.badge() => format!("Posted {} new badge(s) in #{login}", drops_won.len()),
        _ => format!("Posted in #{login}"),
    };
    engine.log(entry(job, EntryKind::Acted, chat_key, line));
    let note = if rule.only_if.only_away {
        format!("Replied as away from {}", engine.device_name())
    } else {
        match action {
            Action::Reply { .. } => format!("Automations replied to @{user}"),
            _ => "Automations posted here".to_string(),
        }
    };
    engine.chat_note(&rule.id, login, &note).await;
}

#[cfg(test)]
mod tests {
    use super::super::outward::Outward;
    use super::super::own_topics::TopicKind;
    use super::super::testkit::*;
    use super::*;
    use crate::models::automations::RunState;
    use serde_json::json;

    #[tokio::test]
    async fn test_mode_logs_and_never_claims_or_sends() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", false)]).await;
        kit.fire("m1").await;
        assert!(kit.activity_texts().iter().any(|t| t.starts_with("Would reply: ")));
        assert_eq!(kit.outward.claim_calls(), 0);
        assert!(kit.outward.sends().is_empty());
        assert!(kit.local.notes().iter().any(|(_, t)| t.starts_with("Would reply: ")));
    }

    #[tokio::test]
    async fn live_books_waits_verifies_claims_then_sends() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        let log = kit.outward.calls();
        let verify = log.iter().position(|c| c == "verify").unwrap();
        let claim = log.iter().position(|c| c == "claim").unwrap();
        let send = log.iter().position(|c| c == "send").unwrap();
        assert!(verify < claim && claim < send, "{log:?}");
        assert_eq!(kit.outward.sends().len(), 1);
        assert!(kit.activity_texts().iter().any(|t| t.starts_with("Replied to @viewer")));
        assert!(kit.outward.releases().is_empty());
    }

    #[tokio::test]
    async fn the_claim_is_made_with_the_owners_verified_token() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        kit.fire("m1").await;
        assert_eq!(kit.outward.claim_bearers(), vec![token_of(&main_id())]);
        assert_eq!(kit.outward.sends()[0].token, token_of(&alt_id()));
    }

    #[tokio::test]
    async fn the_claim_bearer_is_the_token_verified_as_the_owner_never_a_later_one() {
        let kit = Kit::new();
        // The owner's fetch answers T1 (the owner) first and T2 (another user) after.
        kit.outward.script_tokens(&main_id(), vec!["T1", "T2"]);
        kit.outward.validates("T1", &main_id(), &me());
        kit.outward.validates("T2", "4444", "someone");
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        let bearers = kit.outward.claim_bearers();
        assert!(bearers == vec!["T1".to_string()] || kit.outward.claim_calls() == 0, "{bearers:?}");
        assert!(!bearers.iter().any(|b| b == "T2"));
    }

    #[tokio::test]
    async fn a_release_takes_a_freshly_verified_owner_token_never_another_accounts() {
        let kit = Kit::new();
        // The fetches: step 2 (T1), step 9 (T1), then the release's own check (T3, the owner's
        // refreshed token). The release goes out with T3, never T1 or T2.
        kit.outward.script_tokens(&main_id(), vec!["T1", "T1", "T3"]);
        kit.outward.validates("T1", &main_id(), &me());
        kit.outward.validates("T3", &main_id(), &me());
        kit.outward.validates("T2", "4444", "someone");
        kit.outward.lose_sends_with(SendResult::Refused("not sent".into()));
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        kit.settle_claims().await;
        let bearers = kit.outward.release_bearers();
        assert!(!bearers.is_empty());
        assert!(bearers.iter().all(|b| b == "T3"), "{bearers:?}");

        // The release's check answers T2, which validates as another user: the release is dropped.
        let kit = Kit::new();
        kit.outward.script_tokens(&main_id(), vec!["T1", "T1", "T2"]);
        kit.outward.validates("T1", &main_id(), &me());
        kit.outward.validates("T2", "4444", "someone");
        kit.outward.lose_sends_with(SendResult::Refused("not sent".into()));
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        kit.settle_claims().await;
        assert!(kit.outward.release_bearers().is_empty(), "never posted as another account");
        assert!(kit.outward.releases().is_empty());
    }

    #[tokio::test]
    async fn a_token_that_validates_as_someone_else_is_refused_before_the_claim() {
        let kit = Kit::new();
        kit.outward.set_token(&alt_id(), FakeToken::WrongAccount("99".into()));
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        kit.fire("m1").await;
        assert_eq!(kit.outward.claim_calls(), 0);
        assert!(kit.outward.sends().is_empty());
        assert_eq!(chat_send_limiter::alt_entries(&alt_id()), (0, 0), "nothing booked");
        assert!(kit
            .activity_texts()
            .iter()
            .any(|t| t == "Sign in as @alt again: Twitch says this sign-in belongs to another account"));
    }

    #[tokio::test]
    async fn a_send_401_drops_the_cached_owner_and_holds_the_account_a_minute() {
        let kit = Kit::new();
        let unauthorized = format!("{} (401)", crate::services::chat_service::UNAUTHORIZED);
        kit.outward.lose_sends_with(SendResult::Refused(unauthorized));
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        assert!(kit.outward.verifier().remembered_failure(&main_id()).is_some(), "a send 401 is remembered");
        kit.outward.lose_sends_with(SendResult::Sent);
        kit.fire_from("m2", "viewer2").await;
        assert_eq!(kit.outward.send_attempts(), 1, "held for a minute");
        kit.clock.advance(Duration::from_secs(61));
        let before = kit.outward.validate_calls(&token_of(&main_id()));
        kit.fire_from("m3", "viewer3").await;
        assert!(kit.outward.validate_calls(&token_of(&main_id())) > before, "the dropped entry is validated again");
    }

    #[tokio::test]
    async fn another_refusal_keeps_the_cached_owner() {
        let kit = Kit::new();
        kit.outward.lose_sends_with(SendResult::Refused("Your message is identical to the previous one".into()));
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        assert!(kit.outward.verifier().remembered_failure(&main_id()).is_none());
    }

    #[tokio::test]
    async fn a_dead_owner_is_refused_before_the_claim() {
        let kit = Kit::new();
        kit.outward.set_token(&main_id(), FakeToken::Refused);
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        kit.fire("m1").await;
        assert_eq!(kit.outward.claim_calls(), 0);
        assert!(kit.activity_texts().iter().any(|t| *t == format!("Sign in as @{} to run this rule", me())));
    }

    #[tokio::test]
    async fn an_unreachable_validate_skips_this_event_only() {
        let kit = Kit::new();
        kit.outward.set_token(&main_id(), FakeToken::Unreachable);
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        assert!(kit.activity_texts().iter().any(|t| *t == format!("Couldn't reach Twitch to check @{}; skipped", me())));
        kit.outward.set_token(&main_id(), FakeToken::Good);
        kit.fire_from("m2", "viewer2").await;
        assert_eq!(kit.outward.sends().len(), 1, "an unreachable check is not remembered");
    }

    #[tokio::test]
    async fn jobs_for_one_account_and_chat_run_in_admission_order() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        for i in 0..3 {
            kit.engine.on_chat(&chat_row(&format!("m{i}"), &format!("u{i}"), "hey @me"), false);
        }
        kit.idle().await;
        let texts: Vec<String> = kit.outward.sends().iter().map(|s| s.text.clone()).collect();
        assert_eq!(texts.len(), 3);
        assert!(texts[0].contains("u0") && texts[1].contains("u1") && texts[2].contains("u2"), "{texts:?}");
    }

    #[tokio::test]
    async fn a_refused_send_releases_and_stops_later_outward_actions_but_not_notify() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![reply_send_notify_rule("r1", true)]).await;
        kit.outward.lose_sends_with(SendResult::Refused("Your message was not sent".into()));
        kit.fire("m1").await;
        assert_eq!(kit.outward.releases().len(), 1);
        assert_eq!(kit.outward.send_attempts(), 1);
        assert_eq!(kit.local.notifications().len(), 1);
    }

    #[tokio::test]
    async fn an_unknown_send_keeps_the_claim_and_stops_the_rest() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![reply_send_notify_rule("r1", true)]).await;
        kit.outward.lose_sends_with(SendResult::Unknown);
        kit.fire("m1").await;
        assert!(kit.outward.releases().is_empty());
        assert_eq!(kit.outward.send_attempts(), 1);
        assert!(kit.activity_texts().iter().any(|t| t == "Could not confirm the send"));
        assert_eq!(kit.local.notifications().len(), 1);
    }

    #[tokio::test]
    async fn a_second_reply_whose_token_fails_keeps_the_event() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![two_replies_rule("r1", true)]).await;
        kit.outward.fail_token_after(&main_id(), 2);
        kit.fire("m1").await;
        assert_eq!(kit.outward.sends().len(), 1);
        assert!(kit.outward.releases().is_empty(), "the first reply was sent");
        let other = kit.second_device();
        other.settings(RunState::On, vec![as_sender(mention_rule("r9", true), &alt_id(), "alt")]).await;
        other.fire("m1").await;
        assert!(other.outward.sends().is_empty());
    }

    #[tokio::test]
    async fn a_refused_only_reply_frees_the_event_for_the_other_device() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![two_replies_rule("r1", true)]).await;
        kit.outward.lose_sends_with(SendResult::Refused("not sent".into()));
        kit.fire("m1").await;
        assert_eq!(kit.outward.releases().len(), 1);
        let other = kit.second_device();
        other.settings(RunState::On, vec![as_sender(mention_rule("r9", true), &alt_id(), "alt")]).await;
        other.fire("m1").await;
        assert_eq!(other.outward.sends().len(), 1);
    }

    #[tokio::test]
    async fn a_lost_event_cancels_the_booking_and_still_notifies() {
        let kit = Kit::new();
        let other = kit.second_device();
        other.settings(RunState::On, vec![as_sender(mention_rule("r9", true), &alt_id(), "alt")]).await;
        other.outward.park_sends();
        other.fire_no_wait("m1");
        other.outward.wait_for_send_started().await;
        kit.settings(RunState::On, vec![reply_notify_rule("r1", true)]).await;
        kit.fire("m1").await;
        assert!(kit.outward.sends().is_empty());
        assert!(kit.activity_texts().iter().any(|t| t.starts_with("Another device answered this")));
        assert_eq!(kit.local.notifications().len(), 1);
        assert_eq!(chat_send_limiter::alt_entries(&main_id()), (0, 0));
        other.outward.release_sends();
        other.idle().await;
    }

    #[tokio::test]
    async fn a_lost_pace_stamps_nothing_and_a_lost_gap_says_cooldown() {
        let kit = Kit::new();
        kit.server.fill_counted(&main_id(), "pace:", 10);
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        assert!(kit.activity_texts().iter().any(|t| t == "Pace reached across your devices"));
        let kit = Kit::new();
        kit.server.hold_windows("gap:", "Mac 91bc");
        kit.prefs(|p| p.other_channels = true).await;
        kit.settings(RunState::On, vec![these_reply_rule("r1", &["elsewhere"], true)]).await;
        kit.fire_in("m1", "elsewhere").await;
        assert!(kit.activity_texts().iter().any(|t| t == "Cooldown from Mac 91bc"));
    }

    #[tokio::test]
    async fn an_unanswered_claim_cancels_and_skips() {
        let kit = Kit::new();
        kit.outward.claim_answers(vec![FakeHttp::Network]);
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.fire("m1").await;
        assert!(kit.outward.sends().is_empty());
        assert!(kit.activity_texts().iter().any(|t| t == "Couldn't check with StreamNook; skipped"));
    }

    #[tokio::test]
    async fn off_during_the_wait_cancels_the_booking_and_claims_nothing() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.typed_now(&main_id(), &chan());
        kit.fire_no_wait("m1");
        tokio::time::sleep(Duration::from_millis(50)).await;
        kit.engine.set_run_now(RunState::Off);
        kit.idle().await;
        assert_eq!(kit.outward.claim_calls(), 0);
        assert!(kit.outward.sends().is_empty());
        assert_eq!(chat_send_limiter::alt_entries(&main_id()).1, 0, "no booking left behind");
    }

    #[tokio::test]
    async fn a_job_past_its_age_bound_is_dropped_before_the_claim() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        let mut job = kit.job_for("m1", "r1");
        job.admitted_at -= Duration::from_secs(16);
        kit.engine.executor.submit(&kit.engine, job);
        kit.idle().await;
        assert_eq!(kit.outward.claim_calls(), 0);
        assert!(kit.outward.sends().is_empty());
    }

    #[tokio::test]
    async fn two_rules_on_one_device_both_act_for_one_event() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true), mention_rule("r2", true)]).await;
        kit.fire("m1").await;
        assert_eq!(kit.outward.claim_calls(), 2);
        assert_eq!(kit.outward.sends().len(), 2);
    }

    #[tokio::test]
    async fn reply_on_one_device_and_send_here_on_the_other_post_once() {
        for b_first in [false, true] {
            let a = Kit::new();
            let b = a.second_device();
            a.settings(RunState::On, vec![mention_rule("ra", true)]).await;
            b.settings(RunState::On, vec![as_sender(mention_send_here_rule("rb", true), &alt_id(), "alt")]).await;
            if b_first {
                b.fire("m1").await;
                a.fire("m1").await;
            } else {
                a.fire("m1").await;
                b.fire("m1").await;
            }
            assert_eq!(a.outward.sends().len() + b.outward.sends().len(), 1, "b first: {b_first}");
        }
    }

    #[tokio::test]
    async fn a_send_to_another_chat_is_its_own_class() {
        let a = Kit::new();
        let b = a.second_device();
        a.settings(RunState::On, vec![mention_rule("ra", true)]).await;
        b.prefs(|p| p.other_channels = true).await;
        b.settings(RunState::On, vec![as_sender(mention_send_to_rule("rb", true, "elsewhere", "777"), &alt_id(), "alt")]).await;
        a.fire("m1").await;
        b.fire("m1").await;
        assert_eq!(a.outward.sends().len(), 1);
        assert_eq!(b.outward.sends().len(), 1);
    }

    #[tokio::test]
    async fn a_shared_chat_row_and_its_partner_copy_post_once() {
        let a = Kit::new();
        let b = a.second_device();
        a.settings(RunState::On, vec![mention_rule("ra", true)]).await;
        b.settings(RunState::On, vec![as_sender(mention_rule("rb", true), &alt_id(), "alt")]).await;
        a.fire_row(shared_row("native-id", "native-id", "4242", None)).await;
        b.fire_row(shared_row("copy-id", "native-id", "5353", Some("4242"))).await;
        assert_eq!(a.outward.sends().len() + b.outward.sends().len(), 1);
    }

    #[tokio::test]
    async fn a_holder_that_was_refused_lets_the_loser_answer_on_its_reclaim() {
        let a = Kit::new();
        let b = a.second_device();
        a.settings(RunState::On, vec![mention_rule("ra", true)]).await;
        b.settings(RunState::On, vec![as_sender(mention_rule("rb", true), &alt_id(), "alt")]).await;
        a.outward.lose_sends_with(SendResult::Refused("not sent".into()));
        a.outward.delay_sends(Duration::from_millis(500));
        a.fire_no_wait("m1");
        a.outward.wait_for_send_started().await;
        b.fire("m1").await;
        a.idle().await;
        assert_eq!(b.outward.sends().len(), 1);
        assert!(b.outward.bodies().iter().any(|body| body["reclaim"] == serde_json::json!(true)));
        assert!(b.outward.bodies().iter().all(|body| body["action"] == "claim" || body["action"] == "release"));
    }

    #[tokio::test]
    async fn when_the_holder_sent_the_loser_skips_after_the_settle_time() {
        let a = Kit::new();
        let b = a.second_device();
        a.settings(RunState::On, vec![mention_rule("ra", true)]).await;
        b.settings(RunState::On, vec![as_sender(mention_rule("rb", true), &alt_id(), "alt")]).await;
        a.fire("m1").await;
        b.fire("m1").await;
        assert!(b.outward.sends().is_empty());
        assert!(b.activity_texts().iter().any(|t| t.starts_with("Another device answered this")));
    }

    #[tokio::test]
    async fn a_won_claim_is_never_freed_by_time() {
        let a = Kit::new();
        let b = a.second_device();
        a.settings(RunState::On, vec![mention_rule("ra", true)]).await;
        b.settings(RunState::On, vec![as_sender(mention_rule("rb", true), &alt_id(), "alt")]).await;
        a.fire("m1").await;
        for skew_ms in [20_000, 600_000, 47 * 3_600_000] {
            a.server.skew(skew_ms);
            b.fire_from("m1", &format!("viewer{skew_ms}")).await;
        }
        assert!(b.outward.sends().is_empty());
    }

    #[tokio::test]
    async fn a_lost_release_never_lets_a_second_device_post() {
        let a = Kit::new();
        let b = a.second_device();
        a.settings(RunState::On, vec![mention_rule("ra", true)]).await;
        b.settings(RunState::On, vec![as_sender(mention_rule("rb", true), &alt_id(), "alt")]).await;
        a.outward.lose_sends_with(SendResult::Refused("not sent".into()));
        a.outward.drop_releases();
        a.fire("m1").await;
        b.fire("m1").await;
        assert!(b.outward.sends().is_empty());
    }

    #[tokio::test]
    async fn an_off_during_a_slow_room_lookup_stops_the_class_and_releases() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        // The step 9 await (the send's token check) is slow; the Off lands inside it.
        kit.outward.slow_verify_at(2, Duration::from_millis(500));
        kit.fire_no_wait("m1");
        tokio::time::sleep(Duration::from_millis(150)).await;
        kit.engine.set_run_now(RunState::Off);
        kit.idle().await;
        assert!(kit.outward.sends().is_empty());
        assert_eq!(kit.outward.releases().len(), 1, "nothing of the class went out");
    }

    #[tokio::test]
    async fn a_slow_token_rebooks_through_the_tolerance_check() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![two_replies_rule("r1", true)]).await;
        kit.outward.slow_verify_after_first(Duration::from_millis(400));
        kit.fire("m1").await;
        let sends = kit.outward.sends();
        assert_eq!(sends.len(), 2);
        for s in &sends {
            assert!(s.late_by <= REBOOK_TOLERANCE + Duration::from_millis(50), "{:?}", s.late_by);
        }
    }

    #[tokio::test]
    async fn the_main_account_books_on_the_main_limiter() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        // You just typed the same text as the main account: Twitch would drop a repeat.
        chat_send_limiter::record_sent(&chan(), "@viewer thanks (r1)");
        kit.fire("m1").await;
        assert!(kit.outward.sends().is_empty(), "the typed message is on the limiter the engine booked on");
        assert_eq!(kit.outward.claim_calls(), 0, "refused at the booking, before the claim");
        // A linked account books on its own limiter, which never saw it.
        kit.settings(RunState::On, vec![as_sender(mention_rule("r2", true), &alt_id(), "alt")]).await;
        chat_send_limiter::record_sent(&chan(), "@viewer thanks (r2)");
        kit.fire_from("m2", "viewer").await;
        assert_eq!(kit.outward.sends().len(), 1);
        assert_eq!(kit.outward.sends()[0].send_as, alt_id());
    }

    #[tokio::test]
    async fn the_hook_alone_stops_a_waiting_job_and_its_release_goes_out() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        // The send's token check (the third fetch: sender, owner, then step 9) is slow.
        kit.outward.slow_verify_at(3, Duration::from_millis(600));
        kit.fire_no_wait("m1");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let guard = super::super::before_accounts_removed_on(Some(&kit.engine), &[alt_id()]).await;
        kit.idle().await;
        assert!(kit.outward.sends().is_empty());
        assert_eq!(kit.outward.releases().len(), 1);
        drop(guard);
    }

    #[tokio::test]
    async fn a_removed_account_stops_the_send_without_a_hook() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        // The send's token check (the third fetch: sender, owner, then step 9) is slow.
        kit.outward.slow_verify_at(3, Duration::from_millis(500));
        kit.fire_no_wait("m1");
        tokio::time::sleep(Duration::from_millis(150)).await;
        kit.outward.set_own_accounts(vec![(main_id(), me())]);
        kit.engine.recompile().await;
        kit.idle().await;
        assert!(kit.outward.sends().is_empty());
        assert_eq!(kit.outward.releases().len(), 1);
    }

    #[tokio::test]
    async fn step_nine_refuses_a_post_bump_job_while_the_guard_is_held_and_sends_after() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        let guard = super::super::before_accounts_removed_on(Some(&kit.engine), &[alt_id()]).await;
        let job = kit.job_for("m1", "r1");
        kit.engine.executor.submit(&kit.engine, job);
        kit.idle().await;
        assert!(kit.outward.sends().is_empty());
        drop(guard);
        let job = kit.job_for("m2", "r1");
        kit.engine.executor.submit(&kit.engine, job);
        kit.idle().await;
        assert_eq!(kit.outward.sends().len(), 1);
    }

    #[tokio::test]
    async fn a_local_back_between_the_win_and_the_send_gives_the_claim_back() {
        let kit = Kit::new();
        kit.set_away(Some("brb")).await;
        kit.settings(RunState::On, vec![away_rule("r1", true)]).await;
        kit.outward.delay_claims(Duration::from_millis(80));
        kit.fire_no_wait("m1");
        kit.outward.wait_for_claim_started().await;
        kit.engine.set_away_now(None);
        kit.idle().await;
        assert!(kit.outward.sends().is_empty());
        assert!(kit.activity_texts().iter().any(|t| t == "Skipped: You're back"));
        assert_eq!(kit.outward.releases().len(), 1);
    }

    #[tokio::test]
    async fn an_away_reply_fills_the_text_before_booking_and_notes_this_device() {
        let kit = Kit::new();
        kit.set_away(Some("brb")).await;
        kit.settings(RunState::On, vec![away_rule("r1", true)]).await;
        kit.fire("m1").await;
        assert_eq!(kit.outward.sends().len(), 1);
        assert!(kit.outward.sends()[0].text.contains("brb"));
        let device = kit.engine.device_name();
        assert!(kit.local.notes().iter().any(|(_, t)| *t == format!("Replied as away from {device}")));
        assert!(kit.outward.bodies().iter().all(|b| b.get("away").is_none() && b["action"] != "away"));
    }

    #[tokio::test]
    async fn a_refusal_that_says_you_cannot_post_holds_the_chat() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.outward.lose_sends_with(SendResult::Refused("You are banned from talking in this channel".into()));
        kit.fire("m1").await;
        kit.outward.lose_sends_with(SendResult::Sent);
        kit.fire_from("m2", "viewer2").await;
        assert_eq!(kit.outward.send_attempts(), 1);
    }

    #[test]
    fn read_answer_names_the_loser() {
        let elements = vec![event_el("chat:x"), rail_el("gap:1", 30)];
        let won = vec![ElementAnswer::won(0), ElementAnswer::won(1)];
        assert_eq!(read_answer(&elements, &won), Reading::Won(vec![0]));
        let gap = vec![ElementAnswer::lost(0, "rail"), ElementAnswer::lost_by(1, "gap", "Mac 91bc", Some(5_000))];
        assert_eq!(read_answer(&elements, &gap), Reading::Cooldown { holder: "Mac 91bc".into(), free_at_ms: Some(5_000) });
        let pace = vec![ElementAnswer::lost(0, "rail"), ElementAnswer::lost(1, "pace")];
        assert_eq!(read_answer(&elements, &pace), Reading::Pace);
        let held = vec![ElementAnswer::held(0, "Mac 91bc", false, 100, 15_000), ElementAnswer::lost(1, "skipped")];
        assert!(matches!(read_answer(&elements, &held), Reading::Held { age_ms: 100, settle_ms: 15_000, .. }));
        // The whisper budget's answers are their own readings, never "Another device answered this".
        let hour = vec![ElementAnswer::lost(0, "rail"), ElementAnswer::lost(1, "hour")];
        assert_eq!(read_answer(&elements, &hour), Reading::WhisperHour);
        let day = vec![ElementAnswer::lost(0, "rail"), ElementAnswer::lost(1, "day")];
        assert_eq!(read_answer(&elements, &day), Reading::WhisperDay { retry_after_ms: None });
    }

    #[test]
    fn engine_modules_name_no_token_send_or_lookup_function() {
        for (name, src) in [
            ("mod.rs", include_str!("mod.rs")),
            ("executor.rs", include_str!("executor.rs")),
            ("reconcile.rs", include_str!("reconcile.rs")),
            ("claim_client.rs", include_str!("claim_client.rs")),
            ("notify.rs", include_str!("notify.rs")),
            ("local.rs", include_str!("local.rs")),
            ("store.rs", include_str!("store.rs")),
        ] {
            let src = src.replace("\r\n", "\n");
            let code = src.split("#[cfg(test)]\nmod tests").next().unwrap();
            for banned in [
                "get_token(",
                "get_token_with_generation",
                "get_token_for",
                "get_token_after_reject",
                "primary(",
                "AccountStore::list(",
                "user_by_login",
                "send_message_helix_only",
                "post_json(",
                "oauth2/validate",
            ] {
                assert!(!code.contains(banned), "{name} names {banned}");
            }
        }
    }

    #[tokio::test]
    async fn an_edge_job_that_waited_60_s_still_sends_and_one_that_waited_130_s_is_dropped() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![own_rule("f", true, json!({ "kind": "follow" }), vec![send_action("a-1", "Thanks {user}!")])]).await;
        let mut waited = kit.job_with_facts(follow_facts("7"), "f");
        waited.admitted_at -= Duration::from_secs(60);
        kit.engine.executor.submit(&kit.engine, waited);
        kit.idle().await;
        assert_eq!(kit.outward.sends().len(), 1, "inside the 120 s edge allowance, with the normal claim budget");
        assert_eq!(kit.outward.claim_calls(), 1);
        let mut late = kit.job_with_facts(follow_facts("8"), "f");
        late.admitted_at -= Duration::from_secs(130);
        kit.engine.executor.submit(&kit.engine, late);
        kit.idle().await;
        assert_eq!(kit.outward.sends().len(), 1, "130 s is past it");
        assert_eq!(kit.outward.claim_calls(), 1, "dropped before any claim");
        assert!(kit.activity_texts().iter().any(|t| t.contains("too late")));
    }

    #[tokio::test]
    async fn an_edge_post_goes_to_the_owners_own_channel_and_names_the_follower() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![own_rule("f", true, json!({ "kind": "follow" }), vec![send_action("a-1", "Thanks {user}!")])]).await;
        kit.engine.on_own_topic(TopicKind::Follow, &follow_event("7", "Fan"));
        kit.idle().await;
        let sends = kit.outward.sends();
        assert_eq!(sends.len(), 1);
        assert_eq!((sends[0].text.as_str(), sends[0].login.as_str()), ("Thanks Fan!", "me"));
    }

    #[tokio::test]
    async fn an_edge_job_keeps_its_claim_after_an_unknown_send_and_gives_it_back_after_a_refused_one() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![own_rule("f", true, json!({ "kind": "follow" }), vec![send_action("a-1", "Thanks {user}!")])]).await;
        kit.outward.lose_sends_with(SendResult::Unknown);
        kit.engine.on_own_topic(TopicKind::Follow, &follow_event("7", "Fan"));
        kit.idle().await;
        assert!(kit.outward.releases().is_empty(), "an Unknown may have posted");
        kit.outward.lose_sends_with(SendResult::Refused("not sent".into()));
        kit.engine.on_own_topic(TopicKind::Follow, &follow_event("8", "Fan2"));
        kit.idle().await;
        assert_eq!(kit.outward.releases().len(), 1, "a Refused send gives its class back");
    }

    #[tokio::test]
    async fn two_devices_answer_one_follow_once() {
        let kit = Kit::new();
        let other = kit.second_device();
        for k in [&kit, &other] {
            k.settings(RunState::On, vec![own_rule("f", true, json!({ "kind": "follow" }), vec![send_action("a-1", "Thanks {user}!")])]).await;
            k.engine.on_own_topic(TopicKind::Follow, &follow_event("7", "Fan"));
            k.idle().await;
        }
        assert_eq!(kit.outward.sends().len() + other.outward.sends().len(), 1, "each device hears the event; the claim picks one");
    }

    #[tokio::test]
    async fn a_test_rule_on_an_own_event_logs_what_it_would_post_and_claims_nothing() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![own_rule("f", false, json!({ "kind": "follow" }), vec![send_action("a-1", "Thanks {user}!")])]).await;
        kit.engine.on_own_topic(TopicKind::Follow, &follow_event("7", "Fan"));
        kit.idle().await;
        assert!(kit.activity_texts().iter().any(|t| t == "Would post: Thanks Fan!"));
        assert_eq!(kit.outward.claim_calls(), 0);
        assert!(kit.outward.sends().is_empty());
    }

    #[tokio::test]
    async fn speak_runs_in_test_and_live_claims_nothing_and_never_posts() {
        for live in [false, true] {
            let kit = Kit::new();
            kit.settings(RunState::On, vec![own_rule("f", live, json!({ "kind": "follow" }), vec![speak_action("Thanks {user}", false)])]).await;
            kit.engine.on_own_topic(TopicKind::Follow, &follow_event("7", "Fan"));
            kit.idle().await;
            let said: Vec<String> = kit.local.speaks().into_iter().map(|s| s.text).collect();
            assert_eq!(said, vec!["Thanks Fan".to_string()], "live={live}");
            assert_eq!(kit.outward.claim_calls(), 0, "a local action never claims");
            assert!(kit.outward.sends().is_empty(), "nothing is ever posted to chat");
        }
    }

    #[tokio::test]
    async fn speak_skips_in_streamer_mode_unless_it_says_otherwise() {
        let kit = Kit::new();
        kit.local.set_streamer(true);
        kit.settings(RunState::On, vec![own_rule("a", true, json!({ "kind": "follow" }), vec![speak_action("quiet", false)]), own_rule("b", true, json!({ "kind": "follow" }), vec![speak_action("loud", true)])]).await;
        kit.engine.on_own_topic(TopicKind::Follow, &follow_event("7", "Fan"));
        kit.idle().await;
        let said: Vec<String> = kit.local.speaks().into_iter().map(|s| s.text).collect();
        assert_eq!(said, vec!["loud".to_string()]);
        assert!(kit.activity_texts().iter().any(|t| t == "Skipped: streamer mode"));
    }

    #[tokio::test]
    async fn speak_with_no_player_shows_a_system_notice_with_the_text() {
        let kit = Kit::new();
        kit.local.set_targets(vec![]);
        kit.settings(RunState::On, vec![own_rule("f", true, json!({ "kind": "follow" }), vec![speak_action("Thanks {user}", false)])]).await;
        kit.engine.on_own_topic(TopicKind::Follow, &follow_event("7", "Fan"));
        kit.idle().await;
        assert!(kit.local.speaks().is_empty());
        assert_eq!(kit.local.system_bodies(), vec!["Thanks Fan".to_string()], "a system notification only when no window can speak");
    }

    #[tokio::test]
    async fn speak_refuses_the_viewers_words_at_fire_time_too() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![own_rule("r", true, json!({ "kind": "reward_redeemed", "reward_id": null }), vec![speak_action("{input}", false)])]).await;
        kit.engine.on_own_topic(
            TopicKind::Redemption,
            &json!({ "id": "red-1", "user_id": "8", "user_login": "viewer", "user_name": "Viewer", "user_input": "say this", "reward": { "id": "rw", "title": "Voice" } }),
        );
        kit.idle().await;
        assert!(kit.local.speaks().is_empty());
        assert!(kit.activity_texts().iter().any(|t| t.starts_with("Skipped: the chatter's words")));
    }

    #[tokio::test]
    async fn a_highlight_action_runs_nothing_per_event() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![highlight_rule("h", "gg", "#ff8800")]).await;
        kit.fire_from("m1", "viewer").await;
        assert!(kit.local.notifications().is_empty() && kit.outward.sends().is_empty());
    }
}
