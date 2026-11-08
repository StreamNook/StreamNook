//! Bringing whisper history in from Twitch and keeping it in step.
//!
//! Live whispers arrive on EventSub and sends go through Helix; this module
//! covers what Twitch offers no official endpoint for: the history (the first
//! import and every later sync), replies sent from another device, the read
//! state Twitch keeps, and archiving. It owns:
//!
//! - which account is followed, and a generation that moves only when that
//!   account changes, so a run started for one account never writes for
//!   another;
//! - one sync at a time per account (a trigger during a run asks for one more
//!   run when it ends);
//! - the pace of requests (two at a time, a quarter second apart, half a
//!   minute of quiet after Twitch says "slow down");
//! - when an integrity token may be minted (once per run, plus once after a
//!   refusal; after a failed mint, not again for 2 minutes, then 10, then an
//!   hour);
//! - where each conversation's older history and any gap left by a short sync
//!   resume from, and under which device id the cursor was made.
//!
//! Nothing here runs on a timer. Syncs start from sign-in, a return after ten
//! minutes away, a socket gap, the inbox being opened (at most once a minute),
//! a conversation being opened, a whisper received or sent, and the refresh
//! button.

use crate::services::twitch_integrity::Integrity;
use crate::services::whisper_gql::{self, GqlError, GqlMessage, GqlParticipant, GqlThread, Gql, ThreadsPage};
use crate::services::whisper_store::{
    self, Closing, ConversationMeta, Cursor, Deep, Marker, Origin, Scope, StoredWhisper,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Conversations per list page, and messages per history page (Twitch's cap).
const PAGE: u32 = 100;
/// Each conversation's newest messages nested in a list page after the first import.
const NESTED_INCREMENTAL: u32 = 20;
/// Messages a one-conversation refresh reads.
const REFRESH_FIRST: u32 = 20;
const REQUESTS_AT_ONCE: usize = 2;
const REQUEST_SPACING_MS: i64 = 250;
const RATE_LIMIT_QUIET_MS: i64 = 30_000;
const RATE_LIMIT_TRIES: u32 = 4;
const INBOX_OPEN_EVERY_MS: i64 = 60_000;
const AWAY_FOR_SYNC_MS: i64 = 10 * 60_000;
const TRAFFIC_DEBOUNCE_MS: u64 = 1_500;
const MINT_BACKOFF_MS: [i64; 3] = [2 * 60_000, 10 * 60_000, 60 * 60_000];
const SYNC_NOW_WAIT: Duration = Duration::from_secs(60);
const LOAD_OLDER_WAIT: Duration = Duration::from_secs(45);
/// What the inbox shows when the twitch.tv session belongs to someone else.
pub const SESSION_PROBLEM: &str = "Sign in to twitch.tv again to load your whispers";

pub type BoxFut<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
pub type Spawn = Arc<dyn Fn(BoxFut<()>) + Send + Sync>;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum SyncOutcome {
    Done { added: u32 },
    Superseded,
    NoSession,
    WrongSession,
    Partial { added: u32 },
    Failed { reason: String },
    StillRunning,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    Follow,
    Resume,
    ResumePartial,
    SocketGap,
    InboxOpened,
    ConversationOpened(String),
    AfterTraffic(String),
    Manual,
}

impl Reason {
    /// A run that reads the conversation list (every reason but the
    /// partial-only resume and the one-conversation ones).
    fn reads_list(&self) -> bool {
        !matches!(self, Reason::ResumePartial)
    }
}

// ---- Seams ---------------------------------------------------------------------------

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}
pub trait Sleeper: Send + Sync {
    fn sleep(&self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send>>;
}
pub trait Minter: Send + Sync {
    fn mint(&self) -> Pin<Box<dyn Future<Output = Result<Integrity, String>> + Send>>;
    fn cached(&self) -> Option<Integrity>;
    fn invalidate(&self);
    fn last_success_ms(&self) -> Option<i64>;
}
/// The twitch.tv web session the app already holds.
pub trait WebSession: Send + Sync {
    fn token(&self) -> BoxFut<Option<String>>;
    /// Forget the session and read it again from the sign-in.
    fn reharvest(&self) -> BoxFut<()>;
    fn gql(&self, token: &str) -> Arc<dyn Gql>;
}
/// What a run tells the windows.
pub trait Events: Send + Sync {
    fn archive_changed(&self, owner: &str, syncing: bool);
    fn conversation(&self, scope: &Scope, key: &str);
    fn counts(&self);
    /// A new account generation: only claims made under it still count.
    fn claims_scope(&self, generation: u64);
    /// Look again for sends still waiting on Twitch's copy.
    fn recheck_sends(&self);
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
}
pub struct TokioSleeper;
impl Sleeper for TokioSleeper {
    fn sleep(&self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(tokio::time::sleep(Duration::from_millis(ms)))
    }
}
pub struct IntegrityMinter;
impl Minter for IntegrityMinter {
    fn mint(&self) -> Pin<Box<dyn Future<Output = Result<Integrity, String>> + Send>> {
        Box::pin(crate::services::twitch_integrity::get())
    }
    fn cached(&self) -> Option<Integrity> {
        crate::services::twitch_integrity::cached_token()
    }
    fn invalidate(&self) {
        crate::services::twitch_integrity::invalidate();
    }
    fn last_success_ms(&self) -> Option<i64> {
        crate::services::twitch_integrity::last_success_ms()
    }
}
struct AppWebSession;
impl WebSession for AppWebSession {
    fn token(&self) -> BoxFut<Option<String>> {
        Box::pin(crate::services::twitch_web_gql::app_web_session())
    }
    fn reharvest(&self) -> BoxFut<()> {
        Box::pin(async {
            use tauri::Manager;
            let Some(app) = crate::services::providers::app_handle() else { return };
            let Some(state) = app.try_state::<crate::models::settings::AppState>() else { return };
            let auth = state.twitch_auth.clone();
            auth.invalidate().await;
        })
    }
    fn gql(&self, token: &str) -> Arc<dyn Gql> {
        Arc::new(whisper_gql::WebGql { token: token.to_string() })
    }
}
struct AppEvents;
impl Events for AppEvents {
    fn archive_changed(&self, owner: &str, syncing: bool) {
        use tauri::Emitter;
        if let Some(app) = crate::services::providers::app_handle() {
            let _ = app.emit(
                crate::services::whisper_inbox::ARCHIVE_CHANGED_EVENT,
                crate::services::whisper_inbox::ArchiveChanged { owner_id: owner.to_string(), syncing },
            );
        }
    }
    fn conversation(&self, scope: &Scope, key: &str) {
        crate::services::whisper_inbox::emit_conversation(scope, key);
    }
    fn counts(&self) {
        crate::services::inbox_counts::wake();
    }
    fn claims_scope(&self, generation: u64) {
        crate::services::whisper_claims::clear_scope(generation);
    }
    fn recheck_sends(&self) {
        crate::services::whisper_send::recheck_pending();
    }
}

/// One conversation's refresh: whether a read is running, and the newest read
/// that succeeded (by the order reads started in).
struct RefreshSlot {
    running: bool,
    last_ok: u64,
    ended: tokio::sync::watch::Sender<u64>,
}

pub struct Env {
    pub session: Arc<dyn WebSession>,
    pub clock: Arc<dyn Clock>,
    pub sleeper: Arc<dyn Sleeper>,
    pub minter: Arc<dyn Minter>,
    pub events: Arc<dyn Events>,
    pub spawn: Spawn,
}

// ---- Engine state ----------------------------------------------------------------------

#[derive(Default)]
struct Control {
    followed: Option<String>,
    generation: u64,
    scope: Option<Scope>,
    running: bool,
    run_again: Option<Reason>,
    completed: u64,
    last: Option<SyncOutcome>,
    last_inbox_ms: Option<i64>,
    away_since: Option<i64>,
    socket_down_since: Option<i64>,
}

#[derive(Default)]
struct MintGate {
    failures: u32,
    next_allowed_ms: i64,
    last_failure_ms: i64,
}

impl MintGate {
    fn allowed(&mut self, now: i64, last_success: Option<i64>) -> bool {
        if last_success.is_some_and(|s| s > self.last_failure_ms) {
            *self = MintGate::default();
        }
        now >= self.next_allowed_ms
    }
    fn failed(&mut self, now: i64) {
        let step = MINT_BACKOFF_MS[(self.failures as usize).min(MINT_BACKOFF_MS.len() - 1)];
        self.failures += 1;
        self.last_failure_ms = now;
        self.next_allowed_ms = now + step;
    }
    fn succeeded(&mut self) {
        *self = MintGate::default();
    }
}

/// Requests in flight and their spacing, shared by every sync, refresh and
/// Load older of the process.
struct Pacer {
    slots: tokio::sync::Semaphore,
    next_start: Mutex<i64>,
    quiet_until: Mutex<i64>,
}

/// One run's integrity budget and what it found.
struct Run {
    integ: Option<Integrity>,
    mints_left: u8,
    reminted: bool,
    bypass_gate: bool,
    added: u32,
    partial: bool,
}

impl Run {
    fn new(bypass_gate: bool) -> Self {
        Run { integ: None, mints_left: 1, reminted: false, bypass_gate, added: 0, partial: false }
    }
}

pub struct Engine {
    env: Env,
    ctrl: Mutex<Control>,
    done: tokio::sync::watch::Sender<u64>,
    pacer: Pacer,
    gate: Mutex<MintGate>,
    who: Mutex<HashMap<String, String>>,
    loading: Mutex<HashSet<String>>,
    refreshing: Mutex<HashMap<String, RefreshSlot>>,
    /// Orders refresh calls and reads: a call counts only a read started after it.
    refresh_order: AtomicU64,
    debounced: Mutex<HashSet<String>>,
    owed: Mutex<HashMap<String, bool>>,
    restarted: Mutex<HashSet<String>>,
    problem: Mutex<Option<String>>,
    offset_ms: AtomicI64,
    /// Accounts whose sends left Sending by an earlier run were settled.
    settled: Mutex<HashSet<String>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What one conversation's merge found.
#[derive(Default, Debug)]
struct Merged {
    added: u32,
    changed: bool,
    hit_known: bool,
    has_next: bool,
    next_cursor: Option<String>,
    owe: bool,
    /// A gap page reached the gap's floor (or the end of history).
    gap_closed: bool,
}

/// The newest Twitch message held, by (time, id): where a gap opened now ends.
fn newest_twitch(messages: &[StoredWhisper]) -> Option<Marker> {
    messages
        .iter()
        .filter(|m| m.origin == Origin::Twitch)
        .max_by(|a, b| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id)))
        .map(|m| Marker { id: m.id.clone(), at_ms: m.timestamp })
}

/// A gap page reaches the floor when it holds the floor message or anything
/// older, by (time, id).
fn reaches_floor(page: &[GqlMessage], floor: &Marker) -> bool {
    page.iter().any(|gm| (gm.sent_at_ms, gm.id.as_str()) <= (floor.at_ms, floor.id.as_str()))
}

/// How a page of messages is being read.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// The nested newest messages of a list page; `first_import` asks for the
    /// whole history.
    Nested { first_import: bool },
    /// A page of older history (deep paging or Load older).
    Deep,
    /// A page read to close a gap left by a short sync.
    Gap,
    /// The one-conversation refresh.
    Refresh,
}

struct Who<'a> {
    id: &'a str,
    login: &'a str,
    name: &'a str,
}

fn local_day(ms: i64) -> Option<chrono::NaiveDate> {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive())
}

fn stored_of(gm: &GqlMessage, viewer: &str, me: &Who, other: &Who) -> StoredWhisper {
    let mine = gm.from_id == viewer;
    let (from, to) = if mine { (me, other) } else { (other, me) };
    StoredWhisper {
        id: gm.id.clone(),
        from_user_id: from.id.to_string(),
        from_user_login: from.login.to_string(),
        from_user_name: from.name.to_string(),
        to_user_id: to.id.to_string(),
        to_user_login: to.login.to_string(),
        to_user_name: to.name.to_string(),
        message: gm.text.clone(),
        timestamp: gm.sent_at_ms,
        is_sent: mine,
        parts: gm.parts.clone(),
        deleted: gm.deleted,
        edited_ms: gm.edited_ms,
        origin: Origin::Twitch,
        delivery: None,
        sent_local_ms: None,
        got_204: false,
        checks: 0,
        failure: None,
        cut_to: None,
        migrated: false,
    }
}

/// Twitch's copy replaces ours for a message we already hold by id: text,
/// emote parts, edits, deletion and time are Twitch's. Returns whether
/// anything changed.
fn take_twitch(local: &mut StoredWhisper, tw: &StoredWhisper) -> bool {
    let before = (local.message.clone(), local.parts.clone(), local.deleted, local.edited_ms, local.timestamp, local.origin);
    local.message = tw.message.clone();
    local.parts = tw.parts.clone();
    local.deleted = tw.deleted;
    local.edited_ms = tw.edited_ms;
    local.timestamp = tw.timestamp;
    local.origin = Origin::Twitch;
    for (mine, theirs) in [
        (&mut local.from_user_login, &tw.from_user_login),
        (&mut local.from_user_name, &tw.from_user_name),
        (&mut local.to_user_login, &tw.to_user_login),
        (&mut local.to_user_name, &tw.to_user_name),
    ] {
        if mine.is_empty() && !theirs.is_empty() {
            *mine = theirs.clone();
        }
    }
    let mut changed = before != (local.message.clone(), local.parts.clone(), local.deleted, local.edited_ms, local.timestamp, local.origin);
    if local.is_sent && local.delivery.is_some() && local.delivery != Some(whisper_store::Delivery::Delivered) {
        local.delivery = Some(whisper_store::Delivery::Delivered);
        changed = true;
    }
    changed
}

/// Twitch's meta for the conversation, field by field. Names, picture,
/// color, badges, thread id and mute are Twitch's. The archive flag and the
/// read marker are Twitch's unless a local write to them landed after this
/// page's request started (`page_seq`), or an archive is still owed.
fn merge_twitch_meta(meta: &mut ConversationMeta, thread: &GqlThread, page_seq: u64) -> bool {
    let before = meta.clone();
    if let Some(o) = &thread.other {
        if !o.login.is_empty() {
            meta.user_login = o.login.clone();
        }
        if !o.display_name.is_empty() {
            meta.user_name = o.display_name.clone();
        }
        if o.avatar.is_some() {
            meta.profile_image_url = o.avatar.clone();
        }
        meta.color = o.color.clone();
        meta.badges = o.badges.clone();
    }
    meta.thread_id = Some(thread.thread_id.clone());
    meta.muted = thread.muted;
    if meta.read_seq <= page_seq {
        meta.twitch_read = thread.twitch_read.clone();
    }
    if !meta.archive_pending && meta.archived_seq <= page_seq {
        meta.archived = thread.archived;
    }
    *meta != before
}

/// Drop the old importer's copies that Twitch's history now covers: every
/// scraped message dated on or after the local day of the oldest Twitch
/// message held. Older ones stay (history Twitch no longer returns). A dropped
/// read marker moves to the newest Twitch message strictly before its day.
/// Only called once the conversation's history is Complete.
fn drop_scraped(meta: &mut ConversationMeta, messages: &mut Vec<StoredWhisper>) -> bool {
    let Some(oldest) = messages.iter().filter(|m| m.origin == Origin::Twitch).map(|m| m.timestamp).min() else {
        return false;
    };
    let Some(day0) = local_day(oldest) else { return false };
    let marker_day = meta
        .read_through
        .as_ref()
        .and_then(|r| messages.iter().find(|m| m.id == r.id && m.origin == Origin::Scraped))
        .and_then(|m| local_day(m.timestamp));
    let before = messages.len();
    messages.retain(|m| m.origin != Origin::Scraped || local_day(m.timestamp).is_some_and(|d| d < day0));
    if let Some(day) = marker_day.filter(|d| *d >= day0) {
        meta.read_through = messages
            .iter()
            .filter(|m| m.origin == Origin::Twitch && local_day(m.timestamp).is_some_and(|d| d < day))
            .max_by_key(|m| (m.timestamp, m.id.clone()))
            .map(|m| Marker { id: m.id.clone(), at_ms: m.timestamp });
    }
    messages.len() != before
}

/// The other person's id for a conversation, from its participants or else
/// from the thread id.
fn other_id(thread: &GqlThread, viewer: &str) -> Option<String> {
    if let Some(o) = &thread.other {
        return Some(o.id.clone());
    }
    thread.thread_id.split('_').find(|p| *p != viewer && !p.is_empty()).map(str::to_string)
}

fn fresh_meta(key: &str, thread: &GqlThread) -> ConversationMeta {
    let o: Option<&GqlParticipant> = thread.other.as_ref();
    // Built on the store's own empty meta, so no field is missed.
    ConversationMeta {
        user_login: o.map(|p| p.login.clone()).unwrap_or_default(),
        user_name: o.map(|p| p.display_name.clone()).unwrap_or_default(),
        profile_image_url: o.and_then(|p| p.avatar.clone()),
        color: o.and_then(|p| p.color.clone()),
        badges: o.map(|p| p.badges.clone()).unwrap_or_default(),
        thread_id: Some(thread.thread_id.clone()),
        twitch_read: thread.twitch_read.clone(),
        archived: thread.archived,
        muted: thread.muted,
        ..ConversationMeta::new(key)
    }
}

fn is_closed(e: &str) -> bool {
    e == "closed"
}

impl Engine {
    pub fn new(env: Env) -> Self {
        let (done, _) = tokio::sync::watch::channel(0u64);
        Engine {
            env,
            ctrl: Mutex::new(Control::default()),
            done,
            pacer: Pacer {
                slots: tokio::sync::Semaphore::new(REQUESTS_AT_ONCE),
                next_start: Mutex::new(0),
                quiet_until: Mutex::new(0),
            },
            gate: Mutex::new(MintGate::default()),
            who: Mutex::new(HashMap::new()),
            loading: Mutex::new(HashSet::new()),
            refreshing: Mutex::new(HashMap::new()),
            refresh_order: AtomicU64::new(0),
            debounced: Mutex::new(HashSet::new()),
            owed: Mutex::new(HashMap::new()),
            restarted: Mutex::new(HashSet::new()),
            problem: Mutex::new(None),
            offset_ms: AtomicI64::new(0),
            settled: Mutex::new(HashSet::new()),
        }
    }

    /// Once per account per process, before anything merges Twitch's copies:
    /// sends an earlier run of the app left Sending become Unknown, so the
    /// first read of their thread adopts them instead of adding a duplicate.
    fn settle_once(&self, scope: &Scope) {
        if lock(&self.settled).insert(scope.owner.clone()) {
            let n = crate::services::whisper_send::settle_stale_sends(scope);
            if n > 0 {
                log::info!("[WhisperSync] {n} whispers left Sending by the last run wait on Twitch's thread");
            }
        }
    }

    fn now(&self) -> i64 {
        self.env.clock.now_ms()
    }

    // ---- Account follow ----------------------------------------------------------

    /// Follow the signed-in Twitch account. Moves the generation only when the
    /// account changes (every roster build calls this). The old account's
    /// scope is closed at once, before anything is spawned; its teardown is
    /// returned for the caller to run.
    fn follow(self: &Arc<Self>, user_id: Option<String>) -> Option<Closing> {
        let (old, start, generation) = {
            let mut c = lock(&self.ctrl);
            if c.followed == user_id {
                return None;
            }
            c.generation += 1;
            let old = c.scope.take();
            c.followed = user_id.clone();
            c.run_again = None;
            c.last_inbox_ms = None;
            c.away_since = None;
            c.socket_down_since = None;
            if let Some(id) = &user_id {
                c.scope = Some(whisper_store::open(id, c.generation));
            }
            (old, user_id.is_some(), c.generation)
        };
        lock(&self.who).clear();
        lock(&self.gate).succeeded();
        lock(&self.restarted).clear();
        lock(&self.debounced).clear();
        *lock(&self.problem) = None;
        // Claims made under any earlier generation stop counting, on a switch
        // and on sign-out alike; a window shows its conversation again by
        // claiming it under the new one.
        self.env.events.claims_scope(generation);
        let closing = old.map(|s| whisper_store::close(&s));
        if start {
            self.trigger(Reason::Follow);
            // A send left waiting by an earlier run of the app gets its real
            // check against Twitch (each read settles stale sends first).
            self.env.events.recheck_sends();
        }
        closing
    }

    fn scope(&self) -> Option<Scope> {
        lock(&self.ctrl).scope.clone()
    }

    // ---- Triggers ------------------------------------------------------------------

    /// Start a run, or ask the running one for another. Returns how many runs
    /// must complete before this trigger's run has finished (0 when skipped).
    fn trigger(self: &Arc<Self>, reason: Reason) -> u64 {
        match &reason {
            Reason::ConversationOpened(key) => {
                self.spawn_refresh(key.clone(), 0);
                return 0;
            }
            Reason::AfterTraffic(key) => {
                if lock(&self.debounced).insert(key.clone()) {
                    self.spawn_refresh(key.clone(), TRAFFIC_DEBOUNCE_MS);
                }
                return 0;
            }
            _ => {}
        }
        let now = self.now();
        let mut c = lock(&self.ctrl);
        if c.scope.is_none() {
            return 0;
        }
        if reason == Reason::InboxOpened {
            if c.last_inbox_ms.is_some_and(|t| now - t < INBOX_OPEN_EVERY_MS) {
                return 0;
            }
            c.last_inbox_ms = Some(now);
        }
        if c.running {
            let replace = match &c.run_again {
                None => true,
                Some(r) => !r.reads_list() && reason.reads_list(),
            };
            if replace {
                c.run_again = Some(reason);
            }
            return c.completed + 2;
        }
        c.running = true;
        let target = c.completed + 1;
        drop(c);
        let me = Arc::clone(self);
        (self.env.spawn)(Box::pin(async move { me.run_loop(reason).await }));
        target
    }

    async fn run_loop(self: Arc<Self>, first: Reason) {
        let mut reason = first;
        loop {
            let outcome = match self.scope() {
                Some(scope) => self.run_once(&scope, &reason).await,
                None => SyncOutcome::NoSession,
            };
            let again = {
                let mut c = lock(&self.ctrl);
                c.completed += 1;
                c.last = Some(outcome.clone());
                match c.run_again.take() {
                    Some(next) => Some(next),
                    None => {
                        c.running = false;
                        None
                    }
                }
            };
            self.done.send_replace(lock(&self.ctrl).completed);
            match again {
                Some(next) => reason = next,
                None => break,
            }
        }
    }

    async fn sync_now(self: &Arc<Self>) -> SyncOutcome {
        let mut rx = self.done.subscribe();
        let target = self.trigger(Reason::Manual);
        if target == 0 {
            return SyncOutcome::NoSession;
        }
        let waited = tokio::time::timeout(SYNC_NOW_WAIT, async {
            loop {
                if *rx.borrow_and_update() >= target {
                    return;
                }
                if rx.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
        match waited {
            Ok(()) => lock(&self.ctrl).last.clone().unwrap_or(SyncOutcome::StillRunning),
            Err(_) => SyncOutcome::StillRunning,
        }
    }

    fn note_away(self: &Arc<Self>, away: bool) {
        let now = self.now();
        let (long_away, gap) = {
            let mut c = lock(&self.ctrl);
            if away {
                c.away_since.get_or_insert(now);
                return;
            }
            let long = c.away_since.take().is_some_and(|t| now - t >= AWAY_FOR_SYNC_MS);
            (long, c.socket_down_since.is_some())
        };
        // Back on screen: sends still waiting get their real check now, since
        // a phone's timers stop in the background.
        self.env.events.recheck_sends();
        if long_away {
            self.trigger(Reason::Resume);
        } else if gap {
            self.trigger(Reason::SocketGap);
        } else if self.has_partial() {
            self.trigger(Reason::ResumePartial);
        }
    }

    fn note_socket_up(self: &Arc<Self>, first_of_process: bool) {
        let gap = lock(&self.ctrl).socket_down_since.take();
        if !first_of_process && gap.is_some() {
            self.trigger(Reason::SocketGap);
        }
    }

    fn note_socket_down(&self) {
        let now = self.now();
        lock(&self.ctrl).socket_down_since.get_or_insert(now);
    }

    fn has_partial(&self) -> bool {
        let Some(scope) = self.scope() else { return false };
        whisper_store::read_index(&scope, |idx| {
            !matches!(idx.list_deep, Deep::Complete | Deep::Unknown)
                || idx.conversations.iter().any(|m| matches!(m.deep, Deep::Partial { .. }) || m.gap.is_some())
        })
        .unwrap_or(false)
    }

    // ---- Requests ------------------------------------------------------------------

    /// Run one request at the shared pace. A rate-limit answer waits half a
    /// minute counted from that refusal and tries again, a few times at most.
    async fn paced<T, F, Fut>(&self, mut f: F) -> Result<T, GqlError>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, GqlError>>,
    {
        let _slot = self.pacer.slots.acquire().await.map_err(|_| GqlError::Other("closing".into()))?;
        for _ in 0..RATE_LIMIT_TRIES {
            loop {
                let now = self.now();
                let start_at = (*lock(&self.pacer.next_start)).max(*lock(&self.pacer.quiet_until));
                if start_at <= now {
                    *lock(&self.pacer.next_start) = now + REQUEST_SPACING_MS;
                    break;
                }
                self.env.sleeper.sleep((start_at - now) as u64).await;
            }
            match f().await {
                Err(GqlError::RateLimited) => {
                    *lock(&self.pacer.quiet_until) = self.now() + RATE_LIMIT_QUIET_MS;
                }
                other => return other,
            }
        }
        Err(GqlError::RateLimited)
    }

    /// A token for a cursor page within this run's budget, or None.
    async fn integrity(&self, run: &mut Run) -> Option<Integrity> {
        if let Some(i) = &run.integ {
            return Some(i.clone());
        }
        if let Some(i) = self.env.minter.cached() {
            run.integ = Some(i.clone());
            return Some(i);
        }
        if run.mints_left == 0 {
            return None;
        }
        let now = self.now();
        if !run.bypass_gate && !lock(&self.gate).allowed(now, self.env.minter.last_success_ms()) {
            return None;
        }
        run.mints_left -= 1;
        match self.env.minter.mint().await {
            Ok(i) => {
                lock(&self.gate).succeeded();
                run.integ = Some(i.clone());
                Some(i)
            }
            Err(e) => {
                log::info!("[WhisperSync] no integrity token this run: {e}");
                lock(&self.gate).failed(self.now());
                None
            }
        }
    }

    /// Twitch refused a token mid-run: forget it and allow one fresh mint.
    fn refused_token(&self, run: &mut Run) -> bool {
        run.integ = None;
        self.env.minter.invalidate();
        if run.reminted {
            return false;
        }
        run.reminted = true;
        run.mints_left = 1;
        true
    }

    /// Who a token belongs to, asked once per token for the process.
    async fn viewer(&self, token: &str, gql: &dyn Gql) -> Result<String, GqlError> {
        if let Some(id) = lock(&self.who).get(token).cloned() {
            return Ok(id);
        }
        let id = self.paced(|| whisper_gql::who(gql)).await?;
        lock(&self.who).insert(token.to_string(), id.clone());
        Ok(id)
    }

    /// The web session for `scope`'s account, or the outcome that ends the run.
    /// A session that belongs to someone else is read again once from the
    /// sign-in; if it still does, nothing is read or written.
    async fn session(&self, scope: &Scope) -> Result<Arc<dyn Gql>, SyncOutcome> {
        for attempt in 0..2 {
            let Some(token) = self.env.session.token().await else {
                return Err(SyncOutcome::NoSession);
            };
            let gql = self.env.session.gql(&token);
            match self.viewer(&token, &*gql).await {
                Ok(id) if id == scope.owner => {
                    *lock(&self.problem) = None;
                    return Ok(gql);
                }
                Ok(_) | Err(GqlError::Unauthorized) if attempt == 0 => {
                    self.env.session.reharvest().await;
                }
                Ok(_) => {
                    log::warn!("[WhisperSync] the twitch.tv session belongs to another account; whispers not synced");
                    *lock(&self.problem) = Some(SESSION_PROBLEM.to_string());
                    return Err(SyncOutcome::WrongSession);
                }
                Err(GqlError::Unauthorized) => return Err(SyncOutcome::NoSession),
                Err(e) => return Err(SyncOutcome::Failed { reason: e.words() }),
            }
        }
        Err(SyncOutcome::NoSession)
    }

    // ---- Merging -------------------------------------------------------------------

    /// The key a Twitch conversation lives under: the other person's id. A
    /// conversation filed under their login (an old import) moves to the id.
    fn key_for(&self, scope: &Scope, thread: &GqlThread, viewer: &str) -> Result<Option<String>, String> {
        let Some(id) = other_id(thread, viewer) else { return Ok(None) };
        let login = thread.other.as_ref().map(|o| o.login.to_lowercase()).unwrap_or_default();
        let found = whisper_store::read_index(scope, |idx| {
            if idx.conversations.iter().any(|m| m.key == id) {
                return None;
            }
            idx.conversations
                .iter()
                .find(|m| !login.is_empty() && (m.key == login || m.user_login.to_lowercase() == login))
                .map(|m| m.key.clone())
        })?;
        if let Some(from) = found {
            whisper_store::rekey(scope, &from, &id)?;
        }
        Ok(Some(id))
    }

    /// Merge one page of a conversation into the store.
    fn merge(&self, scope: &Scope, thread: &GqlThread, mode: Mode, page_seq: u64, device: Option<&str>) -> Result<Option<(String, Merged)>, String> {
        let viewer = thread.messages.viewer_id.clone();
        let Some(key) = self.key_for(scope, thread, &viewer)? else { return Ok(None) };
        let reading = crate::services::whisper_claims::reading(&scope.owner, &key);
        let create = fresh_meta(&key, thread);
        let out = whisper_store::update(scope, &key, Some(create), |conv| {
            let mut m = Merged { has_next: thread.messages.has_next, next_cursor: thread.messages.next_cursor.clone(), ..Merged::default() };
            // Where a gap this page opens will end: what was held before it.
            let floor_before = newest_twitch(&conv.messages);
            m.changed |= merge_twitch_meta(&mut conv.meta, thread, page_seq);
            let me_p = thread.me.as_ref();
            let me = Who {
                id: &viewer,
                login: me_p.map(|p| p.login.as_str()).unwrap_or(""),
                name: me_p.map(|p| p.display_name.as_str()).unwrap_or(""),
            };
            let other_login = conv.meta.user_login.clone();
            let other_name = conv.meta.user_name.clone();
            let other = Who { id: &key, login: &other_login, name: &other_name };
            // Known ids take Twitch's copy; the rest are offered to
            // whisper_send::adopt in one batch (it pairs Twitch's copies of our
            // own sends with their local copies, oldest first, and takes the
            // adopted ones out); what is left is new.
            let mut fresh: Vec<StoredWhisper> = Vec::new();
            for gm in &thread.messages.messages {
                let tw = stored_of(gm, &viewer, &me, &other);
                if let Some(local) = conv.messages.iter_mut().find(|x| x.id == tw.id) {
                    m.hit_known = true;
                    m.changed |= take_twitch(local, &tw);
                    continue;
                }
                fresh.push(tw);
            }
            if crate::services::whisper_send::adopt(&mut conv.messages, &mut fresh, &viewer) > 0 {
                m.hit_known = true;
                m.changed = true;
            }
            for tw in fresh {
                conv.messages.push(tw);
                m.added += 1;
                m.changed = true;
            }
            let more = thread.messages.has_next;
            let cursor = |c: &Option<String>| c.clone().map(|cursor| Cursor { cursor, device_id: device.map(str::to_string) });
            let deep_before = conv.meta.deep.clone();
            let gap_before = conv.meta.gap.clone();
            let floor_held = conv.meta.gap_floor.clone();
            // A gap keeps the floor of the first sync that left it open, so paging
            // from the newest page reaches everything both missed.
            let open_gap = |meta: &mut ConversationMeta, at: Option<Cursor>| {
                meta.gap = at.or(Some(Cursor { cursor: String::new(), device_id: None }));
                if meta.gap_floor.is_none() {
                    meta.gap_floor = floor_before.clone();
                }
            };
            match mode {
                Mode::Nested { first_import } => {
                    if !more {
                        conv.meta.deep = Deep::Complete;
                        conv.meta.gap = None;
                        conv.meta.gap_floor = None;
                    } else if first_import || conv.meta.deep == Deep::Unknown {
                        if conv.meta.deep != Deep::Complete {
                            conv.meta.deep = Deep::Partial { cursor: cursor(&m.next_cursor) };
                        }
                    } else if !m.hit_known {
                        open_gap(&mut conv.meta, cursor(&m.next_cursor));
                    }
                }
                Mode::Refresh => {
                    if !more && conv.meta.deep == Deep::Unknown {
                        conv.meta.deep = Deep::Complete;
                    } else if more && !m.hit_known {
                        open_gap(&mut conv.meta, cursor(&m.next_cursor));
                    }
                }
                Mode::Deep => {
                    conv.meta.deep = if more { Deep::Partial { cursor: cursor(&m.next_cursor) } } else { Deep::Complete };
                }
                Mode::Gap => {
                    // Meeting what the last sync just merged proves nothing: only
                    // reaching the floor (or the end of history) closes the gap.
                    let reached = floor_held.as_ref().is_some_and(|f| reaches_floor(&thread.messages.messages, f));
                    if reached || !more {
                        if !more {
                            conv.meta.deep = Deep::Complete;
                        }
                        conv.meta.gap = None;
                        conv.meta.gap_floor = None;
                        m.gap_closed = true;
                    } else {
                        conv.meta.gap = cursor(&m.next_cursor);
                    }
                }
            }
            m.changed |= conv.meta.deep != deep_before || conv.meta.gap != gap_before || conv.meta.gap_floor != floor_held;
            if conv.meta.deep == Deep::Complete {
                m.changed |= drop_scraped(&mut conv.meta, &mut conv.messages);
            }
            conv.messages.sort_by(|a, b| (a.timestamp, &a.id).cmp(&(b.timestamp, &b.id)));
            if reading {
                if let Some(newest) = conv.messages.iter().filter(|x| !x.is_sent && !x.deleted).max_by_key(|x| (x.timestamp, x.id.clone())) {
                    let marker = Marker { id: newest.id.clone(), at_ms: newest.timestamp };
                    if conv.meta.read_through.as_ref() != Some(&marker) {
                        conv.meta.read_through = Some(marker);
                        conv.meta.read_seq = whisper_store::next_seq();
                        if newest.origin == Origin::Twitch {
                            conv.meta.twitch_read_owed = Some(newest.id.clone());
                            m.owe = true;
                        }
                        m.changed = true;
                    }
                }
            }
            let changed = m.changed;
            (m, changed)
        })?;
        Ok(out.map(|m| (key, m)))
    }

    // ---- Owed writes ---------------------------------------------------------------

    /// A mutation with the integrity rule: without a token first, and once more
    /// with one if Twitch asks for it.
    async fn mutate<F, Fut>(&self, run: &mut Run, mut f: F) -> Result<(), GqlError>
    where
        F: FnMut(Option<Integrity>) -> Fut,
        Fut: Future<Output = Result<(), GqlError>>,
    {
        match self.paced(|| f(None)).await {
            Err(GqlError::Integrity) => {
                let Some(integ) = self.integrity(run).await else { return Err(GqlError::Integrity) };
                self.paced(|| f(Some(integ.clone()))).await
            }
            other => other,
        }
    }

    /// Send what one conversation owes Twitch: its read marker and an archive.
    async fn send_owed_once(&self, scope: &Scope, key: &str, gql: &dyn Gql, run: &mut Run) -> Result<(), String> {
        let Some(meta) = whisper_store::read_index(scope, |idx| idx.conversations.iter().find(|m| m.key == key).cloned())? else {
            return Ok(());
        };
        let Some(thread_id) = meta.thread_id.clone().or_else(|| {
            meta.user_id.parse::<u64>().ok().map(|_| whisper_gql::thread_id(&scope.owner, &meta.user_id))
        }) else {
            return Ok(());
        };
        if let Some(owed) = meta.twitch_read_owed.clone() {
            let target = whisper_store::read(scope, key, |conv| {
                conv.messages.iter().position(|m| m.id == owed).map(|i| (i, conv.messages[i].timestamp, conv.messages.clone()))
            })?
            .flatten();
            // Never move Twitch's marker backwards: an owed message at or before
            // the marker Twitch already has (by position when Twitch's marker is
            // held here, else by time) is simply dropped. An owed id no longer
            // held is dropped too.
            let behind = match (&target, &meta.twitch_read) {
                (None, _) => true,
                (Some(_), None) => false,
                (Some((pos, at, msgs)), Some(tr)) => match msgs.iter().position(|m| m.id == tr.id) {
                    Some(tr_pos) => tr_pos >= *pos,
                    None => tr.at_ms >= *at,
                },
            };
            if behind {
                whisper_store::update(scope, key, None, |conv| {
                    let hit = conv.meta.twitch_read_owed.as_deref() == Some(owed.as_str());
                    if hit {
                        conv.meta.twitch_read_owed = None;
                    }
                    ((), hit)
                })?;
            } else if let Some((_, at, _)) = target {
                let sent = self.mutate(run, |integ| {
                    let thread_id = thread_id.clone();
                    let owed = owed.clone();
                    async move { whisper_gql::mark_read(gql, &thread_id, &owed, integ.as_ref()).await }
                })
                .await;
                match sent {
                    Ok(()) => {
                        whisper_store::update(scope, key, None, |conv| {
                            conv.meta.twitch_read = Some(Marker { id: owed.clone(), at_ms: at });
                            conv.meta.read_seq = whisper_store::next_seq();
                            if conv.meta.twitch_read_owed.as_deref() == Some(owed.as_str()) {
                                conv.meta.twitch_read_owed = None;
                            }
                            ((), true)
                        })?;
                    }
                    Err(e) => log::info!("[WhisperSync] mark read stays owed: {}", e.words()),
                }
            }
        }
        if meta.archive_pending {
            let newer_whisper = whisper_store::read(scope, key, |conv| {
                let asked = conv.meta.archive_requested_ms.unwrap_or(i64::MAX);
                conv.messages.iter().any(|m| !m.is_sent && m.timestamp > asked)
            })?
            .unwrap_or(false);
            if newer_whisper {
                whisper_store::update(scope, key, None, |conv| {
                    conv.meta.archive_pending = false;
                    ((), true)
                })?;
            } else {
                let sent = self.mutate(run, |integ| {
                    let thread_id = thread_id.clone();
                    async move { whisper_gql::archive(gql, &thread_id, integ.as_ref()).await }
                })
                .await;
                if sent.is_ok() {
                    whisper_store::update(scope, key, None, |conv| {
                        conv.meta.archive_pending = false;
                        conv.meta.archived = true;
                        conv.meta.archived_seq = whisper_store::next_seq();
                        ((), true)
                    })?;
                }
            }
        }
        Ok(())
    }

    /// Send a conversation's owed writes in the background; one at a time per
    /// conversation, and once more if more was owed meanwhile.
    fn send_owed(self: &Arc<Self>, key: String) {
        {
            let mut owed = lock(&self.owed);
            if let Some(again) = owed.get_mut(&key) {
                *again = true;
                return;
            }
            owed.insert(key.clone(), false);
        }
        let me = Arc::clone(self);
        (self.env.spawn)(Box::pin(async move {
            loop {
                if let Some(scope) = me.scope() {
                    if let Ok(gql) = me.session(&scope).await {
                        let mut run = Run::new(false);
                        if let Err(e) = me.send_owed_once(&scope, &key, &*gql, &mut run).await {
                            log::info!("[WhisperSync] owed write for a conversation not sent: {e}");
                        }
                        me.env.events.conversation(&scope, &key);
                        me.env.events.counts();
                    }
                }
                let mut owed = lock(&me.owed);
                if owed.get(&key).copied() == Some(true) {
                    owed.insert(key.clone(), false);
                    continue;
                }
                owed.remove(&key);
                break;
            }
        }));
    }

    // ---- The run -------------------------------------------------------------------

    async fn run_once(&self, scope: &Scope, reason: &Reason) -> SyncOutcome {
        self.env.events.archive_changed(&scope.owner, true);
        let outcome = self.run_inner(scope, reason).await;
        self.env.events.archive_changed(&scope.owner, false);
        self.env.events.counts();
        match &outcome {
            SyncOutcome::Done { added } | SyncOutcome::Partial { added } => {
                log::warn!("[WhisperSync] run done: {added} new messages, outcome {:?}", outcome)
            }
            SyncOutcome::Failed { reason } => log::warn!("[WhisperSync] run failed: {reason}"),
            _ => {}
        }
        outcome
    }

    async fn run_inner(&self, scope: &Scope, reason: &Reason) -> SyncOutcome {
        self.settle_once(scope);
        let gql = match self.session(scope).await {
            Ok(g) => g,
            Err(outcome) => return outcome,
        };
        let mut run = Run::new(false);
        let owed: Vec<String> = match whisper_store::read_index(scope, |idx| {
            idx.conversations
                .iter()
                .filter(|m| m.twitch_read_owed.is_some() || m.archive_pending)
                .map(|m| m.key.clone())
                .collect()
        }) {
            Ok(k) => k,
            Err(e) if is_closed(&e) => return SyncOutcome::Superseded,
            Err(e) => return SyncOutcome::Failed { reason: e },
        };
        for key in owed {
            if let Err(e) = self.send_owed_once(scope, &key, &*gql, &mut run).await {
                if is_closed(&e) {
                    return SyncOutcome::Superseded;
                }
            }
        }
        if reason.reads_list() {
            if let Err(outcome) = self.list_phase(scope, &*gql, &mut run).await {
                return outcome;
            }
        }
        if let Err(outcome) = self.deep_phase(scope, &*gql, &mut run).await {
            return outcome;
        }
        if run.partial {
            SyncOutcome::Partial { added: run.added }
        } else {
            SyncOutcome::Done { added: run.added }
        }
    }

    fn fail(e: String) -> SyncOutcome {
        if is_closed(&e) {
            SyncOutcome::Superseded
        } else {
            SyncOutcome::Failed { reason: e }
        }
    }

    fn gql_fail(e: GqlError) -> SyncOutcome {
        match e {
            GqlError::Unauthorized => SyncOutcome::NoSession,
            e => SyncOutcome::Failed { reason: e.words() },
        }
    }

    /// Merge a list page; returns whether any conversation on it changed.
    fn merge_list_page(&self, scope: &Scope, page: &ThreadsPage, first_import: bool, page_seq: u64, run: &mut Run, owe: &mut Vec<String>) -> Result<bool, SyncOutcome> {
        let mut any = false;
        for thread in &page.threads {
            match self.merge(scope, thread, Mode::Nested { first_import }, page_seq, None) {
                Ok(Some((key, m))) => {
                    run.added += m.added;
                    any |= m.changed;
                    if m.owe {
                        owe.push(key);
                    }
                }
                Ok(None) => {}
                Err(e) => return Err(Self::fail(e)),
            }
        }
        Ok(any)
    }

    async fn list_phase(&self, scope: &Scope, gql: &dyn Gql, run: &mut Run) -> Result<(), SyncOutcome> {
        let (imported, list_deep) = whisper_store::read_index(scope, |idx| (idx.imported_at, idx.list_deep.clone())).map_err(Self::fail)?;
        let first_import = imported.is_none();
        let nested = if first_import { PAGE } else { NESTED_INCREMENTAL };
        let mut owe = Vec::new();
        let page_seq = whisper_store::next_seq();
        let page1 = self.paced(|| whisper_gql::threads(gql, PAGE, nested, None, None)).await.map_err(Self::gql_fail)?;
        if page1.viewer_id != scope.owner {
            *lock(&self.problem) = Some(SESSION_PROBLEM.to_string());
            return Err(SyncOutcome::WrongSession);
        }
        let changed = self.merge_list_page(scope, &page1, first_import, page_seq, run, &mut owe)?;
        let now = self.now();
        whisper_store::update_index(scope, |idx| {
            if idx.imported_at.is_none() {
                idx.imported_at = Some(now);
                ((), true)
            } else {
                ((), false)
            }
        })
        .map_err(Self::fail)?;
        if !page1.has_next {
            whisper_store::update_index(scope, |idx| {
                let changed = idx.list_deep != Deep::Complete;
                idx.list_deep = Deep::Complete;
                ((), changed)
            })
            .map_err(Self::fail)?;
        } else if first_import || matches!(list_deep, Deep::Partial { .. } | Deep::Unknown) || changed {
            self.list_more(scope, gql, run, first_import, nested, &list_deep, &mut owe).await?;
        }
        for key in owe {
            self.send_owed_now(scope, &key, gql, run).await;
        }
        Ok(())
    }

    async fn send_owed_now(&self, scope: &Scope, key: &str, gql: &dyn Gql, run: &mut Run) {
        if let Err(e) = self.send_owed_once(scope, key, gql, run).await {
            log::info!("[WhisperSync] owed write not sent: {e}");
        }
    }

    /// Pages 2 and on of the list, with a token. A stored cursor is used only
    /// when it was made under the token's device id; otherwise page 1 is read
    /// again under the token and paging goes on from its cursor.
    #[allow(clippy::too_many_arguments)]
    async fn list_more(&self, scope: &Scope, gql: &dyn Gql, run: &mut Run, first_import: bool, nested: u32, stored: &Deep, owe: &mut Vec<String>) -> Result<(), SyncOutcome> {
        let Some(mut integ) = self.integrity(run).await else {
            run.partial = true;
            return self.set_list_deep(scope, Deep::Partial { cursor: None });
        };
        let restart_key = format!("list:{}", scope.owner);
        let mut cursor: Option<String> = match stored {
            Deep::Partial { cursor: Some(c) } if c.device_id.as_deref() == Some(integ.device_id.as_str()) && !c.cursor.is_empty() => Some(c.cursor.clone()),
            _ => None,
        };
        let mut resumed = cursor.is_some();
        loop {
            let page_seq = whisper_store::next_seq();
            let answer = match &cursor {
                None => self.paced(|| whisper_gql::threads(gql, PAGE, 1, None, Some(&integ))).await,
                Some(c) => self.paced(|| whisper_gql::threads(gql, PAGE, nested, Some(c), Some(&integ))).await,
            };
            let page = match answer {
                Ok(p) => p,
                Err(GqlError::Integrity) => {
                    if self.refused_token(run) {
                        if let Some(fresh) = self.integrity(run).await {
                            integ = fresh;
                            cursor = None;
                            continue;
                        }
                    }
                    run.partial = true;
                    let at = cursor.clone().map(|c| Cursor { cursor: c, device_id: Some(integ.device_id.clone()) });
                    return self.set_list_deep(scope, Deep::Partial { cursor: at });
                }
                Err(GqlError::Other(_)) | Err(GqlError::Shape(_)) if resumed && lock(&self.restarted).insert(restart_key.clone()) => {
                    resumed = false;
                    cursor = None;
                    continue;
                }
                Err(e) => {
                    run.partial = true;
                    self.set_list_deep(scope, Deep::Partial { cursor: None })?;
                    return Err(Self::gql_fail(e));
                }
            };
            if page.viewer_id != scope.owner {
                return Err(SyncOutcome::WrongSession);
            }
            let changed = if cursor.is_none() {
                // Page 1 again, read only for its cursor.
                true
            } else {
                self.merge_list_page(scope, &page, first_import, page_seq, run, owe)?
            };
            if !page.has_next {
                return self.set_list_deep(scope, Deep::Complete);
            }
            let Some(next) = page.next_cursor.clone() else {
                run.partial = true;
                return self.set_list_deep(scope, Deep::Partial { cursor: None });
            };
            if !first_import && !changed && cursor.is_some() {
                // Nothing on this page changed: later pages are older still.
                return self.set_list_deep(scope, Deep::Partial { cursor: Some(Cursor { cursor: next, device_id: Some(integ.device_id.clone()) }) })
                    .map(|_| ());
            }
            self.set_list_deep(scope, Deep::Partial { cursor: Some(Cursor { cursor: next.clone(), device_id: Some(integ.device_id.clone()) }) })?;
            cursor = Some(next);
            resumed = false;
        }
    }

    fn set_list_deep(&self, scope: &Scope, deep: Deep) -> Result<(), SyncOutcome> {
        whisper_store::update_index(scope, |idx| {
            let changed = idx.list_deep != deep;
            idx.list_deep = deep;
            ((), changed)
        })
        .map_err(Self::fail)
    }

    /// Older history and gaps, for every conversation that has some left.
    async fn deep_phase(&self, scope: &Scope, gql: &dyn Gql, run: &mut Run) -> Result<(), SyncOutcome> {
        let todo: Vec<(String, String, Mode, Option<Cursor>)> = whisper_store::read_index(scope, |idx| {
            idx.conversations
                .iter()
                .filter_map(|m| {
                    let thread = m.thread_id.clone().or_else(|| m.user_id.parse::<u64>().ok().map(|_| whisper_gql::thread_id(&scope.owner, &m.user_id)))?;
                    if let Some(g) = &m.gap {
                        return Some((m.key.clone(), thread, Mode::Gap, Some(g.clone())));
                    }
                    match &m.deep {
                        Deep::Partial { cursor } => Some((m.key.clone(), thread, Mode::Deep, cursor.clone())),
                        _ => None,
                    }
                })
                .collect()
        })
        .map_err(Self::fail)?;
        for (key, thread_id, mode, stored) in todo {
            if lock(&self.loading).contains(&key) {
                continue;
            }
            let Some(integ) = self.integrity(run).await else {
                run.partial = true;
                return Ok(());
            };
            match self.page_history(scope, gql, run, &key, &thread_id, mode, stored, integ, None, false).await {
                Ok(added) => run.added += added,
                Err(HistoryEnd::Partial) => run.partial = true,
                Err(HistoryEnd::Stop(outcome)) => return Err(outcome),
            }
        }
        Ok(())
    }

    /// Page one conversation's older history (or its gap) with a token, until
    /// it is complete, the gap closes, `max_pages` is reached, a page adds
    /// something when `until_new` is set, or Twitch refuses. A stored cursor
    /// made under another device id, or refused once, restarts from the newest
    /// page. Every merged page stores where to go on from.
    #[allow(clippy::too_many_arguments)]
    async fn page_history(
        &self,
        scope: &Scope,
        gql: &dyn Gql,
        run: &mut Run,
        key: &str,
        thread_id: &str,
        mode: Mode,
        stored: Option<Cursor>,
        mut integ: Integrity,
        max_pages: Option<u32>,
        until_new: bool,
    ) -> Result<u32, HistoryEnd> {
        let restart_key = format!("deep:{}:{}", scope.owner, key);
        let mut cursor: Option<String> = stored
            .filter(|c| c.device_id.as_deref() == Some(integ.device_id.as_str()) && !c.cursor.is_empty())
            .map(|c| c.cursor);
        let mut resumed = cursor.is_some();
        let mut added = 0;
        let mut pages = 0;
        loop {
            if max_pages.is_some_and(|m| pages >= m) {
                return Ok(added);
            }
            let page_seq = whisper_store::next_seq();
            let answer = self
                .paced(|| whisper_gql::messages(gql, thread_id, PAGE, cursor.as_deref(), Some(&integ)))
                .await;
            pages += 1;
            let thread = match answer {
                Ok(Some(t)) => t,
                Ok(None) => {
                    whisper_store::update(scope, key, None, |conv| {
                        conv.meta.deep = Deep::Complete;
                        conv.meta.gap = None;
                        conv.meta.gap_floor = None;
                        ((), true)
                    })
                    .map_err(|e| HistoryEnd::Stop(Self::fail(e)))?;
                    return Ok(added);
                }
                Err(GqlError::Integrity) => {
                    if self.refused_token(run) {
                        if let Some(fresh) = self.integrity(run).await {
                            integ = fresh;
                            cursor = None;
                            continue;
                        }
                    }
                    self.keep_cursor(scope, key, mode, cursor.clone(), &integ).map_err(HistoryEnd::Stop)?;
                    return Err(HistoryEnd::Partial);
                }
                Err(GqlError::Other(_)) | Err(GqlError::Shape(_)) if resumed && lock(&self.restarted).insert(restart_key.clone()) => {
                    resumed = false;
                    cursor = None;
                    continue;
                }
                Err(GqlError::Unauthorized) => return Err(HistoryEnd::Stop(SyncOutcome::NoSession)),
                Err(e) => {
                    log::info!("[WhisperSync] history page refused: {}", e.words());
                    return Err(HistoryEnd::Partial);
                }
            };
            if thread.messages.viewer_id != scope.owner {
                return Err(HistoryEnd::Stop(SyncOutcome::WrongSession));
            }
            let device = Some(integ.device_id.as_str());
            let merged = self.merge(scope, &thread, mode, page_seq, device).map_err(|e| HistoryEnd::Stop(Self::fail(e)))?;
            let Some((_, m)) = merged else { return Ok(added) };
            added += m.added;
            if m.owe {
                self.send_owed_now(scope, key, gql, run).await;
            }
            let done = match mode {
                Mode::Gap => m.gap_closed || !m.has_next,
                _ => !m.has_next,
            };
            if done || (until_new && m.added > 0) {
                return Ok(added);
            }
            let Some(next) = m.next_cursor else { return Err(HistoryEnd::Partial) };
            cursor = Some(next);
            resumed = false;
        }
    }

    fn keep_cursor(&self, scope: &Scope, key: &str, mode: Mode, cursor: Option<String>, integ: &Integrity) -> Result<(), SyncOutcome> {
        let at = cursor.map(|c| Cursor { cursor: c, device_id: Some(integ.device_id.clone()) });
        whisper_store::update(scope, key, None, |conv| {
            match mode {
                Mode::Gap => conv.meta.gap = at.clone().or(conv.meta.gap.clone()),
                _ => {
                    if conv.meta.deep != Deep::Complete {
                        conv.meta.deep = Deep::Partial { cursor: at.clone() };
                    }
                }
            }
            ((), true)
        })
        .map(|_| ())
        .map_err(Self::fail)
    }

    // ---- One conversation ------------------------------------------------------------

    fn spawn_refresh(self: &Arc<Self>, key: String, delay_ms: u64) {
        let me = Arc::clone(self);
        (self.env.spawn)(Box::pin(async move {
            if delay_ms > 0 {
                me.env.sleeper.sleep(delay_ms).await;
            }
            lock(&me.debounced).remove(&key);
            if let Err(e) = me.refresh_thread(key).await {
                log::info!("[WhisperSync] conversation refresh: {e}");
            }
        }));
    }

    /// Read a conversation's newest messages and merge them: replies sent from
    /// another device, Twitch's emote parts for live whispers, and the real
    /// id of a whisper we sent. Never mints; a gap it finds is left for the
    /// next sync (and a partial-only one is asked for).
    ///
    /// Returns only after a read that started after this call: a caller that
    /// finds a read already running waits for it, then reads again unless
    /// another caller's newer read already succeeded. A delivery check or a
    /// Retry that gets Ok has seen Twitch's thread as it was when it asked.
    async fn refresh_thread(self: &Arc<Self>, key: String) -> Result<(), String> {
        let Some(scope) = self.scope() else { return Ok(()) };
        let asked = self.refresh_order.fetch_add(1, Ordering::SeqCst) + 1;
        loop {
            let wait = {
                let mut map = lock(&self.refreshing);
                let slot = map
                    .entry(key.clone())
                    .or_insert_with(|| RefreshSlot { running: false, last_ok: 0, ended: tokio::sync::watch::channel(0).0 });
                if slot.last_ok >= asked {
                    return Ok(());
                }
                if slot.running {
                    Some(slot.ended.subscribe())
                } else {
                    slot.running = true;
                    None
                }
            };
            let Some(mut ended) = wait else {
                let started = self.refresh_order.fetch_add(1, Ordering::SeqCst) + 1;
                let result = self.refresh_inner(&scope, &key).await;
                let mut map = lock(&self.refreshing);
                if let Some(slot) = map.get_mut(&key) {
                    slot.running = false;
                    if result.is_ok() {
                        slot.last_ok = slot.last_ok.max(started);
                    }
                    slot.ended.send_replace(slot.last_ok);
                }
                return result;
            };
            if ended.changed().await.is_err() {
                return Err("the conversation refresh stopped".into());
            }
        }
    }

    async fn refresh_inner(self: &Arc<Self>, scope: &Scope, key: &str) -> Result<(), String> {
        self.settle_once(scope);
        let Some(meta) = whisper_store::read_index(scope, |idx| idx.conversations.iter().find(|m| m.key == key).cloned())? else {
            return Ok(());
        };
        let Some(thread_id) = meta.thread_id.clone().or_else(|| {
            meta.user_id.parse::<u64>().ok().map(|_| whisper_gql::thread_id(&scope.owner, &meta.user_id))
        }) else {
            return Ok(());
        };
        let gql = self.session(scope).await.map_err(|o| format!("{o:?}"))?;
        let page_seq = whisper_store::next_seq();
        let thread = self
            .paced(|| whisper_gql::messages(&*gql, &thread_id, REFRESH_FIRST, None, None))
            .await
            .map_err(|e| e.words())?;
        let Some(thread) = thread else { return Ok(()) };
        if thread.messages.viewer_id != scope.owner {
            return Err("the twitch.tv session belongs to another account".into());
        }
        let merged = self.merge(scope, &thread, Mode::Refresh, page_seq, None)?;
        if let Some((key, m)) = merged {
            if m.owe {
                self.send_owed(key.clone());
            }
            if m.changed {
                self.env.events.conversation(scope, &key);
                self.env.events.counts();
            }
            if m.has_next && !m.hit_known {
                self.trigger(Reason::ResumePartial);
            }
        }
        Ok(())
    }

    /// Load older: one page past what is held, minting even inside the
    /// back-off (one attempt per tap). Skipped by the sync while it runs.
    async fn load_older(self: &Arc<Self>, key: String) -> Result<u32, String> {
        let Some(scope) = self.scope() else { return Err("You are not signed in to Twitch.".into()) };
        if !lock(&self.loading).insert(key.clone()) {
            return Ok(0);
        }
        let result = tokio::time::timeout(LOAD_OLDER_WAIT, self.load_older_inner(&scope, &key)).await;
        lock(&self.loading).remove(&key);
        match result {
            Ok(r) => r,
            Err(_) => Err("Twitch took too long to answer. Try again in a moment.".into()),
        }
    }

    async fn load_older_inner(self: &Arc<Self>, scope: &Scope, key: &str) -> Result<u32, String> {
        let Some(meta) = whisper_store::read_index(scope, |idx| idx.conversations.iter().find(|m| m.key == key).cloned())? else {
            return Ok(0);
        };
        let Some(thread_id) = meta.thread_id.clone().or_else(|| {
            meta.user_id.parse::<u64>().ok().map(|_| whisper_gql::thread_id(&scope.owner, &meta.user_id))
        }) else {
            return Ok(0);
        };
        let (mode, stored) = match (&meta.gap, &meta.deep) {
            (Some(g), _) => (Mode::Gap, Some(g.clone())),
            (None, Deep::Partial { cursor }) => (Mode::Deep, cursor.clone()),
            _ => return Ok(0),
        };
        let gql = self.session(scope).await.map_err(|o| format!("{o:?}"))?;
        let mut run = Run::new(true);
        let Some(integ) = self.integrity(&mut run).await else {
            return Err("Twitch's security check did not answer. Try again in a moment.".into());
        };
        // One page when the stored cursor is usable; a restart from the newest
        // page reads until something new is added.
        let usable = stored.as_ref().is_some_and(|c| c.device_id.as_deref() == Some(integ.device_id.as_str()) && !c.cursor.is_empty());
        let (max, until_new) = if usable { (Some(1), false) } else { (None, true) };
        let added = match self.page_history(scope, &*gql, &mut run, key, &thread_id, mode, stored, integ, max, until_new).await {
            Ok(a) => a,
            Err(HistoryEnd::Partial) => 0,
            Err(HistoryEnd::Stop(o)) => return Err(format!("{o:?}")),
        };
        self.env.events.conversation(scope, key);
        self.env.events.counts();
        Ok(added)
    }
}

enum HistoryEnd {
    Partial,
    Stop(SyncOutcome),
}

// ---- Process-wide entry points -------------------------------------------------------------

static ENGINE: once_cell::sync::Lazy<Arc<Engine>> = once_cell::sync::Lazy::new(|| {
    Arc::new(Engine::new(Env {
        session: Arc::new(AppWebSession),
        clock: Arc::new(SystemClock),
        sleeper: Arc::new(TokioSleeper),
        minter: Arc::new(IntegrityMinter),
        events: Arc::new(AppEvents),
        spawn: Arc::new(|f| {
            tauri::async_runtime::spawn(f);
        }),
    }))
});

fn engine() -> &'static Arc<Engine> {
    &ENGINE
}

/// Follow the signed-in Twitch account (None: signed out). Called from
/// `whisper_service::follow_account` on every roster build; only a change of
/// account does anything.
pub fn follow_account(user_id: Option<String>) {
    #[cfg(feature = "whisper-probe")]
    let probe_for = user_id.clone();
    if let Some(closing) = engine().follow(user_id) {
        tauri::async_runtime::spawn(teardown(closing));
    }
    #[cfg(feature = "whisper-probe")]
    if probe_for.is_some() && !PROBED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        tauri::async_runtime::spawn(async {
            let _ = sync_now().await;
            probe().await;
        });
    }
}

#[cfg(feature = "whisper-probe")]
static PROBED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A depth proof for a probe build only (`--features whisper-probe`): after
/// the first sync of the process, page the conversation list and the longest
/// conversation 20 at a time, every page under the integrity token's device,
/// and log the counts beside what the store holds. Counts only, no ids, no
/// text. Read-only by construction: it reaches only the two read documents,
/// never mark read, archive or a send.
#[cfg(feature = "whisper-probe")]
pub async fn probe() {
    let e = engine();
    let Some(scope) = current_scope() else {
        log::warn!("[WhisperProbe] nobody signed in");
        return;
    };
    let gql = match e.session(&scope).await {
        Ok(g) => g,
        Err(outcome) => {
            log::warn!("[WhisperProbe] no session: {outcome:?}");
            return;
        }
    };
    let mut run = Run::new(true);
    let integ = e.integrity(&mut run).await;
    let mut threads = 0usize;
    let mut list_pages = 0usize;
    let mut longest: Option<(String, String, usize, bool)> = None; // thread id, other id, page-1 count, more
    let mut cursor: Option<String> = None;
    loop {
        let page = match whisper_gql::threads(&*gql, 20, 100, cursor.as_deref(), integ.as_ref()).await {
            Ok(p) => p,
            Err(err) => {
                log::warn!("[WhisperProbe] list page {} failed: {err:?}", list_pages + 1);
                break;
            }
        };
        list_pages += 1;
        threads += page.threads.len();
        for t in &page.threads {
            let n = t.messages.messages.len();
            let more = t.messages.has_next;
            let longer = match &longest {
                None => true,
                Some((_, _, best, best_more)) => (more && !best_more) || (more == *best_more && n > *best),
            };
            if longer {
                let other = t.other.as_ref().map(|o| o.id.clone()).unwrap_or_default();
                longest = Some((t.thread_id.clone(), other, n, more));
            }
        }
        if !page.has_next || page.next_cursor.is_none() || list_pages >= 50 {
            break;
        }
        cursor = page.next_cursor;
    }
    let mut messages = 0usize;
    let mut message_pages = 0usize;
    let mut stored = 0usize;
    if let Some((thread_id, other, _, _)) = &longest {
        let mut cursor: Option<String> = None;
        loop {
            match whisper_gql::messages(&*gql, thread_id, 20, cursor.as_deref(), integ.as_ref()).await {
                Ok(Some(t)) => {
                    message_pages += 1;
                    messages += t.messages.messages.len();
                    if !t.messages.has_next || t.messages.next_cursor.is_none() || message_pages >= 500 {
                        break;
                    }
                    cursor = t.messages.next_cursor;
                }
                Ok(None) => break,
                Err(err) => {
                    log::warn!("[WhisperProbe] message page {} failed: {err:?}", message_pages + 1);
                    break;
                }
            }
        }
        stored = whisper_store::read(&scope, other, |c| c.messages.iter().filter(|m| m.origin == Origin::Twitch).count())
            .ok()
            .flatten()
            .unwrap_or(0);
    }
    let local = whisper_store::read_index(&scope, |idx| idx.conversations.len()).unwrap_or(0);
    log::warn!(
        "[WhisperProbe] token {}: Twitch lists {threads} conversations over {list_pages} pages (StreamNook holds {local}); the longest has {messages} messages over {message_pages} pages (StreamNook holds {stored} of Twitch's)",
        if integ.is_some() { "minted" } else { "missing" },
    );
}

/// What the old account leaves behind: its queued sends end, its data
/// reaches disk, then it leaves memory (only after a good flush).
async fn teardown(mut closing: Closing) {
    crate::services::whisper_send::teardown(&closing).await;
    for _ in 0..5 {
        if whisper_store::flush_closing(&closing).is_ok() {
            match whisper_store::drop_owner(closing) {
                Ok(()) => return,
                Err(c) => closing = c,
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    log::warn!("[WhisperSync] the signed-out account's whispers could not be flushed; kept in memory");
}

pub fn current_scope() -> Option<Scope> {
    engine().scope()
}

pub fn trigger(reason: Reason) {
    engine().trigger(reason);
}

pub async fn sync_now() -> SyncOutcome {
    engine().sync_now().await
}

pub async fn load_older(key: String) -> Result<u32, String> {
    engine().load_older(key).await
}

pub async fn refresh_thread(key: String) -> Result<(), String> {
    engine().refresh_thread(key).await
}

pub fn send_owed(key: String) {
    engine().send_owed(key);
}

pub fn note_socket_up(first_of_process: bool) {
    engine().note_socket_up(first_of_process);
}

pub fn note_socket_down() {
    engine().note_socket_down();
}

/// The UI left the screen (`true`: every window minimized or in the tray, the
/// phone app in the background) or came back. Coming back after ten minutes
/// syncs, and sends still waiting are checked again.
pub fn note_away(away: bool) {
    engine().note_away(away);
}

pub fn note_resumed() {
    note_away(false);
}

pub fn has_partial() -> bool {
    engine().has_partial()
}

/// Every EventSub frame carries Twitch's own time; the difference from ours
/// stamps our sends so they sort among Twitch's messages.
pub fn note_twitch_time(twitch_ms: i64) {
    let local = engine().now();
    engine().offset_ms.store(twitch_ms - local, Ordering::Relaxed);
}

pub fn twitch_now(local_ms: i64) -> i64 {
    local_ms + engine().offset_ms.load(Ordering::Relaxed)
}

/// Twitch time minus local time, 0 until measured (whisper_send and
/// whisper_inbox read it).
pub fn twitch_offset_ms() -> i64 {
    engine().offset_ms.load(Ordering::Relaxed)
}

pub fn syncing() -> bool {
    lock(&engine().ctrl).running
}

pub fn session_problem() -> Option<String> {
    lock(&engine().problem).clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::atomic::AtomicU32;

    // ---- Fakes ------------------------------------------------------------------

    struct FakeClock(AtomicI64);
    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    /// Sleeping moves the fake clock instead of waiting.
    struct FakeSleeper {
        clock: Arc<FakeClock>,
        slept: Mutex<Vec<u64>>,
    }
    impl Sleeper for FakeSleeper {
        fn sleep(&self, ms: u64) -> Pin<Box<dyn Future<Output = ()> + Send>> {
            self.clock.0.fetch_add(ms as i64, Ordering::SeqCst);
            lock(&self.slept).push(ms);
            Box::pin(async {})
        }
    }
    struct FakeMinter {
        ok: Mutex<bool>,
        device: String,
        mints: AtomicU32,
        cached: Mutex<Option<Integrity>>,
    }
    impl FakeMinter {
        fn token(&self) -> Integrity {
            Integrity { token: "integrity".into(), device_id: self.device.clone(), user_agent: "ua".into() }
        }
    }
    impl Minter for FakeMinter {
        fn mint(&self) -> Pin<Box<dyn Future<Output = Result<Integrity, String>> + Send>> {
            self.mints.fetch_add(1, Ordering::SeqCst);
            let r = if *lock(&self.ok) { Ok(self.token()) } else { Err("no mint".to_string()) };
            Box::pin(async move { r })
        }
        fn cached(&self) -> Option<Integrity> {
            lock(&self.cached).clone()
        }
        fn invalidate(&self) {
            *lock(&self.cached) = None;
        }
        fn last_success_ms(&self) -> Option<i64> {
            None
        }
    }

    #[derive(Clone)]
    struct FMsg {
        id: String,
        from: String,
        text: String,
        at: i64,
    }
    struct FThread {
        other: String,
        login: String,
        /// Newest first.
        msgs: Vec<FMsg>,
        archived: bool,
        read: Option<(String, i64)>,
    }
    /// Twitch, as far as these tests need it: threads newest first, cursors
    /// "t:<offset>:<device>" and "m:<thread>:<offset>:<device>", integrity
    /// demanded for any cursor, and cursors bound to the device they were
    /// made under.
    struct FakeTwitch {
        viewer: Mutex<String>,
        threads: Mutex<Vec<FThread>>,
        calls: Mutex<Vec<(String, Option<String>, bool)>>,
        inject: Mutex<Vec<(String, GqlError)>>,
        refuse_cursor: Mutex<Option<String>>,
        /// While set, a history read takes its answer, then waits here before
        /// handing it back (an answer already on its way, Twitch moving on).
        hold: Mutex<Option<Arc<tokio::sync::Notify>>>,
        held: AtomicU32,
    }
    fn iso(ms: i64) -> String {
        use chrono::TimeZone;
        chrono::Utc.timestamp_millis_opt(ms).single().unwrap().to_rfc3339()
    }
    impl FakeTwitch {
        fn new(viewer: &str) -> Arc<Self> {
            Arc::new(FakeTwitch {
                viewer: Mutex::new(viewer.into()),
                threads: Mutex::new(Vec::new()),
                calls: Mutex::new(Vec::new()),
                inject: Mutex::new(Vec::new()),
                refuse_cursor: Mutex::new(None),
                hold: Mutex::new(None),
                held: AtomicU32::new(0),
            })
        }
        fn add_thread(&self, other: &str, n: usize, start_ms: i64) {
            let me = lock(&self.viewer).clone();
            let msgs = (0..n)
                .rev()
                .map(|i| FMsg {
                    id: format!("{other}-m{i}"),
                    from: if i % 2 == 0 { other.to_string() } else { me.clone() },
                    text: format!("words {i}"),
                    at: start_ms + i as i64 * 60_000,
                })
                .collect();
            lock(&self.threads).push(FThread { other: other.into(), login: format!("pal{other}"), msgs, archived: false, read: None });
        }
        fn count(&self, op: &str) -> usize {
            lock(&self.calls).iter().filter(|c| c.0 == op).count()
        }
        fn tid(&self, other: &str) -> String {
            whisper_gql::thread_id(&lock(&self.viewer), other)
        }
        fn msg_edges(&self, t: &FThread, from: usize, first: usize, device: &str) -> Value {
            let slice: Vec<&FMsg> = t.msgs.iter().skip(from).take(first).collect();
            let more = from + first < t.msgs.len();
            let edges: Vec<Value> = slice
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    let cursor = if i + 1 == slice.len() { format!("m:{}:{}:{}", t.other, from + first, device) } else { String::new() };
                    json!({ "cursor": cursor, "node": { "id": m.id, "sentAt": iso(m.at), "editedAt": null, "deletedAt": null,
                        "from": { "id": m.from }, "content": { "content": m.text, "emotes": [] } } })
                })
                .collect();
            json!({ "pageInfo": { "hasNextPage": more }, "edges": edges })
        }
        fn node(&self, t: &FThread, messages: Value) -> Value {
            let me = lock(&self.viewer).clone();
            json!({ "id": self.tid(&t.other), "unreadMessagesCount": 0, "isArchived": t.archived, "isMuted": false,
                "userLastMessageRead": t.read.as_ref().map(|(id, at)| json!({ "id": id, "sentAt": iso(*at) })),
                "participants": [ { "id": me, "login": "me_login", "displayName": "Me", "chatColor": null, "profileImageURL": null, "displayBadges": [] },
                                  { "id": t.other, "login": t.login, "displayName": t.login, "chatColor": "#00FF00", "profileImageURL": null, "displayBadges": [] } ],
                "messages": messages })
        }
        fn answer(&self, body: &Value, integ: Option<&Integrity>) -> Result<Value, GqlError> {
            let op = body["operationName"].as_str().unwrap().to_string();
            let vars = &body["variables"];
            let cursor = vars.get("cursor").and_then(Value::as_str).map(str::to_string);
            lock(&self.calls).push((op.clone(), cursor.clone(), integ.is_some()));
            let injected = {
                let mut inj = lock(&self.inject);
                inj.iter().position(|(o, _)| *o == op).map(|i| inj.remove(i).1)
            };
            if let Some(e) = injected {
                return Err(e);
            }
            let viewer = lock(&self.viewer).clone();
            let integrity_err = || Ok(json!({ "errors": [ { "message": "failed integrity check", "extensions": { "code": "IntegrityCheckFailed" } } ], "data": null }));
            let device = integ.map(|i| i.device_id.clone()).unwrap_or_else(|| "none".into());
            if let Some(c) = &cursor {
                if integ.is_none() {
                    return integrity_err();
                }
                if lock(&self.refuse_cursor).as_deref() == Some(c.as_str()) {
                    return Ok(json!({ "errors": [ { "message": "invalid cursor" } ], "data": null }));
                }
                if !c.ends_with(&format!(":{device}")) {
                    return Ok(json!({ "errors": [ { "message": "invalid cursor" } ], "data": null }));
                }
            }
            match op.as_str() {
                "SnWhisperWho" => Ok(json!({ "data": { "currentUser": { "id": viewer } } })),
                "SnWhisperThreads" => {
                    let first = vars["first"].as_u64().unwrap() as usize;
                    let nested = vars["messages"].as_u64().unwrap() as usize;
                    let from = cursor.as_deref().map(|c| c.split(':').nth(1).unwrap().parse::<usize>().unwrap()).unwrap_or(0);
                    let threads = lock(&self.threads);
                    let slice: Vec<&FThread> = threads.iter().skip(from).take(first).collect();
                    let more = from + first < threads.len();
                    let edges: Vec<Value> = slice
                        .iter()
                        .enumerate()
                        .map(|(i, t)| {
                            let cursor = if i + 1 == slice.len() { format!("t:{}:{}", from + first, device) } else { String::new() };
                            json!({ "cursor": cursor, "node": self.node(t, self.msg_edges(t, 0, nested, "none")) })
                        })
                        .collect();
                    Ok(json!({ "data": { "currentUser": { "id": viewer, "whisperThreads": { "pageInfo": { "hasNextPage": more }, "edges": edges } } } }))
                }
                "SnWhisperMessages" => {
                    let id = vars["id"].as_str().unwrap();
                    let first = vars["first"].as_u64().unwrap() as usize;
                    let from = cursor.as_deref().map(|c| c.split(':').nth(2).unwrap().parse::<usize>().unwrap()).unwrap_or(0);
                    let threads = lock(&self.threads);
                    let Some(t) = threads.iter().find(|t| self.tid(&t.other) == id) else {
                        return Ok(json!({ "data": { "currentUser": { "id": viewer }, "whisperThread": null } }));
                    };
                    Ok(json!({ "data": { "currentUser": { "id": viewer }, "whisperThread": self.node(t, self.msg_edges(t, from, first, &device)) } }))
                }
                "SnWhisperMarkRead" | "SnWhisperArchive" => {
                    let input = &vars["input"];
                    let tid = input["threadID"].as_str().unwrap().to_string();
                    let mut threads = lock(&self.threads);
                    let me = viewer.clone();
                    if let Some(t) = threads.iter_mut().find(|t| whisper_gql::thread_id(&me, &t.other) == tid) {
                        if let Some(read) = input.get("lastReadMessageID").and_then(Value::as_str) {
                            let at = t.msgs.iter().find(|m| m.id == read).map(|m| m.at).unwrap_or(0);
                            t.read = Some((read.to_string(), at));
                        }
                        if input.get("isArchived").is_some() {
                            t.archived = true;
                        }
                    }
                    Ok(json!({ "data": { "updateWhisperThread": { "thread": { "id": tid, "unreadMessagesCount": 0, "isArchived": false } } } }))
                }
                other => panic!("unexpected operation {other}"),
            }
        }
    }
    struct FakeGqlOf(Arc<FakeTwitch>);
    #[async_trait::async_trait]
    impl Gql for FakeGqlOf {
        async fn post(&self, body: Value, integrity: Option<&Integrity>) -> Result<Value, GqlError> {
            let answer = self.0.answer(&body, integrity);
            let gate = if body["operationName"] == "SnWhisperMessages" { lock(&self.0.hold).clone() } else { None };
            if let Some(gate) = gate {
                self.0.held.fetch_add(1, Ordering::SeqCst);
                gate.notified().await;
            }
            answer
        }
    }
    struct FakeSession {
        twitch: Arc<FakeTwitch>,
        token: Mutex<Option<String>>,
        reharvests: AtomicU32,
    }
    impl WebSession for FakeSession {
        fn token(&self) -> BoxFut<Option<String>> {
            let t = lock(&self.token).clone();
            Box::pin(async move { t })
        }
        fn reharvest(&self) -> BoxFut<()> {
            self.reharvests.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {})
        }
        fn gql(&self, _token: &str) -> Arc<dyn Gql> {
            Arc::new(FakeGqlOf(Arc::clone(&self.twitch)))
        }
    }
    #[derive(Default)]
    struct FakeEvents {
        log: Mutex<Vec<String>>,
    }
    impl Events for FakeEvents {
        fn archive_changed(&self, owner: &str, syncing: bool) {
            lock(&self.log).push(format!("archive {owner} {syncing}"));
        }
        fn conversation(&self, _scope: &Scope, key: &str) {
            lock(&self.log).push(format!("conversation {key}"));
        }
        fn counts(&self) {
            lock(&self.log).push("counts".into());
        }
        fn claims_scope(&self, generation: u64) {
            lock(&self.log).push(format!("claims {generation}"));
        }
        fn recheck_sends(&self) {
            lock(&self.log).push("recheck".into());
        }
    }

    struct Rig {
        engine: Arc<Engine>,
        twitch: Arc<FakeTwitch>,
        clock: Arc<FakeClock>,
        sleeper: Arc<FakeSleeper>,
        minter: Arc<FakeMinter>,
        session: Arc<FakeSession>,
        events: Arc<FakeEvents>,
        spawned: Arc<Mutex<Vec<BoxFut<()>>>>,
    }

    static ROOT: once_cell::sync::Lazy<std::path::PathBuf> = once_cell::sync::Lazy::new(|| {
        let dir = std::env::temp_dir().join(format!("sn-whisper-sync-tests-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        whisper_store::set_root_for_tests(dir.clone());
        dir
    });

    /// Day boundaries in local time make the scraped tests depend on the
    /// zone; a fixed noon start keeps every message on its intended day.
    const T0: i64 = 1_900_000_000_000;

    fn rig(owner: &str) -> Rig {
        once_cell::sync::Lazy::force(&ROOT);
        let twitch = FakeTwitch::new(owner);
        let clock = Arc::new(FakeClock(AtomicI64::new(T0 + 400 * 86_400_000)));
        let sleeper = Arc::new(FakeSleeper { clock: Arc::clone(&clock), slept: Mutex::new(Vec::new()) });
        let minter = Arc::new(FakeMinter { ok: Mutex::new(true), device: "dev-a".into(), mints: AtomicU32::new(0), cached: Mutex::new(None) });
        let session = Arc::new(FakeSession { twitch: Arc::clone(&twitch), token: Mutex::new(Some(format!("token-{owner}"))), reharvests: AtomicU32::new(0) });
        let events = Arc::new(FakeEvents::default());
        let spawned: Arc<Mutex<Vec<BoxFut<()>>>> = Arc::new(Mutex::new(Vec::new()));
        let held = Arc::clone(&spawned);
        let engine = Arc::new(Engine::new(Env {
            session: session.clone(),
            clock: clock.clone(),
            sleeper: sleeper.clone(),
            minter: minter.clone(),
            events: events.clone(),
            spawn: Arc::new(move |f| lock(&held).push(f)),
        }));
        Rig { engine, twitch, clock, sleeper, minter, session, events, spawned }
    }

    impl Rig {
        /// Follow `owner` without running the sync the follow asks for.
        fn follow(&self, owner: &str) -> Scope {
            let _ = self.engine.follow(Some(owner.into()));
            lock(&self.spawned).clear();
            let mut c = lock(&self.engine.ctrl);
            c.running = false;
            c.run_again = None;
            drop(c);
            self.engine.scope().unwrap()
        }
        async fn run(&self, scope: &Scope, reason: Reason) -> SyncOutcome {
            self.engine.run_once(scope, &reason).await
        }
        fn messages(&self, scope: &Scope, key: &str) -> Vec<StoredWhisper> {
            whisper_store::read(scope, key, |c| c.messages.clone()).unwrap().unwrap_or_default()
        }
        fn meta(&self, scope: &Scope, key: &str) -> ConversationMeta {
            whisper_store::read_index(scope, |idx| idx.conversations.iter().find(|m| m.key == key).cloned()).unwrap().unwrap()
        }
        fn spawned(&self) -> usize {
            lock(&self.spawned).len()
        }
    }

    fn local(id: &str, other: &str, me: &str, at: i64, origin: Origin, mine: bool) -> StoredWhisper {
        StoredWhisper {
            id: id.into(),
            from_user_id: if mine { me.into() } else { other.into() },
            from_user_login: String::new(),
            from_user_name: String::new(),
            to_user_id: if mine { other.into() } else { me.into() },
            to_user_login: String::new(),
            to_user_name: String::new(),
            message: "old words".into(),
            timestamp: at,
            is_sent: mine,
            parts: Vec::new(),
            deleted: false,
            edited_ms: None,
            origin,
            delivery: None,
            sent_local_ms: None,
            got_204: false,
            checks: 0,
            failure: None,
            cut_to: None,
            migrated: false,
        }
    }

    fn seed(scope: &Scope, other: &str, messages: Vec<StoredWhisper>, tweak: impl FnOnce(&mut ConversationMeta)) {
        let thread = GqlThread {
            thread_id: whisper_gql::thread_id(&scope.owner, other),
            other: None,
            me: None,
            unread: 0,
            twitch_read: None,
            archived: false,
            muted: false,
            messages: whisper_gql::MessagesPage { viewer_id: scope.owner.clone(), messages: Vec::new(), has_next: false, next_cursor: None },
        };
        let mut meta = fresh_meta(other, &thread);
        tweak(&mut meta);
        whisper_store::update(scope, other, Some(meta), |conv| {
            conv.messages = messages;
            ((), true)
        })
        .unwrap();
    }

    // ---- Account follow -------------------------------------------------------------

    #[tokio::test]
    async fn same_id_follow_keeps_the_run() {
        let r = rig("99999901");
        let first = r.follow("99999901");
        assert!(r.engine.follow(Some("99999901".into())).is_none());
        assert_eq!(r.engine.scope(), Some(first));
        assert_eq!(r.spawned(), 0, "a same-id follow starts nothing");
    }

    #[tokio::test]
    async fn stale_generation_stops() {
        let r = rig("99999902");
        r.twitch.add_thread("1000000000", 3, T0);
        let old = r.follow("99999902");
        let _ = r.engine.follow(Some("99999903".into()));
        assert_eq!(r.run(&old, Reason::Manual).await, SyncOutcome::Superseded);
        assert!(whisper_store::update(&old, "1000000000", None, |_| ((), true)).is_err(), "nothing more is written for the old account");
    }

    #[tokio::test]
    async fn sign_out_and_back_in_same_account_writes() {
        let r = rig("99999904");
        r.twitch.add_thread("1000000000", 3, T0);
        let _ = r.follow("99999904");
        let closing = r.engine.follow(None).unwrap();
        crate::services::whisper_send::teardown(&closing).await;
        whisper_store::flush_closing(&closing).unwrap();
        let _ = whisper_store::drop_owner(closing);
        let again = r.follow("99999904");
        assert!(matches!(r.run(&again, Reason::Manual).await, SyncOutcome::Done { .. }));
        assert_eq!(r.messages(&again, "1000000000").len(), 3);
    }

    #[tokio::test]
    async fn teardown_writes_and_flushes_after_close() {
        let r = rig("99999905");
        let scope = r.follow("99999905");
        seed(&scope, "1000000000", vec![local("x1", "1000000000", "99999905", T0, Origin::Twitch, false)], |_| {});
        let closing = r.engine.follow(None).unwrap();
        assert!(whisper_store::update(&scope, "1000000000", None, |_| ((), true)).is_err(), "closed scope refuses updates");
        crate::services::whisper_send::teardown(&closing).await;
        whisper_store::flush_closing(&closing).unwrap();
        assert!(whisper_store::drop_owner(closing).is_ok());
        let back = r.follow("99999905");
        assert_eq!(r.messages(&back, "1000000000").len(), 1, "the flushed conversation reloads from disk");
    }

    #[tokio::test]
    async fn sync_refuses_other_accounts_session() {
        let r = rig("99999906");
        r.twitch.add_thread("1000000000", 3, T0);
        *lock(&r.twitch.viewer) = "99999999".into();
        let scope = r.follow("99999906");
        assert_eq!(r.run(&scope, Reason::Manual).await, SyncOutcome::WrongSession);
        assert_eq!(r.session.reharvests.load(Ordering::SeqCst), 1);
        assert_eq!(r.twitch.count("SnWhisperThreads"), 0);
        assert_eq!(r.twitch.count("SnWhisperMarkRead"), 0);
        assert_eq!(whisper_store::read_index(&scope, |idx| idx.conversations.len()).unwrap(), 0);
        assert_eq!(lock(&r.engine.problem).as_deref(), Some(SESSION_PROBLEM));
    }

    // ---- Import and incremental -------------------------------------------------------

    #[tokio::test]
    async fn imported_at_after_page_one() {
        let r = rig("99999907");
        for i in 0..150 {
            r.twitch.add_thread(&format!("{}", 1000000000u64 + i), 1, T0);
        }
        *lock(&r.minter.ok) = false;
        let scope = r.follow("99999907");
        assert!(matches!(r.run(&scope, Reason::Manual).await, SyncOutcome::Partial { .. }));
        let (imported, deep, n) = whisper_store::read_index(&scope, |idx| (idx.imported_at, idx.list_deep.clone(), idx.conversations.len())).unwrap();
        assert!(imported.is_some());
        assert!(matches!(deep, Deep::Partial { .. }));
        assert_eq!(n, 100);
    }

    #[tokio::test]
    async fn error_is_not_an_empty_page() {
        let r = rig("99999908");
        r.twitch.add_thread("1000000000", 3, T0);
        lock(&r.twitch.inject).push(("SnWhisperThreads".into(), GqlError::Other("service error".into())));
        let scope = r.follow("99999908");
        assert!(matches!(r.run(&scope, Reason::Manual).await, SyncOutcome::Failed { .. }));
        let (imported, n) = whisper_store::read_index(&scope, |idx| (idx.imported_at, idx.conversations.len())).unwrap();
        assert!(imported.is_none());
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn sync_after_live_receive_adds_nothing() {
        let r = rig("99999909");
        r.twitch.add_thread("1000000000", 1, T0);
        let scope = r.follow("99999909");
        seed(&scope, "1000000000", vec![local("1000000000-m0", "1000000000", "99999909", T0 + 5, Origin::Twitch, false)], |_| {});
        r.run(&scope, Reason::Manual).await;
        let msgs = r.messages(&scope, "1000000000");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].message, "words 0", "Twitch's content replaces the live copy");
        assert_eq!(msgs[0].timestamp, T0, "and Twitch's time");
    }

    #[tokio::test]
    async fn refresh_updates_parts_edits_deletes_on_known_id() {
        let r = rig("99999910");
        let scope = r.follow("99999910");
        let mut held = local("k1", "1000000000", "99999910", T0, Origin::Twitch, false);
        held.message = "before".into();
        let mut target = held.clone();
        target.message = "hi Kappa".into();
        target.parts = whisper_gql::emote_parts("hi Kappa", &[("25".into(), 3, 7)]);
        target.edited_ms = Some(T0 + 1000);
        target.deleted = true;
        assert!(take_twitch(&mut held, &target));
        assert_eq!(held.message, "hi Kappa");
        assert_eq!(held.parts.len(), 2);
        assert_eq!(held.edited_ms, Some(T0 + 1000));
        assert!(held.deleted);
        assert!(!take_twitch(&mut held, &target), "a second identical copy changes nothing");
        drop(scope);
    }

    #[tokio::test]
    async fn incremental_stops_at_known() {
        let r = rig("99999911");
        r.twitch.add_thread("1000000000", 5, T0);
        let scope = r.follow("99999911");
        r.run(&scope, Reason::Manual).await;
        r.twitch.add_thread("1000000001", 1, T0);
        let before = r.twitch.count("SnWhisperThreads");
        r.run(&scope, Reason::Manual).await;
        assert_eq!(r.twitch.count("SnWhisperThreads") - before, 1, "one list page, nothing more");
        assert_eq!(r.twitch.count("SnWhisperMessages"), 0);
    }

    #[tokio::test]
    async fn incremental_list_stops_at_unchanged_page() {
        let r = rig("99999912");
        for i in 0..150 {
            r.twitch.add_thread(&format!("{}", 1000000000u64 + i), 1, T0);
        }
        let scope = r.follow("99999912");
        r.run(&scope, Reason::Manual).await;
        assert_eq!(whisper_store::read_index(&scope, |idx| idx.list_deep.clone()).unwrap(), Deep::Complete);
        let before = r.twitch.count("SnWhisperThreads");
        r.run(&scope, Reason::Manual).await;
        assert_eq!(r.twitch.count("SnWhisperThreads") - before, 1, "an unchanged first page ends the list read");
    }

    #[tokio::test]
    async fn gap_closes_on_known_id() {
        let r = rig("99999913");
        r.twitch.add_thread("1000000000", 5, T0);
        let scope = r.follow("99999913");
        r.run(&scope, Reason::Manual).await;
        {
            let mut threads = lock(&r.twitch.threads);
            let t = &mut threads[0];
            let newer: Vec<FMsg> = (0..30)
                .rev()
                .map(|i| FMsg { id: format!("new{i}"), from: "1000000000".into(), text: format!("new {i}"), at: T0 + 86_400_000 + i * 1000 })
                .collect();
            let mut all = newer;
            all.extend(t.msgs.clone());
            t.msgs = all;
        }
        r.run(&scope, Reason::Manual).await;
        assert!(r.meta(&scope, "1000000000").gap.is_none(), "the gap closed when paging met a known message");
        assert_eq!(r.messages(&scope, "1000000000").len(), 35);
    }

    // ---- Deep paging, cursors and mints ----------------------------------------------------

    #[tokio::test]
    async fn mint_failure_leaves_partial() {
        let r = rig("99999914");
        r.twitch.add_thread("1000000000", 150, T0);
        *lock(&r.minter.ok) = false;
        let scope = r.follow("99999914");
        assert!(matches!(r.run(&scope, Reason::Manual).await, SyncOutcome::Partial { .. }));
        assert_eq!(r.messages(&scope, "1000000000").len(), 100, "page 1 is kept");
        assert!(matches!(r.meta(&scope, "1000000000").deep, Deep::Partial { .. }));
    }

    #[tokio::test]
    async fn one_mint_per_run() {
        let r = rig("99999915");
        for i in 0..5 {
            r.twitch.add_thread(&format!("{}", 1000000000u64 + i), 150, T0);
        }
        *lock(&r.minter.ok) = false;
        let scope = r.follow("99999915");
        r.run(&scope, Reason::Manual).await;
        assert_eq!(r.minter.mints.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn full_depth_with_a_token() {
        let r = rig("99999916");
        r.twitch.add_thread("1000000000", 250, T0);
        let scope = r.follow("99999916");
        assert!(matches!(r.run(&scope, Reason::Manual).await, SyncOutcome::Done { .. }));
        assert_eq!(r.messages(&scope, "1000000000").len(), 250);
        assert_eq!(r.meta(&scope, "1000000000").deep, Deep::Complete);
    }

    #[tokio::test]
    async fn partial_resumes_from_cursor() {
        let r = rig("99999917");
        r.twitch.add_thread("1000000000", 250, T0);
        let scope = r.follow("99999917");
        r.run(&scope, Reason::Manual).await;
        // Leave the conversation as an interrupted import would: the newest
        // hundred held, and where to go on from.
        whisper_store::update(&scope, "1000000000", None, |conv| {
            conv.messages.retain(|m| m.timestamp >= T0 + 150 * 60_000);
            conv.meta.deep = Deep::Partial { cursor: Some(Cursor { cursor: "m:1000000000:100:dev-a".into(), device_id: Some("dev-a".into()) }) };
            ((), true)
        })
        .unwrap();
        let before = lock(&r.twitch.calls).len();
        r.run(&scope, Reason::ResumePartial).await;
        let calls = lock(&r.twitch.calls);
        let first_deep = calls[before..].iter().find(|c| c.0 == "SnWhisperMessages").unwrap();
        assert_eq!(first_deep.1.as_deref(), Some("m:1000000000:100:dev-a"));
    }

    #[tokio::test]
    async fn cursor_from_other_device_restarts() {
        let r = rig("99999918");
        r.twitch.add_thread("1000000000", 150, T0);
        let scope = r.follow("99999918");
        seed(&scope, "1000000000", Vec::new(), |m| {
            m.deep = Deep::Partial { cursor: Some(Cursor { cursor: "m:1000000000:100:dev-old".into(), device_id: Some("dev-old".into()) }) };
        });
        r.run(&scope, Reason::ResumePartial).await;
        let calls = lock(&r.twitch.calls);
        let first_deep = calls.iter().find(|c| c.0 == "SnWhisperMessages").unwrap();
        assert_eq!(first_deep.1, None, "a cursor from another device is never sent");
        drop(calls);
        assert_eq!(r.meta(&scope, "1000000000").deep, Deep::Complete);
    }

    #[tokio::test]
    async fn stale_cursor_restarts_once() {
        let r = rig("99999919");
        r.twitch.add_thread("1000000000", 150, T0);
        let scope = r.follow("99999919");
        let bad = "m:1000000000:100:dev-a".to_string();
        *lock(&r.twitch.refuse_cursor) = Some(bad.clone());
        seed(&scope, "1000000000", Vec::new(), |m| {
            m.deep = Deep::Partial { cursor: Some(Cursor { cursor: bad.clone(), device_id: Some("dev-a".into()) }) };
        });
        r.run(&scope, Reason::ResumePartial).await;
        let restarts = lock(&r.twitch.calls).iter().filter(|c| c.0 == "SnWhisperMessages" && c.1.is_none()).count();
        assert_eq!(restarts, 1);
        whisper_store::update(&scope, "1000000000", None, |conv| {
            conv.meta.deep = Deep::Partial { cursor: Some(Cursor { cursor: bad.clone(), device_id: Some("dev-a".into()) }) };
            ((), true)
        })
        .unwrap();
        r.run(&scope, Reason::ResumePartial).await;
        let restarts = lock(&r.twitch.calls).iter().filter(|c| c.0 == "SnWhisperMessages" && c.1.is_none()).count();
        assert_eq!(restarts, 1, "a second refusal in the same process does not restart again");
    }

    #[test]
    fn mint_backoff_steps() {
        let mut g = MintGate::default();
        assert!(g.allowed(0, None));
        g.failed(0);
        assert!(!g.allowed(119_999, None));
        assert!(g.allowed(120_000, None));
        g.failed(120_000);
        assert!(!g.allowed(120_000 + 599_999, None));
        assert!(g.allowed(120_000 + 600_000, None));
        g.failed(720_000);
        assert!(!g.allowed(720_000 + 3_599_999, None));
        g.failed(4_320_000);
        assert!(!g.allowed(4_320_000 + 3_599_999, None), "the step stays at an hour");
        assert!(g.allowed(4_320_001, Some(4_320_001)), "a newer mint by any caller resets the gate");
    }

    #[tokio::test]
    async fn pacing_backs_off_from_refusal() {
        let r = rig("99999920");
        r.twitch.add_thread("1000000000", 1, T0);
        let scope = r.follow("99999920");
        lock(&r.twitch.inject).push(("SnWhisperThreads".into(), GqlError::RateLimited));
        let refused_at = r.clock.now_ms();
        r.run(&scope, Reason::Manual).await;
        assert!(r.clock.now_ms() >= refused_at + RATE_LIMIT_QUIET_MS);
        assert!(lock(&r.sleeper.slept).iter().any(|ms| *ms as i64 >= RATE_LIMIT_QUIET_MS - REQUEST_SPACING_MS));
        assert_eq!(r.twitch.count("SnWhisperThreads"), 2);
    }

    // ---- Read state and archive ------------------------------------------------------------

    #[tokio::test]
    async fn owed_mark_read_sends_the_read_id() {
        let r = rig("99999921");
        r.twitch.add_thread("1000000000", 3, T0);
        let scope = r.follow("99999921");
        r.run(&scope, Reason::Manual).await;
        whisper_store::update(&scope, "1000000000", None, |conv| {
            conv.meta.twitch_read_owed = Some("1000000000-m0".into());
            ((), true)
        })
        .unwrap();
        let before = lock(&r.twitch.calls).len();
        r.run(&scope, Reason::Manual).await;
        let calls = lock(&r.twitch.calls);
        let after: Vec<&String> = calls[before..].iter().map(|c| &c.0).collect();
        assert_eq!(after.first().map(|s| s.as_str()), Some("SnWhisperMarkRead"), "owed writes go before reading Twitch's state");
        drop(calls);
        let t = lock(&r.twitch.threads);
        assert_eq!(t[0].read.as_ref().unwrap().0, "1000000000-m0");
        drop(t);
        assert!(r.meta(&scope, "1000000000").twitch_read_owed.is_none());
    }

    #[tokio::test]
    async fn mark_read_never_goes_backwards() {
        let r = rig("99999922");
        r.twitch.add_thread("1000000000", 3, T0);
        let scope = r.follow("99999922");
        r.run(&scope, Reason::Manual).await;
        whisper_store::update(&scope, "1000000000", None, |conv| {
            conv.meta.twitch_read = Some(Marker { id: "1000000000-m2".into(), at_ms: T0 + 120_000 });
            conv.meta.read_seq = whisper_store::next_seq();
            conv.meta.twitch_read_owed = Some("1000000000-m0".into());
            ((), true)
        })
        .unwrap();
        r.run(&scope, Reason::ResumePartial).await;
        assert_eq!(r.twitch.count("SnWhisperMarkRead"), 0);
        assert!(r.meta(&scope, "1000000000").twitch_read_owed.is_none());
    }

    #[tokio::test]
    async fn mark_read_retries_with_integrity() {
        let r = rig("99999923");
        r.twitch.add_thread("1000000000", 3, T0);
        let scope = r.follow("99999923");
        r.run(&scope, Reason::Manual).await;
        whisper_store::update(&scope, "1000000000", None, |conv| {
            conv.meta.twitch_read_owed = Some("1000000000-m2".into());
            ((), true)
        })
        .unwrap();
        lock(&r.twitch.inject).push(("SnWhisperMarkRead".into(), GqlError::Integrity));
        r.run(&scope, Reason::ResumePartial).await;
        let calls = lock(&r.twitch.calls);
        let marks: Vec<bool> = calls.iter().filter(|c| c.0 == "SnWhisperMarkRead").map(|c| c.2).collect();
        assert_eq!(marks, vec![false, true]);
    }

    /// An archive still owed to Twitch is dropped once a
    /// whisper from them arrived after it was asked for; with nothing newer it
    /// goes out.
    #[tokio::test]
    async fn new_whisper_cancels_owed_archive() {
        let r = rig("99999929");
        r.twitch.add_thread("1000000000", 3, T0); // theirs at T0 and T0+120 s, mine at T0+60 s
        let scope = r.follow("99999929");
        r.run(&scope, Reason::Manual).await;
        whisper_store::update(&scope, "1000000000", None, |conv| {
            conv.meta.archive_pending = true;
            conv.meta.archive_requested_ms = Some(T0 + 30_000);
            ((), true)
        })
        .unwrap();
        r.run(&scope, Reason::ResumePartial).await;
        assert_eq!(r.twitch.count("SnWhisperArchive"), 0, "a newer whisper from them cancels the owed archive");
        let meta = r.meta(&scope, "1000000000");
        assert!(!meta.archive_pending);
        assert!(!meta.archived);

        whisper_store::update(&scope, "1000000000", None, |conv| {
            conv.meta.archive_pending = true;
            conv.meta.archive_requested_ms = Some(T0 + 200_000);
            ((), true)
        })
        .unwrap();
        r.run(&scope, Reason::ResumePartial).await;
        assert_eq!(r.twitch.count("SnWhisperArchive"), 1, "with nothing newer the owed archive goes out");
        assert!(r.meta(&scope, "1000000000").archived);
    }

    #[test]
    fn racing_page_keeps_landed_write() {
        let thread = GqlThread {
            thread_id: "99999924_1000000000".into(),
            other: None,
            me: None,
            unread: 0,
            twitch_read: Some(Marker { id: "old".into(), at_ms: 1 }),
            archived: false,
            muted: false,
            messages: whisper_gql::MessagesPage { viewer_id: "99999924".into(), messages: Vec::new(), has_next: false, next_cursor: None },
        };
        let mut meta = fresh_meta("1000000000", &thread);
        let page_seq = whisper_store::next_seq();
        meta.twitch_read = Some(Marker { id: "new".into(), at_ms: 2 });
        meta.read_seq = whisper_store::next_seq();
        merge_twitch_meta(&mut meta, &thread, page_seq);
        assert_eq!(meta.twitch_read.as_ref().unwrap().id, "new");
    }

    #[test]
    fn stale_page_never_rearchives_new_whisper() {
        let thread = GqlThread {
            thread_id: "99999925_1000000000".into(),
            other: None,
            me: None,
            unread: 0,
            twitch_read: None,
            archived: true,
            muted: false,
            messages: whisper_gql::MessagesPage { viewer_id: "99999925".into(), messages: Vec::new(), has_next: false, next_cursor: None },
        };
        let mut meta = fresh_meta("1000000000", &thread);
        meta.archived = false;
        let page_seq = whisper_store::next_seq();
        meta.archived_seq = whisper_store::next_seq();
        merge_twitch_meta(&mut meta, &thread, page_seq);
        assert!(!meta.archived, "the unarchive on receive landed after the page's request started");
        let later = whisper_store::next_seq();
        merge_twitch_meta(&mut meta, &thread, later);
        assert!(meta.archived, "a page started after it is Twitch's word");
    }

    // ---- Scraped copies --------------------------------------------------------------------

    #[test]
    fn scraped_dropped_from_oldest_fetched_day() {
        let day = 86_400_000;
        let mut meta = fresh_meta("1000000000", &GqlThread {
            thread_id: "1_2".into(), other: None, me: None, unread: 0, twitch_read: None, archived: false, muted: false,
            messages: whisper_gql::MessagesPage { viewer_id: "1".into(), messages: Vec::new(), has_next: false, next_cursor: None },
        });
        let mut msgs = vec![
            local("s-old", "2", "1", T0 - 2 * day, Origin::Scraped, false),
            local("s-same", "2", "1", T0, Origin::Scraped, false),
            local("t1", "2", "1", T0 + 3_600_000, Origin::Twitch, false),
        ];
        assert!(drop_scraped(&mut meta, &mut msgs));
        let ids: Vec<&str> = msgs.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["s-old", "t1"]);
    }

    #[test]
    fn scraped_marker_never_reads_same_day_unread() {
        let day = 86_400_000;
        let mut meta = fresh_meta("1000000000", &GqlThread {
            thread_id: "1_2".into(), other: None, me: None, unread: 0, twitch_read: None, archived: false, muted: false,
            messages: whisper_gql::MessagesPage { viewer_id: "1".into(), messages: Vec::new(), has_next: false, next_cursor: None },
        });
        meta.read_through = Some(Marker { id: "s-day".into(), at_ms: T0 });
        let mut msgs = vec![
            local("t-before", "2", "1", T0 - day, Origin::Twitch, false),
            local("s-day", "2", "1", T0, Origin::Scraped, false),
            local("t-same-day", "2", "1", T0 + 3_600_000, Origin::Twitch, false),
        ];
        drop_scraped(&mut meta, &mut msgs);
        assert_eq!(meta.read_through.as_ref().unwrap().id, "t-before", "the marker moves before the scraped day, so the same-day whisper stays unread");
    }

    // ---- Triggers --------------------------------------------------------------------------

    #[tokio::test]
    async fn resubscribe_after_gap_syncs() {
        let r = rig("99999926");
        let _ = r.follow("99999926");
        r.engine.note_socket_up(true);
        assert_eq!(r.spawned(), 0, "the first subscribe of a process is not a gap");
        r.engine.note_socket_down();
        r.engine.note_socket_up(false);
        assert_eq!(r.spawned(), 1);
    }

    #[tokio::test]
    async fn inbox_opened_once_a_minute() {
        let r = rig("99999927");
        let _ = r.follow("99999927");
        r.engine.trigger(Reason::InboxOpened);
        lock(&r.engine.ctrl).running = false;
        r.engine.trigger(Reason::InboxOpened);
        assert_eq!(r.spawned(), 1);
        r.clock.0.fetch_add(INBOX_OPEN_EVERY_MS, Ordering::SeqCst);
        lock(&r.engine.ctrl).running = false;
        r.engine.trigger(Reason::InboxOpened);
        assert_eq!(r.spawned(), 2);
    }

    #[tokio::test]
    async fn resume_after_ten_minutes() {
        let r = rig("99999928");
        let _ = r.follow("99999928");
        r.engine.note_away(true);
        r.clock.0.fetch_add(AWAY_FOR_SYNC_MS - 1, Ordering::SeqCst);
        r.engine.note_away(false);
        assert_eq!(r.spawned(), 0);
        r.engine.note_away(true);
        r.clock.0.fetch_add(AWAY_FOR_SYNC_MS, Ordering::SeqCst);
        r.engine.note_away(false);
        assert_eq!(r.spawned(), 1);
    }

    #[tokio::test]
    async fn single_flight_runs_again() {
        let r = rig("99999939");
        r.twitch.add_thread("1000000000", 1, T0);
        let _ = r.follow("99999939");
        assert_eq!(r.engine.trigger(Reason::Manual), 1);
        assert_eq!(r.engine.trigger(Reason::Manual), 2, "a trigger during a run waits for one more");
        assert_eq!(r.spawned(), 1);
        let job = lock(&r.spawned).pop().unwrap();
        job.await;
        assert_eq!(lock(&r.engine.ctrl).completed, 2);
        assert!(!lock(&r.engine.ctrl).running);
    }

    #[tokio::test]
    async fn chatty_partner_cannot_starve_refresh() {
        let r = rig("99999930");
        let _ = r.follow("99999930");
        r.engine.trigger(Reason::AfterTraffic("1000000000".into()));
        r.engine.trigger(Reason::AfterTraffic("1000000000".into()));
        r.engine.trigger(Reason::AfterTraffic("1000000000".into()));
        assert_eq!(r.spawned(), 1, "later events in a burst do not re-arm the refresh");
        let job = lock(&r.spawned).pop().unwrap();
        job.await;
        assert_eq!(lock(&r.sleeper.slept).first().copied(), Some(TRAFFIC_DEBOUNCE_MS));
    }

    #[tokio::test]
    async fn open_conversation_reads_through_on_merge() {
        let r = rig("99999931");
        r.twitch.add_thread("1000000000", 3, T0);
        let scope = r.follow("99999931");
        r.run(&scope, Reason::Manual).await;
        crate::services::whisper_claims::set_claim("test-window", "99999931", Some("1000000000".into()), "load-1", 1);
        crate::services::whisper_claims::note_focus("test-window", true);
        lock(&r.twitch.threads)[0].msgs.insert(0, FMsg { id: "fresh".into(), from: "1000000000".into(), text: "new".into(), at: T0 + 86_400_000 });
        r.engine.refresh_thread("1000000000".into()).await.unwrap();
        let meta = r.meta(&scope, "1000000000");
        assert_eq!(meta.read_through.as_ref().map(|m| m.id.as_str()), Some("fresh"));
        crate::services::whisper_claims::clear_label("test-window");
    }

    #[tokio::test]
    async fn load_older_one_page_per_tap() {
        let r = rig("99999932");
        r.twitch.add_thread("1000000000", 350, T0);
        *lock(&r.minter.ok) = false;
        let scope = r.follow("99999932");
        r.run(&scope, Reason::Manual).await;
        *lock(&r.minter.ok) = true;
        // Inside the back-off a tap still mints.
        let added = r.engine.load_older("1000000000".into()).await.unwrap();
        assert!(added > 0);
        assert!(matches!(r.meta(&scope, "1000000000").deep, Deep::Partial { .. }), "one tap does not read the whole history");
    }

    // ---- Settling, gaps, joined refreshes, claims, resume ---------------------------------

    /// A send the last run of the app left Sending is settled before the first
    /// merge, so Twitch's copy of it is adopted, not added beside it.
    #[tokio::test]
    async fn stale_sending_is_settled_then_adopted() {
        let r = rig("99999943");
        r.twitch.add_thread("1000000000", 2, T0); // m1 is ours: "words 1" at T0 + 60 s
        let scope = r.follow("99999943");
        let mut ours = local("sent-left", "1000000000", "99999943", T0 + 60_000 - 400, Origin::LocalSent, true);
        ours.message = "words 1".into();
        ours.delivery = Some(whisper_store::Delivery::Sending);
        ours.sent_local_ms = Some(T0 + 60_000 - 400);
        seed(&scope, "1000000000", vec![ours], |_| {});
        r.run(&scope, Reason::Manual).await;
        let msgs = r.messages(&scope, "1000000000");
        assert_eq!(msgs.len(), 2, "{:?}", msgs.iter().map(|m| m.id.clone()).collect::<Vec<_>>());
        let adopted = msgs.iter().find(|m| m.id == "1000000000-m1").unwrap();
        assert_eq!(adopted.delivery, Some(whisper_store::Delivery::Delivered));
        assert!(msgs.iter().all(|m| m.id != "sent-left"));
    }

    /// 150 whispers arrived while the app was closed: the incremental page
    /// holds 20, the gap pages from the newest down to what was held before,
    /// and none is lost or doubled.
    #[tokio::test]
    async fn a_gap_of_150_is_fetched_whole() {
        let r = rig("99999944");
        r.twitch.add_thread("1000000000", 5, T0);
        let scope = r.follow("99999944");
        r.run(&scope, Reason::Manual).await;
        assert_eq!(r.meta(&scope, "1000000000").deep, Deep::Complete);
        {
            let mut threads = lock(&r.twitch.threads);
            let t = &mut threads[0];
            let newer: Vec<FMsg> = (0..150)
                .rev()
                .map(|i| FMsg { id: format!("late{i}"), from: "1000000000".into(), text: format!("late {i}"), at: T0 + 86_400_000 + i * 1000 })
                .collect();
            let mut all = newer;
            all.extend(t.msgs.clone());
            t.msgs = all;
        }
        r.run(&scope, Reason::Manual).await;
        let msgs = r.messages(&scope, "1000000000");
        let ids: HashSet<String> = msgs.iter().map(|m| m.id.clone()).collect();
        assert_eq!(msgs.len(), 155, "every late whisper is stored");
        assert_eq!(ids.len(), 155, "none twice");
        let meta = r.meta(&scope, "1000000000");
        assert!(meta.gap.is_none() && meta.gap_floor.is_none(), "the gap closed at the floor");
    }

    /// A refresh asked for while another is reading waits for it, then reads
    /// again: the answer it gets is never one that left before it asked.
    #[tokio::test]
    async fn retry_during_refresh_reads_twitch_again() {
        let r = rig("99999945");
        r.twitch.add_thread("1000000000", 2, T0);
        let scope = r.follow("99999945");
        r.run(&scope, Reason::Manual).await;
        let before = r.twitch.count("SnWhisperMessages");
        let gate = Arc::new(tokio::sync::Notify::new());
        *lock(&r.twitch.hold) = Some(Arc::clone(&gate));
        let e1 = Arc::clone(&r.engine);
        let first = tokio::spawn(async move { e1.refresh_thread("1000000000".into()).await });
        while r.twitch.held.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        // Twitch moves on after the first answer left.
        lock(&r.twitch.threads)[0].msgs.insert(0, FMsg { id: "late".into(), from: "99999945".into(), text: "late".into(), at: T0 + 86_400_000 });
        *lock(&r.twitch.hold) = None;
        let e2 = Arc::clone(&r.engine);
        let second = tokio::spawn(async move { e2.refresh_thread("1000000000".into()).await });
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(!second.is_finished(), "the second caller waits for a read of its own");
        gate.notify_one();
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
        assert!(r.messages(&scope, "1000000000").iter().any(|m| m.id == "late"), "the second read saw Twitch as it was when asked");
        assert_eq!(r.twitch.count("SnWhisperMessages") - before, 2);
    }

    /// Several callers waiting on one read share the one fresh read after it.
    #[tokio::test]
    async fn waiters_share_one_fresh_read() {
        let r = rig("99999946");
        r.twitch.add_thread("1000000000", 2, T0);
        let scope = r.follow("99999946");
        r.run(&scope, Reason::Manual).await;
        let before = r.twitch.count("SnWhisperMessages");
        let gate = Arc::new(tokio::sync::Notify::new());
        *lock(&r.twitch.hold) = Some(Arc::clone(&gate));
        let e1 = Arc::clone(&r.engine);
        let first = tokio::spawn(async move { e1.refresh_thread("1000000000".into()).await });
        while r.twitch.held.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        *lock(&r.twitch.hold) = None;
        let waiters: Vec<_> = (0..3)
            .map(|_| {
                let e = Arc::clone(&r.engine);
                tokio::spawn(async move { e.refresh_thread("1000000000".into()).await })
            })
            .collect();
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        gate.notify_one();
        first.await.unwrap().unwrap();
        for w in waiters {
            w.await.unwrap().unwrap();
        }
        assert_eq!(r.twitch.count("SnWhisperMessages") - before, 2, "one read in flight, one fresh read for every waiter");
        drop(scope);
    }

    /// Following another account (or none) leaves only claims made under the
    /// new generation counting.
    #[tokio::test]
    async fn a_new_account_keeps_only_its_own_claims() {
        let r = rig("99999947");
        let a = r.follow("99999947");
        let b = r.follow("99999948");
        assert!(b.generation > a.generation);
        let _ = r.engine.follow(None);
        let log = lock(&r.events.log).clone();
        let seen: Vec<String> = log.iter().filter(|l| l.starts_with("claims")).cloned().collect();
        // Each follow keeps the generation it moves to, sign-out included.
        let want = vec![
            format!("claims {}", a.generation),
            format!("claims {}", b.generation),
            format!("claims {}", b.generation + 1),
        ];
        assert_eq!(seen, want);
    }

    /// Coming back on screen checks the sends still waiting on Twitch's copy.
    #[tokio::test]
    async fn delivery_rechecks_on_resume() {
        let r = rig("99999949");
        let _ = r.follow("99999949");
        lock(&r.events.log).clear();
        r.engine.note_away(true);
        assert!(!lock(&r.events.log).contains(&"recheck".to_string()));
        r.clock.0.fetch_add(60_000, Ordering::SeqCst);
        r.engine.note_away(false);
        assert!(lock(&r.events.log).contains(&"recheck".to_string()));
    }
}
