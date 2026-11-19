//! Account automations: rules that reply, alert and moderate for you.
//!
//! The engine lives in one [`Engine`] value. The running app keeps one in a
//! process singleton; every test builds its own, so no test shares state with
//! another or with the app. Every free function here is a no-op until
//! [`init`] has run.
//!
//! The hot path is [`on_chat`]: with no rule that can fire it is one atomic
//! load. Matching is pure (`matcher`); an admitted event becomes a job for
//! the executor, which books a send slot, verifies the accounts' tokens with
//! Twitch, claims the event on streamnook.app and sends through the one door
//! to the outside (`outward`). One long-lived task (`reconcile`) compiles the
//! rules and keeps the chats, liveness watches and badge work the rules need.

pub mod activity;
pub mod badges;
pub mod claim_client;
pub mod reminder_claim;
pub mod claims;
pub mod command;
pub mod compile;
pub mod counts;
pub mod doors;
pub mod executor;
pub mod facts;
pub mod guards;
pub mod local;
pub mod logfile;
pub mod matcher;
pub mod notify;
pub mod outward;
pub mod own_topics;
pub mod provider_facts;
pub mod reconcile;
pub mod speak;
pub mod store;
pub mod template;
pub mod templates;
pub mod timed;
pub mod timers;
#[cfg(test)]
pub(crate) mod testkit;
pub mod verify;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock};
use std::future::Future;
use std::time::{Duration, Instant};

use futures::future::{AbortHandle, Abortable, BoxFuture};
use serde::Serialize;
use tokio::sync::{oneshot, watch};

use crate::models::automations::{Away, BadgeLink, Rule, RunState, Trigger};
use crate::models::chat_layout::ChatMessage;
use crate::rt::AppHandle;

use self::activity::{Entry, EntryKind, Filter, Log, NewEntry};
use self::badges::{BadgeSeen, Batch, LinkStep, LinkWait, Pending};
use self::claim_client::{ClaimClient, ClaimTimings};
use self::compile::{CompileInput, Compiled, Mode};
use self::provider_facts::PlatformId;
use self::counts::Counts;
use self::executor::{Executor, Job};
use self::facts::Facts;
use self::guards::{Admit, Guards, Trip};
use self::local::{AccountChange, AccountChanges, Local};
use self::notify::{NotifyRequest, Notifier};
use self::own_topics::{SeenKeys, TopicKind};
use self::outward::Outward;
use self::store::{Device, Files};
use self::verify::{TokenRefusal, VerifiedToken};

/// The label the engine's chat claims carry.
pub const AUTOMATIONS_LABEL: &str = "automations";
/// Badge drops kept for the editor's "Try with".
const BADGE_SAMPLES: usize = 20;
const COUNTS_FILE: &str = "automation_counts.json";
const ACTIVITY_FILE: &str = "automation_activity.json";
const PENDING_FILE: &str = "automation_pending.json";

const RUN_OFF: u8 = 0;
const RUN_TEST: u8 = 1;
const RUN_ON: u8 = 2;

fn run_code(run: RunState) -> u8 {
    match run {
        RunState::Off => RUN_OFF,
        RunState::Test => RUN_TEST,
        RunState::On => RUN_ON,
    }
}

fn run_from(code: u8) -> RunState {
    match code {
        RUN_ON => RunState::On,
        RUN_TEST => RunState::Test,
        _ => RunState::Off,
    }
}

/// Lock a std mutex, reading through a poisoned one.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read<T: Clone>(m: &RwLock<T>) -> T {
    m.read().unwrap_or_else(PoisonError::into_inner).clone()
}

fn write<T>(m: &RwLock<T>, value: T) {
    *m.write().unwrap_or_else(PoisonError::into_inner) = value;
}

/// Wall-clock milliseconds (Activity times, a row's age, a badge's age).
pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Steady milliseconds: the rails, holds and the token cache run on it.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The real steady clock: milliseconds since the first read.
pub fn steady_clock() -> Clock {
    static START: OnceLock<Instant> = OnceLock::new();
    Arc::new(|| START.get_or_init(Instant::now).elapsed().as_millis() as i64)
}

/// Where the engine runs its own tasks (lanes, batches, flushes, claims and
/// releases). Tests hand in the test's runtime, so paused time moves them.
pub type Spawn = Arc<dyn Fn(BoxFuture<'static, ()>) + Send + Sync>;

/// The app's spawner: Tauri's runtime, the task detached (claims and
/// releases outlive the job that started them). The one place an engine
/// module names Tauri's spawn.
pub fn tauri_spawner() -> Spawn {
    Arc::new(|task: BoxFuture<'static, ()>| {
        tauri::async_runtime::spawn(task);
    })
}

/// The waits the engine uses, so tests can run them in milliseconds.
#[derive(Clone, Copy, Debug)]
pub struct Timings {
    /// How long an account removal waits for the jobs it canceled.
    pub remove_wait: Duration,
    /// How long a player has to confirm a notification.
    pub notify_confirm: Duration,
    /// Reconcile's chat start backs off from this, doubling.
    pub chat_start_first: Duration,
    /// The chat start backoff stops growing at this.
    pub chat_start_cap: Duration,
    /// Activity reaches each watching window at most this often.
    pub activity_emit: Duration,
    /// The badge gallery is kept at most this old while a badge rule runs.
    pub gallery_refresh: Duration,
    /// A failed gallery refresh is tried again after this.
    pub gallery_retry: Duration,
    /// A job waits at most this long for its releases to go out.
    pub job_release_wait: Duration,
    /// An adoption waits at most this long for reconcile.
    pub adopt_wait: Duration,
    /// The badge batcher looks at its open batch at least this often.
    pub batcher_tick: Duration,
    /// Counts are written this long after they change.
    pub counts_flush: Duration,
    /// Activity is written this long after it changes.
    pub activity_flush: Duration,
    /// The claim client's waits.
    pub claims: ClaimTimings,
}

impl Timings {
    pub fn production() -> Timings {
        Timings {
            remove_wait: Duration::from_secs(3),
            notify_confirm: Duration::from_secs(3),
            chat_start_first: Duration::from_secs(5),
            chat_start_cap: Duration::from_secs(300),
            activity_emit: Duration::from_millis(250),
            gallery_refresh: Duration::from_secs(6 * 3600),
            gallery_retry: Duration::from_secs(300),
            job_release_wait: Duration::from_secs(5),
            adopt_wait: Duration::from_secs(3),
            batcher_tick: Duration::from_secs(1),
            counts_flush: Duration::from_secs(5),
            activity_flush: Duration::from_secs(10),
            claims: ClaimTimings::production(),
        }
    }
}

/// What an engine is built from.
pub struct Deps {
    pub outward: Arc<dyn Outward>,
    pub local: Arc<dyn Local>,
    pub app: Option<AppHandle>,
    pub files: Option<Files>,
    pub device: Device,
    pub timings: Timings,
    /// The steady clock (tests move it).
    pub clock: Clock,
    /// Where the engine's tasks run (`tauri_spawner()` in the app).
    pub spawn: Spawn,
}

/// Work reconcile does on request, between its compiles.
#[derive(Default)]
pub(crate) struct RecShared {
    /// IrcService cleared every claim: the engine's rooms must be joined again.
    pub claims_reset: bool,
    /// An account save: a chat start may try again at once.
    pub reset_backoff: bool,
    /// Windows closing whose rooms the engine should keep: (label, done).
    pub adopt: Vec<(String, oneshot::Sender<()>)>,
    /// Rooms adopted from a closing window, with the adoption time.
    pub adopted: HashMap<String, i64>,
    /// The last time a window claimed each adopted room.
    pub window_claims: HashMap<String, i64>,
}

/// Activity entries waiting for their coalesced emit.
#[derive(Default)]
struct EmitQueue {
    entries: Vec<Entry>,
    scheduled: bool,
}

/// The engine: everything that is not persisted, plus the handles to what is.
pub struct Engine {
    pub(crate) outward: Arc<dyn Outward>,
    pub(crate) local: Arc<dyn Local>,
    pub(crate) app: Option<AppHandle>,
    pub(crate) timings: Timings,
    clock: Clock,
    files: Option<Files>,
    compiled: RwLock<Arc<Compiled>>,
    /// The registry's primary, read at each compile.
    main: RwLock<Option<String>>,
    families: AtomicU32,
    run: AtomicU8,
    away: RwLock<Option<Away>>,
    presence: AtomicBool,
    pip: AtomicBool,
    leaving: Mutex<HashMap<String, u32>>,
    any_leaving: AtomicBool,
    pub(crate) guards: Mutex<Guards>,
    pub(crate) counts: Mutex<Counts>,
    log: Mutex<Log>,
    emit_queue: Mutex<EmitQueue>,
    watchers: Mutex<BTreeSet<String>>,
    pub(crate) executor: Executor,
    pub(crate) claims: Arc<ClaimClient>,
    pub(crate) notifier: Notifier,
    spawner: Spawn,
    pending: Mutex<Pending>,
    recent_drops: Mutex<VecDeque<BadgeSeen>>,
    /// Other people's lines per chat key, and each Timer's last-seen value
    /// (D271): outside `Compiled`, so a recompile resets nothing.
    pub(crate) counters: timed::Counters,
    /// The timed machine's per-(rule, chat) phase, carried across compiles.
    pub(crate) timed: Mutex<timers::Machine>,
    /// Chats `JoinChat` keeps joined until `LeaveChat`, Off or restart.
    pub(crate) action_joins: Mutex<BTreeSet<String>>,
    /// When a window joined each chat (steady ms, wall ms): the Delay trigger's start. Engine-owned, never drained by reconcile.
    pub(crate) chat_joins: timed::ChatJoins,
    /// Door holds: a kind in a chat after a 429 (D273's own budgets).
    pub(crate) door_holds: Mutex<doors::Holds>,
    pub(crate) logfile: logfile::LogFile,
    /// P3: event keys of the own-channel notifications answered lately.
    own_seen: Mutex<SeenKeys>,
    /// P3: each own-channel rule's held text, by rule id (reconcile writes it).
    own_held: Mutex<HashMap<String, String>>,
    pub(crate) rec: Mutex<RecShared>,
    pub(crate) starting: AtomicBool,
    kept: Mutex<Vec<String>>,
    device: RwLock<Device>,
    refresh_tx: watch::Sender<u64>,
    compiles: watch::Sender<u64>,
    last_status: Mutex<Option<StatusSnapshot>>,
    counts_dirty: AtomicBool,
    activity_dirty: AtomicBool,
    pending_dirty: AtomicBool,
    #[cfg(test)]
    pub(crate) work: AtomicU64,
    #[cfg(test)]
    pub(crate) refresh_requests: AtomicU64,
}

/// One removal hook's hold on its accounts: while any guard names an id, no
/// rule sending as it or owned by it is admitted or sends. It also holds the
/// account store's account-change guard, so no linked account's token is
/// refreshed until the door is done. Hold it until the account door returns.
#[must_use = "bind it to `_leaving` and hold it until the account door returns"]
pub struct LeavingGuard {
    engine: Option<Arc<Engine>>,
    ids: Vec<String>,
    /// Dropped after the counts are given back (fields drop after `drop`).
    _change: Option<AccountChange>,
}

impl LeavingGuard {
    /// No engine running: the account-change guard alone.
    fn change_only(change: AccountChange) -> LeavingGuard {
        LeavingGuard { engine: None, ids: Vec::new(), _change: Some(change) }
    }
}

impl Drop for LeavingGuard {
    fn drop(&mut self) {
        let Some(engine) = self.engine.take() else { return };
        let mut map = lock(&engine.leaving);
        for id in &self.ids {
            if let Some(n) = map.get_mut(id) {
                *n -= 1;
                if *n == 0 {
                    map.remove(id);
                }
            }
        }
        engine.any_leaving.store(!map.is_empty(), Ordering::SeqCst);
    }
}

/// One rule's line on the page.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct RuleStatus {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub live: bool,
    /// False for a rule a newer StreamNook made: shown read-only.
    pub readable: bool,
    /// The typed rule, the editor's start; None exactly when `readable` is
    /// false. Serialized as an explicit `null`, so every client reads one shape.
    pub rule: Option<Rule>,
    pub summary: String,
    pub count: u64,
    pub last_fire_ms: Option<i64>,
    pub held: Option<String>,
    pub last_outcome: Option<String>,
}

/// An account on another platform the editor offers to send as: the
/// connected Kick account (YouTube and TikTok rules do not post).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlatformAccount {
    pub provider: String,
    pub id: String,
    pub login: String,
}

/// An account the pickers offer.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AccountChoice {
    pub id: String,
    pub login: String,
    /// The profile's Twitch account (the main account).
    pub main: bool,
}

/// Everything the page, the tray, the dot and `/automations` show.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StatusSnapshot {
    pub run: RunState,
    /// What Resume returns to; never Off.
    pub last_active: RunState,
    pub away: Option<Away>,
    /// What Set away starts from; empty before any was written.
    pub last_away_message: String,
    pub rules: Vec<RuleStatus>,
    pub device_name: String,
    /// Chats the rules keep joined.
    pub chats_kept: Vec<String>,
    /// `twitch:<login>` keys where an enabled Live rule with an outward action
    /// can act (the chat header's dot).
    pub live_chats: Vec<String>,
    /// A Live rule can act in any open chat.
    pub live_any: bool,
    /// Three claims in a row went unanswered: rules wait.
    pub outage: bool,
    /// streamnook.app asked the app to slow down.
    pub busy: bool,
    pub accounts: Vec<AccountChoice>,
    /// The connected Kick account, for a Kick rule's Send as.
    pub platform_accounts: Vec<PlatformAccount>,
    /// This device runs rules right now.
    pub foreground: bool,
    /// The Reminders group (P5): every reminder, in the order the settings hold them.
    pub reminders: Vec<crate::services::reminder_service::convert::ReminderSummary>,
}

/// A watched channel went live or offline, as `channel_state` saw it. The
/// start is Helix's `started_at` text (parsed to epoch seconds, so Helix and
/// GQL spellings give one key); for an offline edge it is the broadcast that
/// ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveEdge {
    pub login: String,
    pub channel_id: String,
    pub online: bool,
    pub started_at: String,
}

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();

/// The running engine, if `init` has run.
pub fn engine() -> Option<&'static Arc<Engine>> {
    ENGINE.get()
}

impl Engine {
    /// A stopped engine and the receiver its reconcile task reads. Nothing is
    /// spawned here.
    pub fn new(deps: Deps) -> (Arc<Engine>, watch::Receiver<u64>) {
        let (refresh_tx, refresh_rx) = watch::channel(0u64);
        let (compiles, _) = watch::channel(0u64);
        let log_file = if deps.files.is_some() { logfile::LogFile::real() } else { logfile::LogFile::none() };
        let claims = Arc::new(ClaimClient::new(deps.outward.clone(), &deps.device, deps.timings.claims, deps.spawn.clone()));
        let engine = Arc::new(Engine {
            outward: deps.outward,
            local: deps.local,
            app: deps.app,
            timings: deps.timings,
            clock: deps.clock,
            files: deps.files,
            compiled: RwLock::new(Arc::new(Compiled::empty())),
            main: RwLock::new(None),
            families: AtomicU32::new(0),
            run: AtomicU8::new(RUN_OFF),
            away: RwLock::new(None),
            presence: AtomicBool::new(cfg!(desktop)),
            pip: AtomicBool::new(false),
            leaving: Mutex::new(HashMap::new()),
            any_leaving: AtomicBool::new(false),
            guards: Mutex::new(Guards::new()),
            counts: Mutex::new(Counts::default()),
            log: Mutex::new(Log::default()),
            emit_queue: Mutex::new(EmitQueue::default()),
            watchers: Mutex::new(BTreeSet::new()),
            executor: Executor::new(),
            claims,
            notifier: Notifier::new(),
            spawner: deps.spawn,
            pending: Mutex::new(Pending::default()),
            recent_drops: Mutex::new(VecDeque::new()),
            counters: timed::Counters::default(),
            timed: Mutex::new(timers::Machine::default()),
            action_joins: Mutex::new(BTreeSet::new()),
            chat_joins: timed::ChatJoins::default(),
            door_holds: Mutex::new(doors::Holds::default()),
            logfile: log_file,
            own_seen: Mutex::new(SeenKeys::new()),
            own_held: Mutex::new(HashMap::new()),
            rec: Mutex::new(RecShared::default()),
            starting: AtomicBool::new(false),
            kept: Mutex::new(Vec::new()),
            device: RwLock::new(deps.device),
            refresh_tx,
            compiles,
            last_status: Mutex::new(None),
            counts_dirty: AtomicBool::new(false),
            activity_dirty: AtomicBool::new(false),
            pending_dirty: AtomicBool::new(false),
            #[cfg(test)]
            work: AtomicU64::new(0),
            #[cfg(test)]
            refresh_requests: AtomicU64::new(0),
        });
        (engine, refresh_rx)
    }

    /// Counts work done past the families check, so a test can prove Off does none.
    #[inline]
    fn note_work(&self) {
        #[cfg(test)]
        self.work.fetch_add(1, Ordering::Relaxed);
    }

    /// Steady milliseconds.
    pub(crate) fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Run `task` on the engine's spawner, detached.
    pub(crate) fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        (self.spawner)(Box::pin(task));
    }

    /// Run `task` on the engine's spawner, answering a handle that stops it.
    pub(crate) fn spawn_abortable(&self, task: impl Future<Output = ()> + Send + 'static) -> AbortHandle {
        let (handle, registration) = AbortHandle::new_pair();
        self.spawn(async move {
            let _ = Abortable::new(task, registration).await;
        });
        handle
    }

    /// The rules as last compiled.
    pub fn compiled(&self) -> Arc<Compiled> {
        read(&self.compiled)
    }

    pub fn run_state(&self) -> RunState {
        run_from(self.run.load(Ordering::SeqCst))
    }

    /// This device's away, as the last save left it.
    pub fn away(&self) -> Option<Away> {
        read(&self.away)
    }

    /// Rules run while the app is in front (always on desktop) or in
    /// picture in picture.
    pub fn foreground(&self) -> bool {
        self.presence.load(Ordering::SeqCst) || self.pip.load(Ordering::SeqCst)
    }

    pub fn device_name(&self) -> String {
        read(&self.device).name
    }

    /// Mark the reconcile channel; the loop reads the newest mark.
    pub fn request_refresh(&self) {
        #[cfg(test)]
        self.refresh_requests.fetch_add(1, Ordering::SeqCst);
        self.refresh_tx.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// A receiver that changes after every compile (tests await it).
    pub fn compiled_events(&self) -> watch::Receiver<u64> {
        self.compiles.subscribe()
    }

    /// The main account's id: the registry's primary as the last compile read
    /// it, so the chat lane never reads the registry file.
    pub(crate) fn main_id(&self) -> Option<String> {
        read(&self.main)
    }

    /// Who you are on each other platform (Kick, YouTube, TikTok): local
    /// reads, so a compile never waits on a network.
    pub(crate) fn platform_ids(&self) -> Vec<PlatformId> {
        [provider_facts::KICK, provider_facts::YOUTUBE, provider_facts::TIKTOK]
            .into_iter()
            .filter_map(|provider| {
                self.outward
                    .platform_identity(provider)
                    .map(|(id, login)| PlatformId { provider: provider.to_string(), id, login })
            })
            .collect()
    }

    /// The accounts the editor offers to send as on another platform: the
    /// connected Kick account.
    pub(crate) fn platform_accounts(&self) -> Vec<PlatformAccount> {
        let Some(id) = self.outward.kick_account_id() else { return Vec::new() };
        let login = self.outward.platform_identity(provider_facts::KICK).map(|(_, login)| login).unwrap_or_default();
        vec![PlatformAccount { provider: provider_facts::KICK.to_string(), id, login }]
    }

    /// Compile the rules from the live settings and your accounts, publish
    /// them, and answer them. Reconcile is the only caller in the app. The
    /// accounts are the registry's and the profile's, both local reads: a
    /// compile never waits on the network.
    pub(crate) async fn recompile(self: &Arc<Self>) -> Arc<Compiled> {
        let own = self.outward.own_accounts();
        let profile = self.outward.profile_user_id();
        write(&self.main, self.outward.primary_id());
        let primary = read(&self.main);
        let (automations, prefs) = self.local.settings();
        let compiled = Arc::new(compile::compile_with(
            &CompileInput {
                automations: &automations,
                prefs: &prefs,
                own_accounts: &own,
                profile_user_id: profile.as_deref(),
                primary_id: primary.as_deref(),
            },
            &self.platform_ids(),
        ));
        write(&self.away, automations.away.clone());
        self.run.store(run_code(automations.run), Ordering::SeqCst);
        write(&self.compiled, compiled.clone());
        let families = if self.foreground() { compiled.families } else { 0 };
        self.families.store(families, Ordering::Relaxed);
        let ids: HashSet<String> = compiled.rules.iter().map(|r| r.rule.id.clone()).collect();
        lock(&self.counts).retain_rules(&ids);
        lock(&self.guards).retain_rules(&ids);
        self.compiles.send_modify(|n| *n = n.wrapping_add(1));
        compiled
    }

    /// Set this device's away at once (the save that stores it follows).
    pub fn set_away_now(&self, away: Option<Away>) {
        write(&self.away, away);
        self.announce_status();
    }

    /// Set the run state at once. Off and Test stop every job in flight.
    pub fn set_run_now(&self, run: RunState) {
        self.run.store(run_code(run), Ordering::SeqCst);
        if run != RunState::On {
            self.executor.bump();
        }
        if run == RunState::Off {
            self.families.store(0, Ordering::Relaxed);
            lock(&self.action_joins).clear();
        }
        self.request_refresh();
    }

    pub(crate) fn match_ctx<'a>(&self, away: Option<&'a Away>, trace: bool) -> matcher::Ctx<'a> {
        matcher::Ctx { away, local: matcher::LocalTime::from_naive(chrono::Local::now().naive_local()), trace }
    }

    // ---- the chat path ----------------------------------------------------

    /// A live chat row (PRIVMSG or USERNOTICE) after the chat rules judged it.
    pub fn on_chat(self: &Arc<Self>, msg: &ChatMessage, dropped: bool) {
        if self.families.load(Ordering::Relaxed) & (compile::FAMILY_CHAT | compile::FAMILY_TIMED) == 0 {
            return;
        }
        self.note_work();
        if !self.foreground() {
            return;
        }
        let compiled = self.compiled();
        if msg.provider == "twitch" {
            if compiled.is_mine(&msg.user_id) {
                if self.main_id().as_deref() != Some(msg.user_id.as_str()) {
                    let badges = msg.tags.get("badges").map(String::as_str).unwrap_or("");
                    crate::services::chat_send_limiter::note_alt_badges(&msg.user_id, &msg.channel, badges);
                }
                return;
            }
        } else if !self.provider_row_ok(&compiled, msg) {
            return;
        }
        let Some(mut facts) = facts::from_chat(msg) else { return };
        if !guards::row_fresh(&facts, dropped, now_ms()) {
            return;
        }
        // A Kick event has no shared id: one device that reads one frame twice answers once.
        if facts.event_key.starts_with("kick_evt:") && !lock(&self.guards).first_kick_event(&facts.event_key, self.now()) {
            return;
        }
        // "Messages since" counts other people's live lines (not yours, alts,
        // known bots or backfill; `row_fresh` removed backfill, `is_mine`
        // returned above).
        if self.families.load(Ordering::Relaxed) & compile::FAMILY_TIMED != 0 && !facts.shared_partner && !sn_mention_rule::judge::is_known_bot(&facts.user_login, &facts.user_name.to_lowercase()) {
            self.counters.bump(&facts.channel_key);
        }
        if compiled.expressions {
            facts.message = Some(Arc::new(msg.clone()));
        }
        let now = self.now();
        if lock(&self.guards).is_echo(&facts.channel_key, &facts.text, now) {
            return;
        }
        self.admit(&compiled, Arc::new(facts), now);
    }

    /// A whisper to `owner_id` (called by `whisper_inbox::record_incoming`,
    /// the only caller of the socket's whisper handler; never blocks filing):
    /// the gates and the admission run on the engine's spawner (`Engine::spawn`,
    /// S7.3a: Tauri's runtime in the app, the test's own in tests).
    pub fn on_whisper(self: &Arc<Self>, owner_id: &str, ev: &crate::services::whisper_service::WhisperEvent, twitch_ms: i64) {
        let Some((compiled, facts, wall)) = self.whisper_inputs(owner_id, ev) else { return };
        let engine = self.clone();
        self.spawn(async move { timed::admit_whisper_now(&engine, &compiled, facts, twitch_ms, wall).await });
    }

    /// The same, awaited (tests drive whisper admission without a sleep).
    pub async fn on_whisper_now(self: &Arc<Self>, owner_id: &str, ev: &crate::services::whisper_service::WhisperEvent, twitch_ms: i64) {
        let Some((compiled, facts, wall)) = self.whisper_inputs(owner_id, ev) else { return };
        timed::admit_whisper_now(self, &compiled, facts, twitch_ms, wall).await;
    }

    fn whisper_inputs(&self, owner_id: &str, ev: &crate::services::whisper_service::WhisperEvent) -> Option<(Arc<Compiled>, Arc<Facts>, i64)> {
        if self.families.load(Ordering::Relaxed) & compile::FAMILY_WHISPER == 0 {
            return None;
        }
        self.note_work();
        if !self.foreground() {
            return None;
        }
        let facts = Arc::new(facts::from_whisper(owner_id, &ev.whisper_id, &ev.from_user_id, &ev.from_user_login, &ev.from_user_name, &ev.text, now_ms()));
        let wall = crate::services::whisper_sync::twitch_now(chrono::Utc::now().timestamp_millis());
        Some((self.compiled(), facts, wall))
    }

    /// A row of another platform may fire rules only when it is a platform
    /// the engine reads, you are signed in there (without it your own
    /// messages cannot be told from anyone's), and it is not yours. Your
    /// ids on that platform are the per-provider set, never the Twitch one.
    fn provider_row_ok(&self, compiled: &Compiled, msg: &ChatMessage) -> bool {
        let Some(provider) = provider_facts::provider_of(msg) else { return false };
        let Some((id, login)) = self.outward.platform_identity(provider) else { return false };
        let me = PlatformId { provider: provider.to_string(), id, login };
        !provider_facts::is_own_row(compiled, provider, msg, &me)
    }

    /// Match one row against every rule it can reach and admit each that fired.
    pub(crate) fn admit(self: &Arc<Self>, compiled: &Arc<Compiled>, facts: Arc<Facts>, now: i64) {
        let away = self.away();
        let ctx = self.match_ctx(away.as_ref(), false);
        for verdict in matcher::evaluate(compiled, &facts, &ctx) {
            if verdict.fired {
                self.admit_rule(compiled, verdict.rule, facts.clone(), Vec::new(), HashMap::new(), now, None);
            }
        }
    }

    /// Admit one fired rule: account checks, its classes, the local rails and
    /// the breaker, then a job for the executor.
    pub(crate) fn admit_rule(
        self: &Arc<Self>,
        compiled: &Arc<Compiled>,
        index: usize,
        facts: Arc<Facts>,
        drops: Vec<BadgeSeen>,
        links: HashMap<String, Option<String>>,
        now: i64,
        bound: Option<Duration>,
    ) {
        let cr = &compiled.rules[index];
        let Some(mode) = cr.mode else { return };
        let live = mode == Mode::Live;
        if let Some(reason) = self.refusal(compiled, &cr.rule) {
            self.skip(&cr.rule, &facts.channel_key, &reason);
            return;
        }
        let room = claims::event_room(&cr.rule, &facts).to_string();
        let (plans, unready) = claims::classes_for(&cr.rule, &room);
        let lanes = guards::lanes_for(&cr.rule, &plans);
        let person = (drops.is_empty() && !facts.user_id.is_empty()).then(|| facts.user_id.clone());
        let admitted = lock(&self.guards).admit(&cr.rule, &lanes, &room, person.as_deref(), now);
        match admitted {
            Admit::Skipped { reason, .. } => self.skip(&cr.rule, &facts.channel_key, &reason),
            Admit::Admitted { lanes: open } => {
                let trip = lock(&self.guards).note_admitted(live, compiled.prefs.sends_per_minute, now);
                if let Some(trip) = trip {
                    self.breaker_tripped(trip);
                    if trip == Trip::TurnOff {
                        return;
                    }
                }
                let mut job = Job::new(self, compiled.clone(), index, facts, drops, links, live, plans, unready, open, room, now);
                if let Some(bound) = bound {
                    job.age_bound = bound;
                }
                self.executor.submit(self, job);
            }
        }
    }

    /// Why a rule may not act for its accounts right now, if it may not.
    fn refusal(&self, compiled: &Compiled, rule: &Rule) -> Option<String> {
        // Only a Twitch account is in the registry, the removal counter and
        // the verifier; a Kick id equal to one of them must not match it.
        let twitch: Vec<&crate::models::automations::AccountRef> =
            [&rule.send_as, &rule.owner].into_iter().filter(|a| provider_facts::is_twitch_account(a)).collect();
        if self.any_leaving.load(Ordering::SeqCst) {
            let map = lock(&self.leaving);
            if twitch.iter().any(|a| map.contains_key(a.id.as_str())) {
                return Some("Account being removed".into());
            }
        }
        for account in twitch {
            if compiled.login(&account.id).is_none() {
                return Some(sign_in(&account.login));
            }
            if let Some(why) = self.outward.verifier().remembered_failure(&account.id) {
                return Some(token_words(&why, &account.login));
            }
        }
        if crate::models::automations::is_kick_sender(rule)
            && self.outward.kick_account_id().as_deref() != Some(rule.send_as.id.as_str())
        {
            return Some(executor::CONNECT_KICK.into());
        }
        None
    }

    /// Whether `id` is being removed, read under the lock (never the flag).
    pub(crate) fn is_leaving(&self, id: &str) -> bool {
        lock(&self.leaving).contains_key(id)
    }

    /// The token for `id`, only when Twitch says it is `id`'s, within `budget`
    /// (the caller's remaining time; past it the answer is `Unreachable`).
    /// While an account is being removed it is `Unreachable` with no fetch.
    pub(crate) async fn verified(&self, id: &str, budget: Duration) -> Result<VerifiedToken, TokenRefusal> {
        self.outward.verified_token(id, budget).await
    }

    /// Twitch refused `token` for `account_id` on a send (a 401): its cached
    /// owner goes and the account's failure is remembered for a minute.
    pub(crate) fn token_unauthorized(&self, account_id: &str, token: &str) {
        let verifier = self.outward.verifier();
        verifier.drop_token_of(token);
        verifier.record_send_401(account_id);
    }

    /// An account save or a profile identity change: remembered failures are
    /// forgotten, a chat start may try again at once, the rules recompile.
    pub fn accounts_changed(&self) {
        self.outward.verifier().forget_failures();
        lock(&self.rec).reset_backoff = true;
        lock(&self.guards).clear_save_holds();
        // Fresh before the recompile, so a booking in between uses the new main.
        write(&self.main, self.outward.primary_id());
        self.request_refresh();
    }

    /// A NOTICE for the main account (IRC is the main's) in `channel`.
    pub fn on_notice(&self, channel: &str, msg_id: &str, message: &str) {
        if self.families.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(main) = self.main_id() else { return };
        let chat_key = crate::services::chat_rules::channel_filter_key("twitch", channel);
        lock(&self.guards).hold_after_notice(&main, &chat_key, msg_id, message, self.now());
    }

    /// A CLEARCHAT in `channel`: a timeout or ban of one of your accounts
    /// holds that account there.
    pub fn on_clearchat(&self, channel: &str, target_user_id: Option<&str>, ban_duration_s: Option<i64>) {
        if self.families.load(Ordering::Relaxed) == 0 {
            return;
        }
        let Some(target) = target_user_id else { return };
        let compiled = self.compiled();
        let chat_key = crate::services::chat_rules::channel_filter_key("twitch", channel);
        lock(&self.guards).hold_after_clearchat(compiled.twitch_ids(), target, &chat_key, ban_duration_s, self.now());
    }

    // ---- badges -------------------------------------------------------------

    /// A badge sighting from the live feed.
    pub fn on_badges(self: &Arc<Self>, seen: BadgeSeen) {
        {
            let mut recent = lock(&self.recent_drops);
            recent.push_front(seen.clone());
            recent.truncate(BADGE_SAMPLES);
        }
        if self.families.load(Ordering::Relaxed) & compile::FAMILY_BADGE == 0 || !self.foreground() {
            return;
        }
        // A frame stale by the feed's clock, or a badge the gallery already
        // had, never reaches a rule.
        let offset = crate::services::badge_feed::server_offset_ms();
        if badges::screen(&seen, offset) != badges::Screen::Keep {
            return;
        }
        let room_len = self.compiled().badge_room_len;
        let closed = lock(&self.pending).batcher.push(seen, now_ms(), room_len);
        self.note_pending_changed();
        if let Some(batch) = closed {
            let engine = self.clone();
            self.spawn(async move { engine.run_batch(batch).await });
        }
    }

    /// The open batch, when its time is up (reconcile's batcher clock).
    pub(crate) fn tick_batcher(self: &Arc<Self>) {
        let closed = lock(&self.pending).batcher.tick(now_ms());
        if let Some(batch) = closed {
            self.note_pending_changed();
            let engine = self.clone();
            self.spawn(async move { engine.run_batch(batch).await });
        }
    }

    /// Recent drops for "Try with", newest first.
    pub fn badge_samples(&self) -> Vec<BadgeSeen> {
        lock(&self.recent_drops).iter().cloned().collect()
    }

    /// Badges a gallery refresh added that its previous copy lacked.
    pub fn note_arrivals(&self, pairs: Vec<(String, String)>) {
        if pairs.is_empty() {
            return;
        }
        lock(&self.pending).arrivals.record(pairs, now_ms());
        self.note_pending_changed();
    }

    /// The gallery gained this badge within the last 30 minutes.
    pub fn is_recent_arrival(&self, set_id: &str, version: &str) -> bool {
        lock(&self.pending).arrivals.is_recent(set_id, version, now_ms())
    }

    /// A closed batch: each running badge rule takes the drops it wants and has
    /// not posted here, resolves their links (up to 10 minutes), and is
    /// admitted with them.
    pub(crate) async fn run_batch(self: &Arc<Self>, batch: Batch) {
        let compiled = self.compiled();
        for &index in &compiled.badge_rules {
            let cr = &compiled.rules[index];
            if cr.mode.is_none() {
                continue;
            }
            let Trigger::BadgeDrop { on, link, .. } = &cr.rule.when else { continue };
            let drops: Vec<BadgeSeen> = {
                let counts = lock(&self.counts);
                let id = cr.rule.id.clone();
                badges::drops_for(&batch, *on, |feed, phase| counts.has_posted(&id, feed, phase), now_ms())
                    .into_iter()
                    .cloned()
                    .collect()
            };
            if drops.is_empty() {
                continue;
            }
            let engine = self.clone();
            let compiled = compiled.clone();
            let link = link.clone();
            self.spawn(async move {
                let links = engine.resolve_links(&link, &drops).await;
                let facts = Arc::new(facts::from_badge(&drops[0]));
                let away = engine.away();
                let ctx = engine.match_ctx(away.as_ref(), false);
                let verdict = matcher::evaluate_rule(&compiled, index, &facts, &ctx);
                if verdict.fired {
                    engine.admit_rule(&compiled, index, facts, drops, links, engine.now(), None);
                } else if let Some(reason) = verdict.reason {
                    engine.skip(&compiled.rules[index].rule, "", &reason);
                }
            });
        }
    }

    /// Each drop's link once it answers 200, else the rule's fallback after
    /// 10 minutes (checked together, so a batch waits for its slowest link).
    async fn resolve_links(&self, link: &BadgeLink, drops: &[BadgeSeen]) -> HashMap<String, Option<String>> {
        let checks = drops.iter().map(|d| async move { (d.feed_id.clone(), self.link_for_drop(link, d).await) });
        futures::future::join_all(checks).await.into_iter().collect()
    }

    async fn link_for_drop(&self, link: &BadgeLink, drop: &BadgeSeen) -> Option<String> {
        let url = badges::link_for(link, &drop.badge)?;
        let Ok(wait) = LinkWait::start(url, now_ms()) else {
            return badges::fallback_link(link, &drop.badge);
        };
        loop {
            let probe = self.local.probe_link(&wait.url).await;
            match wait.on_probe(probe, now_ms()) {
                LinkStep::Ready => return Some(wait.url.clone()),
                LinkStep::RetryAt(at) => tokio::time::sleep(Duration::from_millis((at - now_ms()).max(0) as u64)).await,
                LinkStep::GiveUp => return badges::fallback_link(link, &drop.badge),
            }
        }
    }

    // ---- account removal ----------------------------------------------------

    /// Count `ids` as leaving and build the guard that undoes it. The flag is
    /// written inside the same lock, so it can never read false while an id is
    /// in the map.
    fn enter_leaving(self: &Arc<Self>, ids: &[String], change: AccountChange) -> LeavingGuard {
        let mut map = lock(&self.leaving);
        for id in ids {
            *map.entry(id.clone()).or_insert(0) += 1;
        }
        self.any_leaving.store(!map.is_empty(), Ordering::SeqCst);
        drop(map);
        LeavingGuard { engine: Some(self.clone()), ids: ids.to_vec(), _change: Some(change) }
    }

    #[cfg(test)]
    pub(crate) fn leaving_count(&self, id: &str) -> u32 {
        lock(&self.leaving).get(id).copied().unwrap_or(0)
    }

    #[cfg(test)]
    pub(crate) fn any_leaving(&self) -> bool {
        self.any_leaving.load(Ordering::SeqCst)
    }

    // ---- activity, notes and status -------------------------------------------

    /// Log one line; watchers hear it within a quarter second.
    pub(crate) fn log(self: &Arc<Self>, new: NewEntry) {
        let entry = lock(&self.log).push(new, now_ms());
        self.activity_dirty.store(true, Ordering::SeqCst);
        self.schedule_save(false);
        let schedule = {
            let mut q = lock(&self.emit_queue);
            q.entries.retain(|e| e.id != entry.id);
            q.entries.push(entry);
            !std::mem::replace(&mut q.scheduled, true)
        };
        if schedule {
            let engine = self.clone();
            self.spawn(async move {
                tokio::time::sleep(engine.timings.activity_emit).await;
                engine.emit_activity();
            });
        }
    }

    fn emit_activity(&self) {
        let entries = {
            let mut q = lock(&self.emit_queue);
            q.scheduled = false;
            std::mem::take(&mut q.entries)
        };
        if entries.is_empty() {
            return;
        }
        let watchers: Vec<String> = lock(&self.watchers).iter().cloned().collect();
        self.local.emit_activity(&watchers, &entries);
    }

    /// A rule skipped: Activity in the words given.
    pub(crate) fn skip(self: &Arc<Self>, rule: &Rule, channel: &str, reason: &str) {
        self.log(NewEntry {
            kind: EntryKind::Skipped,
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            channel: channel.to_string(),
            text: reason.to_string(),
        });
    }

    /// A line only you see in `login`'s chat, at most one per rule per chat
    /// per 30 s, when chat notes are on.
    pub(crate) async fn chat_note(&self, rule_id: &str, login: &str, text: &str) {
        if !self.compiled().prefs.chat_notes {
            return;
        }
        let chat_key = crate::services::chat_rules::channel_filter_key("twitch", login);
        if !lock(&self.guards).note_ok(rule_id, &chat_key, self.now()) {
            return;
        }
        self.local.room_notice(login, text).await;
    }

    /// The window `label` wants Activity lines (or stops wanting them).
    pub fn watch_activity(&self, label: &str, on: bool) {
        let mut watchers = lock(&self.watchers);
        if on {
            watchers.insert(label.to_string());
        } else {
            watchers.remove(label);
        }
    }

    /// A destroyed window hears nothing more.
    pub fn forget_window(&self, label: &str) {
        lock(&self.watchers).remove(label);
    }

    pub fn activity_list(&self, filter: Filter, limit: usize, before: Option<u64>) -> Vec<Entry> {
        lock(&self.log).query(filter, limit, before)
    }

    pub fn clear_activity(&self) {
        lock(&self.log).clear();
        self.activity_dirty.store(true, Ordering::SeqCst);
        self.flush_files();
        self.announce_status();
    }

    pub(crate) fn set_kept(&self, rooms: Vec<String>) {
        *lock(&self.kept) = rooms;
    }

    /// The page's snapshot.
    pub fn status(&self) -> StatusSnapshot {
        let compiled = self.compiled();
        let now = self.now();
        // Read before the engine's locks are taken (the settings lock is never held under them).
        let (group, _) = self.local.settings();
        let reminders = self.local.reminders();
        let guards = lock(&self.guards);
        let counts = lock(&self.counts);
        let log = lock(&self.log);
        let mut rules: Vec<(usize, RuleStatus)> = compiled
            .rules
            .iter()
            .map(|cr| {
                let id = cr.rule.id.clone();
                let held = cr
                    .held
                    .clone()
                    .or_else(|| self.own_held_for(&id))
                    .or_else(|| guards.rate_held_until(&id, now).map(|_| guards::RATE_HELD.to_string()));
                let status = RuleStatus {
                    name: cr.rule.name.clone(),
                    enabled: cr.rule.enabled,
                    live: cr.rule.live,
                    readable: true,
                    rule: Some(cr.rule.clone()),
                    summary: cr.summary.clone(),
                    count: counts.count(&id),
                    last_fire_ms: counts.rule(&id).map(|r| r.last_fire_ms).filter(|ms| *ms > 0),
                    held,
                    last_outcome: log.last_for_rule(&id).map(|e| e.text.clone()),
                    id,
                };
                (cr.position, status)
            })
            .collect();
        for u in &compiled.unreadable {
            rules.push((
                u.position,
                RuleStatus {
                    id: u.id.clone(),
                    name: u.name.clone(),
                    enabled: false,
                    live: false,
                    readable: false,
                    rule: None,
                    summary: compile::NEWER_BUILD.to_string(),
                    count: 0,
                    last_fire_ms: None,
                    held: Some(u.reason.clone()),
                    last_outcome: None,
                },
            ));
        }
        rules.sort_by_key(|(position, _)| *position);
        let main = self.main_id();
        let mut accounts: Vec<AccountChoice> = compiled
            .logins
            .iter()
            .map(|(id, login)| AccountChoice {
                id: id.clone(),
                login: login.clone(),
                main: main.as_deref() == Some(id.as_str()),
            })
            .collect();
        accounts.sort_by(|a, b| b.main.cmp(&a.main).then_with(|| a.login.cmp(&b.login)));
        StatusSnapshot {
            run: self.run_state(),
            last_active: group.last_active,
            away: self.away(),
            last_away_message: group.last_away_message,
            rules: rules.into_iter().map(|(_, r)| r).collect(),
            device_name: self.device_name(),
            chats_kept: lock(&self.kept).clone(),
            live_chats: compiled.live_chats.clone(),
            live_any: compiled.live_any,
            outage: self.claims.outage(),
            busy: self.claims.busy(),
            accounts,
            platform_accounts: self.platform_accounts(),
            foreground: self.foreground(),
            reminders,
        }
    }

    /// Emit the snapshot when it changed since the last emit.
    pub(crate) fn announce_status(&self) {
        let snapshot = self.status();
        {
            let mut last = lock(&self.last_status);
            if last.as_ref() == Some(&snapshot) {
                return;
            }
            *last = Some(snapshot.clone());
        }
        self.local.emit_status(&snapshot);
    }

    // ---- P3: your channel and live changes ---------------------------------------

    /// A notification from one of the own-channel topics the whisper session
    /// holds. The events are the main account's (the session's user): the
    /// rows are built in its channel and room, and only rules it owns match.
    pub fn on_own_topic(self: &Arc<Self>, kind: TopicKind, event: &serde_json::Value) {
        if self.families.load(Ordering::Relaxed) & compile::FAMILY_OWN == 0 {
            return;
        }
        self.note_work();
        if !self.foreground() {
            return;
        }
        let compiled = self.compiled();
        let Some(owner) = self.main_id() else { return };
        let Some(login) = compiled.login(&owner).map(str::to_string) else { return };
        let Some(facts) = facts::from_own(kind, event, &owner, &login, now_ms()) else { return };
        // Twitch may redeliver; an event is answered once on this device.
        if !lock(&self.own_seen).first_time(&facts.event_key) {
            return;
        }
        let now = self.now();
        self.admit(&compiled, Arc::new(facts), now);
    }

    /// A watched channel went live or offline (both `channel_state` writers).
    /// Online: skipped when the broadcast started more than 10 minutes ago by
    /// Twitch's clock. Both: once per broadcast, kept across restarts.
    pub fn on_live_edge(self: &Arc<Self>, edge: LiveEdge) {
        if self.families.load(Ordering::Relaxed) & compile::FAMILY_LIVE == 0 {
            return;
        }
        self.note_work();
        if !self.foreground() {
            return;
        }
        let Some(started_s) = facts::started_epoch(&edge.started_at) else { return };
        let compiled = self.compiled();
        let key = crate::services::chat_rules::channel_filter_key("twitch", &edge.login);
        if !compiled.live_index.contains_key(&key) {
            return;
        }
        let wall = now_ms();
        if edge.online && !facts::online_edge_fresh(started_s, self.local.twitch_now(wall)) {
            return;
        }
        if !lock(&self.counts).note_live_edge(&edge.login, edge.online, started_s) {
            return;
        }
        self.note_counts_changed();
        let facts = facts::from_live(&edge.login, &edge.channel_id, edge.online, started_s, wall);
        let now = self.now();
        self.admit(&compiled, Arc::new(facts), now);
    }

    /// Reconcile worked out each own-channel rule's held text (a topic
    /// refused, no socket, not the main account); an empty map clears them.
    pub(crate) fn set_own_held(&self, holds: HashMap<String, String>) {
        *lock(&self.own_held) = holds;
    }

    pub(crate) fn own_held_for(&self, rule_id: &str) -> Option<String> {
        lock(&self.own_held).get(rule_id).cloned()
    }

    /// The breaker tripped: Live turns automations off; Test only says so.
    fn breaker_tripped(self: &Arc<Self>, trip: Trip) {
        let live = trip == Trip::TurnOff;
        self.log(NewEntry {
            kind: if live { EntryKind::Notice } else { EntryKind::Test },
            rule_id: String::new(),
            rule_name: String::new(),
            channel: String::new(),
            text: trip.words().to_string(),
        });
        if !live {
            return;
        }
        self.set_run_now(RunState::Off);
        let engine = self.clone();
        self.spawn(async move {
            engine.local.turn_off().await;
            notify::deliver(
                &engine,
                NotifyRequest {
                    rule_id: None,
                    title: "Automations turned off".into(),
                    body: "Rules were firing too fast".into(),
                    toast: true,
                    system: true,
                    sound: None,
                    volume: 1.0,
                    channel: String::new(),
                    settings_link: true,
                },
            )
            .await;
        });
    }

    // ---- files ------------------------------------------------------------------

    /// Read the files next to settings.json; an unreadable one is set aside.
    pub(crate) fn restore_files(&self) {
        let Some(files) = &self.files else { return };
        if let Some(text) = files.read(COUNTS_FILE) {
            match Counts::from_json(&text) {
                Ok(counts) => *lock(&self.counts) = counts,
                Err(e) => files.set_aside(COUNTS_FILE, &e),
            }
        }
        if let Some(text) = files.read(ACTIVITY_FILE) {
            match Log::from_json(&text) {
                Ok(log) => *lock(&self.log) = log,
                Err(e) => files.set_aside(ACTIVITY_FILE, &e),
            }
        }
        if let Some(text) = files.read(PENDING_FILE) {
            match Pending::from_json(&text) {
                Ok(mut pending) => {
                    pending.prune(now_ms());
                    *lock(&self.pending) = pending;
                }
                Err(e) => files.set_aside(PENDING_FILE, &e),
            }
        }
    }

    fn schedule_save(self: &Arc<Self>, counts: bool) {
        if self.files.is_none() {
            return;
        }
        let delay = if counts { self.timings.counts_flush } else { self.timings.activity_flush };
        let engine = self.clone();
        self.spawn(async move {
            tokio::time::sleep(delay).await;
            engine.flush_files();
        });
    }

    pub(crate) fn note_counts_changed(self: &Arc<Self>) {
        if !self.counts_dirty.swap(true, Ordering::SeqCst) {
            self.schedule_save(true);
        }
    }

    /// The batcher's state changed: write the pending store.
    pub(crate) fn note_pending_changed(&self) {
        self.pending_dirty.store(true, Ordering::SeqCst);
        self.save_pending();
    }

    fn save_pending(&self) {
        let Some(files) = &self.files else { return };
        if self.pending_dirty.swap(false, Ordering::SeqCst) {
            let text = lock(&self.pending).to_json();
            files.write(PENDING_FILE, &text);
        }
    }

    /// Write every changed file now (exit, phone background, a timer).
    pub fn flush_files(&self) {
        let Some(files) = &self.files else { return };
        if self.counts_dirty.swap(false, Ordering::SeqCst) {
            let text = lock(&self.counts).to_json();
            files.write(COUNTS_FILE, &text);
        }
        if self.activity_dirty.swap(false, Ordering::SeqCst) {
            let text = lock(&self.log).to_json();
            files.write(ACTIVITY_FILE, &text);
        }
        self.save_pending();
        crate::services::whisper_recipients::flush();
        self.logfile.flush();
    }

    /// Rename this device; other devices see it when they lose a claim.
    pub fn set_device_name(&self, name: &str) -> Result<String, String> {
        let name = store::clean_device_name(name)?;
        let mut device = read(&self.device);
        device.name = name.clone();
        if let Some(files) = &self.files {
            files.save_device(&device)?;
        }
        write(&self.device, device);
        self.claims.set_device_name(&name);
        self.announce_status();
        Ok(name)
    }
}

pub(crate) fn sign_in(login: &str) -> String {
    format!("Sign in as @{login} to run this rule")
}

/// The words for a token that could not be used for `login`.
pub(crate) fn token_words(why: &TokenRefusal, login: &str) -> String {
    match why {
        TokenRefusal::Unreachable(_) => format!("Couldn't reach Twitch to check @{login}; skipped"),
        TokenRefusal::WrongAccount { .. } => {
            format!("Sign in as @{login} again: Twitch says this sign-in belongs to another account")
        }
        _ => sign_in(login),
    }
}

/// Whether removing `id` should hold its rules first: always, unless the
/// registry (`(id, is_main)` pairs) shows `id` as the main account, which the
/// remove door refuses.
pub fn remove_needs_hook(registry: &[(String, bool)], id: &str) -> bool {
    !registry.iter().any(|(account, main)| account == id && *main)
}

/// The accounts a sign-out removes: the registry's main, and the profile's
/// account unless the registry names it as a linked account (which stays),
/// each once.
pub fn sign_out_ids(registry_main: Option<&str>, profile_id: Option<&str>, linked: &[String]) -> Vec<String> {
    let profile = profile_id.filter(|id| !linked.iter().any(|l| l == id));
    let mut ids: Vec<String> = Vec::new();
    for id in [registry_main, profile].into_iter().flatten().filter(|id| !id.is_empty()) {
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
    }
    ids
}

/// Start the engine. The watch channel and the singleton exist before
/// reconcile is spawned, so a refresh requested at any moment is never lost;
/// the first compile runs inside reconcile's loop. The verifier spawns each
/// validate on the app's runtime, captured here (setup runs outside it).
pub fn init(app: &AppHandle) {
    if ENGINE.get().is_some() {
        return;
    }
    // The iPhone core's headless tauri already hands back tokio's handle.
    #[cfg(not(target_os = "ios"))]
    let runtime = tauri::async_runtime::handle().inner().clone();
    #[cfg(target_os = "ios")]
    let runtime = tauri::async_runtime::handle();
    let files = Files::open();
    let device = files.as_ref().map(Files::load_or_create_device).unwrap_or_else(Device::fresh);
    let spawn = tauri_spawner();
    let (engine, refresh_rx) = Engine::new(Deps {
        outward: Arc::new(outward::RealOutward::new(runtime)),
        local: Arc::new(local::RealLocal::new(app.clone(), spawn.clone())),
        app: Some(app.clone()),
        files,
        device,
        timings: Timings::production(),
        clock: steady_clock(),
        spawn,
    });
    engine.restore_files();
    crate::services::whisper_recipients::ensure_loaded();
    if ENGINE.set(engine.clone()).is_err() {
        return;
    }
    timed::start(&engine);
    // Runs for the app's life; its stop handle is not needed.
    let _ = reconcile::spawn(engine, refresh_rx);
}

// ---- free entry points: each is a no-op before init -------------------------

/// The chat lane's hook, once per live row after the chat rules ran.
pub fn on_chat(msg: &ChatMessage, dropped: bool) {
    if let Some(engine) = ENGINE.get() {
        engine.on_chat(msg, dropped);
    }
}

/// An IRC NOTICE for the main account.
pub fn on_notice(channel: &str, msg_id: &str, message: &str) {
    if let Some(engine) = ENGINE.get() {
        engine.on_notice(channel, msg_id, message);
    }
}

/// An IRC CLEARCHAT.
pub fn on_clearchat(channel: &str, target_user_id: Option<&str>, ban_duration_s: Option<i64>) {
    if let Some(engine) = ENGINE.get() {
        engine.on_clearchat(channel, target_user_id, ban_duration_s);
    }
}

/// A live badge sighting.
pub fn on_badges(seen: BadgeSeen) {
    if let Some(engine) = ENGINE.get() {
        engine.on_badges(seen);
    }
}

/// A notification from an own-channel topic the whisper session holds.
pub fn on_own_topic(kind: TopicKind, event: &serde_json::Value) {
    if let Some(engine) = ENGINE.get() {
        engine.on_own_topic(kind, event);
    }
}

/// A watched channel went live or offline (`channel_state`'s two writers).
pub fn on_live_edge(edge: LiveEdge) {
    if let Some(engine) = ENGINE.get() {
        engine.on_live_edge(edge);
    }
}

/// The whisper session's own-topic state changed: reconcile reads it again.
pub fn own_topics_changed() {
    if let Some(engine) = ENGINE.get() {
        engine.request_refresh();
    }
}

/// Badges a Twitch gallery write added (kept 2 h, with the pending store).
pub fn note_arrivals(pairs: Vec<(String, String)>) {
    if let Some(engine) = ENGINE.get() {
        engine.note_arrivals(pairs);
    }
}

/// Whether the gallery gained this badge lately (false before init).
pub fn is_recent_arrival(set_id: &str, version: &str) -> bool {
    ENGINE.get().is_some_and(|e| e.is_recent_arrival(set_id, version))
}

/// The app came to the front or went to the background. The presence is
/// stored here, synchronously; the work runs spawned and reads the current
/// value, so reports finishing out of order still end right.
pub fn on_foreground(foreground: bool) {
    let Some(engine) = ENGINE.get() else { return };
    engine.presence.store(foreground, Ordering::SeqCst);
    let task = engine.clone();
    engine.spawn(async move { presence_changed(&task) });
}

/// The iPhone shell's picture in picture started or stopped.
pub fn on_pip(active: bool) {
    let Some(engine) = ENGINE.get() else { return };
    engine.pip.store(active, Ordering::SeqCst);
    presence_changed(engine);
}

fn presence_changed(engine: &Arc<Engine>) {
    if !engine.foreground() {
        engine.families.store(0, Ordering::Relaxed);
        engine.executor.bump();
        engine.flush_files();
    }
    engine.request_refresh();
}

/// IrcService cleared every claim. Ignored while reconcile's own chat start
/// runs (that start is what cleared them).
pub fn on_claims_reset() {
    // The fresh-start seed and every stop re-hold chats: no chat stays "refused" for a reminder.
    crate::services::reminder_service::gate_forget_all();
    if let Some(engine) = ENGINE.get() {
        on_claims_reset_on(engine);
    }
}

pub(crate) fn on_claims_reset_on(engine: &Engine) {
    if engine.starting.load(Ordering::SeqCst) {
        return;
    }
    lock(&engine.rec).claims_reset = true;
    engine.request_refresh();
}

/// Main is hiding to the tray: keep its rooms for the rules that listen in
/// any open chat. Returns once reconcile adopted them (or after 3 s).
pub async fn adopt_main_chats() {
    adopt_window_chats("main").await;
}

/// Window `label` is going away: keep its rooms (see `adopt_main_chats`).
pub async fn adopt_window_chats(label: &str) {
    let Some(engine) = ENGINE.get() else { return };
    adopt_window_chats_on(engine, label).await;
}

pub(crate) async fn adopt_window_chats_on(engine: &Arc<Engine>, label: &str) {
    if !reconcile::any_open(&engine.compiled()) {
        return;
    }
    let (tx, rx) = oneshot::channel();
    lock(&engine.rec).adopt.push((label.to_string(), tx));
    engine.request_refresh();
    let _ = tokio::time::timeout(engine.timings.adopt_wait, rx).await;
}

/// A window (any label but the engine's) claimed `channel`. Only a room the
/// engine adopted cares, and only about claims after the adoption.
pub fn note_window_claim(channel: &str) {
    // A chat you just opened is never gated for a reminder (works with no engine too).
    crate::services::reminder_service::gate_forget(&channel.trim_start_matches('#').to_lowercase());
    let Some(engine) = ENGINE.get() else { return };
    note_window_claim_on(engine, channel);
}

pub(crate) fn note_window_claim_on(engine: &Engine, channel: &str) {
    let key = channel.trim_start_matches('#').to_lowercase();
    // A window opened this chat: the Delay trigger counts from now (steady `engine.now()`
    // for the deadline, wall ms for the id `fired_for` keeps). Kept until the tick finds
    // no window holding the chat; never drained by reconcile.
    engine.chat_joins.note(&key, engine.now(), now_ms());
    let mut rec = lock(&engine.rec);
    if !rec.adopted.contains_key(&key) {
        return;
    }
    rec.window_claims.insert(key, now_ms());
    drop(rec);
    engine.request_refresh();
}

/// The last window let go of `channel`: its Delay join is gone (a window that opens it
/// again is a new join).
pub fn note_window_release(channel: &str) {
    if let Some(engine) = ENGINE.get() {
        note_window_release_on(engine, channel);
    }
}

pub(crate) fn note_window_release_on(engine: &Engine, channel: &str) {
    engine.chat_joins.forget(channel);
}

/// Before the account door removes `id`: stop its jobs and hold its rules
/// until the returned guard drops.
pub async fn before_account_removed(id: &str) -> LeavingGuard {
    before_accounts_removed_on(ENGINE.get(), &[id.to_string()]).await
}

/// Before a door that removes several accounts at once (one shared wait).
pub async fn before_accounts_removed(ids: &[String]) -> LeavingGuard {
    before_accounts_removed_on(ENGINE.get(), ids).await
}

/// The hook on a given engine: the engine's own seam takes the account-change
/// guard, and with no engine running the real one does.
pub async fn before_accounts_removed_on(engine: Option<&Arc<Engine>>, ids: &[String]) -> LeavingGuard {
    match engine {
        Some(engine) => before_accounts_removed_with(Some(engine), engine.local.as_ref(), ids).await,
        None => before_accounts_removed_with(None, &local::StoreChanges, ids).await,
    }
}

/// The hook over a given account-change seam (None: no engine, a guard that
/// holds only the account-change guard). The account-change guard comes first
/// in every case, so linked-account refreshes already running finish before
/// the door and no new one starts. The leaving guard exists before the wait,
/// so a dropped hook still lets go of its ids. During the wait the canceled
/// jobs' releases still verify and go out: a fresh token needs no refresh.
pub(crate) async fn before_accounts_removed_with(
    engine: Option<&Arc<Engine>>,
    changes: &dyn AccountChanges,
    ids: &[String],
) -> LeavingGuard {
    let change = changes.begin_account_change().await;
    let Some(engine) = engine else { return LeavingGuard::change_only(change) };
    let guard = engine.enter_leaving(ids, change);
    engine.executor.cancel_accounts(ids);
    let _ = tokio::time::timeout(engine.timings.remove_wait, engine.executor.idle_for(ids)).await;
    guard
}

/// Write every changed file now (exit and phone background).
pub fn flush() {
    if let Some(engine) = ENGINE.get() {
        engine.flush_files();
    }
}

/// Recompile soon (settings saved, recovery finished).
pub fn request_refresh() {
    if let Some(engine) = ENGINE.get() {
        engine.request_refresh();
    }
}

/// `AccountStore::save` succeeded, or the profile's Twitch identity changed
/// to another id: remembered token failures are forgotten and the rules
/// recompile.
pub fn on_accounts_changed() {
    if let Some(engine) = ENGINE.get() {
        engine.accounts_changed();
    }
}

/// A destroyed window stops hearing Activity.
pub fn forget_window(label: &str) {
    if let Some(engine) = ENGINE.get() {
        engine.forget_window(label);
    }
}

/// Chats the rules keep joined (for `/disconnect`'s answer).
pub fn chats_kept() -> Vec<String> {
    ENGINE.get().map(|e| lock(&e.kept).clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn on_chat_off_does_no_work() {
        let kit = Kit::new();
        kit.settings(RunState::Off, vec![mention_rule("r1", true)]).await;
        assert_eq!(kit.engine.families.load(Ordering::Relaxed), 0);
        let msg = chat_row("m1", "viewer", "hey @me");
        for _ in 0..24_000 {
            kit.engine.on_chat(&msg, false);
        }
        assert_eq!(kit.engine.work.load(Ordering::Relaxed), 0);
        assert_eq!(kit.engine.executor.submitted(), 0);
        assert!(kit.outward.sends().is_empty());
    }

    #[tokio::test]
    async fn free_entry_points_do_nothing_before_init() {
        // The process singleton is never set in tests.
        assert!(engine().is_none());
        on_chat(&chat_row("m1", "viewer", "hi"), false);
        on_badges(badge_seen("f1"));
        note_arrivals(vec![("set".into(), "1".into())]);
        assert!(!is_recent_arrival("set", "1"));
        on_foreground(true);
        on_claims_reset();
        note_window_claim("somechat");
        request_refresh();
        on_accounts_changed();
        flush();
    }

    #[tokio::test]
    async fn the_hook_with_no_engine_holds_the_account_change_until_dropped() {
        let kit = Kit::new();
        kit.local.park_account_change();
        let ids = [alt_id(), main_id()];
        let mut hook = Box::pin(before_accounts_removed_with(None, kit.local.as_ref(), &ids));
        assert!(futures::poll!(hook.as_mut()).is_pending(), "it waits for the refreshes in flight");
        kit.local.release_account_change();
        let guard = hook.await;
        assert!(guard.engine.is_none(), "no engine, no leaving counter");
        assert_eq!(kit.local.account_changes_held(), 1, "one guard for several ids, held");
        assert_eq!(kit.engine.leaving_count(&alt_id()), 0);
        drop(guard);
        assert_eq!(kit.local.account_changes_held(), 0);
    }

    #[tokio::test]
    async fn mobile_starts_in_the_background_and_desktop_in_front() {
        let (engine, _rx) = Engine::new(test_deps());
        assert_eq!(engine.foreground(), cfg!(desktop));
    }

    #[test]
    fn the_free_hooks_are_one_line_delegations() {
        let src = include_str!("mod.rs").replace("\r\n", "\n");
        let body = |name: &str| {
            let start = src.find(&format!("pub async fn {name}(")).expect(name);
            let end = start + src[start..].find("\n}\n").expect("end");
            src[start..end].to_string()
        };
        assert!(body("before_account_removed").contains("before_accounts_removed_on(ENGINE.get(), &[id.to_string()]).await"));
        assert!(body("before_accounts_removed").contains("before_accounts_removed_on(ENGINE.get(), ids).await"));
        assert_eq!(body("before_account_removed").lines().count(), 2);
        assert_eq!(body("before_accounts_removed").lines().count(), 2);
    }

    #[test]
    fn engine_modules_spawn_only_through_the_engine_spawner() {
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
            let allowed = usize::from(name == "mod.rs");
            assert_eq!(code.matches("tauri::async_runtime::spawn").count(), allowed, "{name}");
            assert_eq!(code.matches("tokio::spawn").count(), 0, "{name}");
        }
        let src = include_str!("mod.rs").replace("\r\n", "\n");
        let start = src.find("pub fn tauri_spawner(").expect("the app's spawner");
        let body = &src[start..start + src[start..].find("\n}\n").expect("end")];
        assert!(body.contains("tauri::async_runtime::spawn(task);"), "{body}");
    }

    #[test]
    fn with_no_engine_the_hook_takes_the_real_account_change() {
        let src = include_str!("mod.rs").replace("\r\n", "\n");
        let start = src.find("pub async fn before_accounts_removed_on(").expect("the hook");
        let hook = &src[start..start + src[start..].find("\n}\n").expect("end")];
        assert!(hook.contains("None => before_accounts_removed_with(None, &local::StoreChanges, ids).await"), "{hook}");
        let local = include_str!("local.rs").replace("\r\n", "\n");
        let at = local.find("impl AccountChanges for StoreChanges").expect("the real seam");
        let seam = &local[at..at + local[at..].find("\n}\n").expect("end")];
        let begin = seam.find("async fn begin_account_change(").expect("begin");
        let lines: Vec<&str> = seam[begin..].lines().skip(1).take_while(|l| l.trim() != "}").collect();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("AccountStore::begin_account_change().await"), "{lines:?}");
    }

    #[test]
    fn any_leaving_is_only_written_inside_the_leaving_lock() {
        let src = include_str!("mod.rs").replace("\r\n", "\n");
        let code = &src[..src.find("#[cfg(test)]\nmod tests").unwrap()];
        let mut from = 0;
        let mut stores = 0;
        while let Some(i) = code[from..].find("any_leaving.store(") {
            let at = from + i;
            let before = &code[..at];
            let lock_at = before.rfind("lock(&self.leaving)").max(before.rfind("lock(&engine.leaving)"));
            let lock_at = lock_at.expect("a store with no lock before it");
            assert!(!code[lock_at..at].contains("drop(map)"), "the store sits after the lock was dropped");
            stores += 1;
            from = at + 1;
        }
        assert_eq!(stores, 2);
    }

    #[tokio::test]
    async fn the_leaving_counter_refuses_until_every_guard_drops() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        assert!(!kit.engine.any_leaving());
        let first = before_accounts_removed_on(Some(&kit.engine), &[alt_id()]).await;
        let second = before_accounts_removed_on(Some(&kit.engine), &[alt_id()]).await;
        assert_eq!(kit.engine.leaving_count(&alt_id()), 2);
        kit.engine.on_chat(&chat_row("m1", "viewer", "hey @me"), false);
        assert_eq!(kit.engine.executor.submitted(), 0);
        drop(first);
        kit.engine.on_chat(&chat_row("m2", "viewer2", "hey @me"), false);
        assert_eq!(kit.engine.executor.submitted(), 0);
        drop(second);
        assert!(!kit.engine.any_leaving());
        kit.engine.on_chat(&chat_row("m3", "viewer3", "hey @me"), false);
        assert_eq!(kit.engine.executor.submitted(), 1);
    }

    #[tokio::test]
    async fn a_dropped_hook_lets_go_of_its_id() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        // A job sending as the alt whose release never finishes keeps the hook waiting.
        kit.outward.park_releases();
        kit.outward.lose_sends_with(SendResult::Refused("no".into()));
        kit.engine.on_chat(&chat_row("m1", "viewer", "hey @me"), false);
        kit.outward.wait_for_release_started().await;
        let ids = [alt_id()];
        let mut hook = Box::pin(before_accounts_removed_on(Some(&kit.engine), &ids));
        assert!(futures::poll!(hook.as_mut()).is_pending());
        assert_eq!(kit.engine.leaving_count(&alt_id()), 1);
        assert!(kit.engine.any_leaving());
        drop(hook);
        assert_eq!(kit.engine.leaving_count(&alt_id()), 0);
        assert!(!kit.engine.any_leaving());
    }

    #[tokio::test]
    async fn a_hook_returns_at_its_deadline_when_a_job_ignores_the_cancel() {
        let kit = Kit::new();
        kit.engine.executor.hold_account_for_test(&alt_id());
        let started = Instant::now();
        let guard = before_accounts_removed_on(Some(&kit.engine), &[alt_id()]).await;
        let took = started.elapsed();
        assert!(took >= kit.engine.timings.remove_wait, "{took:?}");
        assert!(took < kit.engine.timings.remove_wait + Duration::from_millis(500), "{took:?}");
        drop(guard);
        kit.engine.executor.release_account_for_test(&alt_id());
    }

    #[tokio::test]
    async fn forced_reauth_holds_every_id_on_one_guard() {
        let kit = Kit::new();
        let ids: Vec<String> = ["a", "b", "c"].iter().map(|s| format!("{}{s}", alt_id())).collect();
        for id in &ids {
            kit.engine.executor.hold_account_for_test(id);
        }
        let started = Instant::now();
        let guard = before_accounts_removed_on(Some(&kit.engine), &ids).await;
        assert!(started.elapsed() < kit.engine.timings.remove_wait * 2, "one shared deadline");
        for id in &ids {
            assert_eq!(kit.engine.leaving_count(id), 1);
        }
        drop(guard);
        for id in &ids {
            assert_eq!(kit.engine.leaving_count(id), 0);
            kit.engine.executor.release_account_for_test(id);
        }
    }

    #[tokio::test]
    async fn the_registry_the_profile_and_the_rules_make_my_ids() {
        let kit = Kit::new();
        kit.outward.set_own_accounts(vec![(alt_id(), "alt".into())]);
        kit.outward.set_profile(Some("7777"));
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        let compiled = kit.engine.compiled();
        assert!(compiled.is_mine(&alt_id()), "the registry");
        assert!(compiled.is_mine("7777"), "the profile");
        assert!(compiled.is_mine(&main_id()), "the rule's owner and sender");
        assert!(compiled.twitch_ids().contains(&alt_id()));
        kit.outward.set_own_accounts(vec![]);
        kit.engine.recompile().await;
        assert!(!kit.engine.compiled().is_mine(&alt_id()), "an empty registry empties its part");
    }

    #[tokio::test]
    async fn compile_never_waits_on_the_network_or_fetches_a_token() {
        let kit = Kit::new();
        kit.outward.validate_never_answers();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        let off = tokio::time::timeout(Duration::from_secs(1), kit.settings(RunState::Off, vec![mention_rule("r1", true)])).await;
        assert!(off.is_ok(), "the compile waited on id.twitch.tv");
        assert_eq!(kit.engine.families.load(Ordering::Relaxed), 0);
        assert_eq!(kit.outward.fetch_calls(&main_id()), 0);
        assert_eq!(kit.outward.validate_calls(&token_of(&main_id())), 0);
    }

    #[tokio::test]
    async fn the_hook_takes_the_account_change_first_and_holds_it_to_the_end() {
        let kit = Kit::new();
        kit.local.park_account_change();
        let ids = [alt_id()];
        let mut hook = Box::pin(before_accounts_removed_on(Some(&kit.engine), &ids));
        assert!(futures::poll!(hook.as_mut()).is_pending());
        assert_eq!(kit.engine.leaving_count(&alt_id()), 0, "nothing is counted before the account change is held");
        kit.local.release_account_change();
        let guard = hook.await;
        assert_eq!(kit.local.account_changes_held(), 1);
        assert_eq!(kit.engine.leaving_count(&alt_id()), 1);
        assert!(kit.engine.verified(&main_id(), Duration::from_secs(1)).await.is_ok(), "a fresh token still verifies");
        drop(guard);
        assert_eq!(kit.local.account_changes_held(), 0);
        assert_eq!(kit.engine.leaving_count(&alt_id()), 0);
    }

    #[tokio::test]
    async fn accounts_changed_forgets_failures_and_requests_a_refresh() {
        let kit = Kit::new();
        kit.outward.set_token(&alt_id(), FakeToken::NoToken);
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        kit.fire("m1").await;
        assert!(kit.outward.verifier().remembered_failure(&alt_id()).is_some());
        let before = kit.engine.refresh_requests.load(Ordering::SeqCst);
        kit.engine.accounts_changed();
        assert!(kit.outward.verifier().remembered_failure(&alt_id()).is_none());
        assert!(kit.engine.refresh_requests.load(Ordering::SeqCst) > before);
    }

    #[tokio::test]
    async fn a_dead_sender_is_fetched_once_a_minute_and_held_meanwhile() {
        let kit = Kit::new();
        kit.outward.set_token(&alt_id(), FakeToken::NoToken);
        kit.settings(RunState::On, vec![as_sender(mention_rule("r1", true), &alt_id(), "alt")]).await;
        kit.fire("m1").await;
        kit.fire_from("m2", "viewer2").await;
        kit.fire_from("m3", "viewer3").await;
        assert_eq!(kit.outward.fetch_calls(&alt_id()), 1);
        assert_eq!(kit.outward.claim_calls(), 0);
        assert!(kit.activity_texts().iter().any(|t| t == "Sign in as @alt to run this rule"));
        kit.clock.advance(Duration::from_secs(61));
        kit.fire_from("m4", "viewer4").await;
        assert_eq!(kit.outward.fetch_calls(&alt_id()), 2, "after a minute it is fetched again");
        kit.engine.accounts_changed();
        kit.fire_from("m5", "viewer5").await;
        assert_eq!(kit.outward.fetch_calls(&alt_id()), 3, "accounts_changed forgets the failure");
    }

    #[test]
    fn remove_hooks_unless_the_registry_shows_the_main() {
        let list = vec![("1".to_string(), true), ("2".to_string(), false)];
        assert!(!remove_needs_hook(&list, "1"), "the door refuses the main");
        assert!(remove_needs_hook(&list, "2"));
        assert!(remove_needs_hook(&list, "3"));
        assert!(remove_needs_hook(&[], "1"));
    }

    #[test]
    fn sign_out_ids_table() {
        let cases: [(Option<&str>, Option<&str>, &[&str], Vec<&str>); 8] = [
            (Some("1"), Some("2"), &[], vec!["1", "2"]),
            (Some("1"), Some("1"), &[], vec!["1"]),
            (Some("1"), None, &[], vec!["1"]),
            (None, Some("2"), &[], vec!["2"]),
            (Some(""), Some("2"), &[], vec!["2"]),
            (None, None, &[], vec![]),
            (Some("1"), Some("2"), &["2"], vec!["1"]),
            (None, Some("2"), &["2", "3"], vec![]),
        ];
        for (main, profile, linked, want) in cases {
            let linked: Vec<String> = linked.iter().map(|s| s.to_string()).collect();
            let got = sign_out_ids(main, profile, &linked);
            assert_eq!(got, want.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{main:?} {profile:?} {linked:?}");
        }
    }

    #[tokio::test]
    async fn own_rows_never_trigger() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        let mut row = chat_row("m1", "alt", "hey @me");
        row.user_id = alt_id();
        kit.engine.on_chat(&row, false);
        let mut mine = chat_row("m2", "me", "hey @me");
        mine.user_id = main_id();
        kit.engine.on_chat(&mine, false);
        assert_eq!(kit.engine.executor.submitted(), 0);
    }

    #[tokio::test]
    async fn old_dropped_and_backfilled_rows_are_ignored() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        let mut old = chat_row("m1", "viewer", "hey @me");
        old.received_at_ms = Some(now_ms() - 31_000);
        kit.engine.on_chat(&old, false);
        let mut backfill = chat_row("m2", "viewer", "hey @me");
        backfill.metadata.from_backfill = true;
        kit.engine.on_chat(&backfill, false);
        kit.engine.on_chat(&chat_row("m3", "viewer", "hey @me"), true);
        let mut unstamped = chat_row("m4", "viewer", "hey @me");
        unstamped.received_at_ms = None;
        kit.engine.on_chat(&unstamped, false);
        assert_eq!(kit.engine.executor.submitted(), 0);
    }

    #[tokio::test]
    async fn adoption_keeps_only_rooms_no_other_window_holds_and_releases_on_a_later_claim() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![any_open_rule("r1")]).await;
        kit.local.hold("main", "solo");
        kit.local.hold("main", "shared");
        kit.local.hold("multichat-1", "shared");
        let rec = kit.spawn_reconcile();
        adopt_window_chats_on(&kit.engine, "main").await;
        assert!(kit.local.joined().contains("solo"));
        assert!(!kit.local.joined().contains("shared"));
        assert!(lock(&kit.engine.rec).adopted.contains_key("solo"));
        // A claim of another room releases nothing.
        note_window_claim_on(&kit.engine, "elsewhere");
        kit.settle().await;
        assert!(lock(&kit.engine.rec).adopted.contains_key("solo"));
        // A claim of that room after the adoption releases it.
        tokio::time::sleep(Duration::from_millis(5)).await;
        note_window_claim_on(&kit.engine, "solo");
        kit.settle().await;
        assert!(!lock(&kit.engine.rec).adopted.contains_key("solo"));
        rec.abort();
    }

    #[tokio::test]
    async fn a_window_claim_before_the_adoption_releases_nothing() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![any_open_rule("r1")]).await;
        kit.local.hold("main", "solo");
        // Recorded before any adoption: ignored (the room is not adopted yet).
        note_window_claim_on(&kit.engine, "solo");
        let rec = kit.spawn_reconcile();
        adopt_window_chats_on(&kit.engine, "main").await;
        kit.settle().await;
        assert!(lock(&kit.engine.rec).adopted.contains_key("solo"));
        rec.abort();
    }

    #[tokio::test]
    async fn presence_reports_in_any_order_end_on_the_last_value() {
        let kit = Kit::new();
        kit.engine.presence.store(false, Ordering::SeqCst);
        kit.engine.presence.store(true, Ordering::SeqCst);
        presence_changed(&kit.engine);
        kit.engine.presence.store(false, Ordering::SeqCst);
        kit.engine.presence.store(true, Ordering::SeqCst);
        presence_changed(&kit.engine);
        presence_changed(&kit.engine);
        assert!(kit.engine.foreground());
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        assert_ne!(kit.engine.families.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn picture_in_picture_keeps_rules_running_in_the_background() {
        let kit = Kit::new();
        kit.engine.presence.store(false, Ordering::SeqCst);
        kit.engine.pip.store(true, Ordering::SeqCst);
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.engine.on_chat(&chat_row("m1", "viewer", "hey @me"), false);
        assert_eq!(kit.engine.executor.submitted(), 1);
    }

    #[tokio::test]
    async fn the_breaker_only_logs_in_test() {
        let kit = Kit::new();
        kit.prefs(|p| p.sends_per_minute = 1).await;
        kit.settings(RunState::Test, vec![mention_rule("r1", false)]).await;
        kit.trip_breaker_with_rows(|i| chat_row(&format!("t{i}"), &format!("v{i}"), "hey @me")).await;
        assert!(kit.activity_texts().iter().any(|t| t == guards::BREAKER_WOULD));
        assert_eq!(kit.engine.run_state(), RunState::Test);
    }

    #[tokio::test]
    async fn the_breaker_turns_live_off() {
        let kit = Kit::new();
        kit.prefs(|p| p.sends_per_minute = 1).await;
        kit.settings(RunState::On, vec![mention_rule("r1", true)]).await;
        kit.trip_breaker_with_rows(|i| chat_row(&format!("t{i}"), &format!("v{i}"), "hey @me")).await;
        assert_eq!(kit.engine.run_state(), RunState::Off);
        assert!(kit.activity_texts().iter().any(|t| t == guards::BREAKER_OFF));
        kit.wait_until(|| kit.local.turned_off()).await;
    }

    #[tokio::test]
    async fn arrivals_are_kept_with_the_pending_store() {
        let kit = Kit::new();
        kit.engine.note_arrivals(vec![("glitchcon".into(), "1".into())]);
        assert!(kit.engine.is_recent_arrival("glitchcon", "1"));
        assert!(!kit.engine.is_recent_arrival("glitchcon", "2"));
    }

    #[tokio::test]
    async fn a_follow_notifies_once_even_when_twitch_redelivers_it() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![own_rule("f", true, json!({ "kind": "follow" }), vec![notify_titled("New follower {user}")])]).await;
        let event = json!({ "user_id": "7", "user_login": "fan", "user_name": "Fan", "followed_at": "2026-11-13T05:00:00Z" });
        kit.engine.on_own_topic(TopicKind::Follow, &event);
        kit.engine.on_own_topic(TopicKind::Follow, &event);
        kit.idle().await;
        let titles: Vec<String> = kit.local.notifications().into_iter().map(|n| n.title).collect();
        assert_eq!(titles, vec!["New follower Fan".to_string()]);
    }

    #[tokio::test]
    async fn an_own_event_fires_nothing_while_run_is_off_or_the_app_is_in_the_background() {
        let kit = Kit::new();
        let event = json!({ "id": "p1", "title": "Pizza?" });
        kit.settings(RunState::Off, vec![own_rule("p", true, json!({ "kind": "poll_started" }), vec![notify_titled("{poll}")])]).await;
        kit.engine.on_own_topic(TopicKind::PollBegin, &event);
        kit.idle().await;
        assert!(kit.local.notifications().is_empty(), "Off");
        kit.settings(RunState::On, vec![own_rule("p", true, json!({ "kind": "poll_started" }), vec![notify_titled("{poll}")])]).await;
        kit.engine.presence.store(false, Ordering::SeqCst);
        kit.engine.recompile().await;
        kit.engine.on_own_topic(TopicKind::PollBegin, &json!({ "id": "p2", "title": "Again?" }));
        kit.idle().await;
        assert!(kit.local.notifications().is_empty(), "the background runs no rules");
        kit.engine.presence.store(true, Ordering::SeqCst);
        kit.engine.recompile().await;
        kit.engine.on_own_topic(TopicKind::PollBegin, &json!({ "id": "p3", "title": "Back?" }));
        kit.idle().await;
        assert_eq!(kit.local.notifications()[0].title, "Back?");
    }

    #[tokio::test]
    async fn a_live_edge_notifies_once_per_broadcast_and_not_when_stale() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![live_change_rule("l", true, "friend", "555", vec![notify_titled("{channel} is live")])]).await;
        let edge = |at: chrono::DateTime<chrono::Utc>| LiveEdge {
            login: "friend".into(),
            channel_id: "555".into(),
            online: true,
            started_at: at.to_rfc3339(),
        };
        let started = chrono::Utc::now() - chrono::Duration::minutes(2);
        kit.engine.on_live_edge(edge(started));
        // The poll that follows `note_live`: the same broadcast, spelled another way (epoch seconds make one key).
        kit.engine.on_live_edge(LiveEdge { started_at: started.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(), ..edge(started) });
        kit.idle().await;
        assert_eq!(kit.local.notifications().len(), 1);
        assert_eq!(kit.local.notifications()[0].title, "friend is live");
        // Eleven minutes old by Twitch's clock: nothing.
        kit.engine.on_live_edge(edge(chrono::Utc::now() - chrono::Duration::minutes(11)));
        kit.idle().await;
        assert_eq!(kit.local.notifications().len(), 1);
        // Freshness reads Twitch's clock: 5 minutes ahead makes a 6 minute old start 11 old.
        kit.local.set_twitch_offset_ms(5 * 60_000);
        kit.engine.on_live_edge(edge(chrono::Utc::now() - chrono::Duration::minutes(6)));
        kit.idle().await;
        assert_eq!(kit.local.notifications().len(), 1, "freshness is by Twitch time");
        kit.local.set_twitch_offset_ms(0);
        // A channel the rules do not name is never answered.
        kit.engine.on_live_edge(LiveEdge { login: "stranger".into(), ..edge(started) });
        kit.idle().await;
        assert_eq!(kit.local.notifications().len(), 1);
    }

    #[tokio::test]
    async fn an_offline_edge_fires_the_offline_rule_once_per_ended_broadcast() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![live_change_rule("off", false, "friend", "555", vec![notify_titled("{channel} left")])]).await;
        let ended = chrono::Utc::now() - chrono::Duration::hours(3);
        let edge = LiveEdge { login: "friend".into(), channel_id: "555".into(), online: false, started_at: ended.to_rfc3339() };
        kit.engine.on_live_edge(edge.clone());
        kit.engine.on_live_edge(edge);
        kit.idle().await;
        assert_eq!(kit.local.notifications().len(), 1, "an offline edge has no 10 minute rule; it is once per broadcast");
        assert_eq!(kit.local.notifications()[0].title, "friend left");
    }

    #[tokio::test]
    async fn the_status_names_why_an_own_channel_rule_is_not_listening() {
        let kit = Kit::new();
        kit.settings(RunState::On, vec![own_rule("f", true, json!({ "kind": "follow" }), vec![notify_titled("x")])]).await;
        let mut holds = HashMap::new();
        holds.insert("f".to_string(), "Waiting for a Twitch connection".to_string());
        kit.engine.set_own_held(holds);
        assert_eq!(kit.engine.status().rules[0].held.as_deref(), Some("Waiting for a Twitch connection"));
        kit.engine.set_own_held(HashMap::new());
        assert_eq!(kit.engine.status().rules[0].held, None);
    }
}
