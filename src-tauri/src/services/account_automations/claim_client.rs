//! The claim client: asks streamnook.app whether this device answers an
//! event, and gives a claim back when the answer never went out.
//!
//! Every claim runs on its own spawned task, so the request always finishes
//! (and teaches the transport which route works) even after the job stopped
//! waiting for it. The job waits at most its budget. Three pools keep the
//! traffic apart: rule claims (16 in flight, beyond that a claim is Unknown
//! at once), reminder claims (4, a reminder waits for a slot) and releases
//! (8, a queue of at most 256 waits for a slot).
//!
//! What fails, and how it is read:
//!
//! - a valid answer: per element, by its position in the request;
//! - no answer in time, a 5xx, an unreadable 2xx, a network error: Unknown,
//!   and the claim task sends a release once the request ends (the request
//!   may have reached the function; releases are idempotent);
//! - a 429: Unknown with its own status (it never counts toward the outage);
//! - a 400, 401, 404 or 413, or a full pool: Unknown, nothing to release;
//!   a 401 also drops the bearer's cached owner, so its next use is checked.
//!
//! A rule claim's Won that arrives after the job stopped waiting is released
//! by the task. A reminder never releases. Every release names its owner and
//! takes a freshly verified owner token when it is sent (each retry too),
//! never a token kept from the claim: a check Twitch could not answer waits
//! for the release's next retry; a refused or wrong-account check drops the
//! release.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::future::Future;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{oneshot, Semaphore};

use super::claims::{self, Element, KeyMode};
use super::outward::Outward;
use super::store::Device;
use super::verify::TokenRefusal;
use super::{lock, now_ms, Spawn};

/// A release's owner check spends at most this long, per attempt.
const RELEASE_VERIFY: Duration = Duration::from_secs(5);

/// One release attempt's outcome.
enum ReleaseTry {
    /// Answered; how many rows it released.
    Done(Option<u64>),
    /// Try again at the next retry.
    Again,
    /// Give up (the owner has no verified token, or the site refused the body).
    Drop,
}

/// A windowed event element's (key, family, window), the only kind cached.
fn windowed_event(e: &Element) -> Option<(&str, &str, u32)> {
    match e {
        Element::Event { key, family, mode: KeyMode::Windowed, window_s: Some(window), .. } => {
            Some((key.as_str(), family.as_str(), *window))
        }
        _ => None,
    }
}

/// Rule claims in flight at once.
pub const RULE_POOL: usize = 16;
/// Reminder claims in flight at once.
pub const REMINDER_POOL: usize = 4;
/// Releases in flight at once.
pub const RELEASE_POOL: usize = 8;
/// Releases waiting for a slot.
pub const RELEASE_QUEUE_CAP: usize = 256;
/// A loser re-claims at most this often in a minute.
pub const RECLAIMS_PER_MINUTE: usize = 60;
/// No release older than this is ever sent (the server refuses them).
pub const RELEASE_HORIZON: Duration = Duration::from_secs(2 * 3600);
/// Unknown answers in a row before the status says StreamNook can't be reached.
const OUTAGE_AFTER: u32 = 3;
/// A cached windowed win is trusted until this long before it lapses.
const CACHE_MARGIN: Duration = Duration::from_secs(2);

/// The claim client's waits.
#[derive(Clone, Copy, Debug)]
pub struct ClaimTimings {
    /// A lost exact event is claimed again this long after the loss.
    pub reclaim_at: [Duration; 4],
    /// A release that failed is tried again after these.
    pub release_retry: [Duration; 2],
    /// A release after an Unknown that answered `released: 0` is repeated once after this.
    pub unknown_repeat: Duration,
    /// A reminder claim waits at most this long.
    pub reminder_bound: Duration,
}

impl ClaimTimings {
    pub fn production() -> ClaimTimings {
        ClaimTimings {
            reclaim_at: [
                Duration::from_millis(1_500),
                Duration::from_secs(3),
                Duration::from_secs(6),
                Duration::from_secs(12),
            ],
            release_retry: [Duration::from_secs(2), Duration::from_secs(10)],
            unknown_repeat: Duration::from_secs(10),
            reminder_bound: Duration::from_secs(15),
        }
    }
}

/// Which budget a call spends on the site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallKind {
    Rule,
    Reminder,
    /// A loser's re-claim.
    Reclaim,
}

/// One claim call.
#[derive(Clone, Debug)]
pub struct ClaimRequest {
    /// The rule's owner (a release verifies a fresh token for it).
    pub owner_id: String,
    /// The owner's verified token: the site scopes the claim to its account.
    pub bearer: String,
    pub elements: Vec<Element>,
    pub kind: CallKind,
    /// How long the job waits for the answer.
    pub budget: Duration,
    /// The holder's longest time to send, for the release queue's order.
    pub settle_ms: u64,
    /// For a windowed fire: when the next one comes (a cached win must cover it).
    pub next_fire_in: Option<Duration>,
}

/// A present `null` reads as the type's default, the same as a missing key.
/// The database sends every key (`same_device` as a boolean); this keeps an
/// answer readable if a null ever comes back.
fn default_on_null<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// One element's answer, named by its position in the request.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ElementAnswer {
    pub index: usize,
    #[serde(default, deserialize_with = "default_on_null")]
    pub kind: String,
    #[serde(default, deserialize_with = "default_on_null")]
    pub won: bool,
    #[serde(default)]
    pub holder_name: Option<String>,
    #[serde(default, deserialize_with = "default_on_null")]
    pub same_device: bool,
    #[serde(default)]
    pub holder_age_ms: Option<i64>,
    #[serde(default)]
    pub holder_settle_ms: Option<i64>,
    #[serde(default)]
    pub free_at_ms: Option<i64>,
    /// How long, in ms, until a counted rail (or the whisper day budget) has
    /// room: a duration, never a time.
    #[serde(default)]
    pub retry_after_ms: Option<i64>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// A valid claim answer.
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Answer {
    pub elements: Vec<ElementAnswer>,
    pub server_now_ms: i64,
}

impl Answer {
    fn any_won(&self) -> bool {
        self.elements.iter().any(|e| e.won && e.kind == "event")
    }
}

/// Why a claim has no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unknown {
    /// 16 rule claims already in flight.
    PoolFull,
    /// The budget ran out while the request was in flight.
    Deadline,
    /// A 429.
    Busy,
    Network,
    /// A 5xx.
    Server(u16),
    /// A 2xx that is not a claim answer (an HTML page, another shape).
    Unreadable,
    /// An answer that says the request never reached the function (400, 401, 404, 413).
    NotReached(u16),
}

impl Unknown {
    /// The Activity words.
    pub fn words(&self) -> &'static str {
        match self {
            Unknown::PoolFull => "StreamNook is slow right now",
            Unknown::Busy => "StreamNook is busy; rules wait a moment",
            _ => "Couldn't check with StreamNook; skipped",
        }
    }

    fn counts_toward_outage(&self) -> bool {
        !matches!(self, Unknown::Busy | Unknown::PoolFull)
    }
}

/// What a claim decided.
#[derive(Clone, Debug)]
pub enum ClaimResult {
    Answered { call_id: String, claimed_at: Instant, answer: Answer },
    /// A windowed event decided by the cache, with no call.
    Cached { won: bool, holder_name: String, free_at_ms: Option<i64> },
    Unknown { call_id: Option<String>, why: Unknown },
}

/// What the request task saw.
#[derive(Clone, Debug)]
enum Reply {
    Valid(Answer),
    Status(u16),
    Unreadable,
    Network,
}

impl Reply {
    /// The request may have reached the function and written something.
    fn may_have_reached(&self) -> bool {
        match self {
            Reply::Valid(_) => false,
            Reply::Status(code) => *code >= 500,
            Reply::Unreadable | Reply::Network => true,
        }
    }
}

#[derive(Clone, Debug)]
struct Cached {
    won: bool,
    holder_name: String,
    free_at_ms: Option<i64>,
    until: Instant,
}

/// A release waiting for one of the release slots.
struct PendingRelease {
    /// The owner whose verified token the release is sent with.
    owner_id: String,
    call_id: String,
    claimed_at: Instant,
    /// No loser can act on it after this: it is the first to drop when the queue is full.
    horizon_at: Instant,
    repeat_on_zero: bool,
    done: Option<oneshot::Sender<()>>,
}

/// The claim client: one per engine.
pub struct ClaimClient {
    outward: Arc<dyn Outward>,
    timings: ClaimTimings,
    /// Where claim and release tasks run (detached: they outlive the job).
    spawner: Spawn,
    session: String,
    device_hash: String,
    device_name: RwLock<String>,
    rule_pool: Arc<Semaphore>,
    reminder_pool: Arc<Semaphore>,
    releases: Mutex<VecDeque<PendingRelease>>,
    release_workers: AtomicUsize,
    reclaims: Mutex<VecDeque<Instant>>,
    offset_ms: AtomicI64,
    unknown_streak: AtomicU32,
    outage: AtomicBool,
    busy: AtomicBool,
    cache: Mutex<HashMap<(String, String, String), Cached>>,
    /// A 429's `retry_after_s`, by the call id that got it; a reminder takes its own.
    retry_hints: Mutex<HashMap<String, Duration>>,
    #[cfg(test)]
    pub(crate) most_releases_in_flight: AtomicUsize,
    #[cfg(test)]
    pub(crate) dropped_releases: Mutex<Vec<String>>,
}

impl ClaimClient {
    pub fn new(outward: Arc<dyn Outward>, device: &Device, timings: ClaimTimings, spawner: Spawn) -> ClaimClient {
        ClaimClient {
            outward,
            timings,
            spawner,
            session: claims::new_session(),
            device_hash: claims::device_hash(&device.id),
            device_name: RwLock::new(device.name.clone()),
            rule_pool: Arc::new(Semaphore::new(RULE_POOL)),
            reminder_pool: Arc::new(Semaphore::new(REMINDER_POOL)),
            releases: Mutex::new(VecDeque::new()),
            release_workers: AtomicUsize::new(0),
            reclaims: Mutex::new(VecDeque::new()),
            offset_ms: AtomicI64::new(0),
            unknown_streak: AtomicU32::new(0),
            outage: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            cache: Mutex::new(HashMap::new()),
            retry_hints: Mutex::new(HashMap::new()),
            #[cfg(test)]
            most_releases_in_flight: AtomicUsize::new(0),
            #[cfg(test)]
            dropped_releases: Mutex::new(Vec::new()),
        }
    }

    /// Run a claim or release task on the client's spawner, detached.
    fn spawn(&self, task: impl Future<Output = ()> + Send + 'static) {
        (self.spawner)(Box::pin(task));
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    pub fn set_device_name(&self, name: &str) {
        *self.device_name.write().unwrap_or_else(|e| e.into_inner()) = name.to_string();
    }

    fn device_name(&self) -> String {
        self.device_name.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Three claims in a row went unanswered.
    pub fn outage(&self) -> bool {
        self.outage.load(Ordering::SeqCst)
    }

    /// The site asked the app to slow down (a 429) since the last answer.
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    /// The server's clock now, through the offset the last answer taught.
    pub fn server_now_ms(&self) -> i64 {
        now_ms() + self.offset_ms.load(Ordering::SeqCst)
    }

    /// One more re-claim this minute, if the device has any left.
    pub fn take_reclaim(&self) -> bool {
        let now = Instant::now();
        let mut recent = lock(&self.reclaims);
        while recent.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60)) {
            recent.pop_front();
        }
        if recent.len() >= RECLAIMS_PER_MINUTE {
            return false;
        }
        recent.push_back(now);
        true
    }

    fn body(&self, call_id: &str, request: &ClaimRequest) -> Value {
        let mut body = json!({
            "action": "claim",
            "call_id": call_id,
            "session": self.session,
            "device_hash": self.device_hash,
            "name": self.device_name(),
            "elements": request.elements,
        });
        match request.kind {
            CallKind::Reminder => body["reminder"] = json!(true),
            CallKind::Reclaim => body["reclaim"] = json!(true),
            CallKind::Rule => {}
        }
        body
    }

    /// A windowed event decided by the cache alone: every element is a
    /// windowed event with a cached answer that still holds.
    fn from_cache(&self, request: &ClaimRequest) -> Option<ClaimResult> {
        if request.elements.is_empty() || !request.elements.iter().all(|e| windowed_event(e).is_some()) {
            return None;
        }
        let now = Instant::now();
        let next = now + request.next_fire_in.unwrap_or_default();
        let cache = lock(&self.cache);
        let mut all_won = true;
        for e in &request.elements {
            let Some((k, family, _)) = windowed_event(e) else { return None };
            let key = (request.owner_id.clone(), k.to_string(), family.to_string());
            match cache.get(&key) {
                Some(c) if !c.won && c.until > now => {
                    return Some(ClaimResult::Cached { won: false, holder_name: c.holder_name.clone(), free_at_ms: c.free_at_ms });
                }
                Some(c) if c.won && c.until > next => {}
                _ => all_won = false,
            }
        }
        all_won.then(|| ClaimResult::Cached { won: true, holder_name: String::new(), free_at_ms: None })
    }

    fn remember(&self, request: &ClaimRequest, answer: &Answer) {
        let now = Instant::now();
        let mut cache = lock(&self.cache);
        cache.retain(|_, c| c.until > now);
        for a in &answer.elements {
            let Some((k, family, window_s)) = request.elements.get(a.index).and_then(windowed_event) else { continue };
            let key = (request.owner_id.clone(), k.to_string(), family.to_string());
            if a.won {
                let window = Duration::from_secs(u64::from(window_s));
                let until = (now + window).checked_sub(CACHE_MARGIN).unwrap_or(now);
                cache.insert(key, Cached { won: true, holder_name: String::new(), free_at_ms: None, until });
            } else if a.reason.as_deref() == Some("held") {
                let until = self.local_instant(a.free_at_ms);
                cache.insert(
                    key,
                    Cached { won: false, holder_name: a.holder_name.clone().unwrap_or_default(), free_at_ms: a.free_at_ms, until },
                );
            }
        }
    }

    /// A server time as a local instant (now when it has passed).
    fn local_instant(&self, server_ms: Option<i64>) -> Instant {
        let left = server_ms.map_or(0, |at| at - self.server_now_ms()).max(0);
        Instant::now() + Duration::from_millis(left as u64)
    }

    fn note_unknown(&self, why: &Unknown) {
        if *why == Unknown::Busy {
            self.busy.store(true, Ordering::SeqCst);
        }
        if why.counts_toward_outage() && self.unknown_streak.fetch_add(1, Ordering::SeqCst) + 1 >= OUTAGE_AFTER {
            self.outage.store(true, Ordering::SeqCst);
        }
    }

    fn note_answer(&self, answer: &Answer) {
        self.offset_ms.store(answer.server_now_ms - now_ms(), Ordering::SeqCst);
        self.unknown_streak.store(0, Ordering::SeqCst);
        self.outage.store(false, Ordering::SeqCst);
        self.busy.store(false, Ordering::SeqCst);
    }

    /// Claim the elements. A rule claim waits at most its budget; a reminder
    /// claim waits for a slot and for the answer up to its bound.
    pub async fn claim(self: &Arc<Self>, request: ClaimRequest) -> ClaimResult {
        if let Some(cached) = self.from_cache(&request) {
            return cached;
        }
        let call_id = claims::new_call_id();
        let started = Instant::now();
        let reminder = request.kind == CallKind::Reminder;
        // A reminder waits for a pool slot and the answer inside its own bound, and
        // never past what its shared deadline (verify plus claim) has left.
        let budget = if reminder { self.timings.reminder_bound.min(request.budget) } else { request.budget };
        let permit = if reminder {
            match tokio::time::timeout(budget, self.reminder_pool.clone().acquire_owned()).await {
                Ok(Ok(permit)) => permit,
                _ => return self.unknown(None, Unknown::Deadline),
            }
        } else {
            match self.rule_pool.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => return self.unknown(None, Unknown::PoolFull),
            }
        };
        let body = self.body(&call_id, &request);
        let (tx, rx) = oneshot::channel::<Reply>();
        let client = self.clone();
        let task_call = call_id.clone();
        let bearer = request.bearer.clone();
        let owner_id = request.owner_id.clone();
        let settle_ms = request.settle_ms;
        self.spawn(async move {
            let reply = client.post(&bearer, &owner_id, &body).await;
            if matches!(reply, Reply::Status(401)) {
                // Twitch no longer takes this token: its cached owner goes.
                client.outward.verifier().drop_token_of(&bearer);
            }
            let delivered = tx.send(reply.clone()).is_ok();
            drop(permit);
            if !reminder {
                client.after_rule_call(&owner_id, &task_call, started, settle_ms, &reply, delivered);
            }
        });
        let left = budget.saturating_sub(started.elapsed());
        match tokio::time::timeout(left, rx).await {
            Ok(Ok(Reply::Valid(answer))) => {
                self.note_answer(&answer);
                self.remember(&request, &answer);
                ClaimResult::Answered { call_id, claimed_at: started, answer }
            }
            Ok(Ok(Reply::Status(429))) => self.unknown(Some(call_id), Unknown::Busy),
            Ok(Ok(Reply::Status(code))) if code >= 500 => self.unknown(Some(call_id), Unknown::Server(code)),
            Ok(Ok(Reply::Status(code))) => self.unknown(Some(call_id), Unknown::NotReached(code)),
            Ok(Ok(Reply::Unreadable)) => self.unknown(Some(call_id), Unknown::Unreadable),
            Ok(Ok(Reply::Network)) | Ok(Err(_)) => self.unknown(Some(call_id), Unknown::Network),
            Err(_) => self.unknown(Some(call_id), Unknown::Deadline),
        }
    }

    /// Keep the wait a 429's body named (`retry_after_s`), by the request's call id.
    fn note_retry_hint(&self, request: &Value, answer: &str) {
        let Some(call_id) = request["call_id"].as_str() else { return };
        let Some(secs) = serde_json::from_str::<Value>(answer).ok().and_then(|v| v["retry_after_s"].as_u64()) else { return };
        let mut hints = lock(&self.retry_hints);
        if hints.len() >= 64 {
            hints.clear();
        }
        hints.insert(call_id.to_string(), Duration::from_secs(secs));
    }

    /// The wait the 429 for `call_id` named, once.
    pub fn take_retry_hint(&self, call_id: &str) -> Option<Duration> {
        lock(&self.retry_hints).remove(call_id)
    }

    fn unknown(&self, call_id: Option<String>, why: Unknown) -> ClaimResult {
        self.note_unknown(&why);
        ClaimResult::Unknown { call_id, why }
    }

    async fn post(&self, bearer: &str, owner_id: &str, body: &Value) -> Reply {
        match self.outward.claim_post(bearer, owner_id, body).await {
            Err(_) => Reply::Network,
            Ok(resp) if resp.status == 200 => match serde_json::from_str::<Answer>(&resp.body) {
                Ok(answer) => Reply::Valid(answer),
                Err(_) => Reply::Unreadable,
            },
            Ok(resp) if (200..300).contains(&resp.status) => Reply::Unreadable,
            Ok(resp) => {
                if resp.status == 429 {
                    self.note_retry_hint(body, &resp.body);
                }
                Reply::Status(resp.status)
            }
        }
    }

    /// After a rule claim's request ended: give back a late win, and release
    /// whenever the request may have written something nobody will use.
    fn after_rule_call(
        self: &Arc<Self>,
        owner_id: &str,
        call_id: &str,
        claimed_at: Instant,
        settle_ms: u64,
        reply: &Reply,
        delivered: bool,
    ) {
        let late_win = !delivered && matches!(reply, Reply::Valid(a) if a.any_won());
        let maybe_written = reply.may_have_reached();
        if late_win || maybe_written {
            let _ = self.queue_release(owner_id, call_id, claimed_at, settle_ms, maybe_written);
        }
    }

    /// Give back a won call whose class sent nothing. The release is sent
    /// with a token verified as `owner_id` at that moment. Resolves once the
    /// first attempt finished (or the release was dropped or refused as too old).
    pub fn release(self: &Arc<Self>, owner_id: &str, call_id: &str, claimed_at: Instant, settle_ms: u64) -> oneshot::Receiver<()> {
        self.queue_release(owner_id, call_id, claimed_at, settle_ms, false)
    }

    fn queue_release(
        self: &Arc<Self>,
        owner_id: &str,
        call_id: &str,
        claimed_at: Instant,
        settle_ms: u64,
        repeat_on_zero: bool,
    ) -> oneshot::Receiver<()> {
        let (done, rx) = oneshot::channel();
        if claimed_at.elapsed() >= RELEASE_HORIZON {
            let _ = done.send(());
            return rx;
        }
        let last_reclaim = self.timings.reclaim_at[self.timings.reclaim_at.len() - 1];
        let horizon_at = claimed_at + Duration::from_millis(settle_ms) + last_reclaim;
        {
            let mut queue = lock(&self.releases);
            if queue.len() >= RELEASE_QUEUE_CAP {
                let now = Instant::now();
                let drop_at = queue.iter().position(|r| r.horizon_at < now).unwrap_or(0);
                if let Some(gone) = queue.remove(drop_at) {
                    #[cfg(test)]
                    lock(&self.dropped_releases).push(gone.call_id.clone());
                    drop(gone);
                }
            }
            queue.push_back(PendingRelease {
                owner_id: owner_id.to_string(),
                call_id: call_id.to_string(),
                claimed_at,
                horizon_at,
                repeat_on_zero,
                done: Some(done),
            });
        }
        self.pump();
        rx
    }

    /// Start release workers up to the pool size.
    fn pump(self: &Arc<Self>) {
        loop {
            let running = self.release_workers.load(Ordering::SeqCst);
            if running >= RELEASE_POOL || lock(&self.releases).is_empty() {
                return;
            }
            if self
                .release_workers
                .compare_exchange(running, running + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                continue;
            }
            #[cfg(test)]
            self.most_releases_in_flight.fetch_max(running + 1, Ordering::SeqCst);
            let client = self.clone();
            self.spawn(async move {
                loop {
                    let next = lock(&client.releases).pop_front();
                    let Some(release) = next else { break };
                    client.send_release(release).await;
                }
                client.release_workers.fetch_sub(1, Ordering::SeqCst);
                // A release queued while this worker was stopping still gets one.
                client.pump();
            });
        }
    }

    /// Post one release with a token verified as its owner right now.
    async fn post_release(&self, owner_id: &str, body: &Value) -> ReleaseTry {
        let token = match self.outward.verified_token(owner_id, RELEASE_VERIFY).await {
            Ok(verified) => verified.token,
            // Twitch could not be asked (or an account change is running):
            // the release's next retry asks again.
            Err(TokenRefusal::Unreachable(_)) => return ReleaseTry::Again,
            // No token is the owner's any more: nothing can be released.
            Err(_) => return ReleaseTry::Drop,
        };
        match self.outward.claim_post(&token, owner_id, body).await {
            Err(_) => ReleaseTry::Again,
            Ok(resp) if resp.status == 200 => ReleaseTry::Done(
                serde_json::from_str::<Value>(&resp.body).ok().and_then(|v| v["released"].as_u64()),
            ),
            Ok(resp) if resp.status == 401 => {
                self.outward.verifier().drop_token_of(&token);
                ReleaseTry::Again
            }
            Ok(resp) if (400..500).contains(&resp.status) && resp.status != 429 => ReleaseTry::Drop,
            Ok(_) => ReleaseTry::Again,
        }
    }

    /// One release: up to three attempts, each with a freshly verified owner
    /// token; after an Unknown, a `released: 0` answer is repeated once later
    /// (the claim may have landed since).
    async fn send_release(&self, mut release: PendingRelease) {
        let body = json!({ "action": "release", "call_id": release.call_id, "session": self.session });
        let mut released = None;
        for attempt in 0..=self.timings.release_retry.len() {
            if release.claimed_at.elapsed() >= RELEASE_HORIZON {
                break;
            }
            match self.post_release(&release.owner_id, &body).await {
                ReleaseTry::Done(count) => {
                    released = count;
                    break;
                }
                ReleaseTry::Drop => break,
                ReleaseTry::Again => {}
            }
            if let Some(wait) = self.timings.release_retry.get(attempt) {
                tokio::time::sleep(*wait).await;
            }
        }
        if let Some(done) = release.done.take() {
            let _ = done.send(());
        }
        if release.repeat_on_zero && released == Some(0) {
            tokio::time::sleep(self.timings.unknown_repeat).await;
            if release.claimed_at.elapsed() < RELEASE_HORIZON {
                let _ = self.post_release(&release.owner_id, &body).await;
            }
        }
    }
}

#[cfg(test)]
impl ElementAnswer {
    pub(crate) fn won(index: usize) -> ElementAnswer {
        ElementAnswer {
            index,
            kind: "event".into(),
            won: true,
            holder_name: None,
            same_device: false,
            holder_age_ms: None,
            holder_settle_ms: None,
            free_at_ms: None,
            retry_after_ms: None,
            reason: None,
        }
    }

    pub(crate) fn lost(index: usize, reason: &str) -> ElementAnswer {
        ElementAnswer { won: false, reason: Some(reason.into()), ..ElementAnswer::won(index) }
    }

    pub(crate) fn lost_by(index: usize, reason: &str, holder: &str, free_at_ms: Option<i64>) -> ElementAnswer {
        ElementAnswer { holder_name: Some(holder.into()), free_at_ms, ..ElementAnswer::lost(index, reason) }
    }

    pub(crate) fn held(index: usize, holder: &str, same_device: bool, age_ms: i64, settle_ms: i64) -> ElementAnswer {
        ElementAnswer {
            holder_name: Some(holder.into()),
            same_device,
            holder_age_ms: Some(age_ms),
            holder_settle_ms: Some(settle_ms),
            ..ElementAnswer::lost(index, "held")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit::*;
    use super::*;

    fn request(bearer: &str, budget_ms: u64) -> ClaimRequest {
        ClaimRequest {
            owner_id: main_id(),
            bearer: bearer.into(),
            elements: vec![event_el("chat:m1")],
            kind: CallKind::Rule,
            budget: Duration::from_millis(budget_ms),
            settle_ms: 15_000,
            next_fire_in: None,
        }
    }

    /// The database's default element object as the claim function sends it
    /// (every key present, `same_device` false). The skipped rail's
    /// `same_device: null` is the tolerance case: a present null still reads.
    #[test]
    fn the_databases_default_element_shape_parses() {
        use super::super::executor::{read_answer, Reading};
        let won = r#"{"elements": [
            {"index": 0, "kind": "event", "won": true, "holder_name": null, "same_device": false, "holder_age_ms": null,
             "holder_settle_ms": null, "free_at_ms": null, "retry_after_ms": null, "reason": null},
            {"index": 1, "kind": "windowed", "won": true, "holder_name": null, "same_device": false, "holder_age_ms": null,
             "holder_settle_ms": null, "free_at_ms": null, "retry_after_ms": null, "reason": null}
        ], "server_now_ms": 1700000000000}"#;
        let answer: Answer = serde_json::from_str(won).expect("a won claim reads");
        assert!(answer.any_won());
        assert!(!answer.elements[0].same_device);
        assert_eq!(read_answer(&[event_el("chat:m1"), rail_el("gap:4242", 30)], &answer.elements), Reading::Won(vec![0]));

        let skipped = r#"{"elements": [
            {"index": 0, "kind": "event", "won": false, "holder_name": "Mac 91bc", "same_device": false, "holder_age_ms": 900,
             "holder_settle_ms": 15000, "free_at_ms": null, "retry_after_ms": null, "reason": "held"},
            {"index": 1, "kind": "windowed", "won": false, "holder_name": null, "same_device": null, "holder_age_ms": null,
             "holder_settle_ms": null, "free_at_ms": null, "retry_after_ms": null, "reason": "skipped"}
        ], "server_now_ms": 1700000000000}"#;
        let answer: Answer = serde_json::from_str(skipped).expect("a held event and a skipped rail read");
        assert!(!answer.any_won());
        assert_eq!(answer.elements[1].reason.as_deref(), Some("skipped"));

        let pace = r#"{"elements": [
            {"index": 0, "kind": "event", "won": false, "holder_name": null, "same_device": false, "holder_age_ms": null,
             "holder_settle_ms": null, "free_at_ms": null, "retry_after_ms": null, "reason": "rail"},
            {"index": 1, "kind": "counted", "won": false, "holder_name": null, "same_device": false, "holder_age_ms": null,
             "holder_settle_ms": null, "free_at_ms": null, "retry_after_ms": 61000, "reason": "pace"}
        ], "server_now_ms": 1700000000000}"#;
        let answer: Answer = serde_json::from_str(pace).expect("a pace loss reads");
        assert_eq!(answer.elements[1].retry_after_ms, Some(61_000), "a duration in ms");
        assert_eq!(read_answer(&[event_el("chat:m1"), rail_el("pace:1", 60)], &answer.elements), Reading::Pace);
    }

    #[tokio::test]
    async fn html_and_404_are_unknown_and_release_nothing() {
        let kit = Kit::new();
        kit.outward.claim_answers(vec![FakeHttp::Status(200, "<html></html>".into()), FakeHttp::Status(404, "".into())]);
        let a = kit.engine.claims.claim(request("t", 500)).await;
        assert!(matches!(a, ClaimResult::Unknown { why: Unknown::Unreadable, .. }));
        let b = kit.engine.claims.claim(request("t", 500)).await;
        assert!(matches!(b, ClaimResult::Unknown { why: Unknown::NotReached(404), .. }));
        kit.settle_claims().await;
        // One release for the unreadable 2xx, repeated once on `released: 0` (D93); the 404 sends none.
        assert_eq!(kit.outward.releases().len(), 2, "only the unreadable 2xx may have written");
    }

    #[tokio::test]
    async fn a_deadline_mid_flight_releases_from_the_task_after_the_request_ends() {
        let kit = Kit::new();
        kit.outward.delay_claims(Duration::from_millis(200));
        let result = kit.engine.claims.claim(request("t", 50)).await;
        assert!(matches!(result, ClaimResult::Unknown { why: Unknown::Deadline, .. }));
        assert!(kit.outward.releases().is_empty(), "never before the request ends");
        kit.settle_claims().await;
        assert_eq!(kit.outward.releases().len(), 1, "the late win is given back");
        assert_eq!(kit.outward.calls().iter().filter(|c| *c == "claim").count(), 1);
    }

    #[tokio::test]
    async fn no_release_after_pool_full_429_400_413_or_401() {
        for http in [FakeHttp::Status(429, "".into()), FakeHttp::Status(400, "".into()), FakeHttp::Status(413, "".into()), FakeHttp::Status(401, "".into())] {
            let kit = Kit::new();
            kit.outward.claim_answers(vec![http]);
            let _ = kit.engine.claims.claim(request("t", 500)).await;
            kit.settle_claims().await;
            assert!(kit.outward.releases().is_empty());
        }
        let kit = Kit::new();
        kit.outward.park_claims();
        let mut waits = Vec::new();
        for _ in 0..RULE_POOL {
            let client = kit.engine.claims.clone();
            waits.push(tokio::spawn(async move { client.claim(request("t", 300)).await }));
        }
        kit.outward.wait_for_claims_started(RULE_POOL).await;
        let seventeenth = kit.engine.claims.claim(request("t", 300)).await;
        match seventeenth {
            ClaimResult::Unknown { why, call_id } => {
                assert_eq!(why, Unknown::PoolFull);
                assert_eq!(why.words(), "StreamNook is slow right now");
                assert!(call_id.is_none());
            }
            other => panic!("{other:?}"),
        }
        kit.outward.release_claims();
        for w in waits {
            let _ = w.await;
        }
    }

    #[tokio::test]
    async fn a_release_follows_every_5xx_unreadable_and_network_error() {
        for http in [FakeHttp::Status(503, "".into()), FakeHttp::Status(200, "{}".into()), FakeHttp::Network] {
            let kit = Kit::new();
            kit.outward.claim_answers(vec![http]);
            let _ = kit.engine.claims.claim(request("t", 500)).await;
            kit.settle_claims().await;
            // The release, repeated once on `released: 0` (D93).
            assert_eq!(kit.outward.releases().len(), 2);
        }
    }

    #[tokio::test]
    async fn a_429_has_its_own_status_and_never_counts_toward_the_outage() {
        let kit = Kit::new();
        kit.outward.claim_answers(vec![
            FakeHttp::Status(429, "".into()),
            FakeHttp::Status(429, "".into()),
            FakeHttp::Status(429, "".into()),
            FakeHttp::Status(429, "".into()),
        ]);
        for _ in 0..4 {
            let r = kit.engine.claims.claim(request("t", 300)).await;
            assert!(matches!(r, ClaimResult::Unknown { why: Unknown::Busy, .. }));
        }
        assert!(kit.engine.claims.busy());
        assert!(!kit.engine.claims.outage());
    }

    #[tokio::test]
    async fn three_unknowns_in_a_row_set_the_outage_and_an_answer_clears_it() {
        let kit = Kit::new();
        kit.outward.claim_answers(vec![FakeHttp::Network, FakeHttp::Network, FakeHttp::Network]);
        for _ in 0..3 {
            let _ = kit.engine.claims.claim(request(&token_of(&main_id()), 300)).await;
        }
        assert!(kit.engine.claims.outage());
        let r = kit.engine.claims.claim(request(&token_of(&main_id()), 300)).await;
        assert!(matches!(r, ClaimResult::Answered { .. }));
        assert!(!kit.engine.claims.outage());
    }

    #[tokio::test]
    async fn a_won_answer_received_in_time_is_never_also_released_by_the_task() {
        let kit = Kit::new();
        let r = kit.engine.claims.claim(request(&token_of(&main_id()), 500)).await;
        assert!(matches!(r, ClaimResult::Answered { .. }));
        kit.settle_claims().await;
        assert!(kit.outward.releases().is_empty());
    }

    #[tokio::test]
    async fn two_identical_calls_make_two_requests() {
        let kit = Kit::new();
        let a = kit.engine.claims.claim(request(&token_of(&main_id()), 500));
        let b = kit.engine.claims.claim(request(&token_of(&main_id()), 500));
        let _ = futures::future::join(a, b).await;
        assert_eq!(kit.outward.claim_calls(), 2);
    }

    #[tokio::test]
    async fn the_request_task_keeps_running_past_the_deadline() {
        let kit = Kit::new();
        kit.outward.delay_claims(Duration::from_millis(150));
        let _ = kit.engine.claims.claim(request(&token_of(&main_id()), 20)).await;
        kit.settle_claims().await;
        assert_eq!(kit.outward.finished_claims(), 1);
    }

    #[tokio::test]
    async fn a_cached_windowed_lost_skips_with_no_call_until_it_frees() {
        let kit = Kit::new();
        let bearer = token_of(&main_id());
        kit.server.hold_windows("timer:", "Mac 91bc");
        let mut req = request(&bearer, 500);
        req.elements = vec![windowed_event_el("timer:twitch:chan", 60)];
        let first = kit.engine.claims.claim(req.clone()).await;
        assert!(matches!(first, ClaimResult::Answered { .. }));
        let calls = kit.outward.claim_calls();
        let second = kit.engine.claims.claim(req.clone()).await;
        assert!(matches!(second, ClaimResult::Cached { won: false, .. }));
        assert_eq!(kit.outward.claim_calls(), calls);
    }

    #[tokio::test]
    async fn a_cached_windowed_win_stands_in_only_while_the_next_fire_is_inside_it() {
        let kit = Kit::new();
        let bearer = token_of(&main_id());
        let mut req = request(&bearer, 500);
        req.elements = vec![windowed_event_el("timer:twitch:chan", 60)];
        let _ = kit.engine.claims.claim(req.clone()).await;
        let calls = kit.outward.claim_calls();
        req.next_fire_in = Some(Duration::from_secs(30));
        assert!(matches!(kit.engine.claims.claim(req.clone()).await, ClaimResult::Cached { won: true, .. }));
        assert_eq!(kit.outward.claim_calls(), calls);
        req.next_fire_in = Some(Duration::from_secs(59));
        assert!(matches!(kit.engine.claims.claim(req.clone()).await, ClaimResult::Answered { .. }));
        assert_eq!(kit.outward.claim_calls(), calls + 1);
    }

    #[tokio::test]
    async fn rail_losses_and_exact_losses_are_never_cached() {
        let kit = Kit::new();
        let bearer = token_of(&main_id());
        kit.server.hold_windows("gap:", "Mac 91bc");
        let mut req = request(&bearer, 500);
        req.elements = vec![event_el("chat:m9"), rail_el("gap:4242", 30)];
        let _ = kit.engine.claims.claim(req.clone()).await;
        let _ = kit.engine.claims.claim(req.clone()).await;
        assert_eq!(kit.outward.claim_calls(), 2);
    }

    #[tokio::test]
    async fn a_fast_clock_caches_a_lost_until_the_servers_free_time() {
        let kit = Kit::new();
        kit.server.skew(-120_000);
        kit.server.hold_windows("timer:", "Mac 91bc");
        let bearer = token_of(&main_id());
        let mut req = request(&bearer, 500);
        req.elements = vec![windowed_event_el("timer:twitch:chan", 60)];
        let _ = kit.engine.claims.claim(req.clone()).await;
        let cache = lock(&kit.engine.claims.cache);
        let entry = cache.values().next().unwrap();
        let left = entry.until.saturating_duration_since(Instant::now());
        assert!(left > Duration::from_secs(50), "{left:?}: not two minutes early");
    }

    #[tokio::test]
    async fn the_release_queue_holds_256_and_drops_the_ones_past_every_horizon_first() {
        let kit = Kit::new();
        kit.outward.park_releases();
        let old = Instant::now() - Duration::from_secs(600);
        let mut receivers = Vec::new();
        for i in 0..300u32 {
            let claimed_at = if i % 2 == 0 { old } else { Instant::now() };
            receivers.push(kit.engine.claims.release("t", &format!("{i:016x}"), claimed_at, 15_000));
        }
        let queued = lock(&kit.engine.claims.releases).len();
        assert!(queued <= RELEASE_QUEUE_CAP);
        let dropped = lock(&kit.engine.claims.dropped_releases).clone();
        assert!(!dropped.is_empty());
        assert!(dropped.iter().all(|id| u32::from_str_radix(id, 16).unwrap() % 2 == 0), "past-horizon releases go first");
        assert!(kit.engine.claims.most_releases_in_flight.load(Ordering::SeqCst) <= RELEASE_POOL);
        kit.outward.release_releases();
    }

    #[tokio::test]
    async fn a_failed_release_is_tried_twice_more() {
        let kit = Kit::new();
        kit.outward.release_answers(vec![FakeHttp::Network, FakeHttp::Status(503, "".into()), FakeHttp::Network]);
        let rx = kit.engine.claims.release("t", "00000000000000aa", Instant::now(), 15_000);
        let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
        assert_eq!(kit.outward.releases().len(), 3);
    }

    #[tokio::test]
    async fn every_release_attempt_verifies_its_owner_again() {
        let kit = Kit::new();
        // The owner's fetch answers T1, then T3 (a refreshed token): both are the owner's.
        kit.outward.script_tokens(&main_id(), vec!["T1", "T3"]);
        kit.outward.validates("T1", &main_id(), &me());
        kit.outward.validates("T3", &main_id(), &me());
        kit.outward.release_answers(vec![FakeHttp::Network, FakeHttp::Network]);
        let rx = kit.engine.claims.release(&main_id(), "00000000000000ac", Instant::now(), 15_000);
        let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
        assert_eq!(kit.outward.release_bearers(), vec!["T1".to_string(), "T3".to_string(), "T3".to_string()]);
        assert_eq!(kit.outward.fetch_calls(&main_id()), 3, "a fresh check before each attempt");
    }

    #[tokio::test]
    async fn a_claim_401_drops_the_bearers_cached_owner_and_is_not_retried() {
        let kit = Kit::new();
        let bearer = token_of(&main_id());
        kit.engine.verified(&main_id(), Duration::from_secs(5)).await.unwrap();
        kit.engine.verified(&main_id(), Duration::from_secs(5)).await.unwrap();
        assert_eq!(kit.outward.validate_calls(&bearer), 1, "cached");
        kit.outward.claim_answers(vec![FakeHttp::Status(401, "".into())]);
        let r = kit.engine.claims.claim(request(&bearer, 500)).await;
        assert!(matches!(r, ClaimResult::Unknown { why: Unknown::NotReached(401), .. }));
        kit.settle_claims().await;
        assert_eq!(kit.outward.claim_calls(), 1, "no retry");
        assert!(kit.outward.releases().is_empty());
        kit.engine.verified(&main_id(), Duration::from_secs(5)).await.unwrap();
        assert_eq!(kit.outward.validate_calls(&bearer), 2, "the 401 dropped the cached owner");
    }

    #[tokio::test]
    async fn a_release_after_a_switch_is_never_sent_as_another_account() {
        let kit = Kit::new();
        // After a switch the owner's fetch hands back another user's token (T2).
        kit.outward.script_tokens(&main_id(), vec!["T2"]);
        kit.outward.validates("T2", "4444", "someone");
        let rx = kit.engine.claims.release(&main_id(), "00000000000000ad", Instant::now(), 15_000);
        let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
        assert!(kit.outward.release_bearers().is_empty(), "dropped, never posted with T2");
        assert_eq!(kit.outward.fetch_calls(&main_id()), 1, "a wrong account is not retried");
    }

    #[tokio::test]
    async fn a_release_goes_out_while_an_account_is_being_removed() {
        let kit = Kit::new();
        let guard = super::super::before_accounts_removed_on(Some(&kit.engine), &[alt_id()]).await;
        let rx = kit.engine.claims.release(&alt_id(), "00000000000000af", Instant::now(), 15_000);
        let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
        assert_eq!(kit.outward.release_bearers(), vec![token_of(&alt_id())], "a fresh token needs no refresh");
        drop(guard);
    }

    #[tokio::test]
    async fn a_release_whose_owner_check_is_refused_is_dropped_and_unreachable_retries() {
        let kit = Kit::new();
        kit.outward.set_token(&alt_id(), FakeToken::Refused);
        let rx = kit.engine.claims.release(&alt_id(), "00000000000000ae", Instant::now(), 15_000);
        let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
        assert!(kit.outward.releases().is_empty(), "dropped, never sent");
        let unreachable = Kit::new();
        unreachable.outward.set_token(&alt_id(), FakeToken::Unreachable);
        let rx = unreachable.engine.claims.release(&alt_id(), "00000000000000af", Instant::now(), 15_000);
        let _ = tokio::time::timeout(Duration::from_secs(2), rx).await;
        assert!(unreachable.outward.validate_calls(&token_of(&alt_id())) >= 3, "asked again at each retry");
    }

    #[tokio::test]
    async fn a_release_older_than_two_hours_is_never_sent() {
        let kit = Kit::new();
        let rx = kit.engine.claims.release("t", "00000000000000ab", Instant::now() - RELEASE_HORIZON, 15_000);
        let _ = rx.await;
        assert!(kit.outward.releases().is_empty());
    }

    #[test]
    fn reclaims_stop_at_sixty_a_minute() {
        let kit = Kit::new_sync();
        for _ in 0..RECLAIMS_PER_MINUTE {
            assert!(kit.engine.claims.take_reclaim());
        }
        assert!(!kit.engine.claims.take_reclaim());
    }

    #[test]
    fn a_reclaim_body_carries_its_flag_and_never_a_second_kind_of_call() {
        let kit = Kit::new_sync();
        let mut req = request("t", 500);
        req.kind = CallKind::Reclaim;
        let body = kit.engine.claims.body("0123456789abcdef", &req);
        assert_eq!(body["reclaim"], json!(true));
        assert!(body.get("reminder").is_none());
        assert_eq!(body["action"], "claim");
        req.kind = CallKind::Rule;
        let body = kit.engine.claims.body("0123456789abcdef", &req);
        assert!(body.get("reclaim").is_none() && body.get("reminder").is_none());
    }
}
